# Architecture

The contract between notchd's Rust core (the daemon) and the thin native UI of each OS: where each piece lives and how the pieces talk. It began as the plan for moving notchd from "QML plus plain-JS rules" to this; phases 0 to 3 of the [migration](#migration-plan) are done, so the Linux UI is already a client of the daemon, and phase 4 (cleanup) is in progress. The workflow is in [DEVELOPMENT.md](../DEVELOPMENT.md), the rest in the other docs. Where a claim is inferred rather than read in the code, it says so. Items marked **Open** are in [Open questions](#open-questions).

Decided, not up for debate here:

- Targets Linux, macOS and Windows. Linux stays Wayland, Quickshell and QML.
- The **core is Rust** and runs as a **daemon** (the "d" in notchd). **UIs are clients** and thin: they bind state and render. The invariant is "rules in the Rust core, UIs only bind".
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

One daemon serves one **instance**: a config path, a socket name and an IPC target name, with its own socket (`$XDG_RUNTIME_DIR/notchd/<instance>.sock`). There is one daemon per user: every host (Omarchy or standalone) connects to the `default` instance, and several UIs may connect to it. Other instances are for `make dev` and tests only, with their own config.

Responsibility split, the rule everything below follows:

| Core decides | UI does |
|---|---|
| which modules show, the mode of each notch copy, which peek wins, timers (hover, peek, reserve), navigation, config edits, settings pages and fields, every module's logic and view | where pixels go: windows, layer-shell or its equivalent, geometry of the card and its outline, packing notches on an edge, drag math, animation frames, theme mapping, fonts, the settings window's controls |

## Inventory

### `core/*.js` and `core/settings/*.js`

Where each rule file went. "Core" is the Rust daemon. "UI" means the file stays UI-side; on Linux that is QML-side and the file stays where it is, with its Node test. Tests for files that moved were ported to Rust (same cases, kept as `tests/vectors/rules/`), and a Node file went in the phase that removed its last importer. Still in `core/` today: `Position.js`, `Outline.js`, `Terminal.js` (the key that leaves a terminal), `Config.js` and `settings/Settings.js` (each cut down to what the QML still calls: the optimistic copy of an edit and the notch list; control mechanics and the window's assembly), `client/Protocol.js` and `client/prims/Tree.js`.

| File | Goes to | Why |
|---|---|---|
| `Config.js` (`parse`, `valueAt`, `withValue`, `DEFAULTS`, `resolve`, `delay`, `notches`, `withNotchAdded/withoutNotch`, `terminals`, `with*Terminal`, `workspaceList`, `withWorkspace`, `moduleLoadList`...) | core | The file's rules are the invariant "a missing key means its default"; the core owns the file now (`native/notchd-core/src/config.rs`, same functions, same cases). The JS file stays for the optimistic copy of an edit (`withValue`) and the notch list the QML still reads (`notches`, `moduleLoadList`, `withWorkspace`...); the daemon's answer replaces what it computes |
| `State.js` (deleted in 3.2): `activeModules`, `compactModule(s)`, `hasIcon`, `attentionPeek`, `canPeek`, `mode`, `shownModules`, `wantsKeyboard`, `keyboardFocus/Change`, `reservedSpace`, `reserveStep`, `workspaceAllowed`, `notchScreen`, `reserveScreens`, `peekCopy`, `needsVisitor`, `focusedPeek`, `reserveZones` | core | `native/notchd-core/src/state.rs`, `peek.rs`, `store.rs`; the UI binds `state`. |
| `Navigate.js` (deleted in 3.2) | core | The navigation state machine, `native/notchd-core/src/navigate.rs`. The UI sends a normalized key (`Protocol.wireKey`, `"Escape"`, `"Tab"`, `"h"`, `"1"` plus modifiers) and draws `state.navigation`; the hint texts are the UI's. |
| `Position.js`: `resolve`, `configValue`, `isEdge`, `isVertical`, `EDGES`, defaults | core | The config's `position` value and its fallbacks are config rules |
| `Position.js`: `place`, `insets`, `nearestEdge`, `dragTarget`, `screenAt`, `stretch`, `pack`, `rowDepths`, `corners`, `cornerInput`, `coveredByStandalone` | **UI** | Pixel geometry on measured card sizes and screen rectangles; runs per frame during a drag or a growth animation, where a socket round trip is wrong. Each UI reimplements it; the Node test cases become `tests/fixtures/geometry/*.json` golden vectors every UI's tests must pass (*inferred*: `coveredByStandalone` could be core, since it only reads module facts; kept with `pack` because it feeds it) |
| `Outline.js` | **UI** | Draws the card outline (SVG path data from `Shape`); pure drawing. Same golden-vector treatment |
| `Countdown.js` (deleted in 3.3) | core | The timer module's whole state machine, time passed in: `native/notchd-core/src/countdown.rs` |
| `Resources.js` (deleted in 3.3) | core | `/proc` and `df` parsing, pressure rules: `native/notchd-core/src/resources.rs`. The Linux reader is a provider (`notchd-providers/src/linux/procfs.rs`); the rules are portable |
| `Usage.js` (deleted in 3.5) | core | Endpoint shapes, `alerts`, `tightest`: `native/notchd-core/src/usage.rs`. The request is the `usage` provider's (`notchd-providers/src/usage.rs`, `curl` on a thread of its own); `AgentUsage.qml`'s `XMLHttpRequest` is gone |
| `Agent.js` (deleted in 3.5) | core | Terminal-title parsing, done-peek rule, IPC command parsing: `native/notchd-core/src/agent.rs` |
| `Terminal.js` | core | `shellArgs`, `expandHome`, `plainTitle` went with the PTY (3.5: `native/notchd-core/src/terminal.rs`). `PROBE` (is qmltermwidget installed), `SCHEME` and `isLeaveKey` stay UI-side (Linux terminal view) |
| `Live.js` (deleted in 3.6) | core | The bridge's argv, line parser, conversation reducer, status and error text: `native/notchd-core/src/live.rs`. `localPath` went: the daemon finds the sidecars itself (embedded in the binary, see [Distribution](#distribution)) |
| `Components.js` (deleted in 3.6) | core | Row text, buttons, locks, `decorate`/`gate` of the settings schema: `native/notchd-core/src/components.rs`, applied by `schema.rs`. The installer stays Python in v1, run by the daemon |
| `settings/Settings.js` (ported to `native/notchd-core/src/settings.rs`, `schema.rs`): `mergePages`, `pageOf`, `parentPage`, `isSubPage`, `visiblePages`, `navPages`, `effectiveValue`, `shownFields`, `sections`, `parseModuleInfo`, `agentSettings`, `liveSettings`, `localLiveSettings`, `terminalFields/Info`, `settingDefault`, `standaloneFields`, `moduleRefs`, `moduleFields`, `modulePages`, `moduleTitle/Kind`, `catalogPart`, `moduleStatus`, `notchPageId`, `notchIndexOfPage`, `notchSummary`, `modulePageId` | core | Building the schema and deciding what shows. The UI gets finished pages (below) |
| `settings/Settings.js`: `toggleId`, `moveId`, `allIds`, `workspace*`, `sortWorkspaces`, `toggleWorkspace` | core | They are config edits (a module list, a workspace pick). The UI sends an `edit` op with its intent (below) instead of computing the new list |
| `settings/Settings.js`: `findPage`, `resolvePage`, `cyclePage`, `moduleInfoPath`, `snapToStep`, `storedValue`, `toList` | **UI** | Control mechanics: which page is on screen, a slider's snap, QML's Repeater-sequence gotcha. `moduleInfoPath` uses `Quickshell.shellDir`: dropped, the core finds its own module files |

### Main QML

| File | Becomes |
|---|---|
| `NotchService.qml` | Split. **Core**: config load/watch/`setConfigValue`/`replaceConfig`, module registry and `moduleInfo` (the JSONs), module load list, `agentCommand`/`terminalCommand`/`liveCommand`, `requestPeek/dismissPeek/requestKeyboard/openModule/closeModule`, peek arbitration, navigation state and `navKey`, `undoMove`, `notchSpec`, `standaloneNotch`, `addNotch/removeNotch`, the whole `IpcHandler` (becomes the daemon's router), `state`. **UI**: the `Process` and `FileView` plumbing goes; it keeps `settingsOpen`, `screens`, `barEdge`, `compositor` and `Theme.provider` and reports them |
| `Notch.qml`, `NotchCopy.qml` | **Core**: a copy's hover, pin, peek, mode, hover timers (`hoverOpenDelay`, `hoverCloseDelay`, peek duration). **UI**: reports raw pointer facts; keeps the drag preview, `packShift`, the joined-ends bookkeeping |
| `NotchSurface.qml`, `NotchWindow.qml`, `NotchCard.qml`, `DepthReport.qml`, `KeepShown.qml` | UI (layer-shell windows, outline, drag, reserved-space windows). The UI reports the depths its cards measure (`measure`); "grow at once, shrink after 400 ms" is the core's, which sends `reserve`, and the UI holds windows |
| `NotchModule.qml`, `LiveModule.qml`, `TerminalModule.qml`, `AgentUsage.qml`, `ComponentStore.qml` | Replaced by core module logic plus the view trees below: `LiveModule.qml`, `TerminalModule.qml`, `AgentUsage.qml` and `ComponentStore.qml` are gone, and `core/CoreModule.qml` (a `NotchModule` that draws the daemon's views) shows every built-in module. `NotchModule.qml` stays as the base type of a module the QML loads itself (a host's, the tests'); what stays of the terminal is the Linux-only view: `terminal/TerminalSurface.qml` and `TerminalView.qml`, the free surface |
| `Theme.qml`, `Hyprland/GenericCompositor.qml`, `qmldir` | UI. The provider keys of [theming.md](theming.md) become the **token set** every UI implements |
| `settings/*.qml` | UI (the settings window and its controls, one control per field type, [settings.md](settings.md)), fed pages by the core |
| `hosts/omarchy/*` | UI host. `Service.qml` maps `qs.Commons` onto the tokens and keeps the `eryk.notchd` IPC target as a thin forwarder to the daemon. `modules/osd.qml` only owns the `osd` IPC target and forwards its `show <json>`; the module (`osd`, standalone) is the daemon's, the one that owns an OS service |

### Modules

"v1 primitives" are defined in [Primitive tree](#primitive-tree-v1). The set is exactly what these modules draw; nothing else.

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
{"t":"state","rev":41,"navigation":null,"settingsOpen":false,"settingsPage":"general",
 "notches":[{"id":"main","edge":"top","offset":0.5,"screen":"","draggable":true,"tabs":true,
   "modules":["media","activities","resources"],
   "copies":[{"screen":"DP-1","mode":"compact","pinned":false,"tab":"media",
              "peek":null,"shown":["media"],"compact":["media","activities"],"tabs":["media","activities","resources"],
              "covered":false,"needsWindow":true,"keyboard":"none"}]}],
 "reserve":[{"screen":"DP-1","edge":"top","depth":34}],
 "standalone":[{"id":"osd","copies":[{"screen":"DP-1","mode":"rest","pinned":false,"peek":null,"shown":[],"needsWindow":false,"keyboard":"none"}]}],
 "visitor":[{"screen":"DP-1","mode":"rest","pinned":false,"peek":null,"shown":[],"needsWindow":false,"keyboard":"none"}]}
```
```json
{"t":"module","id":"media","rev":7,"title":"Now playing","priority":50,"active":true,"icon":"","attention":false,
 "expandedWidth":400,"urgentPeek":false,"peekOnFocusedScreen":false,"error":null}
```
`mode` is `rest | compact | peek | expanded` (the core's `state::mode`). `compact` is what the closed notch shows side by side, `shown` what the open one shows, `tabs` the modules to make tabs of (`[]` for none), `covered` that the copy hangs on no screen (its workspace is not shown), and `keyboard` what its window asks of the keyboard: `none`, `exclusive` (until its window has it, see `focus`) or `ondemand`. `settingsPage` is the settings window's page, which the UI follows when the core opens it (navigation's `s`, `ipc settings`). A guest copy (`standalone`, `visitor`) has `pinned`, `shown` and `keyboard` too. `navigation` is `null`, or `{"mode":"select|move|remove","selected":0,"tab":""}` (the navigation state machine's state: the selected notch's index and the tab it shows, `""` for its own). `peek` is `{"module":"media","view":"peek","until":1730000000000}` (`view` is a slot name, `until` epoch milliseconds); the UI shows the module's `peek` view while it is set. The UI derives nothing from time: the core clears `peek`.

`reserve` is the depth to keep free per screen edge: `state::reserve_zones` over every copy of a notch with `reserveSpace` (the deepest wins where notches share an edge). The core cannot measure a card, so the UI reports the depths with `measure` (below) and the core applies `state::reserved_space` (the rest lip, or a compact view's depth, the deepest seen and never under the baseline) and `state::reserve_step` (grow at once, shrink 400 ms later, in the core: `reserve-applied` is not needed). `reserve` is empty until a UI has measured.

`standalone` and `visitor` hold the guest notches: a standalone module's own, and the visitor (`NotchService`'s `visitorNotch`), the default notch that peeks at the top of the focused screen when no notch showing the module is there. Both list a copy per screen, with `needsWindow` as `state::needs_window` says for a guest: true while the copy is open and for 5000 ms after it closes (the window linger), so a UI keeps a window only while it is true. The visitor's copies are named by the notch id `""` in `pointer`, `key` and `tab`. `ipc state` lists the visitor's copies that show something, as `NotchService.state()` does.

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
Sent on hello and on any change. This is the adapter contract of `GenericCompositor.qml`, reported instead of called (`hasFullscreen`, `activeWorkspace`, `knownWorkspaces`, `supportsWorkspaces`), plus the screen list and `barEdge`. The core runs `state::notch_screen`, `workspaceAllowed` and `peekCopy` on it.

```json
{"t":"pointer","notch":"main","screen":"DP-1","ev":"enter"}
{"t":"pointer","notch":"main","screen":"DP-1","ev":"leave"}
{"t":"pointer","notch":"main","screen":"DP-1","ev":"press"}
{"t":"drag","notch":"main","screen":"DP-1","ev":"start"}
{"t":"drag","notch":"main","screen":"DP-1","ev":"drop","position":{"edge":"left","offset":0.3,"screen":"DP-2"}}
{"t":"key","notch":"main","screen":"DP-1","key":"Tab","mods":["shift"],"text":""}
{"t":"event","module":"timer","slot":"expanded","action":"toggle","value":null}
{"t":"event","module":"media","slot":"expanded","action":"seek","value":0.42}
{"t":"tab","notch":"main","screen":"DP-1","module":"timer"}
{"t":"pin","notch":"main","screen":"DP-1","on":true}
{"t":"focus","notch":"main","screen":"DP-1","active":true}
{"t":"facts","modules":[{"id":"media","priority":50,"active":true,"compact":true,"peek":true,"icon":false,"expanded":true,"attention":false,"urgentPeek":false,"peekOnFocusedScreen":false,"standalone":false}]}
{"t":"peek","module":"media","ms":2500}
{"t":"peek-dismiss","module":"media"}
{"t":"module-notch","module":"timer","op":"open"}
{"t":"settings","open":true,"page":"modules/media"}
{"t":"catalog","modules":[{"moduleId":"media","title":"Now playing","settings":[],"standalone":false}]}
{"t":"reserve-applied","screen":"DP-1","edge":"top","depth":34}
{"t":"measure","notch":"main","screen":"DP-1","rest":6,"compact":34,"baseline":30}
```

- `pointer`: raw facts; the core owns `hoverOpenDelay`, `hoverCloseDelay` and decides `mode`. `press` pins, or unpins. A copy is named by its notch (a standalone module's by its module id, the visitor's by `""`) and its screen (`""` for the one that moves; for it, `screen` on `enter` and `press` is the screen the pointer is on, which the notch stays on while open). The UI counts the surfaces that report the pointer and sends one `enter` and one `leave`; it sends `enter` again to a daemon that came back.
- `settings`: `page` is optional (absent: keep the page shown). A `notch/<index>` page picks the notch the schema's own-notch page edits.
- `pin`: a copy pinned open (`on: true`) or closed and no longer hovered, by something other than a click on it: a click away, Escape.
- `focus`: a copy's window gained or lost the keyboard. The core keeps whether it had it and answers with `state.keyboard`; a window that took the keyboard from an open notch closes it, except just after navigation ended, when it is asked for again.
- `facts`: what the modules the UI loads itself say of themselves (`id`, `priority`, `active`, `compact`, `peek`, `icon`, `expanded`, `attention`, `urgentPeek`, `peekOnFocusedScreen`, `standalone`); replaces the last list (a user module's facts come from its process). Sent when they change and to a new daemon. The UI sends pending facts, screens and config edits before an input that depends on them.
- `peek`, `peek-dismiss`: `NotchModule.requestPeek` and `dismissPeek` (`ms` and `module` optional). The core routes a peek: to every notch that shows the module (on the focused screen's copy for a notch on every screen), to the visitor when none peeks there, to a standalone module's own notch.
- `module-notch`: a module acts on the notches that show it: `open` (the first one opens on the module's tab), `close`, `keyboard` (every copy open on it pins).
- `catalog`: the modules of the host that the daemon cannot know (the Omarchy `osd`, a terminal of the config), each `{ moduleId, title, settings, standalone, agent, live, terminal }` as the host parsed its JSON file, sent on hello and whenever the list changes; it replaces the last. A UI that has none sends `"modules": []` (or no `catalog` at all, when it has no `defaultModules` either). The built-in modules are the daemon's own: it embeds their JSON files (`native/notchd-modules/modules/<id>.json`, the same ones the modules read their defaults from, so there is one source) and builds their settings pages and defaults itself, so no UI ships or sends them; a host's or a user module's entry with the same id wins. `defaultModules` (optional) is the host's list for a notch whose config lists none (the daemon's own pair, `media` and `activities`, when absent). The daemon builds the schema parts that depend on them: the lists of the Modules page, what a notch's module list says, a page per module. `coreDir` is deprecated and optional (accepted within proto 1): a folder with `live/` and `components/` that the daemon runs its sidecars from instead of its own; a UI omits it. The daemon finds its sidecars without a UI: they are embedded in the binary and written out on first use ([Distribution](#distribution)). The UI learns every module (built-in, host's, user's) from `schema`'s optional `modules`, the list the pages were built from, in the shape of a `catalog` entry: it is how a UI that ships no JSON files knows their titles, kinds and setting defaults.
- `drag`: `start` when the card is picked up (the copy stays closed, nothing opens it, what it reserves holds still), then `drop` with the `position` or `cancel`. The UI computes the drop (`Position.dragTarget`, `screenAt`); the core stores `position` and the workspace the target screen shows (config edit, never writes a default). The motion in between is the UI's own preview.
- `key`: while a notch holds the keyboard. `key` is a normalized name (`"a"`, `"Escape"`, `"Tab"`, `"Left"`, `"Return"`, `"1"`); `mods` from `shift|ctrl|alt|meta`. In navigation and while a module with a text field is focused the core replies by changing `state`; keys the core does not use come back as nothing.
- `event`: a primitive's `action` fired. `value` is always present: `null`, a number (a `progress` seek, as a fraction), a string, or a boolean; never an object or array.
- `measure`: the depths the UI measured for one notch copy (`screen` as in `pointer`: `""` for the one that moves): `rest`, the rest lip's depth (`restDepth`); `compact`, the compact view shown now, 0 when none (`compactDepth`; the core keeps the deepest seen, and forgets it when `baseline` or the edge's orientation changes); `baseline`, the least depth reserved for a compact view (`compactBaseline`, 0 on a side edge). Sent when one changes. The core answers with `state.reserve`.
- `reserve-applied`: optional ack for `reserve`. The daemon ignores it: the shrink delay runs in the core.

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

- `set`: `config::with_value` semantics (`Config.withValue` in the UI's optimistic copy). `value: null` (a `null` on the wire is JS's `undefined` here) removes the key and empties parents. This is today's `set <key> <json>`; the CLI parses its argument and sends it.
- `edit`: the list edits (`toggle_id`, `move_id`, the workspace picks of `settings.rs`) as intent ops; the core computes and writes. v1's ops are `moveModule` (`delta`, an integer, negative towards the start), `toggleModule` and `toggleWorkspace`; a new op is a minor addition, and the core answers one it does not know with `err` code `args`. A request's `id` is an unsigned integer.
- `ipc`: every method of [ipc.md](ipc.md) (`push`, `clear`, `open`, `toggle`, `close`, `openNotch`, `agent`, `terminal`, `timer`, `live`, `navigate`, `settings`, `recenter`, `state`) with the same strings as replies. The `notchd` CLI is `notchd ipc <method> args...` and nothing else. The host's own IPC (`omarchy-shell eryk.notchd ...`, `qs ipc call notchd ...`) stays, and `ipc.md` does not change. Quickshell's `IpcHandler` is synchronous (a function returns its string at once; there is no way to wait for a socket reply), so a handler cannot "forward and return the daemon's reply". What the handler does instead, per method: `set` and `recenter` are config edits, which the daemon owns, so the handler applies them (the same `Config.withValue`), answers with the string the daemon's `set` answers (`ok`, `usage: ...`, `not JSON: ...`), and sends the edit as a `set` request, which the daemon writes; `open`, `close`, `toggle`, `openNotch`, `closeNotch`, `toggleNotch`, `navigate`, `settings` and `state` act on the notch state, which the daemon decides (since 3.2), so the handler sends the intent as an `ipc` request and answers `ok` (`no notch <id>` from the last state; `state` is built from the bound state, as it needs a module's usage too); `push`, `clear`, `agent`, `claude`, `codex`, `terminal`, `timer`, `live` and `component` need a module, whose answer (`no timer running`, the usage text) is the daemon's, so the handler sends the command and answers `ok` (or what it knows: `<id> module not loaded`). The CLI (`notchd ipc <method>`) always goes to the daemon, which answers every method that needs only config and state itself.

### Settings

The schema is core-built; the settings window is a UI. It also carries `modules` (optional), the catalog the pages were made from: the built-in modules' (the daemon's own), the host's and the user modules' `{ moduleId, title, settings, standalone, agent, live, terminal }`.

```json
{"t":"schema","rev":3,
 "pages":[{"id":"general","label":"General","icon":"","sub":false},
          {"id":"modules/media","label":"Now playing","icon":"","sub":true}],
 "fields":[{"page":"general","section":"Hover","key":"hoverOpenDelay","type":"integer","label":"Open delay",
            "min":0,"max":2000,"step":20,"unit":"ms","default":120,"value":120,"visible":true,"locked":null}]}
```

Field types are the existing ones ([settings.md](settings.md); each is one control plus one case in `SettingRow.qml`): `choice`, `integer`, `number`, `boolean`, `string`, `color`, `modules`, `workspaces`, `notches`, `moduleCatalog`, `component`, `components` and `note`. The `schema` is sent after `config` and again whenever the config, the screens or a module's page change it; it holds the pages with a field (the core's, a page per notch, and a user module's own from its `module.json` `settings`) and the fields with their current `value` and `visible`. A `moduleCatalog` field carries `part` (`notch`, `agent`, `terminal` or `standalone`), the list of the Modules page it stands for. (The Components page, the components' rows and the locked choices are in it since 3.6; the built-in modules' pages come from the `catalog`.) A field also carries, when set, `help`, `options` (`[{value,label,preview?,disabled?}]`, `disabled` being `needs` resolved), `display` (`cards|chips|percent`), `when` (`{key,is}`), and for a `component` its `component` id and the `row` the core computes (free-form in v1); `key` is absent on a `note` and a `component` row. `match`/`by` and `needs` are resolved by the core and never reach the UI. A `null` `default` or `value` is left out. `value` is current, `default` from the schema, `visible` is `shown_fields`'s answer (`settings.rs`), `locked` is `components::lock`'s reason or `null`. The UI sends `set` or `edit` and waits for the next `schema`. Component rows (`type: "component"`) carry the view `components::view` computes (title, status, size text, buttons as action ids), and a button press is `{"t":"ipc","method":"component","args":["install","voice-piper-pt-br"]}`; progress arrives as `schema` updates, read by the core from `install.py status` (while the settings are open and something installs, every second).

Theme is **not** in the protocol. Each UI maps the token set to its own source and applies it (`Theme.provider` today).

### Primitive tree (v1)

Every node: `"t"` (the primitive) and optional `"id"` (stable across updates, so a UI may animate a changed node in place), `"visible"` (default true), `"action"` (string, fires `event` on press/tap; absent means not interactive), `"tooltip"`, `"fill"` (in a `row` or `column`, the node takes the rest of the space), `"width"` and `"height"` (a fixed size, design pixels), `"minWidth"`, written in that order before the node's own fields. A `row` and a `column` also take `"maxWidth"` (their natural width up to it). Sizes are **design pixels**, which each UI multiplies by `Theme.space()`'s `spacingScale` (`Theme.space(px)` today). Colors never appear: `tone` is one of `foreground | muted | faint | accent | danger | success | line`. Text sizes: `caption | bodySmall | body | heading | display` (theming.md's five).

| `t` | Fields | Notes |
|---|---|---|
| `row` | `gap`, `align` (`start\|center\|end`), `children` | horizontal layout. `fill: true` on a child makes it take the rest |
| `column` | same | vertical layout |
| `stack` | `align`, `children` | overlay, centered by default: the Voice disc and its glyph |
| `spacer` | `size` | fixed gap, or `fill: true` (the common property) for a flexible one |
| `text` | `text`, `tone`, `size`, `tabular`, `bold`, `elide` (`end\|none`), `wrap`, `maxWidth`, `align`, `lines` | always plain text, never rich (the code's `textFormat: Text.PlainText`) |
| `icon` | `name` (a named icon), `glyph` (a codepoint string, Nerd Font: the fallback), `tone`, `size` (design pixels, or a text size name: the icon beside a text of that size) | needs a `name` or a `glyph`; see "Named icons" below |
| `image` | `src` (absolute path or `file://` / `https://`), `size` or `w`/`h`, `radius` (`none\|small\|round`), `fit` (`crop\|contain`), `fallback` (an `icon` node) | cover art: the UI draws the fallback while loading and on failure, and rounds the corners without a shader where it has none (the software-render rule in CLAUDE.md applies to every UI) |
| `progress` | `value` (0..1; absent or `null`: no bar), `orientation` (`h\|v`), `tone`, `thickness`, `anchor`, `seek` (action id) | `anchor`: `{"at":<epoch ms>,"rate":<fraction per second>}`: the UI interpolates `value + rate * (now - at)` itself, so the media position needs no stream. `seek` fires `event` with a fraction |
| `meter` | `value` (0..1), `style` (`disc`), `size`, `child` (a node drawn inside) | `disc`: the Voice disc that swells with the voice (`1 + value * .35`, a 140 ms ease-out: longer than the time between levels, so the disc never stops between steps). The core sends the normalized level; the UI animates |
| `equalizer` | `active`, `mode` (`smooth\|light`), `bars` (default 4), `height` | the media module's decorative bars (the code's "fake equalizer"): the UI generates the motion, the core sends only `active` and `mode`. Not a canvas, so no 60 Hz stream |
| `list` | `gap`, `stick` (`start\|end`), `maxHeight`, `children` | a scrolling column; `stick: "end"` keeps the newest line in view (the transcript) |
| `button` | `name`, `glyph` or `label` (`name` and `glyph` as an `icon`'s), `action`, `tone`, `enabled`, `selected`, `variant` (`icon\|outline`), `big` (a larger round button) | the `IconButton`, the Voice start button and the timer's controls |
| `chip` | `label`, `value` (a string), `action`, `selected` | the timer's presets |
| `canvas` | `w`, `h`, `cmds`, `animate` | tier 2, below |
| `surface` | `kind` (`terminal\|webview`), `ref`, `w`, `h` | tier 3, below |

**Named icons.** An `icon`, a `button`'s icon and an `image`'s fallback `icon` carry an optional `name` next to `glyph`. A UI draws the name with its own symbol set (the QML UI: a Nerd Font glyph from a table in `core/client/prims/Tree.js`; a GNOME UI: the Adwaita symbolic icon of that name) and, when it does not know the name, draws `glyph`. So a module may send only `glyph` (a Nerd Font codepoint, as before), only `name`, or both, which is what the official modules do. An unknown `name` is never an error (forward compatibility); an `icon` with neither is invalid, a `button` needs a `glyph`, a `name` or a `label`. `icon.size` is design pixels or a text size name (`caption`..`display`), so an icon can sit at the size of the text beside it.

The names are the table `notchd-proto`'s `icons::NAMES` (listed in the schema's `IconName.examples`, a test keeps both equal). Scheme: the freedesktop icon naming specification's name where a standard icon exists (`media-playback-start`, `window-close`, `view-refresh`, `list-add`, `dialog-warning`, `drive-harddisk`, `utilities-terminal`, `audio-input-microphone`, `audio-x-generic`, `media-skip-forward`, ...), without the `-symbolic` suffix (a GNOME UI appends it, and Adwaita's symbolic set follows the same names, so its mapping is mostly the identity); otherwise a short lowercase description (`focus`, `coffee`, `cpu`, `memory`, `alarm`, `notification`, `microphone-disabled`), which a UI maps to the nearest symbol or leaves to the `glyph`. The names the official modules send: `media-playback-start`, `media-playback-pause`, `media-playback-stop`, `media-skip-forward`, `media-skip-backward`, `audio-x-generic`, `audio-input-microphone`, `microphone-disabled`, `list-add`, `view-refresh`, `window-close`, `alarm`, `notification`, `dialog-warning`, `utilities-terminal`, `drive-harddisk`, `cpu`, `memory`, `focus`, `coffee`. A user module may use any of them. Not yet named: the module's own closed-notch `icon` fact and the glyphs a `canvas` `text` command draws.

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
| `terminal` | official modules (`claude`, `codex`, user terminals) | The **PTY lives in the core** (`portable-pty`, `notchd-providers/src/pty.rs`): the program starts when the first view attaches, lives as long as the module is loaded (or until `idleMinutes`) and survives the notch closing, the shell reloading (`make restart`) and any UI going away. The UI draws a terminal view attached to it: Linux keeps `qmltermwidget` and attaches through a surface connection (below); macOS and Windows get their own view |
| `webview` | user modules that need their own HTML (the escape hatch; a module needing it signals a missing primitive) | The UI opens a sandboxed webview, no network by default, no file access, loading the module's `index.html` from its folder through a custom scheme; the module sends/receives JSON through a `postMessage` bridge the daemon relays as `surface` messages to the module process |

```json
{"t":"attach","ref":"term:claude","cols":80,"rows":24}
{"t":"surface","ref":"term:claude","ev":"data","b64":"G1s..."}
{"t":"surface","ref":"term:claude","ev":"input","b64":"bHMK"}
{"t":"surface","ref":"term:claude","ev":"resize","cols":100,"rows":30}
{"t":"surface","ref":"term:claude","ev":"title","text":"⠋ Fix tests | notchd"}
{"t":"surface-close","ref":"term:claude"}
```

`attach` is the UI's first line on a connection of its own for the surface (below); `ev` is `data` or `input` (`b64`), `resize` (`cols`, `rows`) or `title` (`text`). (`surface-open`, `{"t":"surface-open","ref":...,"kind":...,"cols":..,"rows":..}`, is the daemon asking a UI to open a surface; the terminal does not use it.)

- `ref` names the surface (`term:<module id>`); the core sends `data` (PTY output) and `title` (OSC 0/2, which the agent modules read to decide the compact view and the done peek: `agent.rs`), the UI sends `input` and `resize`. Lines carry base64 PTY bytes.
- **A surface has a connection of its own**, so a flood of output never delays a `view` update. Its first line is `{"t":"attach","ref":"term:claude","cols":100,"rows":30}` (`cols` and `rows`, the view's size, are optional: the program starts at it); the daemon answers with the output so far (the last 1 MiB of it, as `data`: a view that attaches later draws what the earlier one did, a reload included), then the output as it comes, `title`s, and `surface-close` when the program ends or is replaced; an unknown ref is an `err`. Closing the connection detaches the view and nothing else: **the program and its session stay**. Several views may attach to one session (the last `resize` wins). The queue to a view is bounded, so a slow view slows the program to its pace, as a real terminal does. A view that attaches to a session that already drew something makes the program draw again (a resize one column narrower and back), since a program only redraws on a size change. A session that wrapped its 1 MiB starts the replay at a line end after a reset of the attributes; a TUI's modes set before that point (mouse, alternate screen) are not replayed, which a redraw mostly repairs.
- **How the output reaches `qmltermwidget`, decided (3.5).** The two designs of the open question were this connection and passing the PTY master's file descriptor. A descriptor cannot be passed on Windows (ConPTY has no such handle) and would let the UI, not the core, own the program's input, so the stream it is. But `qmltermwidget` (the QML port of Konsole's `Session`) only runs a program in a PTY of its own: its QML types have no way to be fed bytes (`Session::receiveData` takes a `char*`, which QML cannot pass, and is not exposed). So the Linux view runs `notchd attach <ref>` as the widget's program: a client of this connection whose standard output is what the program printed and whose standard input (the keys, the terminal's size by its window size, looked at every 100 ms) goes back. The widget thinks it runs a program; the program lives in the daemon. One more hop through a PTY, no change to the widget. The daemon's own binary is the helper (`welcome.exe`), so the two always speak the same protocol. A UI that draws a terminal itself speaks the same lines with its own socket client. Measured (`tests/terminals.rs`, a flood of 20 MB through the connection): 235 MB/s in a release build, far over what a widget draws.
- The view is created by the UI when a `surface` node appears in a view tree, and the module starts the program when the first view attaches (`views` in `term.rs`). The UI says it can draw a terminal with `surfaces: ["terminal"]` in `hello`; without one, the surface node draws a placeholder ("the terminal needs the qmltermwidget package") and `ask` answers so.
- A surface node's `id` names the **run**: a new session (restart) is a new `id`, and the UI replaces the view; the same `id` keeps the view across updates of the tree (the stack, row and column primitives keep their children by position, not by the list the daemon sent, so a `view` update never remakes a terminal).
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

`subscribe: ["resources"]` in the manifest, or at runtime `{"t":"subscribe","name":"media"}`. The core then sends `provider` messages on change (throttled per provider: `resources` 1 s, `media` on change, `screens` on change). A module may `{"t":"unsubscribe","name":"..."}`. A provider name modules cannot have (`media`, `resources`, `screens` and `usage` are the ones with data; `usage` carries an `agent` beside `limits`) is answered with `{"t":"error","code":"provider"}` to the module (and logged); the manifest's `subscribe` is checked against the known names when it is read. Provider payloads are the same shapes as the built-in modules consume (below) and are part of `proto`.

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
| `media` | players, each `{id,name,playing,title,artist,album,artUrl,length,position,canPlay,canPause,canNext,canPrev,canSeek}`, plus commands (`play`, `pause`, `next`, `previous`, `seek`) | MPRIS over D-Bus (`zbus`); `notchd-providers/src/linux/mpris.rs`; the demo player's MPRIS in `tests/visual/demo-player.py` | MediaRemote (private framework, unstable across releases: **Open**) or per-app AppleScript | GlobalSystemMediaTransportControls (WinRT) |
| `resources` | `{cpu,mem:{used,total},disk:{used,total,path}}` | `/proc/stat`, `/proc/meminfo`, `df -P -B1` (the rules are `resources.rs`; the provider does the I/O, `linux/procfs.rs`) | `host_processor_info`, `host_statistics64`, `statvfs` | PDH counters, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceEx` |
| `screens` | the `screens` message above (names, rects, scale, focused, fullscreen, workspaces) | **UI-reported** in v1: Quickshell `Screen` plus the `*Compositor.qml` adapters. CLAUDE.md puts compositor code only in those adapters, so the Hyprland adapter stays QML | the UI knows (`NSScreen`) | the UI knows |
| `usage` | an agent's plan limits `[{id,label,percent,resetsAt,windowSecs}]`, stale and problem text | `~/.claude/.credentials.json`, `~/.codex/auth.json` read by the core, HTTPS (`curl`) to the endpoints `usage.rs` names | the same files; credentials in the Keychain: **Open** | the same files |
| `pty` | a PTY per terminal; output, exit and size (`PtyEvent`), input; the title is read from the output (OSC 0/2) by the session | `portable-pty` (openpty, a process group of its own stopped and reaped) | `portable-pty` | `portable-pty` (ConPTY) |
| `audio` | the Voice's microphone and voice output (`pacat`/`parec` today, via the sidecar); a `level` feed (dB) for the `meter` | the sidecars call `parec`/`pacat`; the `libpulse` component is their dependency | CoreAudio (`cpal`) | WASAPI (`cpal`) |
| `notifications` | `notify(title, body)` | `notify-send` (`notchd-providers/src/linux/notify.rs`); the timer and user modules' `notify` go through it | `UNUserNotificationCenter` or `osascript` | toast notifications |
| `components` | the registry and per-component status | `core/components/install.py` (pacman, AUR, files, LocalAI): Python in v1, run by the core, state files in `$XDG_STATE_HOME/notchd/components/` | later (brew, downloads); the registry grows an OS key per component (`kind: system` is Arch-only today: **Open**) | later |
| `osd` (Omarchy host) | `{icon,message,value,max}` | the host forwards `omarchy-shell osd show <json>` | n/a | n/a |

Payload types are in `notchd-proto::provider` (`MediaData`, `ResourcesData`, `UsageData`, `OsdData`, `NotifyData`; `screens` is the `screens` message), one fixture each under `tests/protocol/provider/`. Providers are Rust traits in `notchd-providers`, one backend module per OS behind `cfg(target_os)`; a module asks the trait, never the OS. A provider with no backend on an OS reports `unavailable` and the module shows its empty state.

Sidecars (`core/live/codex-live.py`, `local-live.py`) are Python, launched by the core as `live::bridge_args` does and speak the same line-JSON documented in their docstrings (`{"status"...}`, `{"transcript"...}`). The core reduces those lines (`live::reduce`) and builds the Voice view. Porting them to Rust is a later phase, out of this plan.

## Config

Unchanged for users: `~/.config/notchd/config.json` (`$XDG_CONFIG_HOME/notchd/config.json`; `%APPDATA%\notchd\config.json` and `~/Library/Application Support/notchd/` or `~/.config/notchd/` on the others: **Open**). The daemon is its only reader and writer. The rules of CLAUDE.md carry over as tests:

- A missing key means its default. Defaults live once, in `Config::DEFAULTS` (ported from `Config.DEFAULTS`) or the module's schema in `module.json`. **Never write a default into the user's file.**
- `set` with `null` removes the key and any parent object left empty (`withValue`).
- The file is watched. A good edit applies live. A parse error keeps the last good config and sends `{"t":"config-error","message":"..."}`; the UI shows it (the settings window's top note) and the file is not touched.
- Writes are atomic (temp file in the same dir, rename), so the watcher never reads half a file, and a write the daemon made is not reapplied as an external edit.
- `NOTCHD_CONFIG` overrides the path, for `make dev` and every test; nothing under `tests/` touches a real user path (hermetic invariant).

## Distribution

The Omarchy plugin is the repo: `omarchy plugin add <git url>` does `git clone` and nothing else (read in `omarchy-plugin-add`: no build hook). `omarchy plugin validate` requires a manifest with `kinds`, `entryPoints` paths that exist and are relative and safe, a non-reserved id, and **no symlink anywhere in the folder** (read in `omarchy-plugin-validate`). So the plugin cannot symlink a binary, and nothing builds Rust at install. `shell.qml` at the root is the standalone host.

How a host finds and launches the daemon (options considered):

| Option | How | For | Against |
|---|---|---|---|
| A. Binary on `PATH` | host runs `notchd daemon --ensure`; a package provides it (AUR `notchd`, distro, Homebrew, winget, `cargo install`) | normal OS practice; one binary for both hosts; the daemon can outlive the shell; a systemd user unit or launchd agent can socket-activate it | the plugin alone no longer works: `omarchy plugin add` plus a package |
| B. Plugin downloads a pinned release | `Service.qml` checks `$XDG_DATA_HOME/notchd/bin/notchd`, else downloads the release matching `manifest.json`'s `version`, verifying a SHA-256 kept in the repo, using the components installer's `files` kind (`install.py`, how LocalAI is fetched today) | `omarchy plugin add` stays the one step; the core version always matches the plugin's QML | a download on first run, in the shell's process; a checksum file to maintain per release; offline installs fail |
| C. Build from source on first run | `cargo build --release` from the plugin folder | no release infra | needs a Rust toolchain, minutes of build, a build step inside a shell plugin |
| D. Commit prebuilt binaries | `bin/linux-x86_64/notchd` in the repo | zero steps, offline | binary blobs in git, one per arch, nothing reviewable; `validate` allows it (no symlinks needed) |

**Decided** (2026-11-02): **B as the main path on Omarchy, A accepted.** The host runs `notchd daemon --ensure`, the binary found by order: `$NOTCHD_BIN`, `PATH` (an AUR package), `$XDG_DATA_HOME/notchd/bin/notchd`; when none is found, the plugin downloads the release its version pins, verified against a SHA-256 kept in the repo, into that last place. macOS and Windows bundle the daemon inside the app, so the question is Linux-only. The version handshake (`proto`, plus the daemon's version) guards against a stale daemon: the host restarts a daemon older than the plugin, and shows "update notchd" if it cannot.

**Mechanism (B, as built).**
- *Release.* A `v*` tag runs `.github/workflows/release.yml`: native builds on an x86_64 and an aarch64 runner (no cross toolchain, tests run on the real target, glibc floor of the oldest runner image), `notchd-<version>-<target>.tar.gz` (a folder holding `notchd` and `LICENSE`) plus a `.sha256` uploaded to the GitHub release.
- *Pins.* The workflow then opens a PR adding the hashes to `native/releases.json` (`{ "<version>": { "<target>": { "url", "sha256" } } }`). The checksums live in the repo and change only by a reviewed merge, never taken from the release at install time. The version the plugin wants is `manifest.json`'s `version`, equal to the daemon crate's (tested).
- *Sidecars.* The release is the one executable. The Python sidecars (`core/live/*.py`, `core/components/{install.py,localai.py,components.json}`) are embedded in it (`native/notchd-daemon/src/assets.rs`, `include_bytes!`) and written to `$XDG_DATA_HOME/notchd/sidecars/<hash>/` (`~/.local/share` by default) the first time a voice module or the installer needs them; the folder is named by a hash of the files, so a new build writes its own, an unchanged one reuses the last (marked complete once written, written to a temp folder and renamed so two daemons never see a half-written one), and folders of other builds untouched for a week are deleted. A dev override: `NOTCHD_CORE_DIR` (a checkout's `core/`) wins, then a UI's deprecated `coreDir`, then the embedded files; the per-file `NOTCHD_LIVE_BRIDGE`, `NOTCHD_LOCAL_LIVE_BRIDGE` and `NOTCHD_COMPONENTS_INSTALLER` (tests) win over all. `make dev` and `make restart` rebuild the daemon, so its embedded files are the checkout's. The built-in modules' JSON files are embedded too (`include_str!`, `notchd-modules`), so the tarball needs no format change: still `notchd` and `LICENSE`.
- *Fetch.* `scripts/fetch-daemon.py [--dest DIR] [--version V] [--releases FILE]` (standard library only) looks up V and this machine's target, downloads to a temp file, verifies SHA-256, extracts `notchd` atomically into `$XDG_DATA_HOME/notchd/bin`, prints its path, and downloads nothing when `notchd --version` there already says V. A failure prints one JSON line `{"error": <code>, "message": ...}` to stderr and exits with: 2 usage, 3 `no-release`, 4 `unsupported-arch`, 5 `offline`, 6 `checksum-mismatch`, 7 `failed`. Nothing is left behind on a failure. The host runs it as a process and shows the message.
- *Installer for path A.* `scripts/install.sh` is the installer for distributions with no host that fetches (GNOME: extensions.gnome.org forbids a download by the extension): it takes the archive pinned in `native/releases.json`, checks the SHA-256, puts `notchd` in `~/.local/bin` and installs and starts a systemd user unit running `notchd daemon` (a plain service: the daemon binds its own socket, so no socket activation; `--ensure` finds it running). It is self-contained, so it needs nothing from the archive but `notchd`; `--uninstall` leaves the config. See [hosts.md](hosts.md#installing-the-daemon-on-other-distributions).

**Instances, decided:** one daemon per user. Every host connects to the `default` instance; other instances exist only for `make dev` and tests, with their own config. The daemon outlives the shell (it is detached, or a systemd user unit from a package), so a shell reload or `make restart` keeps a voice conversation, terminal sessions and timers running; the UI just reconnects.

## Migration plan

Rule for every phase: **Linux behaviour is identical at its end and `make check` passes** (all layers, CLAUDE.md). The strategy is a strangler: the Rust core grows a slice at a time behind a feature the QML reads through `DaemonClient`; the QML rules it replaces are deleted only after the slice's QML tests pass against the daemon.

**Status.** Phases 0 to 3.7 are done; phase 4 (cleanup) is in progress; phase 5 (other OSes) is later and not started. The Linux UI is a client of the daemon: the config, the settings schema, the notch state and every module (the built-in ones, user modules as processes) are decided and run in `native/`, and the QML binds `state`, `config`, `schema`, `module` and `view` and draws (`core/client/`, `ViewTree` and its primitives, `CoreModule.qml`). What phase 4 has left is in [Phase 4](#phase-4-cleanup-serial). The notes below record each slice as it landed, newest first; they are history, and the sections above are the contract. The shared `tests/vectors/rules/` check the Node files still in `core/` and the Rust against the same cases (`knownDiff` marks the accepted differences); `tests/protocol/` holds a fixture for every message; `make rust` is part of `make check`.

**Catalog and sidecars in the daemon (done, phase 0 of the GNOME work).** The built-in modules' JSON files moved to `native/notchd-modules/modules/` and are embedded in the daemon (`notchd_modules::catalog()`, `Store::set_builtin_catalog`), which sends their titles, kinds and fields back in `schema.modules`; the Python sidecars are embedded and written out on first use (`assets.rs`). `NotchService.qml` no longer reads those files, sends only the host's and the terminals' entries and no `coreDir`; its `moduleInfo` for a built-in or a user module comes from `schema.modules`.

**Task 3.6 (voice and components) is done.** `codex-live` and `local-live` are one module (`native/notchd-modules/src/voice.rs`, thin `codex_live.rs` and `local_live.rs`) over the rules of `notchd-core/src/live.rs`. The daemon runs the Python bridges as sidecars (`notchd-providers/src/sidecar.rs`: a process in a group of its own, its lines read on a thread, its input written; `notchd-daemon/src/sidecars.rs`: one per module, `Note`s `sidecar` naming the module and the run, so a replaced run's lines are ignored), finds them (first through the `catalog`'s `coreDir`, now deprecated and replaced by the files embedded in the daemon; or `NOTCHD_LIVE_BRIDGE`, `NOTCHD_LOCAL_LIVE_BRIDGE`, which the tests set to fakes) and stops them with the module (unloaded) and the daemon. A session follows the notch: the host tells a module, whenever the notch state changes and its answer does, whether a notch is open on it by a click or a shortcut and whether its expanded view is on screen (`CoreModule::shown`, `CoreHost::notches_changed`). **Level streams**: the bridge's `level` and `mic` lines are read only while the view is on screen, quantized to 20 steps, and move the view at most every 100 ms (`next_due`/`tick`), and the closed notch's dot does not swell, so a conversation costs the UI about ten small `view` messages a second while it is open and on screen, none otherwise; the UI needed no new message, the daemon already knows what is on screen. The `live` IPC (`live <id> start | stop | toggle | new | mute | unmute`) and the new `component` IPC are the daemon's. **Components**: the registry is compiled in (`components::REGISTRY`, the same file); the daemon runs `install.py status` on a thread of its own (once when the settings window opens and every second while it is open and something installs; `NOTCHD_COMPONENTS_INSTALLER` for a fake) and `install`, `cancel` and `remove` on request, which the installer detaches, so an installation still survives the window, the shell and the daemon. The state feeds `schema::build`: the Components page, every `component` row with its `row` (`components::view`), `components` fields expanded to the chosen language's voices, and the choices whose `needs` are not installed `disabled` (`decorate`, `gate`) are the daemon's; `ComponentRow.qml` draws `field.row` and a button is `{"t":"ipc","method":"component",...}`, and the built-in modules' pages are no longer rebuilt in QML. What stays QML: `Remove` asking twice (a UI affordance), and `Settings.shownFields` (`when`), which the window applies to its own optimistic config so a language edit hides and shows fields at once; the locked choices and the voice rows follow the next `schema`. Deleted: `LiveModule.qml`, `ComponentStore.qml`, `modules/{codex-live,local-live}.qml`, `Live.js`, `Components.js` and their Node tests (the Rust ports and the shared vectors stay).

**Task 3.4 (media) is done.** `native/notchd-modules/src/media.rs` is the `media` module (the chosen player, the pause grace, the delayed peek on a new track, the controls as view actions: `toggle`, `next`, `previous`, `seek`), over the `media` provider: `notchd-providers/src/linux/mpris.rs`, `zbus` 5 with its default features (its blocking API; pure Rust, no libdbus), on two threads of its own that never touch the hub's lock. A reader thread blocks on the MPRIS signals (`PropertiesChanged`, `Seeked`, MPRIS names coming and going) and a worker reads the players after each, hands the hub only what changed (`media::changed`: a playing position that runs on by itself is not news) and makes the commands' D-Bus calls (`MediaCommand`, sent as `Effect::Media`); it does not open the bus until the module is loaded. The position is not streamed: the `progress` node carries `anchor` and the module ticks only to change the clock's seconds. The cover is an `image` (the UI loads `file://` art and paths itself; remote art loads only where the UI has a network). The `text` primitive grew `bold`. The test hook `test-media <MediaData JSON>` gives demo players; `tests/mpris_test.py` runs the real daemon against `tests/visual/demo-player.py` on a private bus. `core/modules/media.qml` is gone (`media.json` stays, now under `native/notchd-modules/modules/`).

**Task 3.5 (agents and terminals) is done.** `claude`, `codex` and the terminals of the config live in the daemon (`native/notchd-modules/src/{claude,codex,terminal}.rs`, over the shared `term.rs`: titles, the working dot, the done peek, `ask`, the idle stop, the views; `plan.rs`: the plan usage), and so do their programs: `notchd-providers/src/pty.rs` runs them in PTYs (`portable-pty`: one API over `openpty` and ConPTY, the doc's choice; it brings `libc` and a few small crates, and the alternative was a `forkpty` of our own for Unix and another thing for Windows), `notchd-daemon/src/ptyhost.rs` keeps each session (the last 1 MiB of output, the title read from OSC 0/2, the views attached) and `attach.rs` is the surface connection of [Free surface](#free-surface-tier-3), both ends. **Terminal sessions survive a shell reload** (`make restart`, a crashed shell): the program is the daemon's, the new shell's view attaches and is told what was drawn; they end with the daemon, or when the module is unloaded, restarted or idle. The plan usage is asked on a thread of its own per request (`notchd-providers/src/usage.rs`: the login file is read, never written; `curl` makes the HTTPS request with the token on its standard input, not on a command line; the same 5-minute interval, the 60 s minimum after a finished task, the 429 `Retry-After` wait, and a lapsed login looked at again every 5 s without touching the network until its file changes; `NOTCHD_OFFLINE=1` asks nothing); tests give the daemon a fake network and a scratch home. The IPC methods `agent`, `claude`, `codex` and `terminal` are the daemon's, with the answers of the QML ones (`no agent <id>; agents: claude, codex`, `no terminal <id>; terminals: ...`, `<id> module not loaded`, the usage, `the terminal needs the qmltermwidget package` when no UI connected can draw one); the host's own target (`omarchy-shell ... agent`) cannot wait for the daemon, so it answers `ok` once the command is sent (the first two kinds of error it still knows). `NotchService` makes a `CoreModule` for them like the timer's, and the expanded view's `surface` node is drawn by `terminal/TerminalSurface.qml` (a frame, `TerminalView.qml` and the helper). Deleted: `TerminalModule.qml`, `AgentUsage.qml`, `modules/{claude,codex}.qml`, `terminal/CustomTerminal.qml`, `Agent.js`, `Usage.js` and their Node tests (the Rust ports and the shared vectors stay; `Terminal.js` is down to the widget's side). Tests: Rust units for the rules and the module, the PTY (a script's output, input, resize, the process group stopped and reaped), the sessions (replay, titles across chunks, replacement) and `tests/terminals.rs` over a real socket with the fake `claude` and `codex`; `tests/qml/shell.qml` keeps the visible facts (the title, the dot, the peek, Escape going to the agent, Ctrl+Escape closing the notch, the same program behind the next view, restart, quit) against the real widget where it is installed.

**Task 3.3 (timer, activities, resources, osd) is done.** The four live in the daemon: `native/notchd-modules/src/{timer,activities,resources,osd}.rs` implement `CoreModule` (state in, facts, view trees and effects out) and `notchd-daemon/src/corehost.rs` drives them like the module host drives a user module's process: loaded while a notch lists them (the OSD, standalone, once a host declares it in its `catalog`), told the config, the time (`next_due`, the ticker), the resources provider's readings (`notchd-providers/src/linux/procfs.rs`, on a thread of its own), an IPC method (`timer`, `push`, `clear`, `osd`, answered by the daemon with the QML modules' strings) or an `event`. The notch draws their views through `core/CoreModule.qml` (see [modules.md](modules.md#a-module-in-the-daemon)), as it draws a user module's. `core/modules/{timer,activities,resources}.qml`, `Countdown.js`, `Resources.js` and their Node tests are gone (the JSON files stay, now under `native/notchd-modules/modules/`: the settings pages and the modules' defaults read them); `hosts/omarchy/modules/osd.qml` is a `CoreModule` that only owns the `osd` IPC target and forwards it. The Quickshell IPC target cannot wait for the daemon, so for `push`, `clear` and `timer` it answers `ok` (`invalid json` for a bad push); `notchd ipc` has the module's own answers. Primitives grew `width`, `height`, `minWidth`, a row's and column's `maxWidth` and a button's `big`.

**Task 3.2 (notch state) is done.** The daemon decides the notch state and `NotchService`, `Notch` and `NotchCopy` bind its `state`: modes, hover timers, pinning, tabs, peeks and where they go (including the visitor and standalone notches), navigation, which screen each copy is on, `covered`, `needsWindow`, the keyboard a window asks for, the notches' place and the space to reserve. The UI sends raw inputs: the pointer per copy (counted over the surfaces that report it), a press, `pin`, `focus`, `tab`, `drag` (`start`, `drop`, `cancel`), navigation keys (`Protocol.wireKey`), `measure` (`DepthReport.qml`) and, for the modules it still loads, their `facts`, `peek`, `peek-dismiss` and `module-notch`; the notch list, the visitor's and the standalone modules' copies are the state's. `State.js`, `Navigate.js`, `ReservedSpace.qml` and their Node tests are gone (the Rust ports and the shared vectors stay; the JS runner skips those two files). The QML keeps the drag's motion, packing and the outline (`Position.js`, `Outline.js`), animations and windows, the notch's config keys the state does not carry (`workspaces`, `reserveSpace`, `idleIcons`, its key prefix: `Config.notches`), and the sync `state` IPC string, built from the bound state. While the daemon is away the last `state` stays: the notch is frozen where it was under the "notchd not running" row. Lists in a daemon message reach the UI as sequences, so the service takes plain copies of `config`, `schema` and `state`. A hover opens within 2 to 4 ms of its delay (`tests/qml/shell.qml`, "hover opens the notch within the delay"). The test hooks are down to `test-linger`.

**Task 3.1 (config, settings schema, IPC) is done.** `NotchService` binds to the daemon's `config` (parsed config and `resolved` hover delays) and no longer reads, watches or writes the file: an edit applies at once, goes to the daemon as `set` (sets to one key coalesce for 120 ms; they wait while the daemon is away), and the daemon's config replaces the service's copy once the edits are answered (so its `with_value` is what counts). A config the daemon cannot parse is logged as before (`<logName>: ignoring <path>: <error>`) and the last config stays. The settings window builds the core's pages and fields from the `schema` (the service reports its `catalog`, `screens` and the settings page for it); the Components page, the built-in modules' pages, the host's own fields and the edge cards' previews stayed built in the QML at this slice (`Settings.windowPages`/`windowFields`); 3.6 moved the Components page and the modules' pages to the daemon. The launcher brings the daemon back when it dies and the notch shows "notchd not running" meanwhile ([hosts.md](hosts.md#the-daemon)). `IpcHandler` cannot await a reply, so the notch methods act through the daemon (the handler sends the `ipc` and answers `ok`, or `no notch <id>` from the last state) and `state` answers from the bound state (see [Requests](#requests-ui-cli-scripts)). `Config.js` and `settings/Settings.js` stay, for what is still decided in the QML: `Config.notches`, `terminals`, `moduleLoadList`, `withValue` (the optimistic copy of an edit) and the notch's own `withWorkspace` (3.2), and the controls' mechanics, the module catalog and the Components page in `Settings.js`; their Node tests stay with them, and the Rust ports and the shared vectors are unchanged. Later slices and the cleanup narrowed those two files to the optimistic copy of an edit and the controls' mechanics. The QML tests give every service a daemon of its own (`tests/qml/TestService.qml`, the real binary on a scratch socket) and `tests/qml/shadow.qml` starts its daemon through the service.

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
*Done (std threads, one per connection; `--ensure` returns 0 at once when a daemon answers, else runs the daemon in the foreground (task 2.5 made it start the daemon detached, replace one of another version through the `shutdown` IPC method, and print the socket path); `config` is read on each `hello`, not yet watched; the router answers every `ipc` method with `err` code `method`: `unknown method: x`, or `x is not implemented yet` for a documented one, and `set`/`edit` with a not-implemented `err`; input messages are accepted and ignored).*

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

**End of Phase 2 state (done):** the daemon ran and served config and a `state` mirror computed alongside the QML and compared with it in a test, which found no disagreement; nothing the user saw changed. Task 3.2 moved the authority and the comparison went with it: `tests/qml/scenarios.qml` holds the same scenarios as plain behaviour checks.

### Phase 3: slices of authority move to the core (serial per slice; slices after the first are parallel)

For each slice, the QML side deletes its rule imports and binds to the daemon; the slice's existing QML tests (`tests/qml/shell.qml`, unchanged in intent) must pass against a real daemon started by `scripts/qml-test.sh` with a temp socket.

| Task | Owns | Depends | Moves |
|---|---|---|---|
| 3.1 Config and IPC (done) | `core/NotchService.qml` (config and IpcHandler parts), `core/settings/SettingsWindow.qml`, `core/settings/SettingRow.qml` (binding to `schema`), `native/notchd-daemon/src/ipc.rs`, `native/notchd-core/src/ipc.rs` | 2.1, 2.3 | the config file (read, watch, write), `set`, every `ipc.md` method but those that need a module; `docs/ipc.md` unchanged. After this, `Config.js` and `Settings.js` are deleted with their Node tests (replaced by Rust) |
| 3.2 Notch state (done) | `core/NotchService.qml` (state parts), `core/Notch.qml`, `core/NotchCopy.qml` | 3.1 | modes, hover timers, peeks, navigation, reserve targets. The QML keeps drag, packing, outline. `State.js`, `Navigate.js` go |
| 3.3 Timer, activities, resources, osd modules (done) | `native/notchd-modules/src/{timer,activities,resources,osd}.rs`, `core/modules/{timer,activities,resources}.qml` (deleted, replaced by the renderer), `native/notchd-modules/modules/*.json` (moved from `core/modules/`) keep | 2.2, 3.2 | the three built-in modules and `osd` as core modules drawn by `ViewTree`. Their `tests/qml/shell.qml` cases keep asserting on the same visible text and layout facts |
| 3.4 Media (done) | `native/notchd-modules/src/media.rs`, `notchd-providers/src/linux/mpris.rs`, `core/modules/media.qml` (deleted) | 2.2, 3.2 | MPRIS through `zbus` against the demo player (`tests/visual/demo-player.py`) on a private bus (`scripts/isolated.sh`); the `equalizer` and `progress.anchor` primitives |
| 3.5 Agents and terminals (done) | `native/notchd-modules/src/{claude,codex,terminal,term,plan}.rs`, `notchd-providers/src/{pty,usage}.rs`, `notchd-daemon/src/{ptyhost,attach}.rs`, `core/terminal/TerminalSurface.qml`, `core/TerminalModule.qml` and `core/AgentUsage.qml` (deleted) | 3.2 | PTY in the core, title events, usage fetch, the fake `claude` and `codex` fixtures of `tests/qml/fixtures`; the terminal surface bridge |
| 3.6 Voice and components (done) | `native/notchd-modules/src/{codex_live,local_live}.rs`, `notchd-core/src/components.rs` wiring, `core/LiveModule.qml`, `core/ComponentStore.qml` (deleted), `notchd-providers/src/{sidecar,components}.rs`, `notchd-daemon/src/sidecars.rs` | 3.2 | the Python sidecars launched by the core; `tests/*_test.py` unchanged (integration layer); `meter`, `list` |
| 3.7 User modules (done) | `native/notchd-daemon/src/modhost/` (finish), `docs/modules.md` (a new "User modules" section), `tests/modules/` | 2.4, 3.2 | the manifest, settings from `module.json`, provider subscriptions; a sample module in `tests/fixtures/modules/hello/` |

3.3 to 3.7 are independent after 3.2 and run in parallel (each owns separate Rust files and separate QML files; the shared `core/NotchService.qml` is owned by 3.1 and 3.2 only, and each of 3.3 to 3.7 hands its QML changes to a module file they own, never to the service).

### Phase 4: cleanup (serial, in progress)

**4.1** Delete what no longer has an importer: the QML modules (gone: `core/modules/` holds only `claude.svg`; their JSON files are `native/notchd-modules/modules/`), the ported Node tests and JS files (gone except those listed under [the inventory](#corejs-and-coresettingsjs)). What remains in `Config.js` and `settings/Settings.js` beyond what the QML calls (the schema-building functions the daemon replaced, kept alive only by their Node tests and vectors) is to be deleted with those tests. Keep: `Position.js`, `Outline.js` (UI-side, golden vectors), `Terminal.js`'s `isLeaveKey`, `settings/Settings.js`'s control mechanics. Owns: those files.
**4.2** Docs: `DEVELOPMENT.md` (layout, prerequisites: Rust toolchain, the gate), `CLAUDE.md` (the invariants of this migration), `README.md`, `docs/*.md` describe the code as it is after the migration; `CHANGELOG.md` has the daemon lines. Owns: those files.
**4.3** `make times` to include the Rust layer: `build/times/` entries per `cargo test` target and a size budget for its tests (as for the Node ones: 0.1 s). Today the Rust layers are timed as a whole.

### Phase 5: other OSes (later, out of the first round)

Per OS, three tasks that touch no Linux path: **the UI** (SwiftUI/AppKit; native on Windows) implementing the tokens, the primitives, `canvas`, the `terminal` surface, window placement and the geometry from `tests/vectors/geometry/*.json`; **the providers** (`notchd-providers/src/{macos,windows}/`); **packaging** (`.app`, installer, login item, the named pipe). Each UI is tested against the same `tests/protocol/` fixtures and a fake daemon (no real core needed), then against the real daemon.

### GNOME UI (planned, decided 2026-11-02)

GNOME (Mutter) has no layer-shell, and a Wayland client there cannot place itself, stay above or reserve an edge: only code inside `gnome-shell` can. So GNOME gets **a second Linux UI, a GNOME Shell extension** (GJS, St/Clutter) in `hosts/gnome/`, speaking this protocol like the QML one. It decides nothing. Running Quickshell as a plain window held in place by a small extension was ruled out (focus stealing, no click-through, flicker on resize, no Quickshell package on Ubuntu or Fedora).

How the extension maps the UI's jobs: overlay with `Main.layoutManager.addTopChrome()`; `reserve` with an invisible actor that has `affectsStruts: true`; `pointer`, `drag` with reactive actors; the keyboard (`focus`, `key`) with `Main.pushModal()`; `screens` from `global.display`, the layout manager's monitors, `workspace_manager` and `monitor.inFullscreen` (workspaces and fullscreen supported, unlike `GenericCompositor.qml`); the outline and `canvas` with Cairo in an `St.DrawingArea`; tokens from `org.gnome.desktop.interface` (`color-scheme`, `accent-color` where present); the socket with `Gio.SocketClient` and `Gio.UnixSocketAddress`.

Decided:

- **Placement**: a notch on the top edge hangs below GNOME's top panel (`barEdge: "top"`, as `topOffset` does in the standalone host). It never covers the clock or changes the panel.
- **Versions**: GNOME Shell 46 and later (Ubuntu 24.04 LTS). The accent color is read where it exists (47+); on 46 the default blue.
- **Icons**: named icons in the protocol (open question 5): an optional field next to `glyph`, from a table in `notchd-proto`. The QML UI maps names to Nerd Font glyphs, GNOME to Adwaita symbolic icons. `glyph` stays as the fallback, so `proto` stays 1.
- **Terminal**: not in the MVP. The extension does not announce `surfaces: ["terminal"]`, so the daemon draws its placeholder. Later, a GTK4 + VTE helper window running `notchd attach <ref>`, which the extension keeps above, under the notch and out of the window switcher.
- **Settings**: the extension's `prefs.js` (libadwaita) renders the daemon's `schema`, one control per field type of [settings.md](settings.md).
- **Distribution**: extensions.gnome.org takes no extension that downloads an executable, so path B does not apply. The daemon comes from the release tarball and an install script (`notchd` in `~/.local/bin`, a systemd user unit, the extension); the extension runs `notchd daemon --ensure` and says "install notchd" when it finds no binary. `.deb` and `.rpm` later.
- **Shared JS**: the extension ships as a self-contained folder (`../../core` is out of reach from the EGO zip). `make gnome-zip` copies `client/Protocol.js`, `client/prims/Tree.js`, `Position.js` and `Outline.js` into it with an `export` line added; a test checks the copies match. The sources stay ES5.
- **Catalog and sidecars move to the daemon**: today the QML sends `catalog` with `core/modules/*.json` and `coreDir`, where the daemon finds `live/*.py` and `components/`. Every built-in module is the daemon's, so the daemon ships its own catalog and sidecars, and a UI's `catalog` carries only the host's modules. Done before the extension, it serves every later UI.

Order: (0) the core work above (icons, catalog, install script and unit), **done** (named icons, the daemon's own catalog and sidecars, `scripts/install.sh` with a systemd user unit); (1) a spike: connect, `hello`, draw the lip, `pointer` enter and leave, struts, `pushModal`, a headless shell test (`gnome-shell --headless --virtual-monitor` under `scripts/isolated.sh`) on 46 and the newest; (2) the MVP: every primitive but `canvas` and `surface`, the card, outline and animations, `screens`, `pointer`, `key`, `drag`, `measure`, `reserve`, theme; (3) `canvas` and rounded `image` with Cairo; (4) settings; (5) the terminal helper; (6) EGO, docs, changelog. Tests: the `tests/protocol/` fixtures against a fake daemon, then the headless shell against the real daemon, screenshots for the visual check. The `osd` module stays Omarchy's (GNOME has its own); the voice components stay Arch-only (open question 10).

**Spike (1), on GNOME 51: what it answered.** The extension is in `hosts/gnome/` (`client.js` the socket, `screens.js` the adapter, `notch.js` a copy, `view.js` the primitives, `theme.js` the tokens). `make gnome-dev` opens a GNOME session in a window of the desktop you are on (`gnome-shell --devkit`) with its own daemon, config, D-Bus and runtime dir; `make gnome-test` (`tests/gnome_test.py`, not in `check`: it needs `gnome-shell`) runs the extension in a headless shell with two virtual monitors against the real daemon. `make gnome-zip` builds the folder and zip for extensions.gnome.org, `lib/` generated, the test hooks left out.

- **Holds, as planned.** The socket client (with reconnect), `hello`, `screens`, `pointer`, `measure`, `reserve`, `key`, `focus` and `pin` against the real daemon, unchanged. Monitors come by connector (`Meta.MonitorManager.get_logical_monitors()` then `Meta.Monitor.get_connector()`, falling back to `monitor-<i>` where the version lacks it), so `position.screen` means the same as on Hyprland. The notch hangs below the top panel on the primary monitor and at the top of the others. The outline is `Outline.js`'s path traced with Cairo (its `Q` raised to a cubic). Named icons draw as Adwaita symbolic icons, the rest of the glyph-only ones as text.
- **Reserve.** A strut is an invisible chrome actor with `affectsStruts` that touches the monitor's top edge, so it is the panel's height plus the daemon's depth. A notch that follows focus reserves on every monitor it may go to, as the daemon decides.
- **Keyboard.** `Main.pushModal` stands in for layer-shell's keyboard focus. Under it a click elsewhere reaches no window, so the copy watches the stage's `captured-event` and treats a click outside the card as a click away (`pin` off). Escape closes a pinned notch; navigation takes a grab of its own and sends keys with no notch, as the QML window does. Every grab goes when the daemon connection does, so a dead daemon never holds the keyboard.
- **The overview is modal.** While it shows, the pointer never reaches the notch. GNOME opens it at startup.
- **`disable()`** takes away everything: five disable/enable cycles leave only the last enable's actors and no grab (the test fails with a strut left behind).
- **API differences seen.** GNOME 51 dropped `affectsInputRegion` from the chrome params (an X11 thing; passing it throws) and `Clutter.get_default_backend()` (the backend is `global.stage.context.get_backend()`); `St.BoxLayout` takes `orientation` where it has it, `vertical` before. GJS's `TextDecoder` has no `stream` mode: the client decodes only complete lines.
- **GNOME 46** (Ubuntu 24.04, `make gnome-dev-46` and `make gnome-test-46`, a podman image from `tests/containers/gnome-46.Containerfile`): the extension loads and draws, hover, pin, Escape and navigation work, checked by hand in the nested window. 46 has no `get_logical_monitors()`, so monitors came out as `monitor-<i>`; the extension now asks `org.gnome.Mutter.DisplayConfig.GetCurrentState` on every version (async: gnome-shell serves it itself) and matches logical monitors by origin, and sends no `screens` until the names are in. Clutter warns "Spurious clutter_actor_allocate" for a copy first laid out. GNOME 46 has no devkit: `--nested` draws the session in an X11 window (Xwayland, `--ipc=host` for shared memory). In a container the shell needs a system bus, no logind activation file (it throws when activating logind fails) and dconf (without it `gsettings` writes nowhere and the extension is never enabled). The image needs `librsvg2-common` too, which `--no-install-recommends` leaves out: without an SVG loader no symbolic icon draws and the shell often dies with SIGSEGV while starting (a desktop install has it). `make gnome-test-46` and `gnome-dev-46` run with a tmpfs `/tmp`, as a desktop has. **Open on 46:** after `gnome-extensions disable` and `enable` the extension does not come back (nothing drawn, no test hook, nothing logged), already at the spike's commit; it passes on 51. GNOME disables extensions on the lock screen, so this is the next fix.
- **No keyboard shortcut yet.** Navigation starts from `notchd ipc navigate`; on GNOME the extension should register its own (`Main.wm.addKeybinding`, a GSettings schema compiled into the extension, set in its preferences).
- **The headless test.** The extension exports test hooks on the session bus only when `NOTCHD_GNOME_TEST` is set (`testhook.js`, loaded with a dynamic import): what each copy shows and its target geometry, the work areas, the grabs, Clutter virtual pointer and keyboard devices inside the shell, and `Shell.Screenshot` (`build/gnome/`). What it took: a runtime dir short enough for mutter's socket path (108 bytes), `XDG_CURRENT_DESKTOP=GNOME` (inherited, Hyprland's portal starts on the private bus and crashes), the host's `PIPEWIRE_RUNTIME_DIR` for the devkit window (it shows the session through PipeWire), hot corners and notification banners off, the overview closed before a case starts, and the document portal's and gvfs's FUSE mounts unmounted on the way out.
- **Not yet, for the MVP.** Easing is a plain `ease` of the card, not the QML's motion; the devkit and headless shells may turn animations off (`--force-animations` exists). The navigation highlight (number and name) is not drawn. Of the primitives only `row`, `column`, `text`, `icon`, `spacer` and `progress` draw; the others show a placeholder naming themselves. Text uses the system font. Clutter warns "needs an allocation" while a copy is first laid out.

Outside the phases: a `webview` surface in the Linux UI (QtWebEngine is a heavy dependency: **Open**; WebKitGTK is another), the sandbox for user modules, porting the voice sidecars to Rust.

## Open questions

1. ~~Daemon discovery and distribution~~: decided, see [Distribution](#distribution).
2. ~~One daemon per host instance, or one per user?~~: one per user, decided (see [Distribution](#distribution)).
3. **Geometry in each UI, or one shared crate?** `Position.js`/`Outline.js` are reimplemented per OS from golden vectors in this plan. A `notchd-geometry` crate exposed through FFI (UniFFI for Swift, a C ABI for the rest) would remove the duplication but QML would need to call it per frame. Revisit when the second UI is written.
4. **Compositor adapters**: UI-reported in v1 (CLAUDE.md keeps compositor code in `*Compositor.qml`). A Rust Hyprland provider (its IPC socket) would let the core serve non-QML UIs on Linux. Not needed by this plan.
5. ~~Icons~~: decided, named icons with `glyph` as the fallback (see "Named icons" under the primitive tree) and [GNOME UI](#gnome-ui-planned-decided-2026-11-02). A UI draws its own symbol for a name (Nerd Font in QML, Adwaita symbolic on GNOME, SF Symbols or Segoe Fluent later); `proto` stays 1. Was: v1 uses Nerd Font glyphs, as every module does today; macOS and Windows UIs would want native symbols.
6. ~~Terminal surface on the wire~~: a connection per surface carrying base64 lines, decided with 3.5 (see [Free surface](#free-surface-tier-3)); the Linux view runs `notchd attach` in `qmltermwidget` because the widget cannot be fed bytes. A patched `qmltermwidget` that exposed `Session::receiveData` would drop the helper process.
7. **Webview on Linux**: QtWebEngine (large dependency), WebKitGTK in a separate process embedded by the host, or none until a user needs it (modules say "signals a missing primitive"). Defer or decide?
8. **Security of the socket**: a Unix socket at `0600` in a `0700` directory protects against other users, not against another program of the same user; a module with `exec` permission can talk to it. Acceptable for "local experiences for yourself"; revisit with sharing and the sandbox.
9. **macOS media**: `MediaRemote` is private and has broken across releases. Alternatives (per-app scripting, the `nowplaying-cli` approach) are weaker.
10. **Components outside Arch**: `components.json`'s `kind: system` is pacman and the AUR. Other OSes need a per-OS install table, or the voice features stay Linux-only.
11. **Config location on macOS and Windows**: keep `~/.config/notchd` everywhere (simple, one doc) or use each OS's convention.
12. ~~Timer and clock text updates~~: decided, the core streams a new `view` once a second for a running timer (the module ticks only when the clock's seconds change), because it keeps the format rule in one place. A `text` node with a `countdown` field would remove the stream but put the rule in every UI.
13. ~~Tree size and nesting limits~~: decided, 2 000 nodes, depth 16 and 256 KiB per `view` (`notchd-proto::version`).
