// notchd on GNOME Shell: a UI for the notchd daemon, like the QML one
// (docs/architecture.md, "GNOME UI"). The daemon decides; this draws its
// `state` and `view`s and reports screens, the pointer, keys and depths.
// `disable()` takes away everything `enable()` added.

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Protocol from './lib/Protocol.js';
import {DaemonClient} from './client.js';
import {NotchCopy, wireKey} from './notch.js';
import {Screens} from './screens.js';
import {Theme} from './theme.js';
import {PRIMITIVES} from './view.js';

export default class NotchdExtension extends Extension {
    enable() {
        // Enabled while the shell starts, it draws once the shell has
        // started: a copy laid out before that is not in the stage yet, and
        // Clutter and St warn about it.
        this._starting = Main.layoutManager._startingUp;
        if (this._starting) {
            this._startupId = Main.layoutManager.connect('startup-complete', () => {
                Main.layoutManager.disconnect(this._startupId);
                this._startupId = 0;
                this._starting = false;
                this._redraw();
            });
        }
        this._copies = new Map();
        this._struts = [];
        this._theme = new Theme(() => this._redraw());
        this._screens = new Screens(msg => this._client?.send(msg), () => this._redraw());
        const version = this.metadata['version-name'] ?? '0';
        this._client = new DaemonClient({
            hello: () => Protocol.hello('gnome-shell', version, 'default',
                {primitives: PRIMITIVES, surfaces: [], webview: false, canvas: false}),
            onConnected: () => {
                this._screens.resend();
                for (const copy of this._copies.values())
                    copy.resendMeasure();
            },
            onStore: (store, key) => this._apply(store, key),
            // Never keep the keyboard for a daemon that is gone: the
            // grabs it asked for go with it.
            onDisconnected: () => {
                this._endNavigation();
                for (const copy of this._copies?.values() ?? [])
                    copy.release();
            },
            log: text => console.log(`notchd: ${text}`),
        });
        this._host = {
            theme: this._theme,
            send: msg => this._client.send(msg),
            store: () => this._client.store,
            names: () => this._screens.names ?? [],
            monitorFor: name => {
                const i = this._screens.names?.indexOf(name) ?? -1;
                return i >= 0 ? i : this._screens.focusedIndex();
            },
        };
        // Test hooks only for tests (NOTCHD_GNOME_TEST): the file is left
        // out of the extension's zip.
        if (GLib.getenv('NOTCHD_GNOME_TEST')) {
            import('./testhook.js').then(Test => {
                if (!this._client)
                    return;
                this._test = new Test.TestHook({navigate: () => this._navigateKey()}, () => ({
                    connected: this._client?.ready ?? false,
                    screens: this._screens?.names ?? [],
                    copies: [...(this._copies?.entries() ?? [])].map(([id, c]) => ({id, ...c.describe()})),
                    struts: (this._struts ?? []).map(a => ({x: a.x, y: a.y, w: a.width, h: a.height})),
                    navigating: !!this._navActor,
                    navigateAction: this._navigateAction ?? 0,
                    preferencesOpened: this._preferencesOpened ?? 0,
                }));
            }).catch(e => console.warn(`notchd: no test hooks: ${e.message}`));
        }
        this._client.start();
        // The navigation key (the schema's `navigate`), as `notchd ipc
        // navigate`: it starts navigation, and leaves it while its grab
        // holds the keyboard (POPUP, the grab's action mode).
        this._settings = this.getSettings();
        // A held key repeats: without IGNORE_AUTOREPEAT each repeat toggled
        // navigation again.
        this._navigateAction = Main.wm.addKeybinding('navigate', this._settings, Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW | Shell.ActionMode.POPUP,
            () => this._navigateKey());
    }

    disable() {
        if (this._startupId)
            Main.layoutManager.disconnect(this._startupId);
        this._startupId = 0;
        Main.wm.removeKeybinding('navigate');
        this._settings = null;
        this._test?.destroy();
        this._test = null;
        this._client?.stop();
        this._client = null;
        for (const copy of this._copies?.values() ?? [])
            copy.destroy();
        this._copies = null;
        this._clearStruts();
        this._endNavigation();
        this._screens?.destroy();
        this._screens = null;
        this._theme?.destroy();
        this._theme = null;
        this._host = null;
        this._settingsOpen = undefined;
    }

    _navigateKey() {
        this._client?.ipc('navigate');
    }

    _redraw() {
        if (this._client)
            this._apply(this._client.store, 'state');
    }

    _apply(store, key) {
        const state = store.state;
        if (!state || !this._copies || this._starting)
            return;
        if (key !== 'state' && !key.startsWith('view:'))
            return;
        const seen = new Set();
        const notches = state.notches ?? [];
        const nav = state.navigation;
        notches.forEach((notch, index) => {
            // While navigating, what this notch's highlight says.
            const highlight = nav ? {
                number: index < 9 ? index + 1 : 0, selected: nav.selected === index, mode: nav.mode,
                count: notches.length, screens: this._screens.names?.length ?? 1,
            } : null;
            for (const copy of notch.copies ?? []) {
                const id = `${notch.id}@${copy.screen}`;
                seen.add(id);
                let actor = this._copies.get(id);
                if (!actor) {
                    actor = new NotchCopy(this._host, notch.id);
                    this._copies.set(id, actor);
                }
                actor.update(notch, copy, highlight);
            }
        });
        for (const [id, actor] of this._copies) {
            if (!seen.has(id)) {
                actor.destroy();
                this._copies.delete(id);
            }
        }
        if (key === 'state') {
            this._reserve(state.reserve ?? []);
            this._navigation(!!state.navigation);
            this._followSettings(state);
        }
    }

    // Navigation's `s` and `notchd ipc settings` open the settings window
    // (`settingsOpen` turning true): the preferences (prefs.js), a process
    // of their own that connects to the daemon, shows its `settingsPage`
    // and says when it closes. A daemon already open when this connects
    // opens nothing.
    _followSettings(state) {
        const open = !!state.settingsOpen;
        if (open && this._settingsOpen === false) {
            this._preferencesOpened = (this._preferencesOpened ?? 0) + 1;
            try {
                this.openPreferences();
            } catch (e) {
                console.warn(`notchd: no preferences window: ${e.message}`);
            }
        }
        this._settingsOpen = open;
    }

    // While navigating, the keyboard is the daemon's: a modal grab on an
    // invisible actor hands every key over (`key` with no notch, as the
    // QML navigation window sends). Each copy draws its own highlight.
    _navigation(on) {
        if (on && !this._navActor) {
            this._navActor = new St.Widget({name: 'notchd-navigation', reactive: true, can_focus: true, width: 1, height: 1, opacity: 0});
            Main.layoutManager.addTopChrome(this._navActor);
            this._navActor.connect('key-press-event', (actor, event) => {
                const wire = wireKey(event);
                if (wire)
                    this._client?.send(Protocol.key('', '', wire.key, wire.mods, ''));
                return Clutter.EVENT_STOP;
            });
            try {
                this._navGrab = Main.pushModal(this._navActor, {actionMode: Shell.ActionMode.POPUP});
                this._navActor.grab_key_focus();
            } catch (e) {
                console.warn(`notchd: no keyboard for navigation: ${e.message}`);
            }
        } else if (!on && this._navActor) {
            this._endNavigation();
        }
    }

    _endNavigation() {
        if (this._navGrab)
            Main.popModal(this._navGrab);
        this._navGrab = null;
        if (this._navActor) {
            Main.layoutManager.removeChrome(this._navActor);
            this._navActor.destroy();
        }
        this._navActor = null;
    }

    // `state.reserve`: the space windows keep clear of a closed notch. A
    // strut is an invisible chrome actor along the whole of the monitor's
    // edge (GNOME reads a strut's side from the edges it spans), so on the
    // top it reaches from the top of the monitor down past the panel.
    _reserve(reserve) {
        // Not before the monitors have names: a daemon that is already
        // running sends its reserve at once, when this enable has not yet
        // asked mutter for them, and the strut for `Meta-0` would have no
        // monitor. `Screens` calls back when they are known.
        if (!this._screens.names)
            return;
        const text = JSON.stringify(reserve);
        if (text === this._lastReserve)
            return;
        this._lastReserve = text;
        this._clearStruts();
        for (const r of reserve) {
            if (!(r.depth > 0))
                continue;
            const index = this._screens.names?.indexOf(r.screen) ?? -1;
            const m = Main.layoutManager.monitors[index];
            if (!m)
                continue;
            const panel = index === Main.layoutManager.primaryIndex ? Main.panel.height : 0;
            const box = {
                top: {x: m.x, y: m.y, width: m.width, height: panel + r.depth},
                bottom: {x: m.x, y: m.y + m.height - r.depth, width: m.width, height: r.depth},
                left: {x: m.x, y: m.y, width: r.depth, height: m.height},
                right: {x: m.x + m.width - r.depth, y: m.y, width: r.depth, height: m.height},
            }[r.edge];
            if (!box)
                continue;
            const strut = new St.Widget({name: 'notchd-strut', ...box, reactive: false, opacity: 0});
            Main.layoutManager.addTopChrome(strut, {affectsStruts: true});
            this._struts.push(strut);
        }
    }

    _clearStruts() {
        for (const strut of this._struts ?? []) {
            Main.layoutManager.removeChrome(strut);
            strut.destroy();
        }
        this._struts = [];
        this._lastReserve = '';
    }
}
