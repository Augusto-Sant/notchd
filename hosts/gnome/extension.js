// notchd on GNOME Shell: a UI for the notchd daemon, like the QML one
// (docs/architecture.md, "GNOME UI"). The daemon decides; this draws its
// `state` and `view`s and reports screens, the pointer, keys and depths.
// `disable()` takes away everything `enable()` added.

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Protocol from './lib/Protocol.js';
import * as Position from './lib/Position.js';
import {DaemonClient, socketPath} from './client.js';
import {NotchCopy, wireKey} from './notch.js';
import {edgeInsets} from './insets.js';
import {Screens} from './screens.js';
import * as Terminal from './terminal.js';
import {Theme} from './theme.js';
import {PRIMITIVES} from './view.js';
import {PREFS_BUS_NAME, PREFS_REQUEST_KEY, preferencesGone} from './prefsPresence.js';

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
        this._layoutId = 0;
        this._struts = [];
        this._theme = new Theme(() => {
            this._terminals?.updateFonts();
            this._terminals?.recolor();
            this._redraw();
        });
        this._screens = new Screens(msg => {
            this._client?.send(msg);
            this._redraw();
        }, () => this._redraw());
        const version = this.metadata['version-name'] ?? '0';
        this._client = new DaemonClient({
            // Terminal surfaces are Shell actors drawn inside the notch.
            hello: () => Protocol.hello('gnome-shell', version, 'default',
                {primitives: PRIMITIVES, surfaces: this._terminals ? ['terminal'] : [], webview: false, canvas: true}),
            onConnected: welcome => {
                this._exe = welcome?.exe ?? '';
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
            send: msg => this._client?.send(msg),
            store: () => this._client.store,
            terminals: null,
            names: () => this._screens.names ?? [],
            insets: monitor => this._insetsOf(monitor),
            requestLayout: () => this._queueLayout(),
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
                    terminals: this._terminals?.describe() ?? null,
                    navigateAction: this._navigateAction ?? 0,
                    preferencesOpened: this._preferencesOpened ?? 0,
                    settingsOpen: !!this._client?.store.state?.settingsOpen,
                    theme: {background: this._theme?.css('background'), foreground: this._theme?.css('foreground'),
                        panel: this._theme?.panelColors ?? null, fonts: this._theme?.fonts ?? null},
                }));
            }).catch(e => console.warn(`notchd: no test hooks: ${e.message}`));
        }
        this._terminals = new Terminal.Terminals({theme: this._theme, socket: () => socketPath(),
            config: () => this._client.store.config?.cfg ?? {}});
        this._host.terminals = this._terminals;
        this._client.start();
        // The panel's and a dock's room, read again when the work area
        // changes (a dock moved, resized, shown or hidden): the notches
        // move with them.
        this._insets = new Map();
        this._workAreasId = global.display.connect('workareas-changed', () => this._insetsChanged());
        this._monitorsId = Main.layoutManager.connect('monitors-changed', () => this._insetsChanged());
        // The navigation key (the schema's `navigate`), as `notchd ipc
        // navigate`: it starts navigation, and leaves it while its grab
        // holds the keyboard (POPUP, the grab's action mode).
        this._settings = this.getSettings();
        this._prefsSeen = false;
        this._prefsWatch = Gio.bus_watch_name(Gio.BusType.SESSION, PREFS_BUS_NAME,
            Gio.BusNameWatcherFlags.NONE,
            () => { this._prefsSeen = true; },
            () => {
                const gone = preferencesGone(this._prefsSeen,
                    !!this._client?.store.state?.settingsOpen);
                this._prefsSeen = false;
                if (gone)
                    this._client.send({t: 'settings', open: false});
            });
        // A held key repeats: without IGNORE_AUTOREPEAT each repeat toggled
        // navigation again.
        this._navigateAction = Main.wm.addKeybinding('navigate', this._settings, Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW | Shell.ActionMode.POPUP,
            () => this._navigateKey());
    }

    _insetsOf(monitor) {
        if (!monitor)
            return {top: 0, bottom: 0, left: 0, right: 0};
        let insets = this._insets?.get(monitor.index);
        if (!insets) {
            insets = edgeInsets(monitor);
            this._insets?.set(monitor.index, insets);
        }
        return insets;
    }

    _insetsChanged() {
        const before = JSON.stringify([...this._insets.entries()]);
        this._insets.clear();
        Main.layoutManager.monitors.forEach(m => this._insetsOf(m));
        if (JSON.stringify([...this._insets.entries()]) === before)
            return;
        // The reserve hangs past them too.
        this._lastReserve = '';
        this._redraw();
    }

    disable() {
        if (this._layoutId)
            GLib.source_remove(this._layoutId);
        this._layoutId = 0;
        if (this._prefsWatch)
            Gio.bus_unwatch_name(this._prefsWatch);
        this._prefsWatch = 0;
        this._prefsSeen = false;
        if (this._workAreasId)
            global.display.disconnect(this._workAreasId);
        if (this._monitorsId)
            Main.layoutManager.disconnect(this._monitorsId);
        this._workAreasId = this._monitorsId = 0;
        this._insets = null;
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
        this._terminals?.destroy();
        this._terminals = null;
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
        if (key === 'config')
            this._terminals?.updateFonts();
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
        // The guests (docs/architecture.md, `state`): each standalone
        // module's own notch, where its `<id>.position` says, and the
        // visitor at the top; a copy per screen, unseen until it peeks.
        // A guest's actor exists only while it shows something or fades
        // out: closed and faded, it goes (fewer actors and JS objects; GNOME
        // 46's JS engine crashed more often with every guest kept).
        const guest = (key, wireId, notch, copy) => {
            let actor = this._copies.get(key);
            const showing = copy.mode === 'peek' || copy.mode === 'expanded';
            if (!showing && (!actor || !actor.seenOrFading))
                return;
            seen.add(key);
            if (!actor) {
                actor = new NotchCopy(this._host, wireId, {guest: true});
                this._copies.set(key, actor);
            }
            actor.update(notch, copy, null);
        };
        const cfg = store.config?.cfg ?? {};
        for (const g of state.standalone ?? []) {
            const pos = cfg[g.id]?.position ?? {};
            const notch = {id: g.id, edge: pos.edge ?? 'top', offset: pos.offset ?? 0.5, screen: '', draggable: false};
            for (const copy of g.copies ?? [])
                guest(`standalone:${g.id}@${copy.screen}`, g.id, notch, copy);
        }
        for (const copy of state.visitor ?? [])
            guest(`visitor@${copy.screen}`, '', {id: '', edge: 'top', offset: 0.5, screen: '', draggable: false}, copy);
        for (const [id, actor] of this._copies) {
            if (!seen.has(id)) {
                actor.destroy();
                this._copies.delete(id);
            }
        }
        this._arrange();
        if (key === 'state') {
            this._reserve(state.reserve ?? []);
            this._navigation(!!state.navigation);
            this._followSettings(state);
        }
    }

    _queueLayout() {
        if (this._layoutId || !this._copies)
            return;
        this._layoutId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._layoutId = 0;
            this._arrange();
            return GLib.SOURCE_REMOVE;
        });
    }

    // Pack ordinary notches on each screen edge. Top and bottom corners
    // reserve their depth before side notches are placed; the side notches
    // then tell the horizontal outlines where their hairline meets them.
    _arrange() {
        if (!this._copies)
            return;
        const copies = [...this._copies.values()].filter(c => !c.guest);
        const pass = () => {
            const packing = Position.pack(copies.map(c => c.packItem), this._theme.space(28));
            copies.forEach((c, i) => c.planPacking(packing[i]));
            return packing;
        };
        pass();
        const initialCorners = Position.corners(copies.map(c => c.cornerItem));
        copies.forEach((c, i) => {
            if (Position.isVertical(c.place.edge))
                c.setCornerInsets(initialCorners[i]);
        });
        const packing = pass();
        const rowDepths = Position.rowDepths(packing, copies.map(c => c.closedDepth));
        let changed = false;
        copies.forEach((c, i) => { changed = c.setRowDepth(rowDepths[i]) || changed; });
        if (changed)
            return;
        const spans = Position.corners(copies.map(c => c.cornerItem));
        copies.forEach((c, i) => c.applyPacking(
            copies[packing[i].joinStart] ?? null, copies[packing[i].joinEnd] ?? null, spans[i]));
        this._cover();
    }

    // Navigation's `s` and `notchd ipc settings` open the settings window
    // (`settingsOpen` turning true): the preferences (prefs.js), a process
    // of their own that connects to the daemon, shows its `settingsPage`
    // and says when it closes. A daemon already open when this connects
    // opens nothing.
    _followSettings(state) {
        const open = !!state.settingsOpen;
        if (open && this._settingsOpen === false && !this._prefsSeen) {
            this._preferencesOpened = (this._preferencesOpened ?? 0) + 1;
            try {
                this._settings.set_uint(PREFS_REQUEST_KEY, Math.floor(Date.now() / 1000));
                Gio.Settings.sync();
                this.openPreferences();
            } catch (e) {
                this._settings.set_uint(PREFS_REQUEST_KEY, 0);
                console.warn(`notchd: no preferences window: ${e.message}`);
                this._client.send({t: 'settings', open: false});
            }
        }
        this._settingsOpen = open;
    }

    // A guest that shows covers the notches of the config it overlaps on
    // its edge of its monitor (Position.coveredByStandalone): they hide
    // while it shows, unless they are open themselves.
    _cover() {
        const copies = [...this._copies.values()];
        const shown = copies.filter(c => c.guest && c.open);
        for (const c of copies) {
            if (c.guest)
                continue;
            const a = c.place;
            const covered = shown.some(g => {
                const b = g.place;
                if (b.monitor !== a.monitor || b.edge !== a.edge)
                    return false;
                const vertical = a.edge === 'left' || a.edge === 'right';
                const [s1, e1] = vertical ? [a.y, a.y + a.h] : [a.x, a.x + a.w];
                const [s2, e2] = vertical ? [b.y, b.y + b.h] : [b.x, b.x + b.w];
                return s2 < e1 && s1 < e2;
            });
            c.setCovered(covered || !!c._packed?.covered);
        }
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
    // edge (GNOME reads a strut's side from the edges it spans), so it
    // reaches from the monitor's edge past the panel or a dock there.
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
            // Past the panel or a dock on that edge: the reserve adds to
            // theirs.
            const inset = this._insetsOf(m);
            const box = {
                top: {x: m.x, y: m.y, width: m.width, height: inset.top + r.depth},
                bottom: {x: m.x, y: m.y + m.height - inset.bottom - r.depth, width: m.width, height: inset.bottom + r.depth},
                left: {x: m.x, y: m.y, width: inset.left + r.depth, height: m.height},
                right: {x: m.x + m.width - inset.right - r.depth, y: m.y, width: inset.right + r.depth, height: m.height},
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
