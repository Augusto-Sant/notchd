# Development

Working on notchd itself. For using it, see the [README](README.md); for the rules that must not break, [CLAUDE.md](CLAUDE.md).

## Prerequisites

| Tool | Package (Arch) | For |
|---|---|---|
| Quickshell | `quickshell` | running the notch, `make dev`, `make visual` |
| Qt 6 `qmllint` | `qt6-declarative` | `make lint` |
| Node.js 18+ | `nodejs` | `make test` (built-in `node:test`, no dependencies) |
| ImageMagick, python-dbus, python-gobject | `imagemagick`, `python-dbus`, `python-gobject` | `make visual`, `make hero` (the demo media player) |
| JetBrainsMono Nerd Font | `ttf-jetbrains-mono-nerd` | `make hero` |
| `rsvg-convert` | `librsvg` | `make icons` |
| Omarchy 4 | | the Omarchy host, `make validate`, `make restart` |
| `qmltermwidget` (optional) | `qmltermwidget` | the `claude` and `codex` modules' terminal (`make deps`); without it their QML tests only check the message that asks for it |
| The voice modules' parts (optional) | none to develop: Settings › Components installs them (table below). Tests use fakes (a fake bridge, a fake LocalAI and `voxtype`, fake `pacman` and a local download server), so `make check` needs none | Voice, Codex Live |

## Optional components

Nothing heavy ships with notchd. Everything external a module needs beyond the app is a component (`core/components/components.json`), installed, removed and shown with its progress by the settings (Components page, and the rows on the Voice and Codex Live pages). `install.py status | install <id> | run <id> | cancel <id> | remove <id>` is the installer; the window runs it, detached for long work, and reads its state from `$XDG_STATE_HOME/notchd/components/<id>.json`. See [docs/settings.md](docs/settings.md#components).

| Component | Kind | Size | Installs |
|---|---|---|---|
| `libpulse` (Audio tools) | system | 2 MB | `libpulse` (pacat, parec) |
| `voxtype` | system (AUR) | 663 MB | `voxtype-bin`, built by yay or paru in a terminal |
| `localai` | files | 197 MB | `bin/local-ai` (the release binary) in `$XDG_DATA_HOME/notchd/localai`; Remove deletes that folder, voices included |
| `voice-piper-<lang>` (`voice-piper-pt-br` for Portuguese) | localai | 63 to 120 MB, plus Piper's backend (`cpu-piper`, 78 MB) once | one Piper voice per language (pt faber, en amy, es davefx, fr upmc, de thorsten, it paola, nl pim; medium), through LocalAI's API |
| `voice-pocket-<lang>` (`voice-pocket-pt` for Portuguese) | localai | 750 MB (English 1.3 GB), plus Pocket TTS's backend (`cpu-pocket-tts`, 1.2 GB) once | the `<language>_24l` weights in the shared cache (`hf/`), through LocalAI's API; every language has alba |
| `codex-live-audio` | system | 8 MB | `python-gobject gst-plugins-{base,good,bad} libnice libpulse` |

The voices of one engine share its backend (and Pocket TTS's weights cache `hf/`): Remove takes only what no other installed voice holds (`forget` in `install.py`), and a size counts a shared path once (`own_size`). A new component is an entry in the registry, a row (`{ type: "component", component: <id> }`) where it belongs, and the bridge naming its id when it is missing (`components` in the bridge's error line). Tests: `tests/components.test.mjs` (rules), `tests/components_test.py` (the installer), `tests/qml/shell.qml` (the rows).

## Layout

```
manifest.json         Omarchy plugin manifest: the repo root is the plugin
shell.qml             standalone host: the repo root is also a Quickshell config
core/                 the framework: no desktop-specific code
  NotchService.qml    state: modules, the notches, peek routing, IPC; a client of the daemon, which owns the config and the settings schema
  DaemonDown*.qml     "notchd not running": the row, and its window while the daemon is away
  Notch.qml           one notch of the config: its modules, place and copies
  NotchCopy.qml       a notch on one screen: hover, pin, peek, drag, mode
  NotchSurface.qml    a window per copy, and the space they reserve
  NotchWindow.qml     the layer-shell overlay that puts one copy on screen
  NotchCard.qml       the notch itself: outline, padding, the current view
  NotchModule.qml     base type for modules
  CoreModule.qml      a module that lives in the daemon (timer, activities, resources, osd, claude, codex, terminals, user modules): draws its views
  Theme.qml           visual tokens (singleton), fed by the host
  Config.js           config rules (pure JS)
  Position.js         edge and offset: where the notch sits, corners and joined rows (pure JS)
  Outline.js          the notch's outline: shoulders, corners, joins (pure JS)
  Terminal.js         the terminal view's widget side: is qmltermwidget there, the color scheme, the key that leaves (pure JS)
  terminal/           TerminalSurface.qml (the frame and the attach) and TerminalView.qml: a terminal (QMLTermWidget, optional package); lint skips them
  Components.js       the optional components: what a row says, its buttons, choices that wait on one, what an error points to (pure JS)
  ComponentStore.qml  the components' state, read from the installer, and the install, cancel and remove calls
  components/         components.json (the registry) and install.py (the installer: downloads, LocalAI models, pacman; stdlib only) and localai.py (where LocalAI lives, how to reach and start it)
  DepthReport.qml     what a notch card measures of itself, sent to the daemon, which works out how much of its edge windows leave to the closed notch
  KeepShown.qml       shows a window again after its monitor goes away and comes back
  *Compositor.qml     compositor adapters (Hyprland, generic)
  modules/            built-in modules: media, claude, codex, the voice ones; and the JSON files of the daemon's (activities, resources, timer)
  settings/           the settings window, its controls and Settings.js
hosts/
  omarchy/            Omarchy shell plugin entry points and the osd module
tests/
  *.test.mjs          Node tests of the pure-JS rules
  load.mjs            loads a core .js file the way QML sees it
  qml/shell.qml       QML behaviour tests, regression tests among them
  qml/fixtures/       modules only the QML tests load (a module you type into), and fake `claude` and `codex`
                      terminal UIs that set the terminal's title as the real ones do
  visual/shell.qml    offscreen scenarios: the settings window, the notch per edge
  visual/hero.qml     the README image: the real notch in four states
  visual/demo-player.py  a fake MPRIS player with a fictional track
  visual/bin/         the README image's `claude` and `codex`: a finished conversation, printed
docs/                 reference: modules, hosts, settings, theming, IPC
docs/images/          hero.webp, generated by `make hero` and committed
assets/icons/         the app icon: hicolor/scalable/apps/notchd.svg is the source; the PNGs at every
                      hicolor size and notchd.ico are generated by `make icons` and committed;
                      hicolor/symbolic/apps/notchd-symbolic.svg is the one-color mark on the Omarchy bar button
scripts/              qml-test.sh, visual-test.sh, hero.sh, icons.sh, isolated.sh (runs them cut off from the session),
                      with-demo-player.sh (isolated.sh plus the demo media player)
                      pytests.py (the Python tests of one layer, in parallel), timed.py and test-times.py
                      (what each layer cost: `make times`), node-times.mjs (the Node tests' times)
                      bench.sh (`make bench`: what the notch costs, on your display)
build/                generated (visual PNGs, dev config, test times); ignored
```

## Running it

### Omarchy host

The plugin folder is a symlink to the checkout:

```sh
ln -sfn "$PWD" ~/.config/omarchy/plugins/eryk.notchd
make restart                 # make deps, then omarchy restart shell
```

The shell does not hot-reload plugin QML: run `make restart` after changing it. It runs `make deps` first, which installs the optional packages the modules use (`qmltermwidget`, for the `codex` module) with pacman when they are missing; sudo asks for your password only then. Config changes apply live. How the plugin gets enabled, and the stock OSD it replaces, are in [hosts/omarchy](hosts/omarchy/README.md).

The shell's log:

```sh
qs log --pid "$(pgrep -xf 'quickshell -n -p /usr/share/omarchy/shell')" | grep -i notchd
```

### Standalone host

```sh
make dev                     # config in build/dev-config.json, never ~/.config
qs -p . ipc call notchd settings
```

`make dev` hot-reloads on every QML change, core included. Next to the Omarchy host it draws a second notch, on IPC target `notchd`.

## The gate

The tests are a pyramid: many cheap ones at the bottom, few costly ones on top, and the costly ones run less often. While you work, run `make test` after every change (about a second) and the layer that covers what you touched; run `make check`, every layer, before every commit.

| Layer | Target | What it does | Run it |
|---|---|---|---|
| unit | `make test` | Node tests of `core/*.js` and `core/settings/*.js`; the Python rules of the live bridges and the components' installer as functions (`tests/*_test.py`, every TestCase without `tier = "integration"`). No processes, no servers | after every change |
| integration | `make integration` | `tests/codex_live_test.py`'s echo canceller on real GStreamer; `tests/local_live_test.py`, the whole local voice bridge with a fake LocalAI, voxtype, pacat, codex and claude; `tests/components_test.py`, the components' installer with fake `pacman`, `pkexec`, `yay`, a fake LocalAI and downloads from a server on 127.0.0.1; `tests/fetch_daemon_test.py`, `scripts/fetch-daemon.py` against a local release server. On parallel workers, each in its own process group, killed with what it left behind (every process is also under `timeout -k`) | after a change to `core/live/` or `core/components/` |
| qml | `make qmltest` | the QML behaviour tests, in parallel shards (below) | after a QML change |
| visual | `make visual` | renders the settings window and the notch on each edge (the real modules, the media one playing the demo player's track) offscreen into `build/visual/`; fails on any QML warning, so it also proves the core loads | after a visual change, and look at the PNGs |
| static | `make lint` | `qmllint` over `shell.qml` and the core; fails on any warning. Also checks that the Python bridges and installer parse | after a QML change |
| static | `make validate` | `omarchy plugin validate .` | before a commit |

`make check` runs lint, validate and the unit layer first, one after another, so a broken rule fails in seconds; then integration, qml and visual at once.

**What it costs.** Every layer records its wall-clock and CPU time, and each test's seconds, in `build/times/`. `make times` prints the pyramid from the last run of each layer: tests, wall and CPU per layer, the slowest tests, and those over their layer's budget (0.1 s for a unit test: one that takes longer is not a unit test, make it cheaper or move it up; 5 s for an integration test or a QML case). The same report lands in `build/test-times.md`. Check it after adding costly tests.

**Where a test goes.** As low as it can: a rule over a whole range (every language, every order) is a unit test of the function; the layer above proves the wiring once, with one value from that range. A Python TestCase that starts a process or a server says `tier = "integration"` (on a base class it covers the subclasses); `scripts/pytests.py unit|integration [file ...]` runs one layer, and fails if a layer has no tests.

The Omarchy host's QML imports `qs.Commons`/`qs.Ui`, which only exist inside the Omarchy shell, so lint skips it; after changing it, `make restart` and read the log.

## Tests

**Rules go in plain JS, views in QML.** Anything that decides something (which peek wins, how a key is edited, which pages show) lives in a `.js` file with no QML globals, imported from QML with `import "X.js" as X`, and has a test in `tests/`. QML files bind those results to the screen. Keep the `.js` files plain functions and `var`s: QML imports them as is and `tests/load.mjs` evaluates them in a Node vm.

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, plain } from './load.mjs';

const Config = load('core/Config.js');

test('valueAt follows dotted keys', () => {
  assert.equal(Config.valueAt({ theme: { accent: '#fff' } }, 'theme.accent'), '#fff');
});
```

Objects and arrays built inside the vm have their own prototypes; compare them through `plain()`.

**Tests are hermetic.** They never read or write `~/.config/notchd` or any real user path: the visual scenarios use a scratch dir, `make dev` uses `build/`. A test that needs a path gets it passed in. Offscreen renders also leave the desktop session alone: `scripts/isolated.sh` gives them a private D-Bus and runtime dir.

**For a bug fix, write the test first** and see it fail. Every bug found by hand gets one, even when the fix is a line: a Node test when the bug is in a rule, a QML behaviour test when it needs the QML runtime.

### QML behaviour tests

`tests/qml/shell.qml`, run by `make qmltest` (part of `make check`): cases that need real bindings, signals or animations, such as a list that stops being an `Array` after a `var` signal, or a height measured halfway through a transition. Each case is a function in the `cases` list that checks with `root.check(cond, why)` and ends with `root.done()`; `root.steps([[ms, fn], ...], last)` waits between steps, and `root.until(pred, fn)` waits for something to load (prefer it to a fixed wait: it costs only what it takes).

**Every service has a daemon.** The daemon owns the config, the settings schema and IPC, so a `NotchService` without one shows "notchd not running" and has no config. The tests create `TestService` (`tests/qml/TestService.qml`, which the scripts copy next to the `shell.qml` they run) instead: a `NotchService` that runs the real `notchd` (`make daemon`, `NOTCHD_TEST_DAEMON`) through the launcher's own path, on a socket in `NOTCHD_TEST_SOCKETS` and with its `configPath` as the daemon's config file. The scripts kill the daemons that are left (every one whose socket is in the scratch dir) when the run ends, and the cases start once each has answered (`daemonReady`). Edits reach the service at once but the daemon's schema and file a moment later: a case that reads either waits for it with `root.until`. `tests/qml/scenarios.qml` has the service start its daemon too. `NOTCHD_TEST_HOOKS=1` (set for the shell shards) gives the daemon `test-linger <ms>`, the guest windows' linger.

**Every case stands alone.** `scripts/qml-test.sh` runs the cases in parallel shards (`QMLTEST_JOBS`, default the CPUs up to 8), each its own Quickshell, longest first by the last run's times, so a case can run first, alone, or after any other. It sets up what it needs: `root.withTwoNotches(fn)` for the multi-notch service, `root.activate(keys)` for the window its keys go to. `QMLTEST_ONLY=<part of a name> make qmltest` runs just those cases; `QMLTEST_VERBOSE=1` lists the passes as well as the failures; `QMLTEST_KEEP=1` keeps the scratch dir (each shard's `log`, its configs and sockets). They run offscreen through `scripts/isolated.sh`, with the service's config in a scratch dir. For hover, clicks and drags, the `pointer` TestCase (QtTest, running no tests of its own) sends real pointer events to a card in a window: `pointer.mouseMove(item, x, y)`, `mousePress`, `mouseRelease`, `mouseClick`. Keys go to a TestCase's own window, so the `keys` TestCase sits inside the window of the card it types into: `keys.keyClick(Qt.Key_Escape)`. A test module the real hosts don't ship goes in `tests/qml/fixtures/`, loaded through a test service's `hostModules`. A case that guards a bug says which one in a comment.

**The scenarios** (`tests/qml/scenarios.qml`, one more shard of `make qmltest`, which builds the daemon first) run the real daemon and the QML service together: each scenario is a config, screens, module facts and a list of inputs (the pointer, a click, tabs, navigation keys, a drop, a peek), and after each `w.snap(label, expected)` the QML must show what `expected` says (a flat map of short texts: a notch's place, each copy's mode, pin, peek, tab, shown modules, `covered`, keyboard; the visitor; navigation). `NOTCHD_QMLTEST_RECORD=1` prints what each snapshot shows, for writing a new one: read it against the label before it goes in. `shell.qml` cases are written as flows (`root.go(function*() { ... yield root.settle(svc) ... })`): the daemon decides, so what a case does shows a round trip later, and `settle` waits for exactly that (the daemon answers a request after the state it made).

### Visual scenarios

`tests/visual/shell.qml` lists the scenarios: a page (`scrollTo`: `"end"` or pixels), a theme (dark or light), the host's fields; or an `edge`, which shows each module's view on that edge; or `down`, the row the notch shows while the daemon is away. Its services are `TestService`s, each with a daemon of its own (`make visual` builds the daemon, and so does `make hero`). Add one when a page, a control or a module's view changes, run `make visual` and look at the PNGs in `build/visual/`. They are not committed; a visual change gets before/after images in its PR or commit discussion.

### README image

`make hero` renders `docs/images/hero.webp` from `tests/visual/hero.qml`: the real `NotchCard` in four states, each with its own `NotchService` and demo data, offscreen. It runs on a private D-Bus where the only media player is `tests/visual/demo-player.py`, so whatever you are playing never ends up in the image, and with `tests/visual/bin` first on PATH, so the agents are demo scripts that print a finished conversation, never your own `claude` or `codex`, and their usage is set by hand (offscreen nothing asks the network). The tiles use Omarchy's Gruvbox colors and its "The Backwater" wallpaper (`WALLPAPER=<image> make hero` for another; a gradient without one). The wallpaper stays out of the repo; only the rendered image is committed. Rerun it after a visual change to the notch and commit the result.

### Local voice

`core/live/local-live.py` speaks the same stdout protocol as `codex-live.py`, so `LiveModule` runs it unchanged (`localVoice: true` in `modules/local-live.qml` picks the bridge and `Live.localBridgeArgs`). The QML tests use `tests/qml/fixtures/bin/local-live` (`NOTCHD_LOCAL_LIVE_BRIDGE`); `tests/local_live_test.py` tests the voice activity detector, sentence splitter, markdown stripper, the agents' command lines and streams as functions, and runs the whole bridge on a fake LocalAI (an HTTP server on 127.0.0.1 in the test: `/readyz`, `/v1/models`, `/v1/audio/speech` returning 16-bit or float WAVs) and fake `voxtype`, `pacat`, `codex` and `claude` with `--mic <wav>` (`NOTCHD_LIVE_MIC_SPEED=10` plays it faster), including that stop, end of stdin and SIGTERM leave no child behind, and that a LocalAI the bridge started (`NOTCHD_LOCALAI_BIN`, a fake program) stops with it while one that was already running is left alone. The real LocalAI, by hand and never in `make check`: install the components from the settings (or `NOTCHD_LOCALAI_URL=http://127.0.0.1:18081 XDG_DATA_HOME=<scratch> python3 core/components/install.py install voice-piper-pt-br`, which starts and stops a LocalAI of its own), then `local-live.py` with a fake `pacat` on PATH. By hand: `python3 core/live/local-live.py --agent codex --session new --cwd <scratch folder>` (never `last` in a folder with sessions you care about: it writes into them), speak, Ctrl+C; `NOTCHD_LIVE_DEBUG=1` copies the log. Bound every run with `timeout -k` and check `ps` for `parec`, `pacat`, `voxtype`, `local-ai` after.

### Codex Live

The QML tests and visual scenarios never run the real installer either: `NOTCHD_COMPONENTS_INSTALLER` points at `tests/qml/fixtures/bin/components`, a fake that reports a canned state and logs the `install`, `cancel` and `remove` it is asked for. They never run the real bridge: `NOTCHD_LIVE_BRIDGE` points at `tests/qml/fixtures/bin/codex-live`, which prints the same lines (live, a question, an answer, a voice level) and waits for `{"stop": true}`; a `--codex` with `missing` in it fails as the real one does without its packages. `tests/codex_live_test.py` covers what the bridge makes of the server's messages, and builds the echo canceller's branch where GStreamer is.

By hand, the bridge runs on its own and prints its lines: `python3 core/live/codex-live.py`, talk, Ctrl+C. `--mic <file.wav>` puts a recording in the microphone's place (begin it with ~12 s of silence, while it connects); `NOTCHD_LIVE_DEBUG=1` copies the log to stderr; `NOTCHD_LIVE_RECORD=<file.wav>` writes Codex's voice as it is decoded. To hear nothing while measuring what it plays, give it a sink of its own: `pactl load-module module-null-sink sink_name=notchd_test`, `PULSE_SINK=notchd_test`, record `notchd_test.monitor` with `parec`, and `pactl unload-module` it after. Bound every such run with `timeout -k`, and check nothing is left playing.

### App icon

The icon is drawn by hand in `assets/icons/hicolor/scalable/apps/notchd.svg`: the notch, with its shoulders, hanging from the top of a Gruvbox tile, holding an orange dot and an `n`. `make icons` renders it into `assets/icons/hicolor/<size>x<size>/apps/notchd.png` for every hicolor size from 16 to 1024 and into a multi-size `assets/icons/notchd.ico`. The tree is laid out like `/usr/share/icons/hicolor`, so a package can copy it as is. Edit the SVG, rerun `make icons` and commit the PNGs with it. `assets/icons/hicolor/symbolic/apps/notchd-symbolic.svg` is the same drawing in one color (a white tile with the notch knocked out), drawn by hand too: the Omarchy bar button tints it with the theme. Change it along with the SVG.

### Resource use

`make bench` measures what the notch costs: the standalone host on your real Wayland session, in a few scenarios (idle, a timer running, music playing; each closed and open), next to an empty Quickshell as the floor. Per scenario it samples CPU (% of one core), context switches per second (frames and timer wakeups show up here) and PSS/RSS from `/proc`, and writes the table to `build/bench.md` (`BENCH_SECONDS=20` for longer samples). A second notch shows on screen while it runs; keep the pointer away from it.

It can't run offscreen like the tests (`PanelWindow` needs a compositor, and rendering real frames is the cost that matters), so it runs on your display but on a private D-Bus that activates nothing, with the demo player as the only MPRIS player and a scratch config. It is not part of `make check`: the numbers depend on the machine and on what else runs. Run it before and after a change that could cost something, anything that animates, polls or adds a window, and put both tables in the commit discussion.

What keeps the cost down: anything that changes on screen makes its whole window render a frame, and every notch window covers its screen. A view that animates without pause (a `Behavior` that is always moving, a timer of a few hundred ms) renders at the monitor's refresh rate; drive it from one 60 fps `Timer` instead, never below 60 fps for something people watch. A copy that is hidden must not keep a live view. An `Image` without `mipmap: true` costs a texture atlas the size of the screen.

## Conventions

### Commits

[Conventional Commits](https://www.conventionalcommits.org/): `type(scope): summary`, imperative, lowercase, no period.

- Types: `feat`, `fix`, `refactor`, `docs`, `test`, `build` (Makefile, scripts, manifest), `chore`.
- Scopes: `core`, `settings`, `theme`, `omarchy`, `standalone`, `modules/<id>`, `ipc`. Omit it when the change spans several.
- The body says why, not what.

```
feat(settings): show compact style as picture cards
fix(omarchy): keep the bar indicator on the hovered monitor
```

### Changelog

Every user-visible change adds a line to `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md), under Added, Changed, Fixed or Removed. Released sections are never edited.

### Versions

`manifest.json`'s `version` is the version, and `native/notchd-daemon/Cargo.toml`'s `version` is the same number (a unit test in `tests/fetch_daemon_test.py` fails when they differ): the plugin wants the daemon of its own version.

### Releasing

The plugin pins the daemon it downloads: `native/releases.json` (`{ "<version>": { "<target>": { "url", "sha256" } } }`) is the only source of checksums, changed only by a reviewed merge to main, so a tampered release asset fails the check. `scripts/fetch-daemon.py` reads it (see [docs/architecture.md](docs/architecture.md#distribution)).

1. `make check`.
2. Bump `version` in `manifest.json` and in `native/notchd-daemon/Cargo.toml` (then `cargo build` to refresh `native/Cargo.lock`). Rename `[Unreleased]` in `CHANGELOG.md` to `[X.Y.Z] — YYYY-MM-DD` and open a new empty one.
3. Commit as `vX.Y.Z`, tag it `git tag -a vX.Y.Z -m "vX.Y.Z"`, push both. Tags are never moved.
4. `.github/workflows/release.yml` builds `notchd` on an x86_64 and an aarch64 runner, uploads `notchd-X.Y.Z-<target>.tar.gz` and its `.sha256` to the GitHub release, and opens a PR on `release-pin-vX.Y.Z` adding the version to `native/releases.json`. Compare the hashes with the release's `.sha256` files and merge it. Until then the plugin on main answers "no release for this version". (Needs "Allow GitHub Actions to create pull requests" in the repo's Actions settings.)

`.github/workflows/ci.yml` runs `make rust`, `make test` and `make integration` on every push to main and every pull request.

### Documentation

A change updates the doc that describes it, in the same commit: `README.md` for users, `docs/` for reference, this file for workflow, `CLAUDE.md` for a new invariant. `grep` for the name you changed.
