import QtQuick
// The primitives are loaded by URL, but the import makes their directory
// known: Quickshell resolves their `PrimBase` base type only through it.
// qmllint disable unused-imports
import "prims"
// qmllint enable unused-imports
import "prims/Tree.js" as Tree
import ".."

// Renders one node of a view tree (docs/architecture.md, "Primitive tree
// (v1)") and, recursively through its container primitives, everything under
// it. Give it a `view` message's `tree`; it emits `action(actionId, value)`
// when a node with an `action` is pressed or changed, and the client forwards
// that as an `event`. A node that is unknown or invalid draws as an error row
// (prims/ErrorRow.qml), never a crash. `visible`, `tooltip` and `fill` are
// honoured here for every primitive; `fill` takes effect when the ViewTree
// sits in a container's layout, which sets it per child.
//
//   ViewTree { tree: msg.tree; onAction: (id, value) => daemon.event(id, value) }
Item {
  id: root

  // The node to draw (a JS object). null: nothing.
  property var tree: null
  // Whether the parent lays this node out in a row (set by the containers).
  property bool inRow: true
  // How deep this node is; the containers count it for the depth limit.
  property int depth: 0
  // Drawn on an accent fill (a meter's child): `foreground` reads as the
  // background token there, as the Voice disc's glyph does.
  property bool onAccent: false

  signal action(string actionId, var value)

  readonly property bool empty: root.tree === null || root.tree === undefined
  readonly property string problem: root.empty ? "" : Tree.check(root.tree, root.depth)
  // What is drawn: the tree, or the error row standing in for it.
  readonly property var node: root.problem === "" ? (root.tree ?? ({})) : ({ t: "error", message: root.problem })
  readonly property string kind: root.empty ? "" : root.problem === "" ? Tree.file(root.node.t) : "ErrorRow"
  readonly property string tip: typeof root.node.tooltip === "string" ? root.node.tooltip : ""
  readonly property bool clickable: !root.empty && root.problem === "" && typeof root.node.action === "string"
    && root.node.action !== "" && !Tree.selfActing(root.node.t)

  implicitWidth: primLoader.implicitWidth
  implicitHeight: primLoader.implicitHeight
  visible: root.kind !== "" && root.node.visible !== false

  // Under the primitive, so a button inside a clickable row still gets its own click.
  MouseArea {
    anchors.fill: parent
    enabled: root.clickable
    cursorShape: root.clickable ? Qt.PointingHandCursor : Qt.ArrowCursor
    onClicked: root.action(String(root.node.action), null)
  }

  Loader {
    id: primLoader
    anchors.fill: parent
    source: root.kind === "" ? "" : "prims/" + root.kind + ".qml"
  }

  Binding { target: primLoader.item; property: "node"; value: root.node; when: primLoader.item !== null }
  Binding { target: primLoader.item; property: "inRow"; value: root.inRow; when: primLoader.item !== null }
  Binding { target: primLoader.item; property: "onAccent"; value: root.onAccent; when: primLoader.item !== null }
  Binding { target: primLoader.item; property: "depth"; value: root.depth; when: primLoader.item !== null }
  Connections {
    target: primLoader.item
    ignoreUnknownSignals: true
    function onFired(actionId, value) { root.action(actionId, value) }
  }

  // The tooltip after the pointer rests on the node, drawn over the window so
  // no parent clips it.
  property bool tipShown: false
  HoverHandler {
    id: hover
    enabled: root.tip !== ""
    onHoveredChanged: if (!hover.hovered) root.tipShown = false
  }
  Timer {
    interval: 600
    running: hover.hovered && root.tip !== ""
    onTriggered: root.tipShown = true
  }
  Loader {
    active: root.tipShown && root.tip !== "" && root.Window.window !== null
    sourceComponent: Rectangle {
      id: bubble
      parent: root.Window.window.contentItem
      readonly property point spot: root.mapToItem(bubble.parent, 0, root.height + Theme.space(4))
      x: Math.max(0, Math.min(bubble.parent.width - width, spot.x))
      y: spot.y
      z: 1000
      width: tipText.implicitWidth + Theme.space(12)
      height: tipText.implicitHeight + Theme.space(8)
      radius: Theme.radiusSmall
      color: Theme.surface
      border.width: 1
      border.color: Theme.line
      Text {
        id: tipText
        anchors.centerIn: parent
        textFormat: Text.PlainText
        text: root.tip
        color: Theme.foreground
        font.family: Theme.font.family
        font.pixelSize: Theme.font.caption
      }
    }
  }
}
