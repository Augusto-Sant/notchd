import QtQuick
import Quickshell
import "core"

// The notch state, scenario by scenario (docs/architecture.md, Phase 3.2): the
// daemon decides modes, hover timers, peeks and where they go, pinning, tabs,
// navigation, which screen each copy is on; the QML binds it. Each scenario
// is a config, screens and module facts, and a list of [ms, step]: the inputs
// the UI sends (the pointer, a click, a tab, keys, a drop, a peek a module
// asks for) and `w.snap(label, expected)`, which waits for the daemon to
// have decided everything sent so far and compares what the QML shows with
// `expected`: a flat map of short texts (`w.dump()`), one per fact: a notch's
// place and modules (`main`), a copy (`main.B`: its mode, `pinned`, the peek
// it shows, the tab, the modules it shows open and closed, `covered`,
// `nowindow`, the keyboard it asks for), the visitor's (`visitor.B`), a
// standalone module's (`solo.B`) and navigation (`nav`). A scenario that
// disagrees prints each difference.
//
// scripts/qml-test.sh runs this as one more shard: the daemon binary in
// NOTCHD_QMLTEST_DAEMON on a socket in the shard's own scratch dir, one per
// scenario and stopped when it ends. NOTCHD_QMLTEST_RECORD=1 prints what each
// snapshot shows instead of comparing (for writing a new scenario: read it
// against the label before it goes in).
ShellRoot {
  id: root

  readonly property string dir: Quickshell.env("NOTCHD_QMLTEST_DIR")
  readonly property string socketDir: Quickshell.env("NOTCHD_QMLTEST_SOCKETS")
  readonly property string daemonBin: Quickshell.env("NOTCHD_QMLTEST_DAEMON")
  readonly property bool record: Quickshell.env("NOTCHD_QMLTEST_RECORD") === "1"

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

  function shorten(v) {
    var s = v === undefined ? "(missing)" : JSON.stringify(v)
    return s.length > 160 ? s.slice(0, 157) + "..." : s
  }

  // --------------------------------------------------------------- world
  // Everything one scenario runs on: the service (its own daemon, started the
  // launcher's way) with its compositor, which the scenario drives.
  Component {
    id: worldComponent
    Scope {
      id: w
      property string tag: ""
      readonly property string sock: root.socketDir + "/" + w.tag + ".sock"
      readonly property string cfgFile: root.socketDir + "/" + w.tag + ".json"
      readonly property alias svc: service
      readonly property alias client: service.client
      // The inputs, kept to change one at a time.
      property var screenSpec: ({ names: ["A", "B"], focused: "B", fullscreen: [], workspaces: ({}), known: [], supports: false })
      property var facts: ({})
      property int pending: 0
      property var diffs: []
      property var labels: []

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
        ipcTarget: "scenario-" + w.tag
        daemonCommand: [root.daemonBin, "--socket", w.sock, "--config", w.cfgFile, "daemon", "--ensure"]
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

      // ----- inputs
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
      }

      // A module's facts, merged into what it had.
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
      }
      // A module asks to peek, or its peek goes (every peek for "").
      function peek(id, ms) { service.moduleById(id).requestPeek(ms) }
      function dismiss(id) { service.dismissPeek(id === "" ? null : service.moduleById(id)) }

      function qmlCopy(notch, screen) {
        var n = service.notchById(notch)
        return n ? n.copyOn(screen) : null
      }
      // The pointer entering or leaving a notch's copy on `screen`.
      function pointer(notch, screen, ev) {
        var copy = w.qmlCopy(notch, screen)
        if (copy) copy.setPointer("notch", ev === "enter", ev === "enter" ? screen : "")
      }
      // A click on the copy (pins, or unpins).
      function press(notch, screen) {
        var copy = w.qmlCopy(notch, screen)
        if (copy) copy.togglePinned(screen)
      }
      function tab(notch, screen, moduleId) {
        var copy = w.qmlCopy(notch, screen)
        if (copy) copy.showTab(moduleId)
      }
      // A key while navigating, as the daemon names it.
      function key(name, mods) { service.navKey(name, mods || []) }
      function navigate() { service.navigate() }
      // An IPC method, as `qs ipc call` would.
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
      }
      function set(key, value) {
        service.setConfigValue(key, value)
        service.flushConfig()
      }
      // A notch dropped at `place` ({ edge, offset, screen }), as its window reports it.
      function drop(notch, screen, place) {
        var copy = w.qmlCopy(notch, screen)
        copy.setDragging(true)
        copy.setDragging(false, place)
      }

      // ----- what the QML shows
      function copyText(c) {
        var ids = function(list) { return list.map(function(m) { return m.moduleId }).join("+") }
        return c.mode + (c.pinned ? " pinned" : "") + (c.peekModule ? " peek=" + c.peekModule.moduleId : "")
          + (c.tab !== "" ? " tab=" + c.tab : "") + " open=" + ids(c.shownModules) + " closed=" + ids(c.compactModules)
          + (c.tabModules.length > 0 ? " tabs=" + ids(c.tabModules) : "")
          + (c.offWorkspace ? " covered" : "") + (c.needsWindow ? "" : " nowindow")
          + (c.keyboardFocus !== "none" ? " keyboard=" + c.keyboardFocus : "")
      }
      function dump() {
        var out = {}
        service.notches.forEach(function(n) {
          out[n.notchId] = n.edge + " " + n.placement.offset + (n.placement.screen !== "" ? " " + n.placement.screen : "")
            + (n.tabs ? " tabs" : "") + (n.canDrag ? "" : " fixed") + " [" + n.modules.map(function(m) { return m.moduleId }).join(",") + "]"
          n.copies.forEach(function(c) { out[n.notchId + "." + c.screenName] = w.copyText(c) })
        })
        service.visitor.copies.forEach(function(c) {
          if (c.mode !== "rest" || c.needsWindow) out["visitor." + c.screenName] = w.copyText(c)
        })
        service.standaloneNotches.forEach(function(n) {
          out[n.standaloneId] = n.edge + " " + n.placement.offset
          n.copies.forEach(function(c) { if (c.mode !== "rest" || c.needsWindow) out[n.standaloneId + "." + c.screenName] = w.copyText(c) })
        })
        if (service.navigating) {
          var copy = service.navNotch ? service.navNotch.copies[0] : null
          out.nav = service.navMode + " " + service.navSelected + (copy && copy.tab !== "" ? " tab=" + copy.tab : "")
        }
        return out
      }

      // Waits for the daemon to have decided everything sent so far, then
      // compares what the QML shows with `expected` (or, recording, prints it).
      function snap(label, expected) {
        w.pending++
        w.labels = w.labels.concat([label])
        service.flushConfig()
        service.flushReports()
        var asked = client.ipc("state", [], function(reply) {
          w.pending--
          var got = w.dump()
          if (root.record) {
            console.log("NOTE REC " + JSON.stringify({ scenario: root.current, label: label, state: got }))
            return
          }
          w.add(label, expected, got)
        })
        if (asked < 0) { w.pending--; w.diffs = w.diffs.concat([{ label: label, key: "(connection)", want: "up", got: "down" }]) }
      }
      function add(label, expected, got) {
        var keys = Object.keys(expected || {})
        Object.keys(got).forEach(function(k) { if (keys.indexOf(k) < 0) keys.push(k) })
        keys.forEach(function(k) {
          if ((expected || {})[k] !== got[k]) w.diffs = w.diffs.concat([{ label: label, key: k, want: (expected || {})[k], got: got[k] }])
        })
      }
    }
  }

  // ------------------------------------------------------------ scenarios
  // A scenario: `config`, `screens` (a patch of the defaults: A and B, focus
  // on B, no workspaces), `modules` ({ id: facts }) and `steps(w)`, the
  // [ms, fn] list.
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
          // The facts of the modules the service loaded.
          w.svc.moduleIds.forEach(function(id) { w.module(id, Object.assign({ standalone: id === "solo" }, mods[id] || {})) })
          // The copies, and the daemon's replies, settle.
          root.steps([[250, function() {}]].concat(sc.steps(w)), function() {
            root.until(function() { return w.pending === 0 }, function() { root.report(sc, w) }, 2000)
          })
        })
    })
  }

  function report(sc, w) {
    root.check(w.pending === 0, "a daemon reply never came")
    root.check(w.labels.length > 0, "the scenario compared nothing")
    if (w.diffs.length > 0) {
      // One line each: the runner prints the lines that say FAIL or NOTE.
      w.diffs.slice(0, 12).forEach(function(d) {
        console.log("NOTE  [" + d.label + "] " + d.key + ": shows " + root.shorten(d.got) + ", want " + root.shorten(d.want))
      })
      root.check(false, w.diffs.length + " difference(s) from what the scenario expects (listed above)")
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

  Component.onCompleted: {
    var all = [
      { name: "scenario: idle, and nothing active", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("idle", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.module("a", { active: false }); w.module("b", { active: false }) }],
        [150, function() { w.snap("nothing active: rest", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "rest open= closed=",
          "main.B": "rest open= closed=",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.module("b", { active: true, hasCompact: false, icon: true }) }],
        [150, function() { w.snap("b has only an icon", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=b closed=b",
          "main.B": "compact open=b closed=b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: hover opens and closes after their delays", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.beforeOpen, function() { w.snap("entered, before the open delay", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [root.afterOpen, function() { w.snap("open", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("main", "B", "leave") }],
        [root.beforeClose, function() { w.snap("left, before the close delay", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [root.afterClose, function() { w.snap("closed again", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        // Leaving before it opened forgets the screen and never opens.
        [0, function() { w.pointer("main", "A", "enter") }],
        [30, function() { w.pointer("main", "A", "leave") }],
        [root.afterOpen, function() { w.snap("a pass over the notch opens nothing", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: a peek shows on the focused screen and ends", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.peek("a", 400) }],
        [80, function() { w.snap("a peeks", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=a open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.screens({ focused: "A" }) }],
        [500, function() { w.snap("the peek ended", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("b", 400) }],
        [80, function() { w.snap("b peeks on the other screen", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "peek peek=b open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss("b") }],
        [80, function() { w.snap("dismissed", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: two competing peeks, the higher priority wins", config: root.oneNotch,
        modules: { a: { priority: 1 }, b: { priority: 5 }, c: {} }, steps: function(w) { return [
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peeks", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=b closed=b+a tabs=b+a",
          "main.B": "peek peek=a open=b closed=b+a tabs=b+a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("b outranks a", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=b closed=b+a tabs=b+a",
          "main.B": "peek peek=b open=b closed=b+a tabs=b+a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss("b"); w.peek("a", 1500) }],
        [80, function() { w.snap("a again after b went", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=b closed=b+a tabs=b+a",
          "main.B": "peek peek=a open=b closed=b+a tabs=b+a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss(""); }],
        [0, function() { w.module("a", { priority: 9 }); w.peek("b", 1500) }],
        [80, function() { w.snap("b under a's higher priority", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=b open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a over b", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=a open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: peeks on the focused screen only, and only where the module is shown", config: root.twoNotches,
        modules: { a: { peekOnFocusedScreen: true }, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peeks on B", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=a open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss("a"); w.screens({ focused: "A" }); w.peek("c", 1500) }],
        [80, function() { w.snap("c, only on the side notch, peeks", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.A": "peek peek=c open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss("c"); w.module("a", { peekOnFocusedScreen: false }); w.peek("b", 1500) }],
        [80, function() { w.snap("b peeks on A", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "peek peek=b open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: an urgent peek over a hovered notch, a plain one under it", config: root.oneNotch,
        modules: { a: {}, b: { urgentPeek: true } }, steps: function(w) { return [
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.afterOpen, function() { w.snap("hovered open", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a plain peek does not show over it", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("b urgent peek shows over it", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=b open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("main", "B", "leave") }],
        [root.afterClose, function() { w.snap("left", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=b open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        // A peek then the pointer comes: a plain peek is dismissed, an urgent one stays.
        [0, function() { w.dismiss(""); w.peek("a", 1500) }],
        [60, function() { w.snap("a peeks closed", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=a open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.afterOpen, function() { w.snap("hovering dismissed a's peek", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: pinned by a click, by open, close and toggle", config: root.oneNotch, modules: root.facts, steps: function(w) { return [
        [0, function() { w.press("main", "B") }],
        [60, function() { w.snap("clicked: pinned", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("main", "B", "enter"); w.pointer("main", "B", "leave") }],
        [root.afterClose, function() { w.snap("pinned stays open after the pointer left", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("a", 1500) }],
        [60, function() { w.snap("a peek does not replace a pinned notch", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.press("main", "B") }],
        [60, function() { w.snap("clicked again: unpinned", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.ipc("open", []) }],
        [60, function() { w.snap("ipc open", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.ipc("close", []) }],
        [60, function() { w.snap("ipc close", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.ipc("toggle", []) }],
        [60, function() { w.snap("ipc toggle", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.ipc("closeNotch", ["main"]) }],
        [60, function() { w.snap("ipc closeNotch", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: tabs pick the module the open notch shows", config: { notches: [{ id: "main", modules: ["a", "b"], tabs: true, position: { screen: "all" } }] },
        modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("tabs closed", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.press("main", "B") }],
        [60, function() { w.snap("open, first tab", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.tab("main", "B", "b") }],
        [60, function() { w.snap("tab b", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned tab=b open=b closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.module("b", { active: false }) }],
        [60, function() { w.snap("the shown tab went inactive", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a",
          "main.B": "expanded pinned tab=b open=a closed=a keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.tab("main", "B", "") }],
        [60, function() { w.snap("no tab picked", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a",
          "main.B": "expanded pinned open=a closed=a keyboard=exclusive",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: navigation selects, walks tabs and leaves", config: { notches: [
          { id: "main", modules: ["a", "b"], tabs: true, position: { screen: "all" } },
          { id: "side", modules: ["c"], position: { edge: "left" } }] },
        modules: { a: {}, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.navigate() }],
        [80, function() { w.snap("navigating, the first notch previews", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "nav": "select 0",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Tab") }],
        [60, function() { w.snap("Tab: the next tab", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded tab=b open=b closed=a+b tabs=a+b",
          "nav": "select 0",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Tab", ["shift"]) }],
        [60, function() { w.snap("Shift+Tab: back", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded tab=a open=a closed=a+b tabs=a+b",
          "nav": "select 0",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Right") }],
        [60, function() { w.snap("Right: the second notch", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact tab=a open=a closed=a+b tabs=a+b",
          "nav": "select 1",
          "side": "left 0.5 tabs [c]",
          "side.B": "expanded open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Right") }],
        [60, function() { w.snap("Right: wraps", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded tab=a open=a closed=a+b tabs=a+b",
          "nav": "select 0",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("2") }],
        [60, function() { w.snap("2: the second notch", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact tab=a open=a closed=a+b tabs=a+b",
          "nav": "select 1",
          "side": "left 0.5 tabs [c]",
          "side.B": "expanded open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Escape") }],
        [60, function() { w.snap("Escape: left", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.navigate() }],
        [0, function() { w.key("Right") }],
        [0, function() { w.key("Return") }],
        [80, function() { w.snap("Enter: the second notch pinned", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.B": "expanded pinned open=c closed=c keyboard=exclusive",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: navigation removes a notch (asked twice) and moves it", config: { notches: [
          { id: "main", modules: ["a", "b"], position: { screen: "all" } },
          { id: "side", modules: ["c"], position: { edge: "left" } },
          { id: "extra", modules: [], position: { edge: "bottom" } }] },
        modules: { a: {}, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.navigate(); w.key("3") }],
        [60, function() { w.snap("the third selected", {
          "extra": "bottom 0.5 tabs []",
          "extra.B": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "select 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("x") }],
        [60, function() { w.snap("x asks first", {
          "extra": "bottom 0.5 tabs []",
          "extra.B": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "remove 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Escape") }],
        [60, function() { w.snap("Escape keeps it", {
          "extra": "bottom 0.5 tabs []",
          "extra.B": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "select 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("m") }],
        [60, function() { w.snap("move mode", {
          "extra": "bottom 0.5 tabs []",
          "extra.B": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "move 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Left") }],
        [60, function() { w.snap("moved to the left edge", {
          "extra": "left 0.5 tabs []",
          "extra.B": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "move 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Right", ["shift"]) }],
        [60, function() { w.snap("slid along it", {
          "extra": "left 0.55 tabs []",
          "extra.B": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "move 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Tab") }],
        [60, function() { w.snap("on the next screen", {
          "extra": "left 0.55 A tabs []",
          "extra.A": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "move 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Escape") }],
        [60, function() { w.snap("Escape puts it back", {
          "extra": "bottom 0.5 tabs []",
          "extra.B": "expanded open= closed=",
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "select 2",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("x"); w.key("x") }],
        [100, function() { w.snap("x twice removed it", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "nav": "select 1",
          "side": "left 0.5 tabs [c]",
          "side.B": "expanded open=c closed=c",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: set changes position, edge and delays", config: root.twoNotches, modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("before", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.set("notches.1.position", { edge: "right", offset: 0.25 }) }],
        [120, function() { w.snap("the side notch on the right at 0.25", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.25 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.set("notches.0.position", { edge: "bottom", screen: "A" }) }],
        [120, function() { w.snap("main kept on A at the bottom", {
          "main": "bottom 0.5 A tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.25 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.set("notches.0.position", undefined) }],
        [120, function() { w.snap("main's position reset", {
          "main": "top 0.5 tabs [a,b]",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.25 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.set("position", { edge: "left" }) }],
        [120, function() { w.snap("the global position, left", {
          "main": "left 0.5 tabs [a,b]",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.25 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.set("notches.0.tabs", true); w.set("notches.0.draggable", false) }],
        [120, function() { w.snap("tabs on, not draggable", {
          "main": "left 0.5 tabs fixed [a,b]",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.25 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.set("notches.0.workspaces", ["1"]); w.set("notches.0.idleIcons", false); w.set("notches.0.reserveSpace", true) }],
        [120, function() { w.snap("workspaces, no idle icons, reserving space", {
          "main": "left 0.5 tabs fixed [a,b]",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.25 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.set("hoverOpenDelay", 30) }],
        [100, function() { w.pointer("main", "B", "enter") }],
        [120, function() { w.snap("opens by the new delay", {
          "main": "left 0.5 tabs fixed [a,b]",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "side": "right 0.25 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: a drop moves a notch", config: root.twoNotches, modules: root.facts, steps: function(w) { return [
        [0, function() { w.drop("side", "", { edge: "right", offset: 0.7, screen: "A" }) }],
        [120, function() { w.snap("the side notch dropped on A, right", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.7 A tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.drop("main", "B", { edge: "bottom", offset: 0.2, screen: "all" }) }],
        [120, function() { w.snap("main dropped at the bottom", {
          "main": "bottom 0.2 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.7 A tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: two screens, focus and workspaces", config: { notches: [
          { id: "main", modules: ["a"], workspaces: ["1"], position: { screen: "all" } },
          { id: "home", modules: ["b"], position: { screen: "A" } }] },
        screens: { supports: true, known: ["1", "2"], workspaces: { A: "1", B: "2" } },
        modules: { a: { peekOnFocusedScreen: true }, b: {} }, steps: function(w) { return [
        [0, function() { w.snap("main hangs on A only (workspace 1)", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "compact open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a",
          "main.B": "compact open=a closed=a covered",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.screens({ workspaces: { A: "2", B: "1" } }) }],
        [100, function() { w.snap("workspaces swapped", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "compact open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a covered",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.screens({ focused: "A" }) }],
        [100, function() { w.snap("focus on A", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "compact open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a covered",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peeks on the focused screen", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "compact open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "peek peek=a open=a closed=a",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss("a"); w.peek("b", 1500) }],
        [80, function() { w.snap("b, kept on A, peeks", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "peek peek=b open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a covered",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss("b"); w.screens({ focused: "B", fullscreen: ["B"] }) }],
        [100, function() { w.snap("focus on B, fullscreen there", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "compact open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a covered",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("home", "A", "enter") }],
        [root.afterOpen, function() { w.snap("hover opens the notch kept on A", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "expanded open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a covered",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("home", "A", "leave") }],
        [root.afterClose, function() { w.snap("closed", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "compact open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a covered",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5"
        }) }],
        // A notch kept on A, a peek while you work on B: the visitor shows it on B.
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("b peeks for a notch kept on another screen", {
          "home": "top 0.5 A tabs [b]",
          "home.A": "peek peek=b open=b closed=b",
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a covered",
          "main.B": "compact open=a closed=a",
          "solo": "top 0.5",
          "visitor.B": "peek peek=b open=a+b closed="
        }) }]
      ] } },

      { name: "scenario: news missed while the notch was open peeks as it closes", config: root.oneNotch,
        modules: { a: {}, b: { attention: true } }, steps: function(w) { return [
        [0, function() { w.press("main", "B") }],
        [60, function() { w.peek("b", 600) }],
        [60, function() { w.snap("b's peek waits: the notch is open", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.press("main", "B") }],
        [120, function() { w.snap("closing: the missed peek shows", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=b open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.module("b", { attention: false }) }],
        [700, function() { w.snap("it ended", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      // The pointer reports the screen the copy is on (its window's).
      { name: "scenario: a notch that moves stays where the pointer opened it", config: { notches: [{ id: "main", modules: ["a", "b"], position: { edge: "top" } }] },
        modules: root.facts, steps: function(w) { return [
        [0, function() { w.snap("one copy, on the focused screen", {
          "main": "top 0.5 tabs [a,b]",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("main", "B", "enter") }],
        [root.afterOpen, function() { w.snap("opens on B", {
          "main": "top 0.5 tabs [a,b]",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.screens({ focused: "A" }) }],
        [100, function() { w.snap("focus moved to A: it stays on B while open", {
          "main": "top 0.5 tabs [a,b]",
          "main.B": "expanded open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.pointer("main", "B", "leave") }],
        [root.afterClose, function() { w.snap("closed: it follows the focus to A", {
          "main": "top 0.5 tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.press("main", "A") }],
        [80, function() { w.snap("a click on A pins it there", {
          "main": "top 0.5 tabs [a,b]",
          "main.A": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.screens({ focused: "B" }) }],
        [100, function() { w.snap("focus on B: pinned stays on A", {
          "main": "top 0.5 tabs [a,b]",
          "main.A": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("a", 1500) }],
        [80, function() { w.snap("a peek while pinned", {
          "main": "top 0.5 tabs [a,b]",
          "main.A": "expanded pinned open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: a peek stays where it started, and peeks during navigation", config: root.twoNotches,
        modules: { a: {}, b: {}, c: {} }, steps: function(w) { return [
        [0, function() { w.peek("a", 1500) }],
        [60, function() { w.screens({ focused: "A" }) }],
        [80, function() { w.snap("focus moved to A: a's peek stays on B", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=a open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.navigate() }],
        [80, function() { w.snap("navigating over a peek", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "expanded open=a closed=a+b tabs=a+b",
          "main.B": "compact peek=a open=a closed=a+b tabs=a+b",
          "nav": "select 0",
          "side": "left 0.5 tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("b", 1500) }],
        [80, function() { w.snap("a peek during navigation", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "expanded peek=b open=a closed=a+b tabs=a+b",
          "main.B": "compact peek=a open=a closed=a+b tabs=a+b",
          "nav": "select 0",
          "side": "left 0.5 tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.key("Escape") }],
        [100, function() { w.snap("navigation left", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "peek peek=b open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=a open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.dismiss("") ; w.navigate(); w.navigate() }],
        [100, function() { w.snap("navigate twice", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "left 0.5 tabs [c]",
          "side.A": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: modules with nothing to show, or no peek view", config: root.oneNotch,
        modules: { a: { hasCompact: false }, b: { hasCompact: false, hasPeek: false } }, steps: function(w) { return [
        [0, function() { w.snap("active without a compact view: nothing to show closed", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "rest open=a closed= tabs=a+b",
          "main.B": "rest open=a closed= tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("b", 400) }],
        [80, function() { w.snap("a peek without a peek view", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "rest open=a closed= tabs=a+b",
          "main.B": "rest open=a closed= tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.peek("a", 400) }],
        [80, function() { w.snap("a peeks without a compact view", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "rest open=a closed= tabs=a+b",
          "main.B": "peek peek=a open=a closed= tabs=a+b",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.module("a", { icon: true }) }],
        [500, function() { w.snap("a's icon shows closed", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a tabs=a+b",
          "main.B": "compact open=a closed=a tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: a peek on another tab waits for the notch to close", config: { notches: [{ id: "main", modules: ["a", "b"], tabs: true, position: { screen: "all" } }] },
        modules: { a: {}, b: { attention: true } }, steps: function(w) { return [
        [0, function() { w.press("main", "B") }],
        [60, function() { w.tab("main", "B", "a") }],
        [60, function() { w.peek("b", 500) }],
        [60, function() { w.snap("b peeks while the notch is open on tab a", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "expanded pinned tab=a open=a closed=a+b tabs=a+b keyboard=exclusive",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.press("main", "B") }],
        [120, function() { w.snap("closed: b's missed peek", {
          "main": "top 0.5 all tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "main.B": "peek peek=b tab=a open=a closed=a+b tabs=a+b",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: recenter puts every notch back", config: root.twoNotches, modules: root.facts, steps: function(w) { return [
        [0, function() { w.set("notches.1.position", { edge: "right", offset: 0.1 }); w.set("notches.0.position", { edge: "bottom", offset: 0.9, screen: "A" }) }],
        [120, function() { w.snap("moved", {
          "main": "bottom 0.9 A tabs [a,b]",
          "main.A": "compact open=a closed=a+b tabs=a+b",
          "side": "right 0.1 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }],
        [0, function() { w.ipc("recenter", []) }],
        [120, function() { w.snap("recentered", {
          "main": "top 0.5 tabs [a,b]",
          "main.B": "compact open=a closed=a+b tabs=a+b",
          "side": "top 0.5 tabs [c]",
          "side.B": "compact open=c closed=c",
          "solo": "top 0.5"
        }) }]
      ] } },

      { name: "scenario: a standalone module has a notch of its own", config: { notches: [{ id: "main", modules: ["a"], position: { screen: "all" } }], solo: { position: { edge: "bottom", offset: 0.3 } } },
        modules: { a: {}, solo: {} }, steps: function(w) { return [
        [0, function() { w.snap("solo rests", {
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a",
          "main.B": "compact open=a closed=a",
          "solo": "bottom 0.3"
        }) }],
        [0, function() { w.peek("solo", 1500) }],
        [80, function() { w.snap("solo peeks on the focused screen", {
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a",
          "main.B": "compact open=a closed=a",
          "solo": "bottom 0.3",
          "solo.B": "peek peek=solo open=solo closed="
        }) }],
        [0, function() { w.dismiss("solo"); w.screens({ focused: "A" }); w.peek("solo", 1500) }],
        [80, function() { w.snap("solo peeks on A", {
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a",
          "main.B": "compact open=a closed=a",
          "solo": "left 0.6",
          "solo.A": "peek peek=solo open=solo closed=",
          "solo.B": "rest open=solo closed= covered"
        }) }],
        [0, function() { w.set("solo.position", { edge: "left", offset: 0.6 }) }],
        [100, function() { w.snap("solo moved", {
          "main": "top 0.5 all tabs [a]",
          "main.A": "compact open=a closed=a",
          "main.B": "compact open=a closed=a",
          "solo": "left 0.6",
          "solo.A": "peek peek=solo open=solo closed=",
          "solo.B": "rest open=solo closed= covered"
        }) }]
      ] } }
    ]
    var only = String(Quickshell.env("NOTCHD_QMLTEST_ONLY") || "")
    root.cases = all.filter(function(c) { return !only || c.name.indexOf(only) >= 0 })
    warmup.start()
  }
  Timer { id: warmup; interval: 300; onTriggered: root.next() }
}
