import QtQuick
import QtQuick.Layouts
import ".."

// Vertical layout. A child with `fill: true` takes the rest of the height.
// Cross axis: without an `align` the rows, columns, lists and progress bars
// take the column's width (the media card's time row and bar span it) and the
// rest keep their size at the start; with `align` (start, center, end) they
// all keep their size there. Text always takes the width, so a long line
// elides instead of pushing the column wider.
PrimBase {
  id: col
  function stretches(child) {
    if (child === null || child === undefined) return false
    return child.t === "text" || (col.node.align === undefined && ["row", "column", "list", "progress"].indexOf(child.t) >= 0)
  }
  readonly property int cross: col.node.align === "center" ? Qt.AlignHCenter : col.node.align === "end" ? Qt.AlignRight : Qt.AlignLeft
  implicitWidth: lay.implicitWidth
  implicitHeight: lay.implicitHeight

  ColumnLayout {
    id: lay
    anchors.fill: parent
    spacing: col.px(col.node.gap, 8)
    Repeater {
      model: col.node.children
      delegate: ViewTree {
        required property var modelData
        tree: modelData
        inRow: false
        depth: col.depth + 1
        Layout.fillHeight: modelData !== null && modelData !== undefined && modelData.fill === true
        Layout.fillWidth: col.stretches(modelData)
        Layout.alignment: col.cross
        onAction: (actionId, value) => col.fired(actionId, value)
      }
    }
  }
}
