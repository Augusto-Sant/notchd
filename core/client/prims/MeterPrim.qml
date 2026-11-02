import QtQuick
import "../.."
import ".."

// The Voice disc (native/notchd-modules/src/voice.rs): a 96 px accent disc with a line
// border that swells with the level, `1 + value * .35` over 90 ms, and its
// `child` (the microphone glyph) centered inside. Plain rectangles, so it
// needs no shader. It reserves the room of the fully swollen disc, so the
// swell never covers its neighbors.
PrimBase {
  id: prim
  readonly property real level: typeof prim.node.value === "number" ? prim.clamp01(prim.node.value) : 0
  readonly property real disc: prim.px(prim.node.size, 96)
  implicitWidth: Math.ceil(prim.disc * 1.35)
  implicitHeight: implicitWidth

  Rectangle {
    anchors.centerIn: parent
    width: prim.disc
    height: width
    radius: width / 2
    scale: 1 + prim.level * 0.35
    Behavior on scale { NumberAnimation { duration: 90 } }
    color: Theme.accent
    border.width: 1
    border.color: Theme.line
    ViewTree {
      anchors.centerIn: parent
      tree: prim.node.child === undefined ? null : prim.node.child
      depth: prim.depth + 1
      onAccent: true
      onAction: (actionId, value) => prim.fired(actionId, value)
    }
  }
}
