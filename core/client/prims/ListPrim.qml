import QtQuick
import QtQuick.Layouts
import "../.."
import ".."

// A scrolling column, as tall as its content up to `maxHeight`. `stick:
// "end"` keeps the newest line in view until the user scrolls up (the
// transcript); scrolling back to the end sticks again.
PrimBase {
  id: list
  readonly property bool stickEnd: list.node.stick === "end"
  readonly property real maxHeight: list.node.maxHeight === undefined || list.node.maxHeight === null
    ? Number.POSITIVE_INFINITY : Theme.space(list.num(list.node.maxHeight, 0))
  property bool pinned: true
  implicitWidth: lay.implicitWidth
  implicitHeight: Math.min(lay.implicitHeight, list.maxHeight)

  function toEnd() { flick.contentY = Math.max(0, flick.contentHeight - flick.height) }

  Flickable {
    id: flick
    anchors.fill: parent
    clip: true
    contentWidth: width
    contentHeight: lay.implicitHeight
    boundsBehavior: Flickable.StopAtBounds
    interactive: contentHeight > height
    onContentHeightChanged: if (list.stickEnd && list.pinned) list.toEnd()
    onHeightChanged: if (list.stickEnd && list.pinned) list.toEnd()
    onMovementEnded: list.pinned = flick.atYEnd

    ColumnLayout {
      id: lay
      width: flick.width
      spacing: list.px(list.node.gap, 8)
      Repeater {
        model: list.node.children
        delegate: ViewTree {
          required property var modelData
          tree: modelData
          inRow: false
          depth: list.depth + 1
          Layout.fillWidth: modelData !== null && modelData !== undefined && ["text", "row", "column", "progress"].indexOf(modelData.t) >= 0
          Layout.alignment: Qt.AlignLeft
          onAction: (actionId, value) => list.fired(actionId, value)
        }
      }
    }
  }
}
