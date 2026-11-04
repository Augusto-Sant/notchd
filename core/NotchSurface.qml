import QtQuick
import Quickshell
import Quickshell.Wayland
import "Position.js" as Position
import "client/Protocol.js" as Protocol

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
// exclusive zone as deep as the deepest closed notch there (the daemon's
// `state.reserve`), so windows tile clear of them and two notches on one
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
      readonly property var packed: layout.packOf(modelData)
      packShift: packed ? packed.shift : 0
      covered: (!!packed && packed.covered) || layout.coveredByStandalone(modelData)
      joinStart: layout.windowAt(packed ? packed.joinStart : -1)
      joinEnd: layout.windowAt(packed ? packed.joinEnd : -1)
      rowDepth: layout.rowDepthOf(modelData)
      cornerInset: layout.cornerInsetOf(modelData)
      cornerSpan: layout.cornerSpanOf(modelData)
    }
  }

  // Side by side, the corners and what a standalone card covers.
  NotchLayout {
    id: layout
    windows: root.windows
  }

  // ------------------------------------------------------- reserved space
  // { "<screen>|<edge>": px }: what the daemon says to keep clear, the
  // deepest closed notch on each edge of each screen. Held as it was while
  // the daemon is away.
  readonly property var zones: root.svc ? root.svc.reserveZones : ({})

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
  // service (Protocol.wireKey, NotchService.navKey), the daemon deciding what
  // it does. It draws nothing and
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
          var key = Protocol.wireKey(event.key, event.modifiers)
          var service = navLoader.owner
          if (key.key === "" || !service) return
          event.accepted = true
          // Later: Enter and Escape end navigation, which takes this window
          // away under its own key handler.
          Qt.callLater(function() { service.navKey(key.key, key.mods) })
        }
      }
    }
  }
}
