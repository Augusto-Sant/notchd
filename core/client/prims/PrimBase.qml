import QtQuick
import "../.."

// The base of every primitive: the node it draws, and the token lookups, so
// no primitive names a color. A primitive fires `fired(actionId, value)`; the
// ViewTree around it forwards that as the client's `event`.
Item {
  id: prim

  // The tree node (a `view` message's subtree). Empty until the ViewTree
  // hands it over, so every primitive reads fields defensively.
  property var node: ({})
  // Whether the parent lays its children out in a row (a `spacer` needs it).
  property bool inRow: true
  property int depth: 0
  // On an accent fill (a meter's child): `foreground` becomes the background.
  property bool onAccent: false
  signal fired(string actionId, var value)

  // A `tone` of the protocol: foreground | muted | faint | accent | danger |
  // success | line.
  function tone(name, fallback) {
    var n = name === undefined || name === null ? fallback : name
    switch (n) {
    case "muted": return Theme.muted
    case "faint": return Theme.faint
    case "accent": return Theme.accent
    case "danger": return Theme.danger
    case "success": return Theme.success
    case "line": return Theme.line
    default: return prim.onAccent ? Theme.background : Theme.foreground
    }
  }

  // Any token of `welcome.tokens` (the canvas takes them all).
  function token(name) {
    switch (name) {
    case "background": return Theme.background
    case "hover": return Theme.hover
    case "selected": return Theme.selected
    case "surface": return Theme.surface
    default: return prim.tone(name, "foreground")
    }
  }

  // A canvas color: a token, or {"mix": [a, b, t]}.
  function color(spec) {
    if (typeof spec === "string") return prim.token(spec)
    if (spec && spec.mix && spec.mix.length >= 3)
      return Theme.mix(prim.token(spec.mix[0]), prim.token(spec.mix[1]), Number(spec.mix[2]))
    return Theme.foreground
  }

  // A text size of the protocol, in pixels.
  function fontPx(name, fallback) {
    switch (name === undefined || name === null ? fallback : name) {
    case "caption": return Theme.font.caption
    case "bodySmall": return Theme.font.bodySmall
    case "heading": return Theme.font.heading
    case "display": return Theme.font.display
    default: return Theme.font.body
    }
  }

  // Design pixels (a number in the node, or `fallback`) to real ones.
  function px(v, fallback) {
    return Theme.space(typeof v === "number" && isFinite(v) ? v : fallback)
  }

  function num(v, fallback) { return typeof v === "number" && isFinite(v) ? v : fallback }
  function clamp01(v) { return Math.max(0, Math.min(1, v)) }
}
