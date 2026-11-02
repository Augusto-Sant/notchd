import QtQuick
import "../.."

// A Nerd Font glyph, as the modules draw theirs: `size` in design pixels
// (default the heading size), `tone` instead of an alpha.
PrimBase {
  id: prim
  implicitWidth: glyph.implicitWidth
  implicitHeight: glyph.implicitHeight

  Text {
    id: glyph
    anchors.centerIn: parent
    textFormat: Text.PlainText
    text: prim.node.glyph === undefined ? "" : String(prim.node.glyph)
    color: prim.tone(prim.node.tone, "foreground")
    font.family: Theme.font.family
    font.pixelSize: prim.node.size === undefined || prim.node.size === null ? Theme.font.heading : prim.px(prim.node.size, 16)
  }
}
