pragma Singleton
import QtQuick

// Visual tokens for the notch and its modules. The core never reads a desktop's
// theme itself: the host sets `provider` to an object with any of the keys
// below (a host's own theme mapped onto these names) and everything else falls
// back to the defaults here. Bindings on Theme follow the provider live.
//
//   foreground, background, accent,
//   danger, success                        colors
//   fontFamily                             font family name
//   fontCaption, fontBodySmall, fontBody,
//   fontHeading, fontDisplay               pixel sizes
//   spacingScale                           multiplier behind space()
//   terminalPalette                        the theme's 16 ANSI colors
//                                          ("#rrggbb", color 0 first)
QtObject {
  id: theme

  property var provider: null

  function pick(key, fallback) {
    var p = theme.provider
    var v = p ? p[key] : undefined
    return v === undefined || v === null || v === "" ? fallback : v
  }

  readonly property color foreground: pick("foreground", "#cacccc")
  readonly property color background: pick("background", "#101315")
  readonly property color accent: pick("accent", theme.foreground)
  readonly property color danger: pick("danger", "#e06c75")
  readonly property color success: pick("success", "#98c379")
  // The terminal's ANSI colors: [] leaves them to the terminal.
  readonly property var terminalPalette: pick("terminalPalette", [])

  // Derived tones. Use these instead of ad-hoc alphas, so every surface
  // (notch, modules, settings) shades the same way on any theme.
  function alpha(c, a) { return Qt.rgba(c.r, c.g, c.b, a) }
  function mix(a, b, t) {
    return Qt.rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1)
  }
  readonly property color muted: alpha(theme.foreground, 0.6)       // secondary text
  readonly property color faint: alpha(theme.foreground, 0.35)      // disabled, hints
  readonly property color line: alpha(theme.foreground, 0.14)       // borders, dividers
  readonly property color hover: alpha(theme.foreground, 0.06)      // hover fill
  readonly property color selected: alpha(theme.foreground, 0.10)   // selected fill
  readonly property color surface: mix(theme.background, theme.foreground, 0.035) // raised panels

  readonly property int radiusSmall: theme.space(6)
  readonly property int radius: theme.space(10)

  readonly property real spacingScale: Math.max(0.1, Number(pick("spacingScale", 1)) || 1)

  // Design pixels to real pixels, so layouts keep their proportions when the
  // host makes everything denser or roomier.
  function space(px) {
    var n = Number(px) * theme.spacingScale
    if (!isFinite(n) || n <= 0) return 0
    return Math.max(1, Math.round(n))
  }

  // Typed (not a bare QtObject) so tooling sees `Theme.font.body` and co.
  component FontScale: QtObject {
    readonly property string family: theme.pick("fontFamily", "monospace")
    readonly property int caption: theme.pick("fontCaption", 10)
    readonly property int bodySmall: theme.pick("fontBodySmall", 11)
    readonly property int body: theme.pick("fontBody", 12)
    readonly property int heading: theme.pick("fontHeading", 16)
    readonly property int display: theme.pick("fontDisplay", 24)
  }
  readonly property FontScale font: FontScale {}
}
