# Omarchy host (plugin `eryk.notchd`)

notchd as an Omarchy shell plugin. The repository root is the plugin folder: `manifest.json` there points at the QML in this directory. It adds three things to the core:

- **Bar button** (`BarIndicator.qml`): the notchd icon in the bar, tinted with the theme. A click opens its panel: every notch, where it is and what it shows; clicking a notch opens it on the monitor you work on (or closes it), its gear opens that notch's settings, and Settings opens the settings window. A right click on the icon opens the settings window straight away.
- **`osd` module** (`modules/osd.qml`): volume, brightness, mic and keyboard light. Standalone: no notch shows it; it has a place of its own on the monitor you work on, top center by default, over any notch there (`"osd": { "position": { "edge", "offset" } }`, or `"enabled": false` to switch it off; settings: Modules › OSD). On a side edge its bar stands up. The module itself lives in the daemon (`native/notchd-modules/src/osd.rs`); `modules/osd.qml` only owns the `osd` IPC target that every `omarchy osd` call uses, forwarding each call to it, so the stock OSD must stay disabled: `omarchy plugin disable omarchy.osd`. Undo it with `omarchy plugin enable omarchy.osd`.
- **Omarchy's theme**: `Service.qml` maps `qs.Commons` `Color`/`Style` onto the core's `Theme`, so the notch follows theme switches and `[font] base-size`.

## Install

```sh
omarchy plugin add https://github.com/eryk-vieira/notchd.git
omarchy plugin disable omarchy.osd      # the notch takes over the OSD
omarchy restart shell
```

For development, link a checkout instead (see [DEVELOPMENT.md](../../DEVELOPMENT.md)):

```sh
ln -sfn "$PWD" ~/.config/omarchy/plugins/eryk.notchd
omarchy restart shell
```

The plugin is enabled by its bar entry (`.bar.layout.right` in `~/.config/omarchy/shell.json` for a new install; it works in any section). It is not listed in `plugins[]`. If it is listed there, `omarchy bar put` thinks it is already on the bar and does nothing.

After changing QML, run `omarchy restart shell`, because the hot reload keeps serving the old compiled QML. Config changes apply live.

## Config

`~/.config/notchd/config.json`, edited by hand, through the settings window (the notchd button in the bar) or with `omarchy-shell eryk.notchd set`. It takes the core keys (see the [README](../../README.md#config)); the OSD is standalone, so it is in no notch's `modules`.

## IPC

`omarchy-shell eryk.notchd <method>`; the methods and the activity fields are in [docs/ipc.md](../../docs/ipc.md).

```sh
omarchy-shell eryk.notchd push '{"id":"deploy","icon":"󰜎","title":"Deploying api","progress":40,"sticky":true}'
omarchy-shell eryk.notchd settings
```
