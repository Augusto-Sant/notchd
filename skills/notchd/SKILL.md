---
name: notchd
description: Control a running notchd instance through its CLI when the user asks to inspect or change notches, activities, timers, agents, terminals, settings, or components. Do not use for ordinary development of the notchd repository or unrelated terminal work.
---

# notchd

notchd is a daemon with a UI. Its `notchd ipc <method> [args...]` command talks directly to the daemon and prints the method's reply. Read `notchd --help` for the installed binary's global options and `docs/ipc.md` in the notchd repository for method details.

## Choose the instance

If `NOTCHD_BIN` is set by a notchd-managed terminal, use that executable. Otherwise use `notchd` on `PATH`, or locate the installed binary before issuing commands. Do not start or replace a daemon merely to inspect it.

A notchd-managed terminal inherits `NOTCHD_SOCKET` and `NOTCHD_INSTANCE`, so the CLI reaches the daemon that owns it. Outside one, the CLI uses its default instance unless the user identifies another. Pass `--socket <path>` or `--instance <name>` **before** `ipc` when targeting a known non-default instance. Keep that target on every command in the workflow. `NOTCHD_CONFIG` or `--config` selects the config file; it does not select a different daemon on its own.

Start with a read when the target or module IDs are uncertain:

```sh
notchd ipc state
```

The reply is JSON with `notches`, `standalone`, and loaded `modules`. Inspect their IDs instead of assuming a notch or module exists. If the daemon cannot be reached, report the socket error; do not run `daemon --ensure` as a retry, since it can replace another version's daemon and end its sessions.

## Use the available IPC

```sh
notchd ipc push '{"id":"build","title":"Building","progress":40,"sticky":true}'
notchd ipc clear build
notchd ipc timer '3m Tea'
notchd ipc agent codex toggle
notchd ipc agent codex 'ask explain this error'
notchd ipc terminal shell toggle
notchd ipc openNotch main
notchd ipc set hoverOpenDelay 200
```

- `push` and `clear` need the `activities` module; `timer` needs `timer`. `clear ""` removes every activity.
- `agent <id> toggle | new | "ask <text>"` controls a loaded `claude` or `codex` module. `terminal <id>` has the same commands for configured terminals. `ask` opens its notch and types into the program; it does not return the program's answer. If `NOTCHD_TERMINAL_ID` names the current terminal, do not send `ask` or `new` to that same terminal from inside it.
- `open`, `close`, `toggle`, `openNotch`, `closeNotch`, and `toggleNotch` affect the visible notch. `settings` toggles the settings window; `navigate` enters or leaves keyboard navigation.
- `set <key> <json>` persists a config edit. An empty value removes the key and restores its default. Read the relevant setting in the config or `docs/settings.md` before changing it; use `set` only when the user asked for that change.
- `component status [id]` reads component status. `install`, `cancel`, and `remove` change installed components; `shutdown` ends the daemon and its terminal sessions. Run these only when the user explicitly requests the corresponding action.

Check both the process exit status and the printed reply. Some application errors, such as `no timer running`, `<id> module not loaded`, or `not JSON: ...`, are successful IPC replies rather than CLI failures. For a method's full syntax, consult `docs/ipc.md` or the installed version's documentation instead of guessing.

## Boundaries

The current CLI can send commands to an agent or terminal, but it has no `agent read` or `agent wait` equivalent. `notchd attach term:<id>` opens an interactive terminal view; it is not a noninteractive result reader. Do not infer that an `ask` finished from an `ok` reply.
