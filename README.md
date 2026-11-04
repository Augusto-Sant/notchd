# <img src="assets/icons/hicolor/scalable/apps/notchd.svg" alt="" width="40" align="top"> notchd

![notchd with Claude Code and Codex: both in the closed notch, a plan-usage warning, a fine-tune pushed by a script, and Claude open in its terminal](docs/images/hero.webp)

A dynamic notch for your desktop. It hangs from an edge of the focused screen (the top by default) and shows modules: media, OSD, script activities, system resources, a timer, Claude Code or Codex in a terminal, or anything you write. You can have several notches, each with its own modules and place, and one on every monitor.

notchd is a **daemon plus a UI per OS**. The daemon (`notchd`, written in Rust) holds everything that decides anything: the config, the settings, what the notch shows and when, and every module, including the programs your terminals and agents run. The UI draws what the daemon says and sends it the pointer and keys. On Linux the UI is QML on [Quickshell](https://quickshell.org), for Wayland compositors with layer-shell; macOS and Windows UIs are planned and not started. The daemon outlives the UI: reload the shell and your agent's conversation, timers and voice session are still there. The contract between the two is [docs/architecture.md](docs/architecture.md).

It is a **notch framework, not a shell**. Modules show what the system is already doing; they never take over a system service. The notch is not a notification daemon, a launcher or a lock screen, so it sits on top of whatever desktop you already have.

## Install

notchd runs inside a *host*: the desktop-specific wiring around the generic UI core. The host starts the daemon with the shell (`scripts/ensure-daemon.sh`), which looks for the `notchd` binary in `$NOTCHD_BIN`, on `PATH`, then in `~/.local/share/notchd/bin/`, and failing all three downloads the release the plugin's version pins, checking its SHA-256 against `native/releases.json`. To build it yourself instead: `make daemon` (needs the Rust toolchain, see [DEVELOPMENT.md](DEVELOPMENT.md#prerequisites)) puts it in `native/target/release/`, and `make restart` (Omarchy) also installs it where the host looks. Without a daemon the notch shows "notchd not running".

- **Omarchy**: the repository is an Omarchy shell plugin (`eryk.notchd`), with volume/brightness OSD and a button in the bar for the notches and the settings. See [hosts/omarchy](hosts/omarchy/README.md).
- **Any other Wayland compositor with layer-shell**: the repository is also a standalone Quickshell config (`qs -p <checkout>`). See [docs/standalone.md](docs/standalone.md).
- **GNOME** (Ubuntu's and Fedora's default desktop, GNOME 46 and later): a GNOME Shell extension, another UI for the same daemon (`hosts/gnome/`). From a checkout, `sh scripts/install.sh --extension` installs the daemon (a verified release and a systemd user service) and the extension, then log out and back in; terminals render directly in the notch with GNOME Shell. On other distributions the daemon alone installs with `scripts/install.sh` (see [docs/hosts.md](docs/hosts.md#installing-the-daemon-on-other-distributions)). The optional voice components install with `pacman` and the AUR, so they are Arch-only for now.

## States

| State | When | Shows |
|---|---|---|
| rest | nothing active | a small lip |
| compact | a module is active | with several modules, every active module's `compact` view, side by side (with `tabs: false`, only the highest-priority one's), hanging from its edge; a module with nothing to show closed, like a stopped timer, as its icon |
| peek | a module calls `requestPeek()` | that module's `peek` view, for a few seconds |
| expanded | hover (click pins it, click outside unpins) | the `expanded` view of one active module at a time, with a tab for each; with `tabs: false`, all of them, one above the other |

Over a fullscreen app the closed notch hides; peeks and an already open notch still show.

The open notch is as large as what it shows, up to the screen less a margin; past that, the modules scroll: the mouse wheel (over a terminal it scrolls the terminal's own history) and a scrollbar beside them. A notch with several modules shows one at a time, with a tab for each; opening the notch from a module (a shortcut, `claude "ask ..."`) picks its tab, and closed it shows every active module side by side. An agent that finishes while you look at another tab gets a dot on its tab and on the closed notch, and its peek shows as the notch closes. `tabs: false` puts them back one above the other, the closed notch showing the highest-priority one.

A module that is active with nothing to show closed (a timer that isn't running) shows its icon in the closed notch; `idleIcons: false` leaves it out until it has something. The resources module set to `"short"` stays out either way.

Hold the notch for a moment, then drag it to move it (a quick click only pins it open): it closes as it is picked up, jumps to the screen edge nearest the pointer as you go, sits with its middle under the pointer, snaps to the middle of an edge when close to it, and stays where you drop it. With more than one monitor you can drag it onto another one: a preview shows it there, and the monitor you drop it on becomes its home, where it stays whichever monitor has focus. A peek shows on the notch, and also at the top of the monitor you work on if no notch there shows it. The volume and brightness OSD, standalone, shows in its own place on the monitor you work on. If that monitor shows a workspace left out of `workspaces`, the drop adds it; on another such workspace later, the notch goes back to the monitors whose workspace takes it. `draggable: false` turns dragging off, and `recenter` over IPC puts it back at the top, following focus.

The drop is stored as `position`, which you can also set by hand: `{"edge": "left", "offset": 0.3, "screen": "eDP-1"}`. `edge` is `"top"` (the default), `"bottom"`, `"left"` or `"right"`; `offset` runs from 0 (the left or top end) to 1, 0.5 being the middle of the stretch the bar leaves free; at either end (or close enough that its shoulder would not fit) the notch sits flush in the corner, square against the screen's side or the bar; `screen` names the home monitor (none: follow focus; `"all"`: a copy on every monitor, see [Several notches](#several-notches)). On a side edge peeks and the expanded notch keep their layout and the notch grows sideways; the closed notch runs down the edge with each module's vertical view (media: artwork over the equalizer; activities: icon over a progress bar) and the volume and brightness OSD stands its bar up. A module without a vertical view shows a dot.

The notch covers the windows under it. `reserveSpace: true` keeps windows clear of it instead: the compositor reserves a strip as deep as the closed notch, along the whole of its edge on its home monitor, or on every monitor it may hang on while it follows focus (layer-shell can only reserve whole edges). At rest that is the thin lip; while a module is active, a compact notch's height. It grows at once and shrinks as soon as the notch has collapsed, so windows move when music starts or stops or an activity comes and goes, not for peeks: volume, brightness and the expanded notch open over windows.

`workspaces` limits where the closed notch shows, e.g. `[1, 2, 3]` (ids or names). When the focused screen shows another workspace, the closed notch moves to a screen that shows one of them, or hides if none does. Peeks and the open notch still show, whatever the workspace, so the volume OSD never goes missing. Reserved space follows too: a screen showing a workspace without the notch reserves nothing, so switching a screen between the two moves its windows. It needs a compositor adapter that knows workspaces (Hyprland and GNOME).

## Modules

| Id | Shows |
|---|---|
| `media` | what's playing over MPRIS: artwork, title, progress and controls; peeks on a new track. Its equalizer glides at 60 fps; `"media": { "equalizer": "light" }` (settings: Modules › Now playing › Equalizer) makes it jump about six times a second instead, for about 1 % of a CPU core rather than 8 % while music plays |
| `activities` | whatever scripts push over IPC: a deploy, a build, a download (see [docs/ipc.md](docs/ipc.md#activities)) |
| `resources` | CPU, memory and the disk holding `/`: a bar each, expanded, and their percentages in the closed notch. One that runs short (90 % or more; the CPU for about 10 s in a row) turns red and peeks once. `"resources": { "compact": "short" }` (settings: Modules › Resources › Closed notch) keeps them out of the closed notch until one runs short. Linux only. Not loaded by default: add it to `modules` |
| `timer` | a countdown or a pomodoro, started from the expanded notch (5, 15, 25 min, Pomodoro) or with `timer 25m` over [IPC](docs/ipc.md#timer). The closed notch shows the time left; at the end it peeks, even over the open notch, and sends a desktop notification. A pomodoro runs its focus rounds and short breaks, then a long break, and stops. Settings: Modules › Timer (lengths, rounds, starting each phase on its own, the notification). Not loaded by default |
| `claude` | [Claude Code](https://claude.com/claude-code) in its own terminal UI inside the notch (needs `claude` on `PATH`; the terminal is notchd's own, emulated by Ghostty's libghostty-vt in the daemon, with the kitty keyboard protocol, images, mouse, selection and Ctrl+Shift+C/V). Closed, it shows the conversation's title, which Claude Code makes after the first message, and a green dot while it works; open, the terminal takes the keyboard, Escape included; Ctrl+Escape, its × button or the shortcut close the notch, and so does moving to a window with a focus or workspace shortcut. It starts the first time the notch opens and keeps running while it is closed, and through a shell reload (`make restart`: it runs in the daemon, and the notch comes back to the same session); when it finishes something while closed, the notch peeks. The restart button (or `agent claude new` over [IPC](docs/ipc.md#agents)) starts a new one (`/resume` brings a conversation back), and `agent claude "ask <question>"` opens it and types the question in. Open, it shows the plan's limits (5 hours, the week) with when each resets, and peeks when one passes 80 % and when it is reached; notchd asks them the way Claude Code does, reading its login without ever changing it. Settings: Modules › Agents › Claude (the folder, by default notchd's own, `notchd` in your Documents folder; plan usage and when to warn; the command, e.g. `claude --permission-mode auto`; size; font; stopping it after a while closed, off by default). Not loaded by default: give it a notch of its own and bind `agent claude toggle` to a key, which opens it whatever the notch is called |
| `codex` | [Codex](https://github.com/openai/codex), OpenAI's coding agent, the same way as `claude`: its own terminal UI in the notch (needs `codex` on `PATH`), the thread's title, which Codex makes after the first message, and a green dot closed, `agent codex toggle`, `agent codex new` and `agent codex "ask <question>"` over [IPC](docs/ipc.md#agents). It runs as plain `codex`, connected to the shared background server: options that change its config (`-c`, `--search`) run it without, while `--approve-for-me` doesn't. Its plan's limits show and warn as Claude's do. Settings: Modules › Agents › Codex (folder, notchd's own by default; plan usage; command; size; font; stopping it after a while closed: it takes 500 to 700 MB). Not loaded by default |
| `local-live` | the same kind of spoken conversation, with the speech to text ([Voxtype](https://github.com/peteonrails/voxtype), the one Omarchy dictates with) and the text to speech ([LocalAI](https://localai.io), any of its text-to-speech models) on your machine, and your words going into a normal text session of Codex or Claude Code that it resumes (the most recent one in its folder, a new one, or an id): voice is one more input to the conversation you also type in. Half duplex: it doesn't listen while it answers. `live local-live start \| stop \| toggle \| new \| mute \| unmute` over [IPC](docs/ipc.md#live). What it needs (Voxtype, LocalAI and a voice, in Portuguese, English, Spanish, French, German, Italian or Dutch: Piper, from 141 MB, or Pocket TTS, from 1.9 GB, both on the CPU) is installed from its settings, with progress. Settings: Modules › Agents › Voice. Not loaded by default |
| `codex-live` | a spoken conversation with [Codex](https://github.com/openai/codex): open its notch with a click or a shortcut and talk; it answers aloud and does what you ask in its folder, in a thread of its own, on the ChatGPT login Codex keeps. A disc swells with its voice, the transcript shows under it, and the microphone is on only while the notch is open on it (a red dot), the speakers' echo taken out. The restart button starts over with the transcript cleared; `live codex-live start | stop | toggle | new | mute | unmute` over [IPC](docs/ipc.md#live). It needs Codex logged in and GStreamer's WebRTC, which its settings install (Components). Settings: Modules › Agents › Codex Live (voice, folder, whether it may change files there: by default it only reads). Not loaded by default |
| `osd` | volume and brightness, Omarchy host only (see [hosts/omarchy](hosts/omarchy/README.md)). Standalone: no notch shows it, it has a place of its own, at the top center of the monitor you work on, over any notch there. `"osd": { "position": { "edge": "bottom", "offset": 0.5 } }` moves it and `"osd": { "enabled": false }` switches it off (settings: Modules › OSD) |

Your own **terminals** are modules too: each entry of the config's `terminals` runs its command in a terminal in the notch, and a notch shows it by its id like any module:

```json
{
  "terminals": {
    "btop": { "title": "System", "command": "btop" },
    "dev": { "title": "Dev server", "command": "cd ~/app && npm run dev" }
  },
  "notches": [{ "modules": ["media"] }, { "id": "tools", "modules": ["btop", "dev"] }],
  "btop": { "width": 900, "height": 500 }
}
```

The command is a shell command line, run in your home unless the terminal's `directory` says otherwise. It starts the first time the notch opens and keeps running while it is closed and through a shell reload (it runs in the daemon); closed, the notch shows the title the program gives the terminal (a shell's is where it is), else the entry's `title`. Its other settings live under its id, as an agent's do (`directory`, `idleMinutes`, `width`, `height`, `font`), so the id can't be a config key or another module's. The settings window makes them too: Modules › Terminals › Add terminal starts one running your shell and opens its page (title, command, folder, size); its trash can takes it out of the config and of every notch. `terminal <id> toggle` over [IPC](docs/ipc.md#terminals) opens or closes it.

A **standalone** module (the OSD) has a fixed job and a place of its own: no notch shows it, even one whose `modules` list it. It loads unless `<id>.enabled` is `false`, shows on the monitor you work on where `<id>.position` (`edge`, `offset`) says, top center by default, and draws over any notch that hangs there, which hides until it goes.

To write your own, see [docs/modules.md](docs/modules.md).

## Your own modules

A module is a folder in `~/.config/notchd/modules/<id>/` with a `module.json` and a program in any language that prints JSON lines for what the notch shows and reads the clicks, settings and provider data (CPU, media, screens, plan usage) notchd sends it. It reloads when you save, and a crash or a mistake shows in the notch with the log's path. Start from the [examples](docs/examples/modules/) (shell, Python, Node); the guide is [User modules](docs/modules.md#user-modules).

### Showcase: Herdr

The [Herdr example](docs/examples/modules/herdr/) reads a running session and shows agent states, workspaces and panes above a regular notchd terminal running Herdr. It peeks when an agent needs input or finishes, with optional desktop notifications. Copy the module folder into `~/.config/notchd/modules/`, then put `herdr` and a terminal running `herdr` in one notch with `tabs: false`. The [example README](docs/examples/modules/herdr/README.md) has the config and setup steps. It needs no built-in integration or daemon rebuild.

## Several notches

`notches` lists them, each with its own modules and place: the media player at the top, script activities at the bottom of the laptop screen, say. A notch takes the keys marked *per notch* in the table below; the ones it leaves out come from the top level of the config, then their defaults (`position` key by key). Without `notches` there is one notch, made of the top-level keys.

```json
{
  "reserveSpace": true,
  "notches": [
    { "id": "main", "modules": ["media"], "position": { "screen": "all" } },
    { "id": "jobs", "modules": ["activities"], "position": { "edge": "bottom", "screen": "eDP-1" } }
  ]
}
```

- `position.screen` decides where each one is: following focus (none), on one monitor (its name), or on every monitor (`"all"`), each with a copy of its own that you hover, pin and drag on its own.
- A module may show in several notches. Its peek shows in each of them: on the focused monitor for a notch on every monitor (or one that follows focus), the others keeping what they show, and where it is for a notch kept on one monitor. When no notch with the module is on the monitor you work on, the peek also shows there, once, at the top. The volume and brightness OSD is standalone: it shows in its own place on the monitor you work on, never in a notch. So changing the volume on the laptop shows it there, and the notch with the music stays on the ultrawide.
- Notches never sit on each other. On the same edge of a monitor they go side by side, each as near its `offset` as the others let it; when one grows (music starts) its neighbors make room. Notches that come within a few pixels of each other (or are dropped near one) join flush into one shape, as deep as the deepest of them. One that opens (a peek, the pointer) grows from where it is and pushes its neighbors aside, staying joined to them: in the middle of the edge it grows evenly to both sides, against the screen's side it grows away from it. A neighbor that no longer fits on the edge hides until it closes.
- Notches that share an edge reserve the deepest of them (`reserveSpace`), not the sum.
- `id` names a notch for IPC (`openNotch jobs`); without one the first is `main` and the others `notch-2`, `notch-3`...

### Navigating from the keyboard

`navigate` over [IPC](docs/ipc.md#navigation), bound to a key, highlights every notch, hidden ones included, with a small label at its top right: its number (config order) and name. The selected one opens as a preview, the keys listed in its label, and the others show closed, even one pinned open:

| Key | |
|---|---|
| `1`–`9` | select that notch |
| `←` `→`, `h` `l` | the notch before or after |
| `Tab`, `Shift+Tab` | the selected notch's next or previous tab |
| `Enter` | open it, pinned, on that tab (an agent gets the keyboard); the other notches close |
| `Esc`, `Ctrl+Esc`, or the shortcut again | leave, everything as it was |
| `s` | the selected notch's settings |
| `x`, then `x` again | remove the selected notch (any other key keeps it); never the last one |
| `m` | move the selected notch: `←` `↑` `↓` `→` (or `hjkl`) put it on that edge, `Shift`+arrow slides it along, `Tab` takes it to the next monitor; `Enter` keeps it there, `Esc` puts it back |

A notch pinned open, this way, by a click or a shortcut, takes the keyboard: `Esc` or `Ctrl+Esc` closes it (in an agent's terminal, `Ctrl+Esc`).

```lua
o.bind("SUPER + N", "Navigate notches", "omarchy-shell eryk.notchd navigate")   -- ~/.config/hypr/bindings.lua
```

In the settings window, pick the notch to set up at the top of the Notches page, which has everything that notch has of its own (modules, position, workspaces); Add notch puts a new one on a free edge, and removing all but one puts its keys back at the top level.

## Config

The config lives in `$XDG_CONFIG_HOME/notchd/config.json` (usually `~/.config/notchd/config.json`), outside the source tree, so plugin updates never touch it. A missing file means every key is at its default. A file you create by hand is picked up within a couple of seconds, and edits apply live.

| Key | Default | |
|---|---|---|
| `modules` | `["media", "activities"]` | module ids to show, in order; *per notch* |
| `hoverOpenDelay` | `120` | ms |
| `hoverCloseDelay` | `300` | ms |
| `position` | top, middle, following focus | `{ "edge", "offset", "screen" }`: where the notch sits; see above; *per notch* |
| `draggable` | `true` | drag the notch to move it; *per notch* |
| `reserveSpace` | `false` | keep windows clear of the closed notch; see above; *per notch* |
| `tabs` | `true` | a notch with several modules shows one at a time open, with a tab for each, and all of them side by side closed; see above; *per notch* |
| `idleIcons` | `true` | a module with nothing to show closed shows its icon there; see above; *per notch* |
| `workspaces` | every one | workspaces the closed notch shows on; see above; *per notch* |
| `notches` | one notch | `[{ "id", ...per-notch keys }]`; see [Several notches](#several-notches) |
| `terminals` | none | `{ "<id>": { "command", "title" } }`: terminals of your own, modules a notch lists by id; see [Modules](#modules) |

Hosts add their own keys; the standalone host, for example, adds `topOffset` and `theme`. A module's own settings live under its id, `resources` for example. A key missing from the file falls back to its default.

## Settings

The settings window opens from the host: in Omarchy, the notchd button in the bar (Settings in its panel, or a right click); anywhere, the `settings` IPC method. It has a sidebar of pages, and on each page its fields grouped by section. Every change applies at once and is written back to the config file; there is no Apply button. Clearing a field, or picking its default, removes the key. Ctrl+Tab cycles pages; Esc closes.

### Optional components

Nothing heavy ships with notchd. What the voice modules need beyond the app (LocalAI and its voices, Voxtype, a few Arch packages) is installed, removed and configured from the Components page of the settings, each with its size, a progress bar while it installs, and Install, Cancel and Remove buttons; the Voice and Codex Live pages have the rows they need. LocalAI and its voices go to `~/.local/share/notchd/`; system packages ask for your password (through polkit, or in a terminal) and are never removed from here. An installation goes on if you close the settings.

## IPC

Scripts drive the notch through the host's IPC (Quickshell's) or the daemon's own CLI, `notchd ipc <method>`: push an activity, open the notch, change a config key. See [docs/ipc.md](docs/ipc.md).

```sh
omarchy-shell eryk.notchd push '{"id":"deploy","title":"Deploying api","progress":40,"sticky":true}'
```

## Documentation

- [docs/architecture.md](docs/architecture.md): the daemon, the protocol with the UIs, view primitives, providers
- [docs/modules.md](docs/modules.md): writing a module, and [user modules](docs/modules.md#user-modules) in any language
- [docs/standalone.md](docs/standalone.md): the standalone host
- [docs/hosts.md](docs/hosts.md): what a host provides, for porting notchd to another desktop
- [docs/settings.md](docs/settings.md): the settings schema, pages and controls
- [docs/theming.md](docs/theming.md): theme tokens
- [docs/ipc.md](docs/ipc.md): IPC methods
- [DEVELOPMENT.md](DEVELOPMENT.md): working on notchd itself
- [CHANGELOG.md](CHANGELOG.md)

## License

MIT, see [LICENSE](LICENSE). The Omarchy host's `osd` module adapts code from Omarchy, which is also MIT; see [NOTICE](NOTICE).
