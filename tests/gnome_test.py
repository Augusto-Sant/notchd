#!/usr/bin/env python3
"""The GNOME Shell extension (hosts/gnome/) in a real, headless GNOME Shell.

Each class starts `scripts/gnome-dev.sh --headless` (gnome-shell with two
virtual monitors, a private D-Bus and runtime dir, its own notchd daemon and
config) and drives it through the extension's test hooks (testhook.js, on
the private bus): what the notch copies show and where, a virtual pointer and
keyboard inside the shell, screenshots. The daemon is the real one, so these
check the extension against the protocol as the daemon speaks it.

Not in `make check`: it needs gnome-shell installed. `make gnome-test`.

    python3 tests/gnome_test.py [-k name]
"""

import json
import os
import http.server
import shutil
import signal
import struct
import subprocess
import tempfile
import threading
import time
import unittest
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEV = os.path.join(ROOT, "scripts", "gnome-dev.sh")
NOTCHD = os.environ.get("NOTCHD_BIN", os.path.join(ROOT, "native", "target", "release", "notchd"))
SHOTS = os.path.join(ROOT, "build", "gnome")
UUID = "notchd@eryk-vieira.github.io"
KEY_ESCAPE = 0xFF1B
# The extension's default navigation key, Super+Alt+N: the X key code of N
# and the Mod4 (Super) and Mod1 (Alt) mask.
NAVIGATE_KEY = (57, (1 << 6) | (1 << 3))
SECOND_MONITOR = "1280x800"

try:
    import gi

    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib
except (ImportError, ValueError):  # pragma: no cover
    Gio = None


def missing():
    if Gio is None:
        return "python3-gi is not installed"
    if not shutil.which("gnome-shell"):
        return "gnome-shell is not installed"
    if not os.access(NOTCHD, os.X_OK):
        return "no daemon at %s: make daemon" % NOTCHD
    return None


class Shell:
    """One headless GNOME Shell with the extension and its daemon."""

    # A shell that works has started within 1-2 s.
    STARTED_WITHIN = 10

    def __init__(self, config, env=None, modules=None):
        # Short on purpose: mutter's Wayland socket path must fit 108 bytes.
        self.scratch = tempfile.mkdtemp(prefix="ngt.", dir="/tmp")
        config_path = os.path.join(self.scratch, "start.json")
        with open(config_path, "w") as f:
            json.dump(config, f)
        env = dict(os.environ, NOTCHD_GNOME_SCRATCH=self.scratch, NOTCHD_GNOME_CONFIG=config_path,
                   NOTCHD_GNOME_TEST="1", NOTCHD_BIN=NOTCHD, **(env or {}))
        if modules:
            env["NOTCHD_GNOME_MODULES"] = modules
        self.log_path = os.path.join(self.scratch, "shell.log")
        self.log = open(self.log_path, "w")
        self.proc = subprocess.Popen(
            ["timeout", "-k", "3", "300", DEV, "--headless", "--", "--virtual-monitor", SECOND_MONITOR],
            env=env, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        self.bus = None
        try:
            self._connect()
        except BaseException:
            # tearDownClass does not run when setUpClass fails.
            self.stop()
            raise

    def _connect(self):
        bus_file = os.path.join(self.scratch, "bus")
        deadline = time.time() + self.STARTED_WITHIN
        while time.time() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError("gnome-shell exited:\n" + self.tail())
            if os.path.exists(bus_file) and self._address():
                break
            time.sleep(0.1)
        while not self._started():
            if self.proc.poll() is not None:
                raise RuntimeError("gnome-shell exited with %s before starting:\n%s" % (self.proc.returncode, self.tail()))
            if time.time() > deadline:
                raise RuntimeError("gnome-shell has not started:\n" + self.tail())
            time.sleep(0.1)
        address = self._address()
        flags = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION
        self.bus = Gio.DBusConnection.new_for_address_sync(address, flags, None, None)
        self.until(lambda s: s.get("connected") and s["copies"], 20, "the extension connected and drew a notch")

    def _started(self):
        with open(self.log_path) as f:
            return "GNOME Shell started" in f.read()

    def _address(self):
        with open(os.path.join(self.scratch, "bus")) as f:
            return f.read().strip()

    def call(self, method, args=None, sig="()"):
        params = GLib.Variant(sig, args) if args is not None else None
        reply = self.bus.call_sync("org.notchd.GnomeTest", "/org/notchd/GnomeTest", "org.notchd.GnomeTest",
                                   method, params, None, Gio.DBusCallFlags.NONE, 5000, None)
        return reply.unpack() if reply is not None else None

    def snapshot(self):
        try:
            return json.loads(self.call("Snapshot")[0])
        except GLib.Error:
            return {}

    def until(self, pred, timeout=3, what="the condition"):
        deadline = time.time() + timeout
        last = {}
        while time.time() < deadline:
            last = self.snapshot()
            try:
                if last and pred(last):
                    return last
            except (KeyError, IndexError, TypeError):
                pass
            time.sleep(0.03)
        raise AssertionError("timed out waiting for %s; last snapshot:\n%s\nlog:\n%s"
                             % (what, json.dumps(last, indent=1), self.tail()))

    def move(self, x, y):
        self.call("PointerMove", (float(x), float(y)), "(dd)")

    def click(self):
        self.call("PointerClick")

    def key(self, keyval):
        self.call("Key", (keyval,), "(u)")

    def keybinding_action(self, keycode, mask):
        return self.call("KeybindingAction", (keycode, mask), "(uu)")[0]

    def screenshot(self, name):
        os.makedirs(SHOTS, exist_ok=True)
        path = os.path.join(SHOTS, name + ".png")
        ok = self.call("Screenshot", (path,), "(s)")[0]
        return path if ok else None

    def ipc(self, *args):
        env = dict(os.environ, NOTCHD_SOCKET=os.path.join(self.scratch, "notchd.sock"))
        return subprocess.run(["timeout", "-k", "2", "10", NOTCHD, "ipc", *args], env=env,
                              capture_output=True, text=True).stdout.strip()

    def extensions(self, verb):
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=self._address())
        subprocess.run(["timeout", "-k", "2", "10", "gnome-extensions", verb, UUID], env=env, check=True,
                       capture_output=True)

    def tail(self, n=40):
        try:
            with open(self.log_path) as f:
                lines = [l for l in f.read().splitlines() if "dbus-daemon" not in l and l.strip()]
            return "\n".join(lines[-n:])
        except OSError:
            return ""

    def js_errors(self):
        with open(self.log_path) as f:
            return [l for l in f.read().splitlines() if "JS ERROR" in l or "JS WARNING" in l]

    def stop(self):
        try:
            os.killpg(self.proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            os.killpg(self.proc.pid, signal.SIGKILL)
            self.proc.wait()
        # A detached daemon outlives its test: clean by socket path.
        subprocess.run(["pkill", "-f", "--", "--socket %s/" % self.scratch])
        # The document portal and gvfs mount FUSE filesystems in the
        # runtime dir; a killed session leaves them mounted.
        with open("/proc/mounts") as f:
            mounts = [l.split()[1] for l in f if l.split()[1].startswith(self.scratch + "/")]
        for m in mounts:
            subprocess.run(["fusermount3", "-u", "-z", m], capture_output=True)
        self.log.close()
        shutil.rmtree(self.scratch, ignore_errors=True)


def grabs(s):
    """The modal grabs that are the extension's: the shell's own overview
    (open at startup, or from the hot corner) is one too."""
    return s["modal"] - (1 if s.get("overview") else 0)


def main_copy(s):
    return next(c for c in s["copies"] if c["id"].startswith("main@"))


def center(c):
    return c["x"] + c["w"] / 2, c["y"] + max(c["h"], 1) / 2


class ShellCase(unittest.TestCase):
    tier = "gnome"
    config = {"modules": ["timer", "resources"], "reserveSpace": True}
    # Extra environment for the shell (and the daemon's modules), and a
    # folder of user modules for the daemon.
    env = {}
    modules = None

    @classmethod
    def setUpClass(cls):
        why = missing()
        if why:
            raise unittest.SkipTest(why)
        cls.shell = Shell(cls.config, cls.env, cls.modules)

    @classmethod
    def tearDownClass(cls):
        shell = getattr(cls, "shell", None)
        if shell:
            errors = shell.js_errors()
            shell.stop()
            if errors:
                raise AssertionError("JS errors in the shell log:\n" + "\n".join(errors))

    def setUp(self):
        # Every case starts with the pointer away, nothing pinned, no
        # overview (GNOME opens it at startup; it holds the pointer), and
        # the notch compact.
        s = self.shell
        s.move(10, 900)
        deadline = time.time() + 5
        while time.time() < deadline:
            snap = s.snapshot()
            if snap.get("modal") == 0 and not snap.get("overview"):
                break
            s.key(KEY_ESCAPE)
            time.sleep(0.3)
        s.until(lambda x: main_copy(x)["mode"] == "compact" and x["modal"] == 0 and not x["overview"], 3,
                "a compact notch, nothing modal")


class NotchOnGnome(ShellCase):
    def test_reports_both_monitors_by_connector(self):
        s = self.shell.snapshot()
        self.assertEqual(len(s["monitors"]), 2)
        self.assertEqual(len(s["screens"]), 2)
        for name in s["screens"]:
            self.assertFalse(name.startswith("monitor-"), "fell back to an index name: %s" % s["screens"])

    def test_the_notch_hangs_below_the_panel_centred_on_the_primary_monitor(self):
        s = self.shell.snapshot()
        c = main_copy(s)
        m = s["monitors"][0]
        self.assertEqual(c["y"], m["y"] + s["panel"])
        self.assertAlmostEqual(c["x"] + c["w"] / 2, m["x"] + m["w"] / 2, delta=1)
        self.assertTrue(c["hasContent"])

    def test_reserve_keeps_windows_below_the_compact_notch(self):
        s = self.shell.until(lambda x: x["struts"], 3, "a strut")
        c = main_copy(s)
        self.assertGreaterEqual(s["workArea"][0]["y"], s["panel"] + c["card"]["h"])
        # A notch that follows focus may hang on either monitor: the daemon
        # reserves on both, the second one has no panel.
        self.assertGreaterEqual(s["workArea"][1]["y"], s["monitors"][1]["y"] + c["card"]["h"])

    def test_hover_opens_within_the_delay_and_leaving_closes(self):
        sh = self.shell
        x, y = center(main_copy(sh.snapshot()))
        start = time.time()
        sh.move(x, y)
        sh.until(lambda s: main_copy(s)["mode"] == "expanded", 2, "the notch to open on hover")
        opened = time.time() - start
        # 150 ms delay; the round trip through the daemon and the polling
        # add a little.
        self.assertLess(opened, 0.6)
        time.sleep(0.4)  # past the card's easing
        shot = sh.screenshot("expanded")
        self.assertIsNotNone(shot)
        sh.move(10, 900)
        sh.until(lambda s: main_copy(s)["mode"] == "compact", 2, "the notch to close on leave")

    def test_a_click_pins_it_with_the_keyboard_and_escape_closes(self):
        sh = self.shell
        x, y = center(main_copy(sh.snapshot()))
        sh.move(x, y)
        sh.until(lambda s: main_copy(s)["mode"] == "expanded", 2)
        sh.click()
        sh.until(lambda s: main_copy(s)["modal"] and grabs(s) > 0, 2, "the pinned notch to take the keyboard")
        sh.key(KEY_ESCAPE)
        sh.until(lambda s: main_copy(s)["mode"] == "compact" and grabs(s) == 0, 2, "Escape to close it")

    def test_a_click_away_closes_a_pinned_notch(self):
        sh = self.shell
        x, y = center(main_copy(sh.snapshot()))
        sh.move(x, y)
        sh.until(lambda s: main_copy(s)["mode"] == "expanded", 2)
        sh.click()
        sh.until(lambda s: grabs(s) > 0, 2)
        sh.move(200, 700)
        sh.click()
        sh.until(lambda s: main_copy(s)["mode"] == "compact" and grabs(s) == 0, 2, "a click away to close it")

    def test_a_click_on_a_tab_shows_its_module(self):
        # GNOME 49+ St.Button: the press also reached the card, which pinned
        # it and took a grab that cancelled the tab's click.
        sh = self.shell
        x, y = center(main_copy(sh.snapshot()))
        sh.move(x, y)
        s = sh.until(lambda s: main_copy(s)["mode"] == "expanded" and len(main_copy(s)["tabs"]) == 2, 3,
                     "the open notch with a tab per module")
        other = next(t for t in main_copy(s)["tabs"] if not t["picked"])
        sh.move(*center(other))
        # The virtual pointer's motion lands before the press, or the press
        # goes where the pointer was.
        time.sleep(0.3)
        sh.click()
        sh.until(lambda s: main_copy(s)["shown"] == [other["id"]], 2, "the tab's module shown")

    def test_navigation_holds_the_keyboard_until_escape(self):
        sh = self.shell
        self.assertEqual(sh.ipc("navigate"), "ok")
        sh.until(lambda s: s["navigating"] and grabs(s) > 0, 2, "navigation to take the keyboard")
        sh.key(KEY_ESCAPE)
        sh.until(lambda s: not s["navigating"] and grabs(s) == 0, 2, "Escape to end navigation")

    def test_navigation_highlights_the_notch_and_its_key_is_bound(self):
        sh = self.shell
        # Super+Alt+N runs the extension's own action (a virtual chord never
        # reaches mutter's keybindings headless, so the binding is asked).
        action = sh.snapshot()["navigateAction"]
        self.assertGreater(action, 0)
        self.assertEqual(sh.keybinding_action(*NAVIGATE_KEY), action)
        # What the key runs starts navigation (an ipc with a string id was
        # dropped by the daemon), and leaves it again.
        sh.call("RunAction", ("navigate",), "(s)")
        s = sh.until(lambda s: s["navigating"] and main_copy(s)["navLabel"], 2, "the notch highlighted")
        label = main_copy(s)["navLabel"]
        # Its number and name, and as the selected (only) notch the keys.
        self.assertEqual(label[:2], ["1", "main"])
        self.assertIn("\u21b5 open", label[2])
        sh.call("RunAction", ("navigate",), "(s)")
        sh.until(lambda s: not s["navigating"] and main_copy(s)["navLabel"] is None, 2, "the key again to leave")

    def test_a_notch_placed_on_the_second_monitor_goes_there_with_no_panel(self):
        sh = self.shell
        second = sh.snapshot()["screens"][1]
        try:
            sh.ipc("set", "position", json.dumps({"edge": "top", "offset": 0.5, "screen": second}))
            s = sh.until(lambda x: main_copy(x)["monitor"] == 1, 3, "the notch on the second monitor")
            c, m = main_copy(s), s["monitors"][1]
            self.assertEqual(c["y"], m["y"])
            self.assertAlmostEqual(c["x"] + c["w"] / 2, m["x"] + m["w"] / 2, delta=1)
        finally:
            sh.ipc("set", "position", "")
            sh.until(lambda x: main_copy(x)["monitor"] == 0, 3)

class EnableDisable(ShellCase):
    def test_disable_takes_away_everything_enable_added(self):
        sh = self.shell
        sh.until(lambda s: s["struts"], 3)
        for _ in range(5):
            sh.extensions("disable")
            time.sleep(0.2)
            sh.extensions("enable")
            sh.until(lambda s: s.get("connected") and s["copies"] and s["struts"], 5, "the extension back")
            sh.move(10, 900)
        after = sh.snapshot()
        # Only what this enable added is there: nothing left from the four
        # before it.
        self.assertEqual(after["chrome"], len(after["copies"]) + len(after["struts"]))
        self.assertEqual(after["modal"], 0)


class DaemonGoesAway(ShellCase):
    def test_a_daemon_that_goes_away_leaves_no_grab_and_it_reconnects(self):
        sh = self.shell
        x, y = center(main_copy(sh.snapshot()))
        sh.move(x, y)
        sh.until(lambda s: main_copy(s)["mode"] == "expanded", 2)
        sh.click()
        sh.until(lambda s: grabs(s) > 0, 2)
        sh.ipc("shutdown")
        sh.until(lambda s: not s["connected"] and grabs(s) == 0, 3, "the grab to go with the daemon")

def solid_png(w, h, rgb):
    """A w x h picture of one color, as PNG bytes."""
    def chunk(kind, data):
        c = kind + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))
    row = b"\x00" + bytes(rgb) * w
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(row * h)) + chunk(b"IEND", b""))


class Pictures(http.server.ThreadingHTTPServer):
    """A server of test pictures on 127.0.0.1 (the tests never touch the
    network): /cover.png, and a 404 for anything else. `hits` counts the
    requests that were answered."""

    daemon_threads = True

    def __init__(self):
        body = solid_png(64, 64, (255, 0, 0))
        self.hits = []
        server = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                server.hits.append(self.path)
                ok = self.path == "/cover.png"
                self.send_response(200 if ok else 404)
                self.send_header("Content-Type", "image/png")
                self.send_header("Content-Length", str(len(body) if ok else 0))
                self.end_headers()
                if ok:
                    self.wfile.write(body)

            def log_message(self, *args):
                pass

        super().__init__(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.serve_forever, daemon=True).start()


class SeekAndPictures(ShellCase):
    """The media module's progress bar (anchor, seek) and cover (a picture
    from the network), as tests/gnome/test-modules/seekbars draws them."""

    config = {"modules": ["seekbars"]}
    modules = os.path.join(ROOT, "tests", "gnome", "test-modules")

    @classmethod
    def setUpClass(cls):
        cls.pictures = Pictures()
        port = cls.pictures.server_address[1]
        cls.env = {"NOTCHD_TEST_COVER": "http://127.0.0.1:%d/cover.png" % port,
                   "NOTCHD_TEST_MISSING": "http://127.0.0.1:%d/missing.png" % port}
        try:
            super().setUpClass()
        except BaseException:
            cls.pictures.shutdown()
            raise

    @classmethod
    def tearDownClass(cls):
        try:
            super().tearDownClass()
        finally:
            cls.pictures.shutdown()

    def open(self):
        sh = self.shell
        x, y = center(main_copy(sh.snapshot()))
        sh.move(x, y)
        s = sh.until(lambda s: main_copy(s)["mode"] == "expanded" and len(main_copy(s)["bars"]) == 3
                     and main_copy(s)["bars"][0]["w"] > 50, 4, "the open notch with the bars")
        time.sleep(0.5)  # past the card's easing
        return main_copy(sh.snapshot())

    def press(self, x, y):
        self.shell.move(x, y)
        time.sleep(0.3)  # the virtual pointer's motion lands before the press
        self.shell.click()

    def test_an_anchored_bar_moves_with_no_new_view(self):
        sh = self.shell
        bar = self.open()["bars"][0]
        self.assertTrue(bar["seek"])
        first = bar["shown"]
        # 5 % a second: past the card's own animations a new view cannot
        # explain it, the module sends none.
        sh.until(lambda s: main_copy(s)["bars"][0]["shown"] > first + 0.04, 4, "the bar to move on by its anchor")
        # A bar with no anchor stays where it is.
        self.assertAlmostEqual(main_copy(sh.snapshot())["bars"][1]["shown"], 0.5, delta=0.01)

    def test_a_press_on_a_bar_seeks_and_does_not_pin_the_notch(self):
        sh = self.shell
        bar = self.open()["bars"][0]
        self.press(bar["x"] + bar["w"] * 0.25, bar["y"] + bar["h"] / 2)
        s = sh.until(lambda s: "seek: 0.25" in main_copy(s)["texts"], 3, "the module to hear the seek at 25 %")
        # The press was the bar's: it did not reach the card, which would
        # have pinned the notch and taken a grab that cancels a drag.
        self.assertEqual(grabs(s), 0)
        self.assertEqual(main_copy(s)["mode"], "expanded")
        # The new view updated the bar in place: it still moves.
        shown = main_copy(s)["bars"][0]["shown"]
        sh.until(lambda s: main_copy(s)["bars"][0]["shown"] > shown + 0.02, 3, "the bar to keep moving after the update")

    def test_a_vertical_bar_seeks_from_the_bottom(self):
        sh = self.shell
        bar = self.open()["bars"][1]
        self.assertGreater(bar["h"], bar["w"])
        # 25 % from the top is 75 % from the bottom.
        self.press(bar["x"] + bar["w"] / 2, bar["y"] + bar["h"] * 0.25)
        sh.until(lambda s: "vseek: 0.75" in main_copy(s)["texts"], 3, "the module to hear the seek at 75 %")

    def test_a_picture_from_the_network_downloads_once_and_a_failure_keeps_the_fallback(self):
        sh = self.shell
        copy = self.open()
        covers = sh.until(lambda s: [i["state"] for i in main_copy(s)["images"]] == ["ready", "failed"], 6,
                          "the cover to load and the missing one to fail")
        sh.screenshot("seekbars")
        cache = os.path.join(sh.scratch, "cache", "notchd-gnome")
        self.assertEqual(len(os.listdir(cache)), 1)
        self.assertEqual(self.pictures.hits.count("/cover.png"), 1)
        # Closed and opened again: the picture is drawn again from the cache.
        sh.move(10, 900)
        sh.until(lambda s: main_copy(s)["mode"] == "compact", 3)
        self.open()
        sh.until(lambda s: [i["state"] for i in main_copy(s)["images"]][0] == "ready", 3)
        self.assertEqual(self.pictures.hits.count("/cover.png"), 1)


if __name__ == "__main__":
    unittest.main()
