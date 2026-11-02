#!/usr/bin/env python3
"""Tests of scripts/fetch-daemon.py, the pinned daemon download, and of the
version the plugin pins.

Hermetic: the destination and XDG_DATA_HOME are temp dirs, the "release" is a
tarball holding a fake `notchd` script, served by a local HTTP server on
127.0.0.1. Nothing touches ~/.local/share. Every process runs under
`timeout -k 2`.

    python3 tests/fetch_daemon_test.py
"""

import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FETCH = os.path.join(ROOT, "scripts", "fetch-daemon.py")
TIMEOUT = shutil.which("timeout")
VERSION = "9.9.9"
TARGET = "x86_64-unknown-linux-gnu"


def fake_daemon(version):
    return ("#!/bin/sh\necho 'notchd %s'\n" % version).encode()


def tarball(version=VERSION, name="notchd"):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        data = fake_daemon(version)
        info = tarfile.TarInfo("notchd-%s-%s/%s" % (version, TARGET, name))
        info.size = len(data)
        info.mode = 0o644
        tar.addfile(info, io.BytesIO(data))
    return buf.getvalue()


class Versions(unittest.TestCase):
    """Unit: the plugin, the daemon crate and the release pin agree."""

    def test_manifest_and_daemon_crate_share_a_version(self):
        with open(os.path.join(ROOT, "manifest.json")) as f:
            manifest = json.load(f)["version"]
        with open(os.path.join(ROOT, "native", "notchd-daemon", "Cargo.toml")) as f:
            crate = re.search(r'^version = "([^"]+)"', f.read(), re.M).group(1)
        self.assertEqual(manifest, crate, "bump both: manifest.json and native/notchd-daemon/Cargo.toml")

    def test_releases_file_is_a_map_of_versions_to_targets(self):
        with open(os.path.join(ROOT, "native", "releases.json")) as f:
            releases = json.load(f)
        for version, targets in releases.items():
            for target, entry in targets.items():
                self.assertRegex(entry["sha256"], r"^[0-9a-f]{64}$", "%s %s" % (version, target))
                self.assertTrue(entry["url"].startswith("https://"), "%s %s" % (version, target))


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.server.hits.append(self.path)
        body = self.server.files.get(self.path)
        if body is None:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class Fetch(unittest.TestCase):
    tier = "integration"  # fetch-daemon.py as a process against a local server

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="notchd-fetch-test.")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.dest = os.path.join(self.dir, "bin")
        self.archive = tarball()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.files = {"/notchd.tar.gz": self.archive}
        self.server.hits = []
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = "http://127.0.0.1:%d" % self.server.server_address[1]
        self.write_releases(sha=hashlib.sha256(self.archive).hexdigest())

    def write_releases(self, sha, url=None, version=VERSION, target=TARGET):
        self.releases = os.path.join(self.dir, "releases.json")
        with open(self.releases, "w") as f:
            json.dump({version: {target: {"url": url or self.url + "/notchd.tar.gz", "sha256": sha}}}, f)

    def fetch(self, *extra, version=VERSION):
        env = {"PATH": os.environ["PATH"], "HOME": self.dir, "XDG_DATA_HOME": os.path.join(self.dir, "data")}
        cmd = [TIMEOUT, "-k", "2", "30", sys.executable, "-I", FETCH, "--dest", self.dest,
               "--releases", self.releases, "--target", TARGET, "--version", version]
        p = subprocess.run(cmd + list(extra), capture_output=True, text=True, env=env)
        return p.returncode, p.stdout.strip(), p.stderr.strip()

    def error(self, stderr):
        return json.loads(stderr)["error"]

    def leftovers(self):
        return sorted(os.listdir(self.dest)) if os.path.isdir(self.dest) else []

    def test_good_download_installs_an_executable(self):
        code, out, err = self.fetch()
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(out, os.path.join(self.dest, "notchd"))
        self.assertTrue(os.access(out, os.X_OK))
        self.assertEqual(subprocess.run([out, "--version"], capture_output=True, text=True).stdout.strip(), "notchd " + VERSION)
        self.assertEqual(self.leftovers(), ["notchd"])

    def test_checksum_mismatch_is_refused_and_leaves_nothing(self):
        self.write_releases(sha="0" * 64)
        code, out, err = self.fetch()
        self.assertEqual(code, 6)
        self.assertEqual(self.error(err), "checksum-mismatch")
        self.assertEqual(out, "")
        self.assertEqual(self.leftovers(), [])

    def test_a_mismatch_keeps_the_binary_that_was_there(self):
        os.makedirs(self.dest)
        old = os.path.join(self.dest, "notchd")
        with open(old, "wb") as f:
            f.write(fake_daemon("0.0.1"))
        os.chmod(old, 0o755)
        self.write_releases(sha="f" * 64)
        code, _, _ = self.fetch()
        self.assertEqual(code, 6)
        with open(old, "rb") as f:
            self.assertEqual(f.read(), fake_daemon("0.0.1"))
        self.assertEqual(self.leftovers(), ["notchd"])

    def test_a_stale_binary_is_replaced(self):
        os.makedirs(self.dest)
        old = os.path.join(self.dest, "notchd")
        with open(old, "wb") as f:
            f.write(fake_daemon("0.0.1"))
        os.chmod(old, 0o755)
        code, out, _ = self.fetch()
        self.assertEqual(code, 0)
        self.assertEqual(subprocess.run([out, "--version"], capture_output=True, text=True).stdout.strip(), "notchd " + VERSION)

    def test_missing_version(self):
        code, _, err = self.fetch(version="1.2.3")
        self.assertEqual((code, self.error(err)), (3, "no-release"))
        self.assertEqual(self.server.hits, [])
        self.assertEqual(self.leftovers(), [])

    def test_unsupported_arch(self):
        code, _, err = self.fetch("--target", "riscv64gc-unknown-linux-gnu")
        self.assertEqual((code, self.error(err)), (4, "unsupported-arch"))
        self.assertEqual(self.server.hits, [])
        self.assertEqual(self.leftovers(), [])

    def test_already_present_is_not_downloaded(self):
        os.makedirs(self.dest)
        have = os.path.join(self.dest, "notchd")
        with open(have, "wb") as f:
            f.write(fake_daemon(VERSION))
        os.chmod(have, 0o755)
        code, out, _ = self.fetch()
        self.assertEqual((code, out), (0, have))
        self.assertEqual(self.server.hits, [])

    def test_offline(self):
        self.write_releases(sha="0" * 64, url="http://127.0.0.1:9/notchd.tar.gz")
        code, _, err = self.fetch()
        self.assertEqual((code, self.error(err)), (5, "offline"))
        self.assertEqual(self.leftovers(), [])

    def test_http_error_and_bad_archive_fail_cleanly(self):
        self.write_releases(sha="0" * 64, url=self.url + "/nope.tar.gz")
        code, _, err = self.fetch()
        self.assertEqual((code, self.error(err)), (7, "failed"))
        junk = b"not a tarball"
        self.server.files["/junk.tar.gz"] = junk
        self.write_releases(sha=hashlib.sha256(junk).hexdigest(), url=self.url + "/junk.tar.gz")
        code, _, err = self.fetch()
        self.assertEqual((code, self.error(err)), (7, "failed"))
        self.assertEqual(self.leftovers(), [])

    def test_default_dest_is_under_xdg_data_home(self):
        env = {"PATH": os.environ["PATH"], "HOME": self.dir, "XDG_DATA_HOME": os.path.join(self.dir, "data")}
        p = subprocess.run([TIMEOUT, "-k", "2", "30", sys.executable, "-I", FETCH, "--releases", self.releases,
                            "--target", TARGET, "--version", VERSION], capture_output=True, text=True, env=env)
        self.assertEqual(p.stdout.strip(), os.path.join(self.dir, "data", "notchd", "bin", "notchd"))


if __name__ == "__main__":
    unittest.main()
