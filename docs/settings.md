# Settings window

`core/settings/SettingsWindow.qml`. An ordinary toplevel window with a fixed size, so tiling compositors float it, and it takes keyboard input, which the notch overlay never does. Every change goes through `NotchService.setConfigValue`: it applies at once and is sent to the daemon (`set`), which writes the config file atomically and sends the result back; sets to one key within 120 ms are one write, so a slider drag is not a write per step.

## Fields

The window is generated from `NotchService.settingsSchema`: the daemon's `schema` message ([architecture.md](architecture.md#settings)) and the host's own fields.

- **The daemon's `schema`** holds every page and field of the core: General, Notches, the Modules page's lists, each notch's page, the Components page and its rows, and a page per module (the built-ins' from their JSON files, a user module's from its `module.json`). It builds them in `native/notchd-core/src/{schema,settings,components}.rs`. The service tells the daemon what it needs: the modules it loads itself (`catalog`: id, title, kind and fields of each built-in module, the host's and the config's terminals), the screens and the compositor's workspaces (`screens`), and the page the window shows (`settings`, which picks the notch the notch pages edit). `Settings.windowPages` and `Settings.windowFields` put the result in the window's format (the edge cards get their `preview` there: a QML file the daemon cannot name).
- **The host's `hostSettingsSchema`**, appended by the service: the host's own config keys.

The window reads the config for the value of a field (`NotchService.config`, the daemon's), and the daemon's `default`; a field that is not in the schema yet is not shown, so after an edit that adds one (a second notch) the page fills in a moment later. Each field is an object:

| Key | |
|---|---|
| `key` | config key; dotted for nested objects (`theme.accent`) |
| `type` | `choice`, `integer`, `number`, `boolean`, `string`, `color`, `modules`, `workspaces`, `notches`, `moduleCatalog`, `component`, `components` (a row per registry component that matches) or `note` (text only, no key) |
| `label`, `help` | title and description |
| `page` | sidebar page id; `general` when absent |
| `section` | heading the field sits under, within its page; sections keep the order their first field appears in |
| `default` | value shown while the key is absent; picking it removes the key |
| `options` | `[{ value, label, preview }]`, for `choice` |
| `display` | `"cards"` shows a `choice` as picture cards instead of segments, `"chips"` as chips that wrap under the label (a long list of short options); `"percent"` shows a 0..1 `number` as 0 to 100 % |
| `min`, `max`, `step`, `unit` | for `integer` and `number` (a slider; a `number`'s step may be a fraction) |
| `absolute` | in a module's fields: `key` is a config key as given, not under the module's id (a terminal's `terminals.<id>.command`) |
| `component` | for `component`: the component's id in `core/components/components.json`; title and help come from the registry unless the field has its own |
| `needs` | on an option of a `choice`: what it waits on; the option shows `disabled` (not pickable) while one isn't installed, and the field's help says which. An entry is a component id, or a registry match (`{ engine: "piper", language: "@voiceLanguage" }`: the component whose `engine` and `language` are those, `@key` being the value of that field of the module: the voice of the chosen language) |
| `when` | `{ key, is }`: the field shows only while the field `key` of the same page has the value `is` (or one of them, when it is a list); the default counts. The daemon computes `visible` (`shown_fields` in `settings.rs`); the window applies the same rule, `Settings.shownFields`, to its own optimistic config so an edit hides and shows fields at once. Voice's text-to-speech section uses it to show the voices of the chosen language only. A module's JSON may use it: `key` (and an option's `@key`) is the module's own, as its fields' keys are |
| `match`, `by` | for `components`: `match` ({ field: value or [values] }) picks the registry components, one `component` row each (`components::decorate`); `by: { key, field }` gives each row `when: { key, is: <its component's field> }` |
| `required` | for `string`: emptying it is refused (marked, as a bad color is) instead of removing the key |

| Type | Control |
|---|---|
| `choice` | `SegmentedControl`, `CardChoice` with `display: "cards"`, or `ChipChoice` with `display: "chips"` |
| `integer`, `number` | `SliderControl` |
| `boolean` | `Toggle` |
| `string`, `color` | `TextControl` (a color adds a swatch and is validated) |
| `modules` | `ModuleList` |
| `workspaces` | `WorkspaceChips`: 1 to 10, the compositor's workspaces and the picked ones; none picked removes the key |
| `notches` | `NotchCatalog`: a row per notch, where it is and what it shows (`notch_summary`), with a gear to its page, a trash can (with more than one) and Add, which opens the new notch's page; it edits the list through the service (`addNotch`, `removeNotch`) |
| `moduleCatalog` | `ModuleCatalog`: one list of the modules the host can load, by `part` (`module_kind`): `"notch"` the ones notches show, `"agent"` the agents, `"terminal"` the config's terminals, `"standalone"` the standalone ones (`catalog_part`); where each shows (`module_status`) and a gear to its page; no key. The terminals' list adds Add terminal and a trash can per row, through the service (`addTerminal`, `removeTerminal`) |
| `component` | `ComponentRow`: one optional component (see [Components](#components)), by the field's `component` id; no config key |
| `note` | none; just the title and help |

## Several notches

The Notches page (`notch`) lists the notches. Each one's gear opens its page, `notch/<index>` (`notch_page_id`), which holds every field for its own keys: name, modules, position, workspaces... Showing that page sets `NotchService.settingsNotch`, the notch the window edits. Those fields take that notch's key: `position.edge` at the top level, `notches.1.position.edge` in a list, with the top-level value it inherits as the field's default. Picking that value removes the notch's own key, so it inherits again. The General page has what applies to every notch: the hover delays and, unless they name another page, the host's fields. In Omarchy, the gear next to a notch in the bar button's panel opens the window on that notch's page.

## Modules

The Modules page lists every module the host can load (`availableModuleIds`) in up to four lists. Notch modules says where each shows: on or off with one notch, the notches that list it with several. Agents, there when the host has an [agent](modules.md#agents), says the same for each agent. Terminals, always there, the same for each [terminal of your own](modules.md#agents), with Add terminal, which makes one running your shell and opens its page, and a trash can on each. Standalone modules, there only when the host has a [standalone module](modules.md#standalone-modules), says whether each is on; a standalone module's page starts with its Enabled switch and position. Which modules a notch shows stays on each notch's page, since it is the notch's own key. A module with settings of its own has a gear that opens its page, `modules/<id>` (`module_page_id`), with its fields, keys under its id unless `absolute` (`module_fields`); a field that names another page goes there, under the module's title. The fields come from the module's JSON file ([modules.md](modules.md#settings)), or a terminal's entry, read without loading it, so a module no notch shows can be set up before it is switched on.

## Pages

`NotchService.settingsPages`: the core's (General, Notches, Modules, Components, Appearance), then the host's `hostSettingsPages`, each `{ id, label, icon }` with `icon` a Nerd Font code point, then a page per notch and per module, both the daemon's (`Settings.windowPages` merges them with the core's and the host's). A host page with a core page's id replaces its label or icon. A page with no fields is hidden. A page whose id has a `/` is a sub-page of the part before it (`notch/1` of `notch`, `modules/media` of `modules`): it has no sidebar entry, the sidebar marks its parent while it shows, a link above its fields leads back, and so does Escape. A sub-page that goes away (its notch removed) falls back to its parent. The page shown is kept on the service, so it survives closing the window; it starts on General.

## Card pictures

An option's `preview` is the url of a QML file whose root is a `CardPreview` (`core/settings/CardPreview.qml`). Draw it with theme colors; its `selected` property follows the card and `value` is its option's value, so one file can draw every option. The pictures in `core/settings/previews/` are examples; `Edge.qml` draws all four edges.

## Components

Nothing heavy ships with notchd: what a module needs beyond the app is an optional component, installed from the settings. The Components page (`components`) lists all of them under their group (Voice, a group of voices per language, Codex Live) with a `component` row each, and a module's page repeats the rows it needs (`{ type: "component", component: "localai", section: "Text to speech (TTS)" }` in its fields: `settings::local_live_settings`; Voice's voices are one `components` field, matching every Piper and Pocket TTS voice, each shown with its language). The registry is `core/components/components.json`: `id`, `title`, `description`, `group`, `size` (about, in bytes), `needed_by` (module ids) and a `kind`. A voice also has an `engine` (`piper` or `pocket`) and a `language` (`pt`, `en`, `es`, `fr`, `de`, `it`, `nl`: one voice of each engine per language), `shared_size` (the part of its `size` that another installed voice of its backend already holds: the backend, so a second language shows its own size) and, for Pocket TTS, `hf_language` (its folder in the Hugging Face weights cache):

| Kind | Installs | Where | Removal |
|---|---|---|---|
| `files` | downloads (`files`: name, url, `exec` to make it runnable), each into `<name>.part`, renamed when its size is the server's; a failed one is resumed by the next try, a cancelled one deleted | `$XDG_DATA_HOME/notchd/<dir>` | its files only; with `remove_dir`, the whole folder (LocalAI's, so its voices go too) |
| `localai` | a model of LocalAI, through its HTTP API (`localai.py` starts it for the time it takes, on `$NOTCHD_LOCALAI_URL`, by default `http://127.0.0.1:18080`): `requires` the engine, installs `backend` (a gallery id, `backend_dir` the folder it makes), then `model` (`gallery` id, `name`, `overrides` written into its configuration: the CPU backend by its own name, the options), polling each job for its bytes, then `warmup` (a first sentence, which makes the backend fetch its weights); the marker `markers/<id>.json` last | `$XDG_DATA_HOME/notchd/localai` (`models/`, `backends/`, `hf/`) | what `paths` lists, and the model's configuration; but a path another installed (or installing) component of the registry lists too (the engine's backend, the weights cache `hf/`) stays: the last one out takes it. Of `hf/` a Pocket TTS voice takes only its `hf_language` (its revisions' folders, then the blobs nothing reaches) while others are installed. A size counts a shared path once, for the first of the installed voices that hold it |
| `system` | Arch `packages`, with `pkexec pacman` when a polkit agent runs, else in a terminal that runs `sudo pacman`; an `aur` one (Voxtype) is built by `yay` or `paru` in a terminal, or the error says how | the system | never offered |

A row shows the state (`components::view`: not installed, `Downloading 42 % (83 MB of 197 MB)`, `Installing: …`, `Installed (197 MB)`, `Installed by the system`, `Failed: why`) and the buttons that fit (Install, Try again, Cancel, Remove, which asks twice). The daemon reads the state from `core/components/install.py status` (when the window opens and every second while it is open and something installs) and runs `install`, `cancel` and `remove` when a button sends the `component` IPC; the rows reach the window as the `row` of the schema's `component` fields. The installer runs the work detached (`install` returns at once), so closing the window or the shell does not stop it: the state is in `$XDG_STATE_HOME/notchd/components/<id>.json`, written atomically, and a LocalAI voice is installed only once its marker exists, written last. The rules are in `native/notchd-core/src/components.rs`; a `choice` option with `needs` is locked until its component is installed (Voice's `voiceModel` choices: `decorate` and `gate` in `components.rs`, applied by `schema.rs`: the ones that name an id, and the ones that match by a field's value, which follow the language chosen). A bridge that lacks a component says so in its error line (`components`), and the module shows "Install X in Settings › Components."

## Adding a control

Every control takes `field` (the schema entry) and `value` (the config value, or the default while the key is absent) and emits `edited(value)`; `undefined` removes the key, and `SettingRow` turns the field's default into `undefined` (`Settings.storedValue` (`stored_value` in `settings.rs`)), so the config never stores a default. `SettingRow.qml` picks the control by type: add the component there, and to the tables above. A type the daemon's schema has to know (one the core builds or checks) is added in `native/notchd-core/src/settings.rs` as well. Controls that need the width of the page (cards, lists) go under the title instead of beside it; `SettingRow.wide` decides.

## Layout of the code

| File | |
|---|---|
| `SettingsWindow.qml` | window, sidebar, page content |
| `Settings.js` | the window's own mechanics: which page is on screen and cycling, sections, a slider's step, the stored form of a value, the daemon's pages and fields put in the window's format; pure, tested in `tests/settings.test.mjs`. The schema's rules (pages, fields, list edits, module pages, summaries) are the daemon's: `native/notchd-core/src/settings.rs`, with the same cases in `tests/vectors/rules/settings.json` |
| `SettingRow.qml` | one field: text and the control for its type |
| `Label.qml`, `Icon.qml`, `Button.qml`, `IconButton.qml`, `NavItem.qml`, `ListRow.qml` | building blocks; `ListRow` is a row of the Notches and Modules lists |
| `ComponentRow.qml` | the `component` control: state, bar, buttons |
| `*Control.qml`, `CardChoice.qml`, `Toggle.qml`, `ModuleList.qml`, `ModuleCatalog.qml`, `NotchCatalog.qml`, `WorkspaceChips.qml` | the controls |
