import QtQuick
import "../.."

// The media module's fake equalizer: bars wander while `active` and settle
// low when not. A fixed-size box, so the bars moving never resize the notch.
// Smooth (the default): one timer eases every bar toward its target 60 times
// a second, with new targets about six times a second. Light: the bars jump
// to new levels about six times a second. Hidden, it stops. Plain rectangles,
// so it has no shader and needs no software fallback.
PrimBase {
  id: eq
  readonly property int count: Math.max(1, Math.round(eq.num(eq.node.bars, 4)))
  readonly property real barHeight: eq.px(eq.node.height, 14)
  readonly property bool smooth: eq.node.mode !== "light"
  readonly property bool active: eq.node.active === true
  readonly property real rest: 0.2
  property var levels: []
  property var targets: []
  property int ticks: 0
  property bool settled: true

  implicitWidth: bars.implicitWidth
  implicitHeight: barHeight

  function fill(v) {
    var a = []
    for (var i = 0; i < eq.count; i++) a.push(v)
    return a
  }
  // Active from the start: begin mid-motion rather than flat.
  function seed() {
    eq.levels = eq.fill(eq.rest)
    if (eq.active) eq.levels = eq.levels.map(function() { return 0.25 + Math.random() * 0.75 })
    eq.targets = eq.levels
  }
  onCountChanged: eq.seed()
  Component.onCompleted: eq.seed()

  function step() {
    var t = eq.targets
    if (!eq.active) t = eq.fill(eq.rest)
    else if (!eq.smooth || ++eq.ticks % 10 === 0)
      t = t.map(function(v) { return !eq.smooth || Math.random() < 0.6 ? 0.25 + Math.random() * 0.75 : v })
    eq.targets = t
    // Smooth closes ~23 % of the gap per 16 ms step: a ~60 ms time constant.
    eq.levels = eq.levels.map(function(v, i) { return eq.smooth ? v + (t[i] - v) * 0.23 : t[i] })
    eq.settled = !eq.active && eq.levels.every(function(v) { return Math.abs(v - eq.rest) < 0.01 })
  }
  Timer {
    interval: eq.smooth ? 16 : 160
    repeat: true
    running: eq.visible && (eq.active || !eq.settled)
    onTriggered: eq.step()
  }

  Row {
    id: bars
    anchors.bottom: parent.bottom
    spacing: Theme.space(2)
    Repeater {
      model: eq.levels.length
      delegate: Rectangle {
        required property int index
        anchors.bottom: parent.bottom
        width: Theme.space(3)
        radius: width / 2
        height: Math.max(width, eq.barHeight * eq.levels[index])
        color: Theme.accent
      }
    }
  }
}
