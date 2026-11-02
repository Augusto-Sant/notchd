import QtQuick
import Quickshell
import Quickshell.Io
import "settings"
import "client"
import "Config.js" as Config
import "Live.js" as Live
import "Components.js" as Components
import "settings/Settings.js" as Settings

// The notches' source of truth: config, modules, the notches (Notch, one per
// entry of the config's `notches`, each shown as one or more NotchCopy) and
// IPC. Every surface that shows a notch (NotchSurface, a host's bar
// indicator) reads it; modules see it as `notch`.
//
// It knows nothing about the desktop it runs in. A host instantiates it and
// sets the properties in the "host" block below.
Item {
  id: svc
  visible: false

  // ------------------------------------------------------------------ host
  // IPC target scripts call (`qs ipc call <target> push ...`).
  property string ipcTarget: "notchd"
  // The JSON config's path, as the daemon says it (its `welcome`): the
  // daemon reads, watches and writes it (below), so edits apply live. It
  // defaults to the user's config dir, so settings survive plugin updates and
  // never land in the source tree. A missing file means every key at its
  // default. Shown by the settings window; before the daemon answers, the
  // default one.
  readonly property string defaultConfigPath: {
    var base = Quickshell.env("XDG_CONFIG_HOME") || (Quickshell.env("HOME") + "/.config")
    return base + "/notchd/config.json"
  }
  property string configPath: svc.client.welcome && svc.client.welcome.config ? svc.client.welcome.config : svc.defaultConfigPath
  // The daemon this service is a client of (docs/hosts.md, "The daemon"): the
  // host gives the ensure script (a file URL) and its client name; the
  // service runs it, reconnects and brings the daemon back when it dies.
  // `daemonCommand` replaces what runs (tests: a daemon on a scratch socket).
  property url daemonScript
  property string daemonClientName: "quickshell"
  property var daemonCommand: null
  // How long without a daemon before the notch says so (`daemonDown`), so a
  // start does not flash it.
  property int daemonDownDelay: 1500
  // Extra modules the host ships, as { id: url }. They win over built-ins
  // with the same id.
  property var hostModules: ({})
  // The screen edge the host's bar is on ("top", "bottom", "left", "right"):
  // a notch on that edge hangs from the bar.
  property string barEdge: "top"
  // Prefix for log lines.
  property string logName: svc.ipcTarget
  // A compositor adapter (HyprlandCompositor, GenericCompositor, ...): the
  // focused screen, fullscreen windows and which workspace each screen
  // shows, for the `workspaces` setting.
  property var compositor: null
  // The screens a notch can be on (anything with a `name`): Quickshell's.
  // Tests give their own, to have more than one.
  property var screens: Quickshell.screens || []
  // Modules to load when the config doesn't list any.
  property var defaultModules: ["media", "activities"]
  // Extra settings-window fields for the host's own config keys, in the
  // same format as coreSettingsSchema below.
  property var hostSettingsSchema: []
  // Extra settings-window pages, as corePages below. A page with a core
  // page's id replaces its label or icon.
  property var hostSettingsPages: []

  // -------------------------------------------------------------- built-ins
  readonly property var builtinModules: ({
    claude: Qt.resolvedUrl("modules/claude.qml"),
    codex: Qt.resolvedUrl("modules/codex.qml"),
    "codex-live": Qt.resolvedUrl("modules/codex-live.qml"),
    "local-live": Qt.resolvedUrl("modules/local-live.qml")
  })
  // The built-ins that live in the daemon (native/notchd-modules): no QML
  // file, only the JSON next to where it would be, which gives the settings
  // page its title and fields. The daemon announces each (`module`) and
  // the service makes a CoreModule for it.
  readonly property var coreModules: ({
    activities: Qt.resolvedUrl("modules/activities.qml"),
    media: Qt.resolvedUrl("modules/media.qml"),
    resources: Qt.resolvedUrl("modules/resources.qml"),
    timer: Qt.resolvedUrl("modules/timer.qml")
  })

  // The QML file of a module this UI loads itself.
  function moduleUrl(id) {
    if (svc.hostModules && svc.hostModules[id]) return svc.hostModules[id]
    if (svc.builtinModules[id]) return svc.builtinModules[id]
    return svc.isTerminal(id) ? Qt.resolvedUrl("terminal/CustomTerminal.qml") : ""
  }
  // Where its JSON file is (a url of the `.qml` beside it), whether it has
  // a QML file or lives in the daemon.
  function infoUrl(id) {
    var u = svc.moduleUrl(id)
    return String(u) !== "" ? u : svc.coreModules[id] || ""
  }
  // What loads a module: its QML file, else a CoreModule when the daemon has
  // announced it (a built-in of its own, or a user module).
  function loadUrl(id) {
    var u = svc.moduleUrl(id)
    if (String(u) !== "") return u
    return svc.daemonModuleIds.indexOf(id) !== -1 ? Qt.resolvedUrl("CoreModule.qml") : ""
  }
  // Every module the daemon runs, as it announced them.
  readonly property var daemonModuleIds: Object.keys(svc.client.modules || {})

  // Every module that has a file: built-ins first, then the host's. Its
  // title and settings fields are in a JSON file next to its QML
  // (media.json), read whether it loads or not (moduleInfo).
  readonly property var fileModuleIds: {
    var out = Object.keys(svc.builtinModules).concat(Object.keys(svc.coreModules))
    var host = Object.keys(svc.hostModules || {})
    for (var i = 0; i < host.length; i++)
      if (out.indexOf(host[i]) === -1) out.push(host[i])
    return out
  }
  // Every module this host can load: those, then the config's terminals.
  readonly property var availableModuleIds: svc.fileModuleIds.concat(svc.terminalSpecs.map(function(t) { return t.id }))

  // Terminals of your own, from the config's `terminals` (Config.terminals):
  // [{ id, title, command }]. Each loads terminal/CustomTerminal.qml, its info made
  // from its entry (Settings.terminalInfo) instead of read from a file.
  property var terminalSpecs: []
  function isTerminal(id) {
    return svc.terminalSpecs.some(function(t) { return t.id === id })
  }
  function terminalCommandLine(id) {
    var t = svc.terminalSpecs.filter(function(s) { return s.id === id })[0]
    return t ? t.command : ""
  }
  // A new terminal, running your shell (Config.withTerminalAdded): its id.
  function addTerminal() {
    var r = Config.withTerminalAdded(svc.config, svc.takenIds())
    svc.replaceConfig(r.config)
    return r.id
  }
  // A terminal out of the config, its settings and the notches that list
  // it (Config.withoutTerminal).
  function removeTerminal(id) { svc.replaceConfig(Config.withoutTerminal(svc.config, id)) }
  // The ids a terminal can't take: the modules' and the config keys the
  // settings window writes (the host's included), by their first part.
  function takenIds() {
    var keys = svc.hostSettingsSchema.map(function(f) { return String(f.key || "").split(".")[0] })
    return svc.fileModuleIds.concat(keys.filter(function(k) { return k !== "" }))
  }
  property string terminalErrors: ""

  // id -> { title, settings, standalone, agent, terminal }, from each
  // module's JSON file (Settings.parseModuleInfo; a module without one has
  // none of them), and each terminal's entry.
  property var fileModuleInfo: ({})
  readonly property var moduleInfo: {
    var out = Object.assign({}, svc.fileModuleInfo)
    svc.terminalSpecs.forEach(function(t) { out[t.id] = Settings.terminalInfo(t) })
    return out
  }
  // Every module has its file read (or found missing): modules load only
  // then, so a standalone one never starts out in a notch. A terminal's
  // info comes with its entry: counting terminals, a new id made it false
  // for a moment, which emptied the list of modules and made them all
  // again, an agent's session with them.
  readonly property bool moduleInfoReady: svc.fileModuleIds.every(function(id) {
    return svc.fileModuleInfo[id] !== undefined
  })
  onModuleInfoReadyChanged: if (svc.moduleInfoReady) svc.refreshModuleIds()
  onConfigResolvedChanged: svc.refreshModuleIds()
  // Every module with its title and fields: [{ moduleId, title, settings,
  // standalone, agent, live, terminal }].
  readonly property var moduleCatalog: svc.availableModuleIds.map(function(id) {
    var info = svc.moduleInfo[id] || {}
    return { moduleId: id, title: Settings.moduleTitle(id, info), settings: info.settings || [],
             standalone: !!info.standalone, agent: !!info.agent, live: !!info.live, terminal: !!info.terminal }
  })
  // Agents ask their plan usage over the network, with the logins their
  // CLIs keep (AgentUsage). Off offscreen (scripts/isolated.sh sets
  // NOTCHD_OFFLINE): tests and renders never read a real login.
  property bool usageLive: Quickshell.env("NOTCHD_OFFLINE") !== "1"
  // Agents (`"agent"` in their file): coding agents in a terminal
  // (TerminalModule), with the fields every agent has and the `agent` IPC.
  function isAgent(id) { return !!svc.moduleInfo[id] && !!svc.moduleInfo[id].agent }
  // An agent's IPC command (TerminalModule.command): "ok", or what was wrong.
  function agentCommand(id, command) {
    if (!svc.isAgent(id)) {
      var agents = svc.availableModuleIds.filter(function(a) { return svc.isAgent(a) })
      return "no agent " + id + (agents.length > 0 ? "; agents: " + agents.join(", ") : "")
    }
    var m = svc.moduleById(id)
    return m ? m.command(command) : id + " module not loaded"
  }
  // The same for a terminal of your own (`terminals`).
  function terminalCommand(id, command) {
    if (!svc.isTerminal(id)) {
      var ids = svc.terminalSpecs.map(function(t) { return t.id })
      return "no terminal " + id + (ids.length > 0 ? "; terminals: " + ids.join(", ") : "")
    }
    var m = svc.moduleById(id)
    return m ? m.command(command) : id + " module not loaded"
  }

  // ---------------------------------------------------------------- live
  // Spoken conversations (`"live"` in their file, LiveModule): the bridge
  // that carries the voice (core/live/codex-live.py; the QML tests put a
  // fake in its place) and the `live` IPC.
  readonly property string liveBridge: Quickshell.env("NOTCHD_LIVE_BRIDGE")
    || Live.localPath(Qt.resolvedUrl("live/codex-live.py"), Quickshell.shellDir)
  // The local one (core/live/local-live.py: Voxtype and LocalAI).
  readonly property string localLiveBridge: Quickshell.env("NOTCHD_LOCAL_LIVE_BRIDGE")
    || Live.localPath(Qt.resolvedUrl("live/local-live.py"), Quickshell.shellDir)
  // The optional components the voice modules need (Components.js): the
  // installer (core/components/install.py; the QML tests put a fake in its
  // place), their registry, and their state while the settings are open.
  readonly property string componentsInstaller: Quickshell.env("NOTCHD_COMPONENTS_INSTALLER")
    || Live.localPath(Qt.resolvedUrl("components/install.py"), Quickshell.shellDir)
  readonly property ComponentStore components: ComponentStore {
    installer: svc.componentsInstaller
    catalogPath: Live.localPath(Qt.resolvedUrl("components/components.json"), Quickshell.shellDir)
    watching: svc.settingsOpen
  }
  function isLive(id) { return !!svc.moduleInfo[id] && !!svc.moduleInfo[id].live }
  // A live module's IPC command (LiveModule.command): "ok", or what was
  // wrong.
  function liveCommand(id, command) {
    if (!svc.isLive(id)) {
      var ids = svc.availableModuleIds.filter(function(a) { return svc.isLive(a) })
      return "no live module " + id + (ids.length > 0 ? "; live modules: " + ids.join(", ") : "")
    }
    var m = svc.moduleById(id)
    return m ? m.command(command) : id + " module not loaded"
  }
  // A notch is open on `module` by a click or a shortcut (pinned, showing
  // it): a live module talks only then.
  function liveOpen(module) {
    return svc.copies.some(function(c) {
      return c.pinned && c.mode === "expanded" && c.shownModules.indexOf(module) !== -1
    })
  }

  // Standalone modules (`"standalone": true` in their file) have a place of
  // their own, a notch that shows only them, and no notch of the config
  // shows them, whatever its `modules` say. They load while switched on
  // ("<id>.enabled", true by default); "<id>.position" places them.
  readonly property var standaloneIds: svc.availableModuleIds.filter(function(id) { return svc.isStandalone(id) })
  function isStandalone(id) { return !!svc.moduleInfo[id] && !!svc.moduleInfo[id].standalone }
  function standaloneEnabled(id) { return Config.valueAt(svc.config, id + ".enabled") !== false }
  // Where a standalone module shows: its edge and offset, on every screen
  // (a peek goes to the focused one).
  function standalonePosition(id) {
    var p = Config.valueAt(svc.config, id + ".position")
    return Object.assign({}, p && typeof p === "object" ? p : {}, { screen: "all" })
  }
  // A module's setting: the config's value, else its field's default.
  function moduleSetting(id, key) {
    var v = Config.valueAt(svc.config, id + "." + key)
    if (v !== undefined) return v
    return svc.moduleSettingDefault(id, key)
  }
  function moduleSettingDefault(id, key) {
    var info = svc.moduleInfo[id]
    return Settings.settingDefault(info ? info.settings : [], key)
  }
  function setModuleInfo(id, info) {
    var next = Object.assign({}, svc.fileModuleInfo)
    next[id] = info
    svc.fileModuleInfo = next
  }
  function moduleInfoPath(url) { return Settings.moduleInfoPath(url, Quickshell.shellDir) }

  Instantiator {
    model: svc.fileModuleIds
    delegate: FileView {
      required property string modelData
      path: svc.moduleInfoPath(svc.infoUrl(modelData))
      blockLoading: true
      printErrors: false
      onLoaded: {
        var info = Settings.parseModuleInfo(text())
        if (info.error) console.warn(svc.logName + ": ignoring " + path + ": " + info.error)
        svc.setModuleInfo(modelData, info)
      }
      onLoadFailed: svc.setModuleInfo(modelData, { title: "", settings: [] })
      // A module url in a form moduleInfoPath doesn't know would lose its
      // title and settings without a word.
      Component.onCompleted: if (path === "" && String(svc.infoUrl(modelData)) !== "")
        console.warn(svc.logName + ": no settings file for module '" + modelData + "' at " + svc.infoUrl(modelData))
    }
  }

  // ---------------------------------------------------------------- config
  // The parsed config, so hosts can read their own keys from it.
  property var config: ({})
  property int hoverOpenDelay: Config.DEFAULTS.hoverOpenDelay
  property int hoverCloseDelay: Config.DEFAULTS.hoverCloseDelay
  // Every notch of the config, resolved (Config.notches): the keys the
  // daemon's `state` does not carry (a notch's own config key prefix, its
  // workspaces, `reserveSpace`, `idleIcons`).
  property var notchSpecs: Config.notches({}, svc.defaultModules)
  // The ids of the notches, as the daemon's `state` lists them (config
  // order): a notch exists once the daemon has said so.
  property var notchIds: []
  // The modules to load: every one some notch shows, and the standalone
  // ones switched on (Config.moduleLoadList).
  property var moduleIds: []
  // Read fresh: called as the module files finish loading, before
  // `standaloneIds` follows them (the OSD was left out at startup).
  function loadList(specs) {
    var standalone = svc.availableModuleIds.filter(function(id) { return svc.isStandalone(id) })
    // A user module that is standalone says so to the daemon, not in a file.
    svc.standaloneStateIds.forEach(function(id) { if (standalone.indexOf(id) === -1) standalone.push(id) })
    var enabled = standalone.filter(function(id) { return svc.standaloneEnabled(id) })
    return Config.moduleLoadList(specs, standalone, enabled)
  }

  function sameList(a, b) { return JSON.stringify(a) === JSON.stringify(b) }
  function refreshModuleIds() {
    var modules = svc.loadList(svc.notchSpecs)
    if (!svc.sameList(modules, svc.moduleIds)) svc.moduleIds = modules
  }

  // Keys missing from the config fall back to their defaults, so deleting a
  // key in the file (or the settings window) resets it. The daemon's
  // `resolved` has the values it works out (the hover delays); Config.notches
  // gives the keys of a notch the daemon's `state` does not carry. The module
  // list is only replaced when it changes: a new list would make every
  // module again, losing what it shows.
  function applyConfig(cfg, resolved) {
    var r = resolved || Config.resolve(cfg)
    var specs = Config.notches(cfg, svc.defaultModules)
    var terminals = Config.terminals(cfg, svc.takenIds())
    var errors = terminals.errors.join("; ")
    if (errors !== "" && errors !== svc.terminalErrors) console.warn(svc.logName + ": skipping terminals: " + errors)
    svc.terminalErrors = errors
    if (!svc.sameList(terminals.list, svc.terminalSpecs)) svc.terminalSpecs = terminals.list
    svc.config = cfg
    svc.hoverOpenDelay = r.hoverOpenDelay
    svc.hoverCloseDelay = r.hoverCloseDelay
    svc.notchSpecs = specs
    var modules = svc.loadList(specs)
    if (!svc.sameList(modules, svc.moduleIds)) svc.moduleIds = modules
  }

  // -------------------------------------------------------------- daemon
  // The daemon reads, watches and writes the config file and builds the
  // settings schema (docs/architecture.md, "Protocol: core to UI"); this
  // service holds what it sends (`config`, `schema`) and sends it every edit.
  readonly property DaemonLauncher launcher: DaemonLauncher {
    id: daemonLauncher
    script: svc.daemonScript
    command: svc.daemonCommand !== null ? svc.daemonCommand : ["bash", daemonLauncher.scriptPath]
    enabled: svc.daemonCommand !== null || String(svc.daemonScript) !== ""
    logName: svc.logName
    clientName: svc.daemonClientName
  }
  readonly property DaemonClient client: daemonLauncher.client
  // The last `schema` message: kept while the daemon is away, like the config.
  property var daemonSchema: null

  // No daemon: it never came up, or it went away and is being started again.
  // The notch says so (DaemonDown), after `daemonDownDelay` so a start (or a
  // restart that takes a moment) does not flash it.
  readonly property bool daemonDown: svc.daemonAway && svc.daemonDownShown
  readonly property bool daemonAway: !svc.client.ready
  property bool daemonDownShown: false
  Timer {
    interval: svc.daemonDownDelay
    running: svc.daemonAway
    onTriggered: svc.daemonDownShown = true
  }
  onDaemonAwayChanged: if (!svc.daemonAway) svc.daemonDownShown = false

  Connections {
    target: svc.client
    function onUpdated(t, key, msg) {
      // Lists in a message that came through a signal are sequences, not
      // arrays (Array.isArray is false for them): plain copies.
      if (t === "config") svc.daemonConfig(svc.plain(msg))
      else if (t === "schema") svc.daemonSchema = svc.plain(msg)
      else if (t === "state") svc.applyState(svc.plain(msg))
    }
    // A config file the daemon could not parse: it keeps the last good one
    // and says which file and why; so do we.
    function onMessage(msg) {
      if (msg.t === "config-error") console.warn(svc.logName + ": ignoring " + msg.message)
    }
    function onWelcomed(welcome) {
      svc.reportCatalog()
      svc.reportSettings()
      // A daemon that came back knows none of the screens or modules: all of it again.
      svc.screensSent = ""
      svc.factsSent = ""
      svc.flushReports()
      svc.flushConfig()
    }
  }

  // ------------------------------------------------------ the notch state
  // The daemon decides the notches' state (docs/architecture.md, "State: full
  // updates per entity"): modes, hover timers, peeks and where they go,
  // pinning, tabs, navigation, which screen each copy is on, the space to
  // reserve. This holds its last `state` and the UI binds to it (Notch,
  // NotchCopy, NotchSurface); what it sends are raw inputs. While the daemon
  // is away the last state stays, so the notch is frozen where it was
  // instead of blank, and is replaced by the new daemon's as it connects.
  property var notchState: null
  // id -> the daemon's notch, and its standalone modules' (by module id).
  readonly property var notchInfo: svc.byId(svc.notchState ? svc.notchState.notches : [])
  readonly property var standaloneInfo: svc.byId(svc.notchState ? svc.notchState.standalone : [])
  // The visitor's copies (by screen).
  readonly property var visitorInfo: svc.notchState ? svc.notchState.visitor : []
  // The standalone modules that have a notch, as the daemon says: stable,
  // like `notchIds`, so a new state does not make every notch again.
  property var standaloneStateIds: []
  onStandaloneStateIdsChanged: svc.refreshModuleIds()
  // { "<screen>|<edge>": depth }: what windows keep clear (NotchSurface).
  property var reserveZones: ({})

  function byId(list) {
    var out = {}
    for (var i = 0; i < list.length; i++) out[list[i].id] = list[i]
    return out
  }

  function applyState(msg) {
    svc.notchState = msg
    var ids = msg.notches.map(function(n) { return n.id })
    if (!svc.sameList(ids, svc.notchIds)) svc.notchIds = ids
    var standalone = msg.standalone.map(function(n) { return n.id })
    if (!svc.sameList(standalone, svc.standaloneStateIds)) svc.standaloneStateIds = standalone
    var zones = {}
    for (var i = 0; i < msg.reserve.length; i++) zones[msg.reserve[i].screen + "|" + msg.reserve[i].edge] = msg.reserve[i].depth
    if (!svc.sameList(zones, svc.reserveZones)) svc.reserveZones = zones
    svc.followSettings(msg)
  }

  // The settings window follows the daemon when it opens or moves it (the
  // `s` key in navigation, `ipc settings`), not when the daemon only echoes
  // what this window reported a moment ago.
  property var settingsReports: []
  property string settingsSeen: "false|general"
  function followSettings(msg) {
    var key = msg.settingsOpen + "|" + msg.settingsPage
    if (key === svc.settingsSeen) return
    svc.settingsSeen = key
    var now = Date.now()
    svc.settingsReports = svc.settingsReports.filter(function(r) { return now - r.at < 2000 })
    if (svc.settingsReports.some(function(r) { return r.key === key })) return
    // The daemon already has it: nothing to report back.
    svc.settingsFollowing = true
    if (msg.settingsOpen && msg.settingsPage !== svc.settingsPage) svc.settingsPage = msg.settingsPage
    if (msg.settingsOpen !== svc.settingsOpen) svc.settingsOpen = msg.settingsOpen
    svc.settingsFollowing = false
  }
  property bool settingsFollowing: false

  // What the modules this UI loads say of themselves (`facts`): sent when
  // they change and again whenever the daemon (re)connects, and before a
  // peek request, so the daemon never decides on a module as it was.
  readonly property string factsReport: JSON.stringify(svc.modules.filter(function(m) { return !m.daemonOwned }).map(function(m) {
    return {
      id: m.moduleId, priority: m.priority, active: m.active, compact: !!m.compact, peek: !!m.peek,
      icon: !!m.icon, expanded: !!m.expanded, attention: m.attention, urgentPeek: m.urgentPeek,
      peekOnFocusedScreen: m.peekOnFocusedScreen, standalone: svc.isStandalone(m.moduleId)
    }
  }))
  property string factsSent: ""
  onFactsReportChanged: Qt.callLater(svc.reportFacts)
  function reportFacts() {
    if (!svc.client.ready || svc.factsSent === svc.factsReport) return
    svc.factsSent = svc.factsReport
    svc.client.facts(JSON.parse(svc.factsReport))
  }

  // The config the daemon sent. While edits of ours are on their way it is
  // the one before them: the optimistic copy stays until they are answered
  // (`settleConfig`).
  function daemonConfig(msg) {
    var first = !svc.configResolved
    svc.configResolved = true
    if (svc.configSyncing) return
    // Our copy usually is it already (our own edit, echoed): nothing changes.
    if (first || !svc.sameConfig(msg.cfg, svc.config)) svc.applyConfig(msg.cfg, msg.resolved)
  }

  // Two configs with the same keys and values, whatever their key order (the
  // daemon sends them sorted).
  function sameConfig(a, b) { return svc.canonical(a) === svc.canonical(b) }
  function plain(v) { return JSON.parse(JSON.stringify(v)) }
  function canonical(v) {
    if (Array.isArray(v)) return "[" + v.map(svc.canonical).join(",") + "]"
    if (v !== null && typeof v === "object")
      return "{" + Object.keys(v).sort().map(function(k) { return JSON.stringify(k) + ":" + svc.canonical(v[k]) }).join(",") + "}"
    return JSON.stringify(v)
  }

  // What the daemon needs to know of this UI to build the schema: the modules
  // it loads itself (`catalog`), the screens and the compositor's workspaces
  // (`screens`), the settings window's page (`settings`). Sent when they
  // change and again whenever the daemon (re)connects.
  readonly property string catalogReport: JSON.stringify(svc.moduleCatalog)
  onCatalogReportChanged: Qt.callLater(svc.reportCatalog)
  onDefaultModulesChanged: Qt.callLater(svc.reportCatalog)
  function reportCatalog() {
    if (svc.client.ready) svc.client.send({ t: "catalog", modules: JSON.parse(svc.catalogReport), defaultModules: svc.defaultModules })
  }
  readonly property string screensReport: {
    var c = svc.compositor
    var fullscreen = []
    var workspaces = {}
    var screens = (svc.screens || []).map(function(s, i) {
      if (c && c.hasFullscreen(s)) fullscreen.push(s.name)
      var ws = c && c.supportsWorkspaces ? c.activeWorkspace(s) : null
      if (ws) workspaces[s.name] = { id: String(ws.id), name: String(ws.name !== undefined ? ws.name : ws.id) }
      return { name: s.name, x: s.x || 0, y: s.y || 0, w: s.width || 0, h: s.height || 0, scale: s.devicePixelRatio || 1 }
    })
    return JSON.stringify({
      t: "screens", screens: screens, focused: c ? String(c.focusedScreenName || "") : "",
      fullscreen: fullscreen, workspaces: workspaces,
      known: c && c.supportsWorkspaces ? Array.prototype.map.call(c.knownWorkspaces || [], String) : [],
      supportsWorkspaces: !!(c && c.supportsWorkspaces), barEdge: svc.barEdge
    })
  }
  onScreensReportChanged: Qt.callLater(svc.reportScreens)
  property string screensSent: ""
  function reportScreens() {
    if (!svc.client.ready || svc.screensSent === svc.screensReport) return
    svc.screensSent = svc.screensReport
    svc.client.send(JSON.parse(svc.screensReport))
  }
  // What the daemon decides on is what this UI last told it: before an
  // input that depends on the config, the screens or the modules (a peek
  // goes to the focused screen, to the notches that show the module), what
  // is waiting to be sent goes first.
  function flushReports() {
    svc.flushConfig()
    svc.reportScreens()
    svc.reportFacts()
  }
  function reportSettings() {
    if (!svc.client.ready || svc.settingsFollowing) return
    svc.settingsReports = svc.settingsReports.concat([{ key: svc.settingsOpen + "|" + svc.settingsPage, at: Date.now() }])
    svc.client.send({ t: "settings", open: svc.settingsOpen, page: svc.settingsPage })
  }

  // ------------------------------------------------------- config editing
  // Dotted keys reach into objects: "theme.foreground".
  function configValue(key) { return Config.valueAt(svc.config, key) }

  // Every write goes to the daemon (`set`), which edits the config as
  // `Config.withValue` does, writes the file atomically and sends the result
  // to every UI. Here it applies at once, so the window and the notch do not
  // wait for the round trip; sets to one key are coalesced and sent shortly
  // after (`saveTimer`), so dragging a slider does not write on every step.
  // `undefined` removes the key (back to its default); emptied parent
  // objects are removed too.
  property var pendingSets: []
  property int setsInFlight: 0
  readonly property bool configSyncing: svc.pendingSets.length > 0 || svc.setsInFlight > 0

  function setConfigValue(key, value) {
    svc.applyConfig(Config.withValue(svc.config, key, value))
    svc.queueSet(key, value)
  }

  // The whole config at once: each top-level key that differs is a `set`.
  function replaceConfig(cfg) {
    var old = svc.config
    svc.applyConfig(cfg)
    var keys = Object.keys(old)
    Object.keys(cfg).forEach(function(k) { if (keys.indexOf(k) === -1) keys.push(k) })
    keys.forEach(function(k) {
      if (JSON.stringify(old[k]) !== JSON.stringify(cfg[k])) svc.queueSet(k, cfg[k])
    })
  }

  function queueSet(key, value) {
    var queue = svc.pendingSets.filter(function(e) { return e.key !== key })
    queue.push({ key: key, value: value })
    svc.pendingSets = queue
    saveTimer.restart()
  }

  Timer {
    id: saveTimer
    interval: 120
    onTriggered: svc.flushConfig()
  }

  // Sends what is queued, if the daemon is there; else it waits for it.
  function flushConfig() {
    saveTimer.stop()
    if (!svc.client.ready || svc.pendingSets.length === 0) return
    var queue = svc.pendingSets
    svc.pendingSets = []
    queue.forEach(function(e) {
      svc.setsInFlight++
      svc.client.set(e.key, e.value, function(reply) { svc.setAnswered(e, reply) })
    })
  }

  function setAnswered(entry, reply) {
    svc.setsInFlight--
    if (reply.t === "err") {
      // The connection dropped first: send it again once it is back, unless a
      // later edit of the key is waiting.
      if (reply.code === "disconnected") {
        if (!svc.pendingSets.some(function(e) { return e.key === entry.key })) svc.pendingSets = [entry].concat(svc.pendingSets)
      } else {
        console.warn(svc.logName + ": could not write " + svc.configPath + ": " + (reply.message || reply.code))
      }
    }
    svc.settleConfig()
  }

  // Nothing of ours is left on its way: the daemon's config is the config
  // (what its `with_value` made of the edits), whatever our copy says.
  function settleConfig() {
    if (svc.configSyncing || !svc.client.config) return
    if (!svc.sameConfig(svc.client.config.cfg, svc.config)) svc.applyConfig(svc.client.config.cfg, svc.client.config.resolved)
  }

  // --------------------------------------------------------------- settings
  // Sidebar pages of the settings window, in order. A page shows only when
  // some field is on it.
  //   id, label
  //   icon       Nerd Font code point (or glyph string)
  readonly property var corePages: [
    { id: "general", label: "General", icon: 0xF0493 },      // nf-md-cog
    { id: "notch", label: "Notches", icon: 0xF0379 },        // nf-md-monitor
    { id: "modules", label: "Modules", icon: 0xF0A66 },      // nf-md-puzzle_outline
    { id: "components", label: "Components", icon: 0xF03D6 }, // nf-md-package_variant_closed
    { id: "appearance", label: "Appearance", icon: 0xF0E0C } // nf-md-palette_outline
  ]
  // Then a sub-page per notch ("notch/<index>") and per module
  // ("modules/<id>"), for their own fields, reached from the Notches and
  // Modules pages. The daemon's schema has the notches' and the user
  // modules'; the built-in modules' pages are built here.
  readonly property var settingsPages: Settings.windowPages(svc.corePages, svc.hostSettingsPages,
    svc.daemonSchema ? svc.daemonSchema.pages : [], svc.moduleCatalog)

  // The fields the settings window shows come from the daemon's `schema`
  // (the core's pages, a notch's, a user module's: Settings.windowFields),
  // and what it does not carry yet is built here: the Components page, the
  // built-in modules' pages, the host's own fields, the edge cards' previews
  // (remaining for later slices: they move with the modules and components).
  // A descriptor is:
  //   key        config key (dotted for nested objects)
  //   type       "choice" | "integer" | "boolean" | "string" | "color"
  //              | "modules" | "workspaces" | "notches" (the notch list)
  //              | "moduleCatalog" (every module, a gear to its page)
  //              | "note" (text only, no key)
  //   label, help
  //   page       sidebar page id (default "general")
  //   section    heading the field sits under, within its page
  //   default    value shown while the key is absent
  //   options    [{ value, label, preview }] for "choice"; `preview` is the
  //              url of a QML file drawn on the card (root takes `selected`)
  //   display    "cards" shows a choice as picture cards instead of segments
  //   min, max, step, unit   for "integer"
  //
  // A notch's own keys sit on its page ("notch/<index>"), reached from the
  // list on the "notch" page, and are those of the notch the window edits
  // (settingsNotch): in a list, "notches.<index>.<key>", whose default is
  // the top-level key it inherits.
  // "general" has what applies to every notch, "modules" every module the
  // host can load, each module's own fields on its page under it.
  // Component rows filled in from the registry, and the choices that wait on
  // a component locked until it is installed (Components.decorate). It
  // follows the set of installed components, never an installation's
  // progress.
  readonly property var settingsSchema: Components.decorate(
    Settings.windowFields(svc.daemonSchema ? svc.daemonSchema.fields : [], svc.moduleCatalog, Qt.resolvedUrl("settings/previews/Edge.qml"))
    .concat(Components.overviewFields(svc.components.catalog, svc.components.settled),
            Settings.moduleFields(svc.moduleCatalog, Qt.resolvedUrl("settings/previews/Edge.qml")),
            svc.hostSettingsSchema || []),
    svc.components.catalog, svc.components.loaded ? svc.components.installed : null, svc.moduleTitles)
  // id -> title of every module, for the texts that name them.
  readonly property var moduleTitles: {
    var out = {}
    svc.moduleCatalog.forEach(function(m) { out[m.moduleId] = m.title })
    return out
  }

  property bool settingsOpen: false
  // The page the window shows; kept here so it survives closing the window.
  property string settingsPage: "general"
  // The notch the window edits, by index; showing a notch's page picks it.
  property int settingsNotch: 0
  // The daemon builds the schema for the notch the page names, so it is told
  // of the page, and of the window opening and closing (its `state`).
  onSettingsPageChanged: {
    var index = Settings.notchIndexOfPage(svc.settingsPage)
    if (index !== -1) svc.settingsNotch = index
    svc.reportSettings()
  }
  onSettingsOpenChanged: svc.reportSettings()
  // `index`: the notch to edit, if given; the window then shows its page.
  function openSettings(index) {
    if (typeof index === "number") {
      svc.settingsNotch = index
      svc.settingsPage = Settings.notchPageId(index)
    }
    svc.settingsOpen = true
  }
  function closeSettings() { svc.settingsOpen = false }

  // `svc: svc` would bind the window's own property to itself (the property
  // name shadows the id), so the reference goes through `owner`.
  LazyLoader {
    id: settingsLoader
    readonly property var owner: svc
    active: svc.settingsOpen
    SettingsWindow { svc: settingsLoader.owner }
  }

  // --------------------------------------------------------------- notches
  function notchSpec(id) {
    for (var i = 0; i < svc.notchSpecs.length; i++)
      if (svc.notchSpecs[i].id === id) return svc.notchSpecs[i]
    return null
  }

  // `notch: svc` would bind the notch's own property to itself (the property
  // name shadows the id), so the reference goes through `owner`.
  Variants {
    id: notchVariants
    readonly property var owner: svc
    model: svc.notchIds
    Notch {
      required property var modelData
      svc: notchVariants.owner
      notchId: modelData
    }
  }
  // A notch per standalone module switched on: on every screen where its
  // position says, hidden until it peeks, and drawn over the notches there
  // (NotchSurface, Position.coveredByStandalone). The daemon says which.
  Variants {
    id: standaloneVariants
    readonly property var owner: svc
    model: svc.configResolved && svc.moduleInfoReady ? svc.standaloneStateIds : []
    Notch {
      required property var modelData
      svc: standaloneVariants.owner
      standaloneId: modelData
    }
  }
  readonly property var standaloneNotches: standaloneVariants.instances
  function standaloneNotch(id) {
    for (var i = 0; i < svc.standaloneNotches.length; i++)
      if (svc.standaloneNotches[i].standaloneId === id) return svc.standaloneNotches[i]
    return null
  }

  // The default notch: a notch of its own at the top of every screen, hidden
  // until it shows a peek where no notch with that module does (the daemon
  // routes it).
  Notch {
    id: visitorNotch
    // Through `owner`, as above.
    svc: notchVariants.owner
    visitor: true
    notchId: ""
  }
  readonly property var visitor: visitorNotch

  // The notches, in config order.
  readonly property var notches: {
    var list = notchVariants.instances.slice()
    list.sort(function(a, b) { return a.index - b.index })
    return list
  }
  // Every copy of every notch: what NotchSurface puts on screen.
  readonly property var copies: {
    var out = []
    for (var i = 0; i < svc.notches.length; i++) out = out.concat(svc.notches[i].copies)
    out = out.concat(visitorNotch.copies)
    for (var j = 0; j < svc.standaloneNotches.length; j++) out = out.concat(svc.standaloneNotches[j].copies)
    return out
  }
  // The first notch's first copy: what a single-notch view (the visual
  // tests, the README image) draws.
  readonly property var firstCopy: svc.copies.length > 0 ? svc.copies[0] : null

  function notchById(id) {
    for (var i = 0; i < svc.notches.length; i++)
      if (svc.notches[i].notchId === id) return svc.notches[i]
    return null
  }

  // A new notch in the config, off the edges the others hang from; and
  // taking one out (Config.withNotchAdded / withoutNotch). Returns the new
  // notch's index.
  function addNotch() {
    var used = svc.notches.map(function(n) { return n.edge })
    var edges = ["bottom", "left", "right", "top"].filter(function(e) { return used.indexOf(e) === -1 })
    var entry = { id: Config.freeNotchId(svc.notchIds), modules: [] }
    entry.position = { edge: edges.length > 0 ? edges[0] : "bottom" }
    var next = Config.withNotchAdded(svc.config, entry)
    svc.replaceConfig(next)
    return next.notches.length - 1
  }
  function removeNotch(index) { svc.replaceConfig(Config.withoutNotch(svc.config, index)) }

  // What modules read: whether any notch is open, and whether one shows
  // `module`'s peek.
  readonly property bool expanded: svc.copies.some(function(c) { return c.mode === "expanded" })
  function peeking(module) {
    return svc.copies.some(function(c) { return c.mode === "peek" && c.peekModule === module })
  }

  // ------------------------------------------------- what the UI tells it
  // A module's peek is the daemon's to route: to every notch that shows the
  // module (on the copy on the focused screen of one that has a copy per
  // screen), and to the visitor when none of them peeks on the focused
  // screen; a module that peeks on the focused screen only peeks once; a
  // standalone module's goes to its own notch. The module's facts go first,
  // so it decides on the module as it is.
  function requestPeek(module, ms) {
    if (!module) return
    svc.flushReports()
    svc.client.peek(module.moduleId, ms)
  }
  function dismissPeek(module) {
    svc.flushReports()
    svc.client.peekDismiss(module ? module.moduleId : "")
  }

  // A module you type into asks for the keyboard (a press on its field):
  // every copy open on it pins, so its window takes the keyboard and the
  // pointer leaving doesn't close it.
  function requestKeyboard(module) {
    svc.flushReports()
    svc.client.moduleNotch(module.moduleId, "keyboard")
  }

  // A module opens the first notch of the config that shows it, or closes
  // every one that does (NotchModule.openNotch / closeNotch). Whether one
  // does is the daemon's `state`: the answer is that of the last one.
  function showsModule(module) {
    var list = svc.notchState ? svc.notchState.notches : []
    for (var i = 0; i < list.length; i++)
      if (list[i].modules.indexOf(module.moduleId) !== -1) return true
    return false
  }
  function openModule(module) {
    if (!svc.showsModule(module)) return false
    svc.flushReports()
    svc.client.moduleNotch(module.moduleId, "open")
    return true
  }
  function closeModule(module) {
    svc.flushReports()
    svc.client.moduleNotch(module.moduleId, "close")
  }

  // An IPC method of a module of the daemon (`push`, `clear`, `timer`, `osd`):
  // sent at once, answered with `answer` (the host's target cannot wait).
  function daemonIpc(method, args, answer) {
    if (answer !== "ok") return answer
    return svc.notchIpc(method, args) ? "ok" : "notchd is not running"
  }
  // A module's own "not loaded" answer, which needs no daemon: the load list
  // is known here. Else `answer`. Without it, a command for a module no notch
  // lists answered `ok` and did nothing.
  function loadedAnswer(id, answer) {
    return svc.moduleIds.indexOf(id) === -1 ? id + " module not loaded" : answer
  }
  // What `push` answers to a payload that is not a JSON object.
  function pushAnswer(json) {
    var o
    try { o = JSON.parse(String(json || "")) } catch (e) { return "invalid json" }
    return o && typeof o === "object" && !Array.isArray(o) ? "ok" : "invalid json"
  }

  // Without a notch named, open and toggle act on the first notch and close
  // closes them all. The daemon does it (`ipc`); false when it is not there.
  function notchIpc(method, args) {
    svc.flushReports()
    return svc.client.ipc(method, args || [], null) >= 0
  }
  function open() { svc.notchIpc("open") }
  function close() { svc.notchIpc("close") }
  function toggle() { svc.notchIpc("toggle") }

  // ------------------------------------------------------------ navigation
  // A shortcut (IPC `navigate`) highlights every notch with its number and
  // the keyboard picks one: its copy on the focused screen opens as a
  // preview, Tab walks its tabs, Enter pins it open (the others close) and
  // Escape leaves everything as it was. Keys also open the selected notch's
  // settings, remove it (asked twice) and move it (mode "move": the edge,
  // along it, the screen; Escape puts it back). The daemon decides it all
  // (its `state.navigation`); NotchSurface's navigation window holds the
  // keyboard meanwhile and hands keys to navKey().
  readonly property var navigation: svc.notchState ? svc.notchState.navigation : null
  readonly property bool navigating: !!svc.navigation
  // "select", "move" or "remove".
  readonly property string navMode: svc.navigation ? svc.navigation.mode : "select"
  // The selected notch, by index in `notches`.
  readonly property int navSelected: svc.navigation ? svc.navigation.selected : -1
  readonly property var navNotch: svc.navigating && svc.navSelected >= 0 && svc.navSelected < svc.notches.length
    ? svc.notches[svc.navSelected] : null

  // Starts it, or leaves it as Escape does: "ok", or what was wrong.
  function navigate() {
    if (!svc.navigating && svc.notchIds.length === 0) return "no notches"
    return svc.notchIpc("navigate") ? "ok" : "notchd is not running"
  }
  // A key while navigating, as the daemon names it (Protocol.wireKey).
  function navKey(key, mods) {
    svc.flushReports()
    svc.client.key("", "", key, mods || [], "")
  }
  // The window that has the keyboard for it went away: navigation ends.
  function endNavigation() { if (svc.navigating) svc.navigate() }

  // --------------------------------------------------------------- modules
  property var modules: []
  // Modules load once the config is read (or known to be missing), so they
  // are not created from the defaults and then again from the config.
  property bool configResolved: false

  function rebuildModules() {
    var out = []
    for (var i = 0; i < moduleLoaders.count; i++) {
      var loader = moduleLoaders.itemAt(i)
      if (loader && loader.item) out.push(loader.item)
    }
    svc.modules = out
  }

  function moduleById(id) {
    for (var i = 0; i < svc.modules.length; i++)
      if (svc.modules[i].moduleId === id) return svc.modules[i]
    return null
  }

  Repeater {
    id: moduleLoaders
    model: svc.configResolved && svc.moduleInfoReady ? svc.moduleIds : []
    delegate: Loader {
      required property string modelData
      source: svc.loadUrl(modelData)
      onLoaded: {
        var module = item as NotchModule
        if (!module) { console.warn(svc.logName + ": module '" + modelData + "' is not a NotchModule"); return }
        module.notch = svc
        if (module.moduleId === "") module.moduleId = modelData
        if (module.title === "" && !module.daemonOwned) {
          var id = module.moduleId
          module.title = Qt.binding(function() { return Settings.moduleTitle(id, svc.moduleInfo[id]) })
        }
        svc.rebuildModules()
      }
      onStatusChanged: if (status === Loader.Error) console.warn(svc.logName + ": module '" + modelData + "' failed to load")
      // The daemon may not have announced a module of its own yet: unknown
      // only if it still has not a moment after it is connected.
      readonly property Timer unknown: Timer {
        interval: 3000
        running: String(parent.source) === "" && svc.client.ready
        onTriggered: console.warn(svc.logName + ": unknown module '" + parent.modelData + "'")
      }
      Component.onDestruction: Qt.callLater(svc.rebuildModules)
    }
  }

  // ------------------------------------------------------------------- IPC
  function ipcNotch(id) { return svc.notchById(id) }
  IpcHandler {
    target: svc.ipcTarget
    function toggle(): void { svc.toggle() }
    function open(): void { svc.open() }
    function close(): void { svc.close() }
    // The same for one notch, by id.
    function toggleNotch(id: string): string { var n = svc.ipcNotch(id); if (!n) return "no notch " + id; n.toggle(); return "ok" }
    function openNotch(id: string): string { var n = svc.ipcNotch(id); if (!n) return "no notch " + id; n.open(); return "ok" }
    function closeNotch(id: string): string { var n = svc.ipcNotch(id); if (!n) return "no notch " + id; n.close(); return "ok" }
    function settings(): void { svc.settingsOpen = !svc.settingsOpen }
    // Navigation: every notch highlighted and numbered, the keyboard picks
    // one; again leaves it.
    function navigate(): string { return svc.navigate() }
    // Every notch back to the middle of the top edge, following focus, after
    // a drag that went astray.
    function recenter(): void {
      var cfg = Config.withValue(svc.config, "position", undefined)
      for (var i = 0; Config.hasNotchList(cfg) && i < cfg.notches.length; i++)
        cfg = Config.withValue(cfg, "notches." + i + ".position", undefined)
      svc.replaceConfig(cfg)
    }
    // Change one config key, as the settings window does. `value` is JSON
    // ("200", "\"notch\"", "[\"media\"]"); empty resets the key to its default.
    // The daemon owns the config: this answers as its `set` method does (an
    // IPC handler cannot wait for a reply) and sends the edit at once.
    function set(key: string, value: string): string {
      if (!key) return "usage: set <key> <json value>"
      if (value === "") { svc.setConfigValue(key, undefined); svc.flushConfig(); return "ok" }
      var parsed
      try { parsed = JSON.parse(value) } catch (e) { return "not JSON: " + value }
      svc.setConfigValue(key, parsed)
      svc.flushConfig()
      return "ok"
    }
    // Generic activities, for scripts, and the timer ("25m", "3m Tea",
    // "pomodoro", "toggle", "+5m", "skip", "stop"): the daemon's modules
    // (native/notchd-modules). An IPC handler cannot wait for the daemon's
    // answer, so these send the method and answer `ok` (`invalid json` for a
    // push that is not, which is known here); `notchd ipc <method>` has the
    // module's own answer (`no timer running`...). A module no notch loads answers
    // `<id> module not loaded` here, as before (svc.loadedAnswer).
    function push(json: string): string { return svc.daemonIpc("push", [json], svc.loadedAnswer("activities", svc.pushAnswer(json))) }
    function clear(id: string): string { return svc.daemonIpc("clear", [id], svc.loadedAnswer("activities", "ok")) }
    function timer(command: string): string { return svc.daemonIpc("timer", [command], svc.loadedAnswer("timer", "ok")) }
    // An agent, by its module's id: "toggle" opens or closes its notch,
    // "new" starts a new session, "ask <question>" opens the notch and types
    // it in; see TerminalModule.qml. `claude` and `codex` are the same.
    function agent(id: string, command: string): string { return svc.agentCommand(id, command) }
    // A terminal of your own (`terminals`), by its id: the same commands.
    function terminal(id: string, command: string): string { return svc.terminalCommand(id, command) }
    function claude(command: string): string { return svc.agentCommand("claude", command) }
    function codex(command: string): string { return svc.agentCommand("codex", command) }
    // A spoken conversation, by its module's id: "start" opens its notch and
    // starts talking, "stop", "toggle", "mute", "unmute"; see LiveModule.qml.
    function live(id: string, command: string): string { return svc.liveCommand(id, command) }
    function state(): string {
      return JSON.stringify({
        // The selected notch while navigating, else null.
        navigation: svc.navNotch ? { notch: svc.navNotch.notchId } : null,
        notches: svc.notches.map(function(n) {
          return {
            id: n.notchId,
            reserveSpace: n.reserveSpace,
            tabs: n.tabs,
            idleIcons: n.idleIcons,
            workspaces: n.workspaces,
            position: n.placement,
            draggable: n.canDrag,
            modules: n.modules.map(function(m) { return m.moduleId }),
            copies: n.copies.map(function(c) {
              return { screen: c.screenName, mode: c.mode, peek: c.peekModule ? c.peekModule.moduleId : null }
            })
          }
        }),
        // The visitor's copies that show something.
        visitor: visitorNotch.copies.filter(function(c) { return c.mode === "peek" || c.mode === "expanded" })
          .map(function(c) { return { screen: c.screenName, mode: c.mode, peek: c.peekModule ? c.peekModule.moduleId : null } }),
        // Each standalone module's place, and its copies that show something.
        standalone: svc.standaloneNotches.map(function(n) {
          return {
            id: n.standaloneId,
            position: n.placement,
            copies: n.copies.filter(function(c) { return c.mode === "peek" || c.mode === "expanded" })
              .map(function(c) { return { screen: c.screenName, mode: c.mode } })
          }
        }),
        modules: svc.modules.map(function(m) {
          var out = { id: m.moduleId, active: m.active, priority: m.priority }
          // An agent's plan usage (AgentUsage): its limits, and why they are stale.
          var u = m.usage
          if (u && u.active) out.usage = { limits: u.limits, stale: u.stale, problem: u.problem, fetchedAt: u.fetchedAt }
          return out
        })
      })
    }
  }
}
