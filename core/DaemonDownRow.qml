import QtQuick

// What the notch says while the daemon is away: one small row in the look of
// a module's error row (client/prims/ErrorRow.qml), on the notch's own
// background so it reads over any wallpaper. DaemonDown puts it on screen;
// the visual scenarios draw it as it is.
Item {
  id: row

  // The text after the warning glyph.
  property string message: "notchd not running"

  implicitWidth: label.implicitWidth + Theme.space(20)
  implicitHeight: label.implicitHeight + Theme.space(12)

  Rectangle {
    anchors.fill: parent
    radius: Theme.radiusSmall
    color: Theme.background
    border.width: 1
    border.color: Theme.danger
  }
  Rectangle {
    anchors.fill: parent
    radius: Theme.radiusSmall
    color: Theme.hover
  }
  Text {
    id: label
    anchors.centerIn: parent
    textFormat: Text.PlainText
    text: "󰀦 " + row.message
    color: Theme.danger
    font.family: Theme.font.family
    font.pixelSize: Theme.font.caption
  }
}
