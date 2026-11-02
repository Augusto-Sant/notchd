import QtQuick
import Quickshell
import Quickshell.Io
import "core"
import "core/client"
import "core/Navigate.js" as Navigate

// The shadow test (docs/architecture.md, Phase 2): the REAL daemon and the QML
// service are driven with the same inputs (config, screens, module facts,
// pointer, keys, tabs, `set`) and what each decides is compared, field by
// field. NotchService still decides what the user sees; the daemon's `state`
// is computed in shadow. A disagreement is a bug in one of them.
//
// What is compared, at each `w.snap(label)`:
//   1. the IPC `state` JSON: the QML handler's reply and the daemon's reply to
//      the same `ipc state`, which are specified to be one string;
//   2. the daemon's `state` entity against the same facts read off the QML
//      service (mode, pinned, tab, peek, shown, covered, needsWindow,
//      keyboard, navigation).
// Only order is normalised (modules by id, copies by screen) and numbers
// compare within 1e-9. Revs and peek deadlines are not compared (a clock).
//
// A scenario that disagrees lists `known: [{ path, reason }]`: differences
// under those paths are printed as NOTE and do not fail it, but an entry that
// matches nothing fails it (the bug was fixed: delete the entry). Anything
// else fails with the diff.
//
// scripts/qml-test.sh runs this as one more shard: the daemon binary in
// NOTCHD_QMLTEST_DAEMON, started with NOTCHD_TEST_HOOKS=1 (see
// native/notchd-daemon/src/testhooks.rs, the way module facts and peeks
// reach it) on a socket in the shard's own scratch dir, one per scenario and
// stopped when it ends. Same harness conventions as shell.qml.
ShellRoot {
  id: root

  readonly property string dir: Quickshell.env("NOTCHD_QMLTEST_DIR")
  readonly property string socketDir: Quickshell.env("NOTCHD_QMLTEST_SOCKETS")
  readonly property string daemonBin: Quickshell.env("NOTCHD_QMLTEST_DAEMON")

  // ------------------------------------------------------------- harness
  property var cases: []
  property int index: -1
  property string current: ""
  property bool failed: false
  property real started: 0
  property var world: null

  function check(cond, why) {
    if (!cond && !root.failed) {
      root.failed = true
      console.log("FAIL " + root.current + ": " + why)
    }
    return cond
  }
  function done() {
    if (root.world) { root.world.destroyWorld(); root.world = null }
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
    try { root.runScenario(root.cases[root.index]) }
    catch (e) { root.check(false, "threw " + e); root.done() }
  }

  function steps(list, last) {
    if (list.length === 0) { last(); return }
    var step = list[0]
    stepTimer.interval = Math.max(1, step[0])
    stepTimer.action = function() { step[1](); root.steps(list.slice(1), last) }
    stepTimer.restart()
  }
  Timer {
    id: stepTimer
    property var action: null
    onTriggered: if (action) action()
  }

  // Waits until pred() holds (every 20 ms, at most ms), then runs fn; on a
  // timeout fn still runs and its checks say what is missing.
  function until(pred, fn, ms) {
    untilTimer.left = Math.ceil((ms || 5000) / untilTimer.interval)
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

  // ---------------------------------------------------------------- diff
  // Every difference between two parsed JSON values, as { path, qml, daemon }.
  function diff(a, b, path, out) {
    if (a === b) return
    if (typeof a === "number" && typeof b === "number" && Math.abs(a - b) < 1e-9) return
    var arrays = Array.isArray(a) && Array.isArray(b)
    if (arrays) {
      if (a.length !== b.length) { out.push({ path: path, qml: a, daemon: b }); return }
      for (var i = 0; i < a.length; i++) root.diff(a[i], b[i], path + "[" + i + "]", out)
      return
    }
    if (a !== null && b !== null && typeof a === "object" && typeof b === "object" && !Array.isArray(a) && !Array.isArray(b)) {
      var keys = Object.keys(a)
      Object.keys(b).forEach(function(k) { if (keys.indexOf(k) < 0) keys.push(k) })
      keys.forEach(function(k) { root.diff(a[k], b[k], path === "" ? k : path + "." + k, out) })
      return
    }
    out.push({ path: path, qml: a, daemon: b })
  }
  function shorten(v) {
    var s = v === undefined ? "(missing)" : JSON.stringify(v)
    return s.length > 160 ? s.slice(0, 157) + "..." : s
  }

  // ------------------------------------------------------------ the keys
  // A wire key name as the UI sends it, and the Qt key code and modifiers
  // the QML window would have got for it (Navigate.keyName reads those).
  function qtKey(name) {
    var table = { Escape: Qt.Key_Escape, Tab: Qt.Key_Tab, Backtab: Qt.Key_Backtab, Return: Qt.Key_Return, Left: Qt.Key_Left, Right: Qt.Key_Right, Up: Qt.Key_Up, Down: Qt.Key_Down }
    return table[name] !== undefined ? table[name] : name.toUpperCase().charCodeAt(0)
  }
  function qtMods(mods) {
    var flags = 0
    var table = { shift: Qt.ShiftModifier, ctrl: Qt.ControlModifier, alt: Qt.AltModifier, meta: Qt.MetaModifier }
    mods.forEach(function(m) { flags |= table[m] })
    return flags
  }

  // --------------------------------------------------------------- world
  // Everything one scenario runs on: the QML service with its compositor, and
  // the daemon with its client, both given every input.
  Component {
    id: worldComponent
    Scope {
      id: w
      property string tag: ""
      readonly property string sock: root.socketDir + "/" + w.tag + ".sock"
      readonly property string cfgFile: root.socketDir + "/" + w.tag + ".json"
      readonly property alias svc: service
      // The service's own client: it starts the daemon (the launcher's path,
      // `daemon --ensure`, with the test hooks on) and is its UI.
      readonly property alias client: service.client
      // The inputs, kept to send both sides the same.
      property var screenSpec: ({ names: ["A", "B"], focused: "B", fullscreen: [], workspaces: ({}), known: [], supports: false })
      property var facts: ({})
      property int pending: 0
      property var diffs: []
      property var labels: []
      property var notes: []

      QtObject {
        id: compositor
        property string focusedScreenName: "B"
        property bool supportsWorkspaces: false
        property var knownWorkspaces: []
        property var fullscreenOn: ({})
        property var workspaces: ({})
        function hasFullscreen(screen) { return !!compositor.fullscreenOn[screen.name] }
        function activeWorkspace(screen) { return compositor.workspaces[screen.name] || null }
      }

      NotchService {
        id: service
        ipcTarget: "shadow-" + w.tag
        // The daemon (with the test hooks on) is started by the service itself.
        daemonCommand: ["env", "NOTCHD_TEST_HOOKS=1", root.daemonBin, "--socket", w.sock, "--config", w.cfgFile, "daemon", "--ensure"]
        daemonDownDelay: 60000
        compositor: compositor
        screens: [{ name: "A" }, { name: "B" }]
        defaultModules: []
        hostModules: ({
          a: Qt.resolvedUrl("fixtures/shadowmod.qml"),
          b: Qt.resolvedUrl("fixtures/shadowmod.qml"),
          c: Qt.resolvedUrl("fixtures/shadowmod.qml"),
          solo: Qt.resolvedUrl("fixtures/shadowsolo.qml")
        })
      }


      function destroyWorld() {
        client.ipc("shutdown", [])
        w.destroy()
      }

      // ----- inputs, to both sides
      function sendScreens() {
        var s = w.screenSpec
        var workspaces = {}
        Object.keys(s.workspaces).forEach(function(n) { workspaces[n] = { id: s.workspaces[n], name: s.workspaces[n] } })
        client.send({
          t: "screens",
          screens: s.names.map(function(n, i) { return { name: n, x: i * 1920, y: 0, w: 1920, h: 1080, scale: 1 } }),
          focused: s.focused, fullscreen: s.fullscreen, workspaces: workspaces, known: s.known,
          supportsWorkspaces: s.supports, barEdge: "top"
        })
      }
      // Changes the screens, workspaces and focus: `patch` has any of names,
      // focused, fullscreen, workspaces ({ screen: "id" }), known, supports.
      function screens(patch) {
        var s = Object.assign({}, w.screenSpec, patch)
        w.screenSpec = s
        var ws = {}
        Object.keys(s.workspaces).forEach(function(n) { ws[n] = { id: s.workspaces[n], name: s.workspaces[n] } })
        var fs = {}
        s.fullscreen.forEach(function(n) { fs[n] = true })
        compositor.supportsWorkspaces = s.supports
        compositor.knownWorkspaces = s.known
        compositor.fullscreenOn = fs
        compositor.workspaces = ws
        compositor.focusedScreenName = s.focused
        service.screens = s.names.map(function(n) { return { name: n } })
        w.sendScreens()
      }

      // A module's facts, merged into what it had, on both sides.
      function module(id, patch) {
        var f = Object.assign({ active: true, priority: 0, urgentPeek: false, attention: false, peekOnFocusedScreen: false,
                                 icon: false, hasCompact: true, hasPeek: true, expanded: true, standalone: false }, w.facts[id] || {}, patch)
        var all = Object.assign({}, w.facts)
        all[id] = f
        w.facts = all
        var m = service.moduleById(id)
        if (m) {
          m.active = f.active; m.priority = f.priority; m.urgentPeek = f.urgentPeek; m.attention = f.attention
          m.peekOnFocusedScreen = f.peekOnFocusedScreen; m.icon = f.icon ? "I" : ""
          if (id !== "solo") { m.hasCompact = f.hasCompact; m.hasPeek = f.hasPeek }
        }
        client.ipc("test-module", [JSON.stringify({
          id: id, active: f.active, priority: f.priority, urgentPeek: f.urgentPeek, attention: f.attention,
          peekOnFocusedScreen: f.peekOnFocusedScreen, icon: f.icon, compact: id === "solo" ? false : f.hasCompact,
          peek: id === "solo" ? true : f.hasPeek, expanded: true, standalone: f.standalone
        })])
      }
      function peek(id, ms) {
        service.moduleById(id).requestPeek(ms)
        client.ipc("test-peek", [id, String(ms === undefined ? 2500 : ms)])
      }
      function dismiss(id) {
        service.dismissPeek(id === "" ? null : service.moduleById(id))
        client.ipc("test-dismiss", [id])
      }

      function qmlCopy(notch, screen) {
        var n = service.notchById(notch)
        return n ? n.copyOn(screen) : null
      }
      // The pointer entering or leaving a notch's copy on `screen`.
      function pointer(notch, screen, ev) {
        var copy = w.qmlCopy(notch, screen)
        if (copy) copy.setPointer("notch", ev === "enter", ev === "enter" ? screen : "")
        client.pointer(notch, screen, ev)
      }
      // A click on the copy (pins, or unpins).
      function press(notch, screen) {
        var copy = w.qmlCopy(notch, screen)
        if (copy) copy.togglePinned(screen)
        client.pointer(notch, screen, "press")
      }
      function tab(notch, screen, moduleId) {
        var copy = w.qmlCopy(notch, screen)
        if (copy) copy.showTab(moduleId)
        client.send({ t: "tab", notch: notch, screen: screen, module: moduleId })
      }
      // A key while navigating.
      function key(name, mods) {
        mods = mods || []
        service.navKey(Navigate.keyName(root.qtKey(name), root.qtMods(mods)))
        client.key("", "", name, mods, "")
      }
      function navigate() {
        service.navigate()
        client.ipc("navigate", [])
      }
      // An IPC method of the QML service's handler the daemon also has, called
      // as `qs ipc call` would.
      function handler(method) {
        for (var i = 0; i < service.data.length; i++) {
          var o = service.data[i]
          if (o && o.target === service.ipcTarget && typeof o[method] === "function") { o[method](); return }
        }
      }
      function ipc(method, args) {
        if (method === "open") service.open()
        else if (method === "close") service.close()
        else if (method === "toggle") service.toggle()
        else if (method === "openNotch") service.notchById(args[0]).open()
        else if (method === "closeNotch") service.notchById(args[0]).close()
        else if (method === "recenter") w.handler("recenter")
        client.ipc(method, args)
      }
      function set(key, value) {
        service.setConfigValue(key, value)
        service.flushConfig()
      }
      // A notch dropped at `place` ({ edge, offset, screen }).
      function drop(notch, screen, place) {
        service.notchById(notch).moveTo(place)
        client.send({ t: "drag", notch: notch, screen: screen, ev: "drop", position: place })
      }

      // ----- the comparison
      // What the QML service says of what the entity holds.
      function qmlEntity() {
        var peekOf = function(c) { return c.peekModule ? c.peekModule.moduleId : null }
        var notches = {}
        service.notches.forEach(function(n) {
          var copies = {}
          n.copies.forEach(function(c) {
            copies[c.screenName] = {
              mode: c.mode, pinned: c.pinned, tab: c.tab, peek: peekOf(c),
              shown: c.shownModules.map(function(m) { return m.moduleId }),
              covered: c.offWorkspace, needsWindow: c.needsWindow, keyboard: c.wantsKeyboard
            }
          })
          notches[n.notchId] = {
            edge: n.edge, offset: n.placement.offset, screen: n.placement.screen, draggable: n.canDrag, tabs: n.tabs,
            modules: n.modules.map(function(m) { return m.moduleId }), copies: copies
          }
        })
        var standalone = {}
        service.standaloneNotches.forEach(function(n) {
          standalone[n.standaloneId] = n.copies.filter(function(c) { return c.mode === "peek" || c.mode === "expanded" })
            .map(function(c) { return { screen: c.screenName, mode: c.mode, peek: peekOf(c) } })
        })
        var nav = null
        if (service.navigating) {
          var copy = service.navCopy()
          nav = { mode: service.navMode, selected: service.navSelected, tab: copy ? copy.tab : "" }
        }
        return { navigation: nav, settingsOpen: service.settingsOpen, notches: notches, standalone: standalone }
      }
      function daemonEntity(st) {
        var notches = {}
        st.notches.forEach(function(n) {
          var copies = {}
          n.copies.forEach(function(c) {
            copies[c.screen] = { mode: c.mode, pinned: c.pinned, tab: c.tab, peek: c.peek ? c.peek.module : null, shown: c.shown,
                                 covered: c.covered, needsWindow: c.needsWindow, keyboard: c.keyboard }
          })
          notches[n.id] = { edge: n.edge, offset: n.offset, screen: n.screen, draggable: n.draggable, tabs: n.tabs, modules: n.modules, copies: copies }
        })
        var standalone = {}
        st.standalone.forEach(function(s) {
          standalone[s.id] = s.copies.filter(function(c) { return c.mode === "peek" || c.mode === "expanded" })
            .map(function(c) { return { screen: c.screen, mode: c.mode, peek: c.peek ? c.peek.module : null } })
        })
        return { navigation: st.navigation, settingsOpen: st.settingsOpen, notches: notches, standalone: standalone }
      }
      function qmlIpcState() {
        for (var i = 0; i < service.data.length; i++) {
          var o = service.data[i]
          if (o && o.target === service.ipcTarget && typeof o.state === "function") return JSON.parse(o.state())
        }
        return null
      }
      function sortedState(s) {
        if (!s) return s
        s.modules.sort(function(a, b) { return a.id < b.id ? -1 : a.id > b.id ? 1 : 0 })
        var byScreen = function(a, b) { return a.screen < b.screen ? -1 : a.screen > b.screen ? 1 : 0 }
        s.notches.forEach(function(n) { n.copies.sort(byScreen) })
        s.standalone.forEach(function(n) { n.copies.sort(byScreen) })
        s.visitor.sort(byScreen)
        return s
      }

      // Compares the two sides now: the QML's is read at once, the daemon's
      // when its reply comes. The differences go to `diffs` under `label`.
      function snap(label) {
        var qml = { ipc: w.sortedState(w.qmlIpcState()), entity: w.qmlEntity() }
        w.pending++
        var asked = client.ipc("state", [], function(reply) {
          w.pending--
          if (reply.t !== "ok") { w.add(label, [{ path: "(ipc state)", qml: "ok", daemon: JSON.stringify(reply) }]); return }
          var out = []
          root.diff(qml.ipc, w.sortedState(JSON.parse(reply.result)), "ipc", out)
          root.diff(qml.entity, w.daemonEntity(client.state), "entity", out)
          if (Quickshell.env("NOTCHD_QMLTEST_TRACE") === "1") console.log("NOTE trace [" + root.current + "] " + label + ": " + JSON.stringify(qml.ipc.notches.map(function(n) { return n.id + ":" + n.copies.map(function(c) { return c.screen + "=" + c.mode + (c.peek ? "(" + c.peek + ")" : "") }).join(",") }))
                      + " solo=" + JSON.stringify(qml.ipc.standalone) + " visitor=" + JSON.stringify(qml.ipc.visitor) + " nav=" + JSON.stringify(qml.ipc.navigation))
          w.add(label, out)
        })
        if (asked < 0) { w.pending--; w.add(label, [{ path: "(connection)", qml: "up", daemon: "down" }]) }
        w.labels = w.labels.concat([label])
      }
      function add(label, list) {
        w.diffs = w.diffs.concat(list.map(function(d) { return { label: label, path: d.path, qml: d.qml, daemon: d.daemon } }))
      }
      function note(text) { w.notes = w.notes.concat([text]) }
    }
  }

  // ------------------------------------------------------------ scenarios
  // A scenario: `config` (the same for both), `screens` (a patch of the
  // defaults: A and B, focus on B, no workspaces), `modules` ({ id: facts }),
  // `steps(w)` the [ms, fn] list, and `known`.
  function runScenario(sc) {
    var w = worldComponent.createObject(root, { tag: "s" + root.index })
    root.world = w
    // The standalone fixture loads whenever it is switched on (by default).
    var mods = Object.assign({ solo: {} }, sc.modules || {})
    root.until(function() { return w.client.ready }, function() {
      if (!root.check(w.client.ready, "the daemon did not come up")) { root.done(); return }
      // The config goes through the service to the daemon, the file's one authority.
      var cfg = Object.assign({ hoverOpenDelay: 100, hoverCloseDelay: 150 }, sc.config)
      w.svc.replaceConfig(cfg)
      w.svc.flushConfig()
      w.screens(sc.screens || {})
      root.until(function() { return w.svc.configResolved && w.svc.moduleIds.length > 0 && w.svc.modules.length === w.svc.moduleIds.length && w.svc.notches.length > 0 },
        function() {
          if (!root.check(w.svc.modules.length === w.svc.moduleIds.length, "the service loaded " + w.svc.modules.length + " of " + w.svc.moduleIds.length + " modules")) { root.done(); return }
          // The facts of the modules the service loaded, to both.
          w.svc.moduleIds.forEach(function(id) { w.module(id, Object.assign({ standalone: id === "solo" }, mods[id] || {})) })
          // The copies, and the daemon's replies, settle.
          root.steps([[250, function() {}]].concat(sc.steps(w)), function() {
            root.until(function() { return w.pending === 0 }, function() { root.report(sc, w) }, 2000)
          })
        })
    })
  }

  function matches(d, k) { return d.path.indexOf(k.path) === 0 && (k.label === undefined || d.label === k.label) }
  function report(sc, w) {
    var known = sc.known || []
    var seen = known.map(function() { return false })
    var unexpected = []
    w.diffs.forEach(function(d) {
      var hit = false
      known.forEach(function(k, i) { if (root.matches(d, k)) { seen[i] = true; hit = true } })
      if (!hit) unexpected.push(d)
    })
    root.check(w.pending === 0, "a daemon reply never came")
    root.check(w.labels.length > 0, "the scenario compared nothing")
    // Each known difference, once, with what each side said.
    known.forEach(function(k, i) {
      if (!seen[i]) { root.check(false, "the known disagreement at '" + k.path + "' (" + k.reason + ") is gone: delete it from the scenario"); return }
      var first = w.diffs.filter(function(d) { return root.matches(d, k) })[0]
      console.log("NOTE known disagreement [" + root.current + "] " + first.path + " at '" + first.label + "': qml " + root.shorten(first.qml)
                  + ", daemon " + root.shorten(first.daemon) + " (" + k.reason + ")")
    })
    if (unexpected.length > 0) {
      var lines = unexpected.slice(0, 12).map(function(d) {
        return "  [" + d.label + "] " + d.path + ": qml " + root.shorten(d.qml) + ", daemon " + root.shorten(d.daemon)
      })
      // One line each: the runner prints the lines that say FAIL or NOTE.
      lines.forEach(function(l) { console.log("NOTE" + l) })
      root.check(false, unexpected.length + " disagreement(s) between the QML and the daemon (listed above)")
    }
    root.done()
  }

  // A mid-delay and an after-delay wait, for the hover delays of the config
  // (100 and 150 ms) with margin for the timers' slack.
  readonly property int beforeOpen: 40
  readonly property int afterOpen: 260
  readonly property int beforeClose: 40
  readonly property int afterClose: 330

  readonly property var oneNotch: ({ notches: [{ id: "main", modules: ["a", "b"], position: { screen: "all" } }] })
  readonly property var twoNotches: ({
    notches: [{ id: "main", modules: ["a", "b"], position: { screen: "all" } },
              { id: "side", modules: ["c"], position: { edge: "left" } }]
  })
  readonly property var facts: ({ a: {}, b: {} })

  // Why the known disagreements disagree (see the report of the task that
  // wrote this test: which side is wrong, and where).

  Component.onCompleted: {
    var all = [
      { name: "shadow: idle, and nothing active", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("idle") }],
        [0, function() { w.module("a", { active: false }); w.module("b", { active: false }) }],
        [150, function() { w.snap("nothing active: rest") }],
        [0, function() { w.module("b", { active: true, hasCompact: false, icon: true }) }],
        [150, function() { w.snap("b has only an icon") }]
      ] } },

      { name: "shadow: hover opens and closes after their delays", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.beforeOpen, function() { w.snap("entered, before the open delay") }],
        [root.afterOpen, function() { w.snap("open") }],
        [0, function() { w.pointer("main", "B", "leave") }],
        [root.beforeClose, function() { w.snap("left, before the close delay") }],
        [root.afterClose, function() { w.snap("closed again") }],
        // Leaving before it opened forgets the screen and never opens.
        [0, function() { w.pointer("main", "A", "enter") }],
        [30, function() { w.pointer("main", "A", "leave") }],
        [root.afterOpen, function() { w.snap("a pass over the notch opens nothing") }]
      ] } },

      { name: "shadow: a peek shows on the focused screen and ends", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.peek("a", 400) }],
        [80, function() { w.snap("a peeks") }],
        [0, function() { w.screens({ focused: "A" }) }],
        [500, function() { w.snap("the peek ended") }],
        [0, function() { w.peek("b", 400) }],
        [80, function() { w.snap("b peeks on the other screen") }],
        [0, function() { w.dismiss("b") }],
        [80, function() { w.snap("dismissed") }]
      ] } },

      { name: "shadow: two competing peeks, the higher priority wins", config: root.oneNotch,
        modules: { a: { priority: 1 }, b: { priority: 5 }, c: {} }, steps: function(w) { return [
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peeks") }],
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("b outranks a") }],
        [0, function() { w.dismiss("b"); w.peek("a", 1500) }],
        [80, function() { w.snap("a again after b went") }],
        [0, function() { w.dismiss(""); }],
        [0, function() { w.module("a", { priority: 9 }); w.peek("b", 1500) }],
        [80, function() { w.snap("b under a's higher priority") }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a over b") }]
      ] } },

      { name: "shadow: peeks on the focused screen only, and only where the module is shown", config: root.twoNotches,
        modules: { a: { peekOnFocusedScreen: true }, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peeks on B") }],
        [0, function() { w.dismiss("a"); w.screens({ focused: "A" }); w.peek("c", 1500) }],
        [80, function() { w.snap("c, only on the side notch, peeks") }],
        [0, function() { w.dismiss("c"); w.module("a", { peekOnFocusedScreen: false }); w.peek("b", 1500) }],
        [80, function() { w.snap("b peeks on A") }]
      ] } },

      { name: "shadow: an urgent peek over a hovered notch, a plain one under it", config: root.oneNotch,
        modules: { a: {}, b: { urgentPeek: true } }, steps: function(w) { return [
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.afterOpen, function() { w.snap("hovered open") }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a plain peek does not show over it") }],
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("b urgent peek shows over it") }],
        [0, function() { w.pointer("main", "B", "leave") }],
        [root.afterClose, function() { w.snap("left") }],
        // A peek then the pointer comes: a plain peek is dismissed, an urgent one stays.
        [0, function() { w.dismiss(""); w.peek("a", 1500) }],
        [60, function() { w.snap("a peeks closed") }],
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.afterOpen, function() { w.snap("hovering dismissed a's peek") }]
      ] } },

      { name: "shadow: pinned by a click, by open, close and toggle", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.press("main", "B") }],
        [60, function() { w.snap("clicked: pinned") }],
        [0, function() { w.pointer("main", "B", "enter"); w.pointer("main", "B", "leave") }],
        [root.afterClose, function() { w.snap("pinned stays open after the pointer left") }],
        [0, function() { w.peek("a", 1500) }],
        [60, function() { w.snap("a peek does not replace a pinned notch") }],
        [0, function() { w.press("main", "B") }],
        [60, function() { w.snap("clicked again: unpinned") }],
        [0, function() { w.ipc("open", []) }],
        [60, function() { w.snap("ipc open") }],
        [0, function() { w.ipc("close", []) }],
        [60, function() { w.snap("ipc close") }],
        [0, function() { w.ipc("toggle", []) }],
        [60, function() { w.snap("ipc toggle") }],
        [0, function() { w.ipc("closeNotch", ["main"]) }],
        [60, function() { w.snap("ipc closeNotch") }]
      ] } },

      { name: "shadow: tabs pick the module the open notch shows", config: { notches: [{ id: "main", modules: ["a", "b"], tabs: true, position: { screen: "all" } }] },
        modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("tabs closed") }],
        [0, function() { w.press("main", "B") }],
        [60, function() { w.snap("open, first tab") }],
        [0, function() { w.tab("main", "B", "b") }],
        [60, function() { w.snap("tab b") }],
        [0, function() { w.module("b", { active: false }) }],
        [60, function() { w.snap("the shown tab went inactive") }],
        [0, function() { w.tab("main", "B", "") }],
        [60, function() { w.snap("no tab picked") }]
      ] } },

      { name: "shadow: navigation selects, walks tabs and leaves", config: { notches: [
          { id: "main", modules: ["a", "b"], tabs: true, position: { screen: "all" } },
          { id: "side", modules: ["c"], position: { edge: "left" } }] },
        modules: { a: {}, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.navigate() }],
        [80, function() { w.snap("navigating, the first notch previews") }],
        [0, function() { w.key("Tab") }],
        [60, function() { w.snap("Tab: the next tab") }],
        [0, function() { w.key("Tab", ["shift"]) }],
        [60, function() { w.snap("Shift+Tab: back") }],
        [0, function() { w.key("Right") }],
        [60, function() { w.snap("Right: the second notch") }],
        [0, function() { w.key("Right") }],
        [60, function() { w.snap("Right: wraps") }],
        [0, function() { w.key("2") }],
        [60, function() { w.snap("2: the second notch") }],
        [0, function() { w.key("Escape") }],
        [60, function() { w.snap("Escape: left") }],
        [0, function() { w.navigate() }],
        [0, function() { w.key("Right") }],
        [0, function() { w.key("Return") }],
        [80, function() { w.snap("Enter: the second notch pinned") }]
      ] } },

      { name: "shadow: navigation removes a notch (asked twice) and moves it", config: { notches: [
          { id: "main", modules: ["a", "b"], position: { screen: "all" } },
          { id: "side", modules: ["c"], position: { edge: "left" } },
          { id: "extra", modules: [], position: { edge: "bottom" } }] },
        modules: { a: {}, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.navigate(); w.key("3") }],
        [60, function() { w.snap("the third selected") }],
        [0, function() { w.key("x") }],
        [60, function() { w.snap("x asks first") }],
        [0, function() { w.key("Escape") }],
        [60, function() { w.snap("Escape keeps it") }],
        [0, function() { w.key("m") }],
        [60, function() { w.snap("move mode") }],
        [0, function() { w.key("Left") }],
        [60, function() { w.snap("moved to the left edge") }],
        [0, function() { w.key("Right", ["shift"]) }],
        [60, function() { w.snap("slid along it") }],
        [0, function() { w.key("Tab") }],
        [60, function() { w.snap("on the next screen") }],
        [0, function() { w.key("Escape") }],
        [60, function() { w.snap("Escape puts it back") }],
        [0, function() { w.key("x"); w.key("x") }],
        [100, function() { w.snap("x twice removed it") }]
      ] } },

      { name: "shadow: set changes position, edge and delays", config: root.twoNotches, modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("before") }],
        [0, function() { w.set("notches.1.position", { edge: "right", offset: 0.25 }) }],
        [120, function() { w.snap("the side notch on the right at 0.25") }],
        [0, function() { w.set("notches.0.position", { edge: "bottom", screen: "A" }) }],
        [120, function() { w.snap("main kept on A at the bottom") }],
        [0, function() { w.set("notches.0.position", undefined) }],
        [120, function() { w.snap("main's position reset") }],
        [0, function() { w.set("position", { edge: "left" }) }],
        [120, function() { w.snap("the global position, left") }],
        [0, function() { w.set("notches.0.tabs", true); w.set("notches.0.draggable", false) }],
        [120, function() { w.snap("tabs on, not draggable") }],
        [0, function() { w.set("notches.0.workspaces", ["1"]); w.set("notches.0.idleIcons", false); w.set("notches.0.reserveSpace", true) }],
        [120, function() { w.snap("workspaces, no idle icons, reserving space") }],
        [0, function() { w.set("hoverOpenDelay", 30) }],
        [100, function() { w.pointer("main", "B", "enter") }],
        [120, function() { w.snap("opens by the new delay") }]
      ] } },

      { name: "shadow: a drop moves a notch", config: root.twoNotches, modules: root.facts, steps: function(w) { return [
        [0, function() { w.drop("side", "", { edge: "right", offset: 0.7, screen: "A" }) }],
        [120, function() { w.snap("the side notch dropped on A, right") }],
        [0, function() { w.drop("main", "B", { edge: "bottom", offset: 0.2, screen: "all" }) }],
        [120, function() { w.snap("main dropped at the bottom") }]
      ] } },

      { name: "shadow: two screens, focus and workspaces", config: { notches: [
          { id: "main", modules: ["a"], workspaces: ["1"], position: { screen: "all" } },
          { id: "home", modules: ["b"], position: { screen: "A" } }] },
        screens: { supports: true, known: ["1", "2"], workspaces: { A: "1", B: "2" } },
        modules: { a: { peekOnFocusedScreen: true }, b: {} }, steps: function(w) { return [
        [0, function() { w.snap("main hangs on A only (workspace 1)") }],
        [0, function() { w.screens({ workspaces: { A: "2", B: "1" } }) }],
        [100, function() { w.snap("workspaces swapped") }],
        [0, function() { w.screens({ focused: "A" }) }],
        [100, function() { w.snap("focus on A") }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peeks on the focused screen") }],
        [0, function() { w.dismiss("a"); w.peek("b", 1500) }],
        [80, function() { w.snap("b, kept on A, peeks") }],
        [0, function() { w.dismiss("b"); w.screens({ focused: "B", fullscreen: ["B"] }) }],
        [100, function() { w.snap("focus on B, fullscreen there") }],
        [0, function() { w.pointer("home", "A", "enter") }],
        [root.afterOpen, function() { w.snap("hover opens the notch kept on A") }],
        [0, function() { w.pointer("home", "A", "leave") }],
        [root.afterClose, function() { w.snap("closed") }],
        // A notch kept on A, a peek while you work on B: the visitor shows it on B.
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("b peeks for a notch kept on another screen") }]
      ] } },

      { name: "shadow: news missed while the notch was open peeks as it closes", config: root.oneNotch,
        modules: { a: {}, b: { attention: true } }, steps: function(w) { return [
        [0, function() { w.press("main", "B") }],
        [60, function() { w.peek("b", 600) }],
        [60, function() { w.snap("b's peek waits: the notch is open") }],
        [0, function() { w.press("main", "B") }],
        [120, function() { w.snap("closing: the missed peek shows") }],
        [0, function() { w.module("b", { attention: false }) }],
        [700, function() { w.snap("it ended") }]
      ] } },

      // The pointer reports the screen the copy is on (its window's).
      { name: "shadow: a notch that moves stays where the pointer opened it", config: { notches: [{ id: "main", modules: ["a", "b"], position: { edge: "top" } }] },
        modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("one copy, on the focused screen") }],
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.afterOpen, function() { w.snap("opens on B") }],
        [0, function() { w.screens({ focused: "A" }) }],
        [100, function() { w.snap("focus moved to A: it stays on B while open") }],
        [0, function() { w.pointer("main", "B", "leave") }],
        [root.afterClose, function() { w.snap("closed: it follows the focus to A") }],
        [0, function() { w.press("main", "A") }],
        [80, function() { w.snap("a click on A pins it there") }],
        [0, function() { w.screens({ focused: "B" }) }],
        [100, function() { w.snap("focus on B: pinned stays on A") }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peek while pinned") }]
      ] } },

      { name: "shadow: a peek stays where it started, and peeks during navigation", config: root.twoNotches,
        modules: { a: {}, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.peek("a", 1500) }],
        [60, function() { w.screens({ focused: "A" }) }],
        [80, function() { w.snap("focus moved to A: a's peek stays on B") }],
        [0, function() { w.navigate() }],
        [80, function() { w.snap("navigating over a peek") }],
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("a peek during navigation") }],
        [0, function() { w.key("Escape") }],
        [100, function() { w.snap("navigation left") }],
        [0, function() { w.dismiss("") ; w.navigate(); w.navigate() }],
        [100, function() { w.snap("navigate twice") }]
      ] } },

      { name: "shadow: modules with nothing to show, or no peek view", config: root.oneNotch,
        modules: { a: { hasCompact: false }, b: { hasCompact: false, hasPeek: false } }, steps: function(w) { return [
        [0, function() { w.snap("active without a compact view: nothing to show closed") }],
        [0, function() { w.peek("b", 400) }],
        [80, function() { w.snap("a peek without a peek view") }],
        [0, function() { w.peek("a", 400) }],
        [80, function() { w.snap("a peeks without a compact view") }],
        [0, function() { w.module("a", { icon: true }) }],
        [500, function() { w.snap("a's icon shows closed") }]
      ] } },

      { name: "shadow: a peek on another tab waits for the notch to close", config: { notches: [{ id: "main", modules: ["a", "b"], tabs: true, position: { screen: "all" } }] },
        modules: { a: {}, b: { attention: true } }, steps: function(w) { return [
        [0, function() { w.press("main", "B") }],
        [60, function() { w.tab("main", "B", "a") }],
        [60, function() { w.peek("b", 500) }],
        [60, function() { w.snap("b peeks while the notch is open on tab a") }],
        [0, function() { w.press("main", "B") }],
        [120, function() { w.snap("closed: b's missed peek") }]
      ] } },

      { name: "shadow: recenter puts every notch back", config: root.twoNotches, modules: root.facts, steps: function(w) { return [
        [0, function() { w.set("notches.1.position", { edge: "right", offset: 0.1 }); w.set("notches.0.position", { edge: "bottom", offset: 0.9, screen: "A" }) }],
        [120, function() { w.snap("moved") }],
        [0, function() { w.ipc("recenter", []) }],
        [120, function() { w.snap("recentered") }]
      ] } },

      { name: "shadow: a standalone module has a notch of its own", config: { notches: [{ id: "main", modules: ["a"], position: { screen: "all" } }], solo: { position: { edge: "bottom", offset: 0.3 } } },
        modules: { a: {}, solo: {} }, steps: function(w) { return [
        [0, function() { w.snap("solo rests") }],
        [0, function() { w.peek("solo", 1500) }],
        [80, function() { w.snap("solo peeks on the focused screen") }],
        [0, function() { w.dismiss("solo"); w.screens({ focused: "A" }); w.peek("solo", 1500) }],
        [80, function() { w.snap("solo peeks on A") }],
        [0, function() { w.set("solo.position", { edge: "left", offset: 0.6 }) }],
        [100, function() { w.snap("solo moved") }]
      ] } }
    ]
    var only = String(Quickshell.env("NOTCHD_QMLTEST_ONLY") || "")
    root.cases = all.filter(function(c) { return !only || c.name.indexOf(only) >= 0 })
    warmup.start()
  }
  Timer { id: warmup; interval: 300; onTriggered: root.next() }
}
