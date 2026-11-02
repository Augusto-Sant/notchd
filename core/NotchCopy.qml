import QtQuick

// A notch on screen: one copy of a Notch. The daemon decides what it is doing
// (mode, pinned, tab, peek, which modules it shows, the screen it is on, the
// keyboard its window asks for: the `state` of NotchService) and this binds
// it; what it sends are the raw inputs (the pointer, a press, a key, a tab, a
// drag, its window's keyboard). A notch that moves, or stays on its home, has
// one copy, and the daemon picks its screen; one on every screen has a copy
// per screen (`fixedScreen`). The visitor's copies are guests: hidden while
// closed, they only show peeks. NotchCard draws a copy and NotchWindow puts
// it on its screen.
Item {
  id: copy
  visible: false

  property var owner: null
  // The screen this copy stays on; "" for the copy that moves.
  property string fixedScreen: ""

  readonly property var svc: copy.owner ? copy.owner.svc : null
  // The daemon connection, for what is sent to it (DaemonClient).
  readonly property var daemon: copy.svc ? copy.svc.client : null
  // What it is called in the messages (Notch.wireId).
  readonly property string wireNotch: copy.owner ? copy.owner.wireId : ""

  // The daemon's state of this copy: a new object with every `state`.
  readonly property var info: copy.owner ? copy.owner.copyInfo(copy.fixedScreen) : null

  // ------------------------------------------------- what the card reads
  readonly property string edge: copy.owner ? copy.owner.edge : "top"
  readonly property bool vertical: !!copy.owner && copy.owner.vertical
  readonly property var placement: copy.owner ? copy.owner.placement : ({ edge: "top", offset: 0.5, screen: "" })
  readonly property bool reserveSpace: !!copy.owner && copy.owner.reserveSpace
  readonly property bool canDrag: !!copy.owner && copy.owner.canDrag
  // A copy of the visitor or of a standalone notch: it only shows peeks and
  // the open notch.
  readonly property bool guest: !!copy.owner && copy.owner.guest

  readonly property string mode: copy.info ? copy.info.mode : "rest"
  readonly property bool isOpen: copy.mode === "peek" || copy.mode === "expanded"
  readonly property bool pinned: !!copy.info && copy.info.pinned === true
  // The tab picked (a module id), "" for the notch's own.
  readonly property string tab: copy.info && copy.info.tab !== undefined ? copy.info.tab : ""

  // The modules, resolved out of the ids the daemon names. Each list is only
  // replaced when it changes: a new one would make the views made from it
  // again (a notch's expanded views, a field you were typing in).
  //   compactModules   what the closed notch shows side by side
  //   compactModule    the first of those with a compact view
  //   shownModules     what the open notch shows: all of them, or with the
  //                    notch's `tabs`, the one whose tab is picked
  //   tabModules       the modules to make tabs of ([] for none)
  //   peekModule       the module whose peek it shows
  property var compactModules: []
  readonly property var compactModule: {
    for (var i = 0; i < copy.compactModules.length; i++)
      if (copy.compactModules[i].compact) return copy.compactModules[i]
    return null
  }
  property var shownModules: []
  property var tabModules: []
  property var peekModule: null

  function sameList(a, b) {
    if (a.length !== b.length) return false
    for (var i = 0; i < a.length; i++) if (a[i] !== b[i]) return false
    return true
  }
  function modulesOf(ids) {
    var out = []
    for (var i = 0; ids && i < ids.length; i++) {
      var m = copy.svc ? copy.svc.moduleById(ids[i]) : null
      if (m) out.push(m)
    }
    return out
  }
  function assign(name, list) { if (!copy.sameList(copy[name], list)) copy[name] = list }
  function sync() {
    var i = copy.info
    copy.assign("compactModules", copy.modulesOf(i ? i.compact : []))
    copy.assign("shownModules", copy.modulesOf(i ? i.shown : []))
    copy.assign("tabModules", copy.modulesOf(i ? i.tabs : []))
    var peeking = i && i.peek ? copy.modulesOf([i.peek.module])[0] || null : null
    if (copy.peekModule !== peeking) copy.peekModule = peeking
  }
  onInfoChanged: copy.sync()
  Connections {
    target: copy.svc
    function onModulesChanged() { copy.sync() }
  }
  Component.onCompleted: copy.sync()

  // Picks a module's tab, by id.
  function showTab(moduleId) {
    if (!copy.daemon) return
    copy.svc.flushReports()
    copy.daemon.tab(copy.wireNotch, copy.fixedScreen, moduleId || "")
  }

  // ----------------------------------------------------------------- hover
  // Several surfaces report the pointer (the notch card, the slack around
  // it); the copy counts as hovered while any has it. The daemon is told
  // when the pointer comes onto the copy and when it leaves it, and owns the
  // delays and the mode that follows.
  property var pointerSources: ({})
  readonly property bool pointerInside: Object.keys(copy.pointerSources).length > 0
  // The screen the pointer was last on, for the copy that moves: the daemon
  // keeps it there while it is open instead of following focus (the closed
  // notch can hang on a screen without focus).
  property string pointerScreen: ""

  function setPointer(source, inside, screenName) {
    // The screen first: the daemon is told as the sources change.
    if (inside && screenName) copy.pointerScreen = screenName
    var next = Object.assign({}, copy.pointerSources)
    if (inside) next[source] = true
    else delete next[source]
    copy.pointerSources = next
  }
  function sendPointer(ev) {
    if (copy.daemon) copy.daemon.pointer(copy.wireNotch, copy.fixedScreen !== "" ? copy.fixedScreen : copy.pointerScreen, ev)
  }
  onPointerInsideChanged: copy.sendPointer(copy.pointerInside ? "enter" : "leave")
  // A daemon that came back has not seen the pointer: tell it where it is.
  Connections {
    target: copy.daemon
    function onWelcomed() { if (copy.pointerInside) copy.sendPointer("enter") }
  }

  // A press on the copy pins it open, or closes it again.
  function togglePinned(screenName) {
    if (!copy.daemon) return
    copy.svc.flushReports()
    copy.daemon.pointer(copy.wireNotch, copy.fixedScreen !== "" ? copy.fixedScreen : (screenName || ""), "press")
  }
  // Pins it open, or closes it (and ends its hover). What is waiting to be
  // sent (a config edit, a module's news) goes first.
  function open() { copy.pin(true) }
  function close() { copy.pin(false) }
  function pin(on) {
    if (!copy.daemon) return
    copy.svc.flushReports()
    copy.daemon.pin(copy.wireNotch, copy.fixedScreen, on)
  }

  // ------------------------------------------------------------------ drag
  // While dragged the daemon keeps the copy closed and nothing opens it;
  // dropping it leaves it closed until the pointer comes back. The motion is
  // the window's (NotchWindow): this only says it started, and where it
  // landed (`place`: { edge, offset, screen }) or that it was let go.
  property bool dragging: false
  function setDragging(on, place) {
    copy.dragging = on
    if (!copy.daemon) return
    if (on) copy.daemon.drag(copy.wireNotch, copy.fixedScreen, "start")
    else copy.daemon.drag(copy.wireNotch, copy.fixedScreen, place ? "drop" : "cancel", place)
  }

  // ------------------------------------------------------------ navigation
  // Navigation (NotchService.navigating) highlights every notch of the
  // config with its number; the selected one's copy on the focused screen
  // opens as a preview, not pinned. Guests (the visitor, standalone notches)
  // take no part.
  readonly property bool navigating: !!copy.svc && copy.svc.navigating
  readonly property bool navShown: copy.navigating && !copy.guest
  readonly property bool navSelected: copy.navShown && !!copy.owner && copy.owner.navSelected
  readonly property int navNumber: copy.owner ? copy.owner.navNumber : 0
  readonly property string navName: copy.owner ? copy.owner.notchId : ""
  // "select", "move" or "remove" (NotchService.navMode), and the keys the
  // selected notch's label lists, a line each.
  readonly property string navMode: copy.navSelected ? copy.svc.navMode : "select"
  readonly property var navHint: copy.navSelected
    ? copy.hint(copy.navMode, copy.svc.notchIds.length, copy.tabModules.length > 1, copy.svc.screens.length) : []
  readonly property bool preview: copy.navSelected && copy.mode === "expanded"

  // The keys the selected notch's label lists, one string per line: `mode`
  // as the daemon's navigation has it, `count` notches, whether it has `tabs`
  // to walk and how many `screens` there are.
  function hint(mode, count, tabs, screens) {
    if (mode === "remove") return ["x remove   Esc keep"]
    if (mode === "move") {
      return [["←↑↓→ edge", "⇧ slide", screens > 1 ? "Tab screen" : ""].filter(Boolean).join("   "),
              "↵ done   Esc undo"]
    }
    return [[count > 1 ? "1-" + Math.min(9, count) + " ←→" : "", tabs ? "Tab" : "", "↵ open", "Esc"].filter(Boolean).join("   "),
            ["m move", "s settings", count > 1 ? "x remove" : ""].filter(Boolean).join("   ")]
  }

  // -------------------------------------------------------------- keyboard
  // The keyboard its window takes: "exclusive" until the window has it, then
  // "ondemand", so a focus shortcut can move to a window; "none" while it is
  // not pinned open or navigation has it. Its window reports when it gains
  // or loses it (`windowKeyboard`), and the daemon closes the notch when a
  // window took it, as a click away does.
  readonly property string keyboardFocus: copy.info && copy.info.keyboard ? copy.info.keyboard : "none"
  function windowKeyboard(active) {
    if (copy.daemon) copy.daemon.focus(copy.wireNotch, copy.fixedScreen, active)
  }

  // ---------------------------------------------------------------- window
  // Whether its window is on screen: a guest's only while open and for a
  // while after it closes, so it closes with its animation and a peek soon
  // after reuses it. The daemon says.
  readonly property bool needsWindow: copy.info ? copy.info.needsWindow : !copy.guest
  // A window going away reports no pointer leaving it: forget what it had.
  onNeedsWindowChanged: if (!copy.needsWindow) copy.pointerSources = ({})

  // ---------------------------------------------------------------- screen
  readonly property var compositor: copy.svc ? copy.svc.compositor : null
  // The screen its window goes on, as the daemon works it out: where it
  // hangs closed (its home, else the focused screen, else the first one
  // whose workspace takes it), and where it opened while open.
  readonly property string screenName: copy.info ? copy.info.screen : copy.fixedScreen
  // Hung nowhere: its workspace is not on any screen. A guest hangs
  // nowhere while closed.
  readonly property bool offWorkspace: copy.guest ? !copy.isOpen : !!copy.info && copy.info.covered === true
}
