import QtQuick
import Quickshell
import "Position.js" as Position

// One notch of the config's `notches` (or the top-level one): its modules,
// where it hangs and how. NotchService makes one per notch the daemon's
// `state` lists; it shows as copies (NotchCopy), which bind the state of
// each screen's copy, while this holds what they share:
//   following focus      one copy, which moves to the focused screen
//   kept on a screen     one copy, which stays where it hangs (its home, or
//                        where `workspaces` sends it), peeks included
//   on every screen      ("all") a copy per screen, each a
//                        notch of its own
//
// The service also makes one `visitor`: no config of its own, a copy per
// screen at the top, each hidden until it shows a peek on a screen where no
// notch with that module does. And one per standalone module switched on
// (`standaloneId`): only that module, on every screen where "<id>.position"
// says, hidden until it peeks. The daemon decides which peeks go where.
Item {
  id: notch
  visible: false

  property var svc: null
  property string notchId: ""
  // The visitor, rather than a notch of the config.
  property bool visitor: false
  // A standalone module's own notch: it shows only that module, where the
  // module's "<id>.position" says.
  property string standaloneId: ""
  // The visitor or a standalone notch: hidden while closed, a copy on every
  // screen, only there for peeks (NotchCopy.guest).
  readonly property bool guest: notch.visitor || notch.standaloneId !== ""
  // What the daemon calls it in the messages that name a copy: a standalone
  // module's own id, the visitor's "".
  readonly property string wireId: notch.standaloneId !== "" ? notch.standaloneId : notch.notchId

  // ----------------------------------------------------------------- state
  // The daemon's notch: its place, its modules, the copies' state.
  readonly property var info: {
    if (!notch.svc || notch.visitor) return null
    return notch.standaloneId !== "" ? notch.svc.standaloneInfo[notch.standaloneId] || null
      : notch.svc.notchInfo[notch.notchId] || null
  }
  // The copies' state, one per screen where there is one.
  readonly property var copyInfos: notch.visitor ? (notch.svc ? notch.svc.visitorInfo : []) : notch.info ? notch.info.copies : []
  // The state of the copy on a screen; the one copy, for a notch that moves.
  function copyInfo(screen) {
    var list = notch.copyInfos
    if (!notch.everyScreen) return list.length > 0 ? list[0] : null
    for (var i = 0; i < list.length; i++) if (list[i].screen === screen) return list[i]
    return null
  }

  // ---------------------------------------------------------------- config
  // The keys of the config's entry the state does not carry (Config.notches);
  // null for the visitor and the standalone notches.
  readonly property var spec: notch.svc && !notch.guest ? notch.svc.notchSpec(notch.notchId) : null
  // Its place among the config's notches, -1 for the guests.
  readonly property int index: notch.svc && !notch.guest ? notch.svc.notchIds.indexOf(notch.notchId) : -1
  // Never a standalone module, but in its own notch.
  readonly property var moduleIds: notch.info && !notch.guest ? notch.info.modules : notch.standaloneId !== "" ? [notch.standaloneId] : []
  // Keep windows out of the strip the closed notch hangs into.
  readonly property bool reserveSpace: !!notch.spec && notch.spec.reserveSpace
  // The open notch shows one module at a time, picked from tabs.
  readonly property bool tabs: !!notch.info && !notch.guest && notch.info.tabs
  // A module with nothing to show closed shows its icon there.
  readonly property bool idleIcons: !!notch.spec && notch.spec.idleIcons
  // Workspaces the closed notch hangs on (ids or names); null: every one.
  readonly property var workspaces: notch.spec ? notch.spec.workspaces : null
  // The position, { edge, offset, screen } (Position.js): the daemon's for a
  // notch of the config; a standalone module's from its own key, the
  // visitor's the default on every screen.
  readonly property var wantedPlace: Position.resolve(notch.standaloneId !== "" ? (notch.svc ? notch.svc.standalonePosition(notch.standaloneId) : { screen: Position.ALL_SCREENS })
    : notch.visitor ? { screen: Position.ALL_SCREENS }
    : notch.info ? { edge: notch.info.edge, offset: notch.info.offset, screen: notch.info.screen }
    : notch.spec ? notch.spec.position : undefined)
  // Where it is. Replaced only when it moves: a state that says the same
  // makes nothing move.
  property var placement: Position.resolve(undefined)
  onWantedPlaceChanged: if (JSON.stringify(notch.wantedPlace) !== JSON.stringify(notch.placement)) notch.placement = notch.wantedPlace
  readonly property string edge: notch.placement.edge
  readonly property bool vertical: Position.isVertical(notch.edge)
  // A copy on every screen, each showing the notch.
  readonly property bool everyScreen: notch.placement.screen === Position.ALL_SCREENS
  // Kept on one screen (its home), by name.
  readonly property bool homed: !notch.everyScreen && notch.placement.screen !== ""
  // Whether it can be dragged to another edge.
  readonly property bool canDrag: !!notch.info && !notch.guest && notch.info.draggable

  // The config key for one of this notch's keys: "position" at the top
  // level, "notches.<index>.position" in a list.
  function configKey(key) { return (notch.spec ? notch.spec.prefix : "") + key }
  function setValue(key, value) { if (notch.svc) notch.svc.setConfigValue(notch.configKey(key), value) }

  // --------------------------------------------------------------- modules
  // Its modules, in its order, out of the ones the service loaded.
  readonly property var modules: {
    var out = []
    var loaded = notch.svc ? notch.svc.modules : []
    for (var i = 0; i < notch.moduleIds.length; i++)
      for (var j = 0; j < loaded.length; j++)
        if (loaded[j].moduleId === notch.moduleIds[i]) { out.push(loaded[j]); break }
    return out
  }
  // The ones with something to show now.
  readonly property var activeModules: notch.modules.filter(function(m) { return m.active })
  // What the closed notch shows side by side, as the daemon says
  // (State.compactModules): its first copy's, which every copy has.
  readonly property var compactModules: notch.copies.length > 0 ? notch.copies[0].compactModules : []
  readonly property var compactModule: notch.copies.length > 0 ? notch.copies[0].compactModule : null

  // ---------------------------------------------------------------- copies
  // Screen names, one copy each, as the daemon has them; [""] for the one
  // copy that moves. Replaced only when the list changes: a new one would
  // make every copy again.
  readonly property var wantedScreens: notch.everyScreen ? notch.copyInfos.map(function(c) { return c.screen }) : [""]
  property var copyScreens: [""]
  onWantedScreensChanged: notch.syncScreens()
  function syncScreens() {
    if (JSON.stringify(notch.wantedScreens) !== JSON.stringify(notch.copyScreens)) notch.copyScreens = notch.wantedScreens
  }
  Component.onCompleted: {
    notch.placement = notch.wantedPlace
    notch.syncScreens()
  }

  Variants {
    id: copyVariants
    readonly property var owner: notch
    model: notch.copyScreens
    NotchCopy {
      required property var modelData
      owner: copyVariants.owner
      fixedScreen: modelData
    }
  }
  readonly property var copies: copyVariants.instances

  // The copy on a screen; the one copy, for a notch that moves.
  function copyOn(screenName) {
    var list = notch.copies
    if (!notch.everyScreen) return list.length > 0 ? list[0] : null
    for (var i = 0; i < list.length; i++) if (list[i].fixedScreen === screenName) return list[i]
    return null
  }

  // ------------------------------------------------------------ navigation
  // The number it shows while navigating (1 to 9, else 0: reached with the
  // arrows) and whether it is the one selected (NotchService.navSelected).
  readonly property int navNumber: notch.index >= 0 && notch.index < 9 ? notch.index + 1 : 0
  readonly property bool navSelected: !!notch.svc && notch.svc.navigating && notch.index >= 0
    && notch.svc.navSelected === notch.index

  // ------------------------------------------------------------------ open
  // The daemon opens it, on the focused screen: its first copy, pinned.
  function open() { if (notch.svc && !notch.guest) notch.svc.notchIpc("openNotch", [notch.notchId]) }
  function close() { if (notch.svc && !notch.guest) notch.svc.notchIpc("closeNotch", [notch.notchId]) }
  function toggle() { if (notch.svc && !notch.guest) notch.svc.notchIpc("toggleNotch", [notch.notchId]) }
}
