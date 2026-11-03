// The settings window's model (prefs.js): the daemon's `schema` turned into
// pages, groups and rows, and a control's value turned into what goes to
// the daemon. Pure: no GJS imports, so Node tests it
// (tests/gnome-prefs.test.mjs). The rules of the schema stay the daemon's;
// what is here is how the window shows them and the edits it sends, as
// core/settings/SettingRow.qml and its controls do in the QML window.
// `Settings` and `Config` are core/settings/Settings.js and core/Config.js
// (lib/ in the built extension), passed in where needed.

// The sidebar's icons: the schema names pages with Nerd Font glyphs, GNOME
// draws Adwaita symbolic icons.
const PAGE_ICONS = {
    general: 'preferences-system-symbolic',
    notch: 'video-display-symbolic',
    modules: 'application-x-addon-symbolic',
    components: 'package-x-generic-symbolic',
    appearance: 'applications-graphics-symbolic',
};

export function pageIcon(id) {
    return PAGE_ICONS[id] ?? 'preferences-other-symbolic';
}

// "modules/media" is under "modules".
export function parentPage(id) {
    const s = String(id ?? '');
    const at = s.indexOf('/');
    return at === -1 ? s : s.slice(0, at);
}

export function isSubPage(id) {
    return parentPage(id) !== String(id ?? '');
}

// The pages of the sidebar: the schema's that are not sub-pages, in order.
export function navPages(schema) {
    return (schema?.pages ?? []).filter(p => !p.sub && !isSubPage(p.id))
        .map(p => ({id: p.id, label: p.label, icon: pageIcon(p.id)}));
}

export function findPage(schema, id) {
    return (schema?.pages ?? []).find(p => p.id === id) ?? null;
}

// The page to show for `wanted`: itself if the schema has it, else its
// parent, else the first page. A notch's page ("notch/1") counts while
// the notch exists: the daemon fills it in only once told the window shows
// it (`settings`), so it is missing from the schema for a moment.
export function resolvePage(schema, state, wanted) {
    if (findPage(schema, wanted))
        return wanted;
    const index = notchIndexOfPage(wanted);
    if (index !== -1 && index < (state?.notches?.length ?? 0))
        return wanted;
    const parent = parentPage(wanted);
    if (findPage(schema, parent) && !isSubPage(parent))
        return parent;
    return navPages(schema)[0]?.id ?? '';
}

export function notchIndexOfPage(id) {
    const m = /^notch\/([0-9]+)$/.exec(String(id ?? ''));
    return m ? Number(m[1]) : -1;
}

// The id of the notch a notch page edits, for `edit`: the page's label (the
// daemon names a notch's page after the notch), else the state's.
export function notchIdOfPage(schema, state, pageId) {
    const index = notchIndexOfPage(pageId);
    if (index === -1)
        return '';
    return findPage(schema, pageId)?.label || state?.notches?.[index]?.id || '';
}

// Which control a field gets. Wide ones (a list of rows) take a group of
// their own; the others are one row.
const CONTROLS = {
    boolean: 'switch',
    choice: 'combo',
    integer: 'spin',
    number: 'spin',
    string: 'entry',
    color: 'color',
    modules: 'modules',
    workspaces: 'workspaces',
    notches: 'notches',
    moduleCatalog: 'catalog',
    component: 'component',
    note: 'note',
};
const WIDE = new Set(['modules', 'workspaces', 'notches', 'catalog']);

export function controlOf(field) {
    return CONTROLS[field?.type] ?? 'unsupported';
}

export function isWide(field) {
    return WIDE.has(controlOf(field));
}

// A page's groups: each section's fields in a group under its name, in the
// order the sections first appear; a wide field takes a group of its own,
// titled with its label (its help as the description). Each group is
// `{title, description, fields, wide}`.
export function pageGroups(schema, pageId) {
    const out = [];
    const titled = new Set();
    let current = null;
    for (const f of schema?.fields ?? []) {
        if ((f.page || 'general') !== pageId)
            continue;
        const name = f.section || '';
        if (isWide(f)) {
            out.push({title: f.label || name, description: f.help || '', fields: [f], wide: true});
            titled.add(name);
            current = null;
            continue;
        }
        if (!current || current.section !== name) {
            current = {title: titled.has(name) ? '' : name, description: '', fields: [], wide: false, section: name};
            titled.add(name);
            out.push(current);
        }
        current.fields.push(f);
    }
    return out;
}

// What a page looks like, without values: equal while only values,
// visibility, locks and component states change, which the rows update in
// place; anything else draws the page again.
export function pageShape(schema, pageId) {
    return JSON.stringify(pageGroups(schema, pageId).map(g => [g.title, g.description,
        g.fields.map(f => [f.type, f.key, f.label, f.help, f.options, f.display, f.min, f.max, f.step,
            f.unit, f.part, f.component, f.required, f.default])]));
}

// A field's value as a control shows it: the config's, else the default.
export function valueOf(field) {
    return field?.value !== undefined && field.value !== null ? field.value : field?.default;
}

// What an edit sends as a `set` value: null (remove the key) for the
// default, so the config never stores a default.
export function wireValue(field, value) {
    if (value === undefined || value === null)
        return null;
    if (field?.default !== undefined && JSON.stringify(value) === JSON.stringify(field.default))
        return null;
    return value;
}

// ------------------------------------------------------------- numbers
function decimals(step) {
    return (String(step).split('.')[1] ?? '').length;
}

// A spin row for an `integer` or `number`: its range and step as shown
// (a `percent` number is 0..1 shown as 0 to 100), and the unit.
export function spinSpec(field) {
    const scale = field.display === 'percent' ? 100 : 1;
    const step = Number(field.step) > 0 ? Number(field.step) : 1;
    const min = Number.isFinite(field.min) ? field.min : 0;
    const max = Number.isFinite(field.max) ? field.max : Math.max(min, 100000);
    const shownStep = Number((step * scale).toFixed(6));
    return {
        min: min * scale, max: max * scale, step: shownStep,
        digits: field.type === 'integer' ? 0 : decimals(shownStep),
        unit: field.display === 'percent' ? '%' : field.unit ?? '',
    };
}

export function toSpin(field, value) {
    const v = Number(value);
    if (!Number.isFinite(v))
        return spinSpec(field).min;
    return field.display === 'percent' ? Number((v * 100).toFixed(6)) : v;
}

// The value a spin row's number means: back from percent, snapped to the
// step from the minimum and kept in range, without float noise.
export function fromSpin(field, shown) {
    const scale = field.display === 'percent' ? 100 : 1;
    const step = Number(field.step) > 0 ? Number(field.step) : 1;
    const min = Number.isFinite(field.min) ? field.min : -Infinity;
    const max = Number.isFinite(field.max) ? field.max : Infinity;
    const base = Number.isFinite(min) ? min : 0;
    const v = Number(shown) / scale;
    const snapped = Number((base + Math.round((v - base) / step) * step).toFixed(decimals(step)));
    const out = Math.max(min, Math.min(max, snapped));
    return field.type === 'integer' ? Math.round(out) : out;
}

// -------------------------------------------------------------- choices
// A combo row's items: an option that waits on a component (`disabled`)
// says so and cannot be picked.
export function comboItems(field) {
    return (field.options ?? []).map(o => ({
        value: o.value,
        label: o.disabled ? `${o.label} (not installed)` : String(o.label ?? o.value),
        disabled: !!o.disabled,
    }));
}

export function comboIndex(field, value) {
    const items = field.options ?? [];
    const at = items.findIndex(o => o.value === value);
    return at === -1 ? 0 : at;
}

// The value the item at `index` stands for, or undefined when it cannot be
// picked (a disabled option, out of range): the control goes back.
export function comboValue(field, index) {
    const o = (field.options ?? [])[index];
    return !o || o.disabled ? undefined : o.value;
}

// --------------------------------------------------------------- text
const COLOR = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

// A text or color entry's edit, as TextControl.qml commits it: trimmed;
// empty removes the key unless the field is `required`; a color must be
// #rgb, #rrggbb or #aarrggbb. `{ok, value}`, value null for a removal.
export function textEdit(field, text) {
    const t = String(text ?? '').trim();
    if (t === '')
        return field.required ? {ok: false} : {ok: true, value: null};
    if (field.type === 'color' && !COLOR.test(t))
        return {ok: false};
    return {ok: true, value: wireValue(field, t)};
}

// The config's colors are Qt's: #aarrggbb with alpha first. As
// {red, green, blue, alpha} in 0..1, or null for anything else.
export function colorToRgba(text) {
    const s = String(text ?? '').trim();
    if (!COLOR.test(s))
        return null;
    let hex = s.slice(1);
    if (hex.length === 3)
        hex = hex.split('').map(c => c + c).join('');
    let alpha = 1;
    if (hex.length === 8) {
        alpha = parseInt(hex.slice(0, 2), 16) / 255;
        hex = hex.slice(2);
    }
    const ch = i => parseInt(hex.slice(i, i + 2), 16) / 255;
    return {red: ch(0), green: ch(2), blue: ch(4), alpha};
}

export function rgbaToColor({red, green, blue, alpha}) {
    const h = v => Math.round(Math.max(0, Math.min(1, v)) * 255).toString(16).padStart(2, '0');
    return alpha === undefined || alpha >= 1 ? `#${h(red)}${h(green)}${h(blue)}` : `#${h(alpha)}${h(red)}${h(green)}${h(blue)}`;
}

// -------------------------------------------------------------- lists
function titles(catalog) {
    const out = {};
    for (const m of catalog ?? [])
        out[m.moduleId] = m.title || m.moduleId;
    return out;
}

function capitalized(id) {
    const s = String(id);
    return s.charAt(0).toUpperCase() + s.slice(1);
}

function list(value) {
    return Array.isArray(value) ? value.map(String) : [];
}

// The config's own terminals, by id (Config.terminals).
export function terminalIds(cfg, Config) {
    return Config.terminals(cfg ?? {}, []).list.map(t => t.id);
}

// A `modules` field (ModuleList.qml): every module a notch can show (not the
// standalone ones), the switched-on ones first and in order. A row:
// `{id, title, on, known, canUp, canDown}`.
export function moduleRows(field, catalog, extraIds = []) {
    const standalone = new Set((catalog ?? []).filter(m => m.standalone).map(m => m.moduleId));
    const available = (catalog ?? []).map(m => m.moduleId).concat(extraIds)
        .filter((id, i, all) => !standalone.has(id) && all.indexOf(id) === i);
    const on = list(valueOf(field)).filter(id => !standalone.has(id));
    const names = titles(catalog);
    const ids = on.concat(available.filter(id => !on.includes(id)));
    return ids.map(id => {
        const at = on.indexOf(id);
        return {
            id, title: names[id] ?? capitalized(id), on: at !== -1, known: available.includes(id),
            canUp: at > 0, canDown: at !== -1 && at < on.length - 1,
        };
    });
}

// A `workspaces` field (WorkspaceChips.qml): a chip per workspace, 1 to 10,
// the ones known and the ones picked. `[{value, label, on}]`.
export function workspaceChips(field, known, Settings) {
    const picked = Settings.toList(valueOf(field) ?? []).map(Settings.workspaceValue);
    return Array.from(Settings.workspaceChoices(known ?? [], picked)).map(w => ({
        value: w, label: String(w), on: picked.includes(w),
    }));
}

// A `notches` field (NotchCatalog.qml): a row per notch of the state, where
// it is and what it shows. `[{index, id, summary, removable}]`.
export function notchRows(state, catalog, Settings) {
    const notches = state?.notches ?? [];
    const names = titles(catalog);
    return notches.map((n, index) => ({
        index, id: n.id,
        summary: Settings.notchSummary({edge: n.edge, moduleIds: n.modules ?? []}, names),
        removable: notches.length > 1,
    }));
}

// A `moduleCatalog` field (ModuleCatalog.qml): the modules of its `part`,
// where each shows and whether it has a page. The config's terminals are
// listed from the config (a UI with no terminal of its own reports none in
// `catalog`, so the daemon's list has them not), removable.
// `[{id, title, status, page, removable}]`.
export function catalogRows(field, schema, state, cfg, Settings, Config) {
    const catalog = schema?.modules ?? [];
    const specs = (state?.notches ?? []).map(n => ({id: n.id, moduleIds: n.modules ?? []}));
    const pageOf = id => findPage(schema, `modules/${id}`) ? `modules/${id}` : '';
    if (field.part === 'terminal') {
        return Config.terminals(cfg ?? {}, []).list.map(t => ({
            id: t.id, title: t.title || t.id,
            status: Settings.moduleStatus(t.id, specs, false, true),
            page: pageOf(t.id), removable: true,
        }));
    }
    return Array.from(Settings.catalogPart(catalog, field.part)).map(m => ({
        id: m.moduleId, title: m.title || capitalized(m.moduleId),
        status: Settings.moduleStatus(m.moduleId, specs, !!m.standalone,
            Config.valueAt(cfg ?? {}, `${m.moduleId}.enabled`) !== false),
        page: pageOf(m.moduleId), removable: false,
    }));
}

// ------------------------------------------------------- config edits
// The `set`s that turn config `from` into `to`, one per top-level key that
// differs (null removes it), as NotchService.replaceConfig sends them.
export function configSets(from, to) {
    const keys = Object.keys(from ?? {});
    for (const k of Object.keys(to ?? {})) {
        if (!keys.includes(k))
            keys.push(k);
    }
    return keys.filter(k => JSON.stringify(from?.[k]) !== JSON.stringify(to?.[k]))
        .map(k => ({key: k, value: to?.[k] === undefined ? null : to[k]}));
}

// A new notch, off the edges the others hang from (NotchService.addNotch):
// `{sets, index}`, its page being `notch/<index>`.
export function addNotch(cfg, state, Config) {
    const notches = state?.notches ?? [];
    const used = notches.map(n => n.edge);
    const free = ['bottom', 'left', 'right', 'top'].filter(e => !used.includes(e));
    const entry = {
        id: Config.freeNotchId(notches.map(n => n.id)), modules: [],
        position: {edge: free[0] ?? 'bottom'},
    };
    const next = Config.withNotchAdded(cfg ?? {}, entry);
    return {sets: configSets(cfg, next), index: next.notches.length - 1};
}

export function removeNotch(cfg, index, Config) {
    return configSets(cfg, Config.withoutNotch(cfg ?? {}, index));
}

// A terminal running your shell (Config.withTerminalAdded), its id not one
// of a module: `{sets, id}`.
export function addTerminal(cfg, catalog, Config) {
    const r = Config.withTerminalAdded(cfg ?? {}, (catalog ?? []).map(m => m.moduleId));
    return {sets: configSets(cfg, r.config), id: r.id};
}

export function removeTerminal(cfg, id, Config) {
    return configSets(cfg, Config.withoutTerminal(cfg ?? {}, id));
}

// ----------------------------------------------------------- components
// A component row's button, as the `component` IPC's first argument
// (ComponentRow.qml): remove and cancel as they are, anything else installs
// (Install, Try again).
export function componentCommand(actionId) {
    return actionId === 'remove' || actionId === 'cancel' ? actionId : 'install';
}

// The line under a component's title: its state, and its size when idle.
export function componentText(row) {
    if (!row)
        return '';
    const size = row.state !== 'working' && row.size ? ` · ${row.size}` : '';
    return `${row.text ?? ''}${size}`;
}

// -------------------------------------------------------- following
// The page the window should go to for the daemon's `state`, or '' to stay:
// the daemon moves the window (navigation's `s` on a notch, `ipc settings`)
// unless it only echoes a page this window reported within two seconds
// (`reports`: [{page, at}]), as NotchService.followSettings does.
export function pageToFollow(state, shown, reports, now) {
    if (!state?.settingsOpen || !state.settingsPage || state.settingsPage === shown)
        return '';
    if ((reports ?? []).some(r => r.page === state.settingsPage && now - r.at < 2000))
        return '';
    return state.settingsPage;
}
