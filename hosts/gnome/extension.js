// notchd on GNOME Shell: a UI for the notchd daemon, like the QML one
// (docs/architecture.md, "GNOME UI"). The daemon decides; this draws its
// `state` and `view`s and reports screens, the pointer, keys and depths.
// `disable()` takes away everything `enable()` added.

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import St from 'gi://St';

import * as Protocol from './lib/Protocol.js';
import {DaemonClient} from './client.js';
import {NotchCopy, wireKey} from './notch.js';
import {Screens} from './screens.js';
import {Theme} from './theme.js';
import {PRIMITIVES} from './view.js';

export default class NotchdExtension extends Extension {
    enable() {
        this._copies = new Map();
        this._struts = [];
        this._theme = new Theme(() => this._redraw());
        this._screens = new Screens(msg => this._client?.send(msg));
        const version = this.metadata['version-name'] ?? '0';
        this._client = new DaemonClient({
            hello: () => Protocol.hello('gnome-shell', version, 'default',
                {primitives: PRIMITIVES, surfaces: [], webview: false, canvas: false}),
            onConnected: () => {
                this._screens.reset();
                this._client.send(this._screens.message());
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
            names: () => this._screens.names,
            monitorFor: name => {
                const i = this._screens.names.indexOf(name);
                return i >= 0 ? i : this._screens.focusedIndex();
            },
        };
        // Test hooks only for tests (NOTCHD_GNOME_TEST): the file is left
        // out of the extension's zip.
        if (GLib.getenv('NOTCHD_GNOME_TEST')) {
            import('./testhook.js').then(Test => {
                if (!this._client)
                    return;
                this._test = new Test.TestHook(() => ({
                    connected: this._client?.ready ?? false,
                    screens: this._screens?.names ?? [],
                    copies: [...(this._copies?.entries() ?? [])].map(([id, c]) => ({id, ...c.describe()})),
                    struts: (this._struts ?? []).map(a => ({x: a.x, y: a.y, w: a.width, h: a.height})),
                    navigating: !!this._navActor,
                }));
            }).catch(e => console.warn(`notchd: no test hooks: ${e.message}`));
        }
        this._client.start();
    }

    disable() {
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
    }

    _redraw() {
        if (this._client)
            this._apply(this._client.store, 'state');
    }

    _apply(store, key) {
        const state = store.state;
        if (!state || !this._copies)
            return;
        if (key !== 'state' && !key.startsWith('view:'))
            return;
        const seen = new Set();
        for (const notch of state.notches ?? []) {
            for (const copy of notch.copies ?? []) {
                const id = `${notch.id}@${copy.screen}`;
                seen.add(id);
                let actor = this._copies.get(id);
                if (!actor) {
                    actor = new NotchCopy(this._host, notch.id);
                    this._copies.set(id, actor);
                }
                actor.update(notch, copy);
            }
        }
        for (const [id, actor] of this._copies) {
            if (!seen.has(id)) {
                actor.destroy();
                this._copies.delete(id);
            }
        }
        if (key === 'state') {
            this._reserve(state.reserve ?? []);
            this._navigation(!!state.navigation);
        }
    }

    // While navigating, the keyboard is the daemon's: a modal grab on an
    // invisible actor hands every key over (`key` with no notch, as the
    // QML navigation window sends). The highlight is not drawn yet.
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
                this._navGrab = Main.pushModal(this._navActor);
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
    // strut is an invisible chrome actor touching the monitor's edge, so it
    // reaches from the top of the monitor down past the panel.
    _reserve(reserve) {
        const text = JSON.stringify(reserve);
        if (text === this._lastReserve)
            return;
        this._lastReserve = text;
        this._clearStruts();
        for (const r of reserve) {
            if (r.edge !== 'top' || !(r.depth > 0))
                continue;
            const index = this._screens.names.indexOf(r.screen);
            const monitor = Main.layoutManager.monitors[index];
            if (!monitor)
                continue;
            const panel = index === Main.layoutManager.primaryIndex ? Main.panel.height : 0;
            const strut = new St.Widget({name: 'notchd-strut', x: monitor.x, y: monitor.y, width: monitor.width, height: panel + r.depth, reactive: false, opacity: 0});
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
