import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import "../../core"

// Omarchy host: the notch service as an Omarchy shell plugin. Wires the core
// to Omarchy: its IPC name, the osd module that receives `omarchy osd` (a
// standalone module: its own place, top-center by default), the bar's edge
// (a notch there hangs from the bar), Hyprland (for workspaces) and
// Omarchy's theme. The bar indicator (BarIndicator.qml) finds it through the
// shell.
NotchService {
  id: svc

  // Injected by the Omarchy shell.
  property var shell: null

  ipcTarget: "eryk.notchd"
  logName: "eryk.notchd"
  barEdge: svc.shell && svc.shell.bar ? String(svc.shell.bar.position || "top") : "top"
  compositor: hyprland
  defaultModules: ["media", "activities"]
  hostModules: ({
    osd: Qt.resolvedUrl("modules/osd.qml")
  })
  hostSettingsSchema: [
    { page: "appearance", section: "Theme", type: "note", label: "Follows your Omarchy theme",
      help: "Colors, fonts and spacing come from the active Omarchy theme and change with it." }
  ]

  HyprlandCompositor { id: hyprland }

  // The daemon (docs/hosts.md, "The daemon"): started with the shell and kept
  // past it, the authority for the config, the settings schema and IPC. The
  // service runs the script, connects and starts it again when it dies;
  // meanwhile the notch says "notchd not running".
  daemonScript: Qt.resolvedUrl("../../scripts/ensure-daemon.sh")
  daemonClientName: "omarchy"

  // Omarchy's theme mapped onto the core's tokens; follows theme switches.
  QtObject {
    id: omarchyTheme
    readonly property color foreground: Color.foreground
    readonly property color background: Color.background
    readonly property color accent: Color.accent
    readonly property color danger: Color.urgent
    // The theme's green: Omarchy's Color has none, its colors.toml does.
    readonly property var success: omarchyGreen.value
    // The terminal's 16 colors: the theme's ghostty.conf.
    readonly property var terminalPalette: omarchyPalette.value
    readonly property string fontFamily: Style.font.family
    readonly property int fontCaption: Style.font.caption
    readonly property int fontBodySmall: Style.font.bodySmall
    readonly property int fontBody: Style.font.body
    readonly property int fontHeading: Style.font.heading
    readonly property int fontDisplay: Style.font.display
    readonly property real spacingScale: Style.effectiveSpacingScale
  }

  // `green = "#..."` from the current theme's colors.toml, read again when
  // the theme changes (Color.shellValues is reassigned then). "" while
  // missing: the core's default.
  QtObject {
    id: omarchyGreen
    property string value: ""
  }
  FileView {
    id: colorsToml
    path: Quickshell.env("HOME") + "/.local/state/omarchy/current/theme/colors.toml"
    printErrors: false
    onLoaded: {
      var match = /^\s*green\s*=\s*["']?(#[0-9a-fA-F]{6})["']?/m.exec(text())
      omarchyGreen.value = match ? match[1] : ""
    }
    onLoadFailed: omarchyGreen.value = ""
  }
  // `palette = N=#...` from the current theme's ghostty.conf, the ANSI
  // colors its terminals use; [] while missing.
  QtObject {
    id: omarchyPalette
    property var value: []
  }
  FileView {
    id: ghosttyConf
    path: Quickshell.env("HOME") + "/.local/state/omarchy/current/theme/ghostty.conf"
    printErrors: false
    onLoaded: {
      var colors = []
      var re = /^\s*palette\s*=\s*(\d+)\s*=\s*(#[0-9a-fA-F]{6})/gm
      var m
      while ((m = re.exec(text())) !== null) {
        var i = Number(m[1])
        if (i < 16) colors[i] = m[2]
      }
      var out = []
      for (var j = 0; j < 16; j++) out.push(colors[j] || "")
      omarchyPalette.value = colors.length > 0 ? out : []
    }
    onLoadFailed: omarchyPalette.value = []
  }
  Connections {
    target: Color
    function onShellValuesChanged() { colorsToml.reload(); ghosttyConf.reload() }
  }

  Component.onCompleted: Theme.provider = omarchyTheme
}
