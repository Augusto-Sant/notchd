import QtQuick
import "../.."

// The timer's preset `Chip`: a 32 px bordered pill with the label in the
// body-small size, a hover fill, and a `selected` state in the accent. The
// press fires `action` with `value` (a string; null without one).
PrimBase {
  id: prim
  readonly property bool live: typeof prim.node.action === "string" && prim.node.action !== ""
  implicitHeight: Theme.space(32)
  implicitWidth: label.implicitWidth + Theme.space(20)

  Rectangle {
    anchors.fill: parent
    radius: Theme.radiusSmall
    color: prim.node.selected === true ? Theme.selected : mouse.containsMouse && prim.live ? Theme.hover : "transparent"
    border.width: 1
    border.color: prim.node.selected === true ? Theme.accent : Theme.line
  }
  Text {
    id: label
    anchors.centerIn: parent
    textFormat: Text.PlainText
    text: prim.node.label === undefined ? "" : String(prim.node.label)
    color: Theme.foreground
    font.family: Theme.font.family
    font.pixelSize: Theme.font.bodySmall
  }
  MouseArea {
    id: mouse
    anchors.fill: parent
    enabled: prim.live
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onClicked: prim.fired(String(prim.node.action), prim.node.value === undefined ? null : prim.node.value)
  }
}
