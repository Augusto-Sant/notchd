# CLAUDE.md

Rules for working in this repo, for Claude Code and humans. Keep it tight:
invariants that are easy to break, not a project tour. The tour is
[DEVELOPMENT.md](DEVELOPMENT.md).

## While you work

- After every change, `make test` (the unit layer, about a second), plus the layer that covers what you touched: `make integration` for `core/live/` or `core/components/` Python, `make qmltest` (and `make lint`) for QML, `make visual` for how something looks. `QMLTEST_ONLY=<name>` narrows the QML cases. Not `make check` after each edit.
- A new test goes in the lowest layer that can hold it (DEVELOPMENT.md, "The gate"); `make times` shows what each layer and test costs.

## Before you commit

- `make check` passes (every layer: unit, integration, QML, lint, plugin validation, visual scenarios).
- A bug fix comes with a regression test that fails without it: `tests/*.test.mjs` for rules, `tests/qml/shell.qml` for QML behaviour.
- A visual change: run `make visual` and look at the PNGs in `build/visual/`.
- A QML change in `hosts/omarchy/`: `make restart` and read the shell log; lint and the visual scenarios cannot load it.
- `CHANGELOG.md` has a line under `[Unreleased]` for anything user-visible.
- The doc that describes what you changed says the new thing (`grep` for it).
- Commit messages: `type(scope): summary` (see DEVELOPMENT.md).

## Invariants

- **A framework, not a shell.** Modules show what the system is doing; they never take over a system service (notifications, launcher, lock screen). The only exception is the Omarchy `osd` module owning the `osd` IPC target, documented in its README.
- **The core knows no desktop.** Nothing under `core/` imports `qs.*` or reads a desktop's theme, and compositor-specific code lives only in the `*Compositor.qml` adapters. Desktop facts come in through host properties, `Theme.provider` and those adapters.
- **No symlinks, and the repo root is both hosts.** `omarchy plugin validate` rejects any symlink in the plugin folder. `manifest.json` (Omarchy) points at `hosts/omarchy/`, which imports `"../../core"`; `shell.qml` (standalone) stays at the root, because Quickshell only resolves types from files inside its config folder.
- **A missing config key means its default.** Never write defaults into the user's file; `undefined` removes a key, and emptied parent objects go with it (`Config.withValue`). Defaults live once, in `Config.DEFAULTS` or the schema.
- **Rules in plain JS, tested.** Decisions (mode, peek arbitration, config edits, settings pages) live in `core/*.js` / `core/settings/*.js` with a Node test. QML only binds them. Keep those files plain ES5-style functions: QML imports them as is and Node evaluates them in a vm.
- **Theme tokens, not ad-hoc colors.** Use `Theme.muted`, `Theme.line`, `Theme.hover`… and `Theme.space()` / `Theme.font.*`. A hard-coded color or alpha breaks light themes.
- **Settings controls share one contract:** `field` and `value` in, `edited(value)` out. A new field type is a control plus one case in `SettingRow.qml`, documented in `docs/settings.md`.
- **Tests are hermetic.** Nothing in `tests/` or `make` targets touches `~/.config/notchd` or other real user paths.
- **QML cases stand alone.** They run in parallel shards in any order: a case sets up what it needs (`root.withTwoNotches`, `root.activate`, `root.until`), never relying on an earlier one.
- **No `svc: svc`.** Inside an object with a `svc` property, a same-named id reads ambiguously; name ids `service`, `compositorLoader`, etc. qmllint flags it (`id-shadows-member`).

## Gotchas

- A JS array that went through a Repeater `model` reaches the delegate as a sequence: `Array.isArray` is false. Use `Settings.toList()`.
- `Loader.item` is typed `QObject`; cast it (`item as NotchModule`, `item as Item`) before reading members, or qmllint fails.
- The Omarchy shell does not hot-reload plugin QML: `make restart`.
- `qs ipc call` reads an argument starting with `[` as a list; put a space before JSON arrays.
- `pkill -f` on a pattern also found in your own command line kills your shell.
- Offscreen renders (`make visual`, `make hero`) use Qt's software renderer: no shaders, so `ClippingRectangle` and effects draw nothing there. Give shader-only visuals a `GraphicsInfo.api === GraphicsInfo.Software` fallback (see `Art` in `core/modules/media.qml`); real users on software rendering hit the same thing.
- Never capture the live desktop for docs: it shows the user's windows and media. Render offscreen with demo data (`make hero`).
- Offscreen runs go through `scripts/isolated.sh` (private D-Bus, own `XDG_RUNTIME_DIR`, no display). A private bus sharing the real runtime dir once replaced the session's AT-SPI socket and broke accessibility until `systemctl --user restart at-spi-dbus-bus`.
