# CLAUDE.md

Rules for working in this repo, for Claude Code and humans. Keep it tight:
invariants that are easy to break, not a project tour. The tour is
[DEVELOPMENT.md](DEVELOPMENT.md); the contract between the daemon and the UIs
is [docs/architecture.md](docs/architecture.md).

## While you work

- After every change, `make test` (the unit layer: Node, Python rules and `cargo test`, seconds), plus the layer that covers what you touched: `make rust` (fmt, clippy `-D warnings`, tests) for `native/`, `make integration` for `core/live/`, `core/components/` or `scripts/fetch-daemon.py`, `make qmltest` (and `make lint`) for QML, `make visual` for how something looks. `QMLTEST_ONLY=<name>` narrows the QML cases. Not `make check` after each edit.
- A new test goes in the lowest layer that can hold it (DEVELOPMENT.md, "The gate"); `make times` shows what each layer and test costs.

## Before you commit

- `make check` passes (every layer: lint, plugin validation, unit, Rust, integration, QML, visual scenarios).
- A bug fix comes with a regression test that fails without it: a Rust test (or a `tests/vectors/rules/` case) for a rule, `tests/qml/shell.qml` or `scenarios.qml` for QML behaviour, `tests/*.test.mjs` for the UI-side JS.
- A visual change: run `make visual` and look at the PNGs in `build/visual/`.
- A QML change in `hosts/omarchy/`: `make restart` and read the shell log; lint and the visual scenarios cannot load it.
- `CHANGELOG.md` has a line under `[Unreleased]` for anything user-visible.
- The doc that describes what you changed says the new thing (`grep` for it).
- Commit messages: `type(scope): summary` (see DEVELOPMENT.md).

## Invariants

- **A framework, not a shell.** Modules show what the system is doing; they never take over a system service (notifications, launcher, lock screen). The only exception is the Omarchy `osd` module owning the `osd` IPC target, documented in its README.
- **The daemon is the authority.** Config, the settings schema, notch state (modes, hover timers, peeks, navigation, reserve) and every module's logic are decided in the Rust core (`native/`). A UI sends raw inputs (a pointer fact, a key, a drop, a measured depth, a button's action) and draws what comes back; it never decides, and never keeps a rule "just for now" in QML.
- **Decisions in Rust, tested; QML binds and draws.** A rule is a function in `native/notchd-core` (or a module in `notchd-modules`) with a Rust test; shared cases with the old JS live in `tests/vectors/rules/`. The plain-JS files left in `core/` are UI-side only: `Position.js` and `Outline.js` (pixel geometry, golden vectors in `tests/vectors/geometry/`), `Config.js` (the optimistic copy of an edit, the notch list), `settings/Settings.js` (control mechanics and assembling the window's pages), `Terminal.js` (the key that leaves a terminal), `client/Protocol.js` and `client/prims/Tree.js` (the wire and the view tree). Keep them ES5-style functions: QML imports them as is and Node evaluates them in a vm.
- **The core knows no desktop.** Nothing under `core/` imports `qs.*` or reads a desktop's theme, nothing under `native/` imports a UI toolkit or reads a desktop's theme or compositor, and compositor-specific code lives only in the `*Compositor.qml` adapters (the UI reports screens, focus and workspaces). OS differences sit behind provider traits in `notchd-providers` (`cfg(target_os)`). Desktop facts come in through host properties, `Theme.provider` and those adapters.
- **Protocol changes go through `native/notchd-proto`.** A new or changed message is a type there, a fixture in `tests/protocol/` (one file per message, canonical form) and a line in `docs/architecture.md`, in the same commit. `proto` is a major integer; additions are optional fields.
- **Module views are primitives.** A module (built-in or a user's process) draws only with the primitives of `docs/architecture.md`; colors are `tone`s, sizes are design pixels. When a view needs something the primitives lack, growing a light primitive (all UIs implement it) is the project's job; a `webview` or a `canvas` there signals a missing primitive, not a way around it.
- **No symlinks, and the repo root is both hosts.** `omarchy plugin validate` rejects any symlink in the plugin folder. `manifest.json` (Omarchy) points at `hosts/omarchy/`, which imports `"../../core"`; `shell.qml` (standalone) stays at the root, because Quickshell only resolves types from files inside its config folder. Nothing builds Rust at install: the plugin downloads the daemon release `native/releases.json` pins (checksums only change by a reviewed merge).
- **A missing config key means its default.** Never write defaults into the user's file; `null` removes a key and emptied parent objects go with it (`with_value` in `native/notchd-core/src/config.rs`, the only writer). Defaults live once, in `config::DEFAULTS` or the schema. The QML's `Config.withValue` is only the optimistic copy the daemon's answer replaces.
- **The QML IPC handler cannot wait for the daemon.** Quickshell's `IpcHandler` returns a string at once, so it answers what the shell knows (`no notch <id>`, `<id> module not loaded`, `ok` once the command is sent) and sends the rest to the daemon. The module's own answers (`no timer running`, usage text, plan usage in `state`) are `notchd ipc <method>`'s. Do not make the handler guess a reply.
- **The daemon outlives the shell.** Terminals, agents, voice and timers belong to it, so `make restart` replaces it only when the build changed (`cmp`); an unchanged daemon keeps its sessions. A daemon exits when its socket file goes.
- **Detached daemons in tests are cleaned by socket path.** A test that starts a daemon (or a QML shard with a `TestService`) puts its socket in a scratch dir and kills every process with `--socket <scratch>/` when it ends (`pkill -f`, as `scripts/qml-test.sh` does), since a detached daemon outlives its test (once, 1807 of them). Rust tests give every daemon a temp dir and shut it down.
- **Theme tokens, not ad-hoc colors.** Use `Theme.muted`, `Theme.line`, `Theme.hover`… and `Theme.space()` / `Theme.font.*`. A hard-coded color or alpha breaks light themes.
- **Settings controls share one contract:** `field` and `value` in, `edited(value)` out. A new field type is a control plus one case in `SettingRow.qml` (and the type in `notchd-core`'s schema), documented in `docs/settings.md`.
- **Tests are hermetic.** Nothing in `tests/`, `native/` tests or `make` targets touches `~/.config/notchd` or other real user paths: temp `XDG_*`, `NOTCHD_CONFIG`, random sockets under a temp dir.
- **QML cases stand alone.** They run in parallel shards in any order: a case sets up what it needs (`root.withTwoNotches`, `root.activate`, `root.until`), never relying on an earlier one.
- **No `svc: svc`.** Inside an object with a `svc` property, a same-named id reads ambiguously; name ids `service`, `compositorLoader`, etc. qmllint flags it (`id-shadows-member`).

## Gotchas

- A JS array that went through a Repeater `model` reaches the delegate as a sequence: `Array.isArray` is false. Use `Settings.toList()`. The same goes for lists in a daemon message: take plain copies.
- `Loader.item` is typed `QObject`; cast it (`item as NotchModule`, `item as Item`) before reading members, or qmllint fails.
- The Omarchy shell does not hot-reload plugin QML: `make restart`. A new daemon build also needs it (it stops the old daemon).
- `qs ipc call` reads an argument starting with `[` as a list; put a space before JSON arrays.
- `pkill -f` on a pattern also found in your own command line kills your shell.
- The Rust toolchain is pinned in `native/rust-toolchain.toml`; `cargo` run from `native/` picks it up, `make` passes `--manifest-path`.
- Offscreen renders (`make visual`, `make hero`) use Qt's software renderer: no shaders, so `ClippingRectangle` and effects draw nothing there. Give shader-only visuals a `GraphicsInfo.api === GraphicsInfo.Software` fallback (see `core/client/prims/ImagePrim.qml`); real users on software rendering hit the same thing.
- Never capture the live desktop for docs: it shows the user's windows and media. Render offscreen with demo data (`make hero`).
- Offscreen runs go through `scripts/isolated.sh` (private D-Bus, own `XDG_RUNTIME_DIR`, no display). A private bus sharing the real runtime dir once replaced the session's AT-SPI socket and broke accessibility until `systemctl --user restart at-spi-dbus-bus`.
