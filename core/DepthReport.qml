import QtQuick

// What a notch card measures of itself, for the daemon. The daemon works out
// how much of its edge windows leave to the closed notch (reserveSpace,
// `state.reserve`: the rest lip, or the deepest compact view seen, never
// under the baseline; it grows at once and shrinks as the notch finishes
// collapsing), but only the card knows how deep it is: its depths go out as
// `measure` whenever one changes, and again to a daemon that came back.
// NotchSurface turns what the daemon answers into the space windows keep
// clear. A guest copy (the visitor, a standalone module) reserves nothing and
// reports nothing.
Item {
  id: report
  visible: false

  // The notch copy (NotchCopy) and its card (NotchCard).
  property var copy: null
  property NotchCard notchCard: null

  // A media compact view's height (18) plus the card's compact padding is
  // the least a compact view reserves. On a side edge compact views differ
  // too much to guess; the deepest seen counts.
  readonly property real compactBaseline: report.notchCard && !report.notchCard.vertical
    ? Theme.space(18) + report.notchCard.compactPadNear + report.notchCard.compactPadFar : 0
  readonly property string depths: JSON.stringify([
    report.notchCard ? report.notchCard.restDepth : 0, report.notchCard ? report.notchCard.compactDepth : 0, report.compactBaseline])
  onDepthsChanged: Qt.callLater(report.send)
  onCopyChanged: Qt.callLater(report.send)

  function send() {
    var c = report.copy
    if (!c || c.guest || !c.daemon) return
    var d = JSON.parse(report.depths)
    c.daemon.measure(c.wireNotch, c.fixedScreen, d[0], d[1], d[2])
  }
  // A daemon that came back knows no depth.
  Connections {
    target: report.copy ? report.copy.daemon : null
    function onWelcomed() { report.send() }
  }
  Component.onCompleted: report.send()
}
