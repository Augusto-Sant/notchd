import QtQuick
import "../.."
import "Tree.js" as Tree

// The free surface (tier 3): a rectangle the UI hands to something that draws
// itself. A `terminal` is the view of a program the daemon runs and emulates
// (docs/architecture.md, "Free surface"): terminal/GhosttySurface.qml draws
// the screen the daemon sends. A surface this UI cannot draw (a webview) is
// a placeholder that says so, so a module's view is never a silent blank.
//
// What it needs of the daemon (the socket, the binary, the settings) it asks
// of `surfaceHost`, a property of the ViewTree a module's slot is drawn in
// (CoreModule's SlotView sets it to the service); without one it is the
// placeholder.
PrimBase {
  id: prim
  implicitWidth: prim.px(prim.node.w, 240)
  implicitHeight: prim.px(prim.node.h, 120)

  readonly property bool isTerminal: prim.node.kind === "terminal"
  property var host: null
  Component.onCompleted: prim.host = Tree.hostOf(prim)
  readonly property bool drawable: prim.isTerminal && !!prim.host && prim.host.terminalAvailable === true
  readonly property string reference: typeof prim.node.ref === "string" ? prim.node.ref : ""

  Loader {
    id: surface
    anchors.fill: parent
    active: prim.drawable
    source: prim.drawable ? Qt.resolvedUrl("../../terminal/GhosttySurface.qml") : ""
  }
  Binding { target: surface.item; property: "host"; value: prim.host; when: surface.item !== null }
  Binding { target: surface.item; property: "reference"; value: prim.reference; when: surface.item !== null }
  // A new run of the program (the surface's `id` changed) is a new view.
  Binding { target: surface.item; property: "run"; value: typeof prim.node.id === "string" ? prim.node.id : ""; when: surface.item !== null }
  // What the surface says goes up as an action of the module's view.
  Connections {
    target: surface.item
    ignoreUnknownSignals: true
    function onFired(actionId, value) { prim.fired(actionId, value) }
  }

  // Not drawable here: what is missing.
  Rectangle {
    visible: !prim.drawable
    anchors.fill: parent
    radius: Theme.radius
    color: Theme.surface
    border.width: 1
    border.color: Theme.line
  }
  Text {
    visible: !prim.drawable
    anchors.centerIn: parent
    width: parent.width - Theme.space(24)
    horizontalAlignment: Text.AlignHCenter
    wrapMode: Text.Wrap
    textFormat: Text.PlainText
    text: prim.isTerminal && !!prim.host
      ? "This UI does not draw terminals."
      : "󰆍 " + (prim.node.kind === undefined ? "" : prim.node.kind) + " surface\n" + prim.reference
    color: Theme.muted
    font.family: Theme.font.family
    font.pixelSize: prim.isTerminal ? Theme.font.body : Theme.font.caption
  }
}
