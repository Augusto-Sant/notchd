import QtQuick
import "../.."
import "Tree.js" as Tree

// The modules' `IconButton` (a 28 px round hover fill around a heading-size
// glyph, dimmed when disabled; `big`: 36 px around a larger glyph) and, with a `label` or `variant: "outline"`,
// the timer's bordered button (the `Chip`'s shape). `selected` keeps the
// `selected` fill; the press fires `action` with a null value.
PrimBase {
  id: prim
  readonly property bool hasLabel: typeof prim.node.label === "string" && prim.node.label !== ""
  readonly property bool outline: prim.node.variant === "outline"
  readonly property bool round: !prim.hasLabel && !prim.outline
  readonly property bool usable: prim.node.enabled !== false
  readonly property bool live: prim.usable && typeof prim.node.action === "string" && prim.node.action !== ""

  readonly property bool big: prim.round && prim.node.big === true
  implicitWidth: prim.round ? Theme.space(prim.big ? 36 : 28) : content.implicitWidth + Theme.space(20)
  implicitHeight: prim.round ? Theme.space(prim.big ? 36 : 28) : Theme.space(32)
  opacity: prim.usable ? 1 : 0.35

  Rectangle {
    anchors.fill: parent
    radius: prim.round ? width / 2 : Theme.radiusSmall
    color: prim.node.selected === true || mouse.containsMouse && prim.live ? Theme.selected : "transparent"
    border.width: prim.outline ? 1 : 0
    border.color: prim.node.selected === true ? Theme.accent : Theme.line
  }
  Row {
    id: content
    anchors.centerIn: parent
    spacing: Theme.space(6)
    Text {
      visible: Tree.glyphOf(prim.node) !== ""
      anchors.verticalCenter: parent.verticalCenter
      textFormat: Text.PlainText
      text: Tree.glyphOf(prim.node)
      color: prim.tone(prim.node.tone, prim.hasLabel ? "accent" : "foreground")
      font.family: Theme.font.family
      font.pixelSize: prim.hasLabel ? Theme.font.bodySmall : prim.big ? Theme.font.heading * 1.25 : Theme.font.heading
    }
    Text {
      visible: prim.hasLabel
      anchors.verticalCenter: parent.verticalCenter
      textFormat: Text.PlainText
      text: prim.hasLabel ? prim.node.label : ""
      color: Tree.glyphOf(prim.node) !== "" ? Theme.foreground : prim.tone(prim.node.tone, "foreground")
      font.family: Theme.font.family
      font.pixelSize: Theme.font.bodySmall
    }
  }
  MouseArea {
    id: mouse
    anchors.fill: parent
    enabled: prim.live
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onClicked: prim.fired(String(prim.node.action), null)
  }
}
