import QtQuick
import "../.."

// Plain text (never rich). The modules' texts: `size` from the five font
// sizes, `tone` instead of an alpha, `elide: "end"` by default so a line in a
// narrow cell is cut with an ellipsis, `maxWidth` to cap the natural width.
PrimBase {
  id: prim
  readonly property real cap: prim.node.maxWidth === undefined || prim.node.maxWidth === null
    ? Number.POSITIVE_INFINITY : Theme.space(prim.num(prim.node.maxWidth, 0))
  readonly property bool wraps: prim.node.wrap === true
  implicitWidth: Math.min(label.implicitWidth, prim.cap)
  implicitHeight: label.implicitHeight

  Text {
    id: label
    width: parent.width
    textFormat: Text.PlainText
    text: prim.node.text === undefined ? "" : String(prim.node.text)
    color: prim.tone(prim.node.tone, "foreground")
    font.family: Theme.font.family
    font.pixelSize: prim.fontPx(prim.node.size, "body")
    font.features: prim.node.tabular === true ? ({ "tnum": 1 }) : ({})
    elide: prim.node.elide === "none" ? Text.ElideNone : Text.ElideRight
    wrapMode: prim.wraps ? Text.Wrap : Text.NoWrap
    maximumLineCount: prim.num(prim.node.lines, 0) > 0 ? prim.node.lines : (prim.wraps ? 100000 : 1)
    horizontalAlignment: prim.node.align === "end" ? Text.AlignRight : prim.node.align === "center" ? Text.AlignHCenter : Text.AlignLeft
  }
}
