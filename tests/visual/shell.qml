import QtQuick
import Quickshell
import "core"
import "core/settings"
import "core/Position.js" as Position

// Deterministic offscreen scenarios, one PNG each: the settings window, and
// the notch on each screen edge with the real modules (media closed,
// activities closed, the volume OSD peeking, the notch open).
// scripts/visual-test.sh links the core next to this file, points
// NOTCHD_VISUAL_DIR at an empty scratch dir and runs it with Qt's offscreen
// backend. The config lives in that scratch dir, never in ~/.config.
ShellRoot {
  id: root

  readonly property string outDir: Quickshell.env("NOTCHD_VISUAL_DIR")

  readonly property var dark: ({ foreground: "#cacccc", background: "#101315", accent: "#7aa2f7" })
  readonly property var light: ({ foreground: "#2a2b30", background: "#f6f5f1", accent: "#3b6fd8" })

  // What the standalone host adds, so its pages have content.
  readonly property var standaloneFields: [
    { page: "general", section: "Layout", key: "topOffset", type: "integer", label: "Top offset",
      default: 0, min: 0, max: 120, step: 1, unit: "px", help: "Room at the top of the screen, e.g. your bar's height." },
    { page: "appearance", section: "Colors", key: "theme.foreground", type: "color", label: "Foreground", default: "#cacccc" },
    { page: "appearance", section: "Colors", key: "theme.accent", type: "color", label: "Accent",
      help: "Defaults to the foreground color." },
    { page: "appearance", section: "Text", key: "theme.fontFamily", type: "string", label: "Font", default: "monospace" }
  ]
  // What the Omarchy host adds.
  readonly property var omarchyFields: [
    { page: "appearance", section: "Theme", type: "note", label: "Follows your Omarchy theme",
      help: "Colors, fonts and spacing come from the active Omarchy theme and change with it." }
  ]

  // Each scenario: the page, the theme, `bar` for the Omarchy host's look
  // (workspaces 1 to 3 picked), its extra fields, config keys to set, the notch the window edits and whether
  // to scroll the page to its end (`scrollTo`: "end", or a number of pixels). Or, with `edge`, the notch on
  // that edge instead of the settings window.
  readonly property var scenarios: [
    { name: "notches-list-dark", page: "notch", theme: root.dark, bar: true, fields: root.omarchyFields,
      config: { notches: [{}, { id: "jobs", modules: ["activities", "timer"], position: { edge: "bottom" } }] } },
    { name: "notch-dark", page: "notch/0", theme: root.dark, bar: true, fields: root.omarchyFields },
    { name: "notch-light", page: "notch/0", theme: root.light, bar: true, fields: root.omarchyFields },
    { name: "notch-end-dark", page: "notch/0", theme: root.dark, bar: true, fields: root.omarchyFields, scrollTo: "end" },
    { name: "general-dark", page: "general", theme: root.dark, bar: true, fields: root.omarchyFields },
    { name: "modules-dark", page: "modules", theme: root.dark, bar: true, fields: root.omarchyFields,
      config: { notches: [{}, { id: "jobs", modules: ["activities", "timer"] }] } },
    { name: "module-resources-light", page: "modules/resources", theme: root.light, bar: true, fields: root.omarchyFields },
    { name: "modules-end-dark", page: "modules", theme: root.dark, bar: true, fields: root.omarchyFields, scrollTo: "end",
      config: { terminals: { btop: { title: "System", command: "btop" }, dev: { command: "cd ~/app && npm run dev" } },
                modules: ["media", "btop"] } },
    { name: "module-terminal-light", page: "modules/btop", theme: root.light, bar: true, fields: root.omarchyFields,
      config: { terminals: { btop: { title: "System", command: "btop" } }, btop: { width: 900 } } },
    { name: "appearance-omarchy-dark", page: "appearance", theme: root.dark, bar: true, fields: root.omarchyFields },
    { name: "notch-hanging-dark", page: "notch/0", theme: root.dark, bar: true, fields: root.omarchyFields,
      config: { position: { edge: "left", offset: 0.3 } }, scrollTo: "end" },
    { name: "notches-dark", page: "notch/1", theme: root.dark, bar: true, fields: root.omarchyFields,
      config: { notches: [{}, { id: "jobs", modules: ["activities"], position: { edge: "bottom", screen: "all" } }] },
      notch: 1 },
    { name: "notches-end-light", page: "notch/1", theme: root.light, bar: true, fields: root.omarchyFields,
      config: { notches: [{}, { id: "jobs", modules: ["activities"], position: { edge: "bottom" } }] },
      notch: 1, scrollTo: "end" },
    { name: "general-standalone-dark", page: "general", theme: root.dark, bar: false, fields: root.standaloneFields },
    { name: "appearance-standalone-light", page: "appearance", theme: root.light, bar: false, fields: root.standaloneFields },
    { name: "edge-top", edge: "top", theme: root.dark },
    { name: "edge-bottom", edge: "bottom", theme: root.dark },
    { name: "edge-left", edge: "left", theme: root.dark },
    { name: "edge-right", edge: "right", theme: root.light },
    { name: "claude-dark", chat: true, theme: root.dark },
    { name: "claude-light", chat: true, theme: root.light },
    { name: "codex-dark", codex: true, theme: root.dark },
    { name: "codex-light", codex: true, theme: root.light },
    // A spoken conversation, with the fake bridge's lines.
    { name: "live-dark", live: true, theme: root.dark },
    { name: "live-light", live: true, theme: root.light },
    { name: "module-live-dark", page: "modules/codex-live", theme: root.dark, bar: true, fields: root.omarchyFields },
    { name: "module-voice-speech-dark", page: "modules/local-live", theme: root.dark, bar: true, fields: root.omarchyFields, scrollTo: 1020 },
    { name: "module-voice-dark", page: "modules/local-live", theme: root.dark, bar: true, fields: root.omarchyFields, scrollTo: "end" },
    { name: "components-working-dark", page: "components", theme: root.dark, bar: true, fields: root.omarchyFields, scrollTo: 700 },
    { name: "components-light", page: "components", theme: root.light, bar: true, fields: root.omarchyFields },
    { name: "agents-dark", agents: true, theme: root.dark }
  ]
  property int index: -1
  readonly property var scenario: index >= 0 && index < scenarios.length ? scenarios[index] : null

  // A compositor with workspaces, so the workspaces field shows; the Omarchy
  // scenarios pick 1 to 3, as a config would.
  QtObject {
    id: fakeCompositor
    readonly property string focusedScreenName: ""
    function hasFullscreen(screen) { return false }
    readonly property bool supportsWorkspaces: true
    readonly property var knownWorkspaces: ["1", "2", "4", "code"]
    function activeWorkspace(screen) { return null }
  }

  NotchService {
    id: svc
    ipcTarget: "notchd-visual"
    compositor: fakeCompositor
    configPath: root.outDir + "/config.json"
    hostSettingsSchema: root.scenario && root.scenario.fields ? root.scenario.fields : []
    // The Omarchy host's standalone OSD, so the Modules page lists it apart;
    // switched off (next()), since osdSvc's owns the `osd` IPC target.
    hostModules: root.scenario && root.scenario.bar ? ({ osd: Qt.resolvedUrl("hosts/omarchy/modules/osd.qml") }) : ({})
    settingsPage: root.scenario && root.scenario.page ? root.scenario.page : "notch"
  }

  SettingsWindow { id: win; svc: svc }

  // ------------------------------------------------------------ notch
  // Real services with the real modules, the media one playing
  // demo-player.py's track: what each module shows on each edge.
  component DemoService: NotchService {
    property string tag: ""
    ipcTarget: "notchd-visual-" + tag
    configPath: root.outDir + "/edge-" + tag + ".json"
  }
  DemoService { id: mediaSvc; tag: "media"; defaultModules: ["media"] }
  DemoService { id: activitySvc; tag: "activity"; defaultModules: ["activities"] }
  // The OSD is standalone: no notch shows it, its own notch does.
  DemoService {
    id: osdSvc; tag: "osd"; defaultModules: []
    hostModules: ({ osd: Qt.resolvedUrl("hosts/omarchy/modules/osd.qml") })
  }
  function standaloneCopy(service, id) {
    var n = service.standaloneNotch(id)
    return n && n.copies.length > 0 ? n.copies[0] : null
  }
  DemoService { id: openSvc; tag: "open"; defaultModules: ["media", "activities"] }
  DemoService { id: resourcesSvc; tag: "resources"; defaultModules: ["resources"] }
  DemoService { id: resourcesOpenSvc; tag: "resources-open"; defaultModules: ["resources"] }
  DemoService { id: timerSvc; tag: "timer"; defaultModules: ["timer"] }
  DemoService { id: pomodoroSvc; tag: "pomodoro"; defaultModules: ["timer"] }
  DemoService { id: timerIdleSvc; tag: "timer-idle"; defaultModules: ["timer"] }
  DemoService { id: timerDoneSvc; tag: "timer-done"; defaultModules: ["timer"] }
  DemoService { id: restSvc; tag: "rest"; defaultModules: [] }
  // Claude Code and Codex in their terminals (the fake `claude` and `codex`),
  // and closed while they work.
  DemoService { id: claudeSvc; tag: "claude"; defaultModules: ["claude"] }
  DemoService { id: claudeBusySvc; tag: "claude-busy"; defaultModules: ["claude"] }
  DemoService { id: codexSvc; tag: "codex"; defaultModules: ["codex"] }
  DemoService { id: codexBusySvc; tag: "codex-busy"; defaultModules: ["codex"] }
  // Codex Live open on a conversation (the fake bridge), and closed while
  // it talks.
  DemoService { id: liveSvc; tag: "live"; defaultModules: ["codex-live"] }
  DemoService { id: liveClosedSvc; tag: "live-closed"; defaultModules: ["codex-live"] }
  // Both in one notch: with tabs (the default), and one above the other
  // (tabs off) on a screen too short for them, which scrolls.
  DemoService { id: agentsTabsSvc; tag: "agents-tabs"; defaultModules: ["claude", "codex"] }
  DemoService { id: agentsStackSvc; tag: "agents-stack"; defaultModules: ["claude", "codex"] }
  // Closed, with tabs: both side by side, claude working, codex done unseen.
  DemoService { id: agentsClosedSvc; tag: "agents-closed"; defaultModules: ["timer", "claude", "codex"] }
  DemoService { id: agentsSideSvc; tag: "agents-side"; defaultModules: ["timer", "claude", "codex"] }
  // Navigation (IPC `navigate`): two notches, numbered, the first selected
  // and open as a preview.
  DemoService {
    id: navSvc; tag: "navigate"; defaultModules: ["media"]
    function navNotch(i) { return navSvc.notches.length > i ? navSvc.notches[i].copies[0] : null }
  }
  readonly property var demoServices: [mediaSvc, activitySvc, osdSvc, openSvc, resourcesSvc, resourcesOpenSvc,
                                       timerSvc, pomodoroSvc, timerIdleSvc, timerDoneSvc, restSvc, navSvc]

  // Demo values instead of this machine's: memory short, the rest fine.
  function demoResources(m) {
    var gib = 1024 * 1024 * 1024
    m.sampling = false
    m.cpu = 0.34
    m.cores = 16
    m.memory = { used: 29.1 * gib, total: 31.2 * gib, fraction: 29.1 / 31.2 }
    m.disk = { used: 412 * gib, total: 931 * gib, fraction: 412 / 931 }
    m.pressure = { cpu: { hot: false, streak: 0 }, memory: { hot: true, streak: 1 }, disk: { hot: false, streak: 0 } }
  }

  // Once the modules are up: an activity on two of them, the volume OSD
  // held open, the last one pinned open.
  Timer {
    interval: 1500
    running: true
    onTriggered: {
      var deploy = JSON.stringify({ id: "deploy", icon: "󰜎", title: "Deploying api", subtitle: "staging · 3 of 5",
                                    progress: 64, sticky: true, peek: false })
      activitySvc.moduleById("activities").push(deploy)
      openSvc.moduleById("activities").push(deploy)
      osdSvc.moduleById("osd").show(JSON.stringify({ icon: "volume", value: "62", duration: "600000" }))
      openSvc.open()
      root.demoResources(resourcesSvc.moduleById("resources"))
      root.demoResources(resourcesOpenSvc.moduleById("resources"))
      resourcesOpenSvc.open()
      timerSvc.moduleById("timer").run({ action: "start", ms: 18 * 60000 + 42000, label: "Tea" })
      pomodoroSvc.moduleById("timer").countdown = { status: "running", kind: "pomodoro", phase: "focus", round: 2, rounds: 4,
                                                    label: "Focus", duration: 25 * 60000, endsAt: Date.now() + 18 * 60000 + 56000 }
      pomodoroSvc.open()
      timerIdleSvc.open()
      var done = timerDoneSvc.moduleById("timer")
      done.countdown = { status: "done", kind: "timer", label: "Tea", duration: 3 * 60000 }
      done.requestPeek(600000)
      claudeSvc.setConfigValue("claude.directory", root.outDir)
      claudeSvc.open()
      // The plan's limits, by hand: offscreen nothing asks the network.
      claudeSvc.moduleById("claude").usage.apply({ limits: [
        { kind: "session", group: "session", percent: 42, resets_at: new Date(Date.now() + 130 * 60000).toISOString() },
        { kind: "weekly_all", group: "weekly", percent: 83, resets_at: new Date(Date.now() + 3 * 86400000).toISOString() },
        { kind: "weekly_scoped", group: "weekly", percent: 5, resets_at: new Date(Date.now() + 3 * 86400000).toISOString(),
          scope: { model: { display_name: "Fable" } } } ] })
      claudeBusySvc.moduleById("claude").status = { working: true, title: "Convert HEIC photos to JPG" }
      codexSvc.setConfigValue("codex.directory", root.outDir)
      codexSvc.open()
      codexBusySvc.moduleById("codex").status = { working: true, title: "Fix the flaky login test" }
      liveSvc.open()
      // Talking, with no bridge: its state by hand.
      liveClosedSvc.moduleById("codex-live").conv = { status: "live", error: "", packages: [], warning: "",
        transcript: [], working: false, muted: false, level: -100 }
      var agents = [agentsTabsSvc, agentsStackSvc]
      for (var a = 0; a < agents.length; a++) {
        agents[a].setConfigValue("claude.directory", root.outDir)
        agents[a].setConfigValue("codex.directory", root.outDir)
      }
      agentsStackSvc.setConfigValue("tabs", false)
      agentsClosedSvc.moduleById("claude").status = { working: true, title: "Convert HEIC photos to JPG" }
      agentsClosedSvc.moduleById("codex").status = { working: false, title: "Fix the flaky login test" }
      agentsClosedSvc.moduleById("codex").attention = true
      agentsSideSvc.setConfigValue("position", { edge: "right", offset: 0.5 })
      agentsSideSvc.moduleById("claude").status = { working: true, title: "" }
      agentsSideSvc.moduleById("codex").attention = true
      agentsTabsSvc.open()
      agentsStackSvc.open()
      navSvc.setConfigValue("notches", [{ id: "Main", modules: ["media"] }, { id: "Computer", modules: ["resources"] }])
      navTimer.start()
    }
  }
  // Once navSvc's notches and modules are up.
  Timer {
    id: navTimer
    interval: 300
    onTriggered: {
      root.demoResources(navSvc.moduleById("resources"))
      navSvc.navigate()
    }
  }

  // A slice of a screen with the bar on top and a service's notch on it.
  component MockScreen: Rectangle {
    id: mock
    property var svc: null
    // The copy to draw: the service's first notch's, unless given.
    property var copy: mock.svc ? mock.svc.firstCopy : null
    property string caption: ""
    // Overrides the notch's own offset (NaN: keep it).
    property real offset: NaN
    readonly property int barHeight: Theme.space(22)
    width: Theme.space(500)
    height: Theme.space(330)
    color: Theme.mix(Theme.background, Theme.foreground, 0.12)
    border.width: 1
    border.color: Theme.line
    clip: true

    Rectangle { width: parent.width; height: mock.barHeight; color: Theme.background }
    Text {
      anchors.verticalCenter: parent.top
      anchors.verticalCenterOffset: mock.barHeight / 2
      x: Theme.space(10)
      text: mock.caption
      color: Theme.muted
      font.family: Theme.font.family
      font.pixelSize: Theme.font.caption
    }
    NotchCard {
      id: mockCard
      notch: mock.copy
      maxWidth: mock.width - Theme.space(24)
      maxHeight: mock.height - Theme.space(24)
      readonly property var spot: Position.place({
        edge: mockCard.edge,
        offset: !isNaN(mock.offset) ? mock.offset : mockCard.notch ? mockCard.notch.placement.offset : 0.5,
        width: mock.width, height: mock.height, cardWidth: mockCard.width, cardHeight: mockCard.height,
        insets: Position.insets("top", mock.barHeight, true), snap: mockCard.shoulder + Theme.space(4)
      })
      x: mockCard.spot.x
      y: mockCard.spot.y
      startSide: mockCard.spot.atStart ? "corner" : "shoulder"
      endSide: mockCard.spot.atEnd ? "corner" : "shoulder"
    }
  }

  // Three notches wanting nearly the same spot, packed as NotchSurface
  // does (Position.pack): joined flush into one shape as deep as its
  // deepest notch (Position.rowDepths), the empty one in the middle too.
  component JoinedScreen: Rectangle {
    id: joined
    property string caption: ""
    // The three notches' services, along the edge.
    property var services: [timerSvc, restSvc, resourcesSvc]
    readonly property int barHeight: Theme.space(22)
    width: Theme.space(500)
    height: Theme.space(330)
    color: Theme.mix(Theme.background, Theme.foreground, 0.12)
    border.width: 1
    border.color: Theme.line
    clip: true

    Rectangle { width: parent.width; height: joined.barHeight; color: Theme.background }
    Text {
      anchors.verticalCenter: parent.top
      anchors.verticalCenterOffset: joined.barHeight / 2
      x: Theme.space(10)
      text: joined.caption
      color: Theme.muted
      font.family: Theme.font.family
      font.pixelSize: Theme.font.caption
    }

    readonly property var cards: [first, second, third]
    readonly property var insets: Position.insets("top", joined.barHeight, true)
    readonly property var stretch: Position.stretch({ edge: first.edge, width: joined.width, height: joined.height,
                                                      insets: joined.insets })
    readonly property real snap: first.shoulder + Theme.space(4)
    function wish(i) { return (joined.stretch.start + joined.stretch.end) / 2 + (i - 1) * Theme.space(30) }
    readonly property var packing: Position.pack(joined.cards.map(function(c, i) {
      return { group: "demo", center: joined.wish(i), size: c.vertical ? c.height : c.width,
               start: joined.stretch.start, end: joined.stretch.end, snap: joined.snap,
               open: c.mode === "peek" || c.mode === "expanded" }
    }), 2 * Theme.space(10) + Theme.space(8))
    readonly property var rowDepths: Position.rowDepths(joined.packing, joined.cards.map(function(c) { return c.closedDepth }))
    function spotOf(i) {
      var c = joined.cards[i]
      var span = joined.stretch.end - joined.stretch.start
      return Position.place({ edge: c.edge, offset: (joined.wish(i) + joined.packing[i].shift - joined.stretch.start) / span,
                              width: joined.width, height: joined.height, cardWidth: c.width, cardHeight: c.height,
                              insets: joined.insets, snap: joined.snap })
    }
    function side(i, which) {
      var j = joined.packing[i][which === "start" ? "joinStart" : "joinEnd"]
      return j >= 0 ? "joined" : joined.spotOf(i)[which === "start" ? "atStart" : "atEnd"] ? "corner" : "shoulder"
    }
    function neighborDepth(i, which) {
      var j = joined.packing[i][which === "start" ? "joinStart" : "joinEnd"]
      return j >= 0 ? joined.cards[j].depth : 0
    }

    NotchCard {
      id: first
      notch: joined.services[0].firstCopy
      x: joined.spotOf(0).x; y: joined.spotOf(0).y
      startSide: joined.side(0, "start"); endSide: joined.side(0, "end")
      startNeighborDepth: joined.neighborDepth(0, "start"); endNeighborDepth: joined.neighborDepth(0, "end")
      minDepth: joined.rowDepths[0]
    }
    NotchCard {
      id: second
      notch: joined.services[1].firstCopy
      x: joined.spotOf(1).x; y: joined.spotOf(1).y
      startSide: joined.side(1, "start"); endSide: joined.side(1, "end")
      startNeighborDepth: joined.neighborDepth(1, "start"); endNeighborDepth: joined.neighborDepth(1, "end")
      minDepth: joined.rowDepths[1]
    }
    NotchCard {
      id: third
      notch: joined.services[2].firstCopy
      x: joined.spotOf(2).x; y: joined.spotOf(2).y
      startSide: joined.side(2, "start"); endSide: joined.side(2, "end")
      startNeighborDepth: joined.neighborDepth(2, "start"); endNeighborDepth: joined.neighborDepth(2, "end")
      minDepth: joined.rowDepths[2]
    }
  }

  // A notch in the top-left corner and one at the top of the left edge:
  // the top one keeps the corner and the side one starts where it ends
  // (Position.corners), whatever edge the scenario puts the others on.
  component CornerScreen: Rectangle {
    id: quina
    property string caption: ""
    readonly property int barHeight: Theme.space(22)
    width: Theme.space(500)
    height: Theme.space(330)
    color: Theme.mix(Theme.background, Theme.foreground, 0.12)
    border.width: 1
    border.color: Theme.line
    clip: true

    Rectangle { width: parent.width; height: quina.barHeight; color: Theme.background }
    Text {
      anchors.verticalCenter: parent.top
      anchors.verticalCenterOffset: quina.barHeight / 2
      x: quina.width - width - Theme.space(10)
      text: quina.caption
      color: Theme.muted
      font.family: Theme.font.family
      font.pixelSize: Theme.font.caption
    }

    readonly property var bar: Position.insets("top", quina.barHeight, true)
    readonly property real snap: topCard.shoulder + Theme.space(4)
    readonly property var topSpot: Position.place({ edge: "top", offset: 0, width: quina.width, height: quina.height,
      cardWidth: topCard.width, cardHeight: topCard.height, insets: quina.bar, snap: quina.snap })
    readonly property var inset: Position.corners([
      { screen: "s", edge: "top", atStart: quina.topSpot.atStart, atEnd: quina.topSpot.atEnd, depth: topCard.depth, shown: true },
      { screen: "s", edge: "left" }])[1]
    readonly property var sideSpot: Position.place({ edge: "left", offset: 0, width: quina.width, height: quina.height,
      cardWidth: sideCard.width, cardHeight: sideCard.height, snap: quina.snap,
      insets: { top: quina.bar.top + quina.inset.start, bottom: quina.bar.bottom + quina.inset.end, left: 0, right: 0 } })
    readonly property var span: Position.corners([
      { screen: "s", edge: "top" },
      { screen: "s", edge: "left", atStart: quina.sideSpot.atStart, atEnd: quina.sideSpot.atEnd, depth: sideCard.depth, shown: true }])[0]

    NotchCard {
      id: topCard
      notch: resourcesSvc.firstCopy
      edge: "top"
      x: quina.topSpot.x; y: quina.topSpot.y
      startSide: quina.topSpot.atStart ? "corner" : "shoulder"
      startNeighborDepth: quina.span.start
    }
    NotchCard {
      id: sideCard
      notch: timerSvc.firstCopy
      edge: "left"
      x: quina.sideSpot.x; y: quina.sideSpot.y
      startSide: quina.sideSpot.atStart ? "corner" : "shoulder"
    }
  }

  FloatingWindow {
    id: notchWin
    visible: !!root.scenario && !!root.scenario.edge
    implicitWidth: notchScene.width
    implicitHeight: notchScene.height
    color: Theme.background
    Grid {
      id: notchScene
      columns: 2
      padding: Theme.space(16)
      spacing: Theme.space(16)
      MockScreen { svc: mediaSvc; caption: "media, compact" }
      MockScreen { svc: activitySvc; caption: "activities, compact" }
      MockScreen { svc: osdSvc; copy: root.standaloneCopy(osdSvc, "osd"); caption: "osd, peek (standalone)" }
      MockScreen { svc: openSvc; caption: "expanded" }
      MockScreen { svc: resourcesSvc; caption: "resources, compact (memory short)" }
      MockScreen { svc: resourcesOpenSvc; caption: "resources, expanded" }
      MockScreen { svc: timerSvc; caption: "timer, compact" }
      MockScreen { svc: pomodoroSvc; caption: "timer, expanded (pomodoro)" }
      MockScreen { svc: timerIdleSvc; caption: "timer, expanded (idle)" }
      MockScreen { svc: timerDoneSvc; caption: "timer, peek (time's up)" }
      MockScreen { svc: mediaSvc; offset: 0; caption: "media, in the corner" }
      JoinedScreen { caption: "timer, an empty notch and resources, joined" }
      JoinedScreen { services: [timerSvc, timerDoneSvc, resourcesSvc]; caption: "the middle one peeking pushes its row aside" }
      CornerScreen { caption: "a corner shared by two edges" }
      MockScreen { svc: navSvc; copy: navSvc.navNotch(0); caption: "navigating, the notch selected" }
      MockScreen { svc: navSvc; copy: navSvc.navNotch(1); caption: "navigating, another notch" }
    }
  }

  FloatingWindow {
    id: chatWin
    visible: !!root.scenario && !!root.scenario.chat
    implicitWidth: chatScene.width
    implicitHeight: chatScene.height
    color: Theme.background
    Row {
      id: chatScene
      padding: Theme.space(16)
      spacing: Theme.space(16)
      MockScreen { svc: claudeSvc; width: Theme.space(880); height: Theme.space(580); caption: "claude, expanded" }
      MockScreen { svc: claudeBusySvc; width: Theme.space(360); height: Theme.space(120); caption: "claude, compact (working)" }
    }
  }

  FloatingWindow {
    id: codexWin
    visible: !!root.scenario && !!root.scenario.codex
    implicitWidth: codexScene.width
    implicitHeight: codexScene.height
    color: Theme.background
    Row {
      id: codexScene
      padding: Theme.space(16)
      spacing: Theme.space(16)
      MockScreen { svc: codexSvc; width: Theme.space(880); height: Theme.space(580); caption: "codex, expanded" }
      MockScreen { svc: codexBusySvc; width: Theme.space(360); height: Theme.space(120); caption: "codex, compact (working)" }
    }
  }

  FloatingWindow {
    id: liveWin
    visible: !!root.scenario && !!root.scenario.live
    implicitWidth: liveScene.width
    implicitHeight: liveScene.height
    color: Theme.background
    Row {
      id: liveScene
      padding: Theme.space(16)
      spacing: Theme.space(16)
      MockScreen { svc: liveSvc; width: Theme.space(560); height: Theme.space(600); caption: "codex live, expanded" }
      MockScreen { svc: liveClosedSvc; width: Theme.space(360); height: Theme.space(120); caption: "closed, listening" }
    }
  }

  FloatingWindow {
    id: agentsWin
    visible: !!root.scenario && !!root.scenario.agents
    implicitWidth: agentsScene.width
    implicitHeight: agentsScene.height
    color: Theme.background
    Grid {
      id: agentsScene
      columns: 2
      padding: Theme.space(16)
      spacing: Theme.space(16)
      MockScreen { svc: agentsTabsSvc; width: Theme.space(880); height: Theme.space(620); caption: "claude and codex, tabs" }
      MockScreen { svc: agentsStackSvc; width: Theme.space(880); height: Theme.space(620); caption: "claude and codex, too tall: scrolls" }
      MockScreen { svc: agentsClosedSvc; width: Theme.space(880); height: Theme.space(120); caption: "closed: timer idle, claude working, codex done unseen" }
      MockScreen { svc: agentsSideSvc; width: Theme.space(880); height: Theme.space(200); caption: "the same on the right edge" }
    }
  }

  // Set a scenario, give it a beat to lay out (and the notch to settle),
  // grab, then the next.
  function next() {
    root.index++
    if (!root.scenario) { quitTimer.start(); return }
    svc.components.refresh()  // the fake installer's canned state
    Theme.provider = root.scenario.theme
    svc.setConfigValue("workspaces", root.scenario.bar ? [1, 2, 3] : undefined)
    svc.setConfigValue("osd.enabled", false)
    var cfg = root.scenario.config || {}
    // Every key a scenario sets, so the next one starts without it. The
    // terminals first: `modules` may list one.
    svc.setConfigValue("terminals", cfg.terminals)
    svc.setConfigValue("btop", cfg.btop)
    svc.setConfigValue("claude", cfg.claude)
    svc.setConfigValue("position", cfg.position)
    svc.setConfigValue("notches", cfg.notches)
    svc.setConfigValue("modules", cfg.modules)
    svc.settingsNotch = root.scenario.notch || 0
    for (var i = 0; i < root.demoServices.length; i++)
      root.demoServices[i].setConfigValue("position", root.scenario.edge ? { edge: root.scenario.edge } : undefined)
    grabTimer.interval = root.scenario.edge || root.scenario.chat || root.scenario.codex || root.scenario.agents
      || root.scenario.live ? 1200 : 400
    grabTimer.start()
    scrollTimer.start()
  }

  Timer { interval: 600; running: true; onTriggered: root.next() }
  Timer {
    id: grabTimer
    interval: 400
    onTriggered: {
      var name = root.scenario.name
      var target = root.scenario.chat ? chatScene : root.scenario.codex ? codexScene : root.scenario.agents ? agentsScene
        : root.scenario.live ? liveScene
        : root.scenario.edge ? notchScene : win.contentItem.children[0]
      target.grabToImage(function(result) {
        if (!result.saveToFile(root.outDir + "/" + name + ".png")) console.warn("could not save " + name)
        root.next()
      })
    }
  }
  // After the page lays out, before the grab.
  Timer {
    id: scrollTimer
    interval: 200
    onTriggered: if (root.scenario) win.pageView.contentY = root.scenario.scrollTo === "end"
      ? Math.max(0, win.pageView.contentHeight - win.pageView.height)
      : (typeof root.scenario.scrollTo === "number" ? root.scenario.scrollTo : 0)
  }
  Timer { id: quitTimer; interval: 100; onTriggered: Qt.quit() }
}
