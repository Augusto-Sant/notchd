# Writing a module

A module is either a QML file whose root is a `NotchModule`, or logic that lives in the daemon and is drawn from view trees ([below](#a-module-in-the-daemon)). Built-ins live in `core/modules/` and import the core with `import ".."`. A host's modules live in its own folder (the Omarchy host's in `hosts/omarchy/modules/`, importing `"../../../core"`) and register through the service's `hostModules`.

```qml
import QtQuick
import ".."

NotchModule {
  id: m
  title: "Example"
  priority: 60          // higher wins the compact slot / outranks lower peeks
  active: true          // show in compact/expanded while true
  // urgentPeek: true   // peek even while expanded by hover
  // peekOnFocusedScreen: true  // peek once, on the focused screen only (feedback on what you just did there)
  // expandedWidth: 520         // its expanded view's width, unscaled px (default 400)
  // icon: String.fromCodePoint(0xF051B)  // shown closed while active with no compact view
  // attention: true           // news you haven't seen: a dot on its tab, its peek as the notch closes

  compact: Component { Text { text: "hi"; color: Theme.foreground } }
  verticalCompact: Component { Text { text: "h\ni"; color: Theme.foreground } }  // optional, for a side edge
  verticalPeek: Component { Text { text: "h\ne\ny"; color: Theme.foreground } }  // optional, for a side edge
  peek: Component { Text { text: "hello!"; color: Theme.foreground } }
  expanded: Component { Text { text: "details"; color: Theme.foreground } }

  // m.requestPeek(3000) pops the notch open with `peek` for 3 s.
  // m.openNotch() pins the first notch that shows it open, on its tab; m.closeNotch() closes them.
}
```

A module that is `active` but has no `compact` view right now leaves the closed notch at rest and still shows when expanded: the `resources` module, set to `"short"`, binds `compact` to null until a resource runs short.

The expanded view is as tall as it lays itself out; it needn't fit the screen. Past the room the notch has, the open notch scrolls its modules (the wheel, where your view doesn't take it, and a scrollbar), and in a notch with several modules (its `tabs`, on by default) it shows one module at a time, its `title` on the tab. `openNotch()` picks the module's own tab. Closed, such a notch shows every active module's `compact` view side by side, so keep it short.

Give the module an `icon` (a Nerd Font glyph, e.g. `String.fromCodePoint(0xF051B)`). While it is active with no `compact` view (the timer, stopped), the closed notch shows the icon in its place, unless the notch's `idleIcons` is off. Without an icon it stays out of the closed notch then.

A peek asked for while the notch is open (on another tab, say) doesn't show. For news that matters, set `attention: true` before `requestPeek()`: its tab gets a dot, and the peek shows once as the notch closes, unless you have seen it by then. Clear `attention` once you have (the terminal modules clear it when their view opens). The rule is `State.attentionPeek`.

## A module in the daemon

The timer, activities, resources, media and OSD, and every user module (a folder of `modules/` in the config dir, run by the daemon as a process), keep their logic in the daemon and send **view trees** (the primitives of [architecture.md](architecture.md#primitive-tree-v1)), not QML. `core/CoreModule.qml` is the one QML type that shows them, so to the notch (tabs, compact row, peeks, the settings page) they are modules like any other:

- Its facts (`title`, `priority`, `active`, `icon`, `attention`, `urgentPeek`, `peekOnFocusedScreen`, `expandedWidth`) are the daemon's `module` message, bound as properties (`daemonOwned` is true, so the service never reports facts for it).
- Each slot (`compact`, `verticalCompact`, `peek`, `verticalPeek`, `expanded`) is a stable `Component` that draws `client.view(id, slot)` with `ViewTree`, or `null` while the daemon sends no tree for it. A press in a view goes back as the client's `event(module, slot, action, value)`.
- `NotchService` makes one for every module the daemon announces that has no QML file (`loadUrl`) and that some notch lists (or that is standalone and on); a built-in in the daemon still has its JSON file next to where its QML was (`coreModules`), which gives its settings page and defaults. A host module that only owns an IPC target (the Omarchy `osd`) is a `CoreModule` with the `IpcHandler` inside.

To move a module to the daemon: write `native/notchd-modules/src/<id>.rs` implementing `CoreModule` (`facts`, `view(slot, now)`, and as needed `config`, `command` for IPC, `event`, `tick`/`next_due`, `feed` for a provider's data, `take_effects` for peeks and notifications), add it to `IDS` and `create` in `lib.rs` and, if it has IPC methods, to `METHODS` in `notchd-daemon/src/corehost.rs`; move the id from `builtinModules` to `coreModules` in `NotchService.qml` and delete the QML. Settings defaults are read from its JSON file (`tree::Settings`). Test it as values in Rust (every slot builds a valid tree), and the QML behaviour with the helpers `root.ask`, `root.texts` of `tests/qml/shell.qml`. A module that needs demo data offscreen implements `demo` (the test hook `test-<id>`, only with `NOTCHD_TEST_HOOKS=1`).

A tree draws like the QML it replaced where a few general properties say so: `width`, `height`, `minWidth` (and a row's or column's `maxWidth`) fix a size, `fill` takes the rest of a row, and glyphs that need a size are `text` nodes (`size: "body"`), which draw the same Nerd Font.

## Settings

A module's title, settings fields and whether it is standalone or an [agent](#agents) live in a JSON file next to its QML, with the same name (`resources.json` for `resources.qml`, the same for a host's modules). The service reads it whether the module loads or not, so the module has its page in the settings window (Modules, then its gear) even while no notch shows it. `settings` uses the settings schema's format (see [settings.md](settings.md)) with `key` relative to the module; values are stored under the module's id, and the module reads them with `setting(key)`: the config's value, else the field's default. A field that names another `page` shows there, under the module's title. A `title` set in the QML wins over the file's; without either, the title is the id. `tests/settings.test.mjs` checks every module file.

```json
{
  "title": "Resources",
  "settings": [
    { "key": "compact", "type": "choice", "label": "Closed notch", "default": "always",
      "options": [{ "value": "always", "label": "Always" }, { "value": "short", "label": "When running short" }] }
  ]
}
```

```qml
readonly property bool alwaysCompact: m.setting("compact") !== "short"   // config: "resources": { "compact": "short" }
```

A module's settings apply to every notch, since it loads once. Pick an id that isn't a config key (`modules`, `position`, `theme`...).

## Standalone modules

A module with a fixed job and a place of its own, the way an OSD always shows volume and brightness in the same spot, sets `"standalone": true` in its JSON file (the Omarchy `osd` does). Then:

- No notch shows it, even one whose `modules` list it; the settings window leaves it out of a notch's module list.
- It loads on its own, unless the config switches it off: `"<id>": { "enabled": false }`.
- The service gives it a notch of its own (`NotchService.standaloneNotch(id)`), a copy on every screen, hidden until the module peeks; the peek shows on the focused screen. `"<id>": { "position": { "edge", "offset" } }` places it, top center by default; there is no `screen`, since it follows focus.
- It draws over the notches it overlaps there, which hide while it shows (`Position.coveredByStandalone`), and stays out of the side-by-side packing.
- Its page in the settings window starts with an Enabled switch and its position (`Settings.standaloneFields`), then its own fields. `enabled` and `position` are taken: don't name a field of its own after them.

Give it a `peek` view (and a `verticalPeek` for a side edge); it has no closed or expanded state of its own to show.

## Typing

The notch's window takes the keyboard while it is pinned open, whatever it shows: a click on the notch, `toggleNotch <id>` from a shortcut, navigation, or a module calling `requestKeyboard()`, which pins every notch open on it. A module with a text field calls it when the field is pressed, since a notch opened by hover has no keyboard. Hover alone never takes the keyboard from the window you are typing in. The window asks for it exclusively, the only way a layer gets it without a click, and on demand once it has it, so a focus or workspace shortcut can still move to a window; the notch then closes, as a click away does. On a monitor without focus it stays exclusive, since Hyprland gives an on-demand layer's keyboard there straight back to the focused monitor; Escape closes it. Escape and Ctrl+Escape close the notch: a field leaves Escape alone and it goes up to the card; a terminal keeps Escape and lets Ctrl+Escape out. The rules are `State.keyboardFocus` and `State.keyboardChange`.

```qml
expanded: Component {
  TextInput {
    focus: true
    TapHandler { onTapped: m.requestKeyboard() }
  }
}
```

## Agents

A coding agent (`claude`, `codex`) is a module that runs the agent's own terminal UI in the expanded view: a `TerminalModule` (a `NotchModule`), with `"agent"` in its JSON file. Notches show it like any module; the Modules page lists agents apart, under Agents. Adding one takes three things:

- `modules/<id>.json`: its title and `"agent": { "command": "<its command>" }`. That brings the fields every agent has (`Settings.agentSettings`): `directory`, `command` (the command is its default), `idleMinutes`, `width`, `height`, `font`. `"usage": "<source>"` there (`claude`, `codex`: `Usage.SOURCES`) adds its plan usage, below, and the fields `usage` and `usageWarn`. `agent.help` adds a sentence to a field's help, by key, for what only that agent needs said; a field of its own in `settings` with the same key replaces the inherited one.
- `modules/<id>.qml`: a `TerminalModule` with `appName`, `parseTitle` (the terminal's title to `{ working, title }`) and, for an image mark, `logoSource`.
- Its title parser in `core/Agent.js` (`claudeTitle`, `codexTitle`), with a test in `tests/agent.test.mjs`: the closed notch's title and working dot come from the title the agent gives the terminal.

```json
{
  "title": "Codex",
  "agent": {
    "command": "codex",
    "help": { "idleMinutes": "Codex takes 500 to 700 MB, besides its shared background server." }
  }
}
```

```qml
import ".."
import "../Agent.js" as Agent

TerminalModule {
  appName: "Codex"
  parseTitle: Agent.codexTitle
}
```

A terminal of your own needs no file: each entry of the config's `terminals` (`"<id>": { "command", "title" }`, `Config.terminals`) is a module that loads `core/terminal/CustomTerminal.qml`, a `TerminalModule` that runs the command as a shell script, starts in your home without a `directory` and shows the terminal's title as it is (`Terminal.plainTitle`). Its info is made from the entry (`Settings.terminalInfo`): its title, the entry's own `title` and `command` as fields with `absolute` keys (`terminals.<id>.command`, `required`), then the fields every agent has but `command`, under its id. So the id is a config key: one the config already has (`Config.RESERVED_KEYS`, a host's settings keys) or a module's is skipped with a warning in the log, as is an id with anything but letters, digits, `-` and `_`. The Modules page lists them under Terminals, where Add terminal makes one (`Config.withTerminalAdded`: a free id, your shell) and the trash can removes one with its settings and its place in every notch (`Config.withoutTerminal`); over IPC, `terminal <id> <command>` reaches them.

An agent with `usage` shows its plan's limits under its header: a bar per window (5 hours, the week, a model's week), how much is used and when it resets (`AgentUsage`, rules in `core/Usage.js`). It asks the endpoint the agent's own CLI calls (Claude's `api/oauth/usage`, Codex's `backend-api/wham/usage`) with the login the CLI keeps (`~/.claude/.credentials.json`, `~/.codex/auth.json`, or under `CLAUDE_CONFIG_DIR`, `CODEX_HOME`): every 5 minutes, when the agent finishes a task (at most once a minute), and when the login file changes. That login is only read, never refreshed or written; when it lapses, the last limits stay, faint, with why, until the agent renews it by running. A 429 leaves it alone for at least 5 minutes. A limit that passes `usageWarn` (80 % by default) peeks once in its window, and again when it is reached; the first answer after a start is taken as seen, so a restart doesn't warn again. Offscreen (`scripts/isolated.sh` sets `NOTCHD_OFFLINE=1`, `NotchService.usageLive`) nothing asks the network or reads a login.

Without a `directory`, an agent starts in notchd's own folder, `notchd` in your documents folder (`xdg-user-dir DOCUMENTS`, else `~/Documents`), made the first time (`Terminal.OWN_DIR`); `/resume` in Claude Code and Codex lists the sessions of the folder they start in, so it is always the same one. The terminal comes from the optional `qmltermwidget` package, probed before it loads, and keeps running while the notch is closed. It takes every key, Escape included, except Ctrl+Escape, which closes the notch and leaves the program running (`Terminal.isLeaveKey`); so does the header's button. Over IPC, `agent <id> <command>` reaches it by its id ([ipc.md](ipc.md#agents)), whatever notch shows it. `TerminalModule` without `"agent"` runs any program the same way, with `defaultCommand` and fields of its own.

## Live

A spoken conversation with an agent is a module with `"live"` in its JSON file: a `LiveModule` (core/LiveModule.qml), listed with the agents on the Modules page. `modules/codex-live.json` says `"live": { "command": "codex" }`, which brings its fields (`Settings.liveSettings`): `command` (the one that runs Codex), `directory` (its folder; empty is notchd's, as an agent's), `voice` (`""` for Codex's, else one of Codex's realtime voices, as chips), `access` (`"read-only"`, or `"workspace-write"` to change files in its folder).

It talks while a notch is open on it by a click or a shortcut (`NotchService.liveOpen`: pinned and showing it; a hover only shows it), from the moment that notch opens until it closes (1.5 s later, so a tab or a shortcut pressed twice doesn't end it), or until you stop it (its button, `live <id> stop`), which holds until the notch closes (`Live.wanted`). The restart button (`new`) ends the session and starts another with the transcript cleared. Closed while it talks, the notch shows a red dot (the microphone) and what it does.

A second kind runs the voice on this machine: `modules/local-live.json` says `"live": { "local": true }` (no command), which brings other fields (`Settings.localLiveSettings`, see [settings.md](settings.md)): `agent` (`codex` or `claude`), `command`, `session` (`last`, `new` or an id), `directory`, `access`, then, in a section of its own, speech to text (STT, Voxtype): `language` (a choice: empty is Voxtype's own, `auto`, `pt`, `en`, `es`, `fr`, `de`, `it`, `nl`; any other code written in the config file is passed to Voxtype as it is) and `speechModel` (empty: what Voxtype is set up with); and, in another, text to speech (TTS, a LocalAI voice): `voiceLanguage` (`pt` by default, the languages above), `voiceModel` (an engine: empty the first installed, `piper` or `pocket`, spoken in that language; the model names of an older version, `voice-pt_BR-faber-medium` and `pocket-pt`, mean their engine), `otherVoiceModel` (any model of your LocalAI, by name; it wins over the choice), `voice` (one of the model's voices, empty its own) and `localaiUrl` (empty: notchd's own, `http://127.0.0.1:18080`). Its `LiveModule` has `localVoice: true`, and its bridge is `core/live/local-live.py` (`Live.localBridgeArgs`), with the same lines: the microphone (`parec`) goes through an energy voice activity detector (`Vad`: the noise floor is the 10th percentile of the last 4 s, followed during speech too and rising at most 10 dB a second, digital silence left out; speech starts 12 dB over it (never under -72 dB), goes on while 6 dB over, ends after 700 ms under, 300 ms minimum; the end shows as `working` at once), each utterance, raised to a steady level (`normalize`, at most 30 times: a microphone turned down in the mixer), becomes a 16 kHz mono WAV that `voxtype -q [--model m] [--language l] transcribe <wav>` transcribes (its stdout also has three lines about the file, which `clean_transcript` drops), the text, with a short instruction to answer speakably (`SPOKEN_HINT`), goes to `codex exec --json resume <id|--last> -` or `claude -p --output-format stream-json --resume <id>|--continue` in the folder, and the answer as it streams is cut into sentences, stripped of markdown, and each sentence is a `POST /v1/audio/speech` to LocalAI (`{"model", "input", "voice"?}`), whose WAV (Piper writes 16-bit, Pocket TTS 32-bit float: `wav_to_pcm16` reads both) plays through `pacat` at its rate while the next sentence is synthesized (the level, 25 a second, is of what it sends, paced to what plays). After a `new` first turn the session id the CLI reports is resumed for the next turns. Half duplex: the microphone is ignored while a turn is in flight. Codex streams whole messages, Claude token by token. Every child is killed when it ends.

LocalAI is the engine of the voice, and `core/components/localai.py` is how notchd reaches it: the bridge asks `/readyz` at `localaiUrl`; when nothing answers and the address is on this machine, it starts notchd's own copy (`$XDG_DATA_HOME/notchd/localai/bin/local-ai run --address ... --models-path ... --backends-path ...`, with `HF_HOME` in the same folder, localhost only, log in `$XDG_STATE_HOME/notchd/localai.log`) and stops it when the conversation ends, so an idle notchd runs no LocalAI; one that already answers (yours, or an installation in progress) is used as it is and never stopped. Then it lists `/v1/models`, takes the model named by `--voice-model`, or the first installed of the registry's voices (`known_voices()`, from `components.json`) of `--voice-lang` (`voiceLanguage`) and, when given, `--voice-engine`, Piper before Pocket TTS (a language no voice has falls back to Portuguese), and speaks a first sentence, in that language (the voice's `warmup`), to load it while the microphone calibrates: the session turns `live` once both are ready (a cold Pocket TTS takes about 12 s, Piper 3). What it needs (libpulse, Voxtype, LocalAI and a voice) are optional components, installed from the Voice page of the settings (Components); when one is missing the bridge's error line names it (`components`: `libpulse`, `voxtype`, `localai`, or the voice of the language chosen: `voice-piper-pt-br`, `voice-pocket-pt`, `voice-piper-en`, `voice-pocket-en`…) and the notch says "Install X in Settings › Components" (`Live.errorText`). No question-specific voice, no tuning: a voice speaks as the model's own defaults say.

The voice goes through a bridge, `core/live/codex-live.py`, one per session, the module's `Process`: it starts Codex's server (`codex app-server`), makes a thread of its own (ephemeral, `approvalPolicy: never`, the sandbox `access` says) and opens Codex's realtime conversation on it, v3 over WebRTC (GStreamer's `webrtcbin`): the version and transport a ChatGPT login may use, where a websocket asks for an API key. The microphone (`pulsesrc`, the echo of what it plays taken out by `webrtcdsp`) waits until it sends without a gap before the connection opens, and a leaky queue keeps it running while `webrtcbin` holds what comes in. Codex's voice is decoded and handed to `pacat` with a 200 ms buffer, no GStreamer clock in between; as the server sends it a little slower than it plays, it is stretched (`pitch`, its pitch kept) while that buffer runs low. Its lines, one JSON object each, become the conversation's state (`Live.reduce`, in core/Live.js): the status (`starting`, `connecting`, `live` once the events' channel is open and Codex's voice has come, `closed`), the transcript (both sides, deltas then the whole line), `level` (the voice as it plays, 25 a second, which swells the disc), `mic`, `working` (Codex on a task), `muted`, `warning`, `error` with the `packages` to install. Each session's log, what the server said and the levels each second, is `$XDG_STATE_HOME/notchd/codex-live.log`. It needs `python-gobject`, `gst-plugins-base`, `gst-plugins-good`, `gst-plugins-bad`, `libnice`, `libpulse` and a PulseAudio server: the `codex-live-audio` component, installed from the Codex Live page of the settings (Components). Without them the bridge's error line says which (`packages`) and names the component (`components`), and the notch says to install it in Settings › Components.

## The scope rule

A module **shows** state, it does not **own** it. A notifications module mirrors toasts; it does not register as the notification server. The one exception today is the Omarchy `osd` module, which takes the `osd` IPC target from the stock OSD plugin (and says so in its README).

## Rules

- Sizes go through `Theme.space(px)` and text through `Theme.font.*`, so the module follows the host's density and fonts.
- Colors come from the theme tokens (`Theme.foreground`, `Theme.muted`, `Theme.line`, …), never hard-coded or ad-hoc alphas. See [theming.md](theming.md).
- Views are Components that size themselves through `implicitWidth`/`implicitHeight`; the notch springs to fit. Expanded views get a fixed width: 400 px, or the module's `expandedWidth`.
- `notch` (set by the service) is the `NotchService`; read state from it, never from another module's internals. A module may show in several notches at once, so the service answers for all of them: `notch.expanded` (some notch is open) and `notch.peeking(module)` (some notch shows that module's peek). Each notch picks the vertical views on a side edge itself.
- Views never rotate. On a side edge the notch runs down the edge for `verticalCompact` and `verticalPeek`; `peek` (for a module without a `verticalPeek`) and `expanded` keep their layout and the notch grows sideways to hold them. Give a side-edge view when a module has something narrow to show: the built-ins stack artwork over the equalizer (media), an icon over a progress bar (activities), each short resource's icon over its number (resources), and stand the OSD's bar up.
- Every change on screen redraws the notch's whole window, at the monitor's refresh rate while something moves. Don't animate without pause: a `Behavior` that never settles, or a timer of a few hundred ms easing into each value, keeps the window drawing frames for as long as it runs. Cap such a view at 60 fps with one `Timer` (interval 16) that moves it, as media's equalizer and progress bar do, instead of letting it run at the monitor's rate (160 Hz and up), and stop its timer while the view is hidden (`running: … && visible`): a notch concealed over a fullscreen app hides its card. An `Image` lands in the scene graph's texture atlas, which is sized to the window and the window covers the screen: the first one costs ~25 MB however small it is. Set `mipmap: true` on images, which keeps them out of it (media's artwork does). `make bench` measures what a view costs.

## Which view shows when

| Property | Used when |
|---|---|
| `compact` | the module is the highest-priority active one with a compact view |
| `verticalCompact` | as `compact`, with the notch on a side edge, and in a vertical bar: narrow and tall. Without one the notch shows a dot there |
| `verticalPeek` | as `peek`, on a side edge: narrow and tall. Without one, `peek` shows there |
| `peek` | the module called `requestPeek(ms)` and won: a pinned notch never peeks, hover only lets `urgentPeek` modules through, and a peek on screen is not cut short by a lower-priority one. It shows in every notch that lists the module (on the focused monitor for a notch on every monitor), and at the top of the focused monitor when no notch with the module is there. With `peekOnFocusedScreen` it shows once, on the focused monitor only: in the first notch there that lists the module, else at the top. A standalone module's peek shows in its own notch, on the focused monitor (see [Standalone modules](#standalone-modules)) |
| `expanded` | the notch is expanded and the module is active; every active module's view stacks, highest priority first |

The daemon decides which view shows (`native/notchd-core/src/state.rs` and `peek.rs`, tested in Rust and by the QML scenarios in `tests/qml/scenarios.qml`): modules report their facts (active, priority, which views they have, attention) and their `requestPeek`/`dismissPeek` to it, and the UI shows what its `state` says.

## Loading

The config's `modules` (each notch's, in a config with `notches`) lists ids in order; the service loads every module some notch lists, once. An id resolves to a host module first, then a built-in. An unknown id, or a file whose root is not a `NotchModule`, is skipped with a warning in the log.
