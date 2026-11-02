import QtQuick
import QtQuick.Layouts
import ".."

// Horizontal layout. `align` is the cross axis (default: centered, as the
// modules' rows are); a child with `fill: true` takes the rest of the width.
PrimBase {
  id: row
  readonly property int cross: row.node.align === "start" ? Qt.AlignTop : row.node.align === "end" ? Qt.AlignBottom : Qt.AlignVCenter
  implicitWidth: lay.implicitWidth
  implicitHeight: lay.implicitHeight

  RowLayout {
    id: lay
    anchors.fill: parent
    spacing: row.px(row.node.gap, 8)
    Repeater {
      model: row.node.children
      delegate: ViewTree {
        required property var modelData
        tree: modelData
        inRow: true
        depth: row.depth + 1
        Layout.fillWidth: modelData !== null && modelData !== undefined && modelData.fill === true
        Layout.alignment: row.cross
        onAction: (actionId, value) => row.fired(actionId, value)
      }
    }
  }
}
