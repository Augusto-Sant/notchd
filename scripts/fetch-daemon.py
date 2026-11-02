#!/usr/bin/env python3
"""Fetches the notchd daemon: the release the plugin pins, verified.

  fetch-daemon.py [--dest DIR] [--version V] [--releases FILE] [--target T]

Looks V up in FILE (default native/releases.json, kept in the repo: the
checksums are the plugin's own, never the release's), picks this machine's
target, downloads the archive to a temp file, checks its SHA-256 and puts
`notchd` into DIR (default $XDG_DATA_HOME/notchd/bin) atomically, executable.
V defaults to manifest.json's version. A `notchd` already in DIR that reports
V (`notchd --version`) is kept: no download. Prints the binary's path.

On failure prints one JSON line to stderr, {"error": <code>, "message": ...},
and exits with the code's number:

  2 usage        3 no-release (no entry for V)   4 unsupported-arch
  5 offline      6 checksum-mismatch             7 failed (HTTP error, bad
                                                   archive, disk)

Standard library only. Nothing is left in DIR on a failure.
"""
import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGETS = {
    "x86_64": "x86_64-unknown-linux-gnu",
    "amd64": "x86_64-unknown-linux-gnu",
    "aarch64": "aarch64-unknown-linux-gnu",
    "arm64": "aarch64-unknown-linux-gnu",
}
CODES = {"usage": 2, "no-release": 3, "unsupported-arch": 4, "offline": 5, "checksum-mismatch": 6, "failed": 7}


class Fail(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def default_dest():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "notchd", "bin")


def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError) as e:
        raise Fail("failed", "cannot read %s: %s" % (path, e))


def installed_version(binary):
    """What `binary --version` says ("notchd X"), or None."""
    if not os.access(binary, os.X_OK):
        return None
    try:
        out = subprocess.run([binary, "--version"], capture_output=True, text=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    words = out.split()
    return words[-1] if words else None


def download(url, dest):
    """url -> dest, returning its SHA-256."""
    digest = hashlib.sha256()
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "notchd-fetch-daemon"})
        with urllib.request.urlopen(request, timeout=30) as resp, open(dest, "wb") as out:
            while True:
                chunk = resp.read(1 << 16)
                if not chunk:
                    break
                digest.update(chunk)
                out.write(chunk)
    except urllib.error.HTTPError as e:
        raise Fail("failed", "the server answered %d %s" % (e.code, e.reason))
    except (urllib.error.URLError, OSError) as e:
        raise Fail("offline", "cannot download: %s" % getattr(e, "reason", e))
    return digest.hexdigest()


def extract(archive, dest_dir):
    """Writes the archive's `notchd` to dest_dir/notchd through a temp file."""
    tmp = None
    try:
        with tarfile.open(archive) as tar:
            member = next((m for m in tar.getmembers() if m.isfile() and os.path.basename(m.name) == "notchd"), None)
            if member is None:
                raise Fail("failed", "the archive holds no notchd")
            src = tar.extractfile(member)
            fd, tmp = tempfile.mkstemp(prefix=".notchd.", dir=dest_dir)
            with os.fdopen(fd, "wb") as out:
                while True:
                    chunk = src.read(1 << 16)
                    if not chunk:
                        break
                    out.write(chunk)
        os.chmod(tmp, 0o755)
        final = os.path.join(dest_dir, "notchd")
        os.replace(tmp, final)
        tmp = None
        return final
    except (tarfile.TarError, OSError, EOFError) as e:
        raise Fail("failed", "cannot unpack the archive: %s" % e)
    finally:
        if tmp and os.path.exists(tmp):
            os.unlink(tmp)


def fetch(dest, version, releases, target):
    binary = os.path.join(dest, "notchd")
    if installed_version(binary) == version:
        return binary
    if target is None:
        target = TARGETS.get(platform.machine().lower())
        if target is None:
            raise Fail("unsupported-arch", "no notchd build for %s" % platform.machine())
    entries = read_json(releases)
    by_target = entries.get(version) if isinstance(entries, dict) else None
    if not isinstance(by_target, dict) or not by_target:
        raise Fail("no-release", "no release for version %s" % version)
    entry = by_target.get(target)
    if not isinstance(entry, dict) or not entry.get("url") or not entry.get("sha256"):
        raise Fail("unsupported-arch", "version %s has no build for %s" % (version, target))
    try:
        os.makedirs(dest, exist_ok=True)
        work = tempfile.mkdtemp(prefix=".fetch.", dir=dest)
    except OSError as e:
        raise Fail("failed", "cannot write to %s: %s" % (dest, e))
    try:
        archive = os.path.join(work, "archive.tar.gz")
        got = download(entry["url"], archive)
        if got != entry["sha256"].lower():
            raise Fail("checksum-mismatch", "checksum mismatch for %s (got %s)" % (entry["url"], got))
        return extract(archive, dest)
    finally:
        for name in os.listdir(work):
            os.unlink(os.path.join(work, name))
        os.rmdir(work)


def main(argv=None):
    p = argparse.ArgumentParser(description="Fetch the pinned notchd daemon.")
    p.add_argument("--dest")
    p.add_argument("--version")
    p.add_argument("--releases", default=os.path.join(ROOT, "native", "releases.json"))
    p.add_argument("--target", help="override the machine's target triple (tests)")
    try:
        a = p.parse_args(argv)
    except SystemExit as e:
        return 0 if e.code == 0 else CODES["usage"]
    try:
        version = a.version or read_json(os.path.join(ROOT, "manifest.json")).get("version")
        if not version:
            raise Fail("failed", "no version: pass --version")
        print(fetch(a.dest or default_dest(), version, a.releases, a.target))
        return 0
    except Fail as e:
        print(json.dumps({"error": e.code, "message": str(e)}), file=sys.stderr)
        return CODES[e.code]


if __name__ == "__main__":
    sys.exit(main())
