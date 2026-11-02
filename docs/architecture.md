# Architecture

The contract for moving notchd from "QML plus plain-JS rules" to a native cross-OS core with a thin native UI per OS. Today's reference is [DEVELOPMENT.md](../DEVELOPMENT.md) and the other docs; this one says where each piece goes and how the pieces talk. Where a claim is inferred rather than read in the code, it says so. Items marked **Open** are in [Open questions](#open-questions).

Decided, not up for debate here:

- Targets Linux, macOS and Windows. Linux stays Wayland, Quickshell and QML.
- The **core is Rust** and runs as a **daemon** (the "d" in notchd). **UIs are clients** and thin: they bind state and render. The invariant "rules in plain JS, QML only binds" becomes "rules in the Rust core, UIs only bind".
- Transport is line-delimited JSON over a local socket. The CLI and IPC of [ipc.md](ipc.md) go through the same daemon.
- Modules render in three tiers: declarative primitives, a canvas, a free surface.
- Third-party modules are local processes the user writes for themselves.
- The theme is semantic tokens; each UI maps them to its OS or desktop.

## Overview

```
                                   ~/.config/notchd/config.json (watched)
                                   ~/.config/notchd/modules/<id>/   (watched)
                                              |
+----------------------+   line JSON    +-----+---------------------------------------------+
| UI client (per OS)   |<-------------->|  notchd daemon (Rust)                               |
|  Linux: Quickshell   |  Unix socket   |                                                     |
|   QML (Omarchy host  |  / named pipe  |  config  settings-model  notch-state  peek/mode     |
|   or standalone)     |                |  navigation  IPC router  module host  view builder  |
|  macOS: SwiftUI      |                |                                                     |
|  Windows: native     |                |  official modules (Rust): activities media          |
|                      |                |   resources timer claude codex codex-live local-live|
| draws primitives,    |                |                                                     |
| canvas, free surface;|                |  providers: media  resources  screens*  usage       |
| window placement,    |                |   audio  notifications  pty  components             |
| pointer and keys     |                +--+-------------+----------------+-------------------+
+----------------------+                   |             |                |
        ^                                  | stdin/stdout| spawn          | spawn
        | webview / native view            v             v                v
        | (free surface)             user module    voice sidecars    portable-pty
        |                            processes      (Python, v1)      (claude, codex,
        |                            (any language) core/live/*.py    user terminals)
        |                                            core/components/install.py
   * screens/focus: reported by the UI in v1 (see Providers)
```

One daemon serves one **instance**: a config path, a socket name and an IPC target name. The Omarchy host and a `make dev` standalone host are two instances today (two services, two notches, [hosts.md](hosts.md)); they stay two instances, each with its own socket (`$XDG_RUNTIME_DIR/notchd/<instance>.sock`). Several UIs may connect to one instance (a bar button and the window host, say). *Inferred*: the default instance `default` is the Omarchy one.

Responsibility split, the rule everything below follows:

| Core decides | UI does |
|---|---|
| which modules show, the mode of each notch copy, which peek wins, timers (hover, peek, reserve), navigation, config edits, settings pages and fields, every module's logic and view | where pixels go: windows, layer-shell or its equivalent, geometry of the card and its outline, packing notches on an edge, drag math, animation frames, theme mapping, fonts, the settings window's controls |

## Inventory

### `core/*.js` and `core/settings/*.js`

"Core" is the Rust daemon. "UI" means the file stays UI-side; on Linux that is QML-side and the file stays where it is, with its Node test. Tests for files that move are ported to Rust (same cases), and the Node file is deleted in the phase that removes its last importer.

| File | Goes to | Why |
|---|---|---|
| `Config.js` (`parse`, `valueAt`, `withValue`, `DEFAULTS`, `resolve`, `delay`, `notches`, `withNotchAdded/withoutNotch`, `terminals`, `with*Terminal`, `workspaceList`, `withWorkspace`, `moduleLoadList`...) | core | The file's rules are the invariant "a missing key means its default"; the core owns the file now. Same functions, same tests |
| `State.js`: `activeModules`, `compactModule(s)`, `hasIcon`, `attentionPeek`, `canPeek`, `mode`, `shownModules`, `wantsKeyboard`, `keyboardFocus/Change`, `reservedSpace`, `reserveStep`, `workspaceAllowed`, `notchScreen`, `reserveScreens`, `peekCopy`, `needsVisitor`, `focusedPeek`, `reserveZones`, `needsWindow` | core | Pure decisions over module and copy facts. Screens, focus and workspace come in as inputs the UI reports (below) |
| `Navigate.js` (`keyName`, `start`, `step`, `wrap`, `moveTarget`, `hint`, `number`) | core | The navigation state machine. The UI sends a normalized key (`"Escape"`, `"Tab"`, `"h"`, `"1"` plus modifiers), not Qt key codes; `KEY`/`SHIFT`/`CHORD` Qt constants are dropped. `moveTarget` calls `Position.dragTarget`-free logic only (it picks an edge from `position`), so it moves with the rest |
| `Position.js`: `resolve`, `configValue`, `isEdge`, `isVertical`, `EDGES`, defaults | core | The config's `position` value and its fallbacks are config rules |
| `Position.js`: `place`, `insets`, `nearestEdge`, `dragTarget`, `screenAt`, `stretch`, `pack`, `rowDepths`, `corners`, `cornerInput`, `coveredByStandalone` | **UI** | Pixel geometry on measured card sizes and screen rectangles; runs per frame during a drag or a growth animation, where a socket round trip is wrong. Each UI reimplements it; the Node test cases become `tests/fixtures/geometry/*.json` golden vectors every UI's tests must pass (*inferred*: `coveredByStandalone` could be core, since it only reads module facts; kept with `pack` because it feeds it) |
| `Outline.js` | **UI** | Draws the card outline (SVG path data from `Shape`); pure drawing. Same golden-vector treatment |
| `Countdown.js` | core | The timer module's whole state machine; time passed in |
| `Resources.js` | core | `/proc` and `df` parsing, pressure rules. The Linux reader is a provider; the rules (`nextPressure`, `formatUsage`) are portable |
| `Usage.js` | core | Endpoint shapes, `alerts`, `tightest`. The HTTP request moves to Rust (`reqwest`); `AgentUsage.qml`'s `XMLHttpRequest` goes |
| `Agent.js` | core | Terminal-title parsing, done-peek rule, IPC command parsing |
| `Terminal.js` | core | `shellArgs`, `expandHome`, `plainTitle` go with the PTY. `PROBE` (is qmltermwidget installed) and `isLeaveKey` stay UI-side (Linux terminal view) |
| `Live.js` | core | The bridge's argv, line parser, conversation reducer, status and error text. `localPath(url, shellDir)` is Quickshell-specific: dropped, the core knows its own sidecar path |
| `Components.js` | core | Row text, buttons, locks, `decorate`/`gate` of the settings schema. The installer stays Python in v1, run by the core |
| `settings/Settings.js`: `mergePages`, `pageOf`, `parentPage`, `isSubPage`, `visiblePages`, `navPages`, `effectiveValue`, `shownFields`, `sections`, `parseModuleInfo`, `agentSettings`, `liveSettings`, `localLiveSettings`, `terminalFields/Info`, `settingDefault`, `standaloneFields`, `moduleRefs`, `moduleFields`, `modulePages`, `moduleTitle/Kind`, `catalogPart`, `moduleStatus`, `notchPageId`, `notchIndexOfPage`, `notchSummary`, `modulePageId` | core | Building the schema and deciding what shows. The UI gets finished pages (below) |
| `settings/Settings.js`: `toggleId`, `moveId`, `allIds`, `workspace*`, `sortWorkspaces`, `toggleWorkspace` | core | They are config edits (a module list, a workspace pick). The UI sends an `edit` op with its intent (below) instead of computing the new list |
| `settings/Settings.js`: `findPage`, `resolvePage`, `cyclePage`, `moduleInfoPath`, `snapToStep`, `storedValue`, `toList` | **UI** | Control mechanics: which page is on screen, a slider's snap, QML's Repeater-sequence gotcha. `moduleInfoPath` uses `Quickshell.shellDir`: dropped, the core finds its own module files |

### Main QML

| File | Becomes |
|---|---|
| `NotchService.qml` | Split. **Core**: config load/watch/`setConfigValue`/`replaceConfig`, module registry and `moduleInfo` (the JSONs), module load list, `agentCommand`/`terminalCommand`/`liveCommand`, `requestPeek/dismissPeek/requestKeyboard/openModule/closeModule`, peek arbitration, navigation state and `navKey`, `undoMove`, `notchSpec`, `standaloneNotch`, `addNotch/removeNotch`, the whole `IpcHandler` (becomes the daemon's router), `state`. **UI**: the `Process` and `FileView` plumbing goes; it keeps `settingsOpen`, `screens`, `barEdge`, `compositor` and `Theme.provider` and reports them |
| `Notch.qml`, `NotchCopy.qml` | **Core**: a copy's hover, pin, peek, mode, hover timers (`hoverOpenDelay`, `hoverCloseDelay`, peek duration). **UI**: reports raw pointer facts; keeps the drag preview, `packShift`, the joined-ends bookkeeping |
| `NotchSurface.qml`, `NotchWindow.qml`, `NotchCard.qml`, `ReservedSpace.qml`, `KeepShown.qml` | UI (layer-shell windows, outline, drag, reserved-space windows). `ReservedSpace`'s "grow at once, shrink after 400 ms" is `State.reserveStep`: the core sends `reserve` targets, the UI holds windows |
| `NotchModule.qml`, `LiveModule.qml`, `TerminalModule.qml`, `AgentUsage.qml`, `ComponentStore.qml` | Replaced by core module logic plus the view trees below. `TerminalModule.qml` keeps its Linux-only part: the `TerminalView.qml` free surface |
| `Theme.qml`, `Hyprland/GenericCompositor.qml`, `qmldir` | UI. The provider keys of [theming.md](theming.md) become the **token set** every UI implements |
| `settings/*.qml` | UI (the settings window and its controls, one control per field type, [settings.md](settings.md)), fed pages by the core |
| `hosts/omarchy/*` | UI host. `Service.qml` maps `qs.Commons` onto the tokens and keeps the `eryk.notchd` IPC target as a thin forwarder to the daemon. `modules/osd.qml` is the one module that owns an OS service; it becomes a core module (`osd`, standalone) fed by the Omarchy host forwarding the `osd` target's `show <json>` |

### Modules

"v1 primitives" are defined in [Primitive tree](#primitive-tree-v1). The set is exactly what these modules draw today; nothing else.

| Module | Tier | Primitives and providers |
|---|---|---|
| `activities` | 1 | `row`, `column`, `text`, `icon`, `progress` (horizontal; vertical for a side edge). Input: `push`/`clear` IPC |
| `timer` | 1 | `row`, `column`, `text` (tabular numbers), `icon`, `progress`, `button`, `chip`. Input: `timer` IPC, settings. A notification through the `notify` provider |
| `resources` | 1 | `row`, `column`, `text`, `icon`, `progress`. Provider `resources` |
| `media` | 1 | `row`, `column`, `text`, `image` (cover, rounded, glyph fallback), `progress` (with `anchor` and `seek`), `button`, `equalizer`. Provider `media` |
| `claude`, `codex` | 3 (native) | `terminal` free surface for the expanded view; compact and peek in tier 1 (`row`, `icon`, `text`). Providers `pty`, `usage`. Title-driven: the core reads the terminal title the PTY reports |
| `codex-live`, `local-live` (Voice) | 1 | `row`, `column`, `text`, `icon`, `button`, `meter` (the disc that swells with the voice, `LiveModule.qml`), `list` (the transcript, scrolled to the end). Sidecars `core/live/*.py` |
| `osd` (Omarchy) | 1 | `row`, `icon`, `text`, `progress`; standalone peek only |
| user terminals (`terminals` in the config) | 3 (native) | `terminal` |

What v1 does not need, and so does not have: text input (no built-in module types into a field; `tests/qml/fixtures` does, and keeps its QML-side test), sparkline, image grids. They are the first growth items.

## Protocol: core to UI

### Transport

- **Linux, macOS**: a Unix stream socket at `$XDG_RUNTIME_DIR/notchd/<instance>.sock` (macOS: `$TMPDIR/notchd/<instance>.sock`, mode 0600, directory 0700). **Windows**: a named pipe `\\.\pipe\notchd-<instance>` with an ACL for the current user.
- One JSON object per line, UTF-8, `\n` terminated, no embedded newlines (JSON escapes them). Max line 4 MiB (cover art goes as a path or URL, never inline).
- Every message has `"t"` (its type). A request carries `"id"` (client-chosen integer); its reply is `{"t":"ok"|"err","id":..}`. Unsolicited messages have no id.
- The daemon is single-writer per connection: messages arrive in order, per type, and a UI applies them in order.
- A client that cannot parse a message or does not know its `t` ignores it and logs once (forward compatibility inside one major version). Unknown **fields** are ignored on input and never emitted.
- **Canonical form**, the one the fixtures hold both ends to: compact (no spaces), object keys in the order this document writes them (`t` first), whole numbers without a fraction (`8`, never `8.0`), an optional field left out when absent. The few fields the doc shows as `null` (`view.tree`, `state.navigation`, a copy's `peek`, `module.error`, `screens.barEdge`, a field's `locked`) are always written. Free-form objects (`config`, a module's `config`, a `provider` payload) serialize with sorted keys. Types: `native/notchd-proto`; fixtures: `tests/protocol/`.

### Handshake and version

```json
{"t":"hello","proto":1,"client":"quickshell-omarchy","clientVersion":"0.4.0","instance":"default",
 "caps":{"primitives":["row","column","text","icon","image","progress","meter","equalizer","list","button","chip","spacer","stack","canvas"],
         "surfaces":["terminal"],"webview":false,"canvas":true}}
```
```json
{"t":"welcome","proto":1,"daemon":"0.4.0","instance":"default","config":"/home/eryk/.config/notchd/config.json",
 "tokens":["foreground","background","accent","danger","success","muted","faint","line","hover","selected","surface"]}
```

- `proto` is an integer major. The daemon speaks one major; a different one gets `{"t":"err","code":"proto","want":1}` and a close. Minor additions are new optional fields and new `t`s, never required ones.
- `caps` lists what the UI can draw. A module whose view needs a primitive or surface the UI lacks gets a visible placeholder (a `text` row naming the missing capability), never a silent blank.
- After `welcome` the daemon sends `config`, `schema`, `state`, then one `view` per module slot in use, then keeps them current.
- `hello` is optional for a connection that only sends requests (the `notchd ipc` CLI skips it and gets replies only, no `welcome` or `config`). A refused handshake is the only `err` without an `id` that closes the connection. A line that does not parse, has an unknown `t` or fails validation is answered with `err` code `args` (with the request's `id` when the line has an integer one) and the connection stays open; a line over the 4 MiB limit gets the same `err` and a close.
- Socket resolution, first hit wins: `--socket`, `NOTCHD_SOCKET`, `$XDG_RUNTIME_DIR/notchd/<instance>.sock` (`$TMPDIR` when there is no runtime dir). The instance is `--instance`, `NOTCHD_INSTANCE`, else `default`. The directory is created `0700`, the socket `0600`; a socket file nobody answers on is replaced, one a daemon answers on makes a second `notchd daemon` fail.

### State: full updates per entity, not diffs

Decision: the daemon sends **whole replacements of one entity**, keyed, each with a monotonically increasing `rev`. No JSON patch.

Why: the data is small (a notch list and a handful of view trees, a few KB); the socket is local; and a diff protocol must be implemented identically in QML, Swift and C# with a bug surface bigger than the bandwidth it saves. Replacing per entity keeps each message independent (a UI that drops one recovers on the next), and per-entity granularity means a clock tick resends only one module's tree, not the whole state. Fast-changing values do not stream: a progress carries an `anchor` the UI interpolates, and an `equalizer` animates locally.

Entities and their messages:

| Message | Entity | Sent when |
|---|---|---|
| `config` | the resolved config: `{"t":"config","rev":1,"cfg":{...},"resolved":{...}}`, `cfg` as in the file, `resolved` with defaults filled for reading | the file changes or `set` runs |
| `schema` | the settings pages and fields, resolved | modules, components or the config that shows fields change |
| `state` | notches and copies: mode, pin, peek, current tab, which modules show, navigation, reserve targets | any of those changes |
| `view` | one module slot's tree | its output changes |
| `module` | one module's facts: id, title, priority, active, icon, attention, expandedWidth, urgentPeek, peekOnFocusedScreen, `error` (`null` when fine) and an optional `status` text | they change |

```json
{"t":"state","rev":41,"navigation":null,"settingsOpen":false,
 "notches":[{"id":"main","edge":"top","offset":0.5,"screen":"","draggable":true,"tabs":true,
   "modules":["media","activities","resources"],
   "copies":[{"screen":"DP-1","mode":"compact","pinned":false,"tab":"media",
              "peek":null,"shown":["media"],"covered":false,"needsWindow":true,"keyboard":false}]}],
 "reserve":[{"screen":"DP-1","edge":"top","depth":34}],
 "standalone":[{"id":"osd","copies":[{"screen":"DP-1","mode":"rest","peek":null}]}]}
```
```json
{"t":"module","id":"media","rev":7,"title":"Now playing","priority":50,"active":true,"icon":"","attention":false,
 "expandedWidth":400,"urgentPeek":false,"peekOnFocusedScreen":false,"error":null}
```
`mode` is `rest | compact | peek | expanded` (`State.mode`). `navigation` is `null`, or `{"mode":"select|move|remove","selected":0,"tab":""}` (`Navigate.step`'s state: the selected notch's index and the tab it shows, `""` for its own). `peek` is `{"module":"media","view":"peek","until":1730000000000}` (`view` is a slot name, `until` epoch milliseconds); the UI shows the module's `peek` view while it is set. The UI derives nothing from time: the core clears `peek`.

### View

A module has up to five slots, as `NotchModule` has today: `compact`, `verticalCompact`, `peek`, `verticalPeek`, `expanded`. The core sends a tree or `null` per slot.

```json
{"t":"view","module":"timer","slot":"compact","rev":112,
 "tree":{"t":"row","gap":8,"children":[
   {"t":"icon","glyph":"󰔛","tone":"accent"},
   {"t":"text","text":"24:31","tone":"foreground","size":"bodySmall","tabular":true}]}}
```
`null` tree: the module has no view for the slot (the notch falls back as it does today: `verticalCompact` missing shows a dot, `verticalPeek` missing shows `peek`). The UI creates a slot's tree only while it is shown, as Components are created today; the core still keeps it current (cheap) or, *inferred, a later optimization*, the UI sends `subscribe`/`unsubscribe` per `(module, slot)` and the core stops building unseen ones. v1: always sent.

### UI to core

```json
{"t":"screens","screens":[{"name":"DP-1","x":0,"y":0,"w":2560,"h":1440,"scale":1}],"focused":"DP-1",
 "fullscreen":["DP-1"],"workspaces":{"DP-1":{"id":"2","name":"2"}},"known":["1","2","3"],"supportsWorkspaces":true,
 "barEdge":"top"}
```
Sent on hello and on any change. This is the adapter contract of `GenericCompositor.qml`, reported instead of called (`hasFullscreen`, `activeWorkspace`, `knownWorkspaces`, `supportsWorkspaces`), plus the screen list and `barEdge`. The core runs `State.notchScreen`, `workspaceAllowed` and `peekCopy` on it.

```json
{"t":"pointer","notch":"main","screen":"DP-1","ev":"enter"}
{"t":"pointer","notch":"main","screen":"DP-1","ev":"leave"}
{"t":"pointer","notch":"main","screen":"DP-1","ev":"press"}
{"t":"drag","notch":"main","screen":"DP-1","ev":"drop","position":{"edge":"left","offset":0.3,"screen":"DP-2"}}
{"t":"key","notch":"main","screen":"DP-1","key":"Tab","mods":["shift"],"text":""}
{"t":"event","module":"timer","slot":"expanded","action":"toggle","value":null}
{"t":"event","module":"media","slot":"expanded","action":"seek","value":0.42}
{"t":"tab","notch":"main","screen":"DP-1","module":"timer"}
{"t":"settings","open":true,"page":"modules/media"}
{"t":"reserve-applied","screen":"DP-1","edge":"top","depth":34}
```

- `pointer`: raw facts; the core owns `hoverOpenDelay`, `hoverCloseDelay` and decides `mode`. `press` pins (`State`'s click rule).
- `settings`: `page` is optional (absent: keep the page shown).
- `drag`: `ev` is only `drop` in v1. The UI computes the drop (`Position.dragTarget`, `screenAt`) and reports the result; the core stores `position` (config edit, never writes a default). While dragging the UI shows its own preview; the core's mode does not change until the drop.
- `key`: while a notch holds the keyboard. `key` is a normalized name (`"a"`, `"Escape"`, `"Tab"`, `"Left"`, `"Return"`, `"1"`); `mods` from `shift|ctrl|alt|meta`. In navigation and while a module with a text field is focused the core replies by changing `state`; keys the core does not use come back as nothing.
- `event`: a primitive's `action` fired. `value` is always present: `null`, a number (a `progress` seek, as a fraction), a string, or a boolean; never an object or array.
- `reserve-applied`: optional ack for `reserve`, to let the core run `reserveStep`'s "shrink 400 ms after collapse" against what is actually mapped.

### Requests (UI, CLI, scripts)

```json
{"t":"set","id":1,"key":"hoverOpenDelay","value":200}
{"t":"set","id":2,"key":"position","value":null}
{"t":"edit","id":3,"op":"moveModule","notch":"main","module":"media","delta":1}
{"t":"edit","id":4,"op":"toggleModule","notch":"main","module":"timer"}
{"t":"edit","id":5,"op":"toggleWorkspace","notch":"main","workspace":"2"}
{"t":"ipc","id":6,"method":"timer","args":["3m Tea"]}
{"t":"ipc","id":7,"method":"state","args":[]}
{"t":"ok","id":6,"result":"ok"}
{"t":"err","id":6,"code":"module","message":"timer module not loaded"}
```

- Replies: `ok` carries `result` (the method's string) for `ipc` and leaves it out for `set` and `edit`. `err` has `code` (one of `proto`, `module`, `method`, `args`, `key`, `config`), an optional `message`, and, only for a refused handshake, no `id` and `want`.

- `set`: `Config.withValue` semantics. `value: null` (a `null` on the wire is JS's `undefined` here) removes the key and empties parents. This is today's `set <key> <json>`; the CLI parses its argument and sends it.
- `edit`: the list edits of `Settings.js` as intent ops; the core computes and writes. v1's ops are `moveModule` (`delta`, an integer, negative towards the start), `toggleModule` and `toggleWorkspace`; a new op is a minor addition, and the core answers one it does not know with `err` code `args`. A request's `id` is an unsigned integer.
- `ipc`: every method of [ipc.md](ipc.md) (`push`, `clear`, `open`, `toggle`, `close`, `openNotch`, `agent`, `terminal`, `timer`, `live`, `navigate`, `settings`, `recenter`, `state`) with the same strings as replies. The `notchd` CLI is `notchd ipc <method> args...` and nothing else. The host's own IPC (`omarchy-shell eryk.notchd ...`, `qs ipc call notchd ...`) stays: a one-line `IpcHandler` per method in the host QML forwards to the daemon (`ipc` request) and returns its reply. `ipc.md` does not change.

### Settings

The schema is core-built; the settings window is a UI.

```json
{"t":"schema","rev":3,
 "pages":[{"id":"general","label":"General","icon":"","sub":false},
          {"id":"modules/media","label":"Now playing","icon":"","sub":true}],
 "fields":[{"page":"general","section":"Hover","key":"hoverOpenDelay","type":"integer","label":"Open delay",
            "min":0,"max":2000,"step":20,"unit":"ms","default":120,"value":120,"visible":true,"locked":null}]}
```

Field types are the existing ones ([settings.md](settings.md); each is one control plus one case in `SettingRow.qml`): `choice`, `integer`, `number`, `boolean`, `string`, `color`, `modules`, `workspaces`, `notches`, `moduleCatalog`, `component`, `components` and `note`. A field also carries, when set, `help`, `options` (`[{value,label,preview?,disabled?}]`, `disabled` being `needs` resolved), `display` (`cards|chips|percent`), `when` (`{key,is}`), and for a `component` its `component` id and the `row` the core computes (free-form in v1); `key` is absent on a `note` and a `component` row. `match`/`by` and `needs` are resolved by the core and never reach the UI. A `null` `default` or `value` is left out. `value` is current, `default` from the schema, `visible` is `Settings.shownFields`'s answer, `locked` is `Components.lock`'s reason or `null`. The UI sends `set` or `edit` and waits for the next `schema`. Component rows (`type: "component"`) carry the view `Components.view` computes (title, status, size text, buttons as action ids), and a button press is `{"t":"ipc","method":"component","args":["install","voice-piper-pt-br"]}`; progress arrives as `schema` updates, read by the core from the installer's state files, as `ComponentStore.qml` does today.

Theme is **not** in the protocol. Each UI maps the token set to its own source and applies it (`Theme.provider` today).

### Primitive tree (v1)

Every node: `"t"` (the primitive) and optional `"id"` (stable across updates, so a UI may animate a changed node in place), `"visible"` (default true), `"action"` (string, fires `event` on press/tap; absent means not interactive), `"tooltip"`, `"fill"` (in a `row` or `column`, the node takes the rest of the space), written in that order before the node's own fields. Sizes are **design pixels**, which each UI multiplies by `Theme.space()`'s `spacingScale` (`Theme.space(px)` today). Colors never appear: `tone` is one of `foreground | muted | faint | accent | danger | success | line`. Text sizes: `caption | bodySmall | body | heading | display` (theming.md's five).

| `t` | Fields | Notes |
|---|---|---|
| `row` | `gap`, `align` (`start\|center\|end`), `children` | horizontal layout. `fill: true` on a child makes it take the rest |
| `column` | same | vertical layout |
| `stack` | `align`, `children` | overlay, centered by default: the Voice disc and its glyph |
| `spacer` | `size` | fixed gap, or `fill: true` (the common property) for a flexible one |
| `text` | `text`, `tone`, `size`, `tabular`, `elide` (`end\|none`), `wrap`, `maxWidth`, `align`, `lines` | always plain text, never rich (the code's `textFormat: Text.PlainText`) |
| `icon` | `glyph` (a codepoint string, Nerd Font), `tone`, `size` | v1 icons are Nerd Font glyphs, as today. *Open*: native symbol sets per OS |
| `image` | `src` (absolute path or `file://` / `https://`), `size` or `w`/`h`, `radius` (`none\|small\|round`), `fit` (`crop\|contain`), `fallback` (an `icon` node) | cover art: the UI draws the fallback while loading and on failure, and rounds the corners without a shader where it has none (the software-render rule in CLAUDE.md applies to every UI) |
| `progress` | `value` (0..1; absent or `null`: no bar), `orientation` (`h\|v`), `tone`, `thickness`, `anchor`, `seek` (action id) | `anchor`: `{"at":<epoch ms>,"rate":<fraction per second>}`: the UI interpolates `value + rate * (now - at)` itself, so the media position needs no stream. `seek` fires `event` with a fraction |
| `meter` | `value` (0..1), `style` (`disc`), `size`, `child` (a node drawn inside) | `disc`: the Voice disc that swells with the voice (`1 + value * .35`, 90 ms ease). The core sends the normalized level; the UI animates |
| `equalizer` | `active`, `mode` (`smooth\|light`), `bars` (default 4), `height` | the media module's decorative bars (the code's "fake equalizer"): the UI generates the motion, the core sends only `active` and `mode`. Not a canvas, so no 60 Hz stream |
| `list` | `gap`, `stick` (`start\|end`), `maxHeight`, `children` | a scrolling column; `stick: "end"` keeps the newest line in view (the transcript) |
| `button` | `glyph` or `label`, `action`, `tone`, `enabled`, `selected`, `variant` (`icon\|outline`) | the `IconButton`, the Voice start button and the timer's controls |
| `chip` | `label`, `value` (a string), `action`, `selected` | the timer's presets |
| `canvas` | `w`, `h`, `cmds`, `animate` | tier 2, below |
| `surface` | `kind` (`terminal\|webview`), `ref`, `w`, `h` | tier 3, below |

**Notch-level props the module sets** (in `module`, not in a tree): `priority`, `active`, `icon`, `attention`, `expandedWidth`, `urgentPeek`, `peekOnFocusedScreen`, all of `NotchModule.qml`'s. They stay core facts.

A UI validates the tree against the schema it implements (`proto`'s JSON Schema in `native/notchd-proto/schema/view.schema.json`, shared); an invalid node renders as an error row, not a crash. Beyond the schema the core enforces limits (`notchd-proto::version`): 2 000 nodes, depth 16 and 256 KiB per `view` (Open question 13's suggestion, adopted); `progress.value` and `meter.value` within 0..1; an `image`'s `fallback` being an `icon`; a `canvas` with positive `w` and `h` and `animate.fps` in 1..30.

### Canvas (tier 2)

A `canvas` node is a fixed-size box drawn from a flat list of vector commands in design pixels and token tones. v1 has the commands the UIs can all draw in a few lines each; it is for custom visuals a module has no primitive for (an audio visualizer, a graph), and growing the primitives is the first answer to "I need a canvas".

```json
{"t":"canvas","w":120,"h":28,"cmds":[
 {"c":"rect","x":0,"y":0,"w":120,"h":28,"r":6,"fill":"selected"},
 {"c":"path","d":"M0 20 L10 12 L20 16 L30 6","stroke":"accent","width":1.5,"cap":"round","join":"round"},
 {"c":"circle","x":60,"y":14,"r":4,"fill":"danger","opacity":0.8},
 {"c":"line","x1":0,"y1":27,"x2":120,"y2":27,"stroke":"line","width":1},
 {"c":"text","x":4,"y":4,"text":"cpu","size":"caption","tone":"muted"}]}
```

Commands: `rect` (rounded, fill and/or stroke), `circle`, `line`, `path` (SVG path data, `M L Q C Z`, the subset `Outline.js` emits), `text`, `clip` (`{"c":"clip","x":..,"y":..,"w":..,"h":..,"r":..}`, a rounded rectangle, ... `{"c":"unclip"}`); the drawing commands take `opacity` (0..1), `width` is the stroke width, `cap` is `butt|round|square`, `join` `miter|round|bevel`. Colors are any **token** of `welcome.tokens` (`foreground background accent danger success muted faint line hover selected surface`; a node's `tone` is only the seven listed above) or `{"mix":["accent","background",0.4]}` (`Theme.mix`), never hex. Animation: `animate: false` (default) redraws only when the core sends a new `view`; `animate: {"fps":30}` lets the core stream at most that rate (the core caps at 30). *Inferred, not decided*: gradients, images in a canvas and `fill-rule` wait for a module that needs them.

### Free surface (tier 3)

A `surface` is a rectangle the UI hands to something that draws itself.

| `kind` | For | Lifecycle |
|---|---|---|
| `terminal` | official modules (`claude`, `codex`, user terminals) | The **PTY lives in the core** (`portable-pty`): the program starts with the first `surface`, lives as long as the module does (as `TerminalModule.qml` says today: "its session lives as long as the view does, wherever the view is put") and survives the notch closing. The UI draws a terminal view bound to it: Linux keeps `qmltermwidget` and speaks to the PTY through the daemon's PTY bridge (below); macOS and Windows get their own view |
| `webview` | user modules that need their own HTML (the escape hatch; a module needing it signals a missing primitive) | The UI opens a sandboxed webview, no network by default, no file access, loading the module's `index.html` from its folder through a custom scheme; the module sends/receives JSON through a `postMessage` bridge the daemon relays as `surface` messages to the module process |

```json
{"t":"surface-open","ref":"term:claude","kind":"terminal","cols":80,"rows":24}
{"t":"surface","ref":"term:claude","ev":"data","b64":"G1s..."}
{"t":"surface","ref":"term:claude","ev":"input","b64":"bHMK"}
{"t":"surface","ref":"term:claude","ev":"resize","cols":100,"rows":30}
{"t":"surface","ref":"term:claude","ev":"title","text":"⠋ Fix tests | notchd"}
{"t":"surface-close","ref":"term:claude"}
```

`ev` is `data` or `input` (`b64`), `resize` (`cols`, `rows`) or `title` (`text`).

- `ref` names the surface; the core sends `data` (PTY output) and `title` (OSC 0/2, which the agent modules read to decide the compact view and the done peek, `Agent.js`), the UI sends `input` and `resize`. Lines carry base64 PTY bytes; a surface's traffic is **not** the line-JSON state channel in practice: for `terminal` the daemon opens a second connection per surface (`{"t":"attach","ref":"term:claude"}`) so a flood of output cannot delay a `view` update. *Inferred design, to confirm in Phase 4.*
- `surface-open` on first need; the UI answers with its capabilities (`surfaces` in `hello`). Without one, the module's view shows a fallback row ("the terminal needs the qmltermwidget package" today, `TerminalModule.qml` and ipc.md's replies).
- A webview's close, crash or unresponsiveness (watchdog 5 s) shows the module's error row; the module process is untouched.

## Protocol: core to module (user modules)

Third-party modules are local programs for yourself. No sandbox in v1 (**deferred**); `permissions` in the manifest is documentation shown in the settings page, not enforced.

### Folder

```
~/.config/notchd/modules/<id>/
  module.json       the manifest
  run.sh            whatever `command` starts: any language
  (assets, index.html for a webview surface)
```

The folder name is the id. An id may not be a config key (`modules`, `position`, `theme`...) or a built-in id, as modules.md says today.

### Manifest

```json
{
  "title": "Weather",
  "command": ["./run.sh"],
  "priority": 40,
  "icon": "󰖐",
  "expandedWidth": 360,
  "permissions": ["network"],
  "settings": [
    { "key": "city", "type": "text", "label": "City", "default": "Berlin" }
  ],
  "subscribe": ["resources"],
  "standalone": false
}
```

| Field | Meaning |
|---|---|
| `title` | shown in settings and the tab. Defaults to the id |
| `command` | argv array, run with the module's folder as cwd; `${config}` and `${cache}` expand to the module's own config and cache dirs. Required |
| `priority`, `icon`, `expandedWidth`, `urgentPeek`, `peekOnFocusedScreen`, `standalone` | the `NotchModule` properties of the same name; module can change the live ones in `set` (below) |
| `settings` | the same field format as built-in module JSON (`modules.md`, "Settings"): `key`, `type`, `label`, `default`, `help`...; the same page appears in the settings window; values live under the module's id in the config file |
| `subscribe` | provider names the module wants pushed to it (below) |
| `permissions` | `network`, `files`, `exec`, ...: documentation only |
| `env` | extra environment variables, a string map |

`module.json` is read by the core whether the module runs or not, so its settings page shows, as today.

### Messages

Stdin and stdout, line-delimited JSON, UTF-8. Stderr is the log, kept per module (below). The module sends first or second; order does not matter.

Core to module:
```json
{"t":"init","proto":1,"id":"weather","config":{"city":"Berlin"},"tokens":["foreground","muted","accent"],
 "slots":["compact","verticalCompact","peek","verticalPeek","expanded"],"dir":"/home/eryk/.config/notchd/modules/weather"}
{"t":"config","config":{"city":"Paris"}}
{"t":"event","slot":"expanded","action":"refresh","value":null}
{"t":"provider","name":"resources","data":{"cpu":0.12,"disk":{"path":"/","total":500000000000,"used":300000000000},"mem":{"total":16000000000,"used":6500000000}}}
{"t":"cmd","method":"push","args":["..."]}
{"t":"visible","slot":"expanded","visible":true}
{"t":"stop"}
{"t":"error","code":"provider"}
```

Module to core:
```json
{"t":"set","active":true,"priority":40,"icon":"󰖐","attention":false}
{"t":"view","slot":"compact","tree":{"t":"row","gap":6,"children":[{"t":"icon","glyph":"󰖐","tone":"accent"},{"t":"text","text":"18°"}]}}
{"t":"view","slot":"expanded","tree":{"t":"column","gap":8,"children":[
   {"t":"text","text":"Berlin","size":"heading"},
   {"t":"button","label":"Refresh","action":"refresh"}]}}
{"t":"peek","ms":3000,"slot":"peek"}
{"t":"log","level":"info","text":"fetched"}
{"t":"error","message":"network down"}
{"t":"notify","title":"Rain","body":"in 10 minutes"}
{"t":"open"}
```

- `view` takes the same tree as the core to UI `view`; the core forwards it unchanged after validating (an invalid tree is an `error` row, the module keeps running). A slot never sent has no view.
- `set` takes `active`, `priority`, `icon`, `attention`, `urgentPeek`, `expandedWidth` (the changeable `NotchModule` properties), every one optional.
- `error` from a module is `{"t":"error","message":".."}`; `notify`'s `body` is optional.
- `peek` is `requestPeek(ms)`, `slot` optional (default `peek`); `open` is `openNotch()`; `notify` goes through the `notifications` provider. A module **never takes over a system service**: the invariant stands, the notification is shown by the OS, the module only posts one.
- `event` carries any `action` the module's tree defined. `cmd` is the module's own IPC (`omarchy-shell eryk.notchd module <id> <method> args...`: *new*, user modules get a hook into `ipc.md`'s shape).
- Unknown `t` is ignored, in both directions.

### Periodic updates

The module owns its timer: it prints a `view` when it wants. The core does not poll modules. For animated values the module uses `progress.anchor` and `equalizer`, not a stream. Rate limit: the core drops views beyond 20 per second per module and logs one warning; `canvas.animate` is the declared exception (30 fps cap).

### Provider subscriptions

`subscribe: ["resources"]` in the manifest, or at runtime `{"t":"subscribe","name":"media"}`. The core then sends `provider` messages on change (throttled per provider: `resources` 1 s, `media` on change, `screens` on change). A module may `{"t":"unsubscribe","name":"..."}`. A provider name the core does not know is answered with `{"t":"error","code":"provider"}` to the module (and logged); the manifest's `subscribe` is checked against the known names when it is read. Provider payloads are the same shapes as the built-in modules consume (below) and are part of `proto`.

### Lifecycle

- **Start**: when a notch lists the module and the config does not disable it (`"<id>": {"enabled": false}`), or at daemon start for a standalone one. The core spawns `command`, writes `init`, waits for the first `view`. No view in 5 s: the module's compact slot shows an error row (`"weather: no view after 5 s, log: <path>"`) until one arrives.
- **Reload on save**: the core watches the folder (`notify` crate, debounced 150 ms). Any change to `module.json` or a file under the folder: `stop`, then SIGTERM after 1 s, SIGKILL after 3 s, then start again. A manifest error shows an error row with the line number; the old process is left running until a good manifest is saved, so a typo does not take the notch down.
- **Crash**: exit code or signal is shown in the notch: the module's slots are replaced by `{"t":"row","children":[{"t":"icon","glyph":"","tone":"danger"},{"t":"text","text":"weather exited (1)  ~/.local/state/notchd/logs/weather.log","tone":"danger","elide":"end"}]}` and `attention` is set. Restart with backoff 1, 2, 4, ... 30 s, reset after 60 s of staying up; a **Restart** button (`event` action `restart` on the error row) restarts at once. Logs: stderr and `log` messages to `$XDG_STATE_HOME/notchd/logs/<id>.log`, rotated at 1 MiB, last 5 kept (*inferred paths, Windows `%LOCALAPPDATA%`*).
- **Stop**: `{"t":"stop"}`, then the same SIGTERM and SIGKILL. A module not listed by any notch is stopped.
- A line over 1 MiB, or non-JSON on stdout, is logged and skipped; ten in a row kill the module as a crash.

## Providers

A provider is a core component that produces facts for official modules and, by subscription, user modules. The facts have one shape across OSes; the backend per OS differs.

| Provider | Shape | Linux today | macOS (later) | Windows (later) |
|---|---|---|---|---|
| `media` | players, each `{id,name,playing,title,artist,album,artUrl,length,position,canPlay,canPause,canNext,canPrev,canSeek}`, plus commands (`play`, `pause`, `next`, `previous`, `seek`) | MPRIS over D-Bus (`zbus`); today `Quickshell.Services.Mpris` in `media.qml`, and the demo player's MPRIS in `tests/visual/demo-player.py` | MediaRemote (private framework, unstable across releases: **Open**) or per-app AppleScript | GlobalSystemMediaTransportControls (WinRT) |
| `resources` | `{cpu,mem:{used,total},disk:{used,total,path}}` | `/proc/stat`, `/proc/meminfo`, `df -P -B1` (the rules are `Resources.js`; the core does the I/O) | `host_processor_info`, `host_statistics64`, `statvfs` | PDH counters, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceEx` |
| `screens` | the `screens` message above (names, rects, scale, focused, fullscreen, workspaces) | **UI-reported** in v1: Quickshell `Screen` plus the `*Compositor.qml` adapters. CLAUDE.md puts compositor code only in those adapters, so the Hyprland adapter stays QML | the UI knows (`NSScreen`) | the UI knows |
| `usage` | an agent's plan limits `[{id,label,percent,resetsAt,windowSecs}]`, stale and problem text | `~/.claude/.credentials.json`, `~/.codex/auth.json` read by the core, HTTPS to the endpoints `Usage.js` names | the same files; credentials in the Keychain: **Open** | the same files |
| `pty` | a PTY per terminal with title events | `portable-pty` (forkpty) | `portable-pty` | `portable-pty` (ConPTY) |
| `audio` | the Voice's microphone and voice output (`pacat`/`parec` today, via the sidecar); a `level` feed (dB) for the `meter` | the sidecars call `parec`/`pacat`; the `libpulse` component is their dependency | CoreAudio (`cpal`) | WASAPI (`cpal`) |
| `notifications` | `notify(title, body)` | the timer shells out to `notify-send` today (`Process { id: notifier }` in `timer.qml`) | `UNUserNotificationCenter` or `osascript` | toast notifications |
| `components` | the registry and per-component status | `core/components/install.py` (pacman, AUR, files, LocalAI): Python in v1, run by the core, state files in `$XDG_STATE_HOME/notchd/components/` | later (brew, downloads); the registry grows an OS key per component (`kind: system` is Arch-only today: **Open**) | later |
| `osd` (Omarchy host) | `{icon,message,value,max}` | the host forwards `omarchy-shell osd show <json>` | n/a | n/a |

Payload types are in `notchd-proto::provider` (`MediaData`, `ResourcesData`, `UsageData`, `OsdData`, `NotifyData`; `screens` is the `screens` message), one fixture each under `tests/protocol/provider/`. Providers are Rust traits in `notchd-providers`, one backend module per OS behind `cfg(target_os)`; a module asks the trait, never the OS. A provider with no backend on an OS reports `unavailable` and the module shows its empty state.

Sidecars (`core/live/codex-live.py`, `local-live.py`) are Python, launched by the core as `Live.bridgeArgs` does and speak the same line-JSON documented in their docstrings (`{"status"...}`, `{"transcript"...}`). The core reduces those lines (`Live.reduce`) and builds the Voice view. Porting them to Rust is a later phase, out of this plan.

## Config

Unchanged for users: `~/.config/notchd/config.json` (`$XDG_CONFIG_HOME/notchd/config.json`; `%APPDATA%\notchd\config.json` and `~/Library/Application Support/notchd/` or `~/.config/notchd/` on the others: **Open**). The daemon is its only reader and writer. The rules of CLAUDE.md carry over as tests:

- A missing key means its default. Defaults live once, in `Config::DEFAULTS` (ported from `Config.DEFAULTS`) or the module's schema in `module.json`. **Never write a default into the user's file.**
- `set` with `null` removes the key and any parent object left empty (`withValue`).
- The file is watched. A good edit applies live. A parse error keeps the last good config and sends `{"t":"config-error","message":"..."}`; the UI shows it (the settings window's top note) and the file is not touched.
- Writes are atomic (temp file in the same dir, rename), so the watcher never reads half a file, and a write the daemon made is not reapplied as an external edit.
- `NOTCHD_CONFIG` overrides the path, for `make dev` and every test; nothing under `tests/` touches a real user path (hermetic invariant).

## Distribution

The Omarchy plugin is the repo: `omarchy plugin add <git url>` does `git clone` and nothing else (read in `omarchy-plugin-add`: no build hook). `omarchy plugin validate` requires a manifest with `kinds`, `entryPoints` paths that exist and are relative and safe, a non-reserved id, and **no symlink anywhere in the folder** (read in `omarchy-plugin-validate`). So the plugin cannot symlink a binary, and nothing builds Rust at install. `shell.qml` at the root is the standalone host.

How a host finds and launches the daemon (**Open**; options):

| Option | How | For | Against |
|---|---|---|---|
| A. Binary on `PATH` | host runs `notchd daemon --ensure`; a package provides it (AUR `notchd`, distro, Homebrew, winget, `cargo install`) | normal OS practice; one binary for both hosts; the daemon can outlive the shell; a systemd user unit or launchd agent can socket-activate it | the plugin alone no longer works: `omarchy plugin add` plus a package |
| B. Plugin downloads a pinned release | `Service.qml` checks `$XDG_DATA_HOME/notchd/bin/notchd`, else downloads the release matching `manifest.json`'s `version`, verifying a SHA-256 kept in the repo, using the components installer's `files` kind (`install.py`, how LocalAI is fetched today) | `omarchy plugin add` stays the one step; the core version always matches the plugin's QML | a download on first run, in the shell's process; a checksum file to maintain per release; offline installs fail |
| C. Build from source on first run | `cargo build --release` from the plugin folder | no release infra | needs a Rust toolchain, minutes of build, a build step inside a shell plugin |
| D. Commit prebuilt binaries | `bin/linux-x86_64/notchd` in the repo | zero steps, offline | binary blobs in git, one per arch, nothing reviewable; `validate` allows it (no symlinks needed) |

**Recommendation**: A as the primary path, B as the bootstrap fallback when `notchd` is not found. The host QML runs `notchd daemon --ensure`, found by order: `$NOTCHD_BIN`, the plugin's own `bin/` if present, `$XDG_DATA_HOME/notchd/bin/notchd`, then `PATH`. B downloads into the second-to-last place. Either way, the daemon is single-instance per socket (it holds the socket; a second `--ensure` returns at once), and the UI reconnects on drop with backoff, showing a "notchd not running" state, never a blank screen. The version handshake (`proto`) is the guard against a stale daemon: an old daemon with a new plugin gets `{"t":"err","code":"proto"}` and the host shows "update notchd".

## Migration plan

Rule for every phase: **Linux behaviour is identical at its end and `make check` passes** (all layers, CLAUDE.md). The strategy is a strangler: the Rust core grows a slice at a time behind a feature the QML reads through `DaemonClient`; the QML rules it replaces are deleted only after the slice's QML tests pass against the daemon.

**Status.** Tasks 0.1 and 0.2 are done: all six crates exist under `native/` (`native/Cargo.toml`, `native/rust-toolchain.toml`), `make rust` runs, and `notchd-proto` has every type of this protocol with fixtures (`tests/protocol/`: `to-ui/`, `to-daemon/`, `to-module/`, `from-module/`, `tree/`, `canvas/`, `provider/`, `manifest/`, one file per message). Phase 1's ports ran ahead of 0.3: every rule file in the inventory is in `notchd-core`, with the `position` config half, and JS semantics live once in `native/notchd-core/src/js.rs`. The shared `tests/vectors/rules/` check JS and Rust against the same cases (`knownDiff` marks the accepted differences). Phase 2.1 to 2.4 are done: the QML client, the daemon core loop deciding in shadow (`store`, `peek`, `timers`, `hub`), and the user-module host.

A new `make rust` (`cargo fmt --all --check`, `clippy -D warnings`, `cargo test --workspace`) joins `make test`'s unit layer and `make check`. Every Rust test is hermetic (temp `XDG_*`, `NOTCHD_CONFIG`, random socket names under a temp dir).

**Ownership rule for parallel agents**: a task owns the paths listed; it edits nothing outside them except by adding a re-export line to its crate's `lib.rs` (which Task 0.1 pre-creates as one `pub mod` per rule file, so no two tasks touch the same line). A task that needs a change in another's path asks the owner through a protocol or fixture change in `native/notchd-proto` (Task 0.2's owner), not by editing it.

### Phase 0: scaffold (serial, one agent each, in this order)

**0.1 Cargo workspace.** Owns: `Cargo.toml`, `rust-toolchain.toml`, `native/`, `.cargo/`, `Makefile` (the `rust` target only), `.gitignore` (`target/`).
Crates, each with `lib.rs` listing its modules as empty `pub mod`s so later tasks fill separate files:
- `notchd-proto`: message types (serde), JSON Schemas, version constants
- `notchd-core`: `config`, `settings`, `state`, `navigate`, `position` (config half), `countdown`, `resources`, `usage`, `agent`, `terminal`, `live`, `components`, `peek`
- `notchd-modules`: one module file per official module
- `notchd-providers`: traits and `linux/` backends
- `notchd-daemon`: binary `notchd`: `daemon`, `ipc` (the CLI) and `--ensure`
- `notchd-testkit`: a fake UI client (`FakeUi`: connects, records `view`/`state`, sends `pointer`/`key`), a fake module process, temp-dir helpers, golden-file loader
Done when: `make rust` passes on empty crates, and `make check` includes it.
**0.2 Protocol crate and fixtures.** Owns: `native/notchd-proto/`, `tests/protocol/` (JSON fixtures: one file per message in this document, `*.json` with `"valid"` and `"invalid"` cases), `native/notchd-proto/schema/`.
Depends on 0.1. Types for every message in this doc, the primitive tree and its JSON Schema, round-trip tests against the fixtures. The fixtures are the contract the QML client and Rust daemon are both tested against. Done when each fixture parses and re-serializes byte-identically (canonical key order).
**0.3 Daemon skeleton.** Owns: `native/notchd-daemon/`, `native/notchd-testkit/`.
Depends on 0.2. A listener on the socket, handshake, `config` echo from a file, `ipc` router with an `unknown method` reply, `FakeUi`. Test: `FakeUi` connects to a daemon in a temp dir and sees `welcome` and `config`; a bad `proto` is refused.
*Done (std threads, one per connection; `--ensure` returns 0 at once when a daemon answers, else runs the daemon in the foreground; `config` is read on each `hello`, not yet watched; the router answers every `ipc` method with `err` code `method`: `unknown method: x`, or `x is not implemented yet` for a documented one, and `set`/`edit` with a not-implemented `err`; input messages are accepted and ignored).*

### Phase 1: the rules, ported (fully parallel after 0.3; no UI change)

Each task ports one group of JS rules to a Rust module with the **same cases as its Node test** (copied as the Rust test's table; cases that were `assert.deepEqual` on plain values become `serde_json::Value` comparisons so the data shapes stay the protocol's). The JS files and Node tests stay: Linux behaviour is unchanged, the Rust is not yet connected. Added in each: a `tests/vectors/<name>.json` extracted from the Node test, run by **both** Node and Rust, so the two stay equal while both exist (the differential check). Where a rule's output shape differs from today's JS (a Qt key code becoming a key name), the vector says so.

| Task | Owns (Rust) | Ports | Node tests that seed it |
|---|---|---|---|
| 1.1 config | `notchd-core/src/config.rs`, `settings/*` (schema and list edits) | `Config.js`, `settings/Settings.js` | `config.test.mjs`, `settings.test.mjs` |
| 1.2 state | `notchd-core/src/state.rs`, `navigate.rs`, `position.rs` (config half) | `State.js`, `Navigate.js`, `Position.js` `resolve`/`configValue` | `state.test.mjs`, `navigate.test.mjs`, the config half of `position.test.mjs` |
| 1.3 timer, resources | `countdown.rs`, `resources.rs` | `Countdown.js`, `Resources.js` | `countdown.test.mjs`, `resources.test.mjs` |
| 1.4 agents | `usage.rs`, `agent.rs`, `terminal.rs` | `Usage.js`, `Agent.js`, `Terminal.js` | `usage.test.mjs`, `agent.test.mjs` |
| 1.5 voice, components | `live.rs`, `components.rs` | `Live.js`, `Components.js` | `live.test.mjs`, `components.test.mjs` |
| 1.6 geometry vectors | `tests/vectors/geometry/*.json` only (no Rust) | extracts `Position.js` geometry and `Outline.js` cases from their Node tests to golden vectors for the UIs | `position.test.mjs`, `outline.test.mjs` |

Dependency: 1.1 first needs only `serde_json`; 1.2 needs a `Config` type from 1.1: it takes `serde_json::Value` for v1 of the port and is retyped in Phase 2, so 1.1 and 1.2 do not block each other. Each task's gate: `make rust`, `make test` (Node still green), `make check` untouched.

### Phase 2: daemon runs, QML becomes a client (Linux identical)

First the client and a fake daemon, so the QML work can be tested without Rust being ready.

| Task | Owns | Depends | How it is tested |
|---|---|---|---|
| 2.1 QML client | `core/client/DaemonClient.qml`, `core/client/Protocol.js`, `core/client/qmldir` (new directory), `tests/qml/fake-daemon.mjs` (a Node script speaking the protocol from `tests/protocol/` fixtures), `scripts/qml-test.sh` (starts it) | 0.2 | `tests/qml/client.qml`: connects, handshake, receives `state`/`view` fixtures into properties, sends `pointer`/`key`/`set`; no Rust |
| 2.2 View renderer | `core/client/ViewTree.qml` (a QML Loader-based renderer of the primitive tree), `core/client/prims/*.qml` (one file per primitive), `tests/visual/primitives.qml` | 0.2 | `make visual` renders every primitive from fixtures with demo data, lint clean; compared by eye, as the rules require; offscreen software renderer: `meter`, `image` rounding and `equalizer` carry a no-shader path |
| 2.3 Daemon core loop | `native/notchd-daemon/src/{server,router,instance}.rs`, `notchd-core/src/{store,peek,timers}.rs` | 1.1, 1.2 | Rust: `FakeUi` scenarios: hover enter, delay, mode `expanded`; peek arbitration; `set` removes a key and empties parents |
| 2.4 Process host | `native/notchd-daemon/src/modhost/` (spawn, restart-on-save, crash rows, logs), `tests/modules/` (fake user modules: crashing, slow, malformed) | 0.3 | Rust: crash row text and log path; reload on save; backoff; an invalid tree becomes an error row |
| 2.5 Launcher | `hosts/omarchy/Service.qml` (launch part), `shell.qml` (launch part), `scripts/ensure-daemon.sh` | 2.1, 0.3 | `make validate` passes (no symlinks); `make dev` and `make restart` start the daemon and reconnect on kill |

**End of Phase 2 state:** the daemon runs and serves config and a `state` mirror, but **`NotchService.qml` still decides** (the daemon's `state` is computed in shadow and compared in a QML test, `tests/qml/shadow.qml`, which fails if the daemon and QML disagree for the same inputs). Nothing the user sees changes. This is the long-lived safety net: a disagreement is a bug in the port.

### Phase 3: slices of authority move to the core (serial per slice; slices after the first are parallel)

For each slice, the QML side deletes its rule imports and binds to the daemon; the slice's existing QML tests (`tests/qml/shell.qml`, unchanged in intent) must pass against a real daemon started by `scripts/qml-test.sh` with a temp socket.

| Task | Owns | Depends | Moves |
|---|---|---|---|
| 3.1 Config and IPC | `core/NotchService.qml` (config and IpcHandler parts), `core/settings/SettingsWindow.qml`, `core/settings/SettingRow.qml` (binding to `schema`), `native/notchd-daemon/src/ipc.rs`, `native/notchd-core/src/ipc.rs` | 2.1, 2.3 | the config file (read, watch, write), `set`, every `ipc.md` method but those that need a module; `docs/ipc.md` unchanged. After this, `Config.js` and `Settings.js` are deleted with their Node tests (replaced by Rust) |
| 3.2 Notch state | `core/NotchService.qml` (state parts), `core/Notch.qml`, `core/NotchCopy.qml` | 3.1 | modes, hover timers, peeks, navigation, reserve targets. The QML keeps drag, packing, outline. `State.js`, `Navigate.js` go |
| 3.3 Timer, activities, resources, osd modules | `native/notchd-modules/src/{timer,activities,resources,osd}.rs`, `core/modules/{timer,activities,resources}.qml` (deleted, replaced by the renderer), `core/modules/*.json` keep | 2.2, 3.2 | the three built-in modules and `osd` as core modules drawn by `ViewTree`. Their `tests/qml/shell.qml` cases keep asserting on the same visible text and layout facts |
| 3.4 Media | `native/notchd-modules/src/media.rs`, `notchd-providers/src/linux/mpris.rs`, `core/modules/media.qml` (deleted) | 2.2, 3.2 | MPRIS through `zbus` against the demo player (`tests/visual/demo-player.py`) on a private bus (`scripts/isolated.sh`); the `equalizer` and `progress.anchor` primitives |
| 3.5 Agents and terminals | `native/notchd-modules/src/{claude,codex,terminal}.rs`, `notchd-providers/src/pty.rs`, `core/TerminalModule.qml` (reduced to the surface), `core/terminal/*` (bridge to PTY), `core/AgentUsage.qml` (deleted) | 3.2 | PTY in the core, title events, usage fetch, the fake `claude` and `codex` fixtures of `tests/qml/fixtures`; the terminal surface bridge |
| 3.6 Voice and components | `native/notchd-modules/src/{codex_live,local_live}.rs`, `notchd-core/src/components.rs` wiring, `core/LiveModule.qml`, `core/ComponentStore.qml` (deleted) | 3.2 | the Python sidecars launched by the core; `tests/*_test.py` unchanged (integration layer); `meter`, `list` |
| 3.7 User modules | `native/notchd-daemon/src/modhost/` (finish), `docs/modules.md` (a new "User modules" section), `tests/modules/` | 2.4, 3.2 | the manifest, settings from `module.json`, provider subscriptions; a sample module in `tests/fixtures/modules/hello/` |

3.3 to 3.7 are independent after 3.2 and run in parallel (each owns separate Rust files and separate QML files; the shared `core/NotchService.qml` is owned by 3.1 and 3.2 only, and each of 3.3 to 3.7 hands its QML changes to a module file they own, never to the service).

### Phase 4: cleanup (serial)

**4.1** Delete what no longer has an importer: `core/modules/*.qml` for the migrated modules, the Node tests ported, `core/Live.js` etc. Keep: `Position.js`, `Outline.js` (and their tests: UI-side), `Terminal.js`'s `PROBE`/`isLeaveKey`, `settings/Settings.js`'s control mechanics. Owns: those files.
**4.2** Docs: `DEVELOPMENT.md` (layout, prerequisites: Rust toolchain, the new gate), `CLAUDE.md` (invariants: "Rules in the Rust core", "The core knows no desktop" now `native/notchd-core` imports no UI and no OS; "No `svc: svc`" stays for QML), `docs/hosts.md`, `docs/modules.md`, `docs/theming.md` (the token set as the contract), `README.md`, `CHANGELOG.md` (a line under `[Unreleased]`: the daemon). Owns: those files.
**4.3** `scripts/` for `make times` to include the Rust layer; `build/times/` entries for `cargo test`; a size budget for the unit layer's Rust tests (as for the Node ones: 0.1 s).

### Phase 5: other OSes (later, out of the first round)

Per OS, three tasks that touch no Linux path: **the UI** (SwiftUI/AppKit; native on Windows) implementing the tokens, the primitives, `canvas`, the `terminal` surface, window placement and the geometry from `tests/vectors/geometry/*.json`; **the providers** (`notchd-providers/src/{macos,windows}/`); **packaging** (`.app`, installer, login item, the named pipe). Each UI is tested against the same `tests/protocol/` fixtures and a fake daemon (no real core needed), then against the real daemon.

Outside the phases: a `webview` surface in the Linux UI (QtWebEngine is a heavy dependency: **Open**; WebKitGTK is another), the sandbox for user modules, porting the voice sidecars to Rust.

## Open questions

1. **Daemon discovery and distribution** ([Distribution](#distribution)): A on `PATH` with B as the first-run fallback is recommended. Needs a release process (GitHub releases, a checksum file) and an AUR package. Who runs it for a plugin-only user?
2. **One daemon per host instance, or one per user?** Omarchy and `make dev` standalone are two instances today. One daemon with named instances is simpler to launch; separate daemons are simpler to isolate in tests.
3. **Geometry in each UI, or one shared crate?** `Position.js`/`Outline.js` are reimplemented per OS from golden vectors in this plan. A `notchd-geometry` crate exposed through FFI (UniFFI for Swift, a C ABI for the rest) would remove the duplication but QML would need to call it per frame. Revisit when the second UI is written.
4. **Compositor adapters**: UI-reported in v1 (CLAUDE.md keeps compositor code in `*Compositor.qml`). A Rust Hyprland provider (its IPC socket) would let the core serve non-QML UIs on Linux. Not needed by this plan.
5. **Icons**: v1 uses Nerd Font glyphs, as every module does today. macOS and Windows UIs would want native symbols (SF Symbols, Segoe Fluent). A named-icon table in the protocol (`"icon":"play"`) mapped per UI would fix it and change every module's tree.
6. **Terminal surface on the wire**: a second connection per surface (this plan) versus an out-of-band PTY master fd (Unix only, not Windows). Confirm with a Phase 3.5 spike on throughput (`yes | head -c 100M`).
7. **Webview on Linux**: QtWebEngine (large dependency), WebKitGTK in a separate process embedded by the host, or none until a user needs it (modules say "signals a missing primitive"). Defer or decide?
8. **Security of the socket**: a Unix socket at `0600` in a `0700` directory protects against other users, not against another program of the same user; a module with `exec` permission can talk to it. Acceptable for "local experiences for yourself"; revisit with sharing and the sandbox.
9. **macOS media**: `MediaRemote` is private and has broken across releases. Alternatives (per-app scripting, the `nowplaying-cli` approach) are weaker.
10. **Components outside Arch**: `components.json`'s `kind: system` is pacman and the AUR. Other OSes need a per-OS install table, or the voice features stay Linux-only.
11. **Config location on macOS and Windows**: keep `~/.config/notchd` everywhere (simple, one doc) or use each OS's convention.
12. **Timer and clock text updates**: the core sends a new `view` once a second for a running timer. A `text` node with a `countdown` field (`{"endsAt":..}`), formatted by the UI, would remove that stream; it needs the format rule (`Countdown.format`) in every UI. Chosen here: stream, because it keeps the rule in one place.
13. **Tree size and nesting limits** for validation (suggested: 2 000 nodes, depth 16, 256 KiB per `view`).
