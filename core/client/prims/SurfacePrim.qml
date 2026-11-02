import QtQuick
import "../.."

// The free surface (tier 3). The real surfaces (the terminal view, the
// webview) come with the client; until then this is a placeholder that names
// the surface, so a module's view is never a silent blank.
PrimBase {
  id: prim
  implicitWidth: prim.px(prim.node.w, 240)
  implicitHeight: prim.px(prim.node.h, 120)

  Rectangle {
    anchors.fill: parent
    radius: Theme.radiusSmall
    color: Theme.surface
    border.width: 1
    border.color: Theme.line
  }
  Text {
    anchors.centerIn: parent
    width: parent.width - Theme.space(12)
    horizontalAlignment: Text.AlignHCenter
    textFormat: Text.PlainText
    elide: Text.ElideMiddle
    text: "󰆍 " + (prim.node.kind === undefined ? "" : prim.node.kind) + " surface\n" + (prim.node.ref === undefined ? "" : prim.node.ref)
    color: Theme.muted
    font.family: Theme.font.family
    font.pixelSize: Theme.font.caption
  }
}
