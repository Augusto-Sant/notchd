// notchd's settings window on GNOME: the extension's preferences, in their
// own process (no Main, no St or Clutter: GTK 4 and libadwaita). It is a UI
// of the daemon like the QML settings window (core/settings/): it connects
// to the socket itself, draws the daemon's `schema` as pages and rows, one
// control per field type (docs/settings.md), and sends each edit as `set`,
// `edit` or `ipc`; the config file is the daemon's to write. It tells the
// daemon the page it shows and when it closes (`settings`), and follows
// the daemon's page (navigation's `s` on a notch). How a field becomes rows
// and an edit becomes a message is prefsModel.js, tested in Node. The
// extension's own setting, the navigation key, has a page of its own.
//
// libadwaita 1.5 (GNOME 46) is the oldest this runs on: nothing newer.

import Adw from 'gi://Adw';
import Gdk from 'gi://Gdk';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

import * as Protocol from './lib/Protocol.js';
import * as Config from './lib/Config.js';
import * as Settings from './lib/Settings.js';
import * as Model from './prefsModel.js';
import {DaemonClient, socketPath} from './client.js';
import {PREFS_BUS_NAME, PREFS_REQUEST_KEY, requestedPreferences} from './prefsPresence.js';

// A spin row sends once it rests this long, so holding an arrow is one
// write, not one per step.
const SPIN_SETTLE_MS = 250;
const KEYS_PAGE = 'gnome-keyboard';
let preferenceWindows = 0;
let preferenceNameId = 0;

export default class NotchdPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        window.set_default_size(760, 680);
        const settings = this.getSettings();
        const requestedAt = settings.get_uint(PREFS_REQUEST_KEY);
        if (requestedAt) {
            settings.set_uint(PREFS_REQUEST_KEY, 0);
            Gio.Settings.sync();
        }
        const fromDaemon = requestedPreferences(requestedAt, Math.floor(Date.now() / 1000));
        if (!preferenceNameId)
            preferenceNameId = Gio.bus_own_name_on_connection(Gio.DBus.session, PREFS_BUS_NAME,
                Gio.BusNameOwnerFlags.NONE, null, null);
        preferenceWindows++;
        const ui = new SettingsUi(window, settings, this.metadata['version-name'] ?? '0', fromDaemon);
        window.connect('close-request', () => {
            ui.close();
            if (--preferenceWindows === 0) {
                Gio.bus_unown_name(preferenceNameId);
                preferenceNameId = 0;
            }
            return false;
        });
    }
}

// A row whose title and subtitle are plain text (help texts have `<id>` in
// them). `use_markup` first: a title set while it is still on is parsed.
function row(Type, props) {
    const r = new Type({use_markup: false});
    for (const [k, v] of Object.entries(props))
        r[k] = v;
    return r;
}

function iconButton(icon, tooltip, onClick) {
    const b = new Gtk.Button({icon_name: icon, tooltip_text: tooltip, valign: Gtk.Align.CENTER});
    b.add_css_class('flat');
    b.connect('clicked', onClick);
    return b;
}

class SettingsUi {
    constructor(window, gsettings, version, fromDaemon) {
        this._window = window;
        this._gsettings = gsettings;
        this._pages = new Map();
        this._navShape = '';
        this._sub = null;
        this._shown = '';
        this._reports = [];
        this._quietDepth = 0;
        this._requestId = 0;
        this._following = false;
        this._started = false;
        this._fromDaemon = fromDaemon;

        // Something must show before this returns (the dialog checks), so
        // the window starts on the status page and the keyboard page.
        this._status = new Adw.PreferencesPage({name: 'status', title: 'notchd', icon_name: 'dialog-warning-symbolic'});
        this._statusView = new Adw.StatusPage({icon_name: 'dialog-warning-symbolic', vexpand: true});
        const statusGroup = new Adw.PreferencesGroup();
        statusGroup.add(this._statusView);
        this._status.add(statusGroup);
        this._keys = this._keysPage();
        window.add(this._status);
        window.add(this._keys);
        this._showStatus(Gio.File.new_for_path(socketPath()).query_exists(null));

        this._pageId = window.connect('notify::visible-page-name', () => this._onVisiblePage());

        this._client = new DaemonClient({
            hello: () => Protocol.hello('gnome-prefs', version, 'default',
                {primitives: [], surfaces: [], webview: false, canvas: false}),
            onConnected: () => {
                this._started = false;
            },
            onStore: (store, key) => this._onStore(store, key),
            onDisconnected: () => this._disconnected(),
            log: text => console.log(`notchd prefs: ${text}`),
        });
        this._client.start();
    }

    close() {
        if (this._closed)
            return;
        this._closed = true;
        // The window is gone: the daemon stops polling components for it,
        // and navigation's `s` opens it again.
        this._client.sendFinal({t: 'settings', open: false});
        this._window.disconnect(this._pageId);
        if (this._keysChangedId)
            this._gsettings.disconnect(this._keysChangedId);
        for (const entry of this._allEntries())
            this._clearEntry(entry);
    }

    // ------------------------------------------------------------ status
    _showStatus(socketThere) {
        this._statusView.title = socketThere ? 'Connecting to notchd…' : 'notchd is not running';
        this._statusView.description = GLib.markup_escape_text(socketThere
            ? 'Waiting for the daemon to answer.'
            : `The settings are the notchd daemon's, and it is not running. Start it with “notchd daemon --ensure”; this window connects once it is up.\n\nSocket: ${socketPath()}`, -1);
    }

    _disconnected() {
        if (this._closed)
            return;
        this._started = false;
        this._showStatus(false);
        this._dropPages();
    }

    // ------------------------------------------------------------- store
    _onStore(store, key) {
        if (this._closed || !['schema', 'config', 'state'].includes(key))
            return;
        const {schema, config, state} = store;
        if (!schema || !config || !state)
            return;
        this._ctx = {schema, state, cfg: config.cfg ?? {}};
        this._render();
        if (!this._started) {
            // A new connection: the window starts on the daemon's page
            // (kept while the window was closed) and says it is open.
            this._started = true;
            if (this._fromDaemon && !state.settingsOpen) {
                this._window.close();
                return;
            }
            this._go(state.settingsPage || 'general');
            this._report(this._shown);
        } else if (key === 'state') {
            // Closed by the daemon (`ipc settings` toggles it): the window
            // goes, as the QML one does.
            if (this._wasOpen && !state.settingsOpen) {
                this._window.close();
                return;
            }
            const page = Model.pageToFollow(state, this._shown, this._reports, Date.now());
            if (page)
                this._go(page);
        }
        if (key === 'state')
            this._wasOpen = !!state.settingsOpen;
    }

    // The pages: the sidebar's drawn again when the list changes, then every
    // page and the open sub-page brought up to date.
    _render() {
        const {schema, state} = this._ctx;
        const nav = Model.navPages(schema);
        const shape = JSON.stringify(nav);
        if (shape !== this._navShape) {
            this._quiet(() => {
                const visible = this._window.visible_page_name;
                this._dropPages();
                this._window.remove(this._status);
                this._window.remove(this._keys);
                for (const p of nav) {
                    const page = new Adw.PreferencesPage({name: p.id, title: p.label, icon_name: p.icon});
                    this._window.add(page);
                    this._pages.set(p.id, {id: p.id, page, shape: null, groups: [], rows: []});
                }
                this._window.add(this._keys);
                this._navShape = shape;
                if (visible && (this._pages.has(visible) || visible === KEYS_PAGE))
                    this._window.visible_page_name = visible;
            });
        }
        for (const entry of this._pages.values())
            this._fill(entry);
        if (this._sub) {
            if (Model.resolvePage(schema, state, this._sub.id) !== this._sub.id)
                this._closeSub();
            else
                this._fill(this._sub);
        }
    }

    _dropPages() {
        this._quiet(() => {
            if (this._sub)
                this._closeSub();
            for (const entry of this._pages.values()) {
                this._clearEntry(entry);
                this._window.remove(entry.page);
            }
            this._pages.clear();
            this._navShape = '';
            if (!this._status.get_parent()) {
                this._window.remove(this._keys);
                this._window.add(this._status);
                this._window.add(this._keys);
            }
        });
    }

    *_allEntries() {
        yield* this._pages.values();
        if (this._sub)
            yield this._sub;
    }

    _clearEntry(entry) {
        for (const r of entry.rows)
            r.destroy?.();
        for (const g of entry.groups)
            entry.page.remove(g);
        entry.rows = [];
        entry.groups = [];
        entry.shape = null;
    }

    // A page's groups and rows: drawn again when its shape changes,
    // otherwise each row takes its field's new value in place.
    _fill(entry) {
        const {schema} = this._ctx;
        const groups = Model.pageGroups(schema, entry.id);
        const shape = Model.pageShape(schema, entry.id);
        if (shape !== entry.shape) {
            this._clearEntry(entry);
            entry.shape = shape;
            for (const g of groups) {
                const group = new Adw.PreferencesGroup({title: GLib.markup_escape_text(g.title, -1),
                    description: GLib.markup_escape_text(g.description, -1)});
                entry.page.add(group);
                entry.groups.push(group);
                for (const f of g.fields)
                    entry.rows.push(this._control(f, group, entry.id));
            }
            if (groups.length === 0) {
                const group = new Adw.PreferencesGroup();
                group.add(row(Adw.ActionRow, {title: 'Loading…'}));
                entry.page.add(group);
                entry.groups.push(group);
            }
        }
        const fields = groups.flatMap(g => g.fields);
        this._quiet(() => entry.rows.forEach((r, i) => r.update(fields[i])));
    }

    // ------------------------------------------------------- navigation
    _onVisiblePage() {
        if (this._quietDepth > 0 || this._following)
            return;
        const name = this._window.visible_page_name;
        if (this._pages.has(name))
            this._report(name);
    }

    // Shows `wanted` (the daemon's page): its sidebar page, and over it the
    // sub-page when it is one.
    _go(wanted) {
        const page = Model.resolvePage(this._ctx.schema, this._ctx.state, wanted);
        if (!page)
            return;
        this._following = true;
        try {
            if (this._sub && this._sub.id !== page)
                this._closeSub();
            const parent = Model.parentPage(page);
            if (this._pages.has(parent))
                this._window.visible_page_name = parent;
            if (Model.isSubPage(page) && !this._sub)
                this._openSub(page, false);
        } finally {
            this._following = false;
        }
        this._shown = page;
    }

    _openSub(id, report = true) {
        if (this._sub)
            this._closeSub();
        const label = Model.findPage(this._ctx.schema, id)?.label ?? id;
        const page = new Adw.PreferencesPage();
        const view = new Adw.ToolbarView({content: page});
        view.add_top_bar(new Adw.HeaderBar());
        const nav = new Adw.NavigationPage({title: label, child: view, tag: id});
        const entry = {id, page, nav, shape: null, groups: [], rows: []};
        this._sub = entry;
        // Back (the header's arrow, Escape) leaves it for its parent.
        entry.hiddenId = nav.connect('hidden', () => {
            if (this._sub !== entry)
                return;
            this._clearEntry(entry);
            this._sub = null;
            this._shown = this._window.visible_page_name;
            if (this._pages.has(this._shown))
                this._report(this._shown);
        });
        this._fill(entry);
        this._window.push_subpage(nav);
        this._shown = id;
        if (report)
            this._report(id);
    }

    _closeSub() {
        const entry = this._sub;
        if (!entry)
            return;
        this._sub = null;
        entry.nav.disconnect(entry.hiddenId);
        this._clearEntry(entry);
        this._window.pop_subpage();
    }

    _report(page) {
        if (!page)
            return;
        this._shown = page;
        const now = Date.now();
        this._reports = this._reports.filter(r => now - r.at < 2000).concat([{page, at: now}]);
        this._client.send(this._ctx?.state?.settingsOpen
            ? {t: 'settings-page', page} : {t: 'settings', open: true, page});
    }

    // ---------------------------------------------------------- sending
    _set(key, value) {
        this._client.send(Protocol.set(++this._requestId, key, value === undefined ? null : value));
    }

    _sets(list) {
        for (const s of list)
            this._set(s.key, s.value);
    }

    _edit(op, fields) {
        this._client.send(Protocol.edit(++this._requestId, op, fields));
    }

    // Runs `fn` without the controls' signals sending what it sets.
    _quiet(fn) {
        this._quietDepth++;
        try {
            fn();
        } finally {
            this._quietDepth--;
        }
    }

    get _loud() {
        return this._quietDepth === 0;
    }

    // ---------------------------------------------------------- controls
    // One field's control, added to `group`: `{update(field), destroy()}`.
    _control(field, group, pageId) {
        switch (Model.controlOf(field)) {
        case 'switch': return this._switch(field, group);
        case 'combo': return this._combo(field, group);
        case 'spin': return this._spin(field, group);
        case 'entry': return this._entry(field, group, false);
        case 'color': return this._entry(field, group, true);
        case 'note': return this._note(field, group);
        case 'component': return this._component(field, group);
        case 'modules': return this._list(group, f => Model.moduleRows(f, this._ctx.schema.modules,
            Model.terminalIds(this._ctx.cfg, Config)), r => this._moduleRow(r, pageId));
        case 'workspaces': return this._workspaces(field, group, pageId);
        case 'notches': return this._notches(field, group);
        case 'catalog': return this._catalog(field, group);
        default: {
            const r = row(Adw.ActionRow, {title: field.label || field.key || '', subtitle: `unsupported field type: ${field.type}`});
            group.add(r);
            return {update: () => {}};
        }
        }
    }

    // What every row takes from its field: shown or not, locked or not.
    _common(widget, field) {
        widget.visible = field.visible !== false;
        widget.sensitive = !field.locked;
        widget.tooltip_text = field.locked ? String(field.locked) : null;
    }

    _switch(field, group) {
        let current = field;
        const r = row(Adw.SwitchRow, {title: field.label || field.key, subtitle: field.help ?? ''});
        r.connect('notify::active', () => {
            if (this._loud)
                this._set(current.key, Model.wireValue(current, r.active));
        });
        group.add(r);
        return {
            update: f => {
                current = f;
                this._common(r, f);
                r.active = !!Model.valueOf(f);
            },
        };
    }

    _combo(field, group) {
        let current = field;
        const items = Model.comboItems(field);
        const r = row(Adw.ComboRow, {title: field.label || field.key, subtitle: field.help ?? '',
            model: Gtk.StringList.new(items.map(i => i.label))});
        r.connect('notify::selected', () => {
            if (!this._loud)
                return;
            const v = Model.comboValue(current, r.selected);
            if (v === undefined)
                this._quiet(() => (r.selected = Model.comboIndex(current, Model.valueOf(current))));
            else
                this._set(current.key, Model.wireValue(current, v));
        });
        group.add(r);
        return {
            update: f => {
                current = f;
                this._common(r, f);
                r.selected = Model.comboIndex(f, Model.valueOf(f));
            },
        };
    }

    _spin(field, group) {
        let current = field;
        let timer = 0;
        const spec = Model.spinSpec(field);
        const r = Adw.SpinRow.new_with_range(spec.min, spec.max, spec.step);
        r.use_markup = false;
        r.title = spec.unit ? `${field.label || field.key} (${spec.unit})` : field.label || field.key;
        r.subtitle = field.help ?? '';
        r.digits = spec.digits;
        r.connect('notify::value', () => {
            if (!this._loud)
                return;
            if (timer)
                GLib.source_remove(timer);
            timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, SPIN_SETTLE_MS, () => {
                timer = 0;
                this._set(current.key, Model.wireValue(current, Model.fromSpin(current, r.value)));
                return GLib.SOURCE_REMOVE;
            });
        });
        group.add(r);
        return {
            update: f => {
                current = f;
                this._common(r, f);
                // Not under a value about to be sent.
                if (!timer)
                    r.value = Model.toSpin(f, Model.valueOf(f));
            },
            destroy: () => {
                if (timer)
                    GLib.source_remove(timer);
                timer = 0;
            },
        };
    }

    // `string` and `color`: the config's own value in an entry, the default
    // as its placeholder (TextControl.qml); Enter or leaving it commits. A
    // color has a button that picks one too.
    _entry(field, group, isColor) {
        let current = field;
        const r = row(Adw.ActionRow, {title: field.label || field.key, subtitle: field.help ?? ''});
        const entry = new Gtk.Entry({valign: Gtk.Align.CENTER, width_chars: isColor ? 12 : 22});
        const focus = new Gtk.EventControllerFocus();
        entry.add_controller(focus);
        const stored = () => Config.valueAt(this._ctx.cfg, current.key);
        const send = value => {
            if (JSON.stringify(value) !== JSON.stringify(stored() ?? null))
                this._set(current.key, value);
        };
        const commit = () => {
            if (!this._loud)
                return;
            const edit = Model.textEdit(current, entry.text);
            if (!edit.ok) {
                entry.add_css_class('error');
                return;
            }
            entry.remove_css_class('error');
            send(edit.value);
        };
        entry.connect('activate', commit);
        focus.connect('leave', commit);
        r.add_suffix(entry);
        let picker = null;
        if (isColor) {
            picker = new Gtk.ColorDialogButton({dialog: new Gtk.ColorDialog({with_alpha: true}), valign: Gtk.Align.CENTER});
            picker.connect('notify::rgba', () => {
                if (!this._loud)
                    return;
                const c = picker.rgba;
                send(Model.wireValue(current, Model.rgbaToColor({red: c.red, green: c.green, blue: c.blue, alpha: c.alpha})));
            });
            r.add_suffix(picker);
        }
        group.add(r);
        return {
            update: f => {
                current = f;
                this._common(r, f);
                const def = f.default === undefined || f.default === null ? '' : String(f.default);
                entry.placeholder_text = def;
                if (!focus.contains_focus) {
                    const v = stored();
                    entry.text = v === undefined || v === null ? '' : String(v);
                    entry.remove_css_class('error');
                }
                if (picker) {
                    const rgba = Model.colorToRgba(Model.valueOf(f));
                    if (rgba)
                        picker.rgba = new Gdk.RGBA(rgba);
                }
            },
        };
    }

    _note(field, group) {
        const r = row(Adw.ActionRow, {title: field.label || '', subtitle: field.help ?? ''});
        group.add(r);
        return {update: f => this._common(r, f)};
    }

    // `component` (ComponentRow.qml): its state, a bar while it installs and
    // its buttons, each the `component` IPC. Remove asks twice.
    _component(field, group) {
        let current = field;
        let actions = '';
        let confirming = 0;
        const r = row(Adw.ActionRow, {title: field.label || field.component || ''});
        const bar = new Gtk.ProgressBar({valign: Gtk.Align.CENTER, width_request: 120, visible: false});
        const buttons = new Gtk.Box({spacing: 6, valign: Gtk.Align.CENTER});
        r.add_suffix(bar);
        r.add_suffix(buttons);
        const act = (id, button) => {
            if (id === 'remove' && !confirming) {
                button.label = 'Sure?';
                confirming = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => {
                    confirming = 0;
                    button.label = 'Remove';
                    return GLib.SOURCE_REMOVE;
                });
                return;
            }
            if (confirming)
                GLib.source_remove(confirming);
            confirming = 0;
            this._client.ipc('component', [Model.componentCommand(id), current.component]);
        };
        group.add(r);
        return {
            update: f => {
                current = f;
                this._common(r, f);
                const info = f.row ?? null;
                r.subtitle = info ? Model.componentText(info) : f.help ?? '';
                r.tooltip_text = f.help ?? null;
                const working = info?.state === 'working';
                bar.visible = working;
                if (working) {
                    if (info.percent >= 0)
                        bar.fraction = info.percent / 100;
                    else
                        bar.pulse();
                }
                const next = JSON.stringify(info?.actions ?? []);
                if (next !== actions) {
                    actions = next;
                    let child;
                    while ((child = buttons.get_first_child()))
                        buttons.remove(child);
                    for (const a of info?.actions ?? []) {
                        const b = new Gtk.Button({label: a.label});
                        if (a.id === 'remove')
                            b.add_css_class('destructive-action');
                        b.connect('clicked', () => act(a.id, b));
                        buttons.append(b);
                    }
                }
            },
            destroy: () => {
                if (confirming)
                    GLib.source_remove(confirming);
                confirming = 0;
            },
        };
    }

    // A list field's rows, drawn again when what they show changes:
    // `rows(field)` computes them, `make(row)` makes one.
    _list(group, rows, make, extra = {}) {
        let shown = null;
        let widgets = [];
        let current = null;
        return {
            update: f => {
                current = f;
                group.visible = f.visible !== false;
                const next = rows(f);
                const text = JSON.stringify(next);
                if (text === shown)
                    return;
                shown = text;
                for (const w of widgets)
                    group.remove(w);
                widgets = next.map(r => make(r, current));
                if (widgets.length === 0 && extra.empty)
                    widgets = [row(Adw.ActionRow, {title: extra.empty})];
                for (const w of widgets)
                    group.add(w);
            },
        };
    }

    _moduleRow(m, pageId) {
        const r = row(Adw.ActionRow, {title: m.title, subtitle: m.known ? m.id : `${m.id} · not found`});
        if (!m.on)
            r.add_css_class('dim-label');
        const notch = Model.notchIdOfPage(this._ctx.schema, this._ctx.state, pageId);
        if (m.on) {
            const up = iconButton('go-up-symbolic', 'Earlier', () => this._edit('moveModule', {notch, module: m.id, delta: -1}));
            const down = iconButton('go-down-symbolic', 'Later', () => this._edit('moveModule', {notch, module: m.id, delta: 1}));
            up.sensitive = m.canUp;
            down.sensitive = m.canDown;
            r.add_suffix(up);
            r.add_suffix(down);
        }
        const toggle = new Gtk.Switch({active: m.on, valign: Gtk.Align.CENTER});
        toggle.connect('notify::active', () => this._edit('toggleModule', {notch, module: m.id}));
        r.add_suffix(toggle);
        r.activatable_widget = toggle;
        return r;
    }

    // `workspaces` (WorkspaceChips.qml): a chip per workspace; none picked
    // means every workspace. The daemon's `edit` toggles one.
    _workspaces(field, group, pageId) {
        const box = new Gtk.FlowBox({selection_mode: Gtk.SelectionMode.NONE, column_spacing: 6, row_spacing: 6,
            max_children_per_line: 12, homogeneous: true});
        group.add(box);
        let shown = '';
        return {
            update: f => {
                group.visible = f.visible !== false;
                // The daemon puts the compositor's reported workspace ids
                // in the schema for this separate preferences process.
                const chips = Model.workspaceChips(f, f.options?.map(o => o.value) ?? [], Settings);
                const text = JSON.stringify(chips);
                if (text === shown)
                    return;
                shown = text;
                let child;
                while ((child = box.get_first_child()))
                    box.remove(child);
                const notch = Model.notchIdOfPage(this._ctx.schema, this._ctx.state, pageId);
                for (const c of chips) {
                    const b = new Gtk.ToggleButton({label: c.label, active: c.on});
                    b.connect('toggled', () => this._edit('toggleWorkspace', {notch, workspace: String(c.value)}));
                    box.append(b);
                }
            },
        };
    }

    // `notches` (NotchCatalog.qml): a row per notch that opens its page, a
    // trash can with more than one, and Add notch, which opens the new one.
    _notches(field, group) {
        const add = new Gtk.Button({label: 'Add notch', valign: Gtk.Align.CENTER});
        add.connect('clicked', () => {
            const r = Model.addNotch(this._ctx.cfg, this._ctx.state, Config);
            this._sets(r.sets);
            this._openSub(`notch/${r.index}`);
        });
        group.header_suffix = add;
        return this._list(group, () => Model.notchRows(this._ctx.state, this._ctx.schema.modules, Settings), n => {
            const r = row(Adw.ActionRow, {title: n.id, subtitle: n.summary, activatable: true});
            if (n.removable) {
                r.add_suffix(iconButton('user-trash-symbolic', 'Remove this notch',
                    () => this._sets(Model.removeNotch(this._ctx.cfg, n.index, Config))));
            }
            r.add_suffix(new Gtk.Image({icon_name: 'go-next-symbolic'}));
            r.connect('activated', () => this._openSub(`notch/${n.index}`));
            return r;
        });
    }

    // `moduleCatalog` (ModuleCatalog.qml): a part of the Modules page, each
    // module with where it shows and its page; the terminals' list adds and
    // removes them.
    _catalog(field, group) {
        const terminals = field.part === 'terminal';
        if (terminals) {
            const add = new Gtk.Button({label: 'Add terminal', valign: Gtk.Align.CENTER});
            add.connect('clicked', () => this._sets(Model.addTerminal(this._ctx.cfg, this._ctx.schema.modules, Config).sets));
            group.header_suffix = add;
        }
        return this._list(group, f => Model.catalogRows(f, this._ctx.schema, this._ctx.state, this._ctx.cfg, Settings, Config), m => {
            const r = row(Adw.ActionRow, {title: m.title, subtitle: `${m.id} · ${m.status}`, activatable: !!m.page});
            if (/off$/i.test(m.status))
                r.add_css_class('dim-label');
            if (m.removable) {
                r.add_suffix(iconButton('user-trash-symbolic', 'Remove this terminal',
                    () => this._sets(Model.removeTerminal(this._ctx.cfg, m.id, Config))));
            }
            if (m.page) {
                r.add_suffix(new Gtk.Image({icon_name: 'go-next-symbolic'}));
                r.connect('activated', () => this._openSub(m.page));
            } else if (!terminals) {
                const none = new Gtk.Label({label: 'No settings', valign: Gtk.Align.CENTER});
                none.add_css_class('dim-label');
                r.add_suffix(none);
            }
            return r;
        }, {empty: terminals ? 'None yet.' : ''});
    }

    // ------------------------------------------------- the keyboard page
    // The extension's own setting: the key that starts navigation (its
    // GSettings `navigate`), picked by pressing it.
    _keysPage() {
        const page = new Adw.PreferencesPage({name: KEYS_PAGE, title: 'Keyboard', icon_name: 'input-keyboard-symbolic'});
        const group = new Adw.PreferencesGroup({title: 'Navigation',
            description: GLib.markup_escape_text('The key GNOME Shell starts notch navigation with, as notchd ipc navigate does. Pressed again, or Escape, it ends.', -1)});
        const r = row(Adw.ActionRow, {title: 'Navigate notches'});
        const label = new Gtk.ShortcutLabel({disabled_text: 'Off', valign: Gtk.Align.CENTER});
        const change = new Gtk.Button({label: 'Change…', valign: Gtk.Align.CENTER});
        const reset = iconButton('edit-undo-symbolic', 'Back to the default', () => this._gsettings.reset('navigate'));
        change.connect('clicked', () => this._captureKey());
        r.add_suffix(label);
        r.add_suffix(change);
        r.add_suffix(reset);
        r.activatable_widget = change;
        group.add(r);
        page.add(group);
        const show = () => {
            label.accelerator = this._gsettings.get_strv('navigate')[0] ?? '';
            reset.sensitive = this._gsettings.get_user_value('navigate') !== null;
        };
        this._keysChangedId = this._gsettings.connect('changed::navigate', show);
        show();
        return page;
    }

    // A small window that takes the next chord: Escape leaves it as it was,
    // Backspace turns the key off. The shell's own shortcuts are held back
    // meanwhile, or the current key would start navigation instead.
    _captureKey() {
        const status = new Adw.StatusPage({icon_name: 'input-keyboard-symbolic', title: 'Press the new key',
            description: GLib.markup_escape_text('With Super, Ctrl or Alt. Escape cancels; Backspace turns it off.', -1)});
        const view = new Adw.ToolbarView({content: status});
        view.add_top_bar(new Adw.HeaderBar());
        const dialog = new Adw.Window({modal: true, transient_for: this._window, default_width: 440,
            default_height: 300, content: view, title: 'Navigate notches'});
        const keys = new Gtk.EventControllerKey();
        keys.connect('key-pressed', (c, keyval, keycode, state) => {
            const mods = state & Gtk.accelerator_get_default_mod_mask();
            const key = Gdk.keyval_to_lower(keyval);
            if (!mods && key === Gdk.KEY_Escape) {
                dialog.close();
            } else if (!mods && key === Gdk.KEY_BackSpace) {
                this._gsettings.set_strv('navigate', []);
                dialog.close();
            } else if (mods && Gtk.accelerator_valid(key, mods)) {
                this._gsettings.set_strv('navigate', [Gtk.accelerator_name_with_keycode(null, key, keycode, mods)]);
                dialog.close();
            }
            return Gdk.EVENT_STOP;
        });
        dialog.add_controller(keys);
        dialog.connect('map', () => dialog.get_surface()?.inhibit_system_shortcuts?.(null));
        dialog.connect('close-request', () => {
            dialog.get_surface()?.restore_system_shortcuts?.();
            return false;
        });
        dialog.present();
    }
}
