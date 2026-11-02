import QtQuick
import "../.."

// A bar, as the media module draws its position: a 4 px track in a faint
// tone, the filled part in `tone`. With an `anchor` ({at, rate}) the bar
// interpolates `value + rate * (now - at)` itself, so a playing position needs
// no stream from the core. With a `seek` action, a press or drag fires it with
// the fraction. No `value`: no bar.
PrimBase {
  id: prim
  readonly property bool vertical: prim.node.orientation === "v"
  readonly property bool hasValue: typeof prim.node.value === "number"
  readonly property real thickness: prim.px(prim.node.thickness, 4)
  readonly property var anchorSpec: prim.node.anchor
  readonly property bool moving: prim.hasValue && prim.anchorSpec !== undefined && prim.anchorSpec !== null
    && prim.num(prim.anchorSpec.rate, 0) !== 0
  property real now: Date.now()
  readonly property real shown: {
    if (!prim.hasValue) return 0
    if (!prim.moving) return prim.clamp01(prim.node.value)
    return prim.clamp01(prim.node.value + prim.anchorSpec.rate * (prim.now - prim.anchorSpec.at) / 1000)
  }

  visible: prim.hasValue
  implicitWidth: prim.hasValue ? (prim.vertical ? prim.thickness : Theme.space(120)) : 0
  implicitHeight: prim.hasValue ? (prim.vertical ? Theme.space(120) : prim.thickness) : 0

  // The interpolation: no animation, a coarse timer, only while it moves.
  Timer {
    interval: 100
    repeat: true
    running: prim.moving && prim.visible
    triggeredOnStart: true
    onTriggered: prim.now = Date.now()
  }

  Rectangle {
    anchors.fill: parent
    radius: (prim.vertical ? width : height) / 2
    color: Theme.line
  }
  Rectangle {
    anchors.bottom: parent.bottom
    width: prim.vertical ? parent.width : parent.width * prim.shown
    height: prim.vertical ? parent.height * prim.shown : parent.height
    radius: (prim.vertical ? width : height) / 2
    color: prim.tone(prim.node.tone, "foreground")
  }

  // A seek hit area taller than the bar, so it is easy to grab.
  MouseArea {
    anchors.fill: parent
    anchors.margins: -Theme.space(5)
    enabled: typeof prim.node.seek === "string" && prim.node.seek !== "" && prim.hasValue
    cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
    function fraction(mouse) {
      var pad = Theme.space(5)
      return prim.clamp01(prim.vertical ? 1 - (mouse.y - pad) / prim.height : (mouse.x - pad) / prim.width)
    }
    onPressed: mouse => prim.fired(String(prim.node.seek), fraction(mouse))
    onPositionChanged: mouse => { if (pressed) prim.fired(String(prim.node.seek), fraction(mouse)) }
  }
}
