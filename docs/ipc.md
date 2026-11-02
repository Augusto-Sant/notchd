# IPC

Every host exposes the same methods on its `ipcTarget`. The CLI depends on the host:

| Host | Command |
|---|---|
| Omarchy | `omarchy-shell eryk.notchd <method> ...` |
| standalone | `qs -p <checkout> ipc call notchd <method> ...` |
| the daemon (any host) | `notchd ipc <method> ...` |

The daemon is what answers: `notchd ipc` talks to it directly and prints the method's own reply. A host's target (Quickshell's `IpcHandler`) cannot wait for a reply, so it sends the command to the daemon and answers what the shell already knows (`ok`, `no notch <id>`, `<id> module not loaded`); where a method's answer depends on a module (`no timer running`, the usage text, an agent's plan `usage` in `state`), only `notchd ipc` has it. The daemon's socket is `$XDG_RUNTIME_DIR/notchd/<instance>.sock`; `NOTCHD_INSTANCE`, `NOTCHD_SOCKET` and `NOTCHD_CONFIG` pick another daemon, as `make dev` does.

| Method | |
|---|---|
| `push '<json>'` | add or update an activity (below) |
| `clear <id>` | remove one; `""` removes all |
| `open`, `toggle` | pin the first notch open on the focused monitor, or flip it |
| `close` | close every notch |
| `openNotch <id>`, `closeNotch <id>`, `toggleNotch <id>` | the same for one notch, by its `id` (`main`, `notch-2`...), its Name in settings: renaming it breaks a shortcut that names it |
| `agent <id> <command>` | an agent (`claude`, `codex`) by its id: `toggle` opens or closes its notch, `new` starts a new session, `"ask <question>"` opens it and types the question in ([below](#agents)) |
| `claude <command>`, `codex <command>` | the same as `agent claude <command>`, `agent codex <command>` |
| `terminal <id> <command>` | a terminal of your own (the config's `terminals`) by its id, with the same commands as an agent ([below](#terminals)) |
| `timer <command>` | the `timer` module: start, pause or stop a countdown or a pomodoro (below) |
| `navigate` | start or leave navigation: every notch highlighted and numbered, the keyboard picks one ([below](#navigation)) |
| `settings` | open or close the settings window |
| `recenter` | put every notch back in the middle of the top edge, following focus (removes each `position`) |
| `set <key> <json>` | change one config key, as the settings window does; an empty value resets it |
| `state` | as JSON: `navigation` (the notch selected while navigating, else null), `notches`, each with its id, reserved space, workspaces, position (where it is), whether it can be dragged, its modules and its `copies` (screen, mode and current peek of each); each `standalone` module's place and the copies that show it; and every loaded module, with an agent's plan `usage` (its limits, whether they are stale and why; only `notchd ipc state` has it, not the host's own target) |
| `shutdown` | the daemon only (`notchd ipc shutdown`; the QML service has no such target): the daemon replies `ok`, closes its socket and exits. What `notchd daemon --ensure` uses to replace a daemon of another version |

```sh
omarchy-shell eryk.notchd set hoverOpenDelay 200
omarchy-shell eryk.notchd set position.edge '"left"'
omarchy-shell eryk.notchd set hoverOpenDelay ''        # back to the default
omarchy-shell eryk.notchd set position ''              # back to top-center
omarchy-shell eryk.notchd set notches.1.position.edge '"bottom"'   # the second notch
omarchy-shell eryk.notchd toggleNotch jobs
```

For an array, put a space before the bracket: `set modules ' ["media","activities"]'`. Quickshell's CLI reads an argument that starts with `[` as a comma-separated list.

## Navigation

`navigate` highlights every notch of the config, hidden ones included, each with a small label at its top right, outside it: its number (its place in `notches`, 1 to 9) and its name. It takes the keyboard until you leave. The selected notch opens as a preview on the focused monitor (not pinned), the keys listed in its label; the others show closed meanwhile, even one pinned open, which opens again on Escape. It starts on the open notch, else the first on the focused monitor.

| Key | |
|---|---|
| `1`–`9` | select that notch |
| `←` `→`, `h` `l` | the notch before or after, wrapping |
| `Tab`, `Shift+Tab` | the selected notch's next or previous tab |
| `Enter` | pin it open on that tab and leave; the other notches close. It takes the keyboard, so `Esc` or `Ctrl+Esc` closes it later |
| `Esc`, `Ctrl+Esc` | leave, each notch on the tab it had |
| `s` | leave and open the selected notch's page in settings |
| `x` | ask to remove the selected notch: `x` again removes it, any other key keeps it. The last notch stays |
| `m` | move the selected notch, live: `←` `↑` `↓` `→` (`h` `k` `j` `l`) onto that edge, `Shift`+arrow 5 % along it, `Tab` / `Shift+Tab` to the next or previous monitor (letting in its workspace, as a drop does). `Enter` keeps it, `Esc` puts it back; both stay in navigation |

`navigate` again leaves too, as Escape does (a move under way goes back), and a click that pins a notch leaves on that one. An agent open when it starts keeps running and gets the keyboard back on Escape. Bind it to a key:

```lua
o.bind("SUPER + N", "Navigate notches", "omarchy-shell eryk.notchd navigate")   -- Omarchy, ~/.config/hypr/bindings.lua
```

```sh
bind = SUPER, N, exec, qs -p <checkout> ipc call notchd navigate   # standalone, hyprland.conf
```

The daemon decides it all (`native/notchd-core/src/navigate.rs`, driven by the `key` messages the navigation window sends); the QML only draws the highlight.

## Activities

The `activities` module shows whatever scripts push: a deploy, a build, a long download.

```sh
omarchy-shell eryk.notchd push '{"id":"deploy","icon":"󰜎","title":"Deploying api","subtitle":"staging","progress":40,"sticky":true}'
omarchy-shell eryk.notchd push '{"id":"deploy","progress":80,"peek":false,"sticky":true,"title":"Deploying api"}'
omarchy-shell eryk.notchd clear deploy
```

| Field | |
|---|---|
| `id` | the same id updates the item in place |
| `icon` | a Nerd Font glyph |
| `title`, `subtitle` | |
| `progress` | 0 to 100; omit it for no bar |
| `sticky` | keep the item in compact/expanded until it is cleared; without it, the item only peeks |
| `peek` | `false` updates without popping open |
| `duration` | peek length in ms |

## Timer

The `timer` module takes one argument, quoted when it has spaces:

| Command | |
|---|---|
| `25m`, `90s`, `1h30m`, `1:30`, `25` (minutes) | start a countdown |
| `"3m Tea"` | with a label |
| `pomodoro` | start a pomodoro (lengths and rounds in settings: Modules › Timer) |
| `toggle` (or nothing) | pause or resume; a pomodoro waiting for its next phase starts it |
| `pause`, `resume` | |
| `+5m` | add time |
| `skip` | a pomodoro's next phase; a countdown stops |
| `stop` | |

```sh
omarchy-shell eryk.notchd timer "3m Tea"
omarchy-shell eryk.notchd timer pomodoro
omarchy-shell eryk.notchd timer toggle
```

It answers `ok`, `no timer running`, the usage, or `timer module not loaded` when no notch lists `timer`. The timer lives in the daemon, which answers: `notchd ipc timer ...` prints those. The shell's own target (`omarchy-shell eryk.notchd timer`) cannot wait for the daemon: it answers `timer module not loaded` itself when no notch lists `timer`, else `ok` once it has sent the command, so `no timer running` and the usage only come from `notchd ipc`. The same goes for `push` and `clear` (`activities module not loaded`, and `invalid json` for a payload that is not a JSON object).

## Agents

The agents (`claude`, `codex`) run in a terminal in the notch. `agent <id> toggle` opens the notch that shows one, on its tab, or closes it: bind that to a key, since it finds the notch by the agent it shows, whatever the notch is called or wherever the agent is. From a Hyprland binding:

```sh
bind = SUPER ALT, C, exec, omarchy-shell eryk.notchd agent claude toggle
```

| Command | |
|---|---|
| `toggle` | open the agent's notch (the first that lists it) on its tab, or close it while it shows the agent |
| `new` | a new session: the running one stops (`/resume` brings it back) |
| `"ask <question>"` | open the agent's notch and type the question in, then Enter, once the agent is up; `ask` alone only opens it |

```sh
omarchy-shell eryk.notchd agent claude "ask what does EXDEV mean?"
omarchy-shell eryk.notchd agent codex "ask explain: $(wl-paste --primary)"   # the selected text, as one line
```

It answers `ok`, `the terminal needs the qmltermwidget package` (no UI connected can draw a terminal), the usage, `<id> module not loaded` when no notch lists it, or `no agent <id>` with the agents there are. The agents live in the daemon, which answers: `notchd ipc agent claude ...` prints those. The shell's own target (`omarchy-shell eryk.notchd agent ...`) cannot wait for the daemon and answers `ok` once it has sent the command (it still answers `no agent <id>` and `<id> module not loaded`, which it knows). The program runs in the daemon, so an agent keeps running through a shell reload (`make restart`) and the notch comes back to the same session.

## Live

A spoken conversation (`codex-live` or `local-live`, [modules.md](modules.md#live)), by its module's id. It talks while its notch is open by a click or a shortcut; these drive it from a key or a script:

```sh
bind = SUPER ALT, V, exec, omarchy-shell eryk.notchd live codex-live toggle
```

| Command | |
|---|---|
| `start` | open its notch (the first that lists it), which starts talking |
| `stop` | end the conversation; the notch stays open, and it starts again the next time it opens |
| `toggle` | `stop` while it talks, else `start` |
| `new` | a new session, the transcript cleared |
| `mute`, `unmute` | the microphone, while it talks |

It answers `ok`, `no notch shows <id>; add it to one`, `not in a conversation`, the usage, `<id> module not loaded`, or `no live module <id>` with the ones there are. The module and its bridge are the daemon's, so `notchd ipc live ...` prints those answers; the shell's own target answers `ok` once it has sent the command.

## Components

The optional components ([settings.md](settings.md#components)): `notchd ipc component status [id]` prints a line per component (`<id>: <state in words>`), `component install <id>`, `component cancel <id>` and `component remove <id>` run the installer (the same as the buttons of the settings) and answer `ok`, or `no component <id>; components: ...`. An installation runs apart from the daemon (detached) and goes on if it stops.

## Terminals

A terminal of your own (an entry of the config's `terminals`) takes the same commands by its id: `terminal <id> toggle`, `terminal <id> new` (its command starts again), `terminal <id> "ask <text>"` (the text typed in, then Enter: a command for a shell, say).

```sh
bind = SUPER ALT, B, exec, omarchy-shell eryk.notchd terminal btop toggle
omarchy-shell eryk.notchd terminal shell "ask git status"
```

It answers as `agent` does, with `no terminal <id>` and the terminals there are for an unknown one. A terminal's program also lives in the daemon and survives a shell reload; `notchd attach term:<id>` shows it on your terminal, which is how the notch's view is made on Linux ([architecture.md](architecture.md#free-surface-tier-3)).
