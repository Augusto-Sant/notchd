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
import shlex
import http.server
import shutil
import signal
import struct
import subprocess
import sys
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
        self._button_down = False
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

    def button(self, pressed):
        if self._button_down == pressed:
            return
        self.call("PointerButton", (pressed,), "(b)")
        self._button_down = pressed

    def click(self):
        self.call("PointerClick")

    def scroll(self):
        self.call("PointerScroll")

    def key(self, keyval):
        self.call("Key", (keyval,), "(u)")

    def keybinding_action(self, keycode, mask):
        return self.call("KeybindingAction", (keycode, mask), "(uu)")[0]

    def screenshot(self, name):
        os.makedirs(SHOTS, exist_ok=True)
        path = os.path.join(SHOTS, name + ".png")
        ok = self.call("Screenshot", (path,), "(s)")[0]
        return path if ok else None

    def scale_monitors(self, scales, count=2):
        """Apply supported logical scales to the two headless monitors."""
        name = "org.gnome.Mutter.DisplayConfig"
        path = "/org/gnome/Mutter/DisplayConfig"
        state = self.bus.call_sync(name, path, name, "GetCurrentState", None, None,
                                   Gio.DBusCallFlags.NONE, 5000, None).unpack()
        logical = []
        x = 0
        for i, (info, modes, _) in enumerate(state[1]):
            if i >= count:
                break
            mode = next((m for m in modes if m[-1].get("is-current")), modes[0])
            scale = scales.get(i, 1.0)
            supported = next((s for s in mode[5] if abs(s - scale) < 0.01), None)
            if supported is None:
                raise unittest.SkipTest("monitor %s does not support %s scale" % (info[0], scale))
            logical.append((x, 0, supported, 0, i == 0, [(info[0], mode[0], {})]))
            x += round(mode[1] / supported)
        params = GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})", (state[0], 1, logical, {}))
        self.bus.call_sync(name, path, name, "ApplyMonitorsConfig", params, None,
                           Gio.DBusCallFlags.NONE, 5000, None)
        # GNOME 46 keeps physical stage coordinates at 2x; newer Mutter
        # reports the logical monitor width here. Accept either convention.
        return self.until(lambda s: len(s.get("monitors", [])) == len(logical) and
                          all(any(abs(s["monitors"][i]["w"] - expected) <= 1 for expected in
                                  (state[1][i][1][0][1], round(state[1][i][1][0][1] / row[2])))
                              for i, row in enumerate(logical)), 5, "scaled monitors")

    def ipc(self, *args):
        env = dict(os.environ, NOTCHD_SOCKET=os.path.join(self.scratch, "notchd.sock"))
        return subprocess.run(["timeout", "-k", "2", "10", NOTCHD, "ipc", *args], env=env,
                              capture_output=True, text=True).stdout.strip()

    def extensions(self, verb):
        # The shell's own session: GNOME 46's gnome-extensions edits the
        # `enabled-extensions` setting itself (not over D-Bus, as 51 does)
        # and reads it from the config dir of the user it runs as, so it
        # needs the shell's XDG dirs (gnome-dev.sh) to see the shell's value.
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=self._address(),
                   XDG_RUNTIME_DIR=os.path.join(self.scratch, "run"), XDG_CONFIG_HOME=os.path.join(self.scratch, "config"),
                   XDG_DATA_HOME=os.path.join(self.scratch, "data"), XDG_CACHE_HOME=os.path.join(self.scratch, "cache"))
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
    def test_fullscreen_hides_the_closed_notch_but_not_an_open_one(self):
        sh = self.shell
        env = dict(os.environ, WAYLAND_DISPLAY="wayland-notchd", GDK_BACKEND="wayland",
                   XDG_RUNTIME_DIR=os.path.join(sh.scratch, "run"),
                   DBUS_SESSION_BUS_ADDRESS=sh._address())
        script = ("import gi; gi.require_version('Gtk', '3.0'); "
                  "from gi.repository import Gtk; "
                  "w = Gtk.Window(); w.set_title('notchd fullscreen test'); "
                  "w.fullscreen(); w.show_all(); Gtk.main()")
        app = subprocess.Popen([sys.executable, "-c", script], env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            sh.until(lambda s: not main_copy(s)["seen"], 8, "the closed notch to hide over fullscreen")
            sh.ipc("open")
            sh.until(lambda s: main_copy(s)["mode"] == "expanded" and main_copy(s)["seen"], 3,
                     "the open notch to stay available over fullscreen")
            sh.ipc("close")
            sh.until(lambda s: not main_copy(s)["seen"], 3, "the closed notch to hide again")
        finally:
            app.terminate()
            try:
                app.wait(3)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait()
            output = app.stdout.read().decode(errors="replace")
            app.stdout.close()
            if app.returncode not in (0, -signal.SIGTERM):
                self.fail("fullscreen test window failed: " + output)
        sh.until(lambda s: main_copy(s)["seen"], 5, "the notch after leaving fullscreen")

    def test_a_closed_notch_is_hidden_on_a_disallowed_workspace(self):
        sh = self.shell
        try:
            sh.ipc("set", "workspaces", "[2]")
            sh.until(lambda s: not main_copy(s)["seen"] and not s["struts"], 3,
                     "the closed notch and its reserve to hide outside workspace 2")
        finally:
            sh.ipc("set", "workspaces", "")
        sh.until(lambda s: main_copy(s)["seen"] and s["struts"], 3,
                 "the closed notch and its reserve to return on every workspace")

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

    # Bug: the notch was Adwaita's #1e1e1e (or #fafafa with the light
    # scheme) under a top bar of another color: it read as a patch stuck to
    # the bar. It takes the bar's colors now, a Shell theme's too.
    def test_the_notch_takes_the_top_bars_colors(self):
        sh = self.shell

        def same(s):
            bar = s["panelBackground"]
            return bar and bar[3] >= 128 and s["theme"]["background"] == "rgba(%d,%d,%d,1)" % tuple(bar[:3])
        s = sh.until(same, 3, "the notch's background to be the bar's")
        self.assertIsNotNone(s["theme"]["panel"], "no colors read from the bar")
        css = os.path.join(sh.scratch, "bar.css")
        with open(css, "w") as f:
            f.write("#panel { background-color: #2a3b4c; } #panel .panel-button { color: #d0e0f0; }")
        try:
            sh.call("PanelStyle", (css,), "(s)")
            s = sh.until(lambda x: x["theme"]["background"] == "rgba(42,59,76,1)", 3,
                         "the notch to follow a Shell theme's bar")
            self.assertEqual(s["theme"]["foreground"], "rgba(208,224,240,1)")
        finally:
            sh.call("PanelStyle", ("",), "(s)")
        sh.until(same, 3, "the notch back on the default bar's background")

    def test_reserve_keeps_windows_below_the_compact_notch(self):
        s = self.shell.until(lambda x: x["struts"], 3, "a strut")
        c = main_copy(s)
        self.assertGreaterEqual(s["workArea"][0]["y"], s["panel"] + c["card"]["depth"])
        # A notch that follows focus may hang on either monitor: the daemon
        # reserves on both, the second one has no panel.
        self.assertGreaterEqual(s["workArea"][1]["y"], s["monitors"][1]["y"] + c["card"]["depth"])

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

class OtherEdges(ShellCase):
    """A notch the daemon puts on another edge hangs from it: the card
    against that edge, centered along it, and the strut on that side."""

    def place(self, edge):
        sh = self.shell
        sh.ipc("set", "position", json.dumps({"edge": edge, "offset": 0.5}))
        s = sh.until(lambda x: main_copy(x)["edge"] == edge and main_copy(x)["mode"] == "compact"
                     and x["struts"] and main_copy(x)["card"]["depth"] > 0, 3, "the notch on the %s edge" % edge)
        time.sleep(0.6)  # past the card's spring
        return sh.snapshot()

    def tearDown(self):
        self.shell.ipc("set", "position", "")
        self.shell.until(lambda x: main_copy(x)["edge"] == "top", 3)

    def test_each_edge_holds_the_card_and_its_strut(self):
        for edge in ("left", "bottom", "right"):
            with self.subTest(edge=edge):
                s = self.place(edge)
                c, m, work = main_copy(s), s["monitors"][c_monitor(s)], s["workArea"][c_monitor(s)]
                depth = c["card"]["depth"]
                if edge == "left":
                    self.assertEqual(c["x"], m["x"])
                    self.assertGreater(c["h"], c["w"], "the card runs down the edge")
                    self.assertGreaterEqual(work["x"], m["x"] + depth)
                elif edge == "right":
                    self.assertEqual(c["x"] + c["w"], m["x"] + m["w"])
                    self.assertGreater(c["h"], c["w"])
                    self.assertLessEqual(work["x"] + work["w"], m["x"] + m["w"] - depth)
                else:
                    self.assertEqual(c["y"] + c["h"], m["y"] + m["h"])
                    self.assertAlmostEqual(c["x"] + c["w"] / 2, m["x"] + m["w"] / 2, delta=1)
                    self.assertLessEqual(work["y"] + work["h"], m["y"] + m["h"] - depth)
                if edge != "bottom":
                    # Along a side edge, centered in what the panel leaves.
                    top = m["y"] + (s["panel"] if c_monitor(s) == 0 else 0)
                    self.assertAlmostEqual(c["y"] + c["h"] / 2, (top + m["y"] + m["h"]) / 2, delta=1)
                self.shell.screenshot("edge-" + edge)


class PackedNotches(ShellCase):
    config = {"reserveSpace": True, "notches": [
        {"id": "main", "modules": ["resources"], "position": {"edge": "top", "offset": 0.49}},
        {"id": "second", "modules": ["resources"], "position": {"edge": "top", "offset": 0.51}},
    ]}

    def test_nearby_notches_share_an_edge_without_overlapping(self):
        s = self.shell.until(lambda s: len(s["copies"]) == 2 and
                             sorted(c["card"]["endSide"] for c in s["copies"]).count("joined") == 1, 5,
                             "the two notches to join")
        left, right = sorted(s["copies"], key=lambda c: c["card"]["start"])
        self.assertLessEqual(left["card"]["end"], right["card"]["start"] + 1)
        self.assertEqual(left["card"]["endSide"], "joined")
        self.assertEqual(right["card"]["startSide"], "joined")


class CornerNotches(ShellCase):
    config = {"reserveSpace": True, "notches": [
        {"id": "main", "modules": ["resources"], "position": {"edge": "top", "offset": 0}},
        {"id": "side", "modules": ["resources"], "position": {"edge": "left", "offset": 0}},
    ]}

    def test_a_side_notch_begins_after_the_top_corner(self):
        s = self.shell.until(lambda s: len(s["copies"]) == 2 and
                             all(c["card"]["startSide"] == "corner" for c in s["copies"]), 5,
                             "the two corner outlines to meet")
        top = next(c for c in s["copies"] if c["edge"] == "top")
        side = next(c for c in s["copies"] if c["edge"] == "left")
        self.assertEqual(top["card"]["startSide"], "corner")
        self.assertEqual(side["card"]["startSide"], "corner")
        self.assertGreaterEqual(side["card"]["start"], top["y"] + top["card"]["depth"] - 1)


def c_monitor(s):
    return main_copy(s)["monitor"]


class Dock(ShellCase):
    """A dock (the Ubuntu Dock, Dash to Dock) on any edge of the primary
    monitor: a notch hangs past it and runs beside it, and its reserve adds
    to the dock's. A fake one here: a chrome actor that reserves its room,
    as a fixed dock does."""

    DOCK = 64

    def tearDown(self):
        sh = self.shell
        sh.call("FakeDock", ("", 0), "(su)")
        sh.ipc("set", "position", "")
        sh.until(lambda s: main_copy(s)["edge"] == "top", 3)

    def dock(self, edge):
        self.shell.call("FakeDock", (edge, self.DOCK), "(su)")

    def place(self, edge, offset=0.5):
        sh = self.shell
        sh.ipc("set", "position", json.dumps({"edge": edge, "offset": offset, "screen": sh.snapshot()["screens"][0]}))
        sh.until(lambda s: main_copy(s)["edge"] == edge and main_copy(s)["monitor"] == 0, 3, "the notch on " + edge)
        time.sleep(0.6)  # past the spring
        return sh.snapshot()

    def test_a_notch_on_the_docks_edge_hangs_past_it_and_reserves_past_it(self):
        for edge in ("left", "bottom", "right"):
            with self.subTest(edge=edge):
                self.dock(edge)
                s = self.place(edge)
                c, m = main_copy(s), s["monitors"][0]
                s = self.shell.until(lambda s: s["struts"], 3, "the reserve")
                work = s["workArea"][0]
                depth = c["card"]["depth"]
                if edge == "left":
                    self.assertEqual(c["x"], m["x"] + self.DOCK)
                    self.assertGreaterEqual(work["x"], m["x"] + self.DOCK + depth)
                elif edge == "right":
                    self.assertEqual(c["x"] + c["w"], m["x"] + m["w"] - self.DOCK)
                    self.assertLessEqual(work["x"] + work["w"], m["x"] + m["w"] - self.DOCK - depth)
                else:
                    self.assertEqual(c["y"] + c["h"], m["y"] + m["h"] - self.DOCK)
                    self.assertLessEqual(work["y"] + work["h"], m["y"] + m["h"] - self.DOCK - depth)
                self.shell.screenshot("dock-" + edge)

    def test_a_notch_along_the_top_runs_beside_a_side_dock(self):
        self.dock("left")
        s = self.place("top", 0.0)
        c, m = main_copy(s), s["monitors"][0]
        self.assertGreaterEqual(c["card"]["start"], m["x"] + self.DOCK)
        self.assertEqual(c["y"], m["y"] + s["panel"])


class Drag(ShellCase):
    def tearDown(self):
        self.shell.button(False)
        self.shell.ipc("set", "position", "")
        self.shell.until(lambda x: main_copy(x)["edge"] == "top", 3)

    def test_a_held_press_drags_the_notch_to_the_edge_it_is_dropped_on(self):
        sh = self.shell
        s = sh.snapshot()
        m = s["monitors"][c_monitor(s)]
        x, y = center(main_copy(s))
        sh.move(x, y)
        time.sleep(0.3)
        sh.button(True)
        time.sleep(0.5)  # past the hold
        sh.until(lambda s: main_copy(s)["dragging"], 2, "the press to pick the notch up")
        # Near the left edge, a quarter of the way down: the preview jumps
        # there before the drop.
        sh.move(m["x"] + 5, m["y"] + m["h"] * 0.25)
        sh.until(lambda s: main_copy(s)["edge"] == "left", 2, "the preview on the left edge")
        sh.button(False)
        sh.until(lambda s: not main_copy(s)["dragging"], 2, "the release to drop it")
        # Past the second the drop's preview is held at most: what shows now
        # is the daemon's state.
        time.sleep(1.3)
        c = main_copy(sh.snapshot())
        self.assertEqual(c["edge"], "left")
        self.assertEqual(c["x"], m["x"])
        self.assertLess(c["y"] + c["h"] / 2, m["y"] + m["h"] / 2, "dropped in the upper half")

    def test_a_quick_click_does_not_drag(self):
        sh = self.shell
        x, y = center(main_copy(sh.snapshot()))
        sh.move(x, y)
        sh.until(lambda s: main_copy(s)["mode"] == "expanded", 2)
        sh.click()
        sh.until(lambda s: grabs(s) > 0, 2, "the click to pin it")
        self.assertFalse(main_copy(sh.snapshot())["dragging"])
        self.assertEqual(main_copy(sh.snapshot())["edge"], "top")


def copy_named(s, prefix):
    return next((c for c in s["copies"] if c["id"].startswith(prefix)), None)


class Guests(ShellCase):
    """The guest notches of `state`: a standalone module's own (the test
    module `pinger`, tests/gnome/guest-modules, peeks every 3 s) and the
    visitor (a peek on a screen where no notch has the module). Unseen while
    closed; while one shows, the notch it overlaps on its edge hides."""

    config = {"modules": ["timer"], "reserveSpace": True,
              "notches": [{}, {"id": "far", "modules": ["activities"], "position": {"screen": "Meta-1"}}]}
    modules = os.path.join(ROOT, "tests", "gnome", "guest-modules")

    def test_a_standalone_module_shows_its_own_notch_only_while_it_peeks(self):
        sh = self.shell
        s = sh.until(lambda s: (copy_named(s, "standalone:pinger@") or {}).get("seen")
                     and copy_named(s, "standalone:pinger@")["mode"] == "peek", 5, "the module's own notch peeking")
        pinger, main = copy_named(s, "standalone:pinger@"), main_copy(s)
        self.assertIn("pinger peeks", pinger["texts"])
        self.assertTrue(main["covered"] and not main["seen"], "the notch under it hides")
        s = sh.until(lambda s: not copy_named(s, "standalone:pinger@")["seen"], 4, "the peek over")
        self.assertTrue(main_copy(s)["seen"])

    def test_the_visitor_shows_a_peek_where_no_notch_has_the_module(self):
        sh = self.shell
        sh.move(400, 400)
        self.assertEqual(sh.ipc("push", json.dumps({"id": "guest-test", "title": "Visiting", "progress": 40})), "ok")
        s = sh.until(lambda s: (copy_named(s, "visitor@") or {}).get("seen")
                     and "Visiting" in copy_named(s, "visitor@")["texts"], 3, "the visitor showing the peek")
        visitor = next(c for c in s["copies"] if c["id"].startswith("visitor@") and c["seen"])
        self.assertEqual(visitor["monitor"], 0, "on the focused screen, where no notch has activities")
        sh.ipc("clear", "guest-test")
        sh.until(lambda s: not any(c["seen"] for c in s["copies"] if c["id"].startswith("visitor@")), 6,
                 "the visitor gone with the peek")


class Canvas(ShellCase):
    """The canvas primitive (a terminal's mark, a `>_` in a rounded tile, and
    its working dot in the closed notch) drawn with Cairo, not a
    placeholder."""

    config = {"modules": ["timer", "shell"], "reserveSpace": True,
              "terminals": {"shell": {"command": "exec sh", "title": "Shell"}}}

    def test_a_terminals_mark_draws_as_a_canvas(self):
        s = self.shell.until(lambda s: main_copy(s)["canvases"], 3, "a canvas in the closed notch")
        c = main_copy(s)
        self.assertFalse([t for t in c["texts"] if "not drawn" in t], c["texts"])
        self.assertTrue(all(cv["w"] > 0 and cv["h"] > 0 for cv in c["canvases"]), c["canvases"])
        # The mark has its commands (the dot has none while idle).
        self.assertTrue(any(cv["cmds"] >= 2 for cv in c["canvases"]), c["canvases"])
        self.shell.screenshot("canvas")


class Scroll(ShellCase):
    config = {"modules": ["tall"]}
    modules = os.path.join(ROOT, "tests", "gnome", "scroll-modules")

    def test_expanded_content_taller_than_the_screen_has_a_scrollbar(self):
        sh = self.shell
        sh.ipc("open")
        s = sh.until(lambda s: main_copy(s)["mode"] == "expanded" and
                     (main_copy(s).get("scroll") or {}).get("upper", 0) >
                     (main_copy(s).get("scroll") or {}).get("pageSize", 0) + 100,
                     5, "the tall view to overflow its scroll area")
        c = main_copy(s)
        self.assertLessEqual(c["scroll"]["height"], s["monitors"][c["monitor"]]["h"] - 64)
        self.assertIn("row 69", c["texts"])
        sh.screenshot("expanded-scroll")
        sh.move(*center(c))
        sh.scroll()
        sh.until(lambda s: (main_copy(s).get("scroll") or {}).get("value", 0) > 0,
                 2, "the wheel to scroll the expanded view")


def terminal_surface(s):
    return next(iter(s.get("terminals") or []), None)


class Terminal(ShellCase):
    """The daemon's grid is rendered by a Shell actor inside the notch."""

    config = {"modules": ["timer", "shell"], "reserveSpace": True,
              "shell": {"font": "DejaVu Sans Mono"},
              "terminals": {"shell": {"command": "exec sh", "title": "Shell"}}}
    env = {"NOTCHD_GNOME_FRACTIONAL": "1"}

    def tearDown(self):
        sh = self.shell
        sh.ipc("close")
        sh.ipc("set", "position", "")
        sh.until(lambda s: main_copy(s)["mode"] == "compact" and main_copy(s)["edge"] == "top", 3)

    def open_terminal(self):
        sh = self.shell
        self.assertEqual(sh.ipc("terminal", "shell", "toggle"), "ok")
        return sh.until(lambda s: main_copy(s)["terminal"] and terminal_surface(s)
                        and terminal_surface(s)["mapped"] and terminal_surface(s)["rows"] > 0,
                        6, "the grid rendered inside the notch")

    def test_grid_draws_inside_the_notch_and_has_the_keyboard(self):
        sh = self.shell
        s = self.open_terminal()
        c, terminal = main_copy(s), terminal_surface(s)
        self.assertTrue(c["terminal"]["embedded"])
        self.assertTrue(terminal["focused"])
        self.assertEqual(terminal["font"], "DejaVu Sans Mono")
        self.assertEqual(grabs(s), 1)
        self.assertGreater(terminal["w"], 0)
        self.assertGreater(terminal["h"], 0)
        self.assertEqual([t for t in s["visibleWindows"] if t.startswith("notchd term:")], [])
        sh.screenshot("terminal")

    def test_closing_destroys_the_actor_and_releases_the_keyboard(self):
        sh = self.shell
        self.open_terminal()
        sh.ipc("close")
        s = sh.until(lambda s: main_copy(s)["mode"] == "compact" and not s["terminals"], 3,
                     "the actor gone with the notch")
        self.assertIsNone(main_copy(s)["terminal"])
        self.assertEqual(grabs(s), 0)

    def test_keys_reach_the_pty_and_the_grid_shows_its_output(self):
        sh = self.shell
        self.open_terminal()
        for char in "echo notchd46":
            sh.key(ord(char))
        sh.key(0xFF0D)  # Return
        sh.until(lambda s: terminal_surface(s) and terminal_surface(s)["text"].count("notchd46") >= 2,
                 4, "the command and its output in the terminal grid")

    def test_font_setting_updates_the_live_grid(self):
        sh = self.shell
        self.open_terminal()
        try:
            sh.ipc("set", "shell.font", json.dumps("Monospace"))
            sh.until(lambda s: terminal_surface(s) and terminal_surface(s)["font"] == "Monospace",
                     3, "the new terminal font")
        finally:
            sh.ipc("set", "shell.font", json.dumps("DejaVu Sans Mono"))

    def test_fractional_scales_keep_the_terminal_inside_the_monitor(self):
        sh = self.shell
        try:
            self.open_terminal()
            for scale in (1.25, 4 / 3, 1.5, 5 / 3, 2.0):
                with self.subTest(scale=scale):
                    # GNOME 46 cannot place differently scaled virtual
                    # monitors adjacent; test the scaled primary alone.
                    sh.scale_monitors({0: scale}, count=1)
                    s = sh.until(lambda s: terminal_surface(s) and terminal_surface(s)["mapped"] and
                                 main_copy(s)["monitor"] == 0, 5)
                    c, m, terminal = main_copy(s), s["monitors"][0], terminal_surface(s)
                    self.assertGreaterEqual(c["x"], m["x"])
                    self.assertLessEqual(c["x"] + c["w"], m["x"] + m["w"])
                    self.assertGreater(terminal["w"], c["w"] * 0.8)

            with self.subTest(scale=1.6666667):
                sh.ipc("close")
                sh.scale_monitors({1: 1.6666667})
                sh.ipc("set", "position", json.dumps({"edge": "top", "offset": 0.5,
                                                       "screen": sh.snapshot()["screens"][1]}))
                sh.until(lambda s: main_copy(s)["monitor"] == 1, 5)
                s = self.open_terminal()
                c, m, terminal = main_copy(s), s["monitors"][1], terminal_surface(s)
                self.assertGreaterEqual(c["x"], m["x"])
                self.assertLessEqual(c["x"] + c["w"], m["x"] + m["w"])
                self.assertGreater(terminal["w"], c["w"] * 0.8)
                self.assertGreater(terminal["rows"], 0)
                time.sleep(0.6)  # capture the card after its opening animation
                sh.screenshot("terminal-fractional-scale")
        finally:
            sh.ipc("close")
            sh.ipc("set", "position", "")
            sh.scale_monitors({})

    def test_restart_replaces_the_grid_without_a_window(self):
        sh = self.shell
        first = self.open_terminal()
        previous_run = terminal_surface(first)["run"]
        self.assertEqual(sh.ipc("terminal", "shell", "new"), "ok")
        s = sh.until(lambda s: terminal_surface(s) and terminal_surface(s)["run"] != previous_run
                     and terminal_surface(s)["rows"] > 0, 6, "the new run's grid")
        self.assertEqual([t for t in s["visibleWindows"] if t.startswith("notchd term:")], [])

    def test_closing_and_reopening_reattaches_to_the_same_session(self):
        sh = self.shell
        first = self.open_terminal()
        run = terminal_surface(first)["run"]
        for char in "echo kept46":
            sh.key(ord(char))
        sh.key(0xFF0D)
        sh.until(lambda s: terminal_surface(s) and terminal_surface(s)["text"].count("kept46") >= 2, 4)
        sh.ipc("close")
        sh.until(lambda s: not s["terminals"], 3)
        self.assertEqual(sh.ipc("terminal", "shell", "toggle"), "ok")
        sh.until(lambda s: terminal_surface(s) and terminal_surface(s)["run"] == run
                 and "kept46" in terminal_surface(s)["text"], 6,
                 "the same PTY session replayed into a new actor")

    def test_side_edge_with_reserve_keeps_the_grid_inside_the_notch(self):
        sh = self.shell
        sh.ipc("set", "position", json.dumps({"edge": "left", "offset": 0.5}))
        sh.until(lambda s: main_copy(s)["edge"] == "left" and s["workArea"][main_copy(s)["monitor"]]["x"] > 0, 3)
        s = self.open_terminal()
        self.assertTrue(terminal_surface(s)["mapped"])
        self.assertGreater(terminal_surface(s)["rows"], 0)


class TerminalImage(ShellCase):
    config = {"modules": ["image-term"], "terminals": {"image-term": {
        "command": "python3 " + shlex.quote(os.path.join(ROOT, "tests", "gnome", "kitty_image.py")),
        "title": "Image terminal"}}}

    def test_a_kitty_png_is_decoded_and_placed_inside_the_terminal(self):
        sh = self.shell
        self.assertEqual(sh.ipc("terminal", "image-term", "toggle"), "ok")
        s = sh.until(lambda s: terminal_surface(s) and terminal_surface(s)["pictures"] > 0
                     and terminal_surface(s)["images"] > 0, 6, "the Kitty image in the terminal")
        self.assertTrue(main_copy(s)["terminal"]["embedded"])
        self.assertIsNotNone(sh.screenshot("terminal-image"))


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

class Preferences(ShellCase):
    def test_a_quick_close_cannot_be_undone_by_the_preferences_startup(self):
        sh = self.shell
        opened = sh.snapshot()["preferencesOpened"]
        sh.ipc("settings")
        sh.until(lambda s: "notchd" in s["windows"], 20, "the preferences window")
        sh.ipc("settings")
        sh.until(lambda s: not s["settingsOpen"] and "notchd" not in s["windows"], 5,
                 "the quickly closed preferences")
        time.sleep(0.4)
        s = sh.snapshot()
        self.assertFalse(s["settingsOpen"])
        self.assertNotIn("notchd", s["windows"])
        self.assertEqual(s["preferencesOpened"], opened + 1)

    def test_manual_preferences_open_uses_the_existing_window(self):
        sh = self.shell
        opened = sh.snapshot()["preferencesOpened"]
        sh.extensions("prefs")
        s = sh.until(lambda s: s["settingsOpen"] and "notchd" in s["windows"], 20,
                     "the independently opened preferences")
        self.assertEqual(s["preferencesOpened"], opened)
        sh.ipc("settings")
        sh.until(lambda s: not s["settingsOpen"] and "notchd" not in s["windows"], 5,
                 "the manually opened preferences to close")

    def test_the_daemon_opening_settings_opens_the_preferences_and_closing_them_closes_it(self):
        # `ipc settings` toggles the daemon's settingsOpen, as navigation's
        # `s` opens it: the extension opens its preferences (prefs.js, a
        # process of its own), which connect to the daemon; toggled again,
        # the window goes.
        sh = self.shell
        opened = sh.snapshot()["preferencesOpened"]
        sh.ipc("settings")
        s = sh.until(lambda s: "notchd" in s["windows"], 20, "the preferences window")
        self.assertEqual(s["preferencesOpened"], opened + 1)
        time.sleep(1)  # the window connects and draws the daemon's pages
        sh.screenshot("preferences")
        sh.ipc("settings")
        sh.until(lambda s: "notchd" not in s["windows"], 5, "the preferences window to close")
        time.sleep(0.3)  # a queued page report must not reopen it
        self.assertFalse(sh.snapshot()["settingsOpen"])
        self.assertEqual(sh.snapshot()["preferencesOpened"], opened + 1)

    def test_a_crashed_preferences_process_releases_settings_and_can_reopen(self):
        sh = self.shell
        sh.ipc("settings")
        sh.until(lambda s: s.get("settingsOpen") and "notchd" in s["windows"], 20,
                 "the preferences process to open")
        bus = ("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus")
        deadline = time.time() + 5
        while True:
            try:
                pid = sh.bus.call_sync(*bus, "GetConnectionUnixProcessID",
                                       GLib.Variant("(s)", ("org.notchd.GnomePreferences",)),
                                       None, Gio.DBusCallFlags.NONE, 1000, None).unpack()[0]
                break
            except GLib.Error:
                if time.time() > deadline:
                    raise
                time.sleep(0.05)
        os.kill(pid, signal.SIGKILL)
        sh.until(lambda s: not s.get("settingsOpen") and "notchd" not in s["windows"], 5,
                 "the daemon to release settings after the preferences crash")
        sh.ipc("settings")
        sh.until(lambda s: s.get("settingsOpen") and "notchd" in s["windows"]
                 and s["preferencesOpened"] >= 2, 20, "preferences to reopen")
        sh.ipc("settings")
        sh.until(lambda s: not s.get("settingsOpen") and "notchd" not in s["windows"], 5,
                 "settings and preferences to close")
        time.sleep(0.3)  # a late report from the closed process must not reopen them
        self.assertFalse(sh.snapshot()["settingsOpen"])


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
