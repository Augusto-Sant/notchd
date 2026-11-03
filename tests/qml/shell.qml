import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "core"
import "core/settings"
import "core/Position.js" as Position
import "core/settings/Settings.js" as Settings

// Behaviour tests that need the QML runtime: bindings, signals, animations.
// Rules that are plain JS belong in the Node tests instead. Most cases here
// are regression tests for bugs found by hand; each says what it guards.
//
// scripts/qml-test.sh links the core next to this file, points
// NOTCHD_QMLTEST_DIR at an empty scratch dir (the service's config lives
// there) and runs it offscreen. Each case logs "PASS <name>" or
// "FAIL <name>: <why>"; "DONE" ends the run.
ShellRoot {
  id: root

  readonly property string dir: Quickshell.env("NOTCHD_QMLTEST_DIR")

  // ------------------------------------------------------------- harness
  // Cases run one after another; a case calls done() (or fail()) when it is
  // finished, so async ones can wait on timers.
  property var cases: []
  property int index: -1
  property string current: ""
  property bool failed: false
  property real started: 0

  function check(cond, why) {
    if (!cond && !root.failed) {
      root.failed = true
      console.log("FAIL " + root.current + ": " + why)
    }
    return cond
  }
  function done() {
    if (!root.failed) console.log("PASS " + root.current)
    console.log("TIME " + (Date.now() - root.started) + " " + root.current)
    Qt.callLater(root.next)
  }
  function next() {
    root.index++
    if (root.index >= root.cases.length) { console.log("DONE"); Qt.quit(); return }
    root.current = root.cases[root.index].name
    root.failed = false
    root.started = Date.now()
    try { root.cases[root.index].run() }
    catch (e) { root.check(false, "threw " + e); root.done() }
  }

  // Runs fn after ms, then the next step; steps([[ms, fn], ...], last).
  function steps(list, last) {
    if (list.length === 0) { last(); return }
    var step = list[0]
    stepTimer.interval = step[0]
    stepTimer.action = function() { step[1](); root.steps(list.slice(1), last) }
    stepTimer.restart()
  }
  Timer {
    id: stepTimer
    property var action: null
    onTriggered: if (action) action()
  }

  // Waits until pred() holds (checked every 20 ms, for at most ms), then
  // runs fn; on a timeout, fn still runs and its checks say what is missing.
  // A case waits for what it needs, never on an earlier case having loaded it.
  function until(pred, fn, ms) {
    untilTimer.left = Math.ceil((ms || 3000) / untilTimer.interval)
    untilTimer.pred = pred
    untilTimer.action = fn
    if (pred()) { untilTimer.action = null; fn(); return }
    untilTimer.restart()
  }
  Timer {
    id: untilTimer
    interval: 20
    repeat: true
    property var pred: null
    property var action: null
    property int left: 0
    onTriggered: {
      if (!pred() && --left > 0) return
      stop()
      var fn = action
      action = null
      if (fn) fn()
    }
  }
  // Keys and focus go to the active window: a case that types activates its own.
  function activate(item) { item.Window.window.requestActivate() }

  // The daemon decides the notch state, so what a case does to a notch shows
  // in the QML a round trip later. A case is written as a flow: a generator
  // that yields what it waits for, each a function that takes `resume`:
  //   yield root.settle(svc)       everything sent so far has been decided and
  //                                the new state is in: the daemon answers a
  //                                request after the state it made, on the
  //                                same connection, so this needs no timing
  //   yield root.waitFor(pred, ms) until pred() holds (3 s by default; the
  //                                checks after it say what is missing)
  //   yield root.wait(ms)          a delay the daemon's own timers need
  //   yield root.nested(fn)        a callback-style helper (fn(resume))
  // The case is done when the flow ends.
  function go(flow) {
    var it = flow()
    function step() {
      var r
      try { r = it.next() }
      catch (e) { root.check(false, "threw " + e); root.done(); return }
      if (r.done) { root.done(); return }
      r.value(step)
    }
    step()
  }
  function settle(svc) {
    return function(resume) {
      svc.flushConfig()
      svc.flushReports()
      if (!svc.client.ready) { resume(); return }
      svc.client.ipc("state", [], function() { resume() })
    }
  }
  // A request to the daemon, answered: `yield root.ask(svc, "timer", ["pause"], function(answer) { ... })`.
  // The answer is the method's string, or "err <code>: <message>". What was
  // sent before it (a config edit, the modules' facts) is sent first.
  function ask(svc, method, args, got) {
    return function(resume) {
      svc.flushReports()
      svc.client.ipc(method, args, function(reply) {
        got(reply.t === "ok" ? reply.result : "err " + reply.code + ": " + (reply.message || ""))
        resume()
      })
    }
  }
  // Every text and label of a view tree (what a module shows), in order.
  function texts(tree) {
    var out = []
    function walk(n) {
      if (n === null || n === undefined) return
      if (typeof n.text === "string") out.push(n.text)
      if (typeof n.label === "string") out.push(n.label)
      var kids = n.children
      for (var i = 0; kids && i < kids.length; i++) walk(kids[i])
    }
    walk(tree)
    return out
  }
  // Clears every activity of a service's module (`clear ""`).
  function clearActivities(svc) { return svc.daemonIpc("clear", [""], "ok") }
  function waitFor(pred, ms) { return function(resume) { root.until(pred, resume, ms) } }
  function wait(ms) { return function(resume) { root.steps([[ms, function() {}]], resume) } }
  function nested(fn) { return function(resume) { fn(resume) } }

  // The pointer comes onto a copy and stays: it opens after the hover delay.
  function hover(copy) { copy.setPointer("test", true, copy.fixedScreen) }
  function unhover(copy) { copy.setPointer("test", false, "") }

  // Navigation's keys by what they mean; the daemon reads them as keys.
  readonly property var navKeys: ({
    previous: ["Left", []], next: ["Right", []], up: ["Up", []], down: ["Down", []],
    slideBack: ["Left", ["shift"]], slideOn: ["Right", ["shift"]],
    previousTab: ["Backtab", []], nextTab: ["Tab", []],
    open: ["Return", []], leave: ["Escape", []], move: ["m", []], settings: ["s", []], remove: ["x", []]
  })
  function navKey(svc, name) {
    var key = root.navKeys[name] || [name, []]
    svc.navKey(key[0], key[1])
  }

  Component.onCompleted: {
    var all = [
      { name: "workspace chips store a real list", run: root.workspaceChipsList },
      { name: "compact depth ignores peek and expanded transitions", run: root.compactDepthTransitions },
      { name: "a joined row's depth ignores a peek opening and closing", run: root.closedDepthTransitions },
      { name: "leaving the notch before it opens forgets the screen", run: root.hoverLeaveForgetsScreen },
      { name: "an opened notch keeps the screen the pointer opened it on", run: root.openKeepsScreen },
      { name: "hover opens the notch within the delay and leaving closes it within the close delay", run: root.hoverLatency },
      { name: "peeks and the open notch never move windows", run: root.reserveIgnoresOpenNotch },
      { name: "reserved space grows at once and shrinks after the delay", run: root.reserveGrowsAndShrinks },
      { name: "one module ending as another starts keeps the space", run: root.reserveBridgesModules },
      { name: "a dragged notch moves windows once, when it lands", run: root.reserveHeldWhileDragging },
      { name: "the notch stays hovered over the controls inside it", run: root.hoverOverControls },
      { name: "a quick click pins the notch and never drags it", run: root.quickClickPins },
      { name: "a quick press-and-pull never drags the notch", run: root.quickPullNoDrag },
      { name: "held, then moved, the notch is dragged and not pinned", run: root.holdThenDrag },
      { name: "a side edge reserves the side compact view, never the old one turned", run: root.reserveFollowsEdge },
      { name: "on a side edge the card grows sideways", run: root.sideEdgeAxes },
      { name: "a peek shows on the focused screen and leaves the other copies alone", run: root.peekOnFocusedCopy },
      { name: "a peek goes only to the notches that show the module", run: root.peekOnlyWhereShown },
      { name: "the visitor's hidden copies hold no compact view", run: root.visitorRestsWhileClosed },
      { name: "a visitor copy has a window only while it shows, and shortly after", run: root.visitorWindowOnlyWhileShown },
      { name: "notches, copies and modules survive a config edit", run: root.survivesConfigEdits },
      { name: "adding a notch and removing it gives the config back", run: root.addAndRemoveNotch },
      { name: "a notch kept on a screen stays there and a peek also shows where you work", run: root.peekBesideHomedNotch },
      { name: "an OSD peek shows once, on the focused screen only", run: root.osdPeekOnFocusedScreen },
      { name: "a standalone module shows in its own place, never in a notch", run: root.standaloneOwnPlace },
      { name: "a notch's settings all sit on its own page, under the Notches list", run: root.notchSettingsOnOnePage },
      { name: "a module no notch loads still has its settings page and title", run: root.unloadedModuleSettings },
      { name: "resources read the system", run: root.resourcesRead },
      { name: "resources show closed by default, or only once one runs short", run: root.resourcesCompactWhenShort },
      { name: "a timer counts down closed, peeks when it ends and goes back to rest", run: root.timerRunsAndEnds },
      { name: "timer commands: pomodoro phases, pause, errors and its settings", run: root.timerCommands },
      { name: "media shows the playing track closed, peeks on a new one and rests when nothing plays", run: root.mediaShowsTheTrack },
      { name: "a module you type into takes the keyboard only while pinned open", run: root.keyboardOnlyWhilePinned },
      { name: "Escape in a field inside the notch closes it, and Ctrl+Escape on the notch", run: root.escapeCloses },
      { name: "a module sets the width of its expanded view", run: root.expandedWidthFromModule },
      { name: "a notch in the list dropped on the top edge stays there", run: root.dropOnTopStays },
      { name: "corners: a side notch's place never feeds back into the insets it takes", run: root.cornersNoLoop },
      { name: "codex: on a UI that draws no terminal, ask says so and the notch still opens", run: root.codexMissing },
      { name: "codex: runs in the notch, keeps running closed, and its title says when it works", run: root.codexRuns },
      { name: "claude: runs in the notch, keeps running closed, and its title says when it works", run: root.claudeRuns },
      { name: "claude: ask over IPC opens the notch and types the question in; new starts another", run: root.claudeAsk },
      { name: "agent toggle opens the agent's notch, whatever it is called, and closes it", run: root.agentToggle },
      { name: "agent usage: the limits show in the open notch, and passing the warning peeks once", run: root.agentUsage },
      { name: "claude: the closed notch keeps its size while it works", run: root.agentBusySteady },
      { name: "codex: closed and idle for idleMinutes, it stops; opened again, it starts", run: root.codexIdle },
      { name: "a terminal of your own: from the config, in your home, its title closed, terminal <id> toggle", run: root.ownTerminal },
      { name: "settings: Add terminal makes one and opens its page; its trash can takes it out of the config", run: root.addAndRemoveTerminal },
      { name: "settings: a required text field refuses to be emptied", run: root.requiredTextField },
      { name: "renaming or removing a terminal keeps the modules ready, and warns of nothing", run: root.terminalRenameStaysReady },
      { name: "terminal: a connection that drops is made again, and the program goes on", run: root.terminalReconnects },
      { name: "a window its monitor took away comes back with the monitor", run: root.keepShownReopens },
      { name: "an open notch taller than its screen fits it and scrolls", run: root.tallNotchScrolls },
      { name: "with tabs the open notch shows one module, and opening for a module picks its tab", run: root.tabsShowOne },
      { name: "with tabs the closed notch shows every module side by side", run: root.tabsCompactAll },
      { name: "news missed while the notch was open peeks once as it closes", run: root.missedPeekOnClose },
      { name: "a user module of the daemon shows in the notch through a CoreModule, its button reaches the module", run: root.userModuleShows },
      { name: "a user module no notch lists is in the settings: catalog, page, defaults, and adding it loads it", run: root.userModuleInSettings },
      { name: "a timer that isn't running shows its icon in the closed notch, unless idleIcons is off", run: root.idleTimerIcon },
      { name: "navigation numbers every notch, previews the selected one and Escape leaves it as it was", run: root.navigationPreview },
      { name: "navigation: Enter pins the selected notch and closes the others; a click ends it too", run: root.navigationOpen },
      { name: "navigation: Tab walks the selected notch's tabs, Escape gives its tab back", run: root.navigationTabs },
      { name: "navigation takes the keyboard from an open agent without closing it, and gives it back", run: root.navigationKeepsAgent },
      { name: "a notch open on the screen without focus keeps the keyboard", run: root.keyboardOnOtherScreen },
      { name: "navigation: x twice removes the selected notch, s opens its settings", run: root.navigationRemove },
      { name: "navigation: m moves the notch live; Enter keeps it, Escape or the shortcut puts it back", run: root.navigationMove },
      { name: "live: a click opens the notch and starts talking, a hover doesn't; closing ends it", run: root.liveTalks },
      { name: "live: stop holds until the notch closes; mute goes to the bridge; IPC words and errors", run: root.liveStopAndCommands },
      { name: "live: without its packages it says what to install, and the config stays as it was", run: root.liveMissingPackages },
      { name: "live: a new session ends the one running, clears the transcript and starts another", run: root.liveNewSession },
      { name: "live: the local voice starts its own bridge with its settings", run: root.localLiveArgs },
      { name: "daemon: a settings edit goes to the daemon and comes back; defaults are never written; a slider drag is one write", run: root.daemonSettingsRoundTrip },
      { name: "daemon: ipc set answers as before, the daemon's own ipc set and a hand edit of the file reach the UI, a bad file is ignored", run: root.daemonIpcAndFile },
      { name: "daemon: a command for a module no notch loads says so, instead of ok", run: root.ipcModuleNotLoaded },
      { name: "daemon: killed, the notch says notchd is not running, the config stays, an edit waits, and it all comes back", run: root.daemonDownAndBack },
      { name: "daemon: SIGKILLed with nothing else happening, it comes back by itself", run: root.daemonKilledComesBack },
      { name: "components: the settings rows show the installer's state, install, cancel and remove (twice) through it, and lock a voice until it is installed", run: root.componentRows }
    ]
    // scripts/qml-test.sh runs the cases in shards at once: NOTCHD_QMLTEST_CASES
    // is the comma-separated indexes of this one's; unset, all of them.
    // NOTCHD_QMLTEST_ONLY keeps those whose name contains it.
    var picked = String(Quickshell.env("NOTCHD_QMLTEST_CASES") || "")
    var only = String(Quickshell.env("NOTCHD_QMLTEST_ONLY") || "")
    var indexes = picked ? picked.split(",").map(Number) : null
    root.cases = all.filter(function(c, i) {
      return (!indexes || indexes.indexOf(i) >= 0) && (!only || c.name.indexOf(only) >= 0)
    })
    warmup.start()
  }
  // The offscreen windows take a moment to be exposed; pointer events sent
  // before that are lost.
  // Every service has a daemon of its own (tests/qml/TestService.qml): the
  // cases start once each has answered and sent its config.
  Timer {
    id: warmup
    interval: 300
    onTriggered: root.until(root.daemonsReady, root.next, 15000)
  }
  function daemonsReady() {
    var services = [service, multi, standaloneSvc, resourced, timed, played, typer, dropSvc, codexSvc, claudeSvc, codexMissingSvc,
                    ownTermSvc, reconnectSvc, liveSvc, localLiveSvc, componentSvc, tallSvc, tabsSvc, idleSvc]
    return services.every(function(s) { return s.daemonReady })
  }

  // --------------------------------------------------------------- fakes
  // A compositor whose every screen shows workspace 4.
  QtObject {
    id: workspace4
    readonly property string focusedScreenName: ""
    function hasFullscreen(screen) { return false }
    readonly property bool supportsWorkspaces: true
    readonly property var knownWorkspaces: ["1", "2", "3", "4"]
    function activeWorkspace(screen) { return { id: 4, name: "4" } }
  }

  TestService {
    id: service
    ipcTarget: "notchd-qmltest"
    configPath: root.dir + "/config.json"
    compositor: workspace4
    // A screen with a name: the daemon hangs a notch on no screen at all
    // ("covered") where the screen's name is empty, as the offscreen one's is.
    screens: [{ name: "DP-1" }]
  }

  // Two screens, A and B; B has focus.
  QtObject {
    id: twoScreens
    property string focusedScreenName: "B"
    function hasFullscreen(screen) { return false }
    readonly property bool supportsWorkspaces: false
    readonly property var knownWorkspaces: []
    function activeWorkspace(screen) { return null }
  }

  TestService {
    id: multi
    ipcTarget: "notchd-qmltest-multi"
    configPath: root.dir + "/multi.json"
    compositor: twoScreens
    screens: [{ name: "A" }, { name: "B" }]
    defaultModules: ["activities"]
    hostModules: ({ focus: Qt.resolvedUrl("fixtures/focus.qml") })
  }

  // ---------------------------------------------------------------- cases
  // Bug: a list edited in the settings went through the control's `var`
  // signal, came out as a sequence (Array.isArray false), and Config.resolve
  // read it as "no list": every workspace. Adding a workspace seemed to work;
  // removing it again did nothing.
  WorkspaceChips {
    id: chips
    svc: service
    value: service.configValue("workspaces")
    onEdited: function(v) { service.setConfigValue("workspaces", v) }
  }

  function workspaceChipsList() {
    root.go(function*() {
      service.replaceConfig({})
      yield root.settle(service)
      var notch = service.notches[0]
      var copy = notch.copies[0]
      chips.toggle(1); chips.toggle(2); chips.toggle(3)
      root.check(Array.isArray(notch.workspaces), "workspaces is not an array: " + JSON.stringify(notch.workspaces))
      root.check(JSON.stringify(notch.workspaces) === "[1,2,3]", "workspaces is " + JSON.stringify(notch.workspaces))
      // The daemon hangs the notch on no screen: every screen shows workspace 4.
      yield root.settle(service)
      root.check(copy.offWorkspace, "shows on workspace 4 with [1,2,3]")
      chips.toggle(4)
      yield root.settle(service)
      root.check(!copy.offWorkspace, "does not show on workspace 4 after adding it")
      chips.toggle(4)
      yield root.settle(service)
      root.check(copy.offWorkspace, "still shows on workspace 4 after removing it")
      chips.toggle(1); chips.toggle(2); chips.toggle(3)
      root.check(service.configValue("workspaces") === undefined, "an empty pick left the key in the config")
      yield root.settle(service)
      root.check(!copy.offWorkspace, "no pick should mean every workspace")
    })
  }

  // Bugs: the reserved space grew past the compact notch. First the peek's
  // height was read as compact (the shown mode and content swap in two
  // steps), then compact content plus the expanded padding (the padding
  // follows the new mode at once while the old content fades out).
  QtObject {
    id: cardSvc
    property string mode: "compact"
    property string edge: "top"
    property bool dragging: false
    readonly property bool reserveSpace: true
    property var compactModule: fakeModule
    readonly property var compactModules: cardSvc.compactModule ? [cardSvc.compactModule] : []
    readonly property var peekModule: fakeModule
    readonly property var expandedModules: []
    function togglePinned(screenName) {}
    function close() {}
  }
  QtObject {
    id: fakeModule
    readonly property Component compact: Component { Item { implicitWidth: 120; implicitHeight: 18 } }
    readonly property Component peek: Component { Item { implicitWidth: 300; implicitHeight: 70 } }
  }
  NotchCard {
    id: card
    notch: cardSvc
    property real tallestCompact: 0
    onCompactDepthChanged: if (compactDepth > tallestCompact) tallestCompact = compactDepth
    property var closedDepths: []
    onClosedDepthChanged: closedDepths = closedDepths.concat([closedDepth])
  }

  function compactDepthTransitions() {
    var settle = 450
    var base = 0
    root.steps([
      [settle, function() { base = card.compactDepth; card.tallestCompact = base }],
      [0, function() { cardSvc.mode = "peek" }],
      [settle, function() { cardSvc.mode = "compact" }],
      [settle, function() { cardSvc.mode = "expanded" }],
      [settle, function() { cardSvc.mode = "compact" }],
      [settle, function() {}]
    ], function() {
      root.check(base > 0, "no compact height measured")
      root.check(card.tallestCompact === base, "compact height reached " + card.tallestCompact + ", compact is " + base)
      root.done()
    })
  }

  // Bug: closing a peek, the notches joined to it grew for a moment and
  // shrank back, as if about to open. The row's depth went by the closing
  // card's own depth, still the peek's while its content faded out.
  function closedDepthTransitions() {
    var settle = 450
    var base = 0
    root.steps([
      [settle, function() { base = card.closedDepth; card.closedDepths = [] }],
      [0, function() { cardSvc.mode = "peek" }],
      [settle, function() { cardSvc.mode = "compact" }],
      [settle, function() { cardSvc.mode = "expanded" }],
      [settle, function() { cardSvc.mode = "compact" }],
      [settle, function() {}]
    ], function() {
      root.check(base > 0, "no closed depth measured")
      root.check(card.closedDepths.length === 0, "the closed depth moved: " + base + " then " + card.closedDepths)
      root.done()
    })
  }

  // Bug: hovering the notch named its screen as the one to open on; leaving
  // before it opened kept that, so the next peek showed on that screen
  // instead of the focused one.
  function hoverLeaveForgetsScreen() {
    root.go(function*() {
      twoScreens.focusedScreenName = "B"
      multi.replaceConfig({})
      yield root.waitFor(function() { return multi.notches.length === 1 && multi.notches[0].copies.length === 1 })
      var copy = multi.notches[0].copies[0]
      copy.setPointer("notch", true, "A")
      copy.setPointer("notch", false, "")
      yield root.settle(multi)
      root.check(copy.mode !== "expanded", "opened without waiting for the hover delay")
      // A peek now shows where you work, not where the pointer touched.
      root.push("forget", true)
      yield root.settle(multi)
      root.check(copy.mode === "peek" && copy.screenName === "B", "the peek is " + copy.mode + " on '" + copy.screenName + "', not on B")
      multi.dismissPeek(null)
      root.clearActivities(multi)
    })
  }

  // Bug: the copy cleared the screen it opened on as it opened (onModeChanged
  // read `isOpen` before it updated), so an open notch followed the focus to
  // another screen instead of staying under the pointer.
  function openKeepsScreen() {
    root.go(function*() {
      twoScreens.focusedScreenName = "B"
      multi.replaceConfig({})
      yield root.waitFor(function() { return multi.notches.length === 1 && multi.notches[0].copies.length === 1 })
      var copy = multi.notches[0].copies[0]
      copy.setPointer("notch", true, "A")
      yield root.waitFor(function() { return copy.mode === "expanded" })
      yield root.settle(multi)
      root.check(copy.mode === "expanded" && copy.screenName === "A", "an open notch is on '" + copy.screenName + "', not under the pointer on A")
      // Focus moving does not take it away.
      twoScreens.focusedScreenName = "A"
      twoScreens.focusedScreenName = "B"
      yield root.settle(multi)
      root.check(copy.screenName === "A", "it followed focus to '" + copy.screenName + "'")
      copy.setPointer("notch", false, "")
      yield root.waitFor(function() { return copy.mode !== "expanded" })
      yield root.settle(multi)
      root.check(copy.screenName === "B", "closed, it did not go back to the focused screen: '" + copy.screenName + "'")
    })
  }

  // The timers live in the daemon: the round trip must not show. A pointer
  // on the card opens it after the hover delay, within a few frames of it;
  // leaving closes it after the close delay the same way.
  function hoverLatency() {
    root.go(function*() {
      twoScreens.focusedScreenName = "B"
      multi.replaceConfig({ hoverOpenDelay: 150, hoverCloseDelay: 150 })
      yield root.waitFor(function() { return multi.notches.length === 1 && multi.notches[0].copies.length === 1 })
      yield root.settle(multi)
      var copy = multi.notches[0].copies[0]
      // The moment the mode changes, not the next poll of a wait.
      var changedAt = 0
      copy.modeChanged.connect(function() { changedAt = Date.now() })
      var margin = 40
      var worst = 0
      for (var round = 0; round < 4; round++) {
        var t0 = Date.now()
        copy.setPointer("notch", true, "B")
        yield root.waitFor(function() { return copy.mode === "expanded" }, 2000)
        var opened = changedAt - t0
        t0 = Date.now()
        copy.setPointer("notch", false, "")
        yield root.waitFor(function() { return copy.mode !== "expanded" }, 2000)
        var closed = changedAt - t0
        worst = Math.max(worst, opened - 150, closed - 150)
        root.check(opened >= 148 && opened <= 150 + margin, "opened after " + opened + " ms; the delay is 150")
        root.check(closed >= 148 && closed <= 150 + margin, "closed after " + closed + " ms; the delay is 150")
        console.log("NOTE hover latency: open " + opened + " ms, close " + closed + " ms (the delay is 150 each)")
      }
      multi.replaceConfig({})
    })
  }

  // ------------------------------------------------------ reserved space
  // The daemon works out the space windows keep clear of the closed notch,
  // from the depths its card measures (`measure`); the surface holds what its
  // `state.reserve` says (NotchSurface, `reserveZones`). The rules (what
  // counts, the 400 ms shrink delay) are the daemon's own tests; these prove
  // the wiring, with a real card and the report it gives the daemon (a window
  // for it needs a compositor: NotchSurface only loads in the shell).
  QtObject {
    id: reserveComp
    property string focusedScreenName: ""
    function hasFullscreen(screen) { return false }
    readonly property bool supportsWorkspaces: false
    readonly property var knownWorkspaces: []
    function activeWorkspace(screen) { return null }
  }
  TestService {
    id: reserved
    ipcTarget: "notchd-qmltest-reserve"
    configPath: root.dir + "/reserve.json"
    compositor: reserveComp
    defaultModules: ["activities"]
  }
  NotchCard {
    id: reserveCard
    notch: reserved.notches.length > 0 ? reserved.notches[0].copies[0] : null
  }
  DepthReport {
    copy: reserved.notches.length > 0 ? reserved.notches[0].copies[0] : null
    notchCard: reserveCard
  }

  // Every depth the zones took, and the lowest and highest of them.
  QtObject {
    id: zoneWatch
    property var seen: []
    readonly property real lowest: Math.min.apply(null, seen)
    readonly property real highest: Math.max.apply(null, seen)
  }
  Connections {
    target: reserved
    function onReserveZonesChanged() {
      var depths = Object.keys(reserved.reserveZones).map(function(k) { return reserved.reserveZones[k] })
      zoneWatch.seen = zoneWatch.seen.concat(depths.length > 0 ? depths : [0])
    }
  }
  // The deepest zone now, 0 for none.
  function zoneDepth() {
    var zones = reserved.reserveZones
    return Object.keys(zones).reduce(function(a, k) { return Math.max(a, zones[k]) }, 0)
  }
  function reservedModule() { return reserved.moduleById("activities") }
  // The service with a quiet module and the card measured: the rest lip.
  function reserveStart(extra) {
    return function(resume) {
      reserved.replaceConfig(Object.assign({ modules: ["activities"], reserveSpace: true }, extra || {}))
      root.until(function() { return !!root.reservedModule() && reserved.notches.length === 1 && reserved.notches[0].copies.length === 1 }, function() {
        root.clearActivities(reserved)
        root.until(function() { return root.zoneDepth() > 0 }, function() {
          root.steps([[600, function() {}]], function() {
            zoneWatch.seen = [root.zoneDepth()]
            resume()
          })
        })
      })
    }
  }
  // Closed notch: compact (a module active) or rest.
  function closeAs(compact) {
    cardSvc.compactModule = compact ? fakeModule : null
    cardSvc.mode = compact ? "compact" : "rest"
  }
  function pushReserved(id, peek) {
    return reserved.daemonIpc("push", [JSON.stringify({ id: id, title: id, sticky: true, peek: peek })], "ok")
  }

  function reserveIgnoresOpenNotch() {
    root.go(function*() {
      yield root.reserveStart()
      var rest = root.zoneDepth()
      root.pushReserved("keep", false)
      yield root.waitFor(function() { return root.zoneDepth() > rest })
      var compact = root.zoneDepth()
      zoneWatch.seen = [compact]
      var copy = reserved.notches[0].copies[0]
      // A peek, then pinned open, then closed: the windows stay where they were.
      reserved.requestPeek(root.reservedModule(), 400)
      yield root.settle(reserved)
      root.check(copy.mode === "peek", "the peek did not show: " + copy.mode)
      yield root.wait(600)
      copy.open()
      yield root.settle(reserved)
      root.check(copy.mode === "expanded", "the notch did not open: " + copy.mode)
      yield root.wait(600)
      copy.close()
      yield root.waitFor(function() { return copy.mode === "compact" })
      yield root.wait(600)
      root.check(zoneWatch.lowest === compact && zoneWatch.highest === compact,
                 "compact: moved between " + zoneWatch.lowest + " and " + zoneWatch.highest + ", closed is " + compact)
      root.clearActivities(reserved)
      // From rest, the OSD peeks and the lip is all that is kept.
      yield root.wait(700)
      zoneWatch.seen = [root.zoneDepth()]
      reserved.requestPeek(root.reservedModule(), 400)
      yield root.wait(900)
      root.check(zoneWatch.lowest === zoneWatch.highest, "a peek from rest moved it between " + zoneWatch.lowest + " and " + zoneWatch.highest)
      root.check(root.zoneDepth() === rest, "rest reserves " + root.zoneDepth() + ", the lip is " + rest)
    })
  }

  function reserveGrowsAndShrinks() {
    root.go(function*() {
      yield root.reserveStart()
      var rest = root.zoneDepth()
      root.pushReserved("grow", false)
      yield root.waitFor(function() { return root.zoneDepth() > rest })
      var compact = root.zoneDepth()
      root.check(compact > rest, "did not grow at once: " + compact)
      root.clearActivities(reserved)
      yield root.wait(150)
      root.check(root.zoneDepth() === compact, "shrank before the delay: " + root.zoneDepth())
      yield root.waitFor(function() { return root.zoneDepth() === rest }, 1500)
      root.check(root.zoneDepth() === rest, "did not shrink after the delay: " + root.zoneDepth())
    })
  }

  function reserveBridgesModules() {
    root.go(function*() {
      yield root.reserveStart()
      var rest = root.zoneDepth()
      root.pushReserved("first", false)
      yield root.waitFor(function() { return root.zoneDepth() > rest })
      var compact = root.zoneDepth()
      zoneWatch.seen = [compact]
      root.clearActivities(reserved)
      yield root.wait(100)
      root.pushReserved("second", false)
      yield root.wait(700)
      root.check(zoneWatch.lowest === compact, "dipped to " + zoneWatch.lowest + " between modules, compact is " + compact)
    })
  }

  // ------------------------------------------------------------ edges
  // Moving a compact notch to a side edge swaps its view for the dot (the
  // fake module has no verticalCompact). For the length of the fade the old,
  // wide view is still loaded; measured on the new axis it would read as a
  // notch 130 px deep and push windows that far.
  function reserveFollowsEdge() {
    root.go(function*() {
      yield root.reserveStart()
      root.pushReserved("edge", false)
      yield root.wait(600)
      var top = root.zoneDepth()
      root.check(!!reserved.reserveZones["|top"], "no zone on the top edge: " + JSON.stringify(reserved.reserveZones))
      zoneWatch.seen = [top]
      reserved.setConfigValue("position", { edge: "left" })
      yield root.waitFor(function() { return !!reserved.reserveZones["|left"] })
      yield root.wait(700)
      root.check(!reserved.reserveZones["|top"], "the top zone stayed: " + JSON.stringify(reserved.reserveZones))
      root.check(root.zoneDepth() > 0 && zoneWatch.highest <= top,
                 "grew to " + zoneWatch.highest + " moving to the side; compact on top is " + top)
      reserved.setConfigValue("position", undefined)
      root.clearActivities(reserved)
    })
  }

  // Content never turns: on a side edge its width is the notch's depth
  // (width) and its height runs along the edge (height).
  function sideEdgeAxes() {
    // The fade (70 ms) swaps the content, then the spring (460 ms) runs.
    var settle = 800
    root.steps([
      [0, function() { cardSvc.edge = "right"; cardSvc.mode = "peek" }],
      [settle, function() {}]
    ], function() {
      root.check(card.width === 300 + card.nearPad + card.farPad, "width " + card.width + " for a 300 px wide peek")
      root.check(card.height === 70 + card.alongPad * 2, "height " + card.height + " for a 70 px tall peek")
      root.check(card.nearPad === card.farPad, "a wide peek sits off-center: " + card.nearPad + " / " + card.farPad)
      cardSvc.edge = "top"
      root.closeAs(false)
      root.done()
    })
  }

  // Dragging turns the card to each edge it passes; the space waits for the
  // drop instead of following it there and back.
  function reserveHeldWhileDragging() {
    root.go(function*() {
      yield root.reserveStart()
      var rest = root.zoneDepth()
      root.pushReserved("drag", false)
      yield root.waitFor(function() { return root.zoneDepth() > rest })
      var compact = root.zoneDepth()
      zoneWatch.seen = [compact]
      var copy = reserved.notches[0].copies[0]
      copy.setDragging(true)
      yield root.settle(reserved)
      root.clearActivities(reserved)
      yield root.wait(800)
      root.check(zoneWatch.lowest === compact && zoneWatch.highest === compact,
                 "moved between " + zoneWatch.lowest + " and " + zoneWatch.highest + " mid-drag; it was " + compact)
      copy.setDragging(false)
      yield root.waitFor(function() { return root.zoneDepth() === rest }, 1500)
      root.check(root.zoneDepth() === rest, "after the drop it reserves " + root.zoneDepth() + ", the lip is " + rest)
    })
  }

  // ------------------------------------------------------------- pointer
  // Real pointer events (QtTest) on a card in a window. The TestCase runs
  // no tests of its own; it only sends the events.
  TestCase { id: pointer; when: false }

  QtObject {
    id: controlModule
    property bool controlHovered: false
    readonly property Component expanded: Component {
      Item {
        implicitWidth: 300
        implicitHeight: 100
        // A control that takes hover, like the media buttons.
        MouseArea {
          anchors.fill: parent
          hoverEnabled: true
          onContainsMouseChanged: controlModule.controlHovered = containsMouse
        }
      }
    }
  }
  QtObject {
    id: pointerSvc
    readonly property string mode: "expanded"
    readonly property string edge: "top"
    readonly property var compactModule: null
    readonly property var peekModule: null
    readonly property var expandedModules: [controlModule]
    property int pins: 0
    property int closes: 0
    function togglePinned(screenName) { pins++ }
    function close() { closes++ }
  }
  FloatingWindow {
    id: pointerWin
    implicitWidth: 500
    implicitHeight: 400
    NotchCard {
      id: pointerCard
      notch: pointerSvc
      draggable: true
      x: 50
      property int started: 0
      property int moved: 0
      property int ended: 0
      onDragStarted: started++
      onDragMoved: function(p) { moved++ }
      onDragEnded: ended++
      function reset() { started = 0; moved = 0; ended = 0; pointerSvc.pins = 0 }
    }
  }
  // A spot on the card outside its content: the open side's padding.
  function padX() { return pointerCard.width / 2 }
  function padY() { return pointerCard.height - 4 }

  // Bug: the hover was tracked on an item beside the card, and any control
  // inside the card that takes hover took the pointer from it. Crossing the
  // media buttons or the gear closed the notch (after the close delay) and
  // opened it again: it flickered.
  function hoverOverControls() {
    root.steps([
      [600, function() { pointer.mouseMove(pointerCard, root.padX(), root.padY()) }],
      [50, function() {
        root.check(pointerCard.hovered === true, "not hovered on the card's padding")
        pointer.mouseMove(pointerCard, pointerCard.width / 2, pointerCard.height / 2)
      }],
      [50, function() {
        root.check(controlModule.controlHovered, "the pointer is not on the control (test setup)")
        root.check(pointerCard.hovered === true, "lost the hover over a control inside the card")
        pointer.mouseMove(pointerCard, root.padX(), root.padY())
      }],
      [50, function() { root.check(pointerCard.hovered === true, "not hovered back on the padding") }]
    ], function() { root.done() })
  }

  function quickClickPins() {
    pointerCard.reset()
    pointer.mouseClick(pointerCard, root.padX(), root.padY())
    root.steps([[50, function() {}]], function() {
      root.check(pointerSvc.pins === 1, "a click pinned " + pointerSvc.pins + " times")
      root.check(pointerCard.started === 0, "a click started a drag")
      root.done()
    })
  }

  function quickPullNoDrag() {
    pointerCard.reset()
    pointer.mousePress(pointerCard, root.padX(), root.padY())
    root.steps([
      [40, function() { pointer.mouseMove(pointerCard, root.padX() + 40, root.padY() + 60, -1, Qt.LeftButton) }],
      [40, function() { pointer.mouseMove(pointerCard, root.padX() + 80, root.padY() + 120, -1, Qt.LeftButton) }],
      [400, function() { pointer.mouseRelease(pointerCard, root.padX() + 80, root.padY() + 120) }],
      [50, function() {}]
    ], function() {
      root.check(pointerCard.started === 0, "moving straight away dragged the notch")
      root.done()
    })
  }

  function holdThenDrag() {
    pointerCard.reset()
    pointer.mousePress(pointerCard, root.padX(), root.padY())
    root.steps([
      [pointerCard.holdToDrag + 150, function() {
        root.check(pointerCard.started === 1, "holding did not pick the notch up")
        pointer.mouseMove(pointerCard, root.padX() + 40, root.padY() + 60, -1, Qt.LeftButton)
      }],
      [40, function() { pointer.mouseMove(pointerCard, root.padX() + 80, root.padY() + 120, -1, Qt.LeftButton) }],
      [40, function() { pointer.mouseRelease(pointerCard, root.padX() + 80, root.padY() + 120) }],
      [50, function() {}]
    ], function() {
      root.check(pointerCard.moved >= 1, "the drag reported no move")
      root.check(pointerCard.ended === 1, "the drop ended " + pointerCard.ended + " drags")
      root.check(!pointerCard.dragging, "still dragging after the drop")
      root.check(pointerSvc.pins === 0, "the drop also pinned the notch")
      root.done()
    })
  }

  // ------------------------------------------------------------- notches
  readonly property var twoNotches: ({
    notches: [{ id: "main", modules: ["activities"], position: { screen: "all" } },
              { id: "side", modules: [], position: { edge: "left" } }]
  })
  // root.twoNotches loaded into the multi service, its copies made, focus on B.
  function withTwoNotches(fn) {
    twoScreens.focusedScreenName = "B"
    multi.replaceConfig(root.twoNotches)
    root.until(function() {
      var main = multi.notchById("main"), side = multi.notchById("side")
      return !!main && !!side && main.copies.length === 2 && side.copies.length === 1
    }, function() { root.settle(multi)(fn) })
  }
  function push(id, peek) {
    return multi.daemonIpc("push", [JSON.stringify({ id: id, title: id, sticky: true, peek: peek })], "ok")
  }

  // Bug: one notch window moved between screens. With media showing on one
  // monitor, changing the volume on another took the notch away from it.
  // A notch on every screen now peeks on the focused one only.
  function peekOnFocusedCopy() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var main = multi.notchById("main")
      root.check(!!main && main.copies.length === 2, "main has " + (main ? main.copies.length : 0) + " copies, not one per screen")
      root.check(root.push("build", false) === "ok", "the daemon is not there")
      yield root.settle(multi)
      root.check(main.copyOn("A").mode === "compact" && main.copyOn("B").mode === "compact",
                 "a quiet push left " + main.copyOn("A").mode + " / " + main.copyOn("B").mode)
      twoScreens.focusedScreenName = "B"
      root.push("deploy", true)
      yield root.settle(multi)
      root.check(main.copyOn("B").mode === "peek", "the focused copy shows " + main.copyOn("B").mode)
      root.check(main.copyOn("A").mode === "compact", "the copy on the other screen shows " + main.copyOn("A").mode)
      root.check(multi.peeking(multi.moduleById("activities")), "the service does not report the peek")
      multi.dismissPeek(null)
      twoScreens.focusedScreenName = "A"
      root.push("lint", true)
      yield root.settle(multi)
      root.check(main.copyOn("A").mode === "peek" && main.copyOn("B").mode === "compact",
                 "focus on A: " + main.copyOn("A").mode + " / " + main.copyOn("B").mode)
      multi.dismissPeek(null)
      root.clearActivities(multi)
    })
  }

  // Bug: the visitor's copies (a fullscreen window per screen, hidden while
  // closed) showed the compact view of whatever was active. Music playing
  // kept the equalizer drawing frames in windows nobody could see.
  function visitorRestsWhileClosed() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var visitor = multi.visitor
      root.push("quiet", false)
      yield root.settle(multi)
      root.check(visitor.copies.length === 2, "the visitor has " + visitor.copies.length + " copies, not one per screen")
      visitor.copies.forEach(function(c) {
        root.check(c.mode === "rest", "a closed visitor copy on " + c.fixedScreen + " shows " + c.mode)
      })
      twoScreens.focusedScreenName = "B"
      multi.requestPeek(multi.moduleById("activities"), 2000)
      yield root.settle(multi)
      root.check(visitor.copies.some(function(c) { return c.mode === "peek" }) || multi.peeking(multi.moduleById("activities")),
                 "nothing peeks")
      multi.dismissPeek(null)
      root.clearActivities(multi)
    })
  }

  // The visitor's copies held a fullscreen window each (its buffers, ~25 MB
  // per screen) while hidden. A guest's window now goes when it closes,
  // after the daemon's linger, so the close animation ends and a peek soon after
  // reuses it.
  function visitorWindowOnlyWhileShown() {
    root.go(function*() {
      var visitor = multi.visitor
      // The daemon's guest windows linger 5 s after they close: shorter here
      // (a hook of the test daemon, NOTCHD_TEST_HOOKS).
      var saved = multi.config
      // A notch kept on A, focus on B: a peek there goes to the visitor.
      multi.replaceConfig({ modules: ["activities"], position: { screen: "A" } })
      twoScreens.focusedScreenName = "B"
      yield root.nested(function(resume) {
        multi.client.ipc("test-linger", ["300"], function(reply) {
          root.check(reply.t === "ok", "the daemon has no test-linger hook: " + JSON.stringify(reply))
          resume()
        })
      })
      yield root.settle(multi)
      root.check(visitor.copies.every(function(c) { return !c.needsWindow }),
                 "a closed visitor copy wants a window: " + visitor.copies.map(function(c) { return c.mode }))
      root.check(multi.notches[0].copies.every(function(c) { return c.needsWindow }), "a notch of the config lost its window")
      // The module's peek view is the daemon's, and Activities has none until
      // something is pushed: a peek of an empty one is no peek.
      root.push("linger", false)
      yield root.settle(multi)
      multi.requestPeek(multi.moduleById("activities"), 400)
      yield root.settle(multi)
      var copy = visitor.copyOn("B")
      root.check(!!copy && copy.mode === "peek", "the visitor does not peek on B: " + (copy ? copy.mode : "no copy"))
      root.check(!!copy && copy.needsWindow, "a peeking visitor copy has no window")
      yield root.waitFor(function() { return copy.mode !== "peek" })
      root.check(copy.needsWindow, "the window went before the notch finished closing")
      yield root.waitFor(function() { return !copy.needsWindow })
      root.check(!copy.needsWindow, "the window stayed after the linger")
      yield root.nested(function(resume) { multi.client.ipc("test-linger", ["5000"], function() { resume() }) })
      root.clearActivities(multi)
      multi.replaceConfig(saved)
    })
  }

  function peekOnlyWhereShown() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var side = multi.notchById("side")
      root.check(!!side && side.copies.length === 1 && !side.everyScreen, "side should be one copy that moves")
      root.push("test", true)
      yield root.settle(multi)
      root.check(side.copies[0].mode === "rest", "a notch without the module shows " + side.copies[0].mode)
      root.check(side.edge === "left", "side hangs from " + side.edge)
      multi.dismissPeek(null)
      root.clearActivities(multi)
    })
  }

  // The notches, their copies and the modules are made once: a config edit
  // (a slider moving) must not make them again, losing what they show.
  function survivesConfigEdits() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var main = multi.notchById("main")
      var copy = main.copyOn("A")
      var module = multi.moduleById("activities")
      root.push("edit", false)
      copy.open()
      yield root.settle(multi)
      root.check(copy.pinned, "the copy did not open (test setup)")
      multi.setConfigValue("hoverOpenDelay", 50)
      multi.setConfigValue("notches.0.position.edge", "bottom")
      yield root.settle(multi)
      root.check(multi.notchById("main") === main, "the notch was made again")
      root.check(main.copyOn("A") === copy, "the copy was made again")
      root.check(copy.pinned, "the copy forgot it was pinned")
      root.check(multi.moduleById("activities") === module, "the module was loaded again")
      root.check(module.active && !!module.compact, "the module lost its activities")
      root.check(main.edge === "bottom", "the edit did not apply: " + main.edge)
      copy.close()
      root.clearActivities(multi)
    })
  }

  function addAndRemoveNotch() {
    root.go(function*() {
      var start = { modules: ["activities"], hoverOpenDelay: 40 }
      multi.replaceConfig(start)
      yield root.settle(multi)
      root.check(multi.notches.length === 1, "one notch to start, not " + multi.notches.length)
      var at = multi.addNotch()
      yield root.settle(multi)
      root.check(at === 1 && multi.notches.length === 2, "added at " + at + ", " + multi.notches.length + " notches")
      root.check(JSON.stringify(multi.config.notches[0]) === "{}", "the first notch should inherit it all: " + JSON.stringify(multi.config.notches))
      root.check(multi.notches[1].edge !== multi.notches[0].edge, "the new notch hangs on the same edge")
      root.check(multi.notches[1].modules.length === 0, "the new notch starts with modules")
      multi.removeNotch(1)
      yield root.settle(multi)
      root.check(multi.notches.length === 1, "removed, " + multi.notches.length + " notches")
      root.check(JSON.stringify(multi.config) === JSON.stringify(start), "the config came back as " + JSON.stringify(multi.config))
    })
  }

  // Bug: the notch kept on the ultrawide (position.screen) showing music
  // left it when the volume changed on the laptop screen. Now it stays and
  // shows the volume there; on the laptop screen, where no notch with the
  // OSD hangs, the visitor shows it once at the top, however many notches
  // kept elsewhere have the module.
  function peekBesideHomedNotch() {
    root.go(function*() {
      var visitor = multi.visitor
      multi.replaceConfig({ modules: ["activities"], position: { screen: "A" },
                            notches: [{}, { id: "side", position: { edge: "left", screen: "A" } }] })
      twoScreens.focusedScreenName = "B"
      yield root.waitFor(function() { return multi.notches.length === 2 && visitor.copies.length === 2 })
      yield root.settle(multi)
      var music = multi.notchById("main"), side = multi.notchById("side")
      root.check(music.copies.length === 1 && music.copies[0].screenName === "A", "the notch kept on A is not one copy on A")
      root.check(visitor.copies.length === 2 && visitor.copyOn("B").offWorkspace, "the visitor shows while nothing peeks")
      root.push("music", false)
      root.push("volume", true)
      yield root.settle(multi)
      root.check(music.copies[0].mode === "peek" && music.copies[0].screenName === "A",
                 "the notch on A shows " + music.copies[0].mode + " on " + music.copies[0].screenName)
      root.check(side.copies[0].mode === "peek" && side.copies[0].screenName === "A", "the side notch left A")
      var guest = visitor.copyOn("B")
      root.check(guest.mode === "peek" && !guest.offWorkspace && guest.edge === "top",
                 "no peek at the top of B: " + guest.mode + " on " + guest.edge)
      root.check(visitor.copyOn("A").mode !== "peek", "the visitor also peeked on A, where the notches are")
      multi.dismissPeek(null)
      yield root.settle(multi)
      root.check(music.copies[0].mode === "compact", "after the peek the notch on A shows " + music.copies[0].mode)
      root.check(visitor.copyOn("B").offWorkspace, "the visitor stays after the peek")
      twoScreens.focusedScreenName = "A"
      root.push("again", true)
      yield root.settle(multi)
      root.check(visitor.copyOn("A").mode !== "peek" && visitor.copyOn("B").mode !== "peek",
                 "the visitor peeked with focus where the notches are")
      multi.dismissPeek(null)
      // A notch that follows focus peeks on B itself: no visitor.
      multi.setConfigValue("notches.1.position.screen", "")
      twoScreens.focusedScreenName = "B"
      root.push("follow", true)
      yield root.settle(multi)
      root.check(visitor.copyOn("B").mode !== "peek", "the visitor peeked beside a notch that follows focus")
      multi.dismissPeek(null)
      root.clearActivities(multi)
    })
  }

  // Bug: brightness changes on the focused monitor only, yet its OSD opened
  // in the notches of the other monitors too. A module with
  // peekOnFocusedScreen (the OSD) peeks once: in the first notch on the
  // focused screen that shows it, else at the top, in the visitor.
  function osdPeekOnFocusedScreen() {
    var visitor = multi.visitor
    function modes() {
      return multi.notches.map(function(n) { return n.notchId + ":" + n.copies[0].mode }).join(" ")
        + " visitor:" + visitor.copies.map(function(c) { return c.mode }).join("/")
    }
    function peeking() {
      return multi.notches.filter(function(n) { return n.copies[0].mode === "peek" }).map(function(n) { return n.notchId })
        .concat(visitor.copies.filter(function(c) { return c.mode === "peek" }).map(function(c) { return "visitor@" + c.fixedScreen }))
        .join(" ")
    }
    root.go(function*() {
      multi.replaceConfig({ modules: ["focus"],
                            notches: [{ position: { screen: "A" } }, { id: "a2", position: { edge: "left", screen: "A" } },
                                      { id: "b", position: { screen: "B" } }] })
      twoScreens.focusedScreenName = "B"
      yield root.waitFor(function() { return multi.notches.length === 3 && visitor.copies.length === 2 && !!multi.moduleById("focus") })
      var module = multi.moduleById("focus")
      yield root.settle(multi)
      module.requestPeek(2000)
      yield root.settle(multi)
      root.check(peeking() === "b", "focus on B: " + modes())
      multi.dismissPeek(null)
      twoScreens.focusedScreenName = "A"
      module.requestPeek(2000)
      yield root.settle(multi)
      root.check(peeking() === "main", "focus on A: " + modes())
      multi.dismissPeek(null)
      multi.setConfigValue("notches.2.position.screen", "A")
      twoScreens.focusedScreenName = "B"
      module.requestPeek(2000)
      yield root.settle(multi)
      root.check(peeking() === "visitor@B", "nothing on B: " + modes())
      multi.dismissPeek(null)
    })
  }

  // Standalone modules (the OSD) are never in a notch, even one whose
  // `modules` list them: they load on their own (unless switched off) and
  // peek in a notch of their own, on the focused screen, where
  // "<id>.position" says.
  TestService {
    id: standaloneSvc
    ipcTarget: "notchd-qmltest-standalone"
    configPath: root.dir + "/standalone.json"
    compositor: twoScreens
    screens: [{ name: "A" }, { name: "B" }]
    defaultModules: ["activities"]
    hostModules: ({ osd: Qt.resolvedUrl("hosts/omarchy/modules/osd.qml") })
  }

  function standaloneOwnPlace() {
    var svc = standaloneSvc
    function own() { return svc.standaloneNotch("osd") }
    function osd() { return svc.moduleById("osd") }
    root.go(function*() {
      // Bug: in the Omarchy shell the OSD did not load at startup; the module
      // list was rebuilt before the standalone ids followed the module files.
      yield root.waitFor(function() { return !!osd() && !!own() })
      root.check(!!osd() && !!own(), "the OSD did not load at startup")
      svc.replaceConfig({ modules: ["osd", "activities"], position: { screen: "all" } })
      twoScreens.focusedScreenName = "B"
      yield root.settle(svc)
      root.check(!!osd(), "the OSD did not load")
      root.check(svc.isStandalone("osd"), "the OSD is not standalone")
      root.check(svc.notches[0].modules.indexOf(osd()) === -1, "the notch took the OSD")
      root.check(!!own() && own().edge === "top" && own().placement.offset === 0.5, "no OSD place at the top center")
      svc.daemonIpc("osd", ["show", JSON.stringify({ icon: "volume", value: "40" })], "ok")
      yield root.settle(svc)
      root.check(own().copyOn("B").mode === "peek", "no peek on B: " + own().copyOn("B").mode)
      root.check(own().copyOn("A").mode !== "peek", "the OSD peeked on A too")
      root.check(svc.notches[0].copies.every(function(c) { return c.mode !== "peek" }), "a notch peeked")
      svc.dismissPeek(null)
      svc.setConfigValue("osd.position", { edge: "bottom", offset: 0.2 })
      twoScreens.focusedScreenName = "A"
      yield root.settle(svc)
      root.check(own().edge === "bottom" && own().placement.offset === 0.2, "position: " + JSON.stringify(own().placement))
      svc.daemonIpc("osd", ["show", JSON.stringify({ icon: "brightness", value: "70" })], "ok")
      yield root.settle(svc)
      root.check(own().copyOn("A").mode === "peek", "no peek on A: " + own().copyOn("A").mode)
      svc.dismissPeek(null)
      svc.setConfigValue("osd.enabled", false)
      yield root.settle(svc)
      yield root.waitFor(function() { return osd() === null && own() === null })
      root.check(osd() === null && own() === null, "switched off, the OSD still loads")
      root.check(svc.moduleCatalog.some(function(m) { return m.moduleId === "osd" && m.standalone }), "the OSD left the catalog")
      root.check(svc.settingsSchema.some(function(f) { return f.key === "osd.enabled" && f.page === "modules/osd" }), "no switch on its page")
      var lists = svc.settingsSchema.filter(function(f) { return f.type === "moduleCatalog" }).map(function(f) { return f.part })
      root.check(JSON.stringify(lists) === JSON.stringify(["notch", "agent", "terminal", "standalone"]),
                 "the Modules page does not list agents and standalone modules apart: " + JSON.stringify(lists))
      root.check(svc.moduleCatalog.some(function(m) { return m.moduleId === "claude" && m.agent && !m.standalone }), "claude is no agent")
      root.check(svc.reportedCatalog.every(function(m) { return m.moduleId === "osd" }), "the host reported more than its own modules: " + JSON.stringify(svc.reportedCatalog.map(function(m) { return m.moduleId })))
      root.check(svc.moduleSetting("codex", "command") === "codex","codex's command default: " + svc.moduleSetting("codex", "command"))
      root.check(svc.agentCommand("media", "toggle").indexOf("no agent media") === 0, "media took an agent command")
      svc.setConfigValue("osd.enabled", undefined)
      yield root.settle(svc)
      yield root.waitFor(function() { return !!osd() && !!own() })
      root.check(!!osd() && !!own(), "switched back on, the OSD did not load")
      svc.replaceConfig({})
    })
  }

  // A notch's keys were split between General and Modules, each with its own
  // picker. The Notches page lists the notches; every field for the picked
  // notch's keys sits on its own page, "notch/<index>"; the other pages only
  // have keys for every notch.
  function notchSettingsOnOnePage() {
    multi.replaceConfig({ notches: [{ modules: ["activities"] }, { id: "side", position: { edge: "bottom" } }] })
    multi.settingsPage = "notch/1"
    root.check(multi.settingsNotch === 1, "showing notch/1 edits notch " + multi.settingsNotch)
    // The schema is the daemon's: it follows the config and the page once
    // they have been through it.
    root.until(function() {
      return multi.settingsSchema.some(function(f) { return f.key === "notches.1.modules" })
        && multi.settingsPages.some(function(p) { return p.id === "notch/1" && p.label === "side" })
    }, function() {
      var schema = multi.settingsSchema
      var lists = schema.filter(function(f) { return f.type === "notches" })
      root.check(lists.length === 1 && lists[0].page === "notch", "lists: " + JSON.stringify(lists.map(function(f) { return f.page })))
      schema.forEach(function(f) {
        var own = !!f.key && f.key.indexOf("notches.") === 0
        if (own) root.check(f.page === "notch/1", f.key + " is on " + f.page)
        else if (f.key && f.key !== "notches") root.check(f.page.indexOf("notch") !== 0, f.key + " applies to every notch but is on " + f.page)
      })
      root.check(schema.some(function(f) { return f.key === "notches.1.modules" }), "the notch's modules are missing")
      root.check(multi.settingsPages.some(function(p) { return p.id === "notch/1" && p.label === "side" }), "no page for side")
      multi.settingsPage = "general"
      multi.openSettings(0)
      root.check(multi.settingsPage === "notch/0" && multi.settingsNotch === 0, "openSettings(0) opened " + multi.settingsPage)
      multi.closeSettings()
      multi.replaceConfig({})
      root.done()
    })
  }

  // A module's settings were read from the loaded module, so one no notch
  // showed had no page and no gear. They come from its JSON file now.
  function unloadedModuleSettings() {
    root.go(function*() {
      // The daemon may still be answering the case before: its settings
      // (the schema, the catalog) are in once it has answered a request.
      // The default config, whatever case ran before on this service.
      multi.replaceConfig({})
      yield root.settle(multi)
      yield root.waitFor(function() { return !!multi.moduleById("activities") && multi.moduleById("timer") === null })
      root.check(multi.moduleById("timer") === null, "timer is loaded (test setup)")
      var fields = multi.settingsSchema.filter(function(f) { return f.page === "modules/timer" })
      root.check(fields.some(function(f) { return f.key === "timer.focus" }), "timer fields: " + JSON.stringify(fields.map(function(f) { return f.key })))
      root.check(multi.settingsPages.some(function(p) { return p.id === "modules/timer" && p.label === "Timer" }), "no page for timer")
      root.check(multi.moduleSetting("timer", "focus") === 25, "default: " + multi.moduleSetting("timer", "focus"))
      multi.setConfigValue("timer.focus", 50)
      root.check(multi.moduleSetting("timer", "focus") === 50, "set: " + multi.moduleSetting("timer", "focus"))
      var media = multi.moduleCatalog.filter(function(m) { return m.moduleId === "media" })[0]
      root.check(!!media && media.title === "Now playing", "media: " + JSON.stringify(media))
      // A loaded module's title is the daemon's own message, after the config's.
      yield root.waitFor(function() { var m = multi.moduleById("activities"); return !!m && m.title === "Activities" })
      var activities = multi.moduleById("activities")
      root.check(!!activities && activities.title === "Activities", "loaded title: " + (activities ? activities.title : ""))
      multi.replaceConfig({})
    })
  }

  TestService {
    id: resourced
    ipcTarget: "notchd-qmltest-resources"
    configPath: root.dir + "/resources.json"
    compositor: twoScreens
    defaultModules: ["resources"]
  }

  // The daemon reads /proc and df and hands the module the numbers: the
  // expanded view shows them (a CPU figure needs two readings, 2 s apart).
  function resourcesRead() {
    root.go(function*() {
      yield root.waitFor(function() { return !!resourced.moduleById("resources") })
      var m = resourced.moduleById("resources")
      root.check(!!m, "the module did not load")
      yield root.waitFor(function() {
        var t = root.texts(m.tree("expanded"))
        return t.length > 0 && t.indexOf("–") === -1 && t.some(function(x) { return /^\d+ cores?$/.test(x) })
      }, 6000)
      var t = root.texts(m.tree("expanded"))
      root.check(t.indexOf("–") === -1, "a figure is missing: " + JSON.stringify(t))
      root.check(t.some(function(x) { return /^\d+ cores?$/.test(x) }), "cores: " + JSON.stringify(t))
      root.check(t.some(function(x) { return /^[\d.]+ of [\d.]+ (GiB|TiB)$/.test(x) }), "memory: " + JSON.stringify(t))
      root.check(t.some(function(x) { return / · \/$/.test(x) }), "disk: " + JSON.stringify(t))
      root.check(m.active && !!m.expanded, "no expanded view")
    })
  }

  // By default the closed notch always shows the three. With `compact` at
  // "short" it rests while every resource is fine; one running short peeks
  // once and takes the compact slot until it drops back. The setting is on
  // the module's own page, under the module's id. The readings are demo
  // values (the daemon's test hook), as the renders use.
  function resourcesCompactWhenShort() {
    root.go(function*() {
      yield root.waitFor(function() { return !!resourced.moduleById("resources") && resourced.notches.length > 0 })
      var m = resourced.moduleById("resources")
      var notch = resourced.notches[0]
      function reading(cpu, memory, disk) {
        return root.ask(resourced, "test-resources", [String(cpu), String(memory), String(disk)], function(a) { root.check(a === "ok", "reading: " + a) })
      }
      function shown() { return root.texts(m.tree("compact")).filter(function(x) { return /%$/.test(x) }).length }
      resourced.replaceConfig({})
      yield reading(0.2, 0.5, 0.4)
      yield root.settle(resourced)
      root.check(notch.compactModule === m && shown() === 3, "not shown by default: " + JSON.stringify(root.texts(m.tree("compact"))))
      var field = resourced.settingsSchema.filter(function(f) { return f.key === "resources.compact" })[0]
      root.check(!!field && field.page === "modules/resources", "field: " + JSON.stringify(field))

      resourced.setConfigValue("resources.compact", "short")
      yield root.settle(resourced)
      root.check(notch.compactModule === null, "the notch shows resources with nothing short")
      yield reading(0.2, 0.95, 0.4)
      yield root.settle(resourced)
      root.check(notch.compactModule === m && shown() === 1, "memory short, no compact view")
      root.check(resourced.peeking(m), "memory short, no peek")
      resourced.dismissPeek(null)
      yield reading(0.2, 0.96, 0.4)
      yield root.settle(resourced)
      root.check(!resourced.peeking(m), "peeked again while still short")
      yield reading(0.2, 0.8, 0.4)
      yield root.settle(resourced)
      root.check(notch.compactModule === null, "still compact after memory dropped")
      yield reading(0.99, 0.8, 0.4)
      yield root.settle(resourced)
      root.check(notch.compactModule === null, "one CPU burst counted as short")
      resourced.replaceConfig({})
    })
  }

  TestService {
    id: timed
    ipcTarget: "notchd-qmltest-timer"
    configPath: root.dir + "/timer.json"
    compositor: twoScreens
    defaultModules: ["timer"]
  }

  // Idle, the notch rests and the timer only offers its quick starts when
  // open. Started, it takes the compact slot with the time left; at the end
  // it peeks (urgent, so even over the open notch) and says so until stopped.
  function timerRunsAndEnds() {
    root.go(function*() {
      yield root.waitFor(function() { return !!timed.moduleById("timer") && timed.notches.length > 0 })
      var m = timed.moduleById("timer")
      var notch = timed.notches[0]
      timed.replaceConfig({ timer: { notify: false } })
      yield root.settle(timed)
      root.check(!!m && m.active && notch.compactModule === null, "idle timer is in the closed notch")
      yield root.ask(timed, "timer", ["2s Tea"], function(a) { root.check(a === "ok", "start refused: " + a) })
      yield root.settle(timed)
      var clock = root.texts(m.tree("compact"))[0]
      root.check(notch.compactModule === m && !!m.compact, "running timer not in the closed notch")
      root.check(clock === "0:02" || clock === "0:01", "clock: " + clock)
      timed.dismissPeek(null)
      yield root.wait(2600)
      yield root.settle(timed)
      root.check(timed.peeking(m), "no peek at the end")
      root.check(notch.compactModule === m && root.texts(m.tree("compact"))[0] === "Time's up", "done text: " + JSON.stringify(root.texts(m.tree("compact"))))
      timed.dismissPeek(null)
      yield root.ask(timed, "timer", ["stop"], function(a) { root.check(a === "ok", "stop refused: " + a) })
      yield root.settle(timed)
      root.check(notch.compactModule === null, "stopped timer still in the closed notch")
      timed.replaceConfig({})
    })
  }

  function timerCommands() {
    root.go(function*() {
      yield root.waitFor(function() { return !!timed.moduleById("timer") })
      var m = timed.moduleById("timer")
      function say(command, want) {
        return root.ask(timed, "timer", [command], function(a) {
          if (want instanceof RegExp) root.check(want.test(a), command + " answered " + a)
          else root.check(a === want, command + " answered " + a + ", not " + want)
        })
      }
      function expanded() { return root.texts(m.tree("expanded")) }
      timed.replaceConfig({ timer: { notify: false, focus: 50 } })
      yield say("pause", "no timer running")
      yield say("soon", /^usage/)
      yield say("pomodoro", "ok")
      yield root.settle(timed)
      root.check(expanded().indexOf("Focus · 1 of 4") !== -1 && expanded().indexOf("50:00") !== -1, "focus setting ignored: " + JSON.stringify(expanded()))
      yield say("toggle", "ok")
      yield root.settle(timed)
      root.check(expanded().indexOf("Focus · 1 of 4 · paused") !== -1, "toggle did not pause: " + JSON.stringify(expanded()))
      yield say("+5m", "ok")
      yield root.settle(timed)
      root.check(expanded().indexOf("55:00") !== -1, "added time lost while paused: " + JSON.stringify(expanded()))
      yield say("skip", "ok")
      yield root.settle(timed)
      root.check(expanded().indexOf("Short break") !== -1 && expanded().indexOf("5:00") !== -1, "skip: " + JSON.stringify(expanded()))
      var keys = timed.settingsSchema.filter(function(f) { return f.page === "modules/timer" }).map(function(f) { return f.key })
      root.check(keys.indexOf("timer.focus") !== -1 && keys.indexOf("timer.notify") !== -1, "settings: " + keys)
      yield say("stop", "ok")
      timed.dismissPeek(null)
      timed.replaceConfig({})
    })
  }

  TestService {
    id: played
    ipcTarget: "notchd-qmltest-media"
    configPath: root.dir + "/media.json"
    compositor: twoScreens
    defaultModules: ["media"]
  }

  // The test hook gives the daemon demo players in place of MPRIS: the
  // playing track takes the closed notch (title, artist), a new one peeks,
  // and once nothing plays and the pause grace is not running the notch
  // rests.
  function mediaShowsTheTrack() {
    root.go(function*() {
      yield root.waitFor(function() { return !!played.moduleById("media") && played.notches.length > 0 })
      var m = played.moduleById("media")
      var notch = played.notches[0]
      function players(title, playing) {
        return JSON.stringify({ players: [{ id: "p", name: "Demo", playing: playing, title: title, artist: "Neon Harbor",
          album: "", artUrl: "", length: 214, position: 92, canPlay: true, canPause: true, canNext: true, canPrev: true, canSeek: true }] })
      }
      root.check(!m.active && notch.compactModule === null, "media active with no player")
      yield root.ask(played, "test-media", [players("Night Drive", true)], function(a) { root.check(a === "ok", "hook: " + a) })
      yield root.settle(played)
      var compact = root.texts(m.tree("compact"))
      root.check(m.active && notch.compactModule === m, "the playing track is not in the closed notch")
      root.check(compact.indexOf("Night Drive") !== -1 && compact.indexOf(" · Neon Harbor") !== -1, "compact: " + JSON.stringify(compact))
      yield root.waitFor(function() { return played.peeking(m) }, 3000)
      root.check(played.peeking(m), "a new track did not peek")
      played.dismissPeek(null)
      var expanded = root.texts(m.tree("expanded"))
      root.check(expanded.indexOf("Demo") !== -1 && expanded.some(function(t) { return /^1:(3[2-9])$/.test(t) }) && expanded.some(function(t) { return /^-2:0[0-2]$/.test(t) }), "expanded: " + JSON.stringify(expanded))
      yield root.ask(played, "test-media", [JSON.stringify({ players: [] })], function(a) { root.check(a === "ok", "hook: " + a) })
      yield root.settle(played)
      root.check(!m.active && notch.compactModule === null, "media still shown with no player")
      played.replaceConfig({})
    })
  }

  // ---------------------------------------------------------- navigation
  // Navigation (IPC `navigate`, Navigate.js): every notch of the config
  // numbered, the selected one's copy on the focused screen open as a
  // preview, the keys driving it through navKey().
  function navigationPreview() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var main = multi.notchById("main")
      var side = multi.notchById("side")
      if (!root.check(!!main && !!side, "the two notches did not load")) return
      root.check(multi.navigate() === "ok", "navigate was refused")
      yield root.settle(multi)
      root.check(multi.navigating, "navigate did not start navigation")
      root.check(main.copyOn("B").preview && main.copyOn("B").mode === "expanded",
                 "the first notch on the focused screen does not preview: " + main.copyOn("B").mode)
      root.check(!main.copyOn("B").pinned, "the preview pinned the notch")
      root.check(main.copyOn("A").mode !== "expanded", "the copy on the other screen opened too")
      root.check(main.copyOn("A").navShown && main.copyOn("A").navNumber === 1, "main is not numbered 1")
      root.check(side.copies[0].navShown && side.copies[0].navNumber === 2, "side is not numbered 2")
      root.check(!multi.visitor.copies.some(function(c) { return c.navShown }), "the visitor takes part in navigation")
      root.navKey(multi, "next")
      yield root.settle(multi)
      root.check(multi.navSelected === 1 && side.copies[0].mode === "expanded", "next did not move to the second notch")
      root.check(main.copyOn("B").mode !== "expanded", "the first notch stayed open after moving on")
      root.navKey(multi, "next")
      yield root.settle(multi)
      root.check(multi.navSelected === 0, "next did not wrap around")
      root.navKey(multi, "2")
      yield root.settle(multi)
      root.check(multi.navSelected === 1, "2 did not select the second notch")
      root.navKey(multi, "leave")
      yield root.settle(multi)
      root.check(!multi.navigating, "Escape did not end navigation")
      root.check(!side.copies[0].isOpen && !main.copyOn("B").isOpen, "Escape left a notch open")
      root.check(!side.copies[0].navShown, "the highlight stayed after Escape")
      multi.navigate()
      yield root.settle(multi)
      multi.navigate()
      yield root.settle(multi)
      root.check(!multi.navigating, "navigate again did not end navigation")
    })
  }

  function navigationOpen() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var main = multi.notchById("main")
      var side = multi.notchById("side")
      if (!root.check(!!main && !!side, "the two notches did not load")) return
      main.open()
      yield root.settle(multi)
      root.check(main.copyOn("B").pinned, "main did not open (test setup)")
      multi.navigate()
      yield root.settle(multi)
      root.check(multi.navSelected === 0, "navigation did not start on the open notch")
      root.navKey(multi, "next")
      yield root.settle(multi)
      root.check(main.copyOn("B").mode !== "expanded", "the pinned notch stayed open with another one selected")
      root.navKey(multi, "previous")
      root.navKey(multi, "leave")
      yield root.settle(multi)
      root.check(main.copyOn("B").pinned && main.copyOn("B").mode === "expanded", "Escape did not open the pinned notch again")
      multi.navigate()
      yield root.settle(multi)
      root.navKey(multi, "next")
      root.navKey(multi, "open")
      yield root.settle(multi)
      root.check(!multi.navigating, "Enter did not end navigation")
      root.check(side.copies[0].pinned, "Enter did not pin the selected notch")
      root.check(!main.copyOn("B").pinned, "Enter left the other notch open")
      side.close()
      yield root.settle(multi)
      multi.navigate()
      yield root.settle(multi)
      main.copyOn("B").togglePinned("")
      yield root.settle(multi)
      root.check(!multi.navigating, "a click that pinned a notch left navigation on")
      main.close()
    })
  }

  function navigationTabs() {
    root.go(function*() {
      var copy = tabsSvc.notches.length > 0 ? tabsSvc.notches[0].copies[0] : null
      if (!root.check(!!copy && copy.tabModules.length === 2, "the notch with two tabs did not load")) return
      var first = copy.shownModules[0].moduleId
      tabsSvc.navigate()
      yield root.settle(tabsSvc)
      root.navKey(tabsSvc, "nextTab")
      yield root.settle(tabsSvc)
      root.check(copy.shownModules[0].moduleId !== first, "Tab did not show the next tab")
      root.navKey(tabsSvc, "previousTab")
      yield root.settle(tabsSvc)
      root.check(copy.shownModules[0].moduleId === first, "Shift+Tab did not go back")
      root.navKey(tabsSvc, "nextTab")
      root.navKey(tabsSvc, "leave")
      yield root.settle(tabsSvc)
      root.check(copy.shownModules[0].moduleId === first, "Escape did not give the notch its tab back")
      tabsSvc.navigate()
      yield root.settle(tabsSvc)
      root.navKey(tabsSvc, "nextTab")
      root.navKey(tabsSvc, "open")
      yield root.settle(tabsSvc)
      root.check(copy.pinned && copy.shownModules[0].moduleId !== first, "Enter did not open on the tab picked")
      copy.close()
      copy.showTab("")
      yield root.settle(tabsSvc)
    })
  }

  // Bug guarded: navigation takes the keyboard from an agent pinned open;
  // losing it that way closed the agent's notch, as a click away does.
  // Navigation's keys for the selected notch (Navigate.step): a removal
  // asked twice, its settings.
  function navigationRemove() {
    root.go(function*() {
      twoScreens.focusedScreenName = "B"
      multi.replaceConfig({ notches: root.twoNotches.notches.concat([{ id: "extra", modules: [], position: { edge: "bottom" } }]) })
      yield root.waitFor(function() { return multi.notches.length === 3 })
      yield root.settle(multi)
      multi.navigate()
      yield root.settle(multi)
      root.navKey(multi, "3")
      yield root.settle(multi)
      root.check(multi.notches.length === 3 && multi.navNotch && multi.navNotch.notchId === "extra",
                 "the third notch is not selected (test setup)")
      root.navKey(multi, "remove")
      yield root.settle(multi)
      root.check(multi.navMode === "remove", "x did not ask first: " + multi.navMode)
      root.check(multi.navNotch.copies[0].navMode === "remove", "the label does not show the question")
      root.navKey(multi, "leave")
      yield root.settle(multi)
      root.check(multi.navigating && multi.navMode === "select" && multi.notches.length === 3,
                 "Escape at the question did not keep the notch and stay in navigation")
      root.navKey(multi, "remove")
      root.navKey(multi, "remove")
      yield root.settle(multi)
      root.check(multi.notches.length === 2, "x twice left " + multi.notches.length + " notches")
      root.check(multi.navSelected === 1 && multi.navNotch && multi.navNotch.notchId === "side",
                 "after removing the last one, " + multi.navSelected + " is selected, not the one before")
      root.navKey(multi, "settings")
      yield root.settle(multi)
      root.check(!multi.navigating, "s did not end navigation")
      root.check(multi.settingsOpen && multi.settingsNotch === 1, "s did not open the selected notch's settings")
      multi.closeSettings()
      yield root.settle(multi)
      multi.replaceConfig(root.twoNotches)
    })
  }

  // Moving the selected notch (m): arrows for the edge, Shift along it, Tab
  // the screen, live; Enter keeps it, Escape or the shortcut puts it back.
  function navigationMove() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var side = multi.notchById("side")
      if (!root.check(!!side, "the side notch did not load")) return
      twoScreens.focusedScreenName = "B"
      multi.navigate()
      yield root.settle(multi)
      root.navKey(multi, "2")
      root.navKey(multi, "move")
      yield root.settle(multi)
      root.check(multi.navMode === "move", "m did not start moving: " + multi.navMode)
      root.navKey(multi, "down")
      yield root.settle(multi)
      root.check(side.placement.edge === "bottom", "down put it on " + side.placement.edge)
      root.navKey(multi, "slideOn")
      yield root.settle(multi)
      root.check(Math.abs(side.placement.offset - 0.55) < 1e-9, "Shift+→ slid it to " + side.placement.offset)
      root.navKey(multi, "leave")
      yield root.settle(multi)
      root.check(multi.navigating && multi.navMode === "select", "Escape while moving left navigation")
      root.check(side.placement.edge === "left" && side.placement.offset === 0.5, "Escape did not put it back: "
                 + side.placement.edge + " " + side.placement.offset)
      root.check(JSON.stringify(multi.configValue("notches.1.position")) === JSON.stringify({ edge: "left" }),
                 "the config after Escape: " + JSON.stringify(multi.configValue("notches.1.position")))
      root.navKey(multi, "move")
      root.navKey(multi, "up")
      root.navKey(multi, "open")
      yield root.settle(multi)
      root.check(multi.navigating && multi.navMode === "select", "Enter while moving left navigation")
      root.check(side.placement.edge === "top", "Enter did not keep it on top: " + side.placement.edge)
      root.navKey(multi, "move")
      root.navKey(multi, "nextTab")
      yield root.settle(multi)
      root.check(side.placement.screen === "A", "Tab took it to '" + side.placement.screen + "', not A")
      root.navKey(multi, "leave")
      yield root.settle(multi)
      root.check(side.placement.screen === "", "Escape left it on " + side.placement.screen)
      root.navKey(multi, "move")
      root.navKey(multi, "next")
      yield root.settle(multi)
      multi.navigate()
      yield root.settle(multi)
      root.check(!multi.navigating && side.placement.edge === "top", "the shortcut while moving left it on " + side.placement.edge)
      multi.replaceConfig(root.twoNotches)
    })
  }

  // Bug: a notch pinned open on the screen without focus closed at once:
  // on demand, Hyprland gave its keyboard back to the focused screen. It
  // stays exclusive there, on demand once its screen has focus.
  function keyboardOnOtherScreen() {
    root.go(function*() {
      yield root.nested(root.withTwoNotches)
      var side = multi.notchById("side")
      if (!root.check(!!side, "the side notch did not load")) return
      var copy = side.copies[0]
      multi.setConfigValue("notches.1.position.screen", "A")
      twoScreens.focusedScreenName = "B"
      yield root.settle(multi)
      root.check(copy.screenName === "A", "the side notch is on " + copy.screenName + ", not A (test setup)")
      side.open()
      copy.windowKeyboard(true)
      yield root.settle(multi)
      root.check(copy.pinned && copy.keyboardFocus === "exclusive", "held on the screen without focus, it asks for " + copy.keyboardFocus)
      twoScreens.focusedScreenName = "A"
      yield root.settle(multi)
      root.check(copy.keyboardFocus === "ondemand", "its screen focused, it asks for " + copy.keyboardFocus)
      side.close()
      twoScreens.focusedScreenName = "B"
      multi.setConfigValue("notches.1.position.screen", undefined)
    })
  }

  function navigationKeepsAgent() {
    root.go(function*() {
      var copy = typer.notches.length > 0 ? typer.notches[0].copies[0] : null
      if (!root.check(!!copy, "the typing notch did not load")) return
      copy.open()
      copy.windowKeyboard(true)
      yield root.settle(typer)
      root.check(copy.keyboardFocus === "ondemand", "pinned, holding the keyboard, it asks for " + copy.keyboardFocus)
      typer.navigate()
      yield root.settle(typer)
      root.check(copy.keyboardFocus === "none", "while navigating, the notch asks for " + copy.keyboardFocus)
      root.check(copy.mode === "expanded", "selected, the open agent shows " + copy.mode)
      copy.windowKeyboard(false)
      yield root.settle(typer)
      root.check(copy.pinned, "losing the keyboard to navigation closed the notch")
      root.navKey(typer, "leave")
      yield root.settle(typer)
      root.check(copy.pinned && copy.keyboardFocus === "exclusive", "after Escape it asks for " + copy.keyboardFocus)
      copy.close()
      yield root.settle(typer)
      // Enter: it takes the keyboard, then the window you were in takes it
      // back as navigation's window goes. It asks again, still open.
      typer.navigate()
      yield root.settle(typer)
      root.navKey(typer, "open")
      copy.windowKeyboard(true)
      copy.windowKeyboard(false)
      yield root.settle(typer)
      root.check(copy.pinned, "losing the keyboard as navigation ended closed the agent")
      root.check(copy.keyboardFocus === "exclusive", "it did not ask for the keyboard again: " + copy.keyboardFocus)
      yield root.wait(800)
      // Later on, a window taking it closes the notch, as before.
      copy.windowKeyboard(true)
      copy.windowKeyboard(false)
      yield root.settle(typer)
      root.check(!copy.pinned, "settled, losing the keyboard to a window left the notch open")
    })
  }

  // ------------------------------------------------------------ keyboard
  // A notch window takes the keyboard (layer-shell Exclusive) only while
  // pinned open on a module you type into: hover alone must never take it
  // from the window you are typing in.
  TestService {
    id: typer
    ipcTarget: "notchd-qmltest-typing"
    configPath: root.dir + "/typing.json"
    compositor: workspace4
    hostModules: ({ typing: Qt.resolvedUrl("fixtures/typing.qml") })
    defaultModules: ["typing"]
  }

  function keyboardOnlyWhilePinned() {
    root.go(function*() {
      yield root.wait(100)
      var module = typer.moduleById("typing")
      var copy = typer.notches.length > 0 ? typer.notches[0].copies[0] : null
      if (!root.check(!!module && !!copy, "the typing module or its notch did not load")) return
      root.check(copy.keyboardFocus === "none", "closed, it takes the keyboard: " + copy.keyboardFocus)
      root.hover(copy)
      yield root.waitFor(function() { return copy.mode === "expanded" })
      root.check(copy.mode === "expanded", "hovered, it is " + copy.mode)
      root.check(copy.keyboardFocus === "none", "hover alone took the keyboard")
      module.requestKeyboard()
      yield root.settle(typer)
      root.check(copy.pinned, "a press on the field did not pin the notch")
      root.check(copy.keyboardFocus === "exclusive", "pinned on the field, it takes " + copy.keyboardFocus)
      // Once its window has the keyboard, on demand: kept exclusive, no
      // shortcut could move focus to a window, only a click away.
      copy.windowKeyboard(true)
      yield root.settle(typer)
      root.check(copy.keyboardFocus === "ondemand", "holding the keyboard, it asks for " + copy.keyboardFocus)
      copy.windowKeyboard(false)
      yield root.settle(typer)
      root.check(!copy.pinned && copy.keyboardFocus === "none", "a window took the keyboard and the notch stayed open")
      root.unhover(copy)
      copy.open()
      yield root.settle(typer)
      root.check(copy.keyboardFocus === "exclusive", "pinned again, it takes " + copy.keyboardFocus)
      copy.close()
      yield root.settle(typer)
      root.check(copy.keyboardFocus === "none", "closed again, it keeps the keyboard")
    })
  }

  function expandedWidthFromModule() {
    var copy = typer.notches.length > 0 ? typer.notches[0].copies[0] : null
    copy.open()
    root.steps([[500, function() {}]], function() {
      root.check(typingCard.mode === "expanded", "the card is " + typingCard.mode)
      root.check(typingCard.width >= Theme.space(520) && typingCard.width < Theme.space(640),
                 "a 520 px expanded view made a card " + typingCard.width + " wide")
      copy.close()
      root.done()
    })
  }
  NotchCard {
    id: typingCard
    notch: typer.notches.length > 0 ? typer.notches[0].copies[0] : null
  }

  // Escape from a field in a view goes up to the card, which closes the
  // notch.
  QtObject {
    id: escapeModule
    property var field: null
    readonly property Component expanded: Component {
      TextInput {
        text: "type here"
        Component.onCompleted: escapeModule.field = this
      }
    }
  }
  QtObject {
    id: escapeSvc
    readonly property string mode: "expanded"
    readonly property string edge: "top"
    readonly property var compactModule: null
    readonly property var peekModule: null
    readonly property var expandedModules: [escapeModule]
    property int closes: 0
    function togglePinned(screenName) {}
    function close() { closes++ }
  }
  FloatingWindow {
    id: escapeWin
    implicitWidth: 500
    implicitHeight: 300
    NotchCard { id: escapeCard; notch: escapeSvc; x: 50 }
    // Keys go to a TestCase's own window: this one sits in it.
    TestCase { id: keys; when: false }
  }

  // Ctrl+Escape closes it too, as on any notch (a pinned one has the
  // keyboard: State.wantsKeyboard).
  function escapeCloses() {
    root.activate(keys)
    root.steps([[500, function() {
      if (!root.check(!!escapeModule.field, "the field did not load")) return
      escapeModule.field.forceActiveFocus()
      root.check(escapeModule.field.activeFocus, "the field did not take focus (test setup)")
      keys.keyClick(Qt.Key_Escape)
    }], [50, function() {
      escapeCard.forceActiveFocus()
      keys.keyClick(Qt.Key_Escape, Qt.ControlModifier)
    }], [50, function() {}]], function() {
      root.check(escapeSvc.closes === 2, "Escape and Ctrl+Escape closed it " + escapeSvc.closes + " times, not twice")
      root.done()
    })
  }

  // ------------------------------------------------------------------ drop
  // Bug: with the top-level position on the left, a notch in `notches`
  // dropped on the top edge was saved without `edge` (top is the default),
  // took "left" from the top level and jumped back. The drop saves what
  // the daemon saves: Position.configValue, against the inherited position.
  TestService {
    id: dropSvc
    ipcTarget: "notchd-qmltest-drop"
    configPath: root.dir + "/drop.json"
    compositor: workspace4
    defaultModules: ["activities"]
  }

  function dropOnTopStays() {
    root.go(function*() {
      dropSvc.replaceConfig({ position: { edge: "left", offset: 0.9, screen: "DP-1" },
                              notches: [{}, { id: "side", modules: ["activities"] }] })
      yield root.waitFor(function() { var n = dropSvc.notchById("side"); return !!n && n.edge === "left" })
      var side = dropSvc.notchById("side")
      if (!root.check(!!side && side.edge === "left", "the second notch should start on the left")) return
      // What NotchWindow reports when the notch is let go: picked up, then
      // dropped. The daemon saves what Position.configValue says.
      var drop = { edge: "top", offset: 0.5, screen: "DP-1" }
      side.copies[0].setDragging(true)
      side.copies[0].setDragging(false, drop)
      yield root.settle(dropSvc)
      root.check(side.edge === "top", "dropped on the top edge, it went to " + side.edge)
      root.check(side.placement.offset === 0.5, "dropped in the middle, it is at " + side.placement.offset)
      var main = dropSvc.notchById("main")
      main.copies[0].setDragging(true)
      main.copies[0].setDragging(false, { edge: "top", offset: 0.5, screen: "" })
      yield root.settle(dropSvc)
      root.check(main.edge === "top" && main.placement.screen === "", "the first notch went to " + JSON.stringify(main.placement))
    })
  }

  // ---------------------------------------------------------------- agents
  // The claude and codex modules (native/notchd-modules/src/term.rs, in the
  // daemon) against fake terminal UIs (tests/qml/fixtures/bin) that set the
  // terminal's title as Claude Code and Codex do. The program runs in the
  // daemon, which emulates its terminal; the terminal view
  // (core/terminal/GhosttySurface.qml) draws the screen it sends.
  TestService {
    id: codexSvc
    ipcTarget: "notchd-qmltest-codex"
    configPath: root.dir + "/codex.json"
    compositor: workspace4
    defaultModules: ["codex"]
  }
  FloatingWindow {
    id: codexWin
    implicitWidth: 900
    implicitHeight: 600
    NotchCard { id: codexCard; notch: codexSvc.notches.length > 0 ? codexSvc.notches[0].copies[0] : null }
    TestCase { id: codexKeys; when: false }
  }
  TestService {
    id: claudeSvc
    ipcTarget: "notchd-qmltest-claude"
    configPath: root.dir + "/claude.json"
    compositor: workspace4
    defaultModules: ["claude"]
  }
  FloatingWindow {
    id: claudeWin
    implicitWidth: 900
    implicitHeight: 600
    NotchCard { id: claudeCard; notch: claudeSvc.notches.length > 0 ? claudeSvc.notches[0].copies[0] : null }
    TestCase { id: claudeKeys; when: false }
  }

  // Runs a command; `out` and `code` once it exits.
  Process {
    id: shellCmd
    property int code: -1
    property string out: ""
    stdout: StdioCollector { onStreamFinished: shellCmd.out = text }
    onExited: function(code) { shellCmd.code = code }
  }
  function run(argv) {
    shellCmd.out = ""
    shellCmd.code = -1
    shellCmd.command = argv
    shellCmd.running = true
  }
  // `run`, and the wait for it to finish (the output is in shellCmd.out).
  function runAndWait(argv) {
    return function(resume) {
      root.run(argv)
      root.until(function() { return shellCmd.code !== -1 }, resume, 3000)
    }
  }

  // The terminal is not there: the case only notes it.
  function noTerminal(svc) {
    if (svc.terminalAvailable) return false
    console.log("NOTE " + root.current + ": this UI draws no terminal; the terminal was not tested")
    root.done()
    return true
  }

  // The terminal view inside an item (the item named "terminal", which has
  // the keyboard: GhosttySurface's), or null.
  function terminalIn(item) {
    if (!item) return null
    if (item.objectName === "terminal") return item
    var kids = item.children
    for (var i = 0; kids && i < kids.length; i++) {
      var t = root.terminalIn(kids[i])
      if (t) return t
    }
    return null
  }
  // What the daemon says of a module (its views, in the closed notch and
  // the open one): the facts the tests read. The tree's JSON, whole.
  function viewJson(svc, id, slot) { return JSON.stringify(svc.client.view(id, slot) || null) }
  function titleOf(svc, id) { return root.texts(svc.client.view(id, "compact"))[0] || "" }
  // The dot of the closed notch: "working" (green), "done" (the accent, news
  // you haven't seen) or "" (off).
  function dotOf(svc, id) {
    var t = root.viewJson(svc, id, "compact")
    return t.indexOf('"fill":"success"') !== -1 ? "working" : t.indexOf('"fill":"accent"') !== -1 ? "done" : ""
  }
  // Waits until a module's closed notch says so.
  function waitDot(svc, id, dot, ms) { return root.waitFor(function() { return root.dotOf(svc, id) === dot }, ms || 3000) }
  function waitTitle(svc, id, title, ms) { return root.waitFor(function() { return root.titleOf(svc, id) === title }, ms || 3000) }
  // Types text into the focused terminal as keys (so the daemon hears the
  // first key: from then on the end of the work is news), then Enter.
  function typeLine(keys, text) {
    for (var i = 0; i < text.length; i++) keys.keyClick(text.charAt(i))
    keys.keyClick(Qt.Key_Return)
  }
  function codexMissing() {
    root.go(function*() {
      var svc = codexMissingSvc
      yield root.settle(svc)
      root.check(!svc.terminalAvailable, "the service believes it has a terminal")
      // The daemon knows its UI can't draw one: the question is not typed.
      var answer = ""
      yield root.ask(svc, "codex", ["ask hello"], function(a) { answer = a })
      root.check(answer === "no UI connected draws a terminal", "ask without a terminal answered: " + answer)
      // The notch still opens on it, and says so in the terminal's place.
      svc.open()
      yield root.settle(svc)
      root.check(svc.notches[0].copies[0].mode === "expanded", "the notch did not open on it")
      svc.close()
    })
  }
  TestService {
    id: codexMissingSvc
    ipcTarget: "notchd-qmltest-codex-missing"
    configPath: root.dir + "/codex-missing.json"
    compositor: workspace4
    defaultModules: ["codex"]
    terminalAvailable: false
  }
  NotchCard { notch: codexMissingSvc.notches.length > 0 ? codexMissingSvc.notches[0].copies[0] : null }

  // One agent from start to restart: its command line and folder, Escape
  // going to it, the dot and the title from its terminal's title, the session
  // going on while the notch is closed, a peek when it finishes while closed,
  // the same program behind the next view, a new one (and the old one
  // stopped) on restart, and noticing when it quits.
  function agentRuns(svc, keys, card, id, expected) {
    if (root.noTerminal(svc)) return
    root.go(function*() {
      var cfg = {}
      cfg[id] = { directory: root.dir }
      svc.replaceConfig(cfg)
      var copy = svc.notches[0].copies[0]
      var argsFile = root.dir + "/" + id + "-args"
      var pid = ""
      root.activate(keys)
      yield root.settle(svc)
      root.check(root.dotOf(svc, id) === "" && root.titleOf(svc, id) !== expected, "before opening, it already works")
      svc.open()
      yield root.waitFor(function() { return !!root.terminalIn(card) && root.terminalIn(card).title !== "" }, 5000)
      var first = root.terminalIn(card)
      if (!root.check(!!first, "opening the notch started no terminal")) { svc.close(); return }
      yield root.waitDot(svc, id, "", 3000)
      root.check(root.dotOf(svc, id) === "" && root.titleOf(svc, id) !== expected, "after starting: dot " + root.dotOf(svc, id) + ", title " + root.titleOf(svc, id))
      yield root.waitFor(function() { return first.activeFocus }, 2000)
      root.check(first.activeFocus, "the terminal does not have the focus")
      // Escape goes to the agent: the notch stays open.
      keys.keyClick(Qt.Key_Escape)
      // The Escape is on the fake's line: end that line first. Real keys, so
      // the daemon hears the first one.
      keys.keyClick(Qt.Key_Return)
      root.typeLine(keys, "work")
      yield root.runAndWait(["cat", argsFile])
      var args = shellCmd.out.split("\n")
      root.check(args[0] === "", id + " got options: " + args[0])
      root.check(args[1] === root.dir, id + " started in " + args[1])
      pid = args[2]
      root.check(copy.pinned, "Escape closed the notch")
      yield root.waitDot(svc, id, "working")
      root.check(root.dotOf(svc, id) === "working", "the title did not make it working")
      yield root.waitTitle(svc, id, expected, 4000)
      yield root.waitDot(svc, id, "", 4000)
      root.check(root.dotOf(svc, id) === "" && root.titleOf(svc, id) === expected,
                 "after work: dot " + root.dotOf(svc, id) + ", title " + root.titleOf(svc, id))
      // Ctrl+Escape is the way out: the notch closes, the agent goes on.
      keys.keyClick(Qt.Key_Escape, Qt.ControlModifier)
      yield root.settle(svc)
      root.check(!copy.pinned, "Ctrl+Escape did not close the notch")
      yield root.waitFor(function() { return !root.terminalIn(card) })
      yield root.runAndWait(["kill", "-0", pid])
      root.check(shellCmd.code === 0, "closing stopped " + id + " (" + pid + ")")
      // Reopened, the same program: it was working while closed, so the end
      // of its work, typed again, is news.
      svc.open()
      yield root.waitFor(function() { return !!root.terminalIn(card) && root.terminalIn(card).title !== "" }, 5000)
      first = root.terminalIn(card)
      root.check(!!first, "reopened, no terminal")
      yield root.waitFor(function() { return first.activeFocus }, 2000)
      root.typeLine(keys, "work")
      yield root.waitDot(svc, id, "working")
      keys.keyClick(Qt.Key_Escape, Qt.ControlModifier)
      yield root.waitFor(function() { return !!copy.peekModule }, 4000)
      root.check(!!copy.peekModule && copy.peekModule.moduleId === id, "the end of work while closed did not peek")
      root.check(root.dotOf(svc, id) === "done" || root.dotOf(svc, id) === "working", "done while closed, it has no news for you")
      yield root.waitDot(svc, id, "done", 4000)
      root.check(root.dotOf(svc, id) === "done", "done while closed, it has no news for you: " + root.dotOf(svc, id))
      svc.open()
      yield root.waitFor(function() { return !!root.terminalIn(card) }, 5000)
      yield root.waitDot(svc, id, "")
      root.check(root.dotOf(svc, id) === "", "seen, it still has news")
      yield root.runAndWait(["cat", argsFile])
      root.check(shellCmd.out.split("\n")[2] === pid, "reopened, it is not the same " + id)
      // A new session: the old program stops with it.
      root.check(svc.agentCommand(id, "new") === "ok", "new failed")
      yield root.waitFor(function() { return !!root.terminalIn(card) && root.terminalIn(card) !== first }, 5000)
      yield root.wait(900)
      yield root.runAndWait(["cat", argsFile])
      root.check(shellCmd.out.split("\n")[2] !== pid, "the new terminal runs the old " + id)
      yield root.runAndWait(["kill", "-0", pid])
      root.check(shellCmd.code !== 0, "the old " + id + " (" + pid + ") still runs after restart")
      // Quitting is noticed: the program is gone, its output stays.
      var second = root.terminalIn(card)
      yield root.waitFor(function() { return second.activeFocus }, 2000)
      root.typeLine(keys, "quit")
      yield root.waitFor(function() { return root.viewJson(svc, id, "expanded").indexOf("exited") !== -1 }, 4000)
      root.check(root.viewJson(svc, id, "expanded").indexOf("exited") !== -1, id + " quit and the module did not notice")
      root.check(root.dotOf(svc, id) === "", id + " quit and is still working")
      svc.close()
      yield root.settle(svc)
    })
  }
  function codexRuns() { root.agentRuns(codexSvc, codexKeys, codexCard, "codex", "Fake thread") }
  function claudeRuns() { root.agentRuns(claudeSvc, claudeKeys, claudeCard, "claude", "Fake conversation") }

  // Over IPC: ask opens the notch with the keyboard and types the question
  // in once the agent is up; new starts another; anything else, the usage
  // (`notchd ipc` has the daemon's answers; the host's target answers ok).
  function claudeAsk() {
    root.go(function*() {
      if (!claudeSvc.terminalAvailable) { root.noTerminal(claudeSvc); return }
      var copy = claudeSvc.notches[0].copies[0]
      var pid = ""
      // The last case's view is still closing.
      yield root.wait(600)
      var answer = ""
      yield root.ask(claudeSvc, "agent", ["claude", "nonsense"], function(a) { answer = a })
      root.check(answer.indexOf("usage: agent claude") === 0, "an unknown command did not answer the usage: " + answer)
      yield root.ask(claudeSvc, "claude", ["ask why is the sky blue?"], function(a) { answer = a })
      root.check(answer === "ok", "ask failed: " + answer)
      yield root.settle(claudeSvc)
      root.check(copy.pinned && copy.keyboardFocus === "exclusive", "ask did not open the notch with the keyboard")
      var lines = []
      yield root.wait(1800)
      yield root.runAndWait(["cat", root.dir + "/claude-args"])
      lines = shellCmd.out.split("\n")
      root.check(lines.indexOf("got why is the sky blue?") !== -1, "the question did not reach claude: " + JSON.stringify(lines))
      pid = lines[2]
      root.check(claudeSvc.agentCommand("claude", "new") === "ok", "new failed")
      yield root.wait(1200)
      yield root.runAndWait(["cat", root.dir + "/claude-args"])
      root.check(shellCmd.out.split("\n")[2] !== pid, "new did not start another claude")
      claudeSvc.close()
      yield root.settle(claudeSvc)
    })
  }

  // Bug: a shortcut named the notch (toggleNotch claude), and renaming the
  // notch in settings broke it without a word. `agent <id> toggle` finds
  // the notch by the module it shows.
  function agentToggle() {
    root.go(function*() {
      var copy = claudeSvc.notches[0].copies[0]
      // The last case's view is still closing.
      yield root.wait(600)
      root.check(claudeSvc.notches[0].notchId !== "claude", "the test notch is called claude: the case proves nothing")
      root.check(claudeSvc.agentCommand("claude", "toggle") === "ok", "toggle failed")
      yield root.settle(claudeSvc)
      root.check(copy.pinned, "toggle did not open the notch")
      // At once, before any terminal attached: the toggle goes by the notch
      // state, not by a view that reports a moment later.
      root.check(claudeSvc.agentCommand("claude", "toggle") === "ok", "toggle again failed")
      yield root.settle(claudeSvc)
      root.check(!copy.pinned, "toggle did not close the notch")
      root.check(claudeSvc.agentCommand("nope", "toggle").indexOf("no agent nope") === 0, "an unknown agent did not say so")
      claudeSvc.close()
      yield root.settle(claudeSvc)
    })
  }

  // The plan's limits, fed answers by hand (the daemon's test hook): offscreen,
  // nothing asks the network or reads a real login (NOTCHD_OFFLINE).
  function agentUsage() {
    root.go(function*() {
      var copy = claudeSvc.notches[0].copies[0]
      // `day`: the session window (its reset), so a new window starts over.
      var answer = function(session, day) {
        return JSON.stringify({ limits: [
          { kind: "session", group: "session", percent: session, resets_at: "2099-01-0" + (day || 2) + "T05:00:00Z" },
          { kind: "weekly_all", group: "weekly", percent: 20, resets_at: "2099-01-07T00:00:00Z" } ] })
      }
      var last = ""
      var feed = function(text) {
        return root.ask(claudeSvc, "test-claude", ["usage", text], function(a) { last = a })
      }
      var shown = function() { return root.texts(claudeSvc.client.view("claude", "expanded")).some(function(t) { return /^\d+ %/.test(t) }) }
      var peekText = function() { return root.texts(claudeSvc.client.view("claude", "peek"))[0] }
      yield root.settle(claudeSvc)
      root.check(!shown(), "usage shown before any answer")
      yield feed(answer(85, 1))
      root.check(last === "ok", "the answer was refused: " + last)
      yield root.settle(claudeSvc)
      var t = root.texts(claudeSvc.client.view("claude", "expanded"))
      root.check(shown() && t.indexOf("5h") !== -1 && t.some(function(x) { return x.indexOf("85 %") === 0 }), "the limits do not show: " + JSON.stringify(t))
      root.check(copy.peekModule === null, "the first answer warned: a restart would warn again")
      yield feed(answer(10))
      root.check(last === "ok", "the answer was refused: " + last)
      yield feed(JSON.stringify({ nope: 1 }))
      root.check(last === "not an answer", "an answer that isn't one was taken: " + last)
      yield feed(answer(81))
      yield root.settle(claudeSvc)
      root.check(!!copy.peekModule && copy.peekModule.moduleId === "claude", "passing 80 % did not peek")
      root.check(peekText() === "Claude: 5h limit at 81 %", "the warning: " + peekText())
      claudeSvc.dismissPeek(copy.peekModule)
      yield feed(answer(90))
      yield root.settle(claudeSvc)
      root.check(copy.peekModule === null, "warned twice in one window")
      yield feed(answer(100))
      yield root.settle(claudeSvc)
      root.check(!!copy.peekModule && peekText() === "Claude: 5h limit reached", "the limit reached did not peek: " + peekText())
      claudeSvc.dismissPeek(copy.peekModule)
      claudeSvc.setConfigValue("claude.usage", false)
      yield root.settle(claudeSvc)
      root.check(!shown(), "switched off, the usage still shows")
      claudeSvc.setConfigValue("claude.usage", undefined)
      yield root.settle(claudeSvc)
    })
  }

  // Bug (claude chat, kept for the terminal): while busy the mark cycled
  // through glyphs of different widths, so the closed notch kept resizing
  // (it springs to fit). Now the mark stays, and the working dot's place is
  // kept while it is off.
  function agentBusySteady() {
    root.go(function*() {
      claudeSvc.close()
      var widths = []
      var title = function(t) { return root.ask(claudeSvc, "test-claude", ["title", t], function() {}) }
      yield root.wait(600)
      yield title("✳ A title")
      yield root.settle(claudeSvc)
      yield root.wait(500)
      widths.push(claudeCard.width)
      yield title("◐ A title")
      yield root.settle(claudeSvc)
      yield root.wait(500)
      widths.push(claudeCard.width)
      yield root.wait(130)
      widths.push(claudeCard.width)
      yield root.wait(130)
      widths.push(claudeCard.width)
      root.check(claudeCard.mode === "compact", "the notch is " + claudeCard.mode)
      var min = Math.min.apply(null, widths), max = Math.max.apply(null, widths)
      root.check(max - min < 0.5, "the closed notch changed width going busy and while busy: " + JSON.stringify(widths))
      yield title("")
      yield root.settle(claudeSvc)
    })
  }

  // A terminal of your own (`terminals`): its id is a module a notch lists,
  // with its title and the fields of a terminal; it runs its command in
  // your home, shows the title the program sets, and `terminal <id>`
  // reaches it, `agent <id>` doesn't.
  TestService {
    id: ownTermSvc
    ipcTarget: "notchd-qmltest-own-terminal"
    configPath: root.dir + "/own-terminal.json"
    compositor: workspace4
    defaultModules: []
  }
  NotchCard { notch: ownTermSvc.notches.length > 0 ? ownTermSvc.notches[0].copies[0] : null }
  function ownTerminal() {
    var pwdFile = root.dir + "/own-terminal-pwd"
    ownTermSvc.replaceConfig({
      terminals: { mine: { title: "Mine", command: "printf '\\033]0;Own title\\007'; pwd > " + pwdFile + "; exec cat" } },
      modules: ["mine"]
    })
    root.go(function*() {
      yield root.waitFor(function() { return !!ownTermSvc.moduleById("mine") }, 4000)
      var m = ownTermSvc.moduleById("mine")
      if (!root.check(!!m, "the terminal from the config did not load")) return
      var copy = ownTermSvc.notches[0].copies[0]
      yield root.settle(ownTermSvc)
      root.check(m.title === "Mine", "its title: " + m.title)
      root.check(ownTermSvc.moduleCatalog.some(function(c) { return c.moduleId === "mine" && c.terminal && !c.agent }),
                 "the catalog has no terminal mine")
      root.check(ownTermSvc.settingsSchema.some(function(f) { return f.key === "terminals.mine.command" && f.page === "modules/mine" }),
                 "its page has no command field")
      root.check(ownTermSvc.settingsSchema.some(function(f) { return f.type === "moduleCatalog" && f.part === "terminal" }),
                 "the Modules page does not list terminals apart")
      root.check(ownTermSvc.agentCommand("mine", "toggle").indexOf("no agent mine") === 0, "a terminal took an agent command")
      root.check(ownTermSvc.terminalCommand("nope", "toggle") === "no terminal nope; terminals: mine", "an unknown terminal did not say so")
      if (!ownTermSvc.terminalAvailable) {
        console.log("NOTE " + root.current + ": this UI draws no terminal; the terminal was not run")
        return
      }
      root.check(ownTermSvc.terminalCommand("mine", "toggle") === "ok", "toggle failed")
      yield root.settle(ownTermSvc)
      root.check(copy.pinned, "toggle did not open it")
      // Its program starts with the first view, and its title is the
      // program's, never "working".
      yield root.waitTitle(ownTermSvc, "mine", "Own title", 5000)
      root.check(root.titleOf(ownTermSvc, "mine") === "Own title" && root.dotOf(ownTermSvc, "mine") === "",
                 "its title: " + root.titleOf(ownTermSvc, "mine"))
      yield root.runAndWait(["cat", pwdFile])
      root.check(shellCmd.out.trim() === Quickshell.env("HOME"), "it started in " + shellCmd.out.trim())
      // Toggle closes it once a view shows it.
      yield root.wait(400)
      root.check(ownTermSvc.terminalCommand("mine", "toggle") === "ok", "toggle failed")
      yield root.settle(ownTermSvc)
      root.check(!copy.pinned, "toggle did not close it")
    })
  }

  // Bug: a terminal view took a connection that dropped (a slow daemon, a
  // theme change) for the program's end, and the next Enter restarted it:
  // a loaded test run restarted the codex agent so. Only the daemon's
  // `surface-close` is an end; a dropped connection is made again.
  TestService {
    id: reconnectSvc
    ipcTarget: "notchd-qmltest-reconnect"
    configPath: root.dir + "/reconnect.json"
    compositor: workspace4
    defaultModules: []
  }
  NotchCard { id: reconnectCard; notch: reconnectSvc.notches.length > 0 ? reconnectSvc.notches[0].copies[0] : null }
  function terminalReconnects() {
    var pidFile = root.dir + "/reconnect-pid"
    reconnectSvc.replaceConfig({ terminals: { rdrop: { command: "echo $$ > " + pidFile + "; exec cat" } }, modules: ["rdrop"] })
    root.go(function*() {
      yield root.waitFor(function() { return !!reconnectSvc.moduleById("rdrop") && reconnectSvc.isTerminal("rdrop") }, 4000)
      root.check(reconnectSvc.terminalCommand("rdrop", "toggle") === "ok", "toggle failed")
      var card = reconnectCard
      yield root.waitFor(function() { var t = root.terminalIn(card); return !!t && t.connected }, 5000)
      var term = root.terminalIn(card)
      if (!root.check(!!term && term.connected, "the terminal did not connect")) return
      // The program starts with the view: its pid, once it wrote it.
      var pid = ""
      for (var i = 0; i < 30 && pid === ""; i++) {
        yield root.runAndWait(["cat", pidFile])
        pid = shellCmd.out.trim()
        if (pid === "") yield root.wait(100)
      }
      root.check(pid !== "", "the program did not start")
      term.dropConnection()
      yield root.wait(100)
      root.check(!term.exited, "a dropped connection was taken for the program's end")
      yield root.waitFor(function() { return term.connected }, 3000)
      root.check(term.connected, "the connection was not made again")
      root.check(!term.exited, "reconnected, it says the program ended")
      yield root.wait(300)
      yield root.runAndWait(["cat", pidFile])
      root.check(shellCmd.out.trim() === pid, "the program was restarted: " + pid + " -> " + shellCmd.out.trim())
      root.check(reconnectSvc.terminalCommand("rdrop", "toggle") === "ok", "toggle failed")
      yield root.settle(reconnectSvc)
      reconnectSvc.replaceConfig({})
    })
  }

  // Bug: renaming a terminal (or removing one) warned of an unknown module
  // and emptied the list of modules for a moment: its new id had no info
  // yet, so the modules were not ready; they came back with the old id.
  // (The run fails on the warning.)
  function terminalRenameStaysReady() {
    var flips = 0
    var count = function() { flips++ }
    ownTermSvc.replaceConfig({ terminals: { a: { command: "cat" } }, modules: ["activities", "a"] })
    ownTermSvc.moduleInfoReadyChanged.connect(count)
    root.steps([
      [200, function() {
        root.check(!!ownTermSvc.moduleById("activities") && !!ownTermSvc.moduleById("a"), "activities and a did not load")
        ownTermSvc.replaceConfig({ terminals: { b: { command: "cat" } }, modules: ["activities", "b"] })
      }],
      [200, function() {
        root.check(!!ownTermSvc.moduleById("b") && !ownTermSvc.moduleById("a"), "the rename did not apply")
        ownTermSvc.replaceConfig({ modules: ["activities"] })
      }],
      [200, function() {
        root.check(!ownTermSvc.moduleById("b"), "the removed terminal is still loaded")
        root.check(flips === 0, "the modules were not ready for a moment, " + flips + " changes")
      }]
    ], function() {
      ownTermSvc.moduleInfoReadyChanged.disconnect(count)
      ownTermSvc.replaceConfig({})
      root.done()
    })
  }

  function addAndRemoveTerminal() {
    var start = { modules: ["activities"] }
    ownTermSvc.replaceConfig(start)
    catalog.svc = ownTermSvc
    // The daemon's answer first: until then its list still has an earlier
    // case's terminal (the same service), and the list showed two.
    root.until(function() { return catalog.modules.length === 0 }, function() {
    var add = null
    for (var i = 0; i < catalog.children.length; i++)
      if (catalog.children[i].text === "Add terminal") add = catalog.children[i]
    if (!root.check(!!add && add.visible, "the Terminals list has no Add button")) { root.done(); return }
    add.clicked()
    root.check(JSON.stringify(ownTermSvc.config.terminals) === JSON.stringify({ terminal: { title: "Terminal", command: "$SHELL" } }),
               "the new terminal: " + JSON.stringify(ownTermSvc.config.terminals))
    root.check(ownTermSvc.settingsPage === "modules/terminal", "Add did not open its page: " + ownTermSvc.settingsPage)
    root.check(catalog.modules.length === 1 && catalog.modules[0].moduleId === "terminal", "the list does not show it")
    ownTermSvc.setConfigValue("modules", ["activities", "terminal"])
    ownTermSvc.removeTerminal("terminal")
    root.check(JSON.stringify(ownTermSvc.config) === JSON.stringify(start), "the config came back as " + JSON.stringify(ownTermSvc.config))
    root.check(catalog.modules.length === 0, "removed, the list still shows it")
    ownTermSvc.settingsPage = "general"
    catalog.svc = null
    root.done()
    }, 4000)
  }
  ModuleCatalog {
    id: catalog
    width: 400
    field: ({ type: "moduleCatalog", part: "terminal" })
  }

  // Bug guarded: emptying a terminal's command removed it, and with it the
  // terminal.
  function requiredTextField() {
    var got = []
    var record = function(v) { got.push(v) }
    requiredText.edited.connect(record)
    requiredText.commit()
    root.check(got.length === 1 && got[0] === "btop", "its text did not go through: " + JSON.stringify(got))
    got = []
    // Emptied (the text follows the stored value).
    requiredText.stored = undefined
    requiredText.commit()
    root.check(got.length === 0 && requiredText.invalid, "emptying a required field went through: " + JSON.stringify(got))
    requiredText.field = { key: "x", type: "string" }
    requiredText.commit()
    root.check(got.length === 1 && got[0] === undefined && !requiredText.invalid, "emptying a plain field did not remove it")
    requiredText.edited.disconnect(record)
    root.done()
  }
  TextControl {
    id: requiredText
    field: ({ key: "x", type: "string", required: true })
    stored: "btop"
  }

  // A spoken conversation (LiveModule) with a fake bridge
  // (tests/qml/fixtures/bin/codex-live): it says it is live, hears a
  // question, answers, and closes on {"stop": true}.
  TestService {
    id: liveSvc
    ipcTarget: "notchd-qmltest-live"
    configPath: root.dir + "/live.json"
    compositor: workspace4
    defaultModules: ["codex-live"]
  }
  FloatingWindow {
    implicitWidth: 700
    implicitHeight: 600
    NotchCard { notch: liveSvc.notches.length > 0 ? liveSvc.notches[0].copies[0] : null }
  }
  FileView {
    id: liveArgs
    path: Quickshell.env("NOTCHD_FAKE_LIVE_ARGS")
    blockLoading: true
    printErrors: false
  }
  function liveCopy() { return liveSvc.notches[0].copies[0] }

  // The local voice (modules/local-live.qml): its own bridge and arguments.
  TestService {
    id: localLiveSvc
    ipcTarget: "notchd-qmltest-local-live"
    configPath: root.dir + "/local-live.json"
    compositor: workspace4
    defaultModules: ["local-live"]
  }
  FloatingWindow {
    implicitWidth: 700
    implicitHeight: 600
    NotchCard { notch: localLiveSvc.notches.length > 0 ? localLiveSvc.notches[0].copies[0] : null }
  }
  FileView {
    id: localLiveArgsFile
    path: Quickshell.env("NOTCHD_FAKE_LOCAL_LIVE_ARGS")
    blockLoading: true
    printErrors: false
  }
  function localLiveArgs() {
    localLiveSvc.replaceConfig({ "local-live": { agent: "claude", session: "new", directory: "~/talk", language: "en",
                                                 speechModel: "small", voiceModel: "pocket-pt", otherVoiceModel: "mine",
                                                 voice: "rafael", localaiUrl: "http://127.0.0.1:9" } })
    localLiveSvc.open()
    root.steps([
      [900, function() {
        var t = root.texts(localLiveSvc.client.view("local-live", "expanded"))
        root.check(t.indexOf("Claude · Listening") !== -1 || t.indexOf("Listening") !== -1, "no conversation: " + t)
        localLiveArgsFile.reload()
        var args = localLiveArgsFile.text().split("\n")
        var home = Quickshell.env("HOME")
        root.check(args[args.indexOf("--agent") + 1] === "claude", "the agent: " + args)
        root.check(args[args.indexOf("--session") + 1] === "new", "the session: " + args)
        root.check(args[args.indexOf("--cwd") + 1] === home + "/talk", "the folder: " + args)
        root.check(args[args.indexOf("--lang") + 1] === "en", "the language: " + args)
        root.check(args[args.indexOf("--speech-model") + 1] === "small", "the speech model: " + args)
        root.check(args[args.indexOf("--voice-model") + 1] === "mine", "a model of your own wins: " + args)
        root.check(args[args.indexOf("--voice") + 1] === "rafael", "the voice: " + args)
        root.check(args[args.indexOf("--localai-url") + 1] === "http://127.0.0.1:9", "the address: " + args)
        root.check(args.indexOf("--whisper-model") === -1 && args.indexOf("--tts") === -1, "an old engine's argument: " + args)
        root.check(args.indexOf("--codex") === -1, "a codex-live argument: " + args)
        localLiveSvc.close()
      }],
      [2500, function() { root.check(root.texts(localLiveSvc.client.view("local-live", "compact")).length === 0, "closed, it went on") }]
    ], function() {
      localLiveSvc.replaceConfig({})
      root.done()
    })
  }

  // The optional components (core/ComponentStore.qml, settings/ComponentRow.qml)
  // against a fake installer (fixtures/bin/components): the state it reports
  // reaches the rows, their buttons run it, and a choice that needs a
  // component is locked until it is installed.
  TestService {
    id: componentSvc
    ipcTarget: "notchd-qmltest-components"
    configPath: root.dir + "/components.json"
    compositor: workspace4
    defaultModules: ["local-live"]
  }
  FileView {
    id: componentLog
    path: root.dir + "/components/log"
    blockLoading: true
    printErrors: false
  }
  FloatingWindow {
    implicitWidth: 700
    implicitHeight: 200
    Item { id: componentHost; anchors.fill: parent }
  }
  Component {
    id: settingsWindowComponent
    SettingsWindow { svc: componentSvc }
  }
  Component {
    id: componentRowComponent
    ComponentRow { svc: componentSvc }
  }
  function componentLog_() {
    componentLog.reload()
    componentLog.waitForJob()
    return componentLog.text().split("\n").filter(function(l) { return l !== "" })
  }
  // The rows are the daemon's (the schema's `row`, its state read from the
  // fake installer while the settings are open); the buttons are the
  // `component` IPC.
  function componentRows() {
    componentSvc.settingsOpen = true
    var rows = []
    function field(id) {
      return componentSvc.settingsSchema.filter(function(f) { return f.type === "component" && f.component === id && f.page === "components" })[0]
    }
    function row(id) {
      var item = componentRowComponent.createObject(componentHost, { field: field(id) })
      rows.push(item)
      return item
    }
    function lockedChoices() {
      var f = componentSvc.settingsSchema.filter(function(x) { return x.key === "local-live.voiceModel" })[0]
      return f.options.map(function(o) { return o.disabled === true })
    }
    function voiceRows() {
      // What the window shows: the schema without the rows whose `when` fails.
      return Settings.shownFields(componentSvc.settingsSchema, componentSvc.configValue).filter(function(f) { return f.type === "component" && f.page === "modules/local-live" && f.component.indexOf("voice-") === 0 })
        .map(function(f) { return f.component })
    }
    root.check(componentSvc.settingsPages.some(function(p) { return p.id === "components" }), "no Components page")
    root.steps([
      [1500, function() {
        var small = row("localai")
        var medium = row("voice-pocket-pt")
        var voice = row("voice-piper-pt-br")
        var base = row("voxtype")
        var pulse = row("libpulse")
        root.check(small.info.text === "Installed (197 MB)", "installed: " + small.info.text)
        root.check(medium.info.text === "Downloading 42 % (644 MB of 1.5 GB)" && medium.working, "downloading: " + medium.info.text)
        root.check(medium.info.actions.length === 1 && medium.info.actions[0].id === "cancel", "a download has Cancel only")
        root.check(voice.info.state === "failed" && voice.info.text.indexOf("could not download") !== -1, "failed: " + voice.info.text)
        root.check(pulse.info.text === "Installed by the system" && pulse.info.actions.length === 0, "a system package is never removed here")
        root.check(base.info.text === "Not installed" && base.info.actions[0].id === "install", "missing: " + base.info.text)
        var rowsOnPage = componentSvc.settingsSchema.filter(function(f) { return f.page === "components" && f.type === "component" })
        root.check(rowsOnPage.length > 6, "the overview misses components: " + rowsOnPage.length)
        var field = componentSvc.settingsSchema.filter(function(f) { return f.component === "localai" && f.page === "modules/local-live" })[0]
        root.check(!!field && field.label === "LocalAI" && field.help.indexOf("For Voice") !== -1, "row not filled: " + JSON.stringify(field))
        // the Voice page: the chosen language's voices only, its choices locked until they are installed
        root.check(JSON.stringify(voiceRows()) === JSON.stringify(["voice-piper-pt-br", "voice-pocket-pt"]), "pt voices: " + JSON.stringify(voiceRows()))
        root.check(JSON.stringify(lockedChoices()) === "[false,true,true]", "pt locks: " + JSON.stringify(lockedChoices()))
        componentSvc.setConfigValue("local-live.voiceLanguage", "en")
      }],
      [600, function() {
        root.check(JSON.stringify(voiceRows()) === JSON.stringify(["voice-piper-en", "voice-pocket-en"]), "en voices: " + JSON.stringify(voiceRows()))
        root.check(JSON.stringify(lockedChoices()) === "[false,true,false]", "en locks (Pocket installed): " + JSON.stringify(lockedChoices()))
        componentSvc.setConfigValue("local-live.voiceLanguage", undefined)
        // Install runs the installer; the row follows its state
        rows[3].act("install")
      }],
      [1500, function() {
        var base = row("voxtype")
        root.check(root.componentLog_().indexOf("install voxtype") !== -1, "install not run: " + root.componentLog_())
        root.check(base.working && base.info.percent === 10, "no progress after Install: " + base.info.text)
        base.act("cancel")
      }],
      [1500, function() {
        var base = row("voxtype")
        root.check(root.componentLog_().indexOf("cancel voxtype") !== -1, "cancel not run: " + root.componentLog_())
        root.check(base.info.state === "missing", "still working after Cancel: " + base.info.text)
        // Remove asks twice
        var small = row("localai")
        small.act("remove")
        root.check(small.confirming && root.componentLog_().indexOf("remove localai") === -1, "removed on the first click")
        small.act("remove")
      }],
      [1500, function() {
        var small = row("localai")
        root.check(root.componentLog_().indexOf("remove localai") !== -1, "remove not run on the second click")
        root.check(small.info.state === "missing", "still installed after Remove: " + small.info.text)
        rows.forEach(function(r) { r.destroy() })
        componentSvc.settingsOpen = false
      }]
    ], function() { root.done() })
  }

  function liveTexts() { return root.texts(liveSvc.client.view("codex-live", "expanded")) }
  function liveRunning() { return liveSvc.client.view("codex-live", "compact") !== null }

  function liveTalks() {
    liveSvc.replaceConfig({ "codex-live": { voice: "maple", directory: "~/talk", access: "workspace-write" } })
    // A hover shows it and leaves the microphone off.
    root.hover(root.liveCopy())
    root.steps([
      [400, function() {
        root.check(root.liveCopy().mode === "expanded", "hovered, the notch didn't open")
        root.check(!root.liveRunning(), "a hover started a conversation")
        root.unhover(root.liveCopy())
        liveSvc.open()
      }],
      [1000, function() {
        var t = root.liveTexts()
        root.check(root.liveRunning(), "no conversation: " + t)
        root.check(t.indexOf("Rodei os testes.") !== -1, "transcript: " + t)
        root.check(t.indexOf("Speaking") !== -1 || t.indexOf("Working") !== -1, "the voice's level didn't show: " + t)
        liveArgs.reload()
        var args = liveArgs.text().split("\n")
        var home = Quickshell.env("HOME")
        root.check(args.indexOf("--voice") !== -1 && args[args.indexOf("--voice") + 1] === "maple", "args: " + args)
        root.check(args[args.indexOf("--cwd") + 1] === home + "/talk", "the folder: " + args)
        root.check(args[args.indexOf("--access") + 1] === "workspace-write", "the access: " + args)
        root.check(args[args.indexOf("--codex") + 1] === "codex", "the command: " + args)
        // Closed for a moment, it goes on.
        liveSvc.close()
      }],
      [500, function() {
        root.check(root.liveRunning(), "closing for a moment ended it, or nothing shows closed")
      }],
      [2200, function() {
        root.check(!root.liveRunning(), "closed, the conversation went on")
        root.check(root.liveTexts().indexOf("Rodei os testes.") !== -1, "the transcript went with it")
      }]
    ], function() {
      liveSvc.replaceConfig({})
      root.done()
    })
  }

  function liveStopAndCommands() {
    root.check(/^not in a conversation/.test(liveSvc.liveCommand("codex-live", "mute")), "mute without a conversation")
    root.check(/^usage: live codex-live/.test(liveSvc.liveCommand("codex-live", "sing")), "no usage")
    root.check(/^no live module codex; live modules: codex-live/.test(liveSvc.liveCommand("codex", "start")), "an agent took live")
    root.check(liveSvc.liveCommand("codex-live", "start") === "ok", "start was refused")
    root.steps([
      [1000, function() {
        root.check(root.liveCopy().pinned && root.liveRunning(), "start didn't open the notch and talk")
        liveSvc.notchIpc("live", ["codex-live", "mute"])
      }],
      [500, function() {
        // The fake bridge keeps "speaking", which the status line prefers to
        // "Muted"; the microphone button is what says it.
        root.check(JSON.stringify(liveSvc.client.view("codex-live", "expanded")).indexOf("Unmute the microphone") !== -1, "the bridge didn't say muted: " + root.liveTexts())
        liveSvc.notchIpc("live", ["codex-live", "stop"])
      }],
      [800, function() {
        root.check(!root.liveRunning(), "stop didn't end it")
        root.check(root.liveCopy().pinned, "stop closed the notch")
      }],
      [1800, function() {
        root.check(!root.liveRunning(), "stopped with the notch open, it started again")
        liveSvc.close()
      }],
      [400, function() { liveSvc.open() }],
      [1000, function() {
        root.check(root.liveRunning(), "opened again after a stop, it didn't talk")
        liveSvc.notchIpc("live", ["codex-live", "toggle"])
      }],
      [800, function() {
        root.check(!root.liveRunning(), "toggle didn't stop it")
        liveSvc.close()
      }],
      [1800, function() {}]
    ], root.done)
  }

  function liveMissingPackages() {
    liveSvc.replaceConfig({ "codex-live": { command: "missing-codex" } })
    liveSvc.open()
    root.steps([
      [1000, function() {
        var t = root.liveTexts()
        root.check(!root.liveRunning(), "it kept running: " + t)
        root.check(t.some(function(x) { return /Install WebRTC and audio in Settings › Components\.$/.test(x) }), "error: " + t)
        root.check(t.indexOf("Stopped") !== -1, "status: " + t)
        root.check(JSON.stringify(liveSvc.config) === JSON.stringify({ "codex-live": { command: "missing-codex" } }),
                   "the config changed: " + JSON.stringify(liveSvc.config))
        liveSvc.close()
      }],
      [1800, function() {}]
    ], function() {
      liveSvc.replaceConfig({})
      root.done()
    })
  }

  function liveNewSession() {
    liveSvc.open()
    root.steps([
      [1000, function() {
        root.check(root.liveRunning() && root.liveTexts().indexOf("Rodei os testes.") !== -1, "no conversation: " + root.liveTexts())
        root.check(liveSvc.notchIpc("live", ["codex-live", "new"]) !== false, "new was refused")
      }],
      [1200, function() {
        var t = root.liveTexts()
        root.check(root.liveRunning(), "the new session isn't live: " + t)
        // The fake says the same two lines again: the old ones are gone.
        root.check(t.filter(function(x) { return x === "Rodei os testes." }).length === 1, "the transcript wasn't cleared: " + t)
        liveSvc.close()
      }],
      [2500, function() { root.check(!root.liveRunning(), "closed, it went on") }]
    ], root.done)
  }

  function codexIdle() {
    if (root.noTerminal(codexSvc)) return
    root.go(function*() {
      // 0.04 min: 2.4 s. Wide enough that a loaded machine, slow to run the
      // `kill -0` below, still looks while the program is within it.
      codexSvc.replaceConfig({ codex: { directory: root.dir, idleMinutes: 0.04 } })
      var argsFile = root.dir + "/codex-args"
      var pid = ""
      // The last case's view is still closing.
      yield root.wait(600)
      // The args file is the last case's until this case's program writes it
      // (and the view still closing can show a title): wait for its own pid.
      yield root.runAndWait(["rm", "-f", argsFile])
      codexSvc.open()
      yield root.waitFor(function() { return !!root.terminalIn(codexCard) && root.terminalIn(codexCard).title !== "" }, 5000)
      for (var written = 0; written < 50 && pid === ""; written++) {
        yield root.runAndWait(["cat", argsFile])
        pid = shellCmd.out.split("\n")[2] || ""
        if (pid === "") yield root.wait(100)
      }
      var closedAt = Date.now()
      codexSvc.close()
      yield root.wait(700)
      yield root.runAndWait(["kill", "-0", pid])
      root.check(shellCmd.code === 0, "it stopped before idleMinutes (pid '" + pid + "', after " + (Date.now() - closedAt) + " ms)")
      // It stops once the idle time is up: polled, not a fixed wait.
      for (var tries = 0; tries < 40 && shellCmd.code === 0; tries++) {
        yield root.wait(200)
        yield root.runAndWait(["kill", "-0", pid])
      }
      root.check(shellCmd.code !== 0, "closed and idle, it kept running")
      codexSvc.open()
      yield root.waitFor(function() { return !!root.terminalIn(codexCard) && root.terminalIn(codexCard).title !== "" }, 5000)
      yield root.runAndWait(["cat", argsFile])
      root.check(shellCmd.out.split("\n")[2] !== pid, "opened again, no new program")
      codexSvc.close()
      codexSvc.replaceConfig({})
      yield root.settle(codexSvc)
    })
  }

  // ---------------------------------------------------------------- corners
  // Bug: NotchSurface read every window's cornerItem for the insets side
  // notches take, then threw the side ones away; the binding still depended
  // on them, and a side notch's cornerItem comes from its place, which comes
  // from those insets: ~1300 binding-loop warnings at start, with notches on
  // the top and left edges of one screen. The same bindings as NotchSurface
  // and NotchWindow, on plain objects (PanelWindow can't load offscreen);
  // qml-test.sh fails on the warning a loop logs.
  QtObject {
    id: cornerSurface
    property var windows: []
    readonly property var cornerInsets: Position.corners(cornerSurface.windows.map(function(w) { return Position.cornerInput(w, false) }))
    function insetOf(w) { var i = cornerSurface.windows.indexOf(w); return i >= 0 ? cornerSurface.cornerInsets[i] : null }
  }
  QtObject {
    id: cornerTop
    readonly property string edge: "top"
    readonly property string windowScreenName: "A"
    readonly property var cornerItem: ({ screen: "A", edge: "top", atStart: true, atEnd: false, depth: 30, shown: true })
  }
  QtObject {
    id: cornerSide
    readonly property string edge: "left"
    readonly property string windowScreenName: "A"
    // Its place starts below the inset the top notch gives it.
    readonly property var inset: cornerSurface.insetOf(cornerSide)
    readonly property var cornerItem: ({ screen: "A", edge: "left", atStart: !cornerSide.inset || cornerSide.inset.start === 0,
                                         atEnd: false, depth: 20, shown: true })
  }
  function cornersNoLoop() {
    cornerSurface.windows = [cornerTop, cornerSide]
    var inset = cornerSide.inset
    root.check(!!inset && inset.start > 0, "the side notch takes no inset from the top one: " + JSON.stringify(inset))
    root.check(cornerSide.cornerItem.atStart === false, "the side notch's place ignored its inset")
    cornerSurface.windows = []
    root.done()
  }

  // ------------------------------------------------------- monitor off/on
  // Bug: switching a monitor off and on left the notches gone. The
  // compositor closes the layer surfaces on an output that goes away and
  // Quickshell marks those windows hidden itself; their `visible` binding
  // never changed, so nothing showed them again. A stand-in for the window
  // (PanelWindow can't load offscreen): `closed` is the compositor closing
  // it, and setting `visible` true again (KeepShown.shown) reopens it.
  QtObject {
    id: keptWindow
    property bool closed: false
    readonly property bool visible: windowKept.shown && !keptWindow.closed
    readonly property Connections reopened: Connections {
      target: windowKept
      function onShownChanged() { if (windowKept.shown) keptWindow.closed = false }
    }
  }
  KeepShown {
    id: windowKept
    target: keptWindow
    wanted: false
  }
  function keepShownReopens() {
    windowKept.wanted = true
    windowKept.screen = { name: "A" }
    root.check(keptWindow.visible, "not shown at first")
    root.steps([
      // Its monitor goes off: closed, and its copy moves to another screen.
      [0, function() { keptWindow.closed = true; windowKept.screen = { name: "B" } }],
      [50, function() {
        root.check(keptWindow.visible, "not back on the screen it moved to")
        // Every monitor off: closed, and nowhere to go.
        keptWindow.closed = true
        windowKept.screen = null
      }],
      [50, function() {
        root.check(!keptWindow.visible, "shown with no screen")
        windowKept.screen = { name: "A" }
      }],
      [50, function() {
        root.check(keptWindow.visible, "not back when a monitor came back")
        // Closed on the same screen: back too.
        keptWindow.closed = true
      }],
      [50, function() { root.check(keptWindow.visible, "not back after a close on the same screen") }]
    ], function() {
      windowKept.wanted = false
      windowKept.screen = null
      root.done()
    })
  }

  // ----------------------------------------------------------- tall notch
  // Bug: the open notch stopped at the screen's height, but what it showed
  // didn't: two terminals one above the other on a laptop screen ran past
  // the bottom, cut off, with no way to reach them. Now the modules scroll
  // past the room the card has: the wheel, and a scrollbar.
  TestService {
    id: tallSvc
    ipcTarget: "notchd-qmltest-tall"
    configPath: root.dir + "/tall.json"
    compositor: workspace4
    hostModules: ({ typing: Qt.resolvedUrl("fixtures/typing.qml"), tall: Qt.resolvedUrl("fixtures/tall.qml") })
    defaultModules: ["typing", "tall"]
  }
  FloatingWindow {
    implicitWidth: 700
    implicitHeight: 500
    NotchCard {
      id: tallCard
      notch: tallSvc.notches.length > 0 ? tallSvc.notches[0].copies[0] : null
      x: 50
      maxWidth: 600
      maxHeight: 400
    }
    TestCase { id: wheel; when: false }
  }
  // The card's Flickable, found by what it has.
  function flickableIn(item) {
    if (!item) return null
    if (item.contentY !== undefined && item.contentHeight !== undefined && item.flicking !== undefined) return item
    var kids = item.children || []
    for (var i = 0; i < kids.length; i++) {
      var found = root.flickableIn(kids[i])
      if (found) return found
    }
    return null
  }
  function tallNotchScrolls() {
    var copy = tallSvc.notches.length > 0 ? tallSvc.notches[0].copies[0] : null
    if (!root.check(!!copy, "the notch did not load")) { root.done(); return }
    // One above the other: the stack is what runs past the screen.
    tallSvc.setConfigValue("tabs", false)
    copy.open()
    var flick = null
    root.steps([
      [600, function() {
        root.check(tallCard.mode === "expanded", "the card is " + tallCard.mode)
        root.check(tallCard.height <= 400, "a card with 400 px of room is " + tallCard.height + " tall")
        flick = root.flickableIn(tallCard)
        if (!root.check(!!flick, "the open notch has nothing that scrolls")) return
        root.check(flick.contentHeight > flick.height, "the modules fit, the test proves nothing: "
                   + flick.contentHeight + " in " + flick.height)
        // The wheel over the tall module, which doesn't take it.
        wheel.mouseWheel(tallCard, tallCard.width / 2, tallCard.height / 2, 0, -360)
      }],
      [100, function() {
        if (flick) root.check(flick.contentY > 0, "the wheel did not scroll the modules")
      }]
    ], function() {
      copy.close()
      tallSvc.setConfigValue("tabs", undefined)
      root.done()
    })
  }

  // With the notch's `tabs`, the open notch shows one module at a time:
  // the first, or the one whose tab is picked; a module that opens the notch
  // (a shortcut, IPC: openNotch) picks its own. Pinned, the window takes
  // the keyboard whichever tab shows.
  function tabsShowOne() {
    root.go(function*() {
      var copy = tallSvc.notches.length > 0 ? tallSvc.notches[0].copies[0] : null
      var typing = tallSvc.moduleById("typing")
      var tall = tallSvc.moduleById("tall")
      if (!root.check(!!copy && !!typing && !!tall, "the notch or its modules did not load")) return
      tallSvc.setConfigValue("tabs", false)
      yield root.settle(tallSvc)
      root.check(copy.shownModules.length === 2 && copy.tabModules.length === 0, "without tabs it shows "
                 + copy.shownModules.length + " modules and " + copy.tabModules.length + " tabs")
      // Tabs are the default for a notch with several modules.
      tallSvc.setConfigValue("tabs", undefined)
      yield root.settle(tallSvc)
      root.check(copy.tabModules.length === 2, "by default, " + copy.tabModules.length + " tabs")
      root.check(copy.shownModules.length === 1 && copy.shownModules[0] === typing, "with tabs it does not show the first module alone")
      tall.openNotch()
      yield root.settle(tallSvc)
      root.check(copy.pinned, "opening for a module did not open the notch")
      root.check(copy.shownModules.length === 1 && copy.shownModules[0] === tall, "opening for a module did not pick its tab")
      root.check(copy.keyboardFocus === "exclusive", "pinned on another tab, it takes " + copy.keyboardFocus)
      copy.showTab("typing")
      yield root.settle(tallSvc)
      root.check(copy.keyboardFocus === "exclusive", "on the typing module's tab, pinned, it takes " + copy.keyboardFocus)
      copy.close()
      tallSvc.setConfigValue("tabs", false)
      yield root.settle(tallSvc)
      root.check(copy.tabModules.length === 0, "tabs off, " + copy.tabModules.length + " tabs")
      tallSvc.setConfigValue("tabs", undefined)
    })
  }

  // Two modules, with tabs: the closed notch and missed news.
  TestService {
    id: tabsSvc
    ipcTarget: "notchd-qmltest-tabs"
    configPath: root.dir + "/tabs.json"
    compositor: workspace4
    hostModules: ({ tall: Qt.resolvedUrl("fixtures/tall.qml"), news: Qt.resolvedUrl("fixtures/news.qml") })
    defaultModules: ["tall", "news"]
  }
  NotchCard {
    id: tabsCard
    notch: tabsSvc.notches.length > 0 ? tabsSvc.notches[0].copies[0] : null
  }

  // Bug: with tabs, the closed notch showed only the first module (codex),
  // the others (claude, the timer) a tab away even when it was closed. Now
  // it shows every active module's compact view, side by side.
  function tabsCompactAll() {
    root.go(function*() {
      var copy = tabsSvc.notches.length > 0 ? tabsSvc.notches[0].copies[0] : null
      if (!root.check(!!copy, "the notch did not load")) return
      tabsSvc.setConfigValue("tabs", false)
      yield root.settle(tabsSvc)
      root.check(copy.compactModules.length === 1, "without tabs the closed notch shows " + copy.compactModules.length + " modules")
      tabsSvc.setConfigValue("tabs", undefined)
      yield root.settle(tabsSvc)
      yield root.wait(600)
      root.check(copy.mode === "compact", "the notch is " + copy.mode)
      root.check(copy.compactModules.length === 2, "with tabs the closed notch shows " + copy.compactModules.length + " modules")
      // 80 and 60 px wide, a line and the gaps between them.
      root.check(tabsCard.width >= 80 + 60 + 2 * Theme.space(10), "a closed notch with both is " + tabsCard.width + " wide")
      tabsSvc.setConfigValue("tabs", undefined)
    })
  }

  // Bug: with tabs, an agent that finished on a tab you weren't looking at
  // asked for its peek while the notch was open, which takes no peeks, and
  // the news was lost (codex peeked, claude never did). A peek of news you
  // haven't seen (NotchModule.attention) now shows as the notch closes,
  // once: hovering and leaving again doesn't repeat it.
  function missedPeekOnClose() {
    root.go(function*() {
      var copy = tabsSvc.notches.length > 0 ? tabsSvc.notches[0].copies[0] : null
      var news = tabsSvc.moduleById("news")
      if (!root.check(!!copy && !!news, "the notch or its module did not load")) return
      tabsSvc.setConfigValue("tabs", true)
      copy.open()
      news.attention = true
      news.requestPeek(3000)
      yield root.settle(tabsSvc)
      root.check(copy.peekModule === null, "an open notch peeked")
      copy.close()
      yield root.settle(tabsSvc)
      root.check(copy.peekModule === news, "closing did not show the missed peek")
      tabsSvc.dismissPeek(news)
      // Hovering and leaving again does not show it again.
      root.hover(copy)
      yield root.waitFor(function() { return copy.mode === "expanded" })
      root.unhover(copy)
      yield root.waitFor(function() { return copy.mode === "compact" })
      yield root.settle(tabsSvc)
      root.check(copy.peekModule === null, "the missed peek showed again")
      // News you saw before the notch closed doesn't peek.
      copy.open()
      news.requestPeek(3000)
      news.attention = false
      copy.close()
      yield root.settle(tabsSvc)
      root.check(copy.peekModule === null, "news already seen peeked")
      news.attention = false
      tabsSvc.setConfigValue("tabs", undefined)
    })
  }

  // A user module (tests/modules/hello, a process the daemon runs) has no QML
  // file: the service makes a CoreModule from the daemon's `module` message,
  // and the notch shows its tree like any module's. A press in its view goes
  // back to the module as an event, and its answer comes back as a new view.
  TestService {
    id: userSvc
    ipcTarget: "notchd-qmltest-user"
    configPath: root.dir + "/user.json"
    compositor: twoScreens
    defaultModules: ["hello"]
  }
  NotchCard {
    id: userCard
    notch: userSvc.notches.length > 0 ? userSvc.notches[0].copies[0] : null
  }
  function userModuleShows() {
    root.go(function*() {
      yield root.waitFor(function() { var m = userSvc.moduleById("hello"); return !!m && !!m.compact && m.active }, 6000)
      var m = userSvc.moduleById("hello")
      if (!root.check(!!m, "no module for hello")) return
      root.check(m.title === "Hello" && m.priority === 40 && m.icon === "H", "facts: " + m.title + " " + m.priority + " " + m.icon)
      root.check(root.texts(m.tree("compact")).join("|") === "hello there", "compact: " + JSON.stringify(m.tree("compact")))
      var copy = userSvc.notches[0].copies[0]
      yield root.settle(userSvc)
      root.check(copy.mode === "compact" && copy.compactModule === m, "the notch shows " + copy.mode)
      root.check(userCard.height > userCard.restDepth, "the closed notch draws nothing: " + userCard.height + " deep")
      root.check(!!m.expanded && !m.peek && !m.verticalCompact, "slots: expanded " + !!m.expanded + ", peek " + !!m.peek)
      // The expanded view's button: its action goes to the module, whose
      // answer is a new compact view.
      var view = m.expanded.createObject(userCard)
      root.check(root.texts(m.tree("expanded")).indexOf("Poke") !== -1, "no button in its expanded view")
      view.action("poke", null)
      yield root.waitFor(function() { return root.texts(m.tree("compact")).indexOf("poked") !== -1 }, 4000)
      root.check(root.texts(m.tree("compact")).indexOf("poked") !== -1, "the button did not reach the module: ")
      view.destroy()
    })
  }

  // A user module in the config's modules/ that no notch lists (tests/modules/
  // gauge, with a setting): the daemon says it and its page, and the settings
  // offer it like a built-in: the Modules page, a notch's list, its page and
  // default. Switching it on in the list loads it through a CoreModule.
  TestService {
    id: shelfSvc
    ipcTarget: "notchd-qmltest-shelf"
    configPath: root.dir + "/shelf.json"
    compositor: twoScreens
    defaultModules: ["activities"]
  }
  ModuleList {
    id: shelfList
    svc: shelfSvc
    visible: false
    value: shelfSvc.configValue("modules")
    onEdited: function(v) { shelfSvc.setConfigValue("modules", v) }
  }
  function userModuleInSettings() {
    root.go(function*() {
      var s = shelfSvc
      yield root.waitFor(function() {
        return s.settingsPages.some(function(p) { return p.id === "modules/gauge" })
      }, 6000)
      root.check(s.settingsPages.some(function(p) { return p.id === "modules/gauge" && p.label === "Gauge" }), "no page for gauge")
      var entry = s.moduleCatalog.filter(function(m) { return m.moduleId === "gauge" })[0]
      if (!root.check(!!entry, "gauge is not in the Modules page's catalog")) return
      root.check(entry.title === "Gauge" && !entry.standalone && entry.settings.length === 1, "catalog entry: " + JSON.stringify(entry))
      root.check(s.moduleSettingDefault("gauge", "unit") === "kg" && s.moduleSetting("gauge", "unit") === "kg", "its default setting is " + s.moduleSettingDefault("gauge", "unit"))
      root.check(s.isStandalone("gauge") === false && s.isAgent("gauge") === false && s.isLive("gauge") === false, "kind of gauge")
      root.check(s.reportedCatalog.every(function(m) { return m.moduleId !== "gauge" }), "the daemon was told of its own module")
      root.check(shelfList.all.indexOf("gauge") !== -1 && shelfList.available.indexOf("gauge") !== -1, "a notch's list does not offer gauge")
      shelfList.toggle("gauge")
      yield root.waitFor(function() { var m = s.moduleById("gauge"); return !!m && !!m.compact }, 6000)
      var m = s.moduleById("gauge")
      if (!root.check(!!m, "adding gauge to the notch did not load it")) return
      root.check(m.title === "Gauge" && m.icon === "G", "facts: " + m.title + " " + m.icon)
      root.check(root.texts(m.tree("compact")).join("|") === "gauge on", "compact: " + JSON.stringify(m.tree("compact")))
    })
  }

  // A timer that isn't running has no compact view: the closed notch was
  // empty (a notch of its own rested; beside claude and codex it was
  // missing). Now an idle module shows its icon there (NotchModule.icon),
  // unless the notch's idleIcons is off.
  TestService {
    id: idleSvc
    ipcTarget: "notchd-qmltest-idle"
    configPath: root.dir + "/idle.json"
    compositor: workspace4
    defaultModules: ["timer", "news"]
    hostModules: ({ news: Qt.resolvedUrl("fixtures/news.qml") })
  }
  NotchCard {
    id: idleCard
    notch: idleSvc.notches.length > 0 ? idleSvc.notches[0].copies[0] : null
  }
  function idleTimerIcon() {
    root.go(function*() {
      var copy = idleSvc.notches.length > 0 ? idleSvc.notches[0].copies[0] : null
      var timer = idleSvc.moduleById("timer")
      if (!root.check(!!copy && !!timer, "the notch or the timer did not load")) return
      root.check(!timer.compact, "the timer is not idle (test setup)")
      // The module list changing may load the modules again: ask each time.
      // Alone in its notch.
      idleSvc.setConfigValue("modules", ["timer"])
      yield root.wait(500)
      yield root.settle(idleSvc)
      timer = idleSvc.moduleById("timer")
      root.check(copy.mode === "compact" && copy.compactModules[0] === timer,
                 "an idle timer leaves the notch " + copy.mode + ", showing " + copy.compactModules.map(function(m) { return m.moduleId }))
      root.check(idleCard.height > idleCard.restDepth, "the closed notch shows nothing: " + idleCard.height + " deep")
      idleSvc.setConfigValue("idleIcons", false)
      yield root.wait(500)
      yield root.settle(idleSvc)
      root.check(copy.mode === "rest", "idleIcons off, an idle timer leaves the notch " + copy.mode)
      // Beside another module, with tabs (the default): both.
      idleSvc.setConfigValue("idleIcons", undefined)
      idleSvc.setConfigValue("modules", undefined)
      yield root.wait(500)
      yield root.settle(idleSvc)
      root.check(copy.compactModules.length === 2, "beside another module, the closed notch shows "
                 + copy.compactModules.length + " modules")
    })
  }

  // ------------------------------------------------------ the daemon owns it
  // The config, the schema and IPC are the daemon's (docs/architecture.md).
  // These cases have services of their own, so that killing a daemon leaves
  // no other case without one.
  TestService {
    id: daemonSvc
    ipcTarget: "notchd-qmltest-daemon"
    configPath: root.dir + "/daemon.json"
    compositor: workspace4
    defaultModules: ["activities"]
  }
  TestService {
    id: killedSvc
    ipcTarget: "notchd-qmltest-killed"
    configPath: root.dir + "/killed.json"
    compositor: workspace4
    defaultModules: ["activities"]
    daemonDownDelay: 150
  }
  TestService {
    id: downSvc
    ipcTarget: "notchd-qmltest-down"
    configPath: root.dir + "/down.json"
    compositor: workspace4
    defaultModules: ["activities"]
    daemonDownDelay: 150
  }
  FileView {
    id: daemonFile
    path: root.dir + "/daemon.json"
    blockLoading: true
    printErrors: false
  }
  FileView {
    id: downFile
    path: root.dir + "/down.json"
    blockLoading: true
    printErrors: false
  }
  SettingRow { id: daemonRow; svc: daemonSvc }

  // A config file as the daemon wrote it, null while it is missing or torn.
  function fileConfig(view) {
    view.reload()
    try { return JSON.parse(view.text()) } catch (e) { return null }
  }
  // Whether the file has `key` at `value` (`undefined`: no such key).
  function fileHas(view, key, value) {
    var c = root.fileConfig(view)
    return !!c && (value === undefined ? c[key] === undefined : c[key] === value)
  }
  function ipcHandler(svc) {
    for (var i = 0; i < svc.data.length; i++) {
      var o = svc.data[i]
      if (o && o.target === svc.ipcTarget && typeof o.set === "function") return o
    }
    return null
  }

  // The window's row for a field the daemon's schema gives: the edit applies
  // at once, goes to the daemon, which writes the file and sends it back. A
  // value equal to the default removes the key (the file never holds
  // defaults); many edits of one key (a slider) reach the daemon as one.
  function daemonSettingsRoundTrip() {
    var s = daemonSvc
    function field() { return s.settingsSchema.filter(function(f) { return f.key === "hoverOpenDelay" })[0] }
    root.until(function() { return s.daemonReady && !!field() }, function() {
      var f = field()
      if (!root.check(!!f, "the open delay is not in the schema the daemon sent")) { root.done(); return }
      root.check(f.type === "integer" && f.page === "general" && f.section === "Hover" && f.default !== undefined,
                 "the field from the daemon: " + JSON.stringify(f))
      root.check(s.settingsPages.some(function(p) { return p.id === "general" }), "no General page")
      daemonRow.field = f
      daemonRow.edit(500)
      root.check(s.hoverOpenDelay === 500, "the edit did not apply at once: " + s.hoverOpenDelay)
      root.until(function() { return root.fileHas(daemonFile, "hoverOpenDelay", 500) && s.client.config.cfg.hoverOpenDelay === 500 }, function() {
        root.check(root.fileHas(daemonFile, "hoverOpenDelay", 500), "the daemon did not write the edit: " + JSON.stringify(root.fileConfig(daemonFile)))
        root.check(s.client.config.cfg.hoverOpenDelay === 500, "the daemon's config is " + JSON.stringify(s.client.config.cfg))
        // Back to the default: the key goes, the file is empty again.
        daemonRow.edit(f.default)
        root.until(function() { return root.fileHas(daemonFile, "hoverOpenDelay", undefined) }, function() {
          var c = root.fileConfig(daemonFile)
          root.check(!!c && Object.keys(c).length === 0, "a default was written: " + JSON.stringify(c))
          root.check(s.hoverOpenDelay === f.default, "the delay is " + s.hoverOpenDelay)
          // A drag: twenty steps in a burst, one write, the last value.
          for (var i = 1; i <= 20; i++) s.setConfigValue("hoverCloseDelay", 100 + i)
          root.check(s.pendingSets.length === 1, "the burst is " + s.pendingSets.length + " sets, not one")
          root.until(function() { return root.fileHas(daemonFile, "hoverCloseDelay", 120) && !s.configSyncing }, function() {
            root.check(root.fileHas(daemonFile, "hoverCloseDelay", 120), "the last step is not in the file")
            root.check(s.hoverCloseDelay === 120, "the service settled on " + s.hoverCloseDelay)
            s.setConfigValue("hoverCloseDelay", undefined)
            s.flushConfig()
            root.until(function() { return root.fileHas(daemonFile, "hoverCloseDelay", undefined) }, function() { root.done() })
          })
        })
      })
    })
  }

  // `qs ipc call <target> set` answers as it did, and the edit goes to the
  // daemon (an IPC handler cannot wait for a reply, so the answer is worked
  // out here). The daemon's own `ipc set` (what `notchd ipc` calls) and an
  // edit of the file by hand reach the UI as the daemon's `config`; a file
  // that does not parse is ignored, the last config stays.
  function daemonIpcAndFile() {
    var s = daemonSvc
    var h = root.ipcHandler(s)
    root.until(function() { return s.daemonReady }, function() {
      if (!root.check(!!h, "no IPC handler")) { root.done(); return }
      root.check(h.set("hoverCloseDelay", "333") === "ok", "set did not answer ok")
      root.check(h.set("hoverCloseDelay", "nope") === "not JSON: nope", "a bad value did not say so")
      root.check(h.set("", "1") === "usage: set <key> <json value>", "no key did not say the usage")
      root.until(function() { return root.fileHas(daemonFile, "hoverCloseDelay", 333) }, function() {
        root.check(root.fileHas(daemonFile, "hoverCloseDelay", 333) && s.hoverCloseDelay === 333, "the set did not reach the file and the service")
        root.check(h.set("hoverCloseDelay", "") === "ok", "an empty value did not answer ok")
        root.until(function() { return root.fileHas(daemonFile, "hoverCloseDelay", undefined) }, function() {
          root.check(root.fileHas(daemonFile, "hoverCloseDelay", undefined) && s.hoverCloseDelay !== 333, "an empty value did not reset the key")
          // The daemon's own ipc: the same method with no QML write, and `state`.
          var answer = ""
          s.client.ipc("set", ["hoverOpenDelay", "444"], function(reply) { answer = reply.result })
          s.client.ipc("state", [], function(reply) { answer += "|" + (reply.t === "ok" && Array.isArray(JSON.parse(reply.result).notches)) })
          root.until(function() { return s.hoverOpenDelay === 444 && answer === "ok|true" }, function() {
            root.check(s.hoverOpenDelay === 444, "the daemon's set did not reach the UI")
            root.check(answer === "ok|true", "the daemon answered " + answer)
            // A hand edit of the file.
            daemonFile.setText("{ \"hoverOpenDelay\": 777 }\n")
            root.until(function() { return s.hoverOpenDelay === 777 }, function() {
              root.check(s.hoverOpenDelay === 777, "an edit of the file did not reach the UI: " + s.hoverOpenDelay)
              // Bug: the lists in the daemon's config reached the UI as sequences
              // (Array.isArray false), so a hand-written `notches` or `modules`
              // was read as no list: one notch, the default modules.
              daemonFile.setText("{ \"hoverOpenDelay\": 778, \"notches\": [ { \"id\": \"one\", \"modules\": [\"activities\"] }, { \"id\": \"two\", \"modules\": [] } ] }\n")
              // The notch list follows the config in the daemon's next message: wait for both.
              root.until(function() { return s.hoverOpenDelay === 778 && s.notchIds.length === 2 }, function() {
                root.check(s.notchSpecs.length === 2 && s.notchSpecs[0].id === "one", "the notches of the file: " + JSON.stringify(s.notchSpecs.map(function(n) { return n.id })))
                root.check(s.moduleIds.indexOf("activities") !== -1 && Array.isArray(s.config.notches), "the lists of the file are not lists: " + JSON.stringify(s.moduleIds))
                root.check(s.notchIds.length === 2, "the daemon's notches: " + JSON.stringify(s.notchIds))
                daemonFile.setText("{ \"hoverOpenDelay\": 777 }\n")
                root.until(function() { return s.hoverOpenDelay === 777 && s.notchSpecs.length === 1 }, function() {
              // A torn file is ignored: the last config stays.
              daemonFile.setText("{ \"hoverOpenDelay\": ")
              root.steps([[700, function() {}]], function() {
                root.check(s.hoverOpenDelay === 777, "a bad file changed the config to " + s.hoverOpenDelay)
                daemonFile.setText("{}\n")
                root.until(function() { return s.hoverOpenDelay !== 777 }, function() {
                  root.check(s.hoverOpenDelay === 120, "the delay is " + s.hoverOpenDelay)
                  root.done()
                })
              })
                }, 5000)
              }, 5000)
            })
          })
        })
      })
    })
  }

  // The daemon killed while the shell runs: the notch says notchd is not
  // running (after a moment, not at once), keeps its last config, an edit
  // made meanwhile waits for the daemon, and the launcher brings it back by
  // itself: the notch is normal again and the edit is in the file.
  function daemonDownAndBack() {
    var s = downSvc
    // Long enough for the notch to say so before the daemon is back.
    s.launcher.restartMin = 700
    root.until(function() { return s.daemonReady }, function() {
      root.check(!s.daemonDown && s.client.ready, "down while the daemon runs")
      s.setConfigValue("hoverOpenDelay", 321)
      s.flushConfig()
      root.until(function() { return root.fileHas(downFile, "hoverOpenDelay", 321) && !s.configSyncing && s.notches.length > 0 }, function() {
        var copies = s.copies.length
        s.client.ipc("shutdown", [])
        root.until(function() { return s.daemonDown }, function() {
          root.check(s.daemonDown && !s.client.ready, "the notch does not say the daemon is gone")
          root.check(s.hoverOpenDelay === 321, "the last config went with the daemon: " + s.hoverOpenDelay)
          // The notch stays as it was, frozen, not blank.
          root.check(s.notches.length > 0 && s.copies.length === copies && s.copies[0].mode !== undefined, "the notches went with the daemon: " + s.notches.length)
          // An edit while it is away waits for it.
          s.setConfigValue("hoverCloseDelay", 222)
          root.until(function() { return s.client.ready && !s.daemonDown && root.fileHas(downFile, "hoverCloseDelay", 222) }, function() {
            root.check(s.client.ready && !s.daemonDown, "the daemon did not come back by itself")
            root.check(root.fileHas(downFile, "hoverCloseDelay", 222) && root.fileHas(downFile, "hoverOpenDelay", 321),
                       "the file is " + JSON.stringify(root.fileConfig(downFile)))
            root.check(s.hoverOpenDelay === 321 && s.hoverCloseDelay === 222, "the config is " + s.hoverOpenDelay + ", " + s.hoverCloseDelay)
            root.check(s.launcher.restarts === 0, "the backoff did not start over: " + s.launcher.restarts)
            root.done()
          }, 8000)
        }, 5000)
      })
    })
  }

  // Bug: the daemon killed (SIGKILL) while nothing else happened never came
  // back into the notch: "notchd not running" stayed until a settings edit
  // nudged the client. Only a graceful shutdown with an edit meanwhile was
  // tested.
  function daemonKilledComesBack() {
    var s = killedSvc
    root.until(function() { return s.daemonReady }, function() {
      root.run(["pkill", "-KILL", "-f", "--socket " + s.daemonSocket])
      root.until(function() { return s.daemonDown }, function() {
        root.until(function() { return s.client.ready && !s.daemonDown }, function() {
          root.check(s.client.ready && !s.daemonDown, "the daemon did not come back into the notch: client " + s.client.status
                     + ", launcher " + s.launcher.state + " " + s.launcher.error)
          root.done()
        }, 10000)
      }, 5000)
    })
  }

  // Bug: `push` (and `timer`, `clear`) for a module no notch loads answered
  // `ok` and did nothing, since the IPC handler cannot wait for the daemon;
  // before the daemon it said `<id> module not loaded`. It says so again.
  function ipcModuleNotLoaded() {
    var s = daemonSvc
    var h = root.ipcHandler(s)
    root.until(function() { return s.daemonReady }, function() {
      if (!root.check(!!h, "no IPC handler")) { root.done(); return }
      root.check(h.timer("5m") === "timer module not loaded", "timer, not loaded: " + h.timer("5m"))
      root.check(h.push('{"id":"x","title":"t"}') === "ok", "push, loaded: " + h.push('{"id":"x","title":"t"}'))
      root.check(h.push("nope") === "invalid json", "push, bad json: " + h.push("nope"))
      h.clear("x")
      root.done()
    })
  }
}
