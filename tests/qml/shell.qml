import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "core"
import "core/settings"
import "core/Position.js" as Position
import "core/Live.js" as Live

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

  Component.onCompleted: {
    var all = [
      { name: "workspace chips store a real list", run: root.workspaceChipsList },
      { name: "compact depth ignores peek and expanded transitions", run: root.compactDepthTransitions },
      { name: "a joined row's depth ignores a peek opening and closing", run: root.closedDepthTransitions },
      { name: "leaving the notch before it opens forgets the screen", run: root.hoverLeaveForgetsScreen },
      { name: "peeks and the open notch never move windows", run: root.reserveIgnoresOpenNotch },
      { name: "reserved space grows at once and shrinks after the delay", run: root.reserveGrowsAndShrinks },
      { name: "one module ending as another starts keeps the space", run: root.reserveBridgesModules },
      { name: "the space shrinks as the notch closes, not seconds later", run: root.reserveShrinksPromptly },
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
      { name: "a module you type into takes the keyboard only while pinned open", run: root.keyboardOnlyWhilePinned },
      { name: "Escape in a field inside the notch closes it, and Ctrl+Escape on the notch", run: root.escapeCloses },
      { name: "a module sets the width of its expanded view", run: root.expandedWidthFromModule },
      { name: "a notch in the list dropped on the top edge stays there", run: root.dropOnTopStays },
      { name: "corners: a side notch's place never feeds back into the insets it takes", run: root.cornersNoLoop },
      { name: "codex: without qmltermwidget, the notch says what to install", run: root.codexMissing },
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
      { name: "terminal: the band behind your messages (ANSI 8) is gray, not orange", run: root.messageBandGray },
      { name: "a window its monitor took away comes back with the monitor", run: root.keepShownReopens },
      { name: "an open notch taller than its screen fits it and scrolls", run: root.tallNotchScrolls },
      { name: "with tabs the open notch shows one module, and opening for a module picks its tab", run: root.tabsShowOne },
      { name: "with tabs the closed notch shows every module side by side", run: root.tabsCompactAll },
      { name: "news missed while the notch was open peeks once as it closes", run: root.missedPeekOnClose },
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
  Timer { id: warmup; interval: 300; onTriggered: root.next() }

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

  NotchService {
    id: service
    ipcTarget: "notchd-qmltest"
    configPath: root.dir + "/config.json"
    compositor: workspace4
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

  NotchService {
    id: multi
    ipcTarget: "notchd-qmltest-multi"
    configPath: root.dir + "/multi.json"
    compositor: twoScreens
    screens: [{ name: "A" }, { name: "B" }]
    defaultModules: ["activities"]
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
    var screen = { name: "test" }
    chips.toggle(1); chips.toggle(2); chips.toggle(3)
    var notch = service.notches[0]
    root.check(Array.isArray(notch.workspaces), "workspaces is not an array: " + JSON.stringify(notch.workspaces))
    root.check(JSON.stringify(notch.workspaces) === "[1,2,3]", "workspaces is " + JSON.stringify(notch.workspaces))
    root.check(!notch.showsOn(screen), "shows on workspace 4 with [1,2,3]")
    chips.toggle(4)
    root.check(notch.showsOn(screen), "does not show on workspace 4 after adding it")
    chips.toggle(4)
    root.check(!notch.showsOn(screen), "still shows on workspace 4 after removing it")
    chips.toggle(1); chips.toggle(2); chips.toggle(3)
    root.check(service.configValue("workspaces") === undefined, "an empty pick left the key in the config")
    root.check(notch.showsOn(screen), "no pick should mean every workspace")
    root.done()
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
    var copy = service.firstCopy
    copy.setPointer("notch", true, "DP-1")
    copy.setPointer("notch", false, "")
    root.check(copy.mode !== "expanded", "opened without waiting for the hover delay")
    root.check(copy.openScreen === "", "kept screen " + copy.openScreen)
    root.done()
  }

  // ------------------------------------------------------ reserved space
  // The space follows the closed notch, so the volume OSD (a peek) and the
  // expanded notch open over windows without moving them; it grows at once
  // and shrinks after a delay.
  ReservedSpace {
    id: reserve
    notch: cardSvc
    notchCard: card
    shrinkDelay: 300
    property int lowest: 0
    property int highest: 0
    function watch() { lowest = space; highest = space }
    onSpaceChanged: { lowest = Math.min(lowest, space); highest = Math.max(highest, space) }
  }

  // Closed notch: compact (a module active) or rest.
  function closeAs(compact) {
    cardSvc.compactModule = compact ? fakeModule : null
    cardSvc.mode = compact ? "compact" : "rest"
  }

  function reserveIgnoresOpenNotch() {
    var settle = 450
    var compact = 0
    root.steps([
      [0, function() { root.closeAs(true) }],
      [settle, function() { compact = reserve.space; reserve.watch(); cardSvc.mode = "peek" }],
      [settle, function() { cardSvc.mode = "expanded" }],
      [settle, function() { cardSvc.mode = "compact" }],
      [settle, function() {
        root.check(reserve.lowest === compact && reserve.highest === compact,
                   "compact: moved between " + reserve.lowest + " and " + reserve.highest + ", closed is " + compact)
        root.closeAs(false)
      }],
      [settle * 2, function() { reserve.watch(); cardSvc.mode = "peek" }],  // the OSD, from rest
      [settle, function() { cardSvc.mode = "rest" }],
      [settle, function() {}]
    ], function() {
      root.check(reserve.space === Math.ceil(card.restDepth), "rest reserves " + reserve.space + ", the lip is " + card.restDepth)
      root.check(reserve.lowest === reserve.highest, "a peek from rest moved it between " + reserve.lowest + " and " + reserve.highest)
      root.done()
    })
  }

  function reserveGrowsAndShrinks() {
    var rest = Math.ceil(card.restDepth)
    var compact = 0
    root.steps([
      [0, function() { root.closeAs(false) }],
      [450, function() {
        root.check(reserve.space === rest, "rest reserves " + reserve.space)
        root.closeAs(true)
        compact = reserve.space
        root.check(compact > rest, "did not grow at once: " + compact)
        root.closeAs(false)
      }],
      [150, function() { root.check(reserve.space === compact, "shrank before the delay: " + reserve.space) }],
      [350, function() { root.check(reserve.space === rest, "did not shrink after the delay: " + reserve.space) }]
    ], function() { root.done() })
  }

  function reserveBridgesModules() {
    var compact = 0
    root.steps([
      [0, function() { root.closeAs(true) }],
      [450, function() { compact = reserve.space; reserve.watch(); root.closeAs(false) }],
      [100, function() { root.closeAs(true) }],
      [500, function() {}]
    ], function() {
      root.check(reserve.lowest === compact, "dipped to " + reserve.lowest + " between modules, compact is " + compact)
      root.done()
    })
  }

  // ------------------------------------------------------------ edges
  // Moving a compact notch to a side edge swaps its view for the dot (the
  // fake module has no verticalCompact). For the length of the fade the old,
  // wide view is still loaded; measured on the new axis it would read as a
  // notch 130 px deep and push windows that far.
  function reserveFollowsEdge() {
    var settle = 450
    var top = 0
    root.steps([
      [0, function() { root.closeAs(true) }],
      [settle, function() { top = reserve.space; reserve.watch(); cardSvc.edge = "left" }],
      [settle, function() {}]
    ], function() {
      root.check(card.vertical, "the card did not take the left edge")
      root.check(reserve.highest <= top, "grew to " + reserve.highest + " moving to the side; compact on top is " + top)
      root.check(reserve.space === Math.ceil(card.compactDepth) && reserve.space > 0,
                 "reserves " + reserve.space + ", the side compact view is " + card.compactDepth + " deep")
      cardSvc.edge = "top"
      root.done()
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

  // Bug: when the last module ended, windows stayed clear of a notch that
  // was already back to its lip for 4 s (the default shrink delay). The
  // space now goes once the notch has collapsed.
  ReservedSpace {
    id: defaultReserve
    notch: cardSvc
    notchCard: card
  }

  function reserveShrinksPromptly() {
    var rest = Math.ceil(card.restDepth)
    root.steps([
      [0, function() { root.closeAs(true) }],
      [450, function() {
        root.check(defaultReserve.space > rest, "compact did not grow the space: " + defaultReserve.space)
        root.closeAs(false)
      }],
      [600, function() {}]
    ], function() {
      root.check(defaultReserve.space === rest,
                 "still " + defaultReserve.space + " px 600 ms after the notch closed; the lip is " + rest)
      root.done()
    })
  }

  // Dragging turns the card to each edge it passes; the space waits for the
  // drop instead of following it there and back.
  function reserveHeldWhileDragging() {
    var compact = 0
    root.steps([
      [0, function() { root.closeAs(true) }],
      [450, function() {
        compact = reserve.space
        reserve.watch()
        cardSvc.dragging = true
        cardSvc.edge = "left"
        root.closeAs(false)
      }],
      [600, function() {
        root.check(reserve.lowest === compact && reserve.highest === compact,
                   "moved between " + reserve.lowest + " and " + reserve.highest + " mid-drag; it was " + compact)
        cardSvc.edge = "top"
        cardSvc.dragging = false
      }],
      [600, function() {}]
    ], function() {
      root.check(reserve.space === Math.ceil(card.restDepth), "after the drop it reserves " + reserve.space)
      root.done()
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
    }, fn)
  }
  function push(id, peek) {
    return multi.moduleById("activities").push(JSON.stringify({ id: id, title: id, sticky: true, peek: peek }))
  }

  // Bug: one notch window moved between screens. With media showing on one
  // monitor, changing the volume on another took the notch away from it.
  // A notch on every screen now peeks on the focused one only.
  function peekOnFocusedCopy() {
    var main = null
    root.steps([
      [0, function() { multi.replaceConfig(root.twoNotches) }],
      [300, function() {
        main = multi.notchById("main")
        root.check(!!main && main.copies.length === 2, "main has " + (main ? main.copies.length : 0) + " copies, not one per screen")
        root.check(root.push("build", false) === "ok", "activities not loaded")
      }],
      [100, function() {
        root.check(main.copyOn("A").mode === "compact" && main.copyOn("B").mode === "compact",
                   "a quiet push left " + main.copyOn("A").mode + " / " + main.copyOn("B").mode)
        twoScreens.focusedScreenName = "B"
        root.push("deploy", true)
      }],
      [100, function() {
        root.check(main.copyOn("B").mode === "peek", "the focused copy shows " + main.copyOn("B").mode)
        root.check(main.copyOn("A").mode === "compact", "the copy on the other screen shows " + main.copyOn("A").mode)
        root.check(multi.peeking(multi.moduleById("activities")), "the service does not report the peek")
        multi.dismissPeek(null)
        twoScreens.focusedScreenName = "A"
        root.push("lint", true)
      }],
      [100, function() {
        root.check(main.copyOn("A").mode === "peek" && main.copyOn("B").mode === "compact",
                   "focus on A: " + main.copyOn("A").mode + " / " + main.copyOn("B").mode)
        multi.dismissPeek(null)
      }]
    ], function() { root.done() })
  }

  // Bug: the visitor's copies (a fullscreen window per screen, hidden while
  // closed) showed the compact view of whatever was active. Music playing
  // kept the equalizer drawing frames in windows nobody could see.
  function visitorRestsWhileClosed() {
    var visitor = multi.visitor
    root.push("quiet", false)
    root.steps([
      [100, function() {
        root.check(visitor.copies.length === 2, "the visitor has " + visitor.copies.length + " copies, not one per screen")
        visitor.copies.forEach(function(c) {
          root.check(c.mode === "rest", "a closed visitor copy on " + c.fixedScreen + " shows " + c.mode)
        })
        twoScreens.focusedScreenName = "B"
        multi.requestPeek(multi.moduleById("activities"), 2000)
      }],
      [100, function() {
        root.check(visitor.copies.some(function(c) { return c.mode === "peek" }) || multi.peeking(multi.moduleById("activities")),
                   "nothing peeks")
        multi.dismissPeek(null)
      }]
    ], function() { root.done() })
  }

  // The visitor's copies held a fullscreen window each (its buffers, ~25 MB
  // per screen) while hidden. A guest's window now goes when it closes,
  // after `windowLinger`, so the close animation ends and a peek soon after
  // reuses it.
  function visitorWindowOnlyWhileShown() {
    var visitor = multi.visitor
    var copy = null
    var saved = multi.config
    visitor.copies.forEach(function(c) { c.windowLinger = 300 })
    // A notch kept on A, focus on B: a peek there goes to the visitor.
    multi.replaceConfig({ modules: ["activities"], position: { screen: "A" } })
    twoScreens.focusedScreenName = "B"
    root.steps([
      [100, function() {
        root.check(visitor.copies.every(function(c) { return !c.needsWindow }),
                   "a closed visitor copy wants a window: " + visitor.copies.map(function(c) { return c.mode }))
        root.check(multi.notches[0].copies.every(function(c) { return c.needsWindow }), "a notch of the config lost its window")
        multi.requestPeek(multi.moduleById("activities"), 400)
      }],
      [100, function() {
        copy = visitor.copyOn("B")
        root.check(!!copy && copy.mode === "peek", "the visitor does not peek on B: " + (copy ? copy.mode : "no copy"))
        root.check(!!copy && copy.needsWindow, "a peeking visitor copy has no window")
      }],
      [500, function() {
        root.check(copy.mode !== "peek", "the peek did not end")
        root.check(copy.needsWindow, "the window went before the notch finished closing")
      }],
      [400, function() {
        root.check(!copy.needsWindow, "the window stayed after windowLinger")
      }]
    ], function() {
      visitor.copies.forEach(function(c) { c.windowLinger = 5000 })
      multi.replaceConfig(saved)
      root.done()
    })
  }

  function peekOnlyWhereShown() {
    root.withTwoNotches(function() {
      var side = multi.notchById("side")
      root.check(!!side && side.copies.length === 1 && !side.everyScreen, "side should be one copy that moves")
      root.push("test", true)
      root.steps([[100, function() {}]], function() {
        root.check(side.copies[0].mode === "rest", "a notch without the module shows " + side.copies[0].mode)
        root.check(side.edge === "left", "side hangs from " + side.edge)
        multi.dismissPeek(null)
        root.done()
      })
    })
  }

  // The notches, their copies and the modules are made once: a config edit
  // (a slider moving) must not make them again, losing what they show.
  function survivesConfigEdits() {
    root.withTwoNotches(function() {
      var main = multi.notchById("main")
      var copy = main.copyOn("A")
      var module = multi.moduleById("activities")
      root.push("edit", false)
      copy.open()
      multi.setConfigValue("hoverOpenDelay", 50)
      multi.setConfigValue("notches.0.position.edge", "bottom")
      root.steps([[50, function() {}]], function() {
        root.check(multi.notchById("main") === main, "the notch was made again")
        root.check(main.copyOn("A") === copy, "the copy was made again")
        root.check(copy.pinned, "the copy forgot it was pinned")
        root.check(multi.moduleById("activities") === module, "the module was loaded again")
        root.check(module.items.length > 0, "the module lost its activities")
        root.check(main.edge === "bottom", "the edit did not apply: " + main.edge)
        copy.close()
        module.clear("")
        root.done()
      })
    })
  }

  function addAndRemoveNotch() {
    var start = { modules: ["activities"], hoverOpenDelay: 40 }
    multi.replaceConfig(start)
    root.check(multi.notches.length === 1, "one notch to start, not " + multi.notches.length)
    var at = multi.addNotch()
    root.check(at === 1 && multi.notches.length === 2, "added at " + at + ", " + multi.notches.length + " notches")
    root.check(JSON.stringify(multi.config.notches[0]) === "{}", "the first notch should inherit it all: " + JSON.stringify(multi.config.notches))
    root.check(multi.notches[1].edge !== multi.notches[0].edge, "the new notch hangs on the same edge")
    root.check(multi.notches[1].modules.length === 0, "the new notch starts with modules")
    multi.removeNotch(1)
    root.check(JSON.stringify(multi.config) === JSON.stringify(start), "the config came back as " + JSON.stringify(multi.config))
    root.done()
  }

  // Bug: the notch kept on the ultrawide (position.screen) showing music
  // left it when the volume changed on the laptop screen. Now it stays and
  // shows the volume there; on the laptop screen, where no notch with the
  // OSD hangs, the visitor shows it once at the top, however many notches
  // kept elsewhere have the module.
  function peekBesideHomedNotch() {
    var music = null, side = null
    var visitor = multi.visitor
    root.steps([
      [0, function() {
        multi.replaceConfig({ modules: ["activities"], position: { screen: "A" },
                              notches: [{}, { id: "side", position: { edge: "left", screen: "A" } }] })
        twoScreens.focusedScreenName = "B"
      }],
      [100, function() {
        music = multi.notchById("main"); side = multi.notchById("side")
        root.check(music.copies.length === 1 && music.copies[0].screenName === "A", "the notch kept on A is not one copy on A")
        root.check(visitor.copies.length === 2 && visitor.copyOn("B").offWorkspace, "the visitor shows while nothing peeks")
        root.push("music", false)
        root.push("volume", true)
      }],
      [100, function() {
        root.check(music.copies[0].mode === "peek" && music.copies[0].screenName === "A",
                   "the notch on A shows " + music.copies[0].mode + " on " + music.copies[0].screenName)
        root.check(side.copies[0].mode === "peek" && side.copies[0].screenName === "A", "the side notch left A")
        var guest = visitor.copyOn("B")
        root.check(guest.mode === "peek" && !guest.offWorkspace && guest.edge === "top",
                   "no peek at the top of B: " + guest.mode + " on " + guest.edge)
        root.check(visitor.copyOn("A").mode !== "peek", "the visitor also peeked on A, where the notches are")
        root.check(JSON.stringify(guest.reservedScreens) === "[]", "the visitor reserves " + JSON.stringify(guest.reservedScreens))
        multi.dismissPeek(null)
      }],
      [100, function() {
        root.check(music.copies[0].mode === "compact", "after the peek the notch on A shows " + music.copies[0].mode)
        root.check(visitor.copyOn("B").offWorkspace, "the visitor stays after the peek")
        twoScreens.focusedScreenName = "A"
        root.push("again", true)
      }],
      [100, function() {
        root.check(visitor.copyOn("A").mode !== "peek" && visitor.copyOn("B").mode !== "peek",
                   "the visitor peeked with focus where the notches are")
        multi.dismissPeek(null)
        // A notch that follows focus peeks on B itself: no visitor.
        multi.setConfigValue("notches.1.position.screen", "")
        twoScreens.focusedScreenName = "B"
        root.push("follow", true)
      }],
      [100, function() {
        root.check(visitor.copyOn("B").mode !== "peek", "the visitor peeked beside a notch that follows focus")
        multi.dismissPeek(null)
        multi.moduleById("activities").clear("")
      }]
    ], function() { root.done() })
  }

  // Bug: brightness changes on the focused monitor only, yet its OSD opened
  // in the notches of the other monitors too. A module with
  // peekOnFocusedScreen (the OSD) peeks once: in the first notch on the
  // focused screen that shows it, else at the top, in the visitor.
  function osdPeekOnFocusedScreen() {
    var module = multi.moduleById("activities")
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
    root.steps([
      [0, function() {
        module.peekOnFocusedScreen = true
        multi.replaceConfig({ modules: ["activities"],
                              notches: [{ position: { screen: "A" } }, { id: "a2", position: { edge: "left", screen: "A" } },
                                        { id: "b", position: { screen: "B" } }] })
        twoScreens.focusedScreenName = "B"
      }],
      [100, function() { root.push("brightness", true) }],
      [100, function() {
        root.check(peeking() === "b", "focus on B: " + modes())
        multi.dismissPeek(null)
        twoScreens.focusedScreenName = "A"
        root.push("volume", true)
      }],
      [100, function() {
        root.check(peeking() === "main", "focus on A: " + modes())
        multi.dismissPeek(null)
        multi.setConfigValue("notches.2.position.screen", "A")
        twoScreens.focusedScreenName = "B"
        root.push("volume", true)
      }],
      [100, function() {
        root.check(peeking() === "visitor@B", "nothing on B: " + modes())
        multi.dismissPeek(null)
        module.peekOnFocusedScreen = false
        module.clear("")
      }]
    ], function() { root.done() })
  }

  // Standalone modules (the OSD) are never in a notch, even one whose
  // `modules` list them: they load on their own (unless switched off) and
  // peek in a notch of their own, on the focused screen, where
  // "<id>.position" says.
  NotchService {
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
    // Bug: in the Omarchy shell the OSD did not load at startup; the module
    // list was rebuilt before the standalone ids followed the module files.
    root.check(!!osd() && !!own(), "the OSD did not load at startup")
    root.steps([
      [0, function() {
        svc.replaceConfig({ modules: ["osd", "activities"], position: { screen: "all" } })
        twoScreens.focusedScreenName = "B"
      }],
      [100, function() {
        root.check(!!osd(), "the OSD did not load")
        root.check(svc.isStandalone("osd"), "the OSD is not standalone")
        root.check(svc.notches[0].modules.indexOf(osd()) === -1, "the notch took the OSD")
        root.check(!!own() && own().edge === "top" && own().placement.offset === 0.5, "no OSD place at the top center")
        osd().show(JSON.stringify({ icon: "volume", value: "40" }))
      }],
      [100, function() {
        root.check(own().copyOn("B").mode === "peek", "no peek on B: " + own().copyOn("B").mode)
        root.check(own().copyOn("A").mode !== "peek", "the OSD peeked on A too")
        root.check(svc.notches[0].copies.every(function(c) { return c.mode !== "peek" }), "a notch peeked")
        svc.dismissPeek(null)
        svc.setConfigValue("osd.position", { edge: "bottom", offset: 0.2 })
        twoScreens.focusedScreenName = "A"
      }],
      [100, function() {
        root.check(own().edge === "bottom" && own().placement.offset === 0.2, "position: " + JSON.stringify(own().placement))
        osd().show(JSON.stringify({ icon: "brightness", value: "70" }))
      }],
      [100, function() {
        root.check(own().copyOn("A").mode === "peek", "no peek on A: " + own().copyOn("A").mode)
        svc.dismissPeek(null)
        svc.setConfigValue("osd.enabled", false)
      }],
      [100, function() {
        root.check(osd() === null && own() === null, "switched off, the OSD still loads")
        root.check(svc.moduleCatalog.some(function(m) { return m.moduleId === "osd" && m.standalone }), "the OSD left the catalog")
        root.check(svc.settingsSchema.some(function(f) { return f.key === "osd.enabled" && f.page === "modules/osd" }), "no switch on its page")
        var lists = svc.settingsSchema.filter(function(f) { return f.type === "moduleCatalog" }).map(function(f) { return f.part })
        root.check(JSON.stringify(lists) === JSON.stringify(["notch", "agent", "terminal", "standalone"]),
                   "the Modules page does not list agents and standalone modules apart: " + JSON.stringify(lists))
        root.check(svc.moduleCatalog.some(function(m) { return m.moduleId === "claude" && m.agent && !m.standalone }), "claude is no agent")
        root.check(svc.moduleSetting("codex", "command") === "codex", "codex's command default: " + svc.moduleSetting("codex", "command"))
        root.check(svc.agentCommand("media", "toggle").indexOf("no agent media") === 0, "media took an agent command")
        svc.setConfigValue("osd.enabled", undefined)
      }],
      [100, function() {
        root.check(!!osd() && !!own(), "switched back on, the OSD did not load")
        svc.replaceConfig({})
      }]
    ], function() { root.done() })
  }

  // A notch's keys were split between General and Modules, each with its own
  // picker. The Notches page lists the notches; every field for the picked
  // notch's keys sits on its own page, "notch/<index>"; the other pages only
  // have keys for every notch.
  function notchSettingsOnOnePage() {
    multi.replaceConfig({ notches: [{ modules: ["activities"] }, { id: "side", position: { edge: "bottom" } }] })
    multi.settingsPage = "notch/1"
    root.check(multi.settingsNotch === 1, "showing notch/1 edits notch " + multi.settingsNotch)
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
  }

  // A module's settings were read from the loaded module, so one no notch
  // showed had no page and no gear. They come from its JSON file now.
  function unloadedModuleSettings() {
    root.check(multi.moduleById("timer") === null, "timer is loaded (test setup)")
    var fields = multi.settingsSchema.filter(function(f) { return f.page === "modules/timer" })
    root.check(fields.some(function(f) { return f.key === "timer.focus" }), "timer fields: " + JSON.stringify(fields.map(function(f) { return f.key })))
    root.check(multi.settingsPages.some(function(p) { return p.id === "modules/timer" && p.label === "Timer" }), "no page for timer")
    root.check(multi.moduleSetting("timer", "focus") === 25, "default: " + multi.moduleSetting("timer", "focus"))
    multi.setConfigValue("timer.focus", 50)
    root.check(multi.moduleSetting("timer", "focus") === 50, "set: " + multi.moduleSetting("timer", "focus"))
    var media = multi.moduleCatalog.filter(function(m) { return m.moduleId === "media" })[0]
    root.check(!!media && media.title === "Now playing", "media: " + JSON.stringify(media))
    var activities = multi.moduleById("activities")
    root.check(!!activities && activities.title === "Activities", "loaded title: " + (activities ? activities.title : ""))
    multi.replaceConfig({})
    root.done()
  }

  NotchService {
    id: resourced
    ipcTarget: "notchd-qmltest-resources"
    configPath: root.dir + "/resources.json"
    compositor: twoScreens
    defaultModules: ["resources"]
  }

  // FileView on /proc (whose files report no size) and df's output both
  // reach the module: a CPU figure needs two samples, 2 s apart.
  function resourcesRead() {
    var m = resourced.moduleById("resources")
    root.check(!!m, "the module did not load")
    root.steps([[2600, function() {
      root.check(m.cpu !== null && m.cpu >= 0 && m.cpu <= 1, "cpu: " + m.cpu)
      root.check(m.cores > 0, "cores: " + m.cores)
      root.check(!!m.memory && m.memory.total > 0, "memory: " + JSON.stringify(m.memory))
      root.check(!!m.disk && m.disk.total > 0, "disk: " + JSON.stringify(m.disk))
      root.check(m.active && !!m.expanded, "no expanded view")
    }]], function() { root.done() })
  }

  // By default the closed notch always shows the three. With `compact` at
  // "short" it rests while every resource is fine; one running short peeks
  // once and takes the compact slot until it drops back. The setting is on
  // the module's own page, under the module's id.
  function resourcesCompactWhenShort() {
    var m = resourced.moduleById("resources")
    var notch = resourced.notches[0]
    m.sampling = false
    m.pressure = ({})
    resourced.replaceConfig({})
    m.record("memory", 0.5)
    root.check(notch.compactModule === m && m.compactRows.length === 3, "not shown by default")
    var field = resourced.settingsSchema.filter(function(f) { return f.key === "resources.compact" })[0]
    root.check(!!field && field.page === "modules/resources", "field: " + JSON.stringify(field))

    resourced.setConfigValue("resources.compact", "short")
    root.check(notch.compactModule === null, "the notch shows resources with nothing short")
    m.record("memory", 0.95)
    root.check(notch.compactModule === m && m.compactRows.length === 1, "memory short, no compact view")
    root.check(m.peekId === "memory" && resourced.peeking(m), "memory short, no peek")
    resourced.dismissPeek(null)
    m.record("memory", 0.96)
    root.check(!resourced.peeking(m), "peeked again while still short")
    m.record("memory", 0.8)
    root.check(notch.compactModule === null, "still compact after memory dropped")
    m.record("cpu", 0.99)
    root.check(notch.compactModule === null, "one CPU burst counted as short")
    resourced.replaceConfig({})
    m.sampling = true
    root.done()
  }

  NotchService {
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
    var m = timed.moduleById("timer")
    var notch = timed.notches[0]
    timed.replaceConfig({ timer: { notify: false } })
    root.check(!!m && m.active && notch.compactModule === null, "idle timer is in the closed notch")
    root.check(m.command("2s Tea") === "ok", "start refused")
    root.check(notch.compactModule === m && m.status === "running", "running timer not in the closed notch")
    root.check(m.clock === "0:02", "clock: " + m.clock)
    timed.dismissPeek(null)
    root.steps([[2600, function() {
      root.check(m.status === "done", "not done after its time: " + m.status)
      root.check(timed.peeking(m), "no peek at the end")
      root.check(notch.compactModule === m && m.doneText === "Time's up", "done text: " + m.doneText)
      timed.dismissPeek(null)
      root.check(m.command("stop") === "ok", "stop refused")
      root.check(notch.compactModule === null, "stopped timer still in the closed notch")
    }]], function() { timed.replaceConfig({}); root.done() })
  }

  function timerCommands() {
    var m = timed.moduleById("timer")
    timed.replaceConfig({ timer: { notify: false, focus: 50 } })
    root.check(m.command("pause") === "no timer running", "pause while idle: " + m.command("pause"))
    root.check(m.command("soon").indexOf("usage") === 0, "a bad command was accepted")
    m.command("pomodoro")
    root.check(m.countdown.phase === "focus" && m.countdown.duration === 50 * 60000, "focus setting ignored: " + JSON.stringify(m.countdown))
    m.command("toggle")
    root.check(m.status === "paused", "toggle did not pause")
    var left = m.remaining
    m.command("+5m")
    root.check(m.remaining === left + 5 * 60000, "added time lost while paused")
    m.command("skip")
    root.check(m.countdown.phase === "short" && m.status === "running", "skip: " + JSON.stringify(m.countdown))
    var keys = timed.settingsSchema.filter(function(f) { return f.page === "modules/timer" }).map(function(f) { return f.key })
    root.check(keys.indexOf("timer.focus") !== -1 && keys.indexOf("timer.notify") !== -1, "settings: " + keys)
    m.command("stop")
    timed.dismissPeek(null)
    timed.replaceConfig({})
    root.done()
  }

  // ---------------------------------------------------------- navigation
  // Navigation (IPC `navigate`, Navigate.js): every notch of the config
  // numbered, the selected one's copy on the focused screen open as a
  // preview, the keys driving it through navKey().
  function navigationPreview() {
    var main = null, side = null
    root.steps([
      [0, function() { twoScreens.focusedScreenName = "B"; multi.replaceConfig(root.twoNotches) }],
      [300, function() {
        main = multi.notchById("main")
        side = multi.notchById("side")
        if (!root.check(!!main && !!side, "the two notches did not load")) return
        root.check(multi.navigate() === "ok" && multi.navigating, "navigate did not start navigation")
        root.check(main.copyOn("B").preview && main.copyOn("B").mode === "expanded",
                   "the first notch on the focused screen does not preview: " + main.copyOn("B").mode)
        root.check(!main.copyOn("B").pinned, "the preview pinned the notch")
        root.check(main.copyOn("A").mode !== "expanded", "the copy on the other screen opened too")
        root.check(main.copyOn("A").navShown && main.copyOn("A").navNumber === 1, "main is not numbered 1")
        root.check(side.copies[0].navShown && side.copies[0].navNumber === 2, "side is not numbered 2")
        root.check(!multi.visitor.copies.some(function(c) { return c.navShown }), "the visitor takes part in navigation")
        multi.navKey("next")
        root.check(multi.navSelected === 1 && side.copies[0].mode === "expanded", "next did not move to the second notch")
        root.check(main.copyOn("B").mode !== "expanded", "the first notch stayed open after moving on")
        multi.navKey("next")
        root.check(multi.navSelected === 0, "next did not wrap around")
        multi.navKey("2")
        root.check(multi.navSelected === 1, "2 did not select the second notch")
        multi.navKey("leave")
        root.check(!multi.navigating, "Escape did not end navigation")
        root.check(!side.copies[0].isOpen && !main.copyOn("B").isOpen, "Escape left a notch open")
        root.check(!side.copies[0].navShown, "the highlight stayed after Escape")
        multi.navigate()
        multi.navigate()
        root.check(!multi.navigating, "navigate again did not end navigation")
      }]
    ], function() { root.done() })
  }

  function navigationOpen() {
    root.withTwoNotches(function() {
      var main = multi.notchById("main")
      var side = multi.notchById("side")
      if (!root.check(!!main && !!side, "the two notches did not load")) { root.done(); return }
      main.open()
      root.check(main.copyOn("B").pinned, "main did not open (test setup)")
      multi.navigate()
      root.check(multi.navSelected === 0, "navigation did not start on the open notch")
      multi.navKey("next")
      root.check(main.copyOn("B").mode !== "expanded", "the pinned notch stayed open with another one selected")
      multi.navKey("previous")
      multi.navKey("leave")
      root.check(main.copyOn("B").pinned && main.copyOn("B").mode === "expanded", "Escape did not open the pinned notch again")
      multi.navigate()
      multi.navKey("next")
      multi.navKey("open")
      root.check(!multi.navigating, "Enter did not end navigation")
      root.check(side.copies[0].pinned, "Enter did not pin the selected notch")
      root.check(!main.copyOn("B").pinned, "Enter left the other notch open")
      side.close()
      multi.navigate()
      main.copyOn("B").togglePinned("")
      root.check(!multi.navigating, "a click that pinned a notch left navigation on")
      main.close()
      root.done()
    })
  }

  function navigationTabs() {
    var copy = tabsSvc.notches.length > 0 ? tabsSvc.notches[0].copies[0] : null
    if (!root.check(!!copy && copy.tabModules.length === 2, "the notch with two tabs did not load")) { root.done(); return }
    var first = copy.shownModules[0].moduleId
    tabsSvc.navigate()
    tabsSvc.navKey("nextTab")
    root.check(copy.shownModules[0].moduleId !== first, "Tab did not show the next tab")
    tabsSvc.navKey("previousTab")
    root.check(copy.shownModules[0].moduleId === first, "Shift+Tab did not go back")
    tabsSvc.navKey("nextTab")
    tabsSvc.navKey("leave")
    root.check(copy.shownModules[0].moduleId === first, "Escape did not give the notch its tab back")
    tabsSvc.navigate()
    tabsSvc.navKey("nextTab")
    tabsSvc.navKey("open")
    root.check(copy.pinned && copy.shownModules[0].moduleId !== first, "Enter did not open on the tab picked")
    copy.close()
    copy.showTab("")
    root.done()
  }

  // Bug guarded: navigation takes the keyboard from an agent pinned open;
  // losing it that way closed the agent's notch, as a click away does.
  // Navigation's keys for the selected notch (Navigate.step): a removal
  // asked twice, its settings.
  function navigationRemove() {
    root.steps([
      [0, function() {
        twoScreens.focusedScreenName = "B"
        multi.replaceConfig({ notches: root.twoNotches.notches.concat([{ id: "extra", modules: [], position: { edge: "bottom" } }]) })
      }],
      [300, function() {
        multi.navigate()
        multi.navKey("3")
        root.check(multi.notches.length === 3 && multi.navNotch && multi.navNotch.notchId === "extra",
                   "the third notch is not selected (test setup)")
        multi.navKey("remove")
        root.check(multi.navMode === "remove", "x did not ask first: " + multi.navMode)
        root.check(multi.navNotch.copies[0].navMode === "remove", "the label does not show the question")
        multi.navKey("leave")
        root.check(multi.navigating && multi.navMode === "select" && multi.notches.length === 3,
                   "Escape at the question did not keep the notch and stay in navigation")
        multi.navKey("remove")
        multi.navKey("remove")
      }],
      [300, function() {
        root.check(multi.notches.length === 2, "x twice left " + multi.notches.length + " notches")
        root.check(multi.navSelected === 1 && multi.navNotch && multi.navNotch.notchId === "side",
                   "after removing the last one, " + multi.navSelected + " is selected, not the one before")
        multi.navKey("settings")
        root.check(!multi.navigating, "s did not end navigation")
        root.check(multi.settingsOpen && multi.settingsNotch === 1, "s did not open the selected notch's settings")
        multi.closeSettings()
      }]
    ], function() { multi.replaceConfig(root.twoNotches); root.done() })
  }

  // Moving the selected notch (m): arrows for the edge, Shift along it, Tab
  // the screen, live; Enter keeps it, Escape or the shortcut puts it back.
  function navigationMove() {
    root.withTwoNotches(function() {
      var side = multi.notchById("side")
      if (!root.check(!!side, "the side notch did not load")) { root.done(); return }
      twoScreens.focusedScreenName = "B"
      multi.navigate()
      multi.navKey("2")
      multi.navKey("move")
      root.check(multi.navMode === "move", "m did not start moving: " + multi.navMode)
      multi.navKey("down")
      root.check(side.placement.edge === "bottom", "down put it on " + side.placement.edge)
      multi.navKey("slideOn")
      root.check(Math.abs(side.placement.offset - 0.55) < 1e-9, "Shift+→ slid it to " + side.placement.offset)
      multi.navKey("leave")
      root.check(multi.navigating && multi.navMode === "select", "Escape while moving left navigation")
      root.check(side.placement.edge === "left" && side.placement.offset === 0.5, "Escape did not put it back: "
                 + side.placement.edge + " " + side.placement.offset)
      root.check(JSON.stringify(multi.configValue("notches.1.position")) === JSON.stringify({ edge: "left" }),
                 "the config after Escape: " + JSON.stringify(multi.configValue("notches.1.position")))
      multi.navKey("move")
      multi.navKey("up")
      multi.navKey("open")
      root.check(multi.navigating && multi.navMode === "select", "Enter while moving left navigation")
      root.check(side.placement.edge === "top", "Enter did not keep it on top: " + side.placement.edge)
      multi.navKey("move")
      multi.navKey("nextTab")
      root.check(side.placement.screen === "A", "Tab took it to '" + side.placement.screen + "', not A")
      multi.navKey("leave")
      root.check(side.placement.screen === "", "Escape left it on " + side.placement.screen)
      multi.navKey("move")
      multi.navKey("next")
      multi.navigate()
      root.check(!multi.navigating && side.placement.edge === "top", "the shortcut while moving left it on " + side.placement.edge)
      multi.replaceConfig(root.twoNotches)
      root.done()
    })
  }

  // Bug: a notch pinned open on the screen without focus closed at once:
  // on demand, Hyprland gave its keyboard back to the focused screen. It
  // stays exclusive there, on demand once its screen has focus.
  function keyboardOnOtherScreen() {
    root.withTwoNotches(function() {
      var side = multi.notchById("side")
      if (!root.check(!!side, "the side notch did not load")) { root.done(); return }
      var copy = side.copies[0]
      multi.setConfigValue("notches.1.position.screen", "A")
      twoScreens.focusedScreenName = "B"
      root.steps([[200, function() {
        root.check(copy.screenName === "A", "the side notch is on " + copy.screenName + ", not A (test setup)")
        side.open()
        copy.windowKeyboard(true)
        root.check(copy.pinned && copy.keyboardFocus === "exclusive", "held on the screen without focus, it asks for " + copy.keyboardFocus)
        twoScreens.focusedScreenName = "A"
        root.check(copy.keyboardFocus === "ondemand", "its screen focused, it asks for " + copy.keyboardFocus)
      }]], function() {
        side.close()
        twoScreens.focusedScreenName = "B"
        multi.setConfigValue("notches.1.position.screen", undefined)
        root.done()
      })
    })
  }

  function navigationKeepsAgent() {
    var copy = typer.notches.length > 0 ? typer.notches[0].copies[0] : null
    if (!root.check(!!copy, "the typing notch did not load")) { root.done(); return }
    copy.open()
    copy.windowKeyboard(true)
    root.check(copy.keyboardFocus === "ondemand", "pinned, holding the keyboard, it asks for " + copy.keyboardFocus)
    typer.navigate()
    root.check(copy.keyboardFocus === "none", "while navigating, the notch asks for " + copy.keyboardFocus)
    root.check(copy.mode === "expanded", "selected, the open agent shows " + copy.mode)
    copy.windowKeyboard(false)
    root.check(copy.pinned, "losing the keyboard to navigation closed the notch")
    typer.navKey("leave")
    root.check(copy.pinned && copy.keyboardFocus === "exclusive", "after Escape it asks for " + copy.keyboardFocus)
    copy.close()
    // Enter: it takes the keyboard, then the window you were in takes it
    // back as navigation's window goes. It asks again, still open.
    typer.navigate()
    typer.navKey("open")
    copy.windowKeyboard(true)
    copy.windowKeyboard(false)
    root.check(copy.pinned, "losing the keyboard as navigation ended closed the agent")
    root.check(copy.keyboardFocus === "exclusive", "it did not ask for the keyboard again: " + copy.keyboardFocus)
    root.steps([[800, function() {}]], function() {
      // Later on, a window taking it closes the notch, as before.
      copy.windowKeyboard(true)
      copy.windowKeyboard(false)
      root.check(!copy.pinned, "settled, losing the keyboard to a window left the notch open")
      root.done()
    })
  }

  // ------------------------------------------------------------ keyboard
  // A notch window takes the keyboard (layer-shell Exclusive) only while
  // pinned open on a module you type into: hover alone must never take it
  // from the window you are typing in.
  NotchService {
    id: typer
    ipcTarget: "notchd-qmltest-typing"
    configPath: root.dir + "/typing.json"
    compositor: workspace4
    hostModules: ({ typing: Qt.resolvedUrl("fixtures/typing.qml") })
    defaultModules: ["typing"]
  }

  function keyboardOnlyWhilePinned() {
    root.steps([[100, function() {}]], function() {
      var module = typer.moduleById("typing")
      var copy = typer.notches.length > 0 ? typer.notches[0].copies[0] : null
      if (!root.check(!!module && !!copy, "the typing module or its notch did not load")) { root.done(); return }
      root.check(copy.keyboardFocus === "none", "closed, it takes the keyboard: " + copy.keyboardFocus)
      copy.hovered = true
      root.check(copy.mode === "expanded", "hovered, it is " + copy.mode)
      root.check(copy.keyboardFocus === "none", "hover alone took the keyboard")
      module.requestKeyboard()
      root.check(copy.pinned, "a press on the field did not pin the notch")
      root.check(copy.keyboardFocus === "exclusive", "pinned on the field, it takes " + copy.keyboardFocus)
      // Once its window has the keyboard, on demand: kept exclusive, no
      // shortcut could move focus to a window, only a click away.
      copy.windowKeyboard(true)
      root.check(copy.keyboardFocus === "ondemand", "holding the keyboard, it asks for " + copy.keyboardFocus)
      copy.windowKeyboard(false)
      root.check(!copy.pinned && copy.keyboardFocus === "none", "a window took the keyboard and the notch stayed open")
      copy.open()
      root.check(copy.keyboardFocus === "exclusive", "pinned again, it takes " + copy.keyboardFocus)
      copy.close()
      root.check(copy.keyboardFocus === "none", "closed again, it keeps the keyboard")
      root.done()
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
  // NotchWindow saves: Position.configValue(drop, notch.inheritedPosition).
  NotchService {
    id: dropSvc
    ipcTarget: "notchd-qmltest-drop"
    configPath: root.dir + "/drop.json"
    compositor: workspace4
    defaultModules: ["activities"]
  }

  function dropOnTopStays() {
    dropSvc.replaceConfig({ position: { edge: "left", offset: 0.9, screen: "DP-1" },
                            notches: [{}, { id: "side", modules: ["activities"] }] })
    var side = dropSvc.notchById("side")
    if (!root.check(!!side && side.edge === "left", "the second notch should start on the left")) { root.done(); return }
    var drop = { edge: "top", offset: 0.5, screen: "DP-1" }
    side.setValue("position", Position.configValue(drop, side.inheritedPosition))
    root.check(side.edge === "top", "dropped on the top edge, it went to " + side.edge)
    root.check(side.placement.offset === 0.5, "dropped in the middle, it is at " + side.placement.offset)
    var main = dropSvc.notchById("main")
    main.setValue("position", Position.configValue({ edge: "top", offset: 0.5, screen: "" }, main.inheritedPosition))
    root.check(main.edge === "top" && main.placement.screen === "", "the first notch went to " + JSON.stringify(main.placement))
    root.done()
  }

  // ---------------------------------------------------------------- agents
  // The claude and codex modules (TerminalModule) against fake terminal UIs
  // (tests/qml/fixtures/bin) that set the terminal's title as Claude Code and
  // Codex do. The terminal needs the qmltermwidget package; where it isn't
  // installed, the cases that need it note so in the log and pass.
  NotchService {
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
    NotchCard { notch: codexSvc.notches.length > 0 ? codexSvc.notches[0].copies[0] : null }
    TestCase { id: codexKeys; when: false }
  }
  NotchService {
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

  // The terminal is not there: the case only notes it.
  function noTerminal(m) {
    if (m.available) return false
    console.log("NOTE " + root.current + ": qmltermwidget is not installed; the terminal was not tested")
    root.done()
    return true
  }

  function codexMissing() {
    var m = codexMissingSvc.moduleById("codex")
    if (!root.check(!!m, "the codex module did not load")) { root.done(); return }
    m.available = false
    codexMissingSvc.open()
    root.steps([[500, function() {}]], function() {
      root.check(m.viewsOpen === 1, "the notch did not open on it: " + m.viewsOpen)
      root.check(m.term === null && m.loadError === "", "it tried to start a terminal")
      codexMissingSvc.close()
      root.done()
    })
  }
  NotchService {
    id: codexMissingSvc
    ipcTarget: "notchd-qmltest-codex-missing"
    configPath: root.dir + "/codex-missing.json"
    compositor: workspace4
    defaultModules: ["codex"]
  }
  NotchCard { notch: codexMissingSvc.notches.length > 0 ? codexMissingSvc.notches[0].copies[0] : null }

  // One agent from start to restart: its command line and folder, Escape
  // going to it, the dot and the title from its terminal's title, parked
  // while closed and still running, a peek when it finishes while closed,
  // the same terminal back on reopening, a new one (and the old one
  // stopped) on restart, and noticing when it quits.
  function agentRuns(svc, keys, id, expected) {
    var m = svc.moduleById(id)
    if (!root.check(!!m, "the " + id + " module did not load")) { root.done(); return }
    if (root.noTerminal(m)) return
    var cfg = {}
    cfg[id] = { directory: root.dir }
    svc.replaceConfig(cfg)
    var copy = svc.notches[0].copies[0]
    var argsFile = root.dir + "/" + id + "-args"
    var first = null, pid = ""
    root.activate(keys)
    svc.open()
    root.steps([
      [900, function() {
        first = m.term
        if (!root.check(!!first, "opening the notch started no terminal")) return
        root.check(!m.working && m.sessionTitle === "", "after starting: working " + m.working + ", title " + m.sessionTitle)
        root.check(first.activeFocus, "the terminal does not have the focus")
        // Escape goes to the agent: the notch stays open.
        keys.keyClick(Qt.Key_Escape)
        // The Escape is on the fake's line: end that line first.
        first.write("\rwork\r")
        root.run(["cat", argsFile])
      }],
      [200, function() {
        var args = shellCmd.out.split("\n")
        root.check(args[0] === "", id + " got options: " + args[0])
        root.check(args[1] === root.dir, id + " started in " + args[1])
        pid = args[2]
        root.check(copy.pinned, "Escape closed the notch")
        root.check(m.working, "the spinner did not make it working")
      }],
      [900, function() {
        root.check(!m.working && m.sessionTitle === expected, "after work: working " + m.working + ", title " + m.sessionTitle)
        // Ctrl+Escape is the way out: the notch closes, the agent goes on.
        keys.keyClick(Qt.Key_Escape, Qt.ControlModifier)
        root.check(!copy.pinned, "Ctrl+Escape did not close the notch")
      }],
      [600, function() {
        root.check(m.viewsOpen === 0 && m.term === first && first.parent === m, "closing did not park the terminal")
        root.check(!m.exited, "closing stopped it")
        m.typed = true
        first.write("work\r")
      }],
      [1100, function() {
        root.check(copy.peekModule === m, "the end of work while closed did not peek")
        root.check(m.attention, "done while closed, it has no news for you")
        svc.open()
      }],
      [500, function() {
        root.check(m.term === first && first.parent !== m && first.visible, "reopened, it did not show the same terminal")
        root.check(!m.attention, "seen, it still has news")
        m.restart()
      }],
      [900, function() {
        root.check(m.term !== first && !!m.term && !m.exited, "restart did not start a new terminal")
        root.run(["cat", argsFile])
      }],
      [300, function() {
        root.check(shellCmd.out.split("\n")[2] !== pid, "the new terminal runs the old " + id)
        root.run(["kill", "-0", pid])
      }],
      [300, function() {
        root.check(shellCmd.code !== 0, "the old " + id + " (" + pid + ") still runs after restart")
        m.term.write("quit\r")
      }],
      [500, function() {
        root.check(m.exited && !m.working, id + " quit and the module did not notice")
        svc.close()
      }]
    ], function() {
      m.stop()
      root.done()
    })
  }
  function codexRuns() { root.agentRuns(codexSvc, codexKeys, "codex", "Fake thread") }
  function claudeRuns() { root.agentRuns(claudeSvc, claudeKeys, "claude", "Fake conversation") }

  // Over IPC: ask opens the notch with the keyboard and types the question
  // in once the agent is up; new starts another; anything else, the usage.
  function claudeAsk() {
    var m = claudeSvc.moduleById("claude")
    if (root.noTerminal(m)) return
    var copy = claudeSvc.notches[0].copies[0]
    var pid = ""
    root.steps([
      // The last case's view is still closing.
      [600, function() {
        root.check(m.command("nonsense").indexOf("usage") === 0, "an unknown command did not answer the usage")
        root.check(m.command("ask why is the sky blue?") === "ok", "ask failed")
        root.check(copy.pinned && copy.keyboardFocus === "exclusive", "ask did not open the notch with the keyboard")
      }],
      [1500, function() { root.run(["cat", root.dir + "/claude-args"]) }],
      [300, function() {
        var lines = shellCmd.out.split("\n")
        root.check(lines.indexOf("got why is the sky blue?") !== -1, "the question did not reach claude: " + JSON.stringify(lines))
        pid = lines[2]
        root.check(m.command("new") === "ok", "new failed")
      }],
      [900, function() { root.run(["cat", root.dir + "/claude-args"]) }],
      [300, function() {
        root.check(shellCmd.out.split("\n")[2] !== pid, "new did not start another claude")
        m.closeNotch()
      }]
    ], function() {
      m.stop()
      root.done()
    })
  }

  // Bug: a shortcut named the notch (toggleNotch claude), and renaming the
  // notch in settings broke it without a word. `agent <id> toggle` finds
  // the notch by the module it shows.
  function agentToggle() {
    var m = claudeSvc.moduleById("claude")
    var copy = claudeSvc.notches[0].copies[0]
    root.steps([
      // The last case's view is still closing.
      [600, function() {
        root.check(claudeSvc.notches[0].notchId !== "claude", "the test notch is called claude: the case proves nothing")
        root.check(claudeSvc.agentCommand("claude", "toggle") === "ok", "toggle failed")
        root.check(copy.pinned, "toggle did not open the notch")
      }],
      [400, function() {
        root.check(m.viewsOpen > 0, "open, no view shows claude")
        root.check(claudeSvc.agentCommand("claude", "toggle") === "ok", "toggle again failed")
        root.check(!copy.pinned, "toggle did not close the notch")
        root.check(claudeSvc.agentCommand("nope", "toggle").indexOf("no agent nope") === 0, "an unknown agent did not say so")
      }]
    ], function() {
      m.stop()
      root.done()
    })
  }

  // The plan's limits (AgentUsage), fed answers by hand: offscreen, nothing
  // asks the network or reads a real login (NOTCHD_OFFLINE).
  function agentUsage() {
    var m = claudeSvc.moduleById("claude")
    var copy = claudeSvc.notches[0].copies[0]
    // `day`: the session window (its reset), so a new window starts over.
    var answer = function(session, day) {
      return { limits: [
        { kind: "session", group: "session", percent: session, resets_at: "2099-01-0" + (day || 2) + "T05:00:00Z" },
        { kind: "weekly_all", group: "weekly", percent: 20, resets_at: "2099-01-07T00:00:00Z" } ] }
    }
    root.check(!m.usage.live, "offscreen, the usage asks the network")
    root.check(m.usageProvider === "claude" && m.usage.active, "claude has no usage: " + m.usageProvider)
    root.check(!m.showsUsage, "usage shown before any answer")
    root.check(m.usage.apply(answer(85, 1)), "the answer was refused")
    root.check(m.showsUsage && m.usage.limits.length === 2, "the limits do not show: " + JSON.stringify(m.usage.limits))
    root.check(copy.peekModule === null, "the first answer warned: a restart would warn again")
    root.check(m.usage.apply(answer(10)) && !m.usage.apply({ nope: 1 }), "an answer that isn't one was taken")
    m.usage.apply(answer(81))
    root.check(copy.peekModule === m && m.peekReason === "usage", "passing 80 % did not peek")
    root.check(m.usageAlert && m.usageAlert.level === 80 && m.usageAlert.label === "5h", "the warning: " + JSON.stringify(m.usageAlert))
    copy.dismissPeek()
    m.usage.apply(answer(90))
    root.check(copy.peekModule === null, "warned twice in one window")
    m.usage.apply(answer(100))
    root.check(copy.peekModule === m && m.usageAlert.level === 100, "the limit reached did not peek")
    copy.dismissPeek()
    claudeSvc.setConfigValue("claude.usage", false)
    root.check(!m.showsUsage, "switched off, the usage still shows")
    claudeSvc.setConfigValue("claude.usage", undefined)
    root.done()
  }

  // Bug (claude chat, kept for the terminal): while busy the mark cycled
  // through glyphs of different widths, so the closed notch kept resizing
  // (it springs to fit). Now the mark stays, and the working dot's place is
  // kept while it is off.
  function agentBusySteady() {
    var m = claudeSvc.moduleById("claude")
    claudeSvc.close()
    var widths = []
    root.steps([
      [600, function() { m.status = { working: false, title: "A title" } }],
      [500, function() { widths.push(claudeCard.width) }],
      [0, function() { m.status = { working: true, title: "A title" } }],
      [500, function() { widths.push(claudeCard.width) }],
      [130, function() { widths.push(claudeCard.width) }],
      [130, function() { widths.push(claudeCard.width) }]
    ], function() {
      root.check(claudeCard.mode === "compact", "the notch is " + claudeCard.mode)
      var min = Math.min.apply(null, widths), max = Math.max.apply(null, widths)
      root.check(max - min < 0.5, "the closed notch changed width going busy and while busy: " + JSON.stringify(widths))
      m.status = { working: false, title: "" }
      root.done()
    })
  }

  // A terminal of your own (`terminals`): its id is a module a notch lists,
  // with its title and the fields of a terminal; it runs its command in
  // your home, shows the title the program sets, and `terminal <id>`
  // reaches it, `agent <id>` doesn't.
  NotchService {
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
    var m = null
    var copy = null
    root.steps([
      [500, function() {
        m = ownTermSvc.moduleById("mine")
        if (!root.check(!!m, "the terminal from the config did not load")) return
        copy = ownTermSvc.notches[0].copies[0]
        root.check(m.title === "Mine" && m.appName === "Mine", "its title: " + m.title + ", " + m.appName)
        root.check(ownTermSvc.moduleCatalog.some(function(c) { return c.moduleId === "mine" && c.terminal && !c.agent }),
                   "the catalog has no terminal mine")
        root.check(ownTermSvc.settingsSchema.some(function(f) { return f.key === "terminals.mine.command" && f.page === "modules/mine" }),
                   "its page has no command field")
        root.check(ownTermSvc.settingsSchema.some(function(f) { return f.type === "moduleCatalog" && f.part === "terminal" }),
                   "the Modules page does not list terminals apart")
        root.check(m.defaultCommand.indexOf("printf") === 0, "it does not run its entry's command: " + m.defaultCommand)
        root.check(ownTermSvc.agentCommand("mine", "toggle").indexOf("no agent mine") === 0, "a terminal took an agent command")
        root.check(ownTermSvc.terminalCommand("nope", "toggle") === "no terminal nope; terminals: mine", "an unknown terminal did not say so")
        if (!m.available) return
        root.check(ownTermSvc.terminalCommand("mine", "toggle") === "ok", "toggle failed")
      }],
      [900, function() {
        if (!m || !m.available) return
        root.check(copy.pinned && !!m.term && !m.exited, "toggle did not open it running: pinned " + copy.pinned + ", term " + !!m.term + ", exited " + m.exited)
        root.check(m.sessionTitle === "Own title" && !m.working, "its title: " + m.sessionTitle + ", working " + m.working)
        root.run(["cat", pwdFile])
      }],
      [300, function() {
        if (!m || !m.available) return
        root.check(shellCmd.out.trim() === Quickshell.env("HOME"), "it started in " + shellCmd.out.trim())
        root.check(ownTermSvc.terminalCommand("mine", "toggle") === "ok" && !copy.pinned, "toggle did not close it")
      }]
    ], function() {
      if (m) {
        if (!m.available) console.log("NOTE " + root.current + ": qmltermwidget is not installed; the terminal was not run")
        m.stop()
      }
      root.done()
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
  NotchService {
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
  NotchService {
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
    var m = localLiveSvc.moduleById("local-live")
    localLiveSvc.replaceConfig({ "local-live": { agent: "claude", session: "new", directory: "~/talk", language: "en",
                                                 speechModel: "small", voiceModel: "pocket-pt", otherVoiceModel: "mine",
                                                 voice: "rafael", localaiUrl: "http://127.0.0.1:9" } })
    localLiveSvc.open()
    root.steps([
      [800, function() {
        root.check(!!m.bridge && m.live, "no conversation: " + JSON.stringify(m.conv))
        root.check(m.appName === "Claude", "app name: " + m.appName)
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
      [2000, function() { root.check(m.bridge === null, "closed, it went on") }]
    ], function() {
      localLiveSvc.replaceConfig({})
      root.done()
    })
  }

  // The optional components (core/ComponentStore.qml, settings/ComponentRow.qml)
  // against a fake installer (fixtures/bin/components): the state it reports
  // reaches the rows, their buttons run it, and a choice that needs a
  // component is locked until it is installed.
  NotchService {
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
  function componentRows() {
    var store = componentSvc.components
    root.check(store.catalog.length > 6, "the catalog was not read: " + store.catalog.length)
    root.check(componentSvc.settingsPages.some(function(p) { return p.id === "components" }), "no Components page")
    // not read yet: nothing locked, rows say so
    var before = componentSvc.settingsSchema.filter(function(f) { return f.key === "local-live.voiceModel" })[0]
    root.check(!!before && !before.options[1].disabled, "a voice locked before the state was read")
    var rows = []
    function row(id) {
      var item = componentRowComponent.createObject(componentHost, { field: { type: "component", component: id } })
      rows.push(item)
      return item
    }
    store.refresh()
    root.steps([
      [600, function() {
        root.check(store.loaded, "the state was not read")
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
        // the settings schema: rows filled in from the registry, a voice locked
        var schema = componentSvc.settingsSchema
        var field = schema.filter(function(f) { return f.component === "localai" && f.page === "modules/local-live" })[0]
        root.check(!!field && field.label === "LocalAI" && field.help.indexOf("For Voice") !== -1, "row not filled: " + JSON.stringify(field))
        var tts = schema.filter(function(f) { return f.key === "local-live.voiceModel" })[0]
        // the choices that wait on the chosen language's voice are locked by the window, not the service
        root.check(tts.options[0].disabled !== true && tts.options[1].disabled !== true && tts.options[2].disabled !== true,
                   "the service locked what depends on a language: " + JSON.stringify(tts.options))
        // the Voice page: the chosen language's voices only, its choices locked until they are installed
        var win = settingsWindowComponent.createObject(root)
        rows.push(win)
        function voiceRows() {
          return win.schema.filter(function(f) { return f.type === "component" && f.page === "modules/local-live" && f.component.indexOf("voice-") === 0 })
            .map(function(f) { return f.component })
        }
        function engineLocks() {
          var f = win.schema.filter(function(x) { return x.key === "local-live.voiceModel" })[0]
          return f.options.map(function(o) { return o.disabled === true })
        }
        root.check(JSON.stringify(voiceRows()) === JSON.stringify(["voice-piper-pt-br", "voice-pocket-pt"]), "pt voices: " + JSON.stringify(voiceRows()))
        root.check(JSON.stringify(engineLocks()) === "[false,true,true]", "pt locks: " + JSON.stringify(engineLocks()))
        componentSvc.setConfigValue("local-live.voiceLanguage", "en")
        root.check(JSON.stringify(voiceRows()) === JSON.stringify(["voice-piper-en", "voice-pocket-en"]), "en voices (a language edit rebuilds the page): " + JSON.stringify(voiceRows()))
        root.check(JSON.stringify(engineLocks()) === "[false,true,false]", "en locks (Pocket installed): " + JSON.stringify(engineLocks()))
        componentSvc.setConfigValue("local-live.voiceLanguage", undefined)
        root.check(JSON.stringify(voiceRows()) === JSON.stringify(["voice-piper-pt-br", "voice-pocket-pt"]), "back to pt: " + JSON.stringify(voiceRows()))
        root.check(schema.filter(function(f) { return f.page === "components" && f.type === "component" }).length === store.catalog.length, "the overview misses components")
        // Install runs the installer; the row follows its state
        base.act("install")
      }],
      [700, function() {
        var base = rows[3]
        root.check(root.componentLog_().indexOf("install voxtype") !== -1, "install not run: " + root.componentLog_())
        root.check(base.working && base.info.percent === 10, "no progress after Install: " + base.info.text)
        // Cancel
        base.act("cancel")
      }],
      [700, function() {
        var base = rows[3]
        root.check(root.componentLog_().indexOf("cancel voxtype") !== -1, "cancel not run: " + root.componentLog_())
        root.check(base.info.state === "missing", "still working after Cancel: " + base.info.text)
        // Remove asks twice
        var small = rows[0]
        small.act("remove")
        root.check(small.confirming && root.componentLog_().indexOf("remove localai") === -1, "removed on the first click")
        small.act("remove")
      }],
      [700, function() {
        var small = rows[0]
        root.check(root.componentLog_().indexOf("remove localai") !== -1, "remove not run on the second click")
        root.check(small.info.state === "missing" && !small.confirming, "still installed after Remove: " + small.info.text)
        rows.forEach(function(r) { r.destroy() })
      }]
    ], function() { root.done() })
  }

  function liveTalks() {
    var m = liveSvc.moduleById("codex-live")
    liveSvc.replaceConfig({ "codex-live": { voice: "maple", directory: "~/talk", access: "workspace-write" } })
    // A hover shows it and leaves the microphone off.
    root.liveCopy().hovered = true
    root.steps([
      [400, function() {
        root.check(root.liveCopy().mode === "expanded", "hovered, the notch didn't open")
        root.check(!m.open && m.bridge === null && !m.running, "a hover started a conversation")
        root.liveCopy().hovered = false
        liveSvc.open()
      }],
      [600, function() {
        root.check(m.open, "opened by a click, the module isn't open")
        root.check(!!m.bridge && m.live, "no conversation: " + JSON.stringify(m.conv))
        root.check(m.conv.transcript.length === 2 && m.conv.transcript[0].text === "Rodei os testes."
                   && m.conv.transcript[1].role === "assistant", "transcript: " + JSON.stringify(m.conv.transcript))
        root.check(m.speaking && m.statusText === "Speaking", "the voice's level didn't make it speak: " + m.statusText)
        root.check(m.working, "the task didn't show")
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
        root.check(!!m.bridge, "closing for a moment ended it")
        root.check(m.compact !== null, "talking with the notch closed, nothing shows closed")
      }],
      [1800, function() {
        root.check(m.bridge === null, "closed, the conversation went on")
        root.check(m.conv.status === "closed" && !m.working && !m.running, "closed state: " + JSON.stringify(m.conv))
        root.check(m.compact === null, "closed and not talking, the compact view stays")
        root.check(m.conv.transcript.length === 2, "the transcript went with it")
      }]
    ], function() {
      liveSvc.replaceConfig({})
      root.done()
    })
  }

  function liveStopAndCommands() {
    var m = liveSvc.moduleById("codex-live")
    root.check(/^not in a conversation/.test(liveSvc.liveCommand("codex-live", "mute")), "mute without a conversation")
    root.check(/^usage: live codex-live/.test(liveSvc.liveCommand("codex-live", "sing")), "no usage")
    root.check(/^no live module codex; live modules: codex-live/.test(liveSvc.liveCommand("codex", "start")), "an agent took live")
    root.check(liveSvc.liveCommand("codex-live", "start") === "ok", "start was refused")
    root.steps([
      [600, function() {
        root.check(root.liveCopy().pinned && m.live, "start didn't open the notch and talk")
        root.check(liveSvc.liveCommand("codex-live", "mute") === "ok", "mute was refused")
      }],
      [300, function() {
        root.check(m.conv.muted, "the bridge didn't say muted")
        root.check(liveSvc.liveCommand("codex-live", "stop") === "ok", "stop was refused")
      }],
      [600, function() {
        root.check(m.bridge === null && !m.running, "stop didn't end it")
        root.check(root.liveCopy().pinned, "stop closed the notch")
      }],
      [1800, function() {
        root.check(m.bridge === null, "stopped with the notch open, it started again")
        liveSvc.close()
      }],
      [400, function() { liveSvc.open() }],
      [600, function() {
        root.check(m.live, "opened again after a stop, it didn't talk")
        root.check(liveSvc.liveCommand("codex-live", "toggle") === "ok", "toggle was refused")
      }],
      [600, function() {
        root.check(m.bridge === null, "toggle didn't stop it")
        liveSvc.close()
      }],
      [1800, function() {}]
    ], root.done)
  }

  function liveMissingPackages() {
    var m = liveSvc.moduleById("codex-live")
    liveSvc.replaceConfig({ "codex-live": { command: "missing-codex" } })
    liveSvc.open()
    root.steps([
      [800, function() {
        root.check(m.bridge === null && !m.running, "it kept running: " + JSON.stringify(m.conv))
        root.check(/Install WebRTC and audio in Settings › Components\.$/.test(m.errorText), "error: " + m.errorText)
        root.check(m.errorText.indexOf("make ") === -1, "it still says make: " + m.errorText)
        root.check(m.statusText === "Stopped", "status: " + m.statusText)
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
    var m = liveSvc.moduleById("codex-live")
    liveSvc.open()
    var first = null
    root.steps([
      [600, function() {
        first = m.bridge
        root.check(!!first && m.live && m.conv.transcript.length > 0, "no conversation: " + JSON.stringify(m.conv))
        root.check(Live.micLevel(m.conv) > 0, "the microphone's level didn't come through: " + m.conv.mic)
        root.check(liveSvc.liveCommand("codex-live", "new") === "ok", "new was refused")
      }],
      [900, function() {
        root.check(!!m.bridge && m.bridge !== first, "no new bridge")
        root.check(m.live, "the new session isn't live: " + JSON.stringify(m.conv))
        // The fake says the same two lines again: the old ones are gone.
        root.check(m.conv.transcript.length === 2, "the transcript wasn't cleared: " + JSON.stringify(m.conv.transcript))
        liveSvc.close()
      }],
      [2000, function() { root.check(m.bridge === null, "closed, it went on") }]
    ], root.done)
  }

  function codexIdle() {
    var m = codexSvc.moduleById("codex")
    if (root.noTerminal(m)) return
    // 0.02 min: 1.2 s.
    codexSvc.replaceConfig({ codex: { directory: root.dir, idleMinutes: 0.02 } })
    root.steps([
      // The last case's view is still closing.
      [600, function() { codexSvc.open() }],
      [800, function() {
        root.check(!!m.term, "opening started no terminal")
        codexSvc.close()
      }],
      [700, function() { root.check(!!m.term, "it stopped before idleMinutes") }],
      [1200, function() {
        root.check(m.term === null, "closed and idle, it kept running")
        codexSvc.open()
      }],
      [600, function() {
        root.check(!!m.term && !m.exited, "opened again, no new terminal")
        codexSvc.close()
      }]
    ], function() {
      m.stop()
      codexSvc.replaceConfig({})
      root.done()
    })
  }

  // Bug: the messages you sent to Codex sat on an orange band. Codex can't
  // ask this terminal for its background (no OSC 11 answer), so it paints
  // them with ANSI 8, Breeze's orange. A terminal paints a band in ANSI 8;
  // the grab's pixel must be a gray.
  Item { id: bandHolder; parent: codexWin.contentItem; y: 500; width: 300; height: 60 }
  Canvas {
    id: bandCanvas
    parent: codexWin.contentItem
    width: 1; height: 1
    property var done: null
    onImageLoaded: requestPaint()
    onPaint: {
      if (!bandCanvas.done) return
      var ctx = getContext("2d")
      ctx.drawImage(bandCanvas.done.url, -40, -8)
      var px = ctx.getImageData(0, 0, 1, 1).data
      var cb = bandCanvas.done.cb
      bandCanvas.done = null
      cb([px[0], px[1], px[2]])
    }
  }
  function messageBandGray() {
    var m = codexSvc.moduleById("codex")
    if (root.noTerminal(m)) return
    var c = Qt.createComponent(Qt.resolvedUrl("core/terminal/TerminalView.qml"))
    var t = c.createObject(bandHolder, { program: "/bin/sh", width: 300, height: 60,
      programArguments: ["-c", "printf '\\033[48;5;8m%30s\\033[0m\\n' ''; sleep 5"] })
    var file = root.dir + "/band.png"
    var rgb = null
    root.steps([
      [700, function() {
        bandHolder.grabToImage(function(result) {
          result.saveToFile(file)
          bandCanvas.done = { url: "file://" + file, cb: function(px) { rgb = px } }
          bandCanvas.loadImage("file://" + file)
        })
      }],
      [500, function() {}]
    ], function() {
      if (root.check(rgb !== null, "could not read the band's color")) {
        var spread = Math.max(rgb[0], rgb[1], rgb[2]) - Math.min(rgb[0], rgb[1], rgb[2])
        root.check(spread < 16, "the band is not gray: rgb " + rgb.join(","))
      }
      t.destroy()
      root.done()
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
  NotchService {
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
    var copy = tallSvc.notches.length > 0 ? tallSvc.notches[0].copies[0] : null
    var typing = tallSvc.moduleById("typing")
    var tall = tallSvc.moduleById("tall")
    if (!root.check(!!copy && !!typing && !!tall, "the notch or its modules did not load")) { root.done(); return }
    tallSvc.setConfigValue("tabs", false)
    root.check(copy.shownModules.length === 2 && copy.tabModules.length === 0, "without tabs it shows "
               + copy.shownModules.length + " modules and " + copy.tabModules.length + " tabs")
    // Tabs are the default for a notch with several modules.
    tallSvc.setConfigValue("tabs", undefined)
    root.check(copy.tabModules.length === 2, "by default, " + copy.tabModules.length + " tabs")
    root.check(copy.shownModules.length === 1 && copy.shownModules[0] === typing, "with tabs it does not show the first module alone")
    tall.openNotch()
    root.check(copy.pinned, "opening for a module did not open the notch")
    root.check(copy.shownModules.length === 1 && copy.shownModules[0] === tall, "opening for a module did not pick its tab")
    root.check(copy.keyboardFocus === "exclusive", "pinned on another tab, it takes " + copy.keyboardFocus)
    copy.showTab("typing")
    root.check(copy.keyboardFocus === "exclusive", "on the typing module's tab, pinned, it takes " + copy.keyboardFocus)
    copy.close()
    tallSvc.setConfigValue("tabs", false)
    root.check(copy.tabModules.length === 0, "tabs off, " + copy.tabModules.length + " tabs")
    tallSvc.setConfigValue("tabs", undefined)
    root.done()
  }

  // Two modules, with tabs: the closed notch and missed news.
  NotchService {
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
    var copy = tabsSvc.notches.length > 0 ? tabsSvc.notches[0].copies[0] : null
    if (!root.check(!!copy, "the notch did not load")) { root.done(); return }
    tabsSvc.setConfigValue("tabs", false)
    root.check(copy.compactModules.length === 1, "without tabs the closed notch shows " + copy.compactModules.length + " modules")
    tabsSvc.setConfigValue("tabs", undefined)
    root.steps([[600, function() {
      root.check(copy.mode === "compact", "the notch is " + copy.mode)
      root.check(copy.compactModules.length === 2, "with tabs the closed notch shows " + copy.compactModules.length + " modules")
      // 80 and 60 px wide, a line and the gaps between them.
      root.check(tabsCard.width >= 80 + 60 + 2 * Theme.space(10), "a closed notch with both is " + tabsCard.width + " wide")
    }]], function() {
      tabsSvc.setConfigValue("tabs", undefined)
      root.done()
    })
  }

  // Bug: with tabs, an agent that finished on a tab you weren't looking at
  // asked for its peek while the notch was open, which takes no peeks, and
  // the news was lost (codex peeked, claude never did). A peek of news you
  // haven't seen (NotchModule.attention) now shows as the notch closes,
  // once: hovering and leaving again doesn't repeat it.
  function missedPeekOnClose() {
    var copy = tabsSvc.notches.length > 0 ? tabsSvc.notches[0].copies[0] : null
    var news = tabsSvc.moduleById("news")
    if (!root.check(!!copy && !!news, "the notch or its module did not load")) { root.done(); return }
    tabsSvc.setConfigValue("tabs", true)
    copy.open()
    news.attention = true
    news.requestPeek(3000)
    root.check(copy.peekModule === null, "an open notch peeked")
    copy.close()
    root.steps([
      [50, function() {
        root.check(copy.peekModule === news, "closing did not show the missed peek")
        copy.dismissPeek(news)
        copy.hovered = true
      }],
      [50, function() { copy.hovered = false }],
      [50, function() {
        root.check(copy.peekModule === null, "the missed peek showed again")
        // News you saw before the notch closed doesn't peek.
        copy.open()
        news.requestPeek(3000)
        news.attention = false
        copy.close()
      }],
      [50, function() { root.check(copy.peekModule === null, "news already seen peeked") }]
    ], function() {
      news.attention = false
      tabsSvc.setConfigValue("tabs", undefined)
      root.done()
    })
  }

  // A timer that isn't running has no compact view: the closed notch was
  // empty (a notch of its own rested; beside claude and codex it was
  // missing). Now an idle module shows its icon there (NotchModule.icon),
  // unless the notch's idleIcons is off.
  NotchService {
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
    var copy = idleSvc.notches.length > 0 ? idleSvc.notches[0].copies[0] : null
    var timer = idleSvc.moduleById("timer")
    if (!root.check(!!copy && !!timer, "the notch or the timer did not load")) { root.done(); return }
    root.check(timer.status === "idle" && !timer.compact, "the timer is not idle (test setup)")
    // The module list changing may load the modules again: ask each time.
    // Alone in its notch.
    idleSvc.setConfigValue("modules", ["timer"])
    root.steps([
      [500, function() {
        root.check(copy.mode === "compact" && copy.compactModules[0] === idleSvc.moduleById("timer"),
                   "an idle timer leaves the notch " + copy.mode + ", showing " + copy.compactModules.map(function(m) { return m.moduleId }))
        root.check(idleCard.height > idleCard.restDepth, "the closed notch shows nothing: " + idleCard.height + " deep")
        idleSvc.setConfigValue("idleIcons", false)
      }],
      [500, function() {
        root.check(copy.mode === "rest", "idleIcons off, an idle timer leaves the notch " + copy.mode)
        // Beside another module, with tabs (the default): both.
        idleSvc.setConfigValue("idleIcons", undefined)
        idleSvc.setConfigValue("modules", undefined)
      }],
      [500, function() {
        root.check(copy.compactModules.length === 2, "beside another module, the closed notch shows "
                   + copy.compactModules.length + " modules")
      }]
    ], function() { root.done() })
  }
}
