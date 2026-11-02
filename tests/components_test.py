#!/usr/bin/env python3
"""Tests of core/components/install.py: the optional components' installer.

Hermetic: every path (XDG_DATA_HOME, XDG_STATE_HOME, HOME) is a temp dir; the
registry is a test's own (NOTCHD_COMPONENTS_FILE), its downloads come from a
local HTTP server on 127.0.0.1, LocalAI is a fake one there too (the gallery
API of backends and models, a first speech request), and `pacman`, `pkexec`,
`sudo`, `yay` and the terminal are fakes on a PATH that holds nothing else.
Nothing here touches ~/.local/share/notchd or installs anything real. Every
process runs under `timeout -k 2`.

    python3 tests/components_test.py
"""

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INSTALL = os.path.join(ROOT, "core", "components", "install.py")
REAL_CATALOG = os.path.join(ROOT, "core", "components", "components.json")
TIMEOUT = shutil.which("timeout")

BLOB = bytes((i * 7 + 3) % 251 for i in range(300_000))
SLOW = bytes(i % 256 for i in range(4_000_000))


def read(path, mode="r"):
    with open(path, mode) as f:
        return f.read()


def write(path, text):
    with open(path, "w") as f:
        f.write(text)


class Handler(BaseHTTPRequestHandler):
    """Serves /small.bin, /tiny.json, /slow.bin (throttled), /broken.bin (cuts
    off at 100 KB while `mode["broken"]`), with Range, and 404 for the rest."""
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        body = {"/small.bin": BLOB, "/tiny.json": b'{"ok": true}', "/slow.bin": SLOW, "/broken.bin": BLOB}.get(self.path)
        if body is None:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        start = 0
        rng = self.headers.get("Range")
        self.server.ranges.append(rng)
        if rng and rng.startswith("bytes="):
            start = int(rng[6:].split("-")[0])
        if start >= len(body):
            self.send_response(416)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.send_response(206 if rng else 200)
        if rng:
            self.send_header("Content-Range", "bytes %d-%d/%d" % (start, len(body) - 1, len(body)))
        self.send_header("Content-Length", str(len(body) - start))
        self.end_headers()
        cut = len(body)
        if self.path == "/broken.bin" and self.server.mode["broken"]:
            cut = min(cut, 100_000)
        pos = start
        try:
            while pos < cut:
                end = min(pos + 65536, cut)
                self.wfile.write(body[pos:end])
                self.wfile.flush()
                pos = end
                if self.path == "/slow.bin":
                    time.sleep(0.05)
        except (BrokenPipeError, ConnectionResetError):
            pass
        if cut < len(body):
            self.close_connection = True


class FakeLocalAI(BaseHTTPRequestHandler):
    """LocalAI's gallery API as far as the installer uses it: backends and
    models are applied as jobs that report bytes once, then finish (and only
    then write what they install, under `root`); a first speech request.
    `server.mode`: slow (a job never ends), model_error, speech_fail."""
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def reply(self, code, obj, raw=None):
        body = raw if raw is not None else json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        srv = self.server
        if self.path == "/readyz":
            return self.reply(200, None, b"OK")
        parts = self.path.strip("/").split("/")
        if len(parts) == 3 and parts[1] == "jobs" and parts[2] in srv.jobs:
            job = srv.jobs[parts[2]]
            job["polls"] += 1
            if srv.mode.get("slow") or job["polls"] < 2:
                return self.reply(200, {"processed": False, "progress": 50, "current_bytes": 500, "total_bytes": 1000,
                                        "message": "Downloading"})
            if parts[0] == "models" and srv.mode.get("model_error"):
                return self.reply(200, {"processed": True, "error": "could not download the model", "message": "error"})
            job["finish"]()
            return self.reply(200, {"processed": True, "message": "completed", "progress": 100})
        self.reply(404, {"error": {"message": "nope"}})

    def do_POST(self):
        srv = self.server
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
        srv.calls.append((self.path, body))
        if self.path in ("/backends/apply", "/models/apply"):
            jid = "job%d" % (len(srv.jobs) + 1)
            kind = self.path.split("/")[1]
            root = srv.root

            def finish(kind=kind, body=body):
                if kind == "backends":
                    os.makedirs(os.path.join(root, "backends", body["id"].split("@")[-1]), exist_ok=True)
                else:
                    os.makedirs(os.path.join(root, "models"), exist_ok=True)
                    with open(os.path.join(root, "models", body["name"] + ".yaml"), "w") as f:
                        f.write("name: %s\n" % body["name"])
                    with open(os.path.join(root, "models", "voice.onnx"), "w") as f:
                        f.write("x" * 100)
            srv.jobs[jid] = {"polls": 0, "finish": finish}
            return self.reply(200, {"uuid": jid, "id": jid})
        if self.path.endswith("/cancel"):
            return self.reply(200, {})
        if self.path == "/v1/audio/speech":
            if srv.mode.get("speech_fail"):
                return self.reply(500, {"error": {"code": 500, "message": "weights could not be fetched"}})
            os.makedirs(os.path.join(srv.root, "hf"), exist_ok=True)
            with open(os.path.join(srv.root, "hf", "weights"), "w") as f:
                f.write("w" * 200)
            return self.reply(200, None, b"RIFF....WAVE")
        self.reply(404, {"error": {"message": "nope"}})


class Case(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="notchd-components-test.")
        self.data = os.path.join(self.dir, "data")
        self.state = os.path.join(self.dir, "state")
        self.bin = os.path.join(self.dir, "bin")
        self.sysbin = os.path.join(self.dir, "sysbin")
        os.makedirs(self.bin)
        os.makedirs(self.sysbin)
        for tool in ("sh", "cat", "touch", "sleep", "env", "mkdir", "chmod", "dirname"):
            os.symlink(shutil.which(tool), os.path.join(self.sysbin, tool))
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.ranges = []
        self.server.mode = {"broken": True}
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = "http://127.0.0.1:%d" % self.server.server_address[1]
        self.ai = ThreadingHTTPServer(("127.0.0.1", 0), FakeLocalAI)
        self.ai.root = os.path.join(self.data, "notchd", "localai")
        self.ai.calls = []
        self.ai.jobs = {}
        self.ai.mode = {}
        self.ai.daemon_threads = True
        threading.Thread(target=self.ai.serve_forever, daemon=True).start()
        self.db = os.path.join(self.dir, "pacman-db")
        self.registry = os.path.join(self.dir, "components.json")
        self.write_registry()
        self.addCleanup(self.cleanup)

    def cleanup(self):
        self.server.shutdown()
        self.server.server_close()
        self.stop_ai()
        # a worker a test left running
        sdir = os.path.join(self.state, "notchd", "components")
        if os.path.isdir(sdir):
            for n in os.listdir(sdir):
                if n.endswith(".json"):
                    try:
                        with open(os.path.join(sdir, n)) as f:
                            pid = json.load(f).get("pid")
                        if pid:
                            os.kill(pid, signal.SIGKILL)
                    except (OSError, ValueError):
                        pass
        shutil.rmtree(self.dir, ignore_errors=True)

    def stop_ai(self):
        if self.ai:
            self.ai.shutdown()
            self.ai.server_close()
            self.ai = None

    def write_registry(self):
        reg = {"components": [
            {"id": "tf", "title": "Files", "kind": "files", "size": 400000, "dir": "models",
             "files": [{"name": "t.json", "url": self.url + "/tiny.json"}, {"name": "small.bin", "url": self.url + "/small.bin"}]},
            {"id": "slow", "title": "Slow", "kind": "files", "size": 4000000, "dir": "models",
             "files": [{"name": "slow.bin", "url": self.url + "/slow.bin"}]},
            {"id": "broken", "title": "Broken", "kind": "files", "size": 300000, "dir": "models",
             "files": [{"name": "broken.bin", "url": self.url + "/broken.bin"}]},
            {"id": "gone", "title": "Gone", "kind": "files", "size": 1, "dir": "models",
             "files": [{"name": "gone.bin", "url": self.url + "/nope.bin"}]},
            {"id": "te", "title": "Engine", "kind": "files", "size": 300000, "dir": "localai", "remove_dir": True,
             "files": [{"name": "bin/local-ai", "exec": True, "url": self.url + "/small.bin"}]},
            {"id": "tm", "title": "Voice", "kind": "localai", "size": 5000, "requires": "te",
             "backend": "localai@cpu-x", "backend_dir": "cpu-x",
             "model": {"gallery": "localai@gal", "name": "m1", "overrides": {"backend": "cpu-x", "options": ["language:pt"]}},
             "paths": ["backends/cpu-x", "models/voice.onnx", "hf"], "warmup": "Olá."},
            {"id": "tn", "title": "Voice without a warm-up", "kind": "localai", "size": 5000, "requires": "te",
             "backend": "localai@cpu-x", "backend_dir": "cpu-x",
             "model": {"gallery": "localai@gal", "name": "m2"}, "paths": ["backends/cpu-x"]},
            {"id": "sa", "title": "Shared A", "kind": "localai", "size": 9000, "requires": "te", "engine": "pocket",
             "language": "a", "hf_language": "lang_a", "backend": "localai@cpu-s", "backend_dir": "cpu-s",
             "model": {"gallery": "localai@s", "name": "ma"}, "paths": ["backends/cpu-s", "hf"]},
            {"id": "sb", "title": "Shared B", "kind": "localai", "size": 9000, "requires": "te", "engine": "pocket",
             "language": "b", "hf_language": "lang_b", "backend": "localai@cpu-s", "backend_dir": "cpu-s",
             "model": {"gallery": "localai@s", "name": "mb"}, "paths": ["backends/cpu-s", "hf"]},
            {"id": "sc", "title": "Shared C", "kind": "localai", "size": 9000, "requires": "te", "engine": "pocket",
             "language": "c", "hf_language": "lang_c", "backend": "localai@cpu-s", "backend_dir": "cpu-s",
             "model": {"gallery": "localai@s", "name": "mc"}, "paths": ["backends/cpu-s", "hf"]},
            {"id": "pa", "title": "Own A", "kind": "localai", "size": 900, "requires": "te", "engine": "piper",
             "language": "a", "backend": "localai@cpu-p", "backend_dir": "cpu-p",
             "model": {"gallery": "localai@p", "name": "pma"}, "paths": ["backends/cpu-p", "models/pa.onnx"]},
            {"id": "pb", "title": "Own B", "kind": "localai", "size": 900, "requires": "te", "engine": "piper",
             "language": "b", "backend": "localai@cpu-p", "backend_dir": "cpu-p",
             "model": {"gallery": "localai@p", "name": "pmb"}, "paths": ["backends/cpu-p", "models/pb.onnx"]},
            {"id": "ta", "title": "From the AUR", "kind": "system", "aur": True, "size": 1000, "packages": ["aur-pkg"], "bins": ["nothing-here"]},
            {"id": "ts", "title": "System", "kind": "system", "size": 1000, "packages": ["pkg-a", "pkg-b"], "bins": ["nothing-here"]},
        ]}
        with open(self.registry, "w") as f:
            json.dump(reg, f)

    def fake(self, name, body):
        path = os.path.join(self.bin, name)
        with open(path, "w") as f:
            f.write("#!/bin/sh\n" + body)
        os.chmod(path, 0o755)

    def fake_pacman(self):
        self.fake("pacman", '''db="$FAKE_PACMAN_DB"; touch "$db"
case "$1" in
  -Qq) cat "$db" ;;
  -S) [ -n "$FAKE_PACMAN_FAIL" ] && { echo "error: failed"; exit 1; }
      shift; for a; do case "$a" in -*) ;; *) echo "$a" >> "$db" ;; esac; done; echo "installing" ;;
esac
''')
        self.fake("pkexec", '[ -n "$FAKE_PKEXEC_CODE" ] && exit "$FAKE_PKEXEC_CODE"\nexec "$@"\n')
        self.fake("sudo", 'exec "$@"\n')
        self.fake("fake-term", 'exec "$@" < /dev/null\n')

    def env(self, **extra):
        env = {"PATH": self.bin + ":" + self.sysbin, "HOME": self.dir, "XDG_DATA_HOME": self.data,
               "XDG_STATE_HOME": self.state, "NOTCHD_COMPONENTS_FILE": self.registry,
               "FAKE_PACMAN_DB": self.db, "FAKE_LOG": os.path.join(self.dir, "log"),
               "NOTCHD_LOCALAI_URL": "http://127.0.0.1:%d" % (self.ai.server_address[1] if self.ai else 1)}
        env.update(extra)
        return env

    def call(self, *args, timeout=30, **extra):
        return subprocess.run([TIMEOUT, "-k", "2", str(timeout), sys.executable, INSTALL, *args],
                              capture_output=True, text=True, env=self.env(**extra))

    def status(self, **extra):
        p = self.call("status", **extra)
        self.assertEqual(p.returncode, 0, p.stderr)
        return json.loads(p.stdout)

    def state_file(self, cid):
        try:
            with open(os.path.join(self.state, "notchd", "components", cid + ".json")) as f:
                return json.load(f)
        except (OSError, ValueError):
            return {}

    def models(self):
        d = os.path.join(self.data, "notchd", "models")
        return sorted(os.listdir(d)) if os.path.isdir(d) else []

    def wait_for(self, cond, what, seconds=15):
        end = time.time() + seconds
        while time.time() < end:
            got = cond()
            if got:
                return got
            time.sleep(0.05)
        self.fail("timed out waiting for " + what)

    def log(self):
        try:
            with open(os.path.join(self.dir, "log")) as f:
                return f.read()
        except OSError:
            return ""


class Catalog(unittest.TestCase):
    def test_the_shipped_registry_is_well_formed(self):
        comps = json.loads(read(REAL_CATALOG))["components"]
        ids = [c["id"] for c in comps]
        self.assertEqual(len(ids), len(set(ids)))
        for c in comps:
            for key in ("id", "title", "description", "kind", "size", "group", "needed_by"):
                self.assertIn(key, c, c["id"])
            self.assertIn(c["kind"], ("files", "localai", "system"))
            if c["kind"] == "files":
                self.assertTrue(c["files"] and all(f["url"].startswith("https://") for f in c["files"]))
                self.assertTrue(c["dir"])
            if c["kind"] == "localai":
                self.assertIn(c["requires"], ids)
                self.assertEqual(c["model"]["overrides"]["backend"], c["backend_dir"])  # the CPU backend, by its own name
                self.assertTrue(c["backend"] and c["backend_dir"] and c["model"]["gallery"] and c["paths"])
                self.assertIn("backends/" + c["backend_dir"], c["paths"])
                self.assertTrue(c["engine"] in ("piper", "pocket") and c["language"], c["id"])
            if c["kind"] == "system":
                self.assertTrue(c["packages"])

    def test_the_bridge_and_the_catalog_agree_on_the_voices(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("local_live", os.path.join(ROOT, "core", "live", "local-live.py"))
        live = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(live)
        comps = {c["id"]: c for c in json.loads(read(REAL_CATALOG))["components"]}
        known = live.known_voices()
        voices = [c for c in comps.values() if c["kind"] == "localai"]
        # the bridge knows every voice the registry installs, by the model LocalAI lists it as
        self.assertEqual([(v["component"], v["model"], v["engine"], v["language"]) for v in known],
                         [(c["id"], c["model"]["name"], c["engine"], c["language"]) for c in voices])
        # and each language has the engine pair the bridge picks between, Piper first
        for code in {v["language"] for v in known}:
            self.assertEqual([v["engine"] for v in known if v["language"] == code], ["piper", "pocket"], code)
        # LocalAI is one file, and the program the installer and the bridge start
        self.assertEqual(comps["localai"]["files"][0]["name"], "bin/local-ai")
        self.assertTrue(comps["localai"]["files"][0]["exec"] and comps["localai"]["remove_dir"])
        self.assertTrue(comps["voxtype"]["aur"] and comps["voxtype"]["bins"] == ["voxtype"])


class Files(Case):
    def test_status_before_anything(self):
        s = self.status()
        self.assertEqual(set(s), {"tf", "slow", "broken", "gone", "te", "tm", "tn", "ts", "ta", "sa", "sb", "sc", "pa", "pb"})
        self.assertFalse(s["tf"]["installed"])
        self.assertFalse(s["tf"]["active"])
        self.assertEqual(s["tf"]["phase"], "")
        self.assertTrue(s["tf"]["removable"])
        self.assertFalse(s["ts"]["removable"])

    def test_installs_and_leaves_no_part_files(self):
        p = self.call("run", "tf")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(self.models(), ["small.bin", "t.json"])
        self.assertEqual(read(os.path.join(self.data, "notchd", "models", "small.bin"), "rb"), BLOB)
        s = self.status()["tf"]
        self.assertTrue(s["installed"])
        self.assertEqual(s["disk"], len(BLOB) + len(b'{"ok": true}'))
        self.assertEqual(s["phase"], "")
        self.assertEqual(self.state_file("tf")["phase"], "done")
        # running it again downloads nothing
        before = len(self.server.ranges)
        self.assertEqual(self.call("run", "tf").returncode, 0)
        self.assertEqual(len(self.server.ranges), before)

    def test_detached_install_shows_progress_then_finishes(self):
        p = self.call("install", "slow")
        self.assertEqual(p.returncode, 0, p.stderr)  # returned at once, the worker goes on
        st = self.wait_for(lambda: (lambda s: s if s.get("phase") == "downloading" and s.get("done") else None)(self.state_file("slow")),
                           "a progress update")
        self.assertEqual(st["total"], len(SLOW))
        self.assertGreater(st["pid"], 0)
        mid = self.wait_for(lambda: (lambda e: e if e["active"] and e["percent"] else None)(self.status()["slow"]), "a percent")
        self.assertEqual(mid["phase"], "downloading")
        self.assertLess(mid["percent"], 100)
        self.assertFalse(mid["installed"])
        self.assertTrue(os.path.exists(os.path.join(self.data, "notchd", "models", "slow.bin.part")))
        self.assertFalse(os.path.exists(os.path.join(self.data, "notchd", "models", "slow.bin")))
        # a second install while it runs does not start another
        self.assertEqual(self.call("install", "slow").returncode, 0)
        self.wait_for(lambda: self.status()["slow"]["installed"], "the end", 30)
        self.assertEqual(os.path.getsize(os.path.join(self.data, "notchd", "models", "slow.bin")), len(SLOW))
        self.assertEqual(self.models(), ["slow.bin"])
        self.assertEqual(self.state_file("slow")["phase"], "done")
        # no temp state files left by the atomic writes
        self.assertEqual([n for n in os.listdir(os.path.join(self.state, "notchd", "components")) if n.endswith(".tmp")], [])

    def test_a_failed_download_is_not_installed_and_a_retry_resumes_it(self):
        p = self.call("run", "broken")
        self.assertEqual(p.returncode, 1)
        s = self.status()["broken"]
        self.assertFalse(s["installed"])
        self.assertEqual(s["phase"], "failed")
        self.assertIn("incomplete", s["error"])
        self.assertEqual(self.models(), ["broken.bin.part"])  # kept to resume, never the final name
        self.assertEqual(os.path.getsize(os.path.join(self.data, "notchd", "models", "broken.bin.part")), 100_000)
        self.server.mode["broken"] = False
        self.server.ranges.clear()
        self.assertEqual(self.call("run", "broken").returncode, 0)
        self.assertEqual(self.server.ranges, ["bytes=100000-"])
        self.assertEqual(read(os.path.join(self.data, "notchd", "models", "broken.bin"), "rb"), BLOB)
        self.assertEqual(self.models(), ["broken.bin"])
        s = self.status()["broken"]
        self.assertTrue(s["installed"])
        self.assertEqual(s["error"], "")

    def test_an_http_error_fails_with_the_reason(self):
        self.assertEqual(self.call("run", "gone").returncode, 1)
        s = self.status()["gone"]
        self.assertEqual(s["phase"], "failed")
        self.assertIn("404", s["error"])
        self.assertEqual(self.models(), [])

    def test_a_server_that_is_down_fails(self):
        self.server.shutdown()
        self.server.server_close()
        self.assertEqual(self.call("run", "tf").returncode, 1)
        s = self.status()["tf"]
        self.assertEqual(s["phase"], "failed")
        self.assertIn("Could not download", s["error"])
        self.assertFalse(s["installed"])

    def test_cancel_stops_it_and_deletes_the_part(self):
        self.call("install", "slow")
        st = self.wait_for(lambda: (lambda s: s if s.get("done") else None)(self.state_file("slow")), "progress")
        pid = st["pid"]
        self.assertEqual(self.call("cancel", "slow").returncode, 0)
        self.wait_for(lambda: self.state_file("slow").get("phase") == "cancelled", "cancelled")
        self.wait_for(lambda: not os.path.exists("/proc/%d" % pid) or read("/proc/%d/stat" % pid).rsplit(")", 1)[1].split()[0] == "Z",
                      "the worker's end")
        self.assertEqual(self.models(), [])
        s = self.status()["slow"]
        self.assertFalse(s["installed"])
        self.assertFalse(s["active"])
        self.assertEqual(s["phase"], "")
        self.assertEqual(s["error"], "")

    def test_remove_deletes_only_its_own_files(self):
        self.assertEqual(self.call("run", "tf").returncode, 0)
        other = os.path.join(self.data, "notchd", "models", "ggml-small.bin")
        write(other, "someone else's")
        self.assertEqual(self.call("remove", "tf").returncode, 0)
        self.assertEqual(self.models(), ["ggml-small.bin"])
        self.assertFalse(self.status()["tf"]["installed"])
        self.assertEqual(self.state_file("tf"), {})

    def test_remove_clears_a_failure_and_its_partial(self):
        self.call("run", "broken")
        self.assertEqual(self.call("remove", "broken").returncode, 0)
        self.assertEqual(self.models(), [])
        self.assertEqual(self.status()["broken"]["phase"], "")

    def test_a_worker_that_died_is_a_failure_not_a_progress_bar(self):
        os.makedirs(os.path.join(self.state, "notchd", "components"))
        write(os.path.join(self.state, "notchd", "components", "tf.json"),
              json.dumps({"phase": "downloading", "pid": 2 ** 22 + 12345, "done": 5, "total": 10, "updated": time.time()}))
        s = self.status()["tf"]
        self.assertFalse(s["active"])
        self.assertEqual(s["phase"], "failed")
        self.assertIn("stopped unexpectedly", s["error"])
        # and a retry is allowed
        self.assertEqual(self.call("run", "tf").returncode, 0)
        self.assertTrue(self.status()["tf"]["installed"])

    def test_a_corrupt_state_file_is_ignored(self):
        os.makedirs(os.path.join(self.state, "notchd", "components"))
        write(os.path.join(self.state, "notchd", "components", "tf.json"), "{not json")
        self.assertEqual(self.status()["tf"]["phase"], "")

    def test_unknown_ids_and_commands(self):
        self.assertEqual(self.call("install", "nope").returncode, 2)
        self.assertEqual(self.call("remove", "nope").returncode, 2)
        self.assertEqual(self.call("frobnicate").returncode, 2)


class Engine(Case):
    """LocalAI's program: one executable, and what it holds goes with it."""

    def test_installs_an_executable_file(self):
        p = self.call("run", "te")
        self.assertEqual(p.returncode, 0, p.stderr)
        exe = os.path.join(self.data, "notchd", "localai", "bin", "local-ai")
        self.assertTrue(os.access(exe, os.X_OK))
        self.assertEqual(read(exe, "rb"), BLOB)
        self.assertTrue(self.status()["te"]["installed"])

    def test_a_file_that_is_not_executable_is_not_installed(self):
        self.call("run", "te")
        exe = os.path.join(self.data, "notchd", "localai", "bin", "local-ai")
        os.chmod(exe, 0o644)
        self.assertFalse(self.status()["te"]["installed"])
        self.assertEqual(self.call("run", "te").returncode, 0)  # the same file, made runnable again
        self.assertTrue(self.status()["te"]["installed"])

    def test_remove_deletes_the_voices_it_holds_but_not_a_running_one(self):
        self.call("run", "te")
        held = os.path.join(self.data, "notchd", "localai", "models", "m1.yaml")
        os.makedirs(os.path.dirname(held))
        write(held, "x")
        p = self.call("remove", "te")  # the fake LocalAI answers at the address: someone is using it
        self.assertEqual(p.returncode, 1)
        self.assertIn("is running", p.stderr)
        self.assertTrue(os.path.exists(held))
        self.stop_ai()
        p = self.call("remove", "te", NOTCHD_LOCALAI_URL="http://127.0.0.1:1")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.data, "notchd", "localai")))


class Models(Case):
    """A voice installed through LocalAI's API (a fake one here)."""

    def install_engine(self):
        self.assertEqual(self.call("run", "te").returncode, 0)

    def root(self, *parts):
        return os.path.join(self.data, "notchd", "localai", *parts)

    def test_needs_the_engine(self):
        p = self.call("run", "tm")
        self.assertEqual(p.returncode, 1)
        s = self.status()["tm"]
        self.assertEqual(s["phase"], "failed")
        self.assertIn("Install Engine first", s["error"])
        self.assertEqual(self.ai.calls, [])

    def test_installs_the_backend_the_model_and_loads_it_once(self):
        self.install_engine()
        self.assertFalse(self.status()["tm"]["installed"])
        p = self.call("run", "tm")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(self.ai.calls, [
            ("/backends/apply", {"id": "localai@cpu-x"}),
            ("/models/apply", {"id": "localai@gal", "name": "m1", "overrides": {"backend": "cpu-x", "options": ["language:pt"]}}),
            ("/v1/audio/speech", {"model": "m1", "input": "Olá."})])
        s = self.status()["tm"]
        self.assertTrue(s["installed"])
        self.assertEqual(s["phase"], "")
        self.assertGreater(s["disk"], 300)  # the model's file, its configuration and the weights it fetched
        self.assertTrue(os.path.isfile(self.root("markers", "tm.json")))

    def test_without_the_marker_it_is_not_installed(self):
        self.install_engine()
        self.call("run", "tm")
        os.unlink(self.root("markers", "tm.json"))
        self.assertFalse(self.status()["tm"]["installed"])
        shutil.rmtree(self.root("backends"))
        self.call("run", "tm")  # installs again (the marker last)
        self.assertTrue(self.status()["tm"]["installed"])

    def test_a_backend_that_is_there_is_not_installed_again(self):
        self.install_engine()
        os.makedirs(self.root("backends", "cpu-x"))
        self.call("run", "tn")
        self.assertEqual([c[0] for c in self.ai.calls], ["/models/apply"])  # and no warm-up for a voice without one
        self.assertTrue(self.status()["tn"]["installed"])

    def test_a_failed_model_says_why_and_leaves_no_marker(self):
        self.install_engine()
        self.ai.mode["model_error"] = True
        p = self.call("run", "tm")
        self.assertEqual(p.returncode, 1)
        s = self.status()["tm"]
        self.assertEqual(s["phase"], "failed")
        self.assertIn("could not download the model", s["error"])
        self.assertFalse(s["installed"])
        self.ai.mode["model_error"] = False
        self.ai.jobs.clear()
        self.assertEqual(self.call("run", "tm").returncode, 0)
        self.assertTrue(self.status()["tm"]["installed"])

    def test_a_voice_that_does_not_load_is_not_installed(self):
        self.install_engine()
        self.ai.mode["speech_fail"] = True
        self.assertEqual(self.call("run", "tm").returncode, 1)
        s = self.status()["tm"]
        self.assertIn("weights could not be fetched", s["error"])
        self.assertFalse(s["installed"])

    def test_a_localai_that_does_not_answer_and_does_not_start_fails_the_install(self):
        self.install_engine()  # its "program" here is the download's bytes: not one that can run
        self.stop_ai()
        p = self.call("run", "tm")
        self.assertEqual(p.returncode, 1)
        s = self.status()["tm"]
        self.assertIn("LocalAI", s["error"])
        self.assertFalse(s["installed"])

    def test_the_bytes_are_the_progress_and_cancel_cleans_up(self):
        self.install_engine()
        self.ai.mode["slow"] = True
        self.call("install", "tm")
        st = self.wait_for(lambda: self.state_file("tm") if self.state_file("tm").get("total") else None, "the job's bytes")
        self.assertEqual((st["phase"], st["done"], st["total"]), ("downloading", 500, 1000))
        self.assertEqual(self.status()["tm"]["percent"], 50)
        os.makedirs(self.root("backends", "cpu-x"))  # what a job half done leaves
        self.call("cancel", "tm")
        self.wait_for(lambda: self.state_file("tm").get("phase") == "cancelled", "cancelled")
        self.assertTrue(any(c[0].endswith("/cancel") for c in self.ai.calls))  # LocalAI was told, too
        self.assertFalse(os.path.exists(self.root("backends", "cpu-x")))
        self.assertFalse(self.status()["tm"]["installed"])

    def test_remove_deletes_what_the_voice_took_and_not_the_engine(self):
        self.install_engine()
        self.call("run", "tm")
        p = self.call("remove", "tm")
        self.assertEqual(p.returncode, 0, p.stderr)
        for gone in (("models", "m1.yaml"), ("models", "voice.onnx"), ("backends", "cpu-x"), ("hf",), ("markers", "tm.json")):
            self.assertFalse(os.path.exists(self.root(*gone)), gone)
        self.assertTrue(os.access(self.root("bin", "local-ai"), os.X_OK))
        self.assertFalse(self.status()["tm"]["installed"])
        self.assertTrue(self.status()["te"]["installed"])


class Shared(Case):
    """Voices of one engine share its backend and, for Pocket TTS, the weights
    cache (hf/): what one removes, the others still need."""
    SHARED = ["sa", "sb", "sc"]

    def root(self, *parts):
        return os.path.join(self.data, "notchd", "localai", *parts)

    def put(self, path, size, fill="x"):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        write(path, fill * size)

    def install_by_hand(self, cid):
        """What an installation leaves: the backend (once), the model's
        files, and for a Pocket TTS voice its language in the weights cache
        (snapshots point at blobs, as Hugging Face's cache has them)."""
        c = {x["id"]: x for x in json.loads(read(self.registry))["components"]}[cid]
        self.put(self.root("backends", c["backend_dir"], "backend.bin"), 5000)
        self.put(self.root("models", c["model"]["name"] + ".yaml"), 20)
        for p in c["paths"]:
            if p.startswith("models/"):
                self.put(self.root(p), 300)
        if c.get("hf_language"):
            repo = self.root("hf", "models--org--repo")
            snap = os.path.join(repo, "snapshots", "rev1")
            os.makedirs(os.path.join(snap, "languages", c["hf_language"]), exist_ok=True)
            # as the cache lays it out: a revision's file -> the repository's blob -> the shared store's file
            for name, size in (("common", 100), ("w_" + c["hf_language"], 2000 + 100 * len(c["hf_language"]))):
                shared = self.root("hf", "blobs", name[:2], "hash_" + name)
                if not os.path.exists(shared):
                    self.put(shared, size, "w")
                    self.put(shared + ".lock", 0)
                    self.put(shared + ".refs", 30)
                blob = os.path.join(repo, "blobs", name)
                os.makedirs(os.path.dirname(blob), exist_ok=True)
                if not os.path.lexists(blob):
                    os.symlink(os.path.relpath(shared, os.path.dirname(blob)), blob)
                link = os.path.join(snap, "config.json") if name == "common" else os.path.join(snap, "languages", c["hf_language"], "model.safetensors")
                if not os.path.lexists(link):
                    os.symlink(os.path.relpath(blob, os.path.dirname(link)), link)
        os.makedirs(self.root("markers"), exist_ok=True)
        write(self.root("markers", cid + ".json"), "{}")

    def on_disk(self):
        """The bytes of every real file under LocalAI's folder but the engine's and the markers."""
        total = 0
        for base, dirs, names in os.walk(self.root()):
            dirs[:] = [d for d in dirs if d not in ("bin", "markers")]
            for n in names:
                q = os.path.join(base, n)
                if not os.path.islink(q):
                    total += os.path.getsize(q)
        return total

    def test_removing_any_voice_in_any_order_keeps_the_others_whole(self):
        import itertools
        for order in itertools.permutations(self.SHARED):
            with self.subTest(order=order):
                shutil.rmtree(self.root(), ignore_errors=True)
                self.assertEqual(self.call("run", "te").returncode, 0)
                for cid in self.SHARED:
                    self.install_by_hand(cid)
                left = list(self.SHARED)
                for cid in order:
                    self.assertEqual(self.call("remove", cid).returncode, 0)
                    left.remove(cid)
                    st = self.status()
                    lang = {"sa": "lang_a", "sb": "lang_b", "sc": "lang_c"}
                    self.assertFalse(st[cid]["installed"])
                    self.assertFalse(os.path.exists(self.root("models", {"sa": "ma", "sb": "mb", "sc": "mc"}[cid] + ".yaml")))
                    for other in left:  # whole: installed, its weights and the backend there
                        self.assertTrue(st[other]["installed"], (cid, other))
                        self.assertTrue(os.path.isfile(self.root("hf", "models--org--repo", "snapshots", "rev1", "languages",
                                                                 lang[other], "model.safetensors")))
                        self.assertGreater(os.path.getsize(os.path.realpath(self.root("hf", "models--org--repo", "snapshots", "rev1",
                                                                                    "languages", lang[other], "model.safetensors"))), 2000)
                    if left:
                        self.assertTrue(os.path.isfile(self.root("backends", "cpu-s", "backend.bin")))
                        # the removed language's weights are gone, with its blob: nothing of it is left to take space
                        self.assertFalse(os.path.exists(self.root("hf", "models--org--repo", "snapshots", "rev1", "languages", lang[cid])))
                        self.assertFalse(os.path.lexists(self.root("hf", "models--org--repo", "blobs", "w_" + lang[cid])))
                        self.assertFalse(os.path.exists(self.root("hf", "blobs", "w_", "hash_w_" + lang[cid])))
                        self.assertFalse(any(n.startswith("hash_w_" + lang[cid]) for _b, _d, ns in os.walk(self.root("hf", "blobs")) for n in ns))
                        self.assertTrue(os.path.isfile(self.root("hf", "blobs", "co", "hash_common")))
                        self.assertTrue(os.path.isfile(self.root("hf", "blobs", "co", "hash_common.refs")))
                    else:  # the last one takes the shared things with it
                        self.assertFalse(os.path.exists(self.root("backends", "cpu-s")))
                        self.assertFalse(os.path.exists(self.root("hf")))
                self.assertTrue(self.status()["te"]["installed"])

    def test_the_sizes_add_up_to_what_is_on_disk_whatever_is_installed(self):
        import itertools
        for n in range(1, 4):
            for subset in itertools.combinations(self.SHARED + ["pa", "pb"], n):
                with self.subTest(subset=subset):
                    shutil.rmtree(self.root(), ignore_errors=True)
                    for cid in subset:
                        self.install_by_hand(cid)
                    self.assertEqual(self.call("run", "te").returncode, 0)
                    st = self.status()
                    disks = {cid: st[cid]["disk"] for cid in subset}
                    self.assertTrue(all(st[cid]["installed"] for cid in subset))
                    self.assertEqual(sum(disks.values()), self.on_disk())
                    # the first of them carries the backend (and what no language owns); the others only their own
                    first, rest = subset[0], subset[1:]
                    for cid in rest:
                        if cid[0] == first[0]:
                            self.assertLess(disks[cid], disks[first], (cid, first))

    def test_a_voice_of_another_engine_is_not_shared_with(self):
        self.assertEqual(self.call("run", "te").returncode, 0)
        for cid in ("sa", "pa", "pb"):
            self.install_by_hand(cid)
        self.assertEqual(self.call("remove", "sa").returncode, 0)  # the only Pocket voice: its backend and weights go
        self.assertFalse(os.path.exists(self.root("backends", "cpu-s")))
        self.assertFalse(os.path.exists(self.root("hf")))
        self.assertEqual(self.call("remove", "pa").returncode, 0)  # Piper's backend stays for the other voice
        self.assertTrue(os.path.isfile(self.root("backends", "cpu-p", "backend.bin")))
        self.assertTrue(self.status()["pb"]["installed"] and os.path.isfile(self.root("models", "pb.onnx")))
        self.assertFalse(os.path.exists(self.root("models", "pa.onnx")))
        self.assertEqual(self.call("remove", "pb").returncode, 0)
        self.assertFalse(os.path.exists(self.root("backends", "cpu-p")))

    def test_a_voice_being_installed_still_holds_what_it_shares(self):
        self.assertEqual(self.call("run", "te").returncode, 0)
        self.install_by_hand("sa")
        self.install_by_hand("sb")
        os.unlink(self.root("markers", "sb.json"))  # not installed yet...
        os.makedirs(os.path.join(self.state, "notchd", "components"), exist_ok=True)
        worker = subprocess.Popen(["sleep", "30"])
        self.addCleanup(worker.wait)
        self.addCleanup(worker.kill)
        write(os.path.join(self.state, "notchd", "components", "sb.json"),
              json.dumps({"phase": "installing", "pid": worker.pid, "updated": time.time()}))  # ...but a worker is on it
        self.assertEqual(self.call("remove", "sa").returncode, 0)
        self.assertTrue(os.path.isfile(self.root("backends", "cpu-s", "backend.bin")))
        self.assertTrue(os.path.isfile(self.root("hf", "models--org--repo", "snapshots", "rev1", "languages", "lang_b", "model.safetensors")))

    def test_cancelling_an_installation_leaves_what_the_others_hold(self):
        self.assertEqual(self.call("run", "te").returncode, 0)
        self.install_by_hand("sa")
        self.install_by_hand("sb")
        os.unlink(self.root("markers", "sb.json"))  # sb half installed by a worker that is gone
        os.makedirs(os.path.join(self.state, "notchd", "components"), exist_ok=True)
        write(os.path.join(self.state, "notchd", "components", "sb.json"), json.dumps({"phase": "installing", "pid": 0, "updated": 0}))
        self.assertEqual(self.call("cancel", "sb").returncode, 0)
        self.assertFalse(os.path.exists(self.root("hf", "models--org--repo", "snapshots", "rev1", "languages", "lang_b")))
        self.assertTrue(os.path.isfile(self.root("backends", "cpu-s", "backend.bin")))
        self.assertTrue(self.status()["sa"]["installed"])

    def test_the_installation_of_a_second_language_downloads_neither_the_backend_nor_asks_for_it_again(self):
        self.assertEqual(self.call("run", "te").returncode, 0)
        self.install_by_hand("sa")
        self.assertEqual(self.call("run", "sb").returncode, 0)
        self.assertEqual([x[0] for x in self.ai.calls], ["/models/apply"])  # the backend is there


class System(Case):
    def setUp(self):
        super().setUp()
        self.fake_pacman()

    def test_status_reads_pacman(self):
        write(self.db, "pkg-a\nother\n")
        s = self.status()["ts"]
        self.assertFalse(s["installed"])
        self.assertEqual(s["missing"], ["pkg-b"])
        write(self.db, "pkg-a\nother\npkg-b\n")
        self.assertTrue(self.status()["ts"]["installed"])

    def test_installs_with_pkexec_when_an_agent_can_ask(self):
        p = self.call("run", "ts", NOTCHD_POLKIT_AGENT="1")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(read(self.db).split(), ["pkg-a", "pkg-b"])
        self.assertTrue(self.status()["ts"]["installed"])

    def test_only_the_missing_packages_are_asked_for(self):
        write(self.db, "pkg-a\n")
        self.call("run", "ts", NOTCHD_POLKIT_AGENT="1")
        self.assertEqual(read(self.db).split(), ["pkg-a", "pkg-b"])

    def test_refused_authentication_says_so(self):
        p = self.call("run", "ts", NOTCHD_POLKIT_AGENT="1", FAKE_PKEXEC_CODE="126")
        self.assertEqual(p.returncode, 1)
        s = self.status()["ts"]
        self.assertEqual(s["phase"], "failed")
        self.assertIn("Authentication", s["error"])
        self.assertFalse(s["installed"])

    def test_without_an_agent_it_asks_in_a_terminal(self):
        p = self.call("run", "ts", NOTCHD_POLKIT_AGENT="0", NOTCHD_TERMINAL="fake-term")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(read(self.db).split(), ["pkg-a", "pkg-b"])
        self.assertEqual([n for n in os.listdir(os.path.join(self.state, "notchd", "components")) if n.endswith(".exit")], [])

    def test_a_failing_pacman_in_the_terminal_fails(self):
        p = self.call("run", "ts", NOTCHD_POLKIT_AGENT="0", NOTCHD_TERMINAL="fake-term", FAKE_PACMAN_FAIL="1")
        self.assertEqual(p.returncode, 1)
        self.assertIn("pacman failed", self.status()["ts"]["error"])

    def test_cancel_while_the_terminal_waits(self):
        self.fake("hang-term", "sleep 30\n")
        self.call("install", "ts", NOTCHD_POLKIT_AGENT="0", NOTCHD_TERMINAL="hang-term")
        self.wait_for(lambda: "terminal" in self.state_file("ts").get("detail", ""), "the terminal wait")
        self.call("cancel", "ts")
        self.wait_for(lambda: self.state_file("ts").get("phase") == "cancelled", "cancelled")
        time.sleep(0.2)
        self.assertEqual(subprocess.run(["pgrep", "-f", "hang-term"], capture_output=True, text=True).stdout.split(), [])  # the terminal closed

    def test_never_removed_from_here(self):
        write(self.db, "pkg-a\npkg-b\n")
        p = self.call("remove", "ts")
        self.assertEqual(p.returncode, 1)
        self.assertIn("installed by the system", p.stderr)
        self.assertTrue(self.status()["ts"]["installed"])

    def test_an_aur_package_is_built_by_the_helper_in_a_terminal(self):
        self.fake("yay", 'echo "$@" >> "$FAKE_LOG"; echo aur-pkg >> "$FAKE_PACMAN_DB"\n')
        p = self.call("run", "ta", NOTCHD_POLKIT_AGENT="1", NOTCHD_TERMINAL="fake-term")  # pkexec is for pacman only
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(self.log().split(), ["-S", "--needed", "aur-pkg"])
        self.assertTrue(self.status()["ta"]["installed"])

    def test_an_aur_package_without_a_helper_says_how(self):
        p = self.call("run", "ta", NOTCHD_TERMINAL="fake-term")
        self.assertEqual(p.returncode, 1)
        self.assertIn("yay -S aur-pkg", self.status()["ta"]["error"])
        self.assertEqual(self.status()["ta"]["removable"], False)

    def test_not_arch_says_what_to_install(self):
        os.unlink(os.path.join(self.bin, "pacman"))
        p = self.call("run", "ts")
        self.assertEqual(p.returncode, 1)
        self.assertIn("pkg-a, pkg-b", self.status()["ts"]["error"])


if __name__ == "__main__":
    unittest.main()
