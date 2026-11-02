#!/usr/bin/env python3
"""The media module in the daemon, over MPRIS, end to end: the real `notchd`
against tests/visual/demo-player.py, both on a private D-Bus.

Hermetic: scripts/isolated.sh cuts the whole thing off from the desktop
session (private bus, its own XDG_RUNTIME_DIR, no display), the daemon's
socket, config, state and data are in a temp dir, and the player's cover is a
file there. Nothing touches the user's session bus, ~/.config/notchd or the
real runtime socket. Every process runs under `timeout -k`, and the whole
group is killed when the test ends.

    python3 tests/mpris_test.py
"""

import json
import os
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DAEMON = os.environ.get("NOTCHD_TEST_DAEMON", os.path.join(ROOT, "native", "target", "release", "notchd"))
PLAYER = os.path.join(ROOT, "tests", "visual", "demo-player.py")
ISOLATED = os.path.join(ROOT, "scripts", "isolated.sh")
WAIT = 20.0

# The player starts, then the daemon, both under the private bus. `timeout`
# stays in the test's process group (--foreground), so killing the group at
# the end takes the daemon with it.
SCRIPT = """
python3 "$1" "$2" "$3" &
sleep 0.5
exec timeout --foreground -k 2 120 "$4" --instance mpris --socket "$5" --config "$6" daemon
"""


class Ui:
    """A UI on the daemon's socket: reads its messages, sends events."""

    def __init__(self, path):
        deadline = time.time() + WAIT
        while True:
            try:
                self.sock = socket.socket(socket.AF_UNIX)
                self.sock.connect(path)
                break
            except OSError:
                self.sock.close()
                if time.time() > deadline:
                    raise
                time.sleep(0.05)
        self.sock.settimeout(0.2)
        self.buf = b""
        self.seen = []
        self.send({
            "t": "hello", "proto": 1, "client": "mpris-test", "clientVersion": "0", "instance": "mpris",
            "caps": {"primitives": [], "surfaces": [], "webview": False, "canvas": False},
        })

    def send(self, msg):
        self.sock.sendall((json.dumps(msg) + "\n").encode())

    def messages(self, limit):
        """What arrives within `limit` seconds, as it comes."""
        deadline = time.time() + limit
        while time.time() < deadline:
            while b"\n" in self.buf:
                line, self.buf = self.buf.split(b"\n", 1)
                yield json.loads(line)
            try:
                data = self.sock.recv(65536)
            except socket.timeout:
                continue
            if not data:
                return
            self.buf += data

    def wait_for(self, pred, what):
        """The first message that matches, among those already seen and then
        among those that arrive."""
        for m in self.seen:
            if pred(m):
                return m
        for m in self.messages(WAIT):
            self.seen.append(m)
            if pred(m):
                return m
        raise AssertionError("never saw: " + what)

    def close(self):
        self.sock.close()


def texts(tree):
    """Every text of a view tree."""
    out = []
    if isinstance(tree, dict):
        if tree.get("t") == "text":
            out.append(tree["text"])
        for child in tree.get("children", []):
            out += texts(child)
    return out


def find(tree, kind):
    """Every node of a kind in a view tree."""
    out = []
    if isinstance(tree, dict):
        if tree.get("t") == kind:
            out.append(tree)
        for child in tree.get("children", []):
            out += find(child, kind)
        if isinstance(tree.get("fallback"), dict):
            out += find(tree["fallback"], kind)
    return out


class Mpris(unittest.TestCase):
    tier = "integration"

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="nd-mpris-")
        os.mkdir(os.path.join(self.dir, "run"), 0o700)
        self.cover = os.path.join(self.dir, "cover.png")
        with open(self.cover, "wb") as f:
            f.write(b"not a real picture: the daemon only passes the path on")
        self.calls = os.path.join(self.dir, "calls")
        self.config = os.path.join(self.dir, "config.json")
        with open(self.config, "w") as f:
            json.dump({"modules": ["media"]}, f)
        self.socket = os.path.join(self.dir, "s.sock")
        env = dict(os.environ)
        for key, sub in (("XDG_STATE_HOME", "state"), ("XDG_DATA_HOME", "data"), ("XDG_CACHE_HOME", "cache"),
                         ("XDG_CONFIG_HOME", "config"), ("HOME", "home")):
            env[key] = os.path.join(self.dir, sub)
            os.mkdir(env[key])
        env.pop("NOTCHD_TEST_HOOKS", None)
        self.proc = subprocess.Popen(
            [ISOLATED, os.path.join(self.dir, "run"), "bash", "-c", SCRIPT, "_",
             PLAYER, self.cover, self.calls, DAEMON, self.socket, self.config],
            env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        self.ui = None

    def tearDown(self):
        if self.ui:
            self.ui.close()
        try:
            os.killpg(self.proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.proc.wait()
        # dbus-run-session's own bus daemon is in the group too.
        shutil.rmtree(self.dir, ignore_errors=True)

    def connect(self):
        self.ui = Ui(self.socket)
        return self.ui

    def view(self, ui, slot, pred=lambda tree: True):
        m = ui.wait_for(
            lambda m: m.get("t") == "view" and m["module"] == "media" and m["slot"] == slot
            and m.get("tree") is not None and pred(m["tree"]),
            "a media %s view" % slot)
        return m["tree"]

    def called(self):
        try:
            with open(self.calls) as f:
                return f.read().split("\n")[:-1]
        except OSError:
            return []

    def wait_called(self, line):
        deadline = time.time() + WAIT
        while time.time() < deadline:
            if line in self.called():
                return
            time.sleep(0.05)
        self.fail("the player never got %r; it got %r" % (line, self.called()))

    def test_the_demo_players_track_reaches_the_views(self):
        ui = self.connect()
        compact = self.view(ui, "compact", lambda t: "Night Drive" in texts(t))
        self.assertIn(" · Neon Harbor", texts(compact))
        images = find(compact, "image")
        self.assertEqual(images[0]["src"], "file://" + self.cover, "the cover is the player's art path")
        self.assertTrue(find(compact, "equalizer")[0]["active"])
        module = ui.wait_for(lambda m: m.get("t") == "module" and m["id"] == "media" and m["active"], "the module, active")
        self.assertEqual((module["title"], module["priority"], module["active"]), ("Now playing", 50, True))
        expanded = self.view(ui, "expanded")
        bar = find(expanded, "progress")[0]
        self.assertAlmostEqual(bar["value"], 92 / 214, places=3)
        self.assertAlmostEqual(bar["anchor"]["rate"], 1 / 214, places=6)
        self.assertIn("notchd demo", texts(expanded))

    def test_the_controls_call_the_player_and_its_answer_comes_back(self):
        ui = self.connect()
        expanded = self.view(ui, "expanded")
        self.assertTrue(find(expanded, "button")[1].get("big"))
        ui.send({"t": "event", "module": "media", "slot": "expanded", "action": "toggle", "value": None})
        self.wait_called("PlayPause")
        paused = self.view(ui, "expanded", lambda t: "anchor" not in find(t, "progress")[0])
        self.assertNotIn("anchor", find(paused, "progress")[0], "paused: the bar stands still")
        for action, call in (("next", "Next"), ("previous", "Previous")):
            ui.send({"t": "event", "module": "media", "slot": "expanded", "action": action, "value": None})
            self.wait_called(call)
        ui.send({"t": "event", "module": "media", "slot": "expanded", "action": "seek", "value": 0.5})
        self.wait_called("SetPosition %d" % (107 * 1000000))


if __name__ == "__main__":
    unittest.main()
