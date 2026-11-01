import QtQuick
import Quickshell
import Quickshell.Io
import "settings"
import "Config.js" as Config
import "State.js" as State
import "Navigate.js" as Navigate
import "Live.js" as Live
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
  // Absolute path of the JSON config. Watched; edits apply live. Defaults
  // to the user's config dir, so settings survive plugin updates and never
  // land in the source tree. A missing file means every key at its default.
  readonly property string defaultConfigPath: {
    var base = Quickshell.env("XDG_CONFIG_HOME") || (Quickshell.env("HOME") + "/.config")
    return base + "/notchd/config.json"
  }
  property string configPath: svc.defaultConfigPath
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
    media: Qt.resolvedUrl("modules/media.qml"),
    activities: Qt.resolvedUrl("modules/activities.qml"),
    resources: Qt.resolvedUrl("modules/resources.qml"),
    timer: Qt.resolvedUrl("modules/timer.qml"),
    claude: Qt.resolvedUrl("modules/claude.qml"),
    codex: Qt.resolvedUrl("modules/codex.qml"),
    "codex-live": Qt.resolvedUrl("modules/codex-live.qml"),
    "local-live": Qt.resolvedUrl("modules/local-live.qml")
  })

  function moduleUrl(id) {
    if (svc.hostModules && svc.hostModules[id]) return svc.hostModules[id]
    if (svc.builtinModules[id]) return svc.builtinModules[id]
    return svc.isTerminal(id) ? Qt.resolvedUrl("terminal/CustomTerminal.qml") : ""
  }

  // Every module that has a file: built-ins first, then the host's. Its
  // title and settings fields are in a JSON file next to its QML
  // (media.json), read whether it loads or not (moduleInfo).
  readonly property var fileModuleIds: {
    var out = Object.keys(svc.builtinModules)
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
  // The local one (core/live/local-live.py: whisper.cpp and Piper).
  readonly property string localLiveBridge: Quickshell.env("NOTCHD_LOCAL_LIVE_BRIDGE")
    || Live.localPath(Qt.resolvedUrl("live/local-live.py"), Quickshell.shellDir)
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
      path: svc.moduleInfoPath(svc.moduleUrl(modelData))
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
      Component.onCompleted: if (path === "" && String(svc.moduleUrl(modelData)) !== "")
        console.warn(svc.logName + ": no settings file for module '" + modelData + "' at " + svc.moduleUrl(modelData))
    }
  }

  // ---------------------------------------------------------------- config
  // The parsed config, so hosts can read their own keys from it.
  property var config: ({})
  property int hoverOpenDelay: Config.DEFAULTS.hoverOpenDelay
  property int hoverCloseDelay: Config.DEFAULTS.hoverCloseDelay
  // Every notch, resolved (Config.notches), and their ids.
  property var notchSpecs: Config.notches({}, svc.defaultModules)
  property var notchIds: svc.notchSpecs.map(function(n) { return n.id })
  // The modules to load: every one some notch shows, and the standalone
  // ones switched on (Config.moduleLoadList).
  property var moduleIds: []
  // Read fresh: called as the module files finish loading, before
  // `standaloneIds` follows them (the OSD was left out at startup).
  function loadList(specs) {
    var standalone = svc.availableModuleIds.filter(function(id) { return svc.isStandalone(id) })
    var enabled = standalone.filter(function(id) { return svc.standaloneEnabled(id) })
    return Config.moduleLoadList(specs, standalone, enabled)
  }

  function sameList(a, b) { return JSON.stringify(a) === JSON.stringify(b) }
  function refreshModuleIds() {
    var modules = svc.loadList(svc.notchSpecs)
    if (!svc.sameList(modules, svc.moduleIds)) svc.moduleIds = modules
  }

  // Keys missing from the config fall back to their defaults, so deleting a
  // key in the file (or the settings window) resets it. Config.resolve and
  // Config.notches have the rules. The id and module lists are only
  // replaced when they change: a new list would make every notch and module
  // again, losing what they show.
  function applyConfig(cfg) {
    var r = Config.resolve(cfg)
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
    var ids = specs.map(function(n) { return n.id })
    if (!svc.sameList(ids, svc.notchIds)) svc.notchIds = ids
    var modules = svc.loadList(specs)
    if (!svc.sameList(modules, svc.moduleIds)) svc.moduleIds = modules
  }

  function loadConfig(text) {
    // Our own pending edit is newer than whatever the file says right now.
    if (saveTimer.running) return
    var parsed = Config.parse(text)
    if (parsed.ok) svc.applyConfig(parsed.config)
    else console.warn(svc.logName + ": ignoring " + svc.configPath + ": " + parsed.error)
  }

  FileView {
    id: configFile
    path: svc.configPath
    watchChanges: true
    printErrors: false
    onLoaded: { missingConfigRetry.stop(); svc.loadConfig(text()); svc.configResolved = true }
    onLoadFailed: { missingConfigRetry.start(); svc.configResolved = true }
    onFileChanged: reload()
    onSaveFailed: function(error) { console.warn(svc.logName + ": could not write " + svc.configPath + ": " + error) }
    // A file that didn't exist when the view loaded isn't watched yet; a
    // reload after the first save starts watching it.
    onSaved: reload()
  }

  // While the file is missing, look again now and then, so creating it by
  // hand is picked up without a restart.
  Timer {
    id: missingConfigRetry
    interval: 2000
    repeat: true
    onTriggered: configFile.reload()
  }

  // ------------------------------------------------------- config editing
  // Dotted keys reach into objects: "theme.foreground".
  function configValue(key) { return Config.valueAt(svc.config, key) }

  // Applies at once and writes the file shortly after, so dragging a slider
  // doesn't write on every step. `undefined` removes the key (back to its
  // default); emptied parent objects are removed too.
  function setConfigValue(key, value) {
    svc.replaceConfig(Config.withValue(svc.config, key, value))
  }

  // The whole config at once, applied and then written.
  function replaceConfig(cfg) {
    svc.applyConfig(cfg)
    saveTimer.restart()
  }

  Timer {
    id: saveTimer
    interval: 250
    onTriggered: {
      // The config dir may not exist yet on the first save.
      ensureConfigDir.command = ["mkdir", "-p", svc.configPath.replace(/\/[^\/]*$/, "")]
      ensureConfigDir.running = true
    }
  }

  Process {
    id: ensureConfigDir
    onExited: function(code) {
      if (code !== 0) console.warn(svc.logName + ": could not create the config dir for " + svc.configPath)
      configFile.setText(JSON.stringify(svc.config, null, 2) + "\n")
    }
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
    { id: "appearance", label: "Appearance", icon: 0xF0E0C } // nf-md-palette_outline
  ]
  // Then a sub-page per notch ("notch/<index>") and per module
  // ("modules/<id>"), for their own fields, reached from the Notches and
  // Modules pages.
  readonly property var settingsPages: Settings.mergePages(svc.corePages, svc.hostSettingsPages)
    .concat(svc.notches.map(function(n) { return { id: Settings.notchPageId(n.index), label: n.notchId } }),
            Settings.modulePages(svc.moduleCatalog))

  // The fields the settings window shows, built from these descriptors:
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
  readonly property var coreSettingsSchema: {
    var out = []
    var n = svc.settingsNotchObject
    var prefix = n && n.spec ? n.spec.prefix : ""
    function key(k) { return prefix + k }
    function inherited(k, fallback) {
      if (prefix === "") return fallback
      var v = Config.valueAt(svc.config, k)
      return v === undefined ? fallback : v
    }
    var several = svc.notches.length > 1
    // The picked notch's page, under Notches.
    var own = Settings.notchPageId(n ? n.index : 0)

    out.push({ page: "notch", key: "notches", type: "notches", label: "Notches",
               help: several ? "Each has its own modules and place; the gear opens its settings."
                             : "The gear opens the notch's settings. Add a notch to show other modules somewhere else, each with its own place." })
    if (prefix !== "")
      out.push({ page: own, key: key("id"), type: "string", label: "Name",
                 default: Config.defaultNotchId(n.index),
                 help: "What IPC calls it: openNotch, closeNotch and toggleNotch take it, so renaming it breaks a shortcut that names it. An agent opens by its own id instead: agent <id> toggle." })
    out.push({ page: own, section: "Modules", key: key("modules"), type: "modules", label: "Loaded modules",
               default: inherited("modules", svc.defaultModules),
               help: (several ? "What this notch shows. A module may show in several notches. Their order breaks ties in the expanded view."
                              : "Switched-on modules load. Their order breaks ties in the expanded view.")
                     + (svc.standaloneIds.length > 0 ? " Standalone modules, such as " + svc.moduleCatalog.filter(function(m) { return m.standalone })
                          .map(function(m) { return m.title }).join(", ") + ", have a place of their own: see the Modules page." : "") })
    out.push({ page: own, section: "Layout", key: key("reserveSpace"), type: "boolean",
               label: "Keep windows clear", default: inherited("reserveSpace", Config.DEFAULTS.reserveSpace),
               help: "Windows keep clear of the closed notch instead of going under it, along its whole edge: a sliver at rest, more while a module is active. Peeks and the open notch still cover them." })
    out.push({ page: own, section: "Layout", key: key("tabs"), type: "boolean",
               label: "Tabs", default: inherited("tabs", Config.DEFAULTS.tabs),
               help: "Open, the notch shows one module at a time, with a tab for each, instead of all of them one above the other; closed, every active module side by side. Opening the notch from a module (a shortcut, IPC) picks its tab. Only a notch with several modules has tabs." })
    out.push({ page: own, section: "Layout", key: key("idleIcons"), type: "boolean",
               label: "Icons for idle modules", default: inherited("idleIcons", Config.DEFAULTS.idleIcons),
               help: "A module with nothing to show closed, like a timer that isn't running, shows its icon in the closed notch. Off, it stays out until it has something." })
    out.push({ page: own, section: "Position", key: key("position.edge"), type: "choice", display: "cards",
               label: "Edge", default: inherited("position.edge", "top"),
               help: "The screen edge the notch hangs from. On a side edge, an active module without a vertical view shows a dot.",
               options: ["top", "bottom", "left", "right"].map(function(e) {
                 return { value: e, label: e.charAt(0).toUpperCase() + e.slice(1),
                          preview: Qt.resolvedUrl("settings/previews/Edge.qml") }
               }) })
    out.push({ page: own, section: "Position", key: key("position.offset"), type: "number", display: "percent",
               label: "Along the edge", default: inherited("position.offset", 0.5), min: 0, max: 1, step: 0.01,
               help: "Where on the edge: 0 % is its left (or top) end, 50 % the middle." })
    var screens = svc.screens
    if (screens.length > 1)
      out.push({ page: own, section: "Position", key: key("position.screen"), type: "choice",
                 label: "Screen", default: inherited("position.screen", ""),
                 help: "Where the notch stays. Following focus, it moves to the screen you work on. On a screen of its own, it stays there, peeks included; on a screen without a notch that shows it, a peek appears at the top. On every screen, each screen has its own copy, and a peek shows on the focused one.",
                 options: [{ value: "", label: "Follow focus" }, { value: "all", label: "Every screen" }].concat(screens.map(function(s) {
                   return { value: s.name, label: s.name }
                 })) })
    out.push({ page: own, section: "Position", key: key("draggable"), type: "boolean",
               label: "Drag to move", default: inherited("draggable", Config.DEFAULTS.draggable),
               help: "Hold the notch for a moment, then drag it to any edge of the screen; it snaps to the nearest one, and to the middle when close to it. A quick click only pins it." })
    if (svc.compositor && svc.compositor.supportsWorkspaces)
      out.push({ page: own, section: "Layout", key: key("workspaces"), type: "workspaces",
                 label: "Workspaces", default: inherited("workspaces", undefined),
                 help: "Where the closed notch shows; none picked means every workspace. Peeks and the open notch show on any." })
    out.push({ page: "general", section: "Hover", key: "hoverOpenDelay", type: "integer", label: "Open delay",
               help: several ? "How long the pointer rests on a notch before it expands. For every notch."
                             : "How long the pointer rests on the notch before it expands.",
               default: Config.DEFAULTS.hoverOpenDelay, min: 0, max: 1000, step: 10, unit: "ms" })
    out.push({ page: "general", section: "Hover", key: "hoverCloseDelay", type: "integer", label: "Close delay",
               help: several ? "How long it stays open after the pointer leaves. For every notch."
                             : "How long it stays open after the pointer leaves.",
               default: Config.DEFAULTS.hoverCloseDelay, min: 0, max: 2000, step: 10, unit: "ms" })
    out.push({ page: "modules", type: "moduleCatalog", part: "notch", label: "Notch modules",
               help: "Shown by notches: which ones a notch shows is set on its page, under Notches. The gear opens a module's own settings." })
    if (svc.moduleCatalog.some(function(m) { return Settings.moduleKind(m) === "agent" }))
      out.push({ page: "modules", type: "moduleCatalog", part: "agent", label: "Agents",
                 help: "Coding agents, each in its own terminal. Notches show them like any module; a notch of their own suits them best. The gear opens an agent's folder, command and size. A shortcut opens one with agent <id> toggle." })
    out.push({ page: "modules", type: "moduleCatalog", part: "terminal", label: "Terminals",
               help: "Commands of your own, each in a terminal: btop, a dev server, a shell. Add terminal makes one running your shell; its gear changes its command, folder and size. A notch shows it once the notch's page lists it. A shortcut opens one with terminal <id> toggle." })
    if (svc.standaloneIds.length > 0)
      out.push({ page: "modules", type: "moduleCatalog", part: "standalone", label: "Standalone modules",
                 help: "Each has a place of its own, over any notch that hangs there, and no notch shows it. Its page switches it on and places it." })
    return out
  }
  readonly property var settingsSchema: svc.coreSettingsSchema
    .concat(Settings.moduleFields(svc.moduleCatalog, Qt.resolvedUrl("settings/previews/Edge.qml")),
            svc.hostSettingsSchema || [])

  property bool settingsOpen: false
  // The page the window shows; kept here so it survives closing the window.
  property string settingsPage: "general"
  // The notch the window edits, by index; showing a notch's page picks it.
  property int settingsNotch: 0
  onSettingsPageChanged: {
    var index = Settings.notchIndexOfPage(svc.settingsPage)
    if (index !== -1) svc.settingsNotch = index
  }
  readonly property var settingsNotchObject: svc.notches.length > 0
    ? svc.notches[Math.max(0, Math.min(svc.settingsNotch, svc.notches.length - 1))] : null
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
  // (NotchSurface, Position.coveredByStandalone).
  Variants {
    id: standaloneVariants
    readonly property var owner: svc
    model: svc.configResolved && svc.moduleInfoReady ? svc.standaloneIds.filter(function(id) { return svc.moduleIds.indexOf(id) !== -1 }) : []
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
  // until it shows a peek where no notch with that module does
  // (State.needsVisitor, State.focusedPeek).
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

  // A module's peek goes to every notch that shows the module (on the copy
  // on the focused screen of one that has a copy per screen), and to the
  // visitor when none of them peeks on the focused screen. A module that
  // peeks on the focused screen only peeks once: in the first notch there
  // that shows it, else in the visitor (State.focusedPeek). A standalone
  // module's goes to its own notch, on the focused screen.
  function requestPeek(module, ms) {
    var own = module ? svc.standaloneNotch(module.moduleId) : null
    if (own) { own.requestPeek(module, ms); return }
    if (module && svc.isStandalone(module.moduleId)) return
    var places = []
    var owners = []
    for (var i = 0; i < svc.notches.length; i++) {
      if (!svc.notches[i].hasModule(module)) continue
      places.push(svc.notches[i].peekPlace())
      owners.push(svc.notches[i])
    }
    var focused = svc.compositor ? svc.compositor.focusedScreenName : ""
    if (module && module.peekOnFocusedScreen) {
      var at = State.focusedPeek(places, focused)
      if (at >= 0) { owners[at].requestPeek(module, ms); return }
      visitorNotch.requestPeek(module, ms)
      return
    }
    for (var j = 0; j < owners.length; j++) owners[j].requestPeek(module, ms)
    if (State.needsVisitor(places, focused)) visitorNotch.requestPeek(module, ms)
  }
  function dismissPeek(module) {
    for (var i = 0; i < svc.notches.length; i++) svc.notches[i].dismissPeek(module)
    visitorNotch.dismissPeek(module)
    for (var j = 0; j < svc.standaloneNotches.length; j++) svc.standaloneNotches[j].dismissPeek(module)
  }

  // A module you type into asks for the keyboard (a press on its field):
  // every copy open on it pins, so its window takes the keyboard
  // (State.keyboardFocus) and the pointer leaving doesn't close it.
  function requestKeyboard(module) {
    for (var i = 0; i < svc.copies.length; i++) {
      var copy = svc.copies[i]
      if (copy.mode === "expanded" && copy.shownModules.indexOf(module) !== -1) copy.open()
    }
  }

  // A module opens the first notch of the config that shows it, or closes
  // every one that does (NotchModule.openNotch / closeNotch).
  function openModule(module) {
    for (var i = 0; i < svc.notches.length; i++)
      if (svc.notches[i].hasModule(module)) { svc.notches[i].open(module); return true }
    return false
  }
  function closeModule(module) {
    for (var i = 0; i < svc.notches.length; i++)
      if (svc.notches[i].hasModule(module)) svc.notches[i].close()
  }

  // Without a notch named, open and toggle act on the first notch and close
  // closes them all.
  function open() { if (svc.notches.length > 0) svc.notches[0].open() }
  function close() { for (var i = 0; i < svc.notches.length; i++) svc.notches[i].close() }
  function toggle() { if (svc.notches.length > 0) svc.notches[0].toggle() }

  // ------------------------------------------------------------ navigation
  // A shortcut (IPC `navigate`) highlights every notch with its number and
  // the keyboard picks one (Navigate.js): its copy on the focused screen
  // opens as a preview, Tab walks its tabs, Enter pins it open (the others
  // close) and Escape leaves everything as it was. Keys also open the
  // selected notch's settings, remove it (asked twice) and move it
  // (`navMode` "move": the edge, along it, the screen; Escape puts it
  // back). NotchSurface's navigation window holds the keyboard meanwhile
  // and hands keys to navKey().
  property bool navigating: false
  // "select", "move" or "remove" (Navigate.step).
  property string navMode: "select"
  // Where the notch being moved was: { notch, position, workspaces }, its
  // own config keys as they were, put back on Escape.
  property var navMoveFrom: null
  // The selected notch, by index in `notches`.
  property int navSelected: -1
  // Each copy's tab as navigation started, given back on Escape.
  property var navTabs: []
  readonly property var navNotch: svc.navigating && svc.navSelected >= 0 && svc.navSelected < svc.notches.length
    ? svc.notches[svc.navSelected] : null

  function startNavigation() {
    if (svc.notches.length === 0) return "no notches"
    var focused = svc.compositor ? svc.compositor.focusedScreenName : ""
    svc.navSelected = Navigate.start(svc.notches.map(function(n) {
      return {
        open: n.copies.some(function(c) { return c.isOpen }),
        screens: n.copies.map(function(c) { return c.screenName })
      }
    }), focused)
    var tabs = []
    for (var i = 0; i < svc.notches.length; i++)
      for (var j = 0; j < svc.notches[i].copies.length; j++)
        tabs.push({ copy: svc.notches[i].copies[j], tab: svc.notches[i].copies[j].tab })
    svc.navTabs = tabs
    svc.navMode = "select"
    svc.navMoveFrom = null
    svc.navigating = true
    return "ok"
  }
  // Just after navigation ends: as its window goes, the compositor hands
  // focus back to the window you were in, taking it from an agent Enter
  // just opened; that agent asks again instead of closing
  // (State.keyboardChange).
  property bool navSettling: false
  Timer { id: navSettleTimer; interval: 600; onTriggered: svc.navSettling = false }
  // `restore`: give each copy back the tab it had (Escape), and a notch
  // being moved its place.
  function endNavigation(restore) {
    if (!svc.navigating) return
    if (restore) svc.undoMove()
    svc.navMoveFrom = null
    svc.navMode = "select"
    svc.navSettling = true
    navSettleTimer.restart()
    svc.navigating = false
    if (restore)
      for (var i = 0; i < svc.navTabs.length; i++)
        if (svc.navTabs[i].copy) svc.navTabs[i].copy.showTab(svc.navTabs[i].tab)
    svc.navTabs = []
  }
  function navigate() {
    if (svc.navigating) { svc.endNavigation(true); return "ok" }
    return svc.startNavigation()
  }
  // The selected notch's copy that previews.
  function navCopy() { return svc.navNotch ? svc.navNotch.focusedCopy() : null }
  // A key (Navigate.keyName) while navigating.
  function navKey(key) {
    if (!svc.navigating) return
    var copy = svc.navCopy()
    var tabs = copy ? copy.tabModules.map(function(m) { return m.moduleId }) : []
    var shown = copy && copy.shownModules.length > 0 ? copy.shownModules[0].moduleId : ""
    var next = Navigate.step({ mode: svc.navMode, selected: svc.navSelected, count: svc.notches.length,
                               tabs: tabs, tab: shown }, key)
    var notch = svc.navNotch
    if (next.action === "leave") { svc.endNavigation(true); return }
    if (next.action === "open") {
      svc.endNavigation(false)
      for (var i = 0; i < svc.notches.length; i++) if (svc.notches[i] !== notch) svc.notches[i].close()
      if (notch) notch.open()
      return
    }
    if (next.action === "settings") {
      var index = svc.navSelected
      svc.endNavigation(true)
      svc.openSettings(index)
      return
    }
    if (next.action === "remove" && notch) {
      // Its copies go with it: no tab to give back on Escape.
      svc.navTabs = svc.navTabs.filter(function(e) { return notch.copies.indexOf(e.copy) === -1 })
      svc.removeNotch(svc.navSelected)
    }
    if (next.action === "moveStart" && notch) {
      svc.navMoveFrom = {
        notch: notch,
        position: svc.plainValue(notch.configKey("position")),
        workspaces: svc.plainValue(notch.configKey("workspaces"))
      }
    }
    if (next.action === "move" && notch) {
      notch.moveTo(Navigate.moveTarget(notch.placement, next.move,
        svc.screens.map(function(s) { return s.name }), copy ? copy.screenName : ""))
    }
    if (next.action === "moveDone") svc.navMoveFrom = null
    if (next.action === "moveCancel") { svc.undoMove(); svc.navMoveFrom = null }
    svc.navMode = next.mode
    svc.navSelected = next.selected
    if (next.tab !== "" && next.tab !== shown && copy) copy.showTab(next.tab)
  }
  // A config key's value as it is, copied (undefined when it isn't set).
  function plainValue(key) {
    var v = svc.configValue(key)
    return v === undefined ? undefined : JSON.parse(JSON.stringify(v))
  }
  // A notch being moved goes back where it was, its workspaces too.
  function undoMove() {
    var from = svc.navMoveFrom
    if (!from || !from.notch) return
    from.notch.setValue("workspaces", from.workspaces)
    from.notch.setValue("position", from.position)
  }

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
      source: svc.moduleUrl(modelData)
      onLoaded: {
        var module = item as NotchModule
        if (!module) { console.warn(svc.logName + ": module '" + modelData + "' is not a NotchModule"); return }
        module.notch = svc
        if (module.moduleId === "") module.moduleId = modelData
        if (module.title === "") {
          var id = module.moduleId
          module.title = Qt.binding(function() { return Settings.moduleTitle(id, svc.moduleInfo[id]) })
        }
        svc.rebuildModules()
      }
      onStatusChanged: if (status === Loader.Error) console.warn(svc.logName + ": module '" + modelData + "' failed to load")
      Component.onCompleted: if (String(source) === "") console.warn(svc.logName + ": unknown module '" + modelData + "'")
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
    function set(key: string, value: string): string {
      if (!key) return "usage: set <key> <json value>"
      if (value === "") { svc.setConfigValue(key, undefined); return "ok" }
      try { svc.setConfigValue(key, JSON.parse(value)) } catch (e) { return "not JSON: " + value }
      return "ok"
    }
    // Generic activities, for scripts: see modules/activities.qml.
    function push(json: string): string {
      var m = svc.moduleById("activities")
      return m ? m.push(json) : "activities module not loaded"
    }
    function clear(id: string): string {
      var m = svc.moduleById("activities")
      return m ? m.clear(id) : "activities module not loaded"
    }
    // The timer module: "25m", "3m Tea", "pomodoro", "toggle", "+5m",
    // "skip", "stop"; see modules/timer.qml.
    function timer(command: string): string {
      var m = svc.moduleById("timer")
      return m ? m.command(command) : "timer module not loaded"
    }
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
