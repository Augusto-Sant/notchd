import QtQuick
import ".."

// Overlay: every child on top of the others, centered by default (the Voice
// disc and its glyph). The size is the largest child's.
PrimBase {
  id: stack
  readonly property bool start: stack.node.align === "start"
  readonly property bool end: stack.node.align === "end"
  implicitWidth: {
    var m = 0
    for (var i = 0; i < layers.count; i++) m = Math.max(m, (layers.itemAt(i) as Item).implicitWidth)
    return m
  }
  implicitHeight: {
    var m = 0
    for (var i = 0; i < layers.count; i++) m = Math.max(m, (layers.itemAt(i) as Item).implicitHeight)
    return m
  }

  Repeater {
    id: layers
    model: stack.node.children
    delegate: ViewTree {
      required property var modelData
      tree: modelData
      depth: stack.depth + 1
      x: stack.start ? 0 : stack.end ? stack.width - width : (stack.width - width) / 2
      y: stack.start ? 0 : stack.end ? stack.height - height : (stack.height - height) / 2
      onAction: (actionId, value) => stack.fired(actionId, value)
    }
  }
}
