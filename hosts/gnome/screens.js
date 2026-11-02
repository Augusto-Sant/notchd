// The compositor adapter: GNOME's monitors, focus, fullscreen and
// workspaces as the `screens` message (docs/architecture.md, "UI to core").
// Reported, never decided: the daemon picks the screens the notch goes on.

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// A monitor's connector name (DP-1, eDP-1), the name the config's
// `position.screen` uses. Mutter exposes it on Meta.Monitor where the
// version has it; otherwise the index stands in.
export function connectors() {
    const names = {};
    try {
        const manager = global.backend.get_monitor_manager();
        for (const logical of manager.get_logical_monitors?.() ?? []) {
            const index = logical.get_number?.();
            const monitor = logical.get_monitors?.()[0];
            const name = monitor?.get_connector?.();
            if (index !== undefined && name)
                names[index] = name;
        }
    } catch (e) {
        console.warn(`notchd: no monitor connectors: ${e.message}`);
    }
    return Main.layoutManager.monitors.map((m, i) => names[i] ?? `monitor-${i}`);
}

export class Screens {
    constructor(onChanged) {
        this._onChanged = onChanged;
        this._ids = [
            [Main.layoutManager, Main.layoutManager.connect('monitors-changed', () => this._changed())],
            [global.display, global.display.connect('in-fullscreen-changed', () => this._changed())],
            [global.display, global.display.connect('notify::focus-window', () => this._changed())],
            [global.workspace_manager, global.workspace_manager.connect('active-workspace-changed', () => this._changed())],
            [global.workspace_manager, global.workspace_manager.connect('notify::n-workspaces', () => this._changed())],
        ];
        this._last = '';
        this.names = connectors();
    }

    destroy() {
        for (const [obj, id] of this._ids)
            obj.disconnect(id);
        this._ids = [];
    }

    _changed() {
        this.names = connectors();
        const msg = this.message();
        const text = JSON.stringify(msg);
        if (text === this._last)
            return;
        this._last = text;
        this._onChanged(msg);
    }

    // Forget the last report, so the next change (or a new daemon) gets one.
    reset() {
        this._last = '';
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
