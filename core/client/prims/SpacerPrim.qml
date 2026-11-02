import QtQuick

// A fixed gap of `size` along the parent's axis, or (`fill: true`, handled by
// the container) a flexible one.
PrimBase {
  id: spacer
  implicitWidth: spacer.inRow ? spacer.px(spacer.node.size, 0) : 0
  implicitHeight: spacer.inRow ? 0 : spacer.px(spacer.node.size, 0)
}
