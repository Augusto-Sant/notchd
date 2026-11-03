// Test hooks, exported on the session bus only when NOTCHD_GNOME_TEST is
// set (scripts/gnome-dev.sh --headless, tests/gnome_test.py). A test drives
// the shell through them: what the notch copies look like, a virtual
// pointer and keyboard inside the shell, a screenshot. Never loaded in a
// user's session.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const IFACE = `
<node>
  <interface name="org.notchd.GnomeTest">
    <method name="Snapshot"><arg type="s" direction="out" name="json"/></method>
    <method name="PointerMove"><arg type="d" direction="in" name="x"/><arg type="d" direction="in" name="y"/></method>
    <method name="PointerClick"/>
    <method name="PointerButton"><arg type="b" direction="in" name="pressed"/></method>
    <method name="Key"><arg type="u" direction="in" name="keyval"/></method>
    <method name="RunAction"><arg type="s" direction="in" name="name"/></method>
    <method name="KeybindingAction"><arg type="u" direction="in" name="keycode"/><arg type="u" direction="in" name="mask"/><arg type="u" direction="out" name="action"/></method>
    <method name="Screenshot"><arg type="s" direction="in" name="path"/><arg type="b" direction="out" name="ok"/></method>
  </interface>
</node>`;

export class TestHook {
    // `actions`: what the extension's keybindings run, by name (a headless
    // shell's virtual chords never reach them, so a test runs them here).
    // `snapshot()`: the extension's own description of what it shows.
    constructor(actions, snapshot) {
        this._actions = actions;
        this._snapshot = snapshot;
        // Clutter.get_default_backend() is gone in GNOME 51; the stage's
        // context has the backend there.
        const backend = global.stage.context?.get_backend?.() ?? Clutter.get_default_backend();
        const seat = backend.get_default_seat();
        this._pointer = seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
        this._keyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        this._impl = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._impl.export(Gio.DBus.session, '/org/notchd/GnomeTest');
        this._nameId = Gio.bus_own_name_on_connection(Gio.DBus.session, 'org.notchd.GnomeTest',
            Gio.BusNameOwnerFlags.NONE, null, null);
    }

    destroy() {
        this._impl.unexport();
        Gio.bus_unown_name(this._nameId);
        this._pointer = null;
        this._keyboard = null;
    }

    _now() {
        return GLib.get_monotonic_time();
    }

    Snapshot() {
        const monitors = Main.layoutManager.monitors.map(m => ({x: m.x, y: m.y, w: m.width, h: m.height}));
        const workArea = Main.layoutManager.monitors.map((m, i) => {
            const r = global.workspace_manager.get_active_workspace().get_work_area_for_monitor(i);
            return {x: r.x, y: r.y, w: r.width, h: r.height};
        });
        return JSON.stringify({monitors, workArea, panel: Main.panel.height, modal: Main.modalCount,
            overview: Main.overview.visible,
            // The chrome actors the extension added (all named notchd-*).
            chrome: (Main.layoutManager._trackedActors ?? []).filter(a => a.actor.name?.startsWith('notchd-')).length,
            // The windows' titles (the preferences window is "notchd").
            windows: global.get_window_actors().map(a => a.meta_window.get_title() ?? ''),
            // The windows one can see (shown, not transparent).
            visibleWindows: global.get_window_actors().filter(a => a.visible && a.opacity > 0)
                .map(a => a.meta_window.get_title() ?? ''),
            ...this._snapshot()});
    }

    PointerMove(x, y) {
        this._pointer.notify_absolute_motion(this._now(), x, y);
    }

    PointerClick() {
        const t = this._now();
        this._pointer.notify_button(t, Clutter.BUTTON_PRIMARY, Clutter.ButtonState.PRESSED);
        this._pointer.notify_button(t + 1000, Clutter.BUTTON_PRIMARY, Clutter.ButtonState.RELEASED);
    }

    // The primary button down or up alone: a test holds it to drag.
    PointerButton(pressed) {
        this._pointer.notify_button(this._now(), Clutter.BUTTON_PRIMARY,
            pressed ? Clutter.ButtonState.PRESSED : Clutter.ButtonState.RELEASED);
    }

    Key(keyval) {
        const t = this._now();
        this._keyboard.notify_keyval(t, keyval, Clutter.KeyState.PRESSED);
        this._keyboard.notify_keyval(t + 1000, keyval, Clutter.KeyState.RELEASED);
    }

    RunAction(name) {
        this._actions[name]?.();
    }

    // The keybinding action mutter would run for an X key code and a
    // modifier mask (0: none). The virtual keyboard's chords never reach
    // mutter's keybindings in a headless shell, so a test asks this instead.
    KeybindingAction(keycode, mask) {
        return global.display.get_keybinding_action(keycode, mask);
    }

    ScreenshotAsync(params, invocation) {
        const [path] = params;
        const shooter = new Shell.Screenshot();
        const file = Gio.File.new_for_path(path);
        const stream = file.replace(null, false, Gio.FileCreateFlags.NONE, null);
        shooter.screenshot(false, stream, (s, res) => {
            let ok = false;
            try {
                s.screenshot_finish(res);
                ok = true;
            } catch (e) {
                console.warn(`notchd test: screenshot failed: ${e.message}`);
            }
            stream.close(null);
            invocation.return_value(new GLib.Variant('(b)', [ok]));
        });
    }
}
