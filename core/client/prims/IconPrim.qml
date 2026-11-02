import QtQuick
import "../.."
import "Tree.js" as Tree

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
    text: Tree.glyphOf(prim.node)
    color: prim.tone(prim.node.tone, "foreground")
    font.family: Theme.font.family
    font.pixelSize: typeof prim.node.size === "string" ? prim.fontPx(prim.node.size, "heading") : prim.node.size === undefined || prim.node.size === null ? Theme.font.heading : prim.px(prim.node.size, 16)
  }
}
