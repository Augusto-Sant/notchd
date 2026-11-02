import QtQuick
import Quickshell
import Quickshell.Wayland
import "State.js" as State
import "Position.js" as Position
import "Navigate.js" as Navigate

// Puts the notches on screen: a NotchWindow for every copy of every notch
// (NotchService.copies), which places it and routes its pointer, plus the
// space the closed notches keep clear of windows. Notches on one edge of a
// screen go side by side, never on each other (Position.pack): each as
// near its offset as the others let it, ones that come close joined flush
// into one shape, and an open one hides those it grows over. A standalone
// module's card stays out of that: it shows in its own place, over the
// notches there, which hide until it goes.
//
// With reserveSpace on, an empty window per screen edge holds a layer-shell
// exclusive zone as deep as the deepest closed notch there (ReservedSpace,
// State.reserveZones), so windows tile clear of them and two notches on one
// edge don't add up. The compositor stacks it inside the host bar's own
// zone, where the notch hangs.
Item {
  id: root

  // ------------------------------------------------------------------ host
  property var svc: null
  // The host's bar, so a notch hangs off it when they share an edge and
  // keeps clear of it otherwise: its thickness (barHeight; 0 = no bar) and
  // whether it is hidden. Its edge is the service's `barEdge`.
  property int barHeight: 0
  property bool barHidden: false
  property string layerNamespace: "notchd"

  // "notchd not running" while the daemon is away.
  DaemonDown {
    svc: root.svc
    barHeight: root.barHeight
  }

  // The window of each copy, in the service's order.
  readonly property var windows: windowVariants.instances

  Variants {
    id: windowVariants
    model: root.svc ? root.svc.copies : []
    NotchWindow {
      required property var modelData
      copy: modelData
      barHeight: root.barHeight
      barHidden: root.barHidden
      layerNamespace: root.layerNamespace
      readonly property var packed: root.packOf(modelData)
      packShift: packed ? packed.shift : 0
      covered: (!!packed && packed.covered) || root.coveredByStandalone(modelData)
      joinStart: root.windowAt(packed ? packed.joinStart : -1)
      joinEnd: root.windowAt(packed ? packed.joinEnd : -1)
      rowDepth: root.rowDepthOf(modelData)
      cornerInset: root.cornerOf(modelData, root.cornerInsets)
      cornerSpan: root.cornerOf(modelData, root.cornerSpans)
    }
  }

  // ---------------------------------------------------------- side by side
  // Notches closer than this join flush into one shape: two shoulders,
  // which flare past the card, and a little air would not fit between them.
  readonly property real packGap: 2 * Theme.space(10) + Theme.space(8)
  readonly property var packing: Position.pack(root.windows.map(function(w) { return w.packItem }), root.packGap)
  // A joined row as deep as its deepest notch closed; an open one, deeper,
  // steps down to its neighbors and keeps the row as it was.
  readonly property var rowDepths: Position.rowDepths(root.packing,
    root.windows.map(function(w) { return w.closedDepth }))
  function rowDepthOf(copy) {
    var list = root.windows
    for (var i = 0; i < list.length; i++) if (list[i].copy === copy) return root.rowDepths[i] || 0
    return 0
  }
  // Corners (Position.corners), in two halves so no window's place feeds
  // back into itself: where side notches start, from the top and bottom
  // ones flush in the corners; and where those meet the side notches, from
  // the side ones.
  readonly property var cornerInsets: Position.corners(root.windows.map(function(w) { return Position.cornerInput(w, false) }))
  readonly property var cornerSpans: Position.corners(root.windows.map(function(w) { return Position.cornerInput(w, true) }))
  function cornerOf(copy, list) {
    var ws = root.windows
    for (var i = 0; i < ws.length; i++) if (ws[i].copy === copy) return list[i] || { start: 0, end: 0 }
    return { start: 0, end: 0 }
  }
  function windowAt(index) {
    return index >= 0 && index < root.windows.length ? root.windows[index] : null
  }
  function packOf(copy) {
    var list = root.windows
    for (var i = 0; i < list.length; i++) if (list[i].copy === copy) return root.packing[i] || null
    return null
  }

  // A standalone module's card (the OSD) keeps its own place and draws over
  // the notches it overlaps, which hide while it shows.
  readonly property var standaloneCovers: Position.coveredByStandalone(root.windows.map(function(w) { return w.coverItem }))
  function coveredByStandalone(copy) {
    var list = root.windows
    for (var i = 0; i < list.length; i++) if (list[i].copy === copy) return !!root.standaloneCovers[i]
    return false
  }

  // ------------------------------------------------------- reserved space
  // { "<screen>|<edge>": px } over every copy's reserve.
  readonly property var zones: State.reserveZones(root.windows.map(function(w) { return w.reserve }))

  // A mapped layer surface does not take new anchors (the strip stayed on
  // the old edge), so each window is one screen edge, and a notch moving to
  // another edge makes a new one. Its depth follows the zone.
  Variants {
    model: Object.keys(root.zones)
    PanelWindow {
      id: strip
      required property var modelData
      readonly property string screenName: modelData.split("|")[0]
      readonly property string edge: modelData.split("|")[1]
      readonly property int space: root.zones[modelData] || 0
      readonly property var targetScreen: {
        var screens = Quickshell.screens || []
        for (var i = 0; i < screens.length; i++) if (screens[i].name === strip.screenName) return screens[i]
        return null
      }
      screen: strip.targetScreen
      // Back after its monitor goes away and comes back (KeepShown).
      readonly property KeepShown keeper: KeepShown {
        target: strip
        screen: strip.targetScreen
      }
      visible: strip.keeper.shown
      anchors {
        top: strip.edge !== "bottom"
        bottom: strip.edge !== "top"
        left: strip.edge !== "right"
        right: strip.edge !== "left"
      }
      readonly property bool vertical: strip.edge === "left" || strip.edge === "right"
      implicitWidth: strip.vertical ? strip.space : 0
      implicitHeight: strip.vertical ? 0 : strip.space
      color: "transparent"
      exclusionMode: ExclusionMode.Normal
      exclusiveZone: strip.space
      // Overlay, so it is arranged after the bar (top layer) and stacks
      // inside it rather than pushing the bar away.
      WlrLayershell.namespace: root.layerNamespace + "-reserve"
      WlrLayershell.layer: WlrLayer.Overlay
      WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
      // Draws nothing and takes no input.
      mask: Region {}
    }
  }

  // ------------------------------------------------------------ navigation
  // While navigating (NotchService.navigating), a window on the focused
  // screen holds the keyboard, exclusively, and hands each key to the
  // service (Navigate.keyName, NotchService.navKey). It draws nothing and
  // takes no pointer: the notches show the highlight. A window taking the
  // keyboard from it ends navigation, as Escape does.
  LazyLoader {
    id: navLoader
    readonly property var owner: root.svc
    active: !!root.svc && root.svc.navigating
    PanelWindow {
      id: navWindow
      readonly property string focusedName: navLoader.owner && navLoader.owner.compositor
        ? navLoader.owner.compositor.focusedScreenName : ""
      screen: {
        var screens = Quickshell.screens || []
        for (var i = 0; i < screens.length; i++) if (screens[i].name === navWindow.focusedName) return screens[i]
        return screens.length > 0 ? screens[0] : null
      }
      anchors { top: true; left: true }
      implicitWidth: 1
      implicitHeight: 1
      color: "transparent"
      exclusionMode: ExclusionMode.Ignore
      WlrLayershell.namespace: root.layerNamespace + "-navigate"
      WlrLayershell.layer: WlrLayer.Overlay
      WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
      mask: Region {}

      property bool held: false
      Item {
        anchors.fill: parent
        focus: true
        readonly property bool active: Window.active
        onActiveChanged: {
          if (active) navWindow.held = true
          else if (navWindow.held && navLoader.owner) navLoader.owner.endNavigation(true)
        }
        Keys.onPressed: function(event) {
          var key = Navigate.keyName(event.key, event.modifiers)
          var service = navLoader.owner
          if (key === "" || !service) return
          event.accepted = true
          // Later: Enter and Escape end navigation, which takes this window
          // away under its own key handler.
          Qt.callLater(function() { service.navKey(key) })
        }
      }
    }
  }
}
