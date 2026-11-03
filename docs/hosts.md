# Hosts

The UI core (`core/`) has no desktop-specific code, and neither does the daemon it talks to (`native/`). A host instantiates the UI core and fills in what only the desktop knows: its IPC name, its bar, its theme, its extra modules. Two exist, and the repository root is both:

- **Omarchy**: `manifest.json` points the Omarchy shell at `hosts/omarchy/*.qml`, which import the core with `import "../../core"`.
- **Standalone**: `shell.qml`, so `qs -p <checkout>` runs it. It has to sit at the root: Quickshell only resolves types from files inside the config folder, so a `hosts/standalone/shell.qml` importing `"../../core"` fails with "NotchSurface is not a type".

No symlinks: `omarchy plugin validate` rejects any inside the plugin folder.

## The daemon

Each host starts the notchd daemon (`native/notchd-daemon`, the `notchd` binary) and connects to it. The host gives its `NotchService` the script (`daemonScript: Qt.resolvedUrl(".../scripts/ensure-daemon.sh")`) and its client name (`daemonClientName`); the service runs `core/client/DaemonLauncher.qml` with them, which runs `scripts/ensure-daemon.sh` at startup and points a `DaemonClient` at the socket path the script prints. One daemon serves the user, instance `default`, and it is detached: a shell restart leaves it running and the client reconnects.

The daemon is the authority for the **config** (it reads, watches and atomically writes the file; the service binds to its `config` message and sends every edit as `set`), the **settings schema** (the service sends the daemon the host's own modules (`catalog`: the Omarchy `osd`, the config's terminals; the built-in modules' titles, fields and defaults are the daemon's own), its screens and the settings page, and the window builds its pages from the `schema` that comes back, see [settings.md](settings.md#fields); the daemon also ships its own sidecars, so a host gives it no `coreDir`) and **IPC** (`notchd ipc <method>`). The notch's state (modes, peeks, navigation, reserve) is decided by the daemon too; the service binds it, and shows the last state, frozen, while the daemon is away.

It also keeps the daemon alive. When the connection drops, or the script fails, the launcher runs the script again after a short backoff (300 ms, doubling up to 5 s; `restartMin`, `restartMax`) for as long as the shell runs, and the client reconnects on its own. Meanwhile the notch shows a small "notchd not running" row at the top of the screen you work on (`NotchSurface` puts a `DaemonDown` window there, in the look of a module's error row; it shows after `daemonDownDelay`, 1.5 s, so a start does not flash it), and the service keeps its last config and notches. Edits made while the daemon is away wait for it and are sent when it is back; a start that keeps failing logs its reason once.

`scripts/ensure-daemon.sh` finds the binary in this order: `$NOTCHD_BIN`, `notchd` on `PATH`, `$XDG_DATA_HOME/notchd/bin/notchd`; with none, it calls `scripts/fetch-daemon.py --dest "$XDG_DATA_HOME/notchd/bin"` (if present, else the error is "notchd not found"). It then runs `notchd daemon --ensure`, which returns at once when a daemon of the same version answers, and otherwise starts one in its own session with stdio to `$XDG_STATE_HOME/notchd/daemon.log` and waits (at most 10 s) for the socket. A daemon of another version is asked to stop (`notchd ipc shutdown`) and replaced; one that cannot be stopped is an error ("update notchd"). On failure the script prints one line of JSON, `{"error": "..."}`, and exits 1; the launcher logs it once (`state` is `failed`, `error` has the text).

The launcher reads `NOTCHD_INSTANCE` (and the script and daemon `NOTCHD_CONFIG`, `NOTCHD_SOCKET`), which is how `make dev` runs its own `dev` instance on the dev config and never meets the user's daemon.

### Installing the daemon on other distributions

On Ubuntu, Fedora and the like (no Omarchy, no AUR), `scripts/install.sh` installs the pinned release and runs it as a systemd user service:

```sh
sh scripts/install.sh                 # from a checkout; or curl -fsSL <raw url>/scripts/install.sh | sh
sh scripts/install.sh --version 0.4.0 # a pinned version other than the newest
sh scripts/install.sh --prefix ~/opt  # notchd in ~/opt/bin (default ~/.local/bin)
sh scripts/install.sh --uninstall     # removes the binary, the unit and the extension, never ~/.config/notchd
sh scripts/install.sh --extension     # also the GNOME Shell extension (GNOME 46+), from this checkout
sh scripts/install.sh --binary native/target/release/notchd --extension
                                      # a build of your own instead of a release (before the first one)
```

It needs `curl` or `wget`, `tar`, `sha256sum` and `python3` (to read `native/releases.json`; Ubuntu and Fedora ship it). It downloads the archive `native/releases.json` pins for this machine (x86_64 or aarch64), refuses it when its SHA-256 differs from the pinned one (the checksums come from that file, a checkout's or the one on `main`, never from the release), puts `notchd` in `~/.local/bin` atomically (nothing else from the archive), writes `~/.config/systemd/user/notchd.service` (`ExecStart=<prefix>/bin/notchd daemon`, `Restart=on-failure`, wanted by `default.target`), and enables and starts it. Run it again to upgrade: the daemon is stopped (`notchd ipc shutdown`) and started again only when the binary or the unit changed. `--no-service` installs the binary only. `--binary FILE` installs that `notchd` instead of a pinned release (no download, no checksum: a build of your own, which runs on Ubuntu 24.04 as built on a current distribution, since its glibc floor is 24.04's).

`--extension` also installs the GNOME Shell extension (`hosts/gnome/`, GNOME 46 and later): built from the checkout by `scripts/gnome-build.sh` (or taken from `--extension-zip FILE`, which `make gnome-zip` makes), without its test hooks, into `~/.local/share/gnome-shell/extensions/notchd@eryk-vieira.github.io`, replacing the one there, and enabled (`gnome-extensions enable`, else `org.gnome.shell enabled-extensions`); GNOME loads a new extension at the next login. When VTE for GTK 4 is missing it says what to install for terminals in the notch (Ubuntu `gir1.2-vte-3.91`, Fedora `vte291-gtk4`); it installs no system package. With no `~/.config/notchd/config.json` yet it writes a starter one, `{"reserveSpace": true}`, so windows keep clear of the notch; an existing config is never touched.

The unit runs `notchd daemon` itself, not `--ensure`, and is not socket-activated: the daemon creates and removes its own socket (`native/notchd-daemon/src/server.rs`) and exits when the file goes, which a socket unit would own instead. A host that runs `notchd daemon --ensure` (the standalone shell, the GNOME extension) finds the unit's daemon answering on the socket and starts no second one. If a host started a daemon first, the unit's own start fails with "a daemon already listens" and systemd retries every 5 s, harmlessly; an upgrade through the script stops that daemon too. Programs the daemon starts (agents, terminals) get the user manager's `PATH`: put `claude` and `codex` where it sees them (`~/.local/bin` is on it by default on current systemd).

## NotchService

The UI's side of the daemon: it binds the daemon's config, schema and state, reports what only the shell knows (screens, workspaces, the modules it loads itself) and answers the host's IPC target. It decides nothing. It makes a `Notch` per notch the daemon's `state` lists (one per entry of the config's `notches`, one without a list): its modules, position, workspaces. The daemon decides the notch state (modes, hover timers, pins, tabs, peeks and where they go, navigation, the screen each copy is on, the space to reserve) and the copies bind it; they send raw inputs (the pointer, a click, a key, a drop) and the modules' facts. The QML keeps the drag's motion, packing, the outline, animations and windows. While the daemon is away the last state stays (frozen) under the "notchd not running" row. Each notch shows as one or more `NotchCopy`: one that moves (following focus, or kept on its home screen), or one per screen with `position.screen` `"all"`. The service also has a default notch (`visitor`), with every module and a copy at the top of each screen; its copies (`guest`) stay hidden, at rest so they hold no live view, with no window on screen until they open (and for a few seconds after: the daemon's `needsWindow`), and only show a peek on a focused screen where no notch with that module is (`needs_visitor` in `native/notchd-core/src/state.rs`). A copy holds the state the pointer and peeks change (hover, pin, peek, drag) and so its own mode.

| Property | Meaning |
|---|---|
| `ipcTarget` | IPC target name for scripts |
| `logName` | prefix for log lines; defaults to `ipcTarget` |
| `daemonScript`, `daemonClientName` | the ensure script (a file URL) the service runs, and the client name of its `hello` ([The daemon](#the-daemon)); `daemonCommand` replaces what runs (tests), `daemonDownDelay` is how long without a daemon before the notch says so |
| `configPath` | absolute path of the JSON config, as the daemon says it (it reads, watches and writes it; live); defaults to `$XDG_CONFIG_HOME/notchd/config.json`, or `NOTCHD_CONFIG` |
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

With the config's `reserveSpace` on, it also maps one empty window per screen edge a notch keeps clear, namespace `<layerNamespace>-reserve`, holding an exclusive zone as deep as the deepest closed notch on that edge (the daemon's `state.reserve`: notches sharing an edge don't add up). Each copy's depth is measured by its card (`DepthReport.qml`) and the daemon works out the space: the rest lip, or the deepest compact view seen while a module is active; it grows at once and shrinks once the notch has collapsed, 400 ms later. It sits on the overlay layer, so the compositor stacks it inside the bar's own zone; a host bar needs a normal exclusive zone for the strip to land where the notch hangs. Moving the notch to another edge replaces these windows: a mapped layer surface keeps its first anchors. A monitor switched off takes its layer surfaces with it; these windows and the notches' own reopen when it comes back (`KeepShown.qml`).

Give the service and the adapter ids that differ from these property names (`service`, `compositorLoader`): `svc: svc` inside the surface reads as the surface's own property.

## Theme

Set `Theme.provider` to an object mapping the host's theme onto the core's tokens. See [theming.md](theming.md).

## Compositor adapters

`HyprlandCompositor` and `GenericCompositor`, the only place compositor-specific code lives. An adapter tells the notch which screen is focused, whether a fullscreen window covers it and which workspace each screen shows (`supportsWorkspaces`, `knownWorkspaces`, `activeWorkspace(screen)`); the generic one assumes the first screen, never fullscreen, and no workspaces, which hides the `workspaces` setting. `core/GenericCompositor.qml` has the contract. The service reports what an adapter says to the daemon (`screens`), which decides with it; a UI for another OS reports the same facts from its own windowing system.
