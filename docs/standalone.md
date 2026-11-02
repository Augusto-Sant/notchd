# Standalone host

The notch as its own Quickshell config (`shell.qml` at the repo root), for any Wayland compositor with layer-shell. It uses the Hyprland adapter when `HYPRLAND_INSTANCE_SIGNATURE` is set, and the generic one otherwise. The generic adapter keeps the notch on the first screen, doesn't hide it for fullscreen apps and knows no workspaces, so the `workspaces` setting is Hyprland-only here.

```sh
git clone https://github.com/eryk-vieira/notchd.git ~/.local/share/notchd
qs -p ~/.local/share/notchd
NOTCHD_CONFIG=~/notchd-test.json qs -p ~/.local/share/notchd   # another config file
```

Start it with your session, e.g. Hyprland's `exec-once = qs -p ~/.local/share/notchd`.

It also starts the notchd daemon (see [hosts.md](hosts.md#the-daemon)): `scripts/ensure-daemon.sh` needs the `notchd` binary on your `PATH`, in `$NOTCHD_BIN`, or in `~/.local/share/notchd/bin/` (`make daemon` builds it into `native/target/release/`; no release is published yet, so there is nothing to download until one is). The daemon decides the notches' state: without a binary the log says so once and the notch shows "notchd not running". It stays running after the shell exits; `notchd ipc shutdown` stops it.

Differences from the Omarchy host:

- **No bar button**, so the settings window opens over IPC (`settings`, below); bind it to a key in your compositor to have it at hand.
- **No `osd` module yet.** It needs its own volume and brightness sources.
- **Theme from the config.**

## Config

`$NOTCHD_CONFIG`, or `~/.config/notchd/config.json` by default, the same file the Omarchy host uses. Each host ignores the other's keys; an Omarchy-only module such as `osd` in `modules` is skipped with a warning in the log. It takes the core keys (see the [README](../README.md#config)), plus:

| Key | Default | |
|---|---|---|
| `topOffset` | `0` | room at the top of the screen, such as your bar's height: a notch on the top edge hangs below it, one on a side edge stays below it |
| `theme` | `{}` | any of `foreground`, `background`, `accent`, `fontFamily`, `spacingScale` |

All of these keys except `spacingScale` are in the settings window too.

Use a Nerd Font for `fontFamily` so module icons render.

## IPC

```sh
qs -p ~/.local/share/notchd ipc call notchd push '{"id":"deploy","title":"Deploying api","progress":40,"sticky":true}'
qs -p ~/.local/share/notchd ipc call notchd clear deploy
qs -p ~/.local/share/notchd ipc call notchd settings          # settings window
qs -p ~/.local/share/notchd ipc call notchd set topOffset 32   # change one key; '' resets it
qs -p ~/.local/share/notchd ipc call notchd state
```
