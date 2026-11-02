#!/usr/bin/env python3
"""Tests of scripts/install.sh, the installer for other distributions.

Hermetic: HOME and XDG_* are temp dirs, `systemctl` is a stub on PATH that logs
its arguments, the "release" is a tarball holding a fake `notchd` served by a
local HTTP server on 127.0.0.1 and pinned by a temp releases.json. Nothing
touches the real ~/.local, systemd or ~/.config. Every process runs under
`timeout -k 2`.

    python3 tests/install_script_test.py
"""

import hashlib
import io
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INSTALL = os.path.join(ROOT, "scripts", "install.sh")
TIMEOUT = shutil.which("timeout")
TARGET = "x86_64-unknown-linux-gnu"


def tarball(version, extra=None):
    script = "#!/bin/sh\ncase \"$1\" in --version) echo 'notchd %s';; ipc) echo \"$@\" >> \"$FAKE_LOG\";; esac\n" % version
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        files = {"notchd": script.encode(), "LICENSE": b"MIT\n"}
        files.update(extra or {})
        for name, data in files.items():
            info = tarfile.TarInfo("notchd-%s-%s/%s" % (version, TARGET, name))
            info.size = len(data)
            info.mode = 0o755 if name == "notchd" else 0o644
            tar.addfile(info, io.BytesIO(data))
    return buf.getvalue()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        body = self.server.files.get(self.path)
        self.send_response(200 if body is not None else 404)
        self.send_header("Content-Length", str(len(body or b"")))
        self.end_headers()
        self.wfile.write(body or b"")


class Install(unittest.TestCase):
    tier = "integration"  # install.sh as a process against a local server

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="notchd-install-test.")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.home = os.path.join(self.dir, "home")
        self.stubs = os.path.join(self.dir, "stubs")
        self.log = os.path.join(self.dir, "systemctl.log")
        self.fake_log = os.path.join(self.dir, "notchd.log")
        os.makedirs(self.home)
        os.makedirs(self.stubs)
        stub = os.path.join(self.stubs, "systemctl")
        with open(stub, "w") as f:
            f.write("#!/bin/sh\necho \"$@\" >> \"%s\"\n" % self.log)
        os.chmod(stub, 0o755)
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.files = {}
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = "http://127.0.0.1:%d" % self.server.server_address[1]
        self.pins = {}
        self.release("1.0.0")

    def release(self, version, sha=None, extra=None):
        archive = tarball(version, extra)
        self.server.files["/%s.tar.gz" % version] = archive
        self.pins[version] = {TARGET: {
            "url": "%s/%s.tar.gz" % (self.url, version),
            "sha256": sha or hashlib.sha256(archive).hexdigest(),
        }}
        self.releases = os.path.join(self.dir, "releases.json")
        with open(self.releases, "w") as f:
            json.dump(self.pins, f)

    def run_install(self, *args):
        env = {
            "PATH": self.stubs + ":" + os.environ["PATH"],
            "HOME": self.home,
            "XDG_CONFIG_HOME": os.path.join(self.home, ".config"),
            "TMPDIR": self.dir,
            "FAKE_LOG": self.fake_log,
            "NOTCHD_INSTALL_TARGET": TARGET,
        }
        cmd = [TIMEOUT, "-k", "2", "60", "sh", INSTALL, "--releases", self.releases] + list(args)
        p = subprocess.run(cmd, capture_output=True, text=True, env=env)
        return p.returncode, p.stdout + p.stderr

    @property
    def bin(self):
        return os.path.join(self.home, ".local", "bin", "notchd")

    @property
    def unit(self):
        return os.path.join(self.home, ".config", "systemd", "user", "notchd.service")

    def systemctl(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as f:
            return f.read().splitlines()

    def version_of(self, path):
        return subprocess.run([path, "--version"], capture_output=True, text=True).stdout.split()[-1]

    def test_install_puts_the_binary_and_the_unit_and_starts_it(self):
        code, out = self.run_install()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.version_of(self.bin), "1.0.0")
        self.assertTrue(os.access(self.bin, os.X_OK))
        with open(self.unit) as f:
            text = f.read()
        self.assertIn("ExecStart=%s daemon\n" % self.bin, text)
        self.assertNotIn("--ensure", text)
        self.assertIn("WantedBy=default.target", text)
        calls = self.systemctl()
        self.assertIn("--user enable notchd.service", calls)
        self.assertIn("--user start notchd.service", calls)
        self.assertEqual(os.listdir(os.path.dirname(self.bin)), ["notchd"], "no temp file left")

    def test_the_newest_pinned_version_is_the_default(self):
        self.release("1.10.0")
        self.release("1.9.0")
        code, out = self.run_install()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.version_of(self.bin), "1.10.0")

    def test_an_asked_version_is_installed_and_an_unknown_one_refused(self):
        self.release("1.1.0")
        code, out = self.run_install("--version", "1.0.0")
        self.assertEqual((code, self.version_of(self.bin)), (0, "1.0.0"), out)
        code, out = self.run_install("--version", "7.7.7")
        self.assertNotEqual(code, 0)
        self.assertIn("no release for version 7.7.7", out)

    def test_extra_files_in_the_archive_are_ignored(self):
        self.release("1.0.0", extra={"sidecar/tool": b"x"})
        code, out = self.run_install()
        self.assertEqual(code, 0, out)
        self.assertEqual(os.listdir(os.path.dirname(self.bin)), ["notchd"])

    def test_a_checksum_mismatch_installs_nothing(self):
        self.release("1.0.0", sha="0" * 64)
        code, out = self.run_install()
        self.assertNotEqual(code, 0)
        self.assertIn("checksum mismatch", out)
        self.assertFalse(os.path.exists(self.bin))
        self.assertFalse(os.path.exists(self.unit))
        self.assertEqual(self.systemctl(), [])

    def test_a_mismatch_keeps_the_binary_that_was_there(self):
        self.run_install()
        self.release("2.0.0", sha="0" * 64)
        code, out = self.run_install()
        self.assertNotEqual(code, 0)
        self.assertEqual(self.version_of(self.bin), "1.0.0")

    def test_rerun_changes_nothing_and_does_not_restart(self):
        self.run_install()
        with open(self.unit) as f:
            unit = f.read()
        os.remove(self.fake_log) if os.path.exists(self.fake_log) else None
        os.remove(self.log)
        code, out = self.run_install()
        self.assertEqual(code, 0, out)
        self.assertIn("already installed", out)
        with open(self.unit) as f:
            self.assertEqual(f.read(), unit)
        self.assertFalse(os.path.exists(self.fake_log), "no shutdown on an unchanged binary")
        self.assertNotIn("--user daemon-reload", self.systemctl())

    def test_upgrade_replaces_the_binary_and_stops_the_old_daemon(self):
        self.run_install()
        self.release("2.0.0")
        code, out = self.run_install()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.version_of(self.bin), "2.0.0")
        with open(self.fake_log) as f:
            self.assertIn("ipc shutdown", f.read())
        self.assertEqual(self.systemctl()[-1], "--user start notchd.service")

    def test_a_prefix_moves_the_binary(self):
        prefix = os.path.join(self.dir, "opt")
        code, out = self.run_install("--prefix", prefix)
        self.assertEqual(code, 0, out)
        self.assertEqual(self.version_of(os.path.join(prefix, "bin", "notchd")), "1.0.0")
        self.assertFalse(os.path.exists(self.bin))
        with open(self.unit) as f:
            self.assertIn("ExecStart=%s/bin/notchd daemon" % prefix, f.read())

    def test_no_service_skips_the_unit_and_systemctl(self):
        code, out = self.run_install("--no-service")
        self.assertEqual(code, 0, out)
        self.assertTrue(os.path.exists(self.bin))
        self.assertFalse(os.path.exists(self.unit))
        self.assertEqual(self.systemctl(), [])

    def test_uninstall_removes_what_it_installed_and_keeps_the_config(self):
        self.run_install()
        config = os.path.join(self.home, ".config", "notchd", "config.json")
        os.makedirs(os.path.dirname(config))
        with open(config, "w") as f:
            f.write("{}")
        code, out = self.run_install("--uninstall")
        self.assertEqual(code, 0, out)
        self.assertFalse(os.path.exists(self.bin))
        self.assertFalse(os.path.exists(self.unit))
        self.assertIn("--user disable --now notchd.service", self.systemctl())
        self.assertTrue(os.path.exists(config))
        code, out = self.run_install("--uninstall")  # twice is fine
        self.assertEqual(code, 0, out)

    def test_the_extension_flag_is_reserved(self):
        code, out = self.run_install("--extension")
        self.assertNotEqual(code, 0)
        self.assertIn("does not exist yet", out)


if __name__ == "__main__":
    unittest.main()
