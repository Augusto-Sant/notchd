# Hosts

The core (`core/`) has no desktop-specific code. A host instantiates it and fills in what only the desktop knows: its IPC name, its bar, its theme, its extra modules. Two exist, and the repository root is both:

- **Omarchy**: `manifest.json` points the Omarchy shell at `hosts/omarchy/*.qml`, which import the core with `import "../../core"`.
- **Standalone**: `shell.qml`, so `qs -p <checkout>` runs it. It has to sit at the root: Quickshell only resolves types from files inside the config folder, so a `hosts/standalone/shell.qml` importing `"../../core"` fails with "NotchSurface is not a type".

No symlinks: `omarchy plugin validate` rejects any inside the plugin folder.

## The daemon

Each host starts the notchd daemon (`native/notchd-daemon`, the `notchd` binary) and connects a `DaemonClient` to it, with `core/client/DaemonLauncher.qml`: it runs `scripts/ensure-daemon.sh` once at startup (`script: Qt.resolvedUrl(".../scripts/ensure-daemon.sh")`) and points its `client` at the socket path the script prints. One daemon serves the user, instance `default`, and it is detached: a shell restart leaves it running and the client reconnects.

`scripts/ensure-daemon.sh` finds the binary in this order: `$NOTCHD_BIN`, `notchd` on `PATH`, `$XDG_DATA_HOME/notchd/bin/notchd`; with none, it calls `scripts/fetch-daemon.py --dest "$XDG_DATA_HOME/notchd/bin"` (if present, else the error is "notchd not found"). It then runs `notchd daemon --ensure`, which returns at once when a daemon of the same version answers, and otherwise starts one in its own session with stdio to `$XDG_STATE_HOME/notchd/daemon.log` and waits (at most 10 s) for the socket. A daemon of another version is asked to stop (`notchd ipc shutdown`) and replaced; one that cannot be stopped is an error ("update notchd"). On failure the script prints one line of JSON, `{"error": "..."}`, and exits 1; the launcher logs it once (`state` is `failed`, `error` has the text).

While `NotchService` still decides everything the daemon runs in shadow: the client is connected, the UI shows nothing of it, and a failed start is only a log line. The launcher reads `NOTCHD_INSTANCE` (and the script and daemon `NOTCHD_CONFIG`, `NOTCHD_SOCKET`), which is how `make dev` runs its own `dev` instance on the dev config and never meets the user's daemon.

## NotchService

The single source of truth: config, modules, the notches and IPC. It makes a `Notch` per entry of the config's `notches` (one, from the top-level keys, without a list): its modules, position, workspaces. Each notch shows as one or more `NotchCopy`: one that moves (following focus, or kept on its home screen), or one per screen with `position.screen` `"all"`. The service also has a default notch (`visitor`), with every module and a copy at the top of each screen; its copies (`guest`) stay hidden, at rest so they hold no live view, with no window on screen until they open (and for a few seconds after, `State.needsWindow`), and only show a peek on a focused screen where no notch with that module is (`State.needsVisitor`). A copy holds the state the pointer and peeks change (hover, pin, peek, drag) and so its own mode.

| Property | Meaning |
|---|---|
| `ipcTarget` | IPC target name for scripts |
| `logName` | prefix for log lines; defaults to `ipcTarget` |
| `configPath` | absolute path of the JSON config (watched, live); defaults to `$XDG_CONFIG_HOME/notchd/config.json` |
| `hostModules` | `{ id: url }` of modules the host ships; they win over built-ins |
| `barEdge` | the screen edge the host's bar is on (`"top"`, `"bottom"`, `"left"`, `"right"`); a notch on that edge hangs from the bar |
| `compositor` | a compositor adapter: the focused screen, fullscreen windows and workspaces; the `workspaces` setting needs one that knows workspaces |
| `screens` | the screens a notch can be on; Quickshell's (tests give their own) |
| `defaultModules` | modules to load when the config lists none |
| `hostSettingsSchema` | settings-window fields for the host's own config keys (see [settings.md](settings.md)) |
| `hostSettingsPages` | extra settings-window pages, or new labels/icons for the core ones |

A host reads its own config keys from `service.config`. `service.notches` lists the notches (`notchById(id)` finds one) and `service.copies` every copy of every notch. Each notch has `edge`, `vertical` and `placement` (`Position.js` has the rules) and `showsOn(screen)`, whether its closed notch belongs on that screen's workspace.

A host with a bar can give it a button for notchd, as the Omarchy host does (`hosts/omarchy/BarIndicator.qml`): it lists `service.notches`, opens or closes one with its `open()`, `close()` and `toggle()` (a copy's `pinned` says whether it is open), and opens the settings window with `service.openSettings(index)` (no index: the page it last showed; a notch's index: that notch's page). The core has no settings button of its own: without a host button, the window opens over IPC (`settings`).

## NotchSurface

Puts the notches on screen: a `NotchWindow` per copy, the layer-shell overlay that places a `NotchCard` (the notch itself, which also renders offscreen) on the screen the copy names, on its edge, off the bar where they share one, and routes the pointer. It also moves the notch when it is dragged (`NotchCard` reports the drag, `Position.dragTarget` picks the edge and offset) and stores the drop as `position`. The pointer stays with the notch's window while the button is down, even over another screen, so a drag there is followed too: `Position.screenAt` finds the screen, a preview window (namespace `<layerNamespace>-drag`) shows the notch on it, and the drop names it as the home screen. A notch on every screen only takes the drop's edge and offset: all its copies move.

Notches on one edge of a screen go side by side (`Position.pack`): each window reports its card's wanted middle and closed length (`packItem`), the surface packs them per screen edge and hands each its `packShift` along the edge. Ones closer than two shoulders and a little air join flush into a row: each window gets the windows beside it (`joinStart`, `joinEnd`), and its card draws those ends square (`NotchCard.startSide`/`endSide`, `core/Outline.js`) and takes the row's depth (`Position.rowDepths`). A card at an end of the edge sits flush in the corner (`Position.place`'s `atStart`/`atEnd`). An open card goes into the packing at its length as it grows, so it pushes its neighbors along frame by frame, where they can go; it stays joined to them and, deeper, steps down to them (an open card takes no part in the row's depth). A closed one pushed off the edge gets `covered` and hides until it closes.

| Property | Meaning |
|---|---|
| `svc` | the service |
| `barHeight`, `barHidden` | the host's bar (its thickness, 0 for none), so the notch hangs off it where they share an edge and keeps clear of it elsewhere; its edge is the service's `barEdge` |
| `layerNamespace` | layer-shell namespace |

With the config's `reserveSpace` on, it also maps one empty window per screen edge a notch keeps clear, namespace `<layerNamespace>-reserve`, holding an exclusive zone as deep as the deepest closed notch on that edge (`State.reserveZones`: notches sharing an edge don't add up). Each copy's depth comes from `ReservedSpace.qml`: the rest lip, or the deepest compact view seen while a module is active; it grows at once and shrinks once the notch has collapsed, 400 ms later. It sits on the overlay layer, so the compositor stacks it inside the bar's own zone; a host bar needs a normal exclusive zone for the strip to land where the notch hangs. Moving the notch to another edge replaces these windows: a mapped layer surface keeps its first anchors. A monitor switched off takes its layer surfaces with it; these windows and the notches' own reopen when it comes back (`KeepShown.qml`).

Give the service and the adapter ids that differ from these property names (`service`, `compositorLoader`): `svc: svc` inside the surface reads as the surface's own property.

## Theme

Set `Theme.provider` to an object mapping the host's theme onto the core's tokens. See [theming.md](theming.md).

## Compositor adapters

`HyprlandCompositor` and `GenericCompositor`. An adapter tells the notch which screen is focused, whether a fullscreen window covers it and which workspace each screen shows (`supportsWorkspaces`, `knownWorkspaces`, `activeWorkspace(screen)`); the generic one assumes the first screen, never fullscreen, and no workspaces, which hides the `workspaces` setting. `core/GenericCompositor.qml` has the contract.
