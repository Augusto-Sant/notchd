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
    // The count, not the list: a new list of the same shape (every `view` is one)
    // must not remake the children, or a terminal in one would start over.
    model: stack.node.children ? stack.node.children.length : 0
    delegate: ViewTree {
      required property int index
      readonly property var child: stack.node.children[index]
      tree: child
      depth: stack.depth + 1
      x: stack.start ? 0 : stack.end ? stack.width - width : (stack.width - width) / 2
      y: stack.start ? 0 : stack.end ? stack.height - height : (stack.height - height) / 2
      onAction: (actionId, value) => stack.fired(actionId, value)
    }
  }
}
