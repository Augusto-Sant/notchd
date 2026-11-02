import QtQuick
import "State.js" as State
import "Navigate.js" as Navigate

// A notch on screen: one copy of a Notch, with the state the pointer and
// peeks change (hover, pin, peek, drag) and so its own mode. A notch that
// moves, or stays on its home, has one copy, and this picks its screen; one
// on every screen has a copy per screen (`fixedScreen`). The visitor's
// copies are guests: hidden while closed, they only show peeks. NotchCard
// draws a copy and NotchWindow puts it on its screen.
Item {
  id: copy
  visible: false

  property var owner: null
  // The screen this copy stays on; "" for the copy that moves.
  property string fixedScreen: ""

  readonly property var svc: copy.owner ? copy.owner.svc : null

  // ------------------------------------------------- what the card reads
  readonly property string edge: copy.owner ? copy.owner.edge : "top"
  readonly property bool vertical: !!copy.owner && copy.owner.vertical
  readonly property var placement: copy.owner ? copy.owner.placement : ({ edge: "top", offset: 0.5, screen: "" })
  readonly property bool reserveSpace: !!copy.owner && copy.owner.reserveSpace
  readonly property bool canDrag: !!copy.owner && copy.owner.canDrag
  readonly property var compactModule: copy.owner ? copy.owner.compactModule : null
  readonly property var compactModules: copy.owner ? copy.owner.compactModules : []
  readonly property var expandedModules: copy.owner ? copy.owner.expandedModules : []
  // What the open notch shows of those (State.shownModules): all of them,
  // or with the notch's `tabs`, the one whose tab is picked; and the
  // modules to make tabs of ([] for none).
  property string tab: ""
  readonly property var shown: State.shownModules(copy.expandedModules, !!copy.owner && copy.owner.tabs, copy.tab)
  readonly property var shownModules: copy.shown.modules
  readonly property var tabModules: copy.shown.tabs
  // Picks a module's tab, by id.
  function showTab(moduleId) { copy.tab = moduleId || "" }

  // ------------------------------------------------------------------ peek
  property var peekModule: null

  function requestPeek(module, ms) {
    if (!State.canPeek(module, { pinned: copy.pinned, hovered: copy.hovered,
                                 current: copy.peekModule, currentShowing: peekTimer.running })) {
      copy.keepMissed(module, ms)
      return
    }
    copy.peekModule = module
    peekTimer.interval = Math.max(400, Number(ms) || 2500)
    peekTimer.restart()
  }

  function dismissPeek(module) {
    if (!module || copy.peekModule === module) {
      peekTimer.stop()
      copy.peekModule = null
    }
  }

  Timer { id: peekTimer; onTriggered: copy.peekModule = null }

  // Peeks the notch couldn't show, of modules with news you haven't seen
  // (NotchModule.attention): { module, ms }. Each shows once, as the notch
  // closes (State.attentionPeek), unless you have seen it by then.
  property var missed: []
  function keepMissed(module, ms) {
    if (!module || !module.attention) return
    copy.missed = copy.missed.filter(function(e) { return e.module !== module }).concat([{ module: module, ms: ms }])
  }
  function peekMissed() {
    if (copy.isOpen) return
    var list = copy.missed.filter(function(e) { return e.module.attention })
    var module = State.attentionPeek(list.map(function(e) { return e.module }))
    copy.missed = list.filter(function(e) { return e.module !== module })
    if (!module) return
    for (var i = 0; i < list.length; i++) if (list[i].module === module) copy.requestPeek(module, list[i].ms)
  }

  // ----------------------------------------------------------------- hover
  // Several surfaces report the pointer (the notch card, the slack around
  // it); the copy counts as hovered while any has it.
  property var pointerSources: ({})
  readonly property bool pointerInside: Object.keys(copy.pointerSources).length > 0
  property bool hovered: false
  property bool pinned: false

  // The screen the pointer opened the copy that moves on; it stays there
  // while open instead of following focus (the closed notch can hang on a
  // screen without focus).
  property string openScreen: ""

  function setPointer(source, inside, screenName) {
    var next = Object.assign({}, copy.pointerSources)
    if (inside) next[source] = true
    else delete next[source]
    copy.pointerSources = next
    if (inside && screenName) copy.openScreen = screenName
  }

  onPointerInsideChanged: {
    if (pointerInside) { closeTimer.stop(); openTimer.restart() }
    else {
      openTimer.stop()
      closeTimer.restart()
      // Left before it opened: forget the screen it was about to open on.
      if (!copy.isOpen) copy.openScreen = ""
    }
  }
  onHoveredChanged: if (hovered && copy.peekModule && !copy.peekModule.urgentPeek) copy.dismissPeek()
  onPinnedChanged: if (pinned) {
    copy.dismissPeek()
    if (copy.navigating && copy.svc) copy.svc.endNavigation(false)
  }
  // On the new mode, not `isOpen`: that binding may still hold the old mode
  // here, and would wipe the screen as the copy opens.
  onModeChanged: if (copy.mode !== "peek" && copy.mode !== "expanded") copy.openScreen = ""

  Timer { id: openTimer; interval: copy.svc ? copy.svc.hoverOpenDelay : 0; onTriggered: if (!copy.dragging) copy.hovered = true }
  Timer { id: closeTimer; interval: copy.svc ? copy.svc.hoverCloseDelay : 0; onTriggered: copy.hovered = false }

  // While dragged the copy stays closed (State.mode) and nothing opens it;
  // dropping it leaves it closed until the pointer comes back.
  property bool dragging: false
  function setDragging(on) {
    copy.dragging = on
    openTimer.stop()
    copy.pinned = false
    copy.hovered = false
  }

  // ------------------------------------------------------------ navigation
  // Navigation (NotchService.navigating, Navigate.js) highlights every
  // notch of the config with its number; the selected one's copy on the
  // focused screen opens as a preview (`preview`), not pinned. Guests (the
  // visitor, standalone notches) take no part.
  readonly property bool navigating: !!copy.svc && copy.svc.navigating
  readonly property bool navShown: copy.navigating && !copy.guest
  readonly property bool navSelected: copy.navShown && !!copy.owner && copy.owner.navSelected
  readonly property int navNumber: copy.owner ? copy.owner.navNumber : 0
  readonly property string navName: copy.owner ? copy.owner.notchId : ""
  // "select", "move" or "remove" (NotchService.navMode), and the keys the
  // selected notch's label lists, a line each (Navigate.hint).
  readonly property string navMode: copy.navSelected ? copy.svc.navMode : "select"
  readonly property var navHint: copy.navSelected
    ? Navigate.hint(copy.navMode, copy.svc.notches.length, copy.tabModules.length > 1, copy.svc.screens.length) : []
  readonly property bool preview: copy.navSelected && copy.owner.focusedCopy() === copy
  // Pinned some other way while navigating (a click, a shortcut), it is the
  // notch you went to, so navigation ends (onPinnedChanged); Enter ends it
  // first.

  function open() { copy.pinned = true }
  function close() { copy.pinned = false; copy.hovered = false }
  function togglePinned(screenName) {
    if (screenName) copy.openScreen = screenName
    copy.pinned = !copy.pinned
  }

  // ------------------------------------------------------------------ mode
  readonly property string mode: State.mode({ peekModule: copy.peekModule, pinned: copy.pinned, dragging: copy.dragging,
                                             hovered: copy.hovered, compactModule: copy.compactModules.length > 0 ? copy.compactModules[0] : null,
                                             guest: copy.guest, navigating: copy.navShown, preview: copy.preview })
  readonly property bool isOpen: copy.mode === "peek" || copy.mode === "expanded"
  // The keyboard its window takes (State.keyboardFocus), while pinned open:
  // "exclusive" until the window has it, then "ondemand", so a focus
  // shortcut can move to a window; else "none".
  // While navigating, the navigation has the keyboard; leaving it, a
  // pinned copy asks for it again, exclusively (`keyboardHeld` cleared).
  readonly property var keyboardState: ({ pinned: copy.pinned, mode: copy.mode, modules: copy.shownModules,
                                          navigating: copy.navigating, settling: !!copy.svc && copy.svc.navSettling,
                                          elsewhere: copy.focusedScreenName !== "" && copy.screenName !== copy.focusedScreenName })
  readonly property bool wantsKeyboard: State.wantsKeyboard(copy.keyboardState)
  // Its window got the keyboard since the copy last wanted it.
  property bool keyboardHeld: false
  onWantsKeyboardChanged: if (!copy.wantsKeyboard) copy.keyboardHeld = false
  onNavigatingChanged: copy.keyboardHeld = false
  readonly property string keyboardFocus: State.keyboardFocus(Object.assign({ held: copy.keyboardHeld }, copy.keyboardState))
  // Its window gained or lost the keyboard (State.keyboardChange): lost to
  // a window while it wanted it, the notch closes, as a click away does;
  // lost as navigation's window goes, it asks again.
  function windowKeyboard(active) {
    var change = State.keyboardChange(Object.assign({ active: active, held: copy.keyboardHeld }, copy.keyboardState))
    if (change === "hold") copy.keyboardHeld = true
    else if (change === "retake") copy.keyboardHeld = false
    else if (change === "close") copy.close()
  }

  // ---------------------------------------------------------------- window
  // Whether its window is on screen (State.needsWindow): a guest's only
  // while open and for `windowLinger` ms after it closes.
  property int windowLinger: 5000
  property bool lingering: false
  onIsOpenChanged: if (!copy.isOpen) {
    copy.lingering = true
    lingerTimer.restart()
    Qt.callLater(copy.peekMissed)
  }
  Timer { id: lingerTimer; interval: copy.windowLinger; onTriggered: copy.lingering = false }
  readonly property bool needsWindow: State.needsWindow({ guest: copy.guest, open: copy.isOpen, lingering: copy.lingering })
  // A window going away reports no pointer leaving it: forget what it had.
  onNeedsWindowChanged: if (!copy.needsWindow) copy.pointerSources = ({})

  // ---------------------------------------------------------------- screen
  readonly property var compositor: copy.svc ? copy.svc.compositor : null
  readonly property string focusedScreenName: copy.owner ? copy.owner.focusedScreenName : ""
  readonly property var screenStates: copy.owner ? copy.owner.screenStates : []
  function stateOf(name) {
    for (var i = 0; i < copy.screenStates.length; i++)
      if (copy.screenStates[i].name === name) return copy.screenStates[i]
    return null
  }
  // The screen the config's position names, where the closed notch stays.
  readonly property string homeScreen: copy.placement.screen === "all" ? copy.fixedScreen : copy.placement.screen
  // A copy of the visitor or of a standalone notch: it only shows peeks and
  // the open notch.
  readonly property bool guest: !!copy.owner && copy.owner.guest
  // The screen the copy wants, "" when it doesn't show the closed notch. A
  // copy on its own screen stays there, a guest only while open; the one
  // that moves goes where State.notchScreen says.
  readonly property string wantedScreen: {
    if (copy.fixedScreen !== "") {
      if (copy.guest) return copy.isOpen ? copy.fixedScreen : ""
      var own = copy.stateOf(copy.fixedScreen)
      return copy.isOpen || (own && own.allowed) ? copy.fixedScreen : ""
    }
    return State.notchScreen({
      screens: copy.screenStates, focused: copy.focusedScreenName, open: copy.isOpen,
      override: copy.openScreen, home: copy.homeScreen
    })
  }
  readonly property bool offWorkspace: copy.wantedScreen === ""
  // Where its window goes: the wanted screen; with none, its own screen or
  // the focused one, concealed.
  readonly property string screenName: copy.wantedScreen !== "" ? copy.wantedScreen
    : copy.fixedScreen !== "" ? copy.fixedScreen : copy.focusedScreenName

  // The screens that keep windows clear of the closed notch (reserveSpace):
  // a copy on its own screen reserves there while it may hang there, a
  // guest nowhere; the one copy as State.reserveScreens says.
  readonly property var reservedScreens: {
    if (copy.guest) return []
    if (copy.fixedScreen !== "") {
      var own = copy.stateOf(copy.fixedScreen)
      return own && own.allowed ? [copy.fixedScreen] : []
    }
    return State.reserveScreens(copy.screenStates, copy.homeScreen)
  }
}
