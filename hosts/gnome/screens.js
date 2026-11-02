// The compositor adapter: GNOME's monitors, focus, fullscreen and
// workspaces as the `screens` message (docs/architecture.md, "UI to core").
// Reported, never decided: the daemon picks the screens the notch goes on.

import Gio from 'gi://Gio';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// Mutter's display configuration, served by gnome-shell itself: a call
// must be async, since a sync one waits on the main loop that answers it.
const DISPLAY_CONFIG = ['org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig'];

// Monitor connector names (DP-1, eDP-1), the names the config's
// `position.screen` uses, in Main.layoutManager.monitors' order. They come
// from DisplayConfig.GetCurrentState on every version (Meta's
// get_logical_monitors() is missing on 46): a logical monitor is matched to
// the layout manager's by its origin. `done(names)` gets `monitor-<i>` for
// a monitor it could not name. Returns a function that drops the answer.
export function queryConnectors(done) {
    let cancelled = false;
    const finish = byOrigin => {
        if (!cancelled)
            done(Main.layoutManager.monitors.map((m, i) => byOrigin[`${m.x},${m.y}`] ?? `monitor-${i}`));
    };
    Gio.DBus.session.call(DISPLAY_CONFIG[0], DISPLAY_CONFIG[1], DISPLAY_CONFIG[0], 'GetCurrentState',
        null, null, Gio.DBusCallFlags.NONE, 5000, null, (bus, res) => {
            const byOrigin = {};
            try {
                // (serial, monitors, logical_monitors, properties); a logical
                // monitor is (x, y, scale, transform, primary, [(connector,
                // vendor, product, serial)], properties).
                const [, , logical] = bus.call_finish(res).deepUnpack();
                for (const [x, y, , , , monitors] of logical) {
                    if (monitors.length)
                        byOrigin[`${x},${y}`] = monitors[0][0];
                }
            } catch (e) {
                console.warn(`notchd: no monitor connectors: ${e.message}`);
            }
            finish(byOrigin);
        });
    return () => {
        cancelled = true;
    };
}

export class Screens {
    constructor(onChanged) {
        this._onChanged = onChanged;
        this._ids = [
            [Main.layoutManager, Main.layoutManager.connect('monitors-changed', () => this._monitorsChanged())],
            [global.display, global.display.connect('in-fullscreen-changed', () => this._changed())],
            [global.display, global.display.connect('notify::focus-window', () => this._changed())],
            [global.workspace_manager, global.workspace_manager.connect('active-workspace-changed', () => this._changed())],
            [global.workspace_manager, global.workspace_manager.connect('notify::n-workspaces', () => this._changed())],
        ];
        this._last = '';
        this.names = null;
        this._query();
    }

    destroy() {
        for (const [obj, id] of this._ids)
            obj.disconnect(id);
        this._ids = [];
        this._cancel?.();
        this._cancel = null;
    }

    // No `screens` while the monitors' names are being asked for: a daemon
    // told `monitor-0`, or a stale name, would place a notch and move it again.
    _query() {
        this._cancel?.();
        this._cancel = queryConnectors(names => {
            this._cancel = null;
            this.names = names;
            this._changed();
        });
    }

    _monitorsChanged() {
        this._query();
    }

    _changed() {
        if (!this.names || this._cancel)
            return;
        const msg = this.message();
        const text = JSON.stringify(msg);
        if (text === this._last)
            return;
        this._last = text;
        this._onChanged(msg);
    }

    // Report again (a new daemon), as soon as the monitors have names.
    resend() {
        this._last = '';
        this._changed();
    }

    focusedIndex() {
        const win = global.display.focus_window;
        return win ? win.get_monitor() : Main.layoutManager.primaryIndex;
    }

    message() {
        const monitors = Main.layoutManager.monitors;
        const ws = global.workspace_manager.get_active_workspace();
        const wsEntry = {id: String(ws.index() + 1), name: String(ws.index() + 1)};
        const workspaces = {};
        const known = [];
        for (let i = 0; i < global.workspace_manager.n_workspaces; i++)
            known.push(String(i + 1));
        const screens = monitors.map((m, i) => {
            // Workspaces span every monitor unless "workspaces on primary
            // only" is off; either way the active one is what each shows.
            workspaces[this.names[i]] = wsEntry;
            return {name: this.names[i], x: m.x, y: m.y, w: m.width, h: m.height, scale: m.geometry_scale};
        });
        const fullscreen = monitors
            .map((m, i) => (global.display.get_monitor_in_fullscreen(i) ? this.names[i] : null))
            .filter(n => n !== null);
        return {
            t: 'screens',
            screens,
            focused: this.names[this.focusedIndex()] ?? '',
            fullscreen,
            workspaces,
            known,
            supportsWorkspaces: true,
            barEdge: 'top',
        };
    }
}
