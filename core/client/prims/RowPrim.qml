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
      // The count, not the list: a new list of the same shape (every `view` is one)
      // must not remake the children, or a terminal in one would start over.
      model: row.node.children ? row.node.children.length : 0
      delegate: ViewTree {
        required property int index
        readonly property var child: row.node.children[index]
        tree: child
        inRow: true
        depth: row.depth + 1
        Layout.fillWidth: child !== null && child !== undefined && child.fill === true
        Layout.alignment: row.cross
        onAction: (actionId, value) => row.fired(actionId, value)
      }
    }
  }
}
