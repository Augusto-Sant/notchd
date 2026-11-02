import QtQuick
import Quickshell
import Quickshell.Wayland
import "Position.js" as Position

// One notch copy (NotchCopy) on screen: a layer-shell overlay on the screen
// the copy names, the card hanging from the edge its notch's `position`
// names, at its offset along it (off the host's bar where they share an
// edge). Hidden over a fullscreen app and, closed, where its workspace
// doesn't take it; peeks and the open notch show regardless. The copy holds
// the daemon's state and NotchCard draws it; this places the card, routes the
// pointer and handles the drag.
//
// The window covers the whole screen on the overlay layer but its input mask
// is just the notch, so everything else stays click-through. What the closed
// notch keeps clear of windows (reserveSpace) is the daemon's to decide, from
// the depths this reports (`measure`); NotchSurface holds the space it says.
Scope {
  id: root

  property var copy: null
  // The host's bar, as NotchSurface has it.
  property int barHeight: 0
  property bool barHidden: false
  property string layerNamespace: "notchd"

  readonly property var notch: root.copy ? root.copy.owner : null
  // A standalone module's card: it keeps its own place, out of the packing,
  // and the notches it overlaps hide while it shows (`coverItem`).
  readonly property bool standalone: !!root.notch && root.notch.standaloneId !== ""
  readonly property var compositor: root.copy ? root.copy.compositor : null

  // What the notch shows (NotchCard.mode): the copy's mode.
  readonly property string mode: card.mode

  // -------------------------------------------------------------- geometry
  readonly property var targetScreen: {
    var screens = Quickshell.screens || []
    var name = root.copy ? root.copy.screenName : ""
    for (var i = 0; i < screens.length; i++)
      if (screens[i].name === name) return screens[i]
    return screens.length > 0 ? screens[0] : null
  }
  readonly property bool offWorkspace: !!root.copy && root.copy.offWorkspace
  readonly property bool fullscreenOnScreen: !!(root.compositor && root.targetScreen
    && root.compositor.hasFullscreen(root.targetScreen))

  // The bar hides over fullscreen apps, so the notch moves up with it.
  readonly property bool barVisible: !root.fullscreenOnScreen && !root.barHidden
  // Where the notch is shown: while dragged, where it would land. On another
  // screen (`dragScreen`), a preview there shows it instead of this card.
  property var dragPlacement: null
  property string dragScreen: ""
  readonly property bool dragAway: !!root.dragPlacement && root.dragScreen !== ""
    && root.dragScreen !== root.windowScreenName
  readonly property string windowScreenName: window.screen ? window.screen.name : ""
  readonly property var shownPlacement: root.dragPlacement
    || (root.copy ? root.copy.placement : { edge: "top", offset: 0.5 })
  readonly property string edge: card.edge
  // What the bar takes on each edge.
  readonly property var insets: Position.insets(root.copy && root.copy.svc ? root.copy.svc.barEdge : "top",
                                                root.barHeight, root.barVisible)

  // Over a fullscreen app, or with no screen on an allowed workspace, only
  // peeks and the open notch show; while navigating, every notch does.
  readonly property bool hidden: (root.fullscreenOnScreen || root.offWorkspace)
    && (root.mode === "rest" || root.mode === "compact") && !(root.copy && root.copy.navShown)
  // Also hidden while another notch on its edge is open over it, or a
  // standalone card shows over it (`covered`).
  readonly property bool concealed: root.hidden || (root.covered && !root.cardOpen)
  readonly property bool pinned: !!root.copy && root.copy.pinned

  // ------------------------------------------------------------ side by side
  // Notches on one edge of a screen never sit on each other: NotchSurface
  // packs them (Position.pack) from what each says here, and hands back how
  // far along the edge to move (`packShift`), whether an open one covers
  // this one (`covered`) and which ones it is joined to.
  property real packShift: 0
  property bool covered: false
  // The windows of the notches flush against this one's start (left or
  // top) and end, joined into one shape with it; null for none.
  property var joinStart: null
  property var joinEnd: null
  // The card's depth, which a joined neighbor steps down from or up to,
  // and its own closed, which sets a joined row's (Position.rowDepths).
  readonly property real cardDepth: card.depth
  readonly property real closedDepth: card.closedDepth
  // A joined row's depth, which this card takes at least.
  property real rowDepth: 0

  // ------------------------------------------------------------ corners
  // A notch on the top or bottom edge flush in a corner keeps it; one on
  // the side edge there starts where it ends (Position.corners). From
  // NotchSurface: `cornerInset`, how far in a side notch's edge starts at
  // each end (top, bottom); `cornerSpan`, how far along a top or bottom
  // notch's far side the side notch meets it (left, right).
  property var cornerInset: ({ start: 0, end: 0 })
  property var cornerSpan: ({ start: 0, end: 0 })
  // What it reports for that: where it is and whether it is flush in a
  // corner, how deep, and whether it shows.
  readonly property var cornerItem: ({
    screen: root.windowScreenName, edge: card.edge,
    atStart: !!card.spot.atStart, atEnd: !!card.spot.atEnd,
    depth: card.depth, shown: !root.concealed && !root.dragPlacement
  })
  // The bar's insets, and on a side edge the corners' too: where the card's
  // stretch runs. The pinned notch's click-away area and drags keep the
  // bar's alone.
  readonly property var edgeInsets: {
    var i = root.insets
    if (!card.vertical) return i
    return { top: i.top + root.cornerInset.start, bottom: i.bottom + root.cornerInset.end, left: i.left, right: i.right }
  }
  readonly property bool cardOpen: root.mode === "peek" || root.mode === "expanded"
  readonly property var stretch: Position.stretch({
    edge: card.edge, width: window.width, height: window.height, insets: root.edgeInsets
  })
  // Where its middle wants to be along the edge: the configured offset.
  readonly property real wishCenter: root.stretch.start + root.shownPlacement.offset * (root.stretch.end - root.stretch.start)
  // Its length as it is, an open one's as it grows: the packing pushes its
  // neighbors along with it, frame by frame.
  readonly property var packItem: ({
    group: root.windowScreenName + "|" + card.edge,
    center: root.wishCenter,
    size: card.along,
    start: root.stretch.start,
    end: root.stretch.end,
    snap: card.snap,
    open: root.cardOpen,
    skip: root.hidden || !!root.dragPlacement || root.windowScreenName === "" || root.standalone
  })
  // Its span along the edge where it shows, for Position.coveredByStandalone:
  // a standalone card that shows covers the notches it overlaps.
  readonly property var coverItem: {
    var middle = root.standalone ? root.wishCenter : root.wishCenter + root.packShift
    return {
      group: root.windowScreenName + "|" + card.edge,
      start: middle - card.along / 2,
      end: middle + card.along / 2,
      cover: root.standalone,
      shown: root.standalone && root.cardOpen && !root.hidden && root.windowScreenName !== ""
    }
  }
  // The offset the card is placed at: the configured one, moved by the
  // packing (never while dragged, when it follows the pointer).
  readonly property real shownOffset: {
    var span = root.stretch.end - root.stretch.start
    if (root.dragPlacement || span <= 0) return root.shownPlacement.offset
    return (root.wishCenter + root.packShift - root.stretch.start) / span
  }

  // ------------------------------------------------------- reserved space
  // The daemon works out what windows keep clear of the closed notch from the
  // depths this card measures; NotchSurface holds the space it says.
  DepthReport {
    copy: root.copy
    notchCard: card
  }

  // ---------------------------------------------------------------- window
  KeepShown {
    id: windowKeeper
    target: window
    wanted: !root.copy || root.copy.needsWindow
    screen: root.targetScreen
  }
  PanelWindow {
    id: window
    screen: root.targetScreen
    // Off while its copy has nothing to show (a closed guest): a hidden
    // window gives back its buffers. Back after its monitor goes away and
    // comes back (KeepShown).
    visible: windowKeeper.shown

    anchors { top: true; left: true; right: true; bottom: true }
    color: "transparent"
    exclusionMode: ExclusionMode.Ignore
    WlrLayershell.namespace: root.layerNamespace
    WlrLayershell.layer: WlrLayer.Overlay
    // The keyboard only while pinned open on a module you type into
    // (NotchCopy.keyboardFocus); otherwise typing stays with your windows.
    WlrLayershell.keyboardFocus: !root.copy ? WlrKeyboardFocus.None
      : root.copy.keyboardFocus === "exclusive" ? WlrKeyboardFocus.Exclusive
      : root.copy.keyboardFocus === "ondemand" ? WlrKeyboardFocus.OnDemand
      : WlrKeyboardFocus.None

    // Tells the copy when the window gains or loses the keyboard
    // (NotchCopy.windowKeyboard).
    Item {
      readonly property bool active: Window.active
      onActiveChanged: if (root.copy) root.copy.windowKeyboard(active)
    }

    mask: Region { item: hitArea }

    // The notch plus some slack, so the small rest lip is easy to hover:
    // at both ends along the edge and twice that on the open side.
    readonly property real slack: Theme.space(root.mode === "rest" ? 10 : 4)
    readonly property var zone: {
      var s = window.slack
      var x = card.x - (root.edge === "left" ? 0 : root.edge === "right" ? 2 * s : s)
      var y = card.y - (root.edge === "top" ? 0 : root.edge === "bottom" ? 2 * s : s)
      if (root.concealed) return { x: x, y: y, width: 0, height: 0 }
      return { x: x, y: y, width: card.width + 2 * s, height: card.height + 2 * s }
    }

    // Input region: the zone; while pinned, everything off the bar, to
    // click away.
    Item {
      id: hitArea
      x: root.pinned ? root.insets.left : window.zone.x
      y: root.pinned ? root.insets.top : window.zone.y
      width: root.pinned ? Math.max(0, window.width - root.insets.left - root.insets.right) : window.zone.width
      height: root.pinned ? Math.max(0, window.height - root.insets.top - root.insets.bottom) : window.zone.height
    }

    MouseArea {
      anchors.fill: parent
      visible: root.pinned
      onClicked: if (root.copy) root.copy.close()
    }

    // The pointer is on the notch while it is on the card or the slack
    // around it. The card reports its own hover: an item beside it loses
    // the pointer to any control inside the card that takes hover, which
    // closed the notch as the pointer crossed its buttons. Naming the
    // screen keeps the copy that moves on it while open, even when another
    // screen has focus (the closed notch can hang off-focus).
    readonly property string screenName: window.screen ? window.screen.name : ""
    Item {
      id: hoverZone
      x: window.zone.x
      y: window.zone.y
      width: window.zone.width
      height: window.zone.height
      HoverHandler {
        onHoveredChanged: if (root.copy) root.copy.setPointer("notch-slack", hovered, window.screenName)
      }
    }

    NotchCard {
      id: card
      notch: root.copy
      edge: root.shownPlacement.edge
      maxWidth: window.width - Theme.space(64)
      maxHeight: window.height - Theme.space(64)
      // Position.js has the rules; the shoulders stay on screen.
      // Closer than this to an end of the edge, there is no room for the
      // shoulder: the card goes flush into the corner (Position.place).
      readonly property real snap: card.shoulder + Theme.space(4)
      readonly property var spot: Position.place({
        edge: card.edge, offset: root.shownOffset,
        width: window.width, height: window.height, cardWidth: card.width, cardHeight: card.height,
        insets: root.edgeInsets, snap: card.snap
      })
      // Flush with a neighbor, or in a corner, the end loses its shoulder.
      // Not while dragged: it follows the pointer, alone.
      startSide: root.joinStart && !root.dragPlacement ? "joined" : card.spot.atStart ? "corner" : "shoulder"
      endSide: root.joinEnd && !root.dragPlacement ? "joined" : card.spot.atEnd ? "corner" : "shoulder"
      startNeighborDepth: root.joinStart ? root.joinStart.cardDepth : card.vertical ? 0 : root.cornerSpan.start
      endNeighborDepth: root.joinEnd ? root.joinEnd.cardDepth : card.vertical ? 0 : root.cornerSpan.end
      minDepth: root.dragPlacement ? 0 : root.rowDepth
      x: card.spot.x
      y: card.spot.y

      onHoveredChanged: if (root.copy) root.copy.setPointer("notch", card.hovered, window.screenName)

      // Held and dragged, it jumps to the edge nearest the pointer, its
      // middle under the pointer, and lands there on release
      // (Position.dragTarget). The pointer stays with this window while the
      // button is down, even over another screen: its position plus this
      // screen's gives the point on the desktop, and the screen there
      // (Position.screenAt) gets a preview and, on the drop, the notch.
      // With more than one screen, the drop also makes that screen its home,
      // and lets in the workspace it shows if `workspaces` left it out, so
      // the notch stays where it was dropped (Config.withWorkspace). A notch
      // on every screen only takes the edge and offset: all its copies move.
      draggable: !!root.copy && root.copy.canDrag
      onDragStarted: if (root.copy) root.copy.setDragging(true)
      onDragMoved: function(scenePosition) {
        var here = window.screen
        var gx = (here ? here.x : 0) + scenePosition.x
        var gy = (here ? here.y : 0) + scenePosition.y
        var name = Position.screenAt(root.screenGeometry(), gx, gy) || root.windowScreenName
        var target = root.screenNamed(name) || here
        root.dragScreen = name
        root.dragPlacement = Position.dragTarget({
          x: gx - (target ? target.x : 0), y: gy - (target ? target.y : 0),
          width: target ? target.width : window.width, height: target ? target.height : window.height,
          insets: root.insets, magnet: Theme.space(24)
        })
      }
      onDragEnded: {
        var notch = root.notch
        var drop = null
        if (notch && root.dragPlacement) {
          drop = Object.assign({}, root.dragPlacement)
          drop.screen = notch.everyScreen ? Position.ALL_SCREENS
            : (Quickshell.screens || []).length > 1 ? root.dragScreen : ""
        }
        root.dragPlacement = null
        root.dragScreen = ""
        // The daemon puts it there (and lets in the workspace the screen shows).
        if (root.copy) root.copy.setDragging(false, drop)
      }
      opacity: root.concealed || root.dragAway ? 0 : 1
      Behavior on opacity { NumberAnimation { duration: 160 } }
      // Concealed and faded out, it stops drawing, and views that animate
      // pause (they check `visible`): a notch hidden over a fullscreen app
      // costs no frames. Dragged away it stays visible: it holds the pointer.
      visible: !root.concealed || opacity > 0
    }
  }

  // ------------------------------------------------------ drag preview
  function screenGeometry() {
    return (Quickshell.screens || []).map(function(s) {
      return { name: s.name, x: s.x, y: s.y, width: s.width, height: s.height }
    })
  }
  function screenNamed(name) {
    var screens = Quickshell.screens || []
    for (var i = 0; i < screens.length; i++) if (screens[i].name === name) return screens[i]
    return null
  }

  // The notch where a drag on another screen would land it: a window of its
  // own there (this one can't draw beyond its screen), showing the closed
  // notch and taking no input.
  Variants {
    model: root.dragAway ? [root.screenNamed(root.dragScreen)].filter(function(s) { return !!s }) : []
    PanelWindow {
      id: preview
      required property var modelData
      screen: modelData
      anchors { top: true; left: true; right: true; bottom: true }
      color: "transparent"
      exclusionMode: ExclusionMode.Ignore
      WlrLayershell.namespace: root.layerNamespace + "-drag"
      WlrLayershell.layer: WlrLayer.Overlay
      WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
      mask: Region {}

      NotchCard {
        id: previewCard
        notch: root.copy
        edge: root.dragPlacement ? root.dragPlacement.edge : "top"
        maxWidth: preview.width - Theme.space(64)
        maxHeight: preview.height - Theme.space(64)
        readonly property var spot: Position.place({
          edge: previewCard.edge, offset: root.dragPlacement ? root.dragPlacement.offset : 0.5,
          width: preview.width, height: preview.height, cardWidth: previewCard.width, cardHeight: previewCard.height,
          insets: root.insets, snap: previewCard.shoulder + Theme.space(4)
        })
        startSide: previewCard.spot.atStart ? "corner" : "shoulder"
        endSide: previewCard.spot.atEnd ? "corner" : "shoulder"
        x: previewCard.spot.x
        y: previewCard.spot.y
      }
    }
  }
}
