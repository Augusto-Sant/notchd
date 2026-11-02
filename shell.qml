import QtQuick
import Quickshell
import "core"

// Standalone host: the notch as its own Quickshell config, for any Wayland
// compositor with layer-shell. Run it with `qs -p <repo root>`.
//
// It sits at the repo root, not under hosts/, because Quickshell only
// resolves types from files inside the config folder: `import "../../core"`
// from hosts/standalone fails with "NotchSurface is not a type".
//
// No desktop shell to lean on, so: no bar indicator (settings open over IPC),
// the theme comes from the config's "theme" object, and the gap
// above the notch (e.g. your bar's height) from "topOffset".
//
// Config: $NOTCHD_CONFIG if set, else $XDG_CONFIG_HOME/notchd/config.json.
ShellRoot {
  id: shell

  readonly property var config: service.config || ({})
  readonly property var themeConfig: shell.config.theme || ({})

  NotchService {
    id: service
    ipcTarget: "notchd"
    logName: "notchd"
    // The daemon (docs/hosts.md, "The daemon"): started with the shell and
    // kept past it, the authority for the config (NOTCHD_CONFIG, else the
    // user's config dir), the settings schema and IPC. The service runs the
    // script, connects and starts it again when it dies; meanwhile the notch
    // says "notchd not running".
    daemonScript: Qt.resolvedUrl("scripts/ensure-daemon.sh")
    daemonClientName: "standalone"
    compositor: compositorLoader.item
    hostSettingsSchema: [
      { page: "general", section: "Layout", key: "topOffset", type: "integer", label: "Top offset",
        default: 0, min: 0, max: 120, step: 1, unit: "px",
        help: "Room at the top of the screen, e.g. your bar's height." },
      { page: "appearance", section: "Colors", key: "theme.foreground", type: "color", label: "Foreground", default: "#cacccc" },
      { page: "appearance", section: "Colors", key: "theme.background", type: "color", label: "Background", default: "#101315" },
      { page: "appearance", section: "Colors", key: "theme.accent", type: "color", label: "Accent",
        help: "Defaults to the foreground color." },
      { page: "appearance", section: "Text", key: "theme.fontFamily", type: "string", label: "Font", default: "monospace",
        help: "Use a Nerd Font so module icons render." }
    ]
  }

  QtObject {
    id: configTheme
    readonly property var foreground: shell.themeConfig.foreground
    readonly property var background: shell.themeConfig.background
    readonly property var accent: shell.themeConfig.accent
    readonly property var fontFamily: shell.themeConfig.fontFamily
    readonly property var spacingScale: shell.themeConfig.spacingScale
  }
  Component.onCompleted: Theme.provider = configTheme

  readonly property bool onHyprland: !!Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE")
  Loader {
    id: compositorLoader
    sourceComponent: shell.onHyprland ? hyprlandAdapter : genericAdapter
  }
  Component { id: hyprlandAdapter; HyprlandCompositor {} }
  Component { id: genericAdapter; GenericCompositor {} }

  NotchSurface {
    svc: service
    barHeight: Math.max(0, Number(shell.config.topOffset) || 0)
  }
}
