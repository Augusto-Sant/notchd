import QtQuick
import ".."

// `choice` field: one segment per option, the selected one raised.
Rectangle {
  id: control

  property var field: ({})
  property var value
  signal edited(var value)

  implicitWidth: segments.implicitWidth + Theme.space(8)
  implicitHeight: Theme.space(34)
  radius: Theme.radiusSmall + Theme.space(2)
  color: Theme.alpha(Theme.background, 0.5)
  border.width: 1
  border.color: Theme.line

  Row {
    id: segments
    anchors.centerIn: parent
    Repeater {
      model: control.field.options || []
      delegate: Rectangle {
        id: segment
        required property var modelData
        readonly property bool isSelected: control.value === modelData.value
        // An option that waits on something (a component to install) is shown, not pickable.
        readonly property bool off: modelData.disabled === true
        width: optionText.implicitWidth + Theme.space(28)
        height: control.height - Theme.space(8)
        radius: Theme.radiusSmall
        color: isSelected ? Theme.selected : (area.containsMouse && !segment.off ? Theme.hover : "transparent")
        border.width: isSelected ? 1 : 0
        border.color: Theme.line
        Label {
          id: optionText
          anchors.centerIn: parent
          text: segment.modelData.label || String(segment.modelData.value)
          wrapMode: Text.NoWrap
          font.pixelSize: Theme.font.bodySmall
          color: segment.isSelected ? Theme.foreground : (segment.off ? Theme.faint : Theme.muted)
        }
        MouseArea {
          id: area
          anchors.fill: parent
          hoverEnabled: true
          cursorShape: segment.off ? Qt.ArrowCursor : Qt.PointingHandCursor
          onClicked: if (!segment.off) control.edited(segment.modelData.value)
        }
      }
    }
  }
}
