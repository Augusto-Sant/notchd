import QtQuick
import "../.."

// What an invalid or unknown node becomes: a visible row, never a crash.
PrimBase {
  id: err
  implicitWidth: label.implicitWidth + Theme.space(12)
  implicitHeight: label.implicitHeight + Theme.space(6)
  Rectangle {
    anchors.fill: parent
    radius: Theme.radiusSmall
    color: Theme.hover
    border.width: 1
    border.color: Theme.danger
  }
  Text {
    id: label
    anchors.left: parent.left
    anchors.leftMargin: Theme.space(6)
    anchors.verticalCenter: parent.verticalCenter
    textFormat: Text.PlainText
    text: "󰀦 " + (err.node.message || "invalid node")
    color: Theme.danger
    font.family: Theme.font.family
    font.pixelSize: Theme.font.caption
  }
}
