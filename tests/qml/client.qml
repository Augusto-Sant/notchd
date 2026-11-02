import QtQuick
import Quickshell
import Quickshell.Io
import "core/client"

// Behaviour tests of the daemon client (core/client/DaemonClient.qml) against
// the fake daemon (tests/qml/fake-daemon.mjs, no Rust). scripts/qml-test.sh
// starts the daemon on NOTCHD_QMLTEST_SOCKET, links this file as the shell and
// runs it as one more shard beside shell.qml's. Same harness conventions as
// shell.qml: a case logs "PASS <name>" or "FAIL <name>: <why>", "DONE" ends the
// run. Each case makes its own client (client()) and resets the daemon
// (resetDaemon()), so cases need no order.
ShellRoot {
  id: root

  readonly property string socketPath: Quickshell.env("NOTCHD_QMLTEST_SOCKET")

  // ------------------------------------------------------------- harness
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
    root.disposeClients()
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
    root.rx = []
    root.events = []
    root.resetDaemon()
    try { root.cases[root.index].run() }
    catch (e) { root.check(false, "threw " + e); root.done() }
  }

  // Waits until pred() holds (checked every 20 ms, for at most ms), then runs
  // fn; on a timeout, fn still runs and its checks say what is missing.
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
  // Runs fn once after ms.
  function later(ms, fn) {
    laterTimer.interval = ms
    laterTimer.action = fn
    laterTimer.restart()
  }
  Timer {
    id: laterTimer
    property var action: null
    onTriggered: { var fn = action; action = null; if (fn) fn() }
  }

  // ------------------------------------------------------- the fake daemon
  // What it received ({ msg, n }, n the connection) and its events, this case.
  property var rx: []
  property var events: []
  function received(t) { return root.rx.filter(function(r) { return r.msg.t === t }).map(function(r) { return r.msg }) }
  function connections() { return root.events.filter(function(e) { return e.ev === "connect" }).length }

  function control(cmd) { ctl.write(JSON.stringify(cmd) + "\n"); ctl.flush() }
  function resetDaemon() { root.control({ cmd: "reset" }) }

  Socket {
    id: ctl
    path: root.socketPath + ".ctl"
    connected: true
    parser: SplitParser {
      onRead: line => {
        var m = JSON.parse(line)
        if (m.rx) root.rx = root.rx.concat([{ msg: m.rx, n: m.n }])
        else if (m.ev) root.events = root.events.concat([m])
      }
    }
  }

  // ------------------------------------------------------------- clients
  property var made: []
  Component {
    id: clientComponent
    DaemonClient {
      clientName: "qmltest"
      clientVersion: "0.0.1"
      reconnectMin: 50
      reconnectMax: 200
    }
  }
  // A fresh client on the fake daemon's socket (or `path`).
  function client(path) {
    var c = clientComponent.createObject(root, { socketPath: path === undefined ? root.socketPath : path })
    root.made.push(c)
    return c
  }
  function disposeClients() {
    root.made.forEach(function(c) { if (c.enabled !== undefined) c.enabled = false; c.destroy() })
    root.made = []
  }

  Component.onCompleted: {
    var all = [
      { name: "client: hello, welcome, then the entities land in properties", run: root.handshake },
      { name: "client: an update older than the one held is ignored, a newer one replaces it", run: root.staleRev },
      { name: "client: lines split across reads, and several in one read, are put together", run: root.partialReads },
      { name: "client: pointer, key, event, set, edit and ipc reach the daemon; requests get their replies", run: root.sendsAndReplies },
      { name: "client: the daemon restarting is survived: requests fail while it is down, then it reconnects and the revs start over", run: root.reconnects },
      { name: "client: a daemon that is not there yet is waited for, with backoff", run: root.daemonLate },
      { name: "client: a refused connection (a dead daemon's socket file) is retried, not given up", run: root.refusedRetried },
      { name: "launcher: with the daemon away and no attempt pending, it still reconnects when the daemon is back", run: root.launcherNeverGivesUp },
      { name: "client: a protocol refusal is an error state, with no retry", run: root.refusal },
      { name: "launcher: the socket path the start script prints is connected to", run: root.launcherStarts },
      { name: "launcher: a start script that fails leaves the client idle and the error readable", run: root.launcherFails }
    ]
    var only = String(Quickshell.env("NOTCHD_QMLTEST_ONLY") || "")
    root.cases = all.filter(function(c) { return !only || c.name.indexOf(only) >= 0 })
    warmup.start()
  }
  // The control socket needs a moment to connect.
  Timer { id: warmup; interval: 300; onTriggered: root.next() }

  // --------------------------------------------------------------- cases
  function handshake() {
    var c = root.client()
    var welcomed = 0
    c.welcomed.connect(function() { welcomed++ })
    root.until(function() { return c.ready && c.state && c.view("timer", "compact") && c.module("media") }, function() {
      root.check(c.ready && c.status === "ready", "not ready: " + c.status)
      root.check(welcomed === 1, "welcomed fired " + welcomed + " times")
      root.check(c.welcome && c.welcome.daemon === "0.4.0" && c.welcome.tokens.length === 11, "welcome: " + JSON.stringify(c.welcome))
      var hello = root.received("hello")
      root.check(hello.length === 1 && hello[0].proto === 1 && hello[0].client === "qmltest"
                 && hello[0].clientVersion === "0.0.1" && hello[0].instance === "default" && !!hello[0].caps,
                 "hello: " + JSON.stringify(hello))
      root.check(c.config && c.config.rev === 1 && c.config.cfg.hoverOpenDelay === 200, "config: " + JSON.stringify(c.config))
      root.check(c.schema && c.schema.pages.length === 2, "schema: " + JSON.stringify(c.schema))
      root.check(c.state && c.state.rev === 41 && c.state.notches[0].id === "main"
                 && c.state.notches[0].copies[0].mode === "compact", "state: " + JSON.stringify(c.state))
      var tree = c.view("timer", "compact")
      root.check(tree && tree.t === "row" && tree.children.length === 2 && tree.children[1].text === "24:31", "view: " + JSON.stringify(tree))
      root.check(c.view("timer", "verticalPeek") === null, "a null tree is not null")
      root.check(c.view("nobody", "compact") === null, "an unknown slot is not null")
      root.check(c.module("media").title === "Now playing" && c.module("media").error === null, "module: " + JSON.stringify(c.module("media")))
      root.check(c.module("nobody") === null, "an unknown module is not null")
      root.check(c.error === "", "error: " + c.error)
      root.done()
    })
  }

  function staleState(rev, mode) {
    return { t: "state", rev: rev, navigation: null, settingsOpen: false, notches: [], reserve: [], standalone: [], marker: mode }
  }

  // Bug it guards: an update that arrives late (an older rev) replacing a
  // newer one would show the notch as it was.
  function staleRev() {
    var c = root.client()
    var seen = []
    c.updated.connect(function(t, key) { seen.push(t + ":" + key) })
    root.until(function() { return c.state && c.view("timer", "compact") }, function() {
      root.check(c.state.rev === 41, "state rev " + c.state.rev)
      seen = []
      root.control({ cmd: "send", msg: root.staleState(40, "old") })
      root.control({ cmd: "send", msg: root.staleState(41, "same") })
      root.control({ cmd: "send", msg: { t: "view", module: "timer", slot: "compact", rev: 100, tree: null } })
      root.control({ cmd: "send", msg: root.staleState(42, "new") })
      root.until(function() { return c.state.rev === 42 }, function() {
        root.check(c.state.rev === 42 && c.state.marker === "new", "state after: " + JSON.stringify(c.state))
        root.check(seen.join(",") === "state:state",
                   "updates seen: " + seen.join(",") + " (the old and the repeated revs are not updates)")
        // Revs are per entity: the view's 112 stays held, its own 100 is stale.
        root.check(c.view("timer", "compact") !== null, "a stale view replaced the held one")
        root.control({ cmd: "send", msg: { t: "module", id: "media", rev: 6, title: "Old", priority: 1, active: true, icon: "", attention: false,
                                          expandedWidth: 400, urgentPeek: false, peekOnFocusedScreen: false, error: null } })
        root.control({ cmd: "send", msg: { t: "config", rev: 2, cfg: { hoverOpenDelay: 300 }, resolved: {} } })
        root.until(function() { return c.config.rev === 2 }, function() {
          root.check(c.module("media").title === "Now playing", "a stale module replaced the held one: " + c.module("media").title)
          root.check(c.config.cfg.hoverOpenDelay === 300, "config not replaced")
          root.done()
        })
      })
    })
  }

  // Bug it guards: a read ends anywhere, so a line cut in two reads, or two
  // lines in one, must not be lost or mangled.
  function partialReads() {
    var c = root.client()
    root.until(function() { return c.state }, function() {
      var a = JSON.stringify(root.staleState(60, "a")) + "\n"
      var b = JSON.stringify(root.staleState(61, "b")) + "\n"
      var d = JSON.stringify(root.staleState(62, "d")) + "\n"
      var whole = a + b + d
      // a|b cut in the middle of a's text, then the rest of b with all of d in one read,
      // d's own newline in a read of its own.
      root.control({ cmd: "raw", gapMs: 30, chunks: [
        whole.slice(0, 20), whole.slice(20, a.length + 10), whole.slice(a.length + 10, whole.length - 1), "\n",
        "not json\n", JSON.stringify({ t: "future-thing", rev: 1 }) + "\n"
      ] })
      root.until(function() { return c.state.rev === 62 }, function() {
        root.check(c.state.rev === 62 && c.state.marker === "d", "state: " + JSON.stringify(c.state))
        root.check(c.ready, "a bad line broke the connection")
        root.done()
      })
    })
  }

  function sendsAndReplies() {
    var c = root.client()
    root.until(function() { return c.ready }, function() {
      c.pointer("main", "DP-1", "enter")
      c.key("main", "DP-1", "Tab", ["shift"], "")
      c.key("main", "DP-1", "a", [], "a")
      c.event("timer", "expanded", "toggle")
      c.event("media", "expanded", "seek", 0.42)
      var replies = {}
      var ids = {
        set: c.set("hoverOpenDelay", 200, function(m) { replies.set = m }),
        remove: c.set("position", undefined, function(m) { replies.remove = m }),
        edit: c.edit("moveModule", { notch: "main", module: "media", delta: 1 }, function(m) { replies.edit = m }),
        ipc: c.ipc("timer", ["3m", 5], function(m) { replies.ipc = m }),
        fail: c.ipc("fail", [], function(m) { replies.fail = m })
      }
      root.check(ids.set > 0 && ids.remove > ids.set && ids.fail > ids.ipc, "request ids: " + JSON.stringify(ids))
      root.until(function() { return replies.fail && root.received("event").length === 2 }, function() {
        root.check(root.received("pointer").length === 1 && JSON.stringify(root.received("pointer")[0])
                   === JSON.stringify({ t: "pointer", notch: "main", screen: "DP-1", ev: "enter" }), "pointer: " + JSON.stringify(root.received("pointer")))
        var keys = root.received("key")
        root.check(keys.length === 2 && keys[0].key === "Tab" && keys[0].mods[0] === "shift" && keys[0].text === ""
                   && keys[1].key === "a" && keys[1].mods.length === 0 && keys[1].text === "a", "keys: " + JSON.stringify(keys))
        var events = root.received("event")
        root.check(events[0].value === null && events[0].action === "toggle" && events[1].value === 0.42, "events: " + JSON.stringify(events))
        var sets = root.received("set")
        root.check(sets.length === 2 && sets[0].id === ids.set && sets[0].key === "hoverOpenDelay" && sets[0].value === 200,
                   "set: " + JSON.stringify(sets))
        root.check(sets[1].key === "position" && sets[1].value === null && ("value" in sets[1]), "a removal is a null value: " + JSON.stringify(sets[1]))
        var edit = root.received("edit")[0]
        root.check(edit && edit.op === "moveModule" && edit.delta === 1 && edit.notch === "main" && edit.module === "media", "edit: " + JSON.stringify(edit))
        var ipc = root.received("ipc")[0]
        root.check(ipc && ipc.method === "timer" && ipc.args[0] === "3m" && ipc.args[1] === "5", "ipc: " + JSON.stringify(ipc))
        root.check(replies.set && replies.set.t === "ok" && replies.set.id === ids.set, "set reply: " + JSON.stringify(replies.set))
        root.check(replies.ipc && replies.ipc.t === "ok" && replies.ipc.result === "timer:3m,5", "ipc reply: " + JSON.stringify(replies.ipc))
        root.check(replies.fail && replies.fail.t === "err" && replies.fail.code === "module", "err reply: " + JSON.stringify(replies.fail))
        root.check(replies.edit && replies.edit.t === "ok" && replies.remove && replies.remove.t === "ok", "edit or removal not answered")
        root.done()
      })
    })
  }

  // Bug it guards: the daemon restarts (an update, a crash) and the shell would
  // sit on a dead socket, or keep rev numbers the new daemon starts over.
  function reconnects() {
    var c = root.client()
    var disconnects = 0
    c.disconnected.connect(function() { disconnects++ })
    root.until(function() { return c.state }, function() {
      var reply = null
      // The daemon goes down.
      root.control({ cmd: "restart", downMs: 400 })
      root.until(function() { return c.status !== "ready" }, function() {
        root.check(c.status === "disconnected" && disconnects === 1, "after the drop: " + c.status + ", disconnected fired " + disconnects)
        root.check(c.state === null && c.module("media") === null, "the old entities stayed after the drop")
        root.check(c.set("x", 1, function(m) { reply = m }) === -1 && reply && reply.code === "disconnected", "a request while down: " + JSON.stringify(reply))
        root.until(function() { return c.ready && c.state }, function() {
          root.check(c.ready, "never reconnected: " + c.status)
          root.check(c.state && c.state.rev === 41, "the new daemon's rev 41 was ignored as stale: " + JSON.stringify(c.state))
          root.check(c.attempts === 0, "attempts after a welcome: " + c.attempts)
          root.check(root.received("hello").length === 2, "hellos: " + root.received("hello").length)
          root.check(root.connections() === 2, "connections: " + root.connections())
          // And requests work again.
          c.set("y", 2, function(m) { reply = m })
          root.until(function() { return reply && reply.t === "ok" }, function() {
            root.check(reply && reply.t === "ok", "no reply after reconnecting")
            root.done()
          })
        }, 4000)
      })
    })
  }

  function daemonLate() {
    var c = root.client(root.socketPath + ".nowhere")
    root.until(function() { return c.attempts >= 3 }, function() {
      root.check(c.attempts >= 3 && c.status === "disconnected", "tried " + c.attempts + " times, status " + c.status)
      root.check(root.connections() === 0, "something connected")
      c.socketPath = root.socketPath
      root.until(function() { return c.ready }, function() {
        root.check(c.ready && c.attempts === 0, "not ready after the path was fixed: " + c.status)
        root.done()
      })
    })
  }

  function refusal() {
    root.control({ cmd: "proto", value: 2 })
    var c = root.client()
    var refused = null
    c.refused.connect(function(err) { refused = err })
    root.until(function() { return c.status === "refused" }, function() {
      root.check(c.status === "refused" && !c.ready, "status " + c.status)
      root.check(refused && refused.code === "proto" && refused.want === 2, "refused: " + JSON.stringify(refused))
      root.check(c.error.indexOf("2") >= 0, "error: " + c.error)
      root.check(c.state === null, "entities after a refusal")
      var before = root.connections()
      // No retry: the daemon would only refuse again.
      root.later(600, function() {
        root.check(c.status === "refused" && root.connections() === before, "retried after a refusal: " + root.connections() + " connections")
        root.done()
      })
    })
  }

  // A launcher whose start script is `sh -c <script>`.
  function launcher(script) {
    var l = launcherComponent.createObject(root, { command: ["sh", "-c", script] })
    root.made.push(l)
    return l
  }
  Component {
    id: launcherComponent
    DaemonLauncher {
      clientName: "qmltest"
      logName: "qmltest"
    }
  }

  function launcherStarts() {
    var l = root.launcher('printf "%s\\n" "$NOTCHD_QMLTEST_SOCKET"')
    root.until(function() { return l.client.ready }, function() {
      root.check(l.state === "ready" && l.socketPath === root.socketPath, "state " + l.state + ", socket " + l.socketPath)
      root.check(l.client.ready, "the client did not connect: " + l.client.status)
      root.done()
    })
  }

  function launcherFails() {
    var l = root.launcher('echo "{\\"error\\":\\"notchd not found\\"}"; exit 1')
    root.until(function() { return l.state === "failed" }, function() {
      root.check(l.state === "failed" && l.error === "notchd not found", "state " + l.state + ", error " + l.error)
      root.check(l.client.status === "disconnected" && l.client.socketPath === "", "the client has a path: " + l.client.socketPath)
      root.done()
    })
  }

  // Bug: a daemon killed with SIGKILL leaves its socket file; connecting to
  // it is refused, and the client stopped retrying after that one refusal,
  // so the daemon the launcher brought back was never reached ("notchd not
  // running" stayed until a settings edit). It keeps retrying on its backoff.
  Process { id: staleMaker }
  function refusedRetried() {
    var path = root.socketPath + ".stale"
    staleMaker.command = ["python3", "-c", "import socket,sys; socket.socket(socket.AF_UNIX).bind(sys.argv[1])", path]
    staleMaker.running = true
    root.later(400, function() {
      var c = root.client(path)
      root.until(function() { return c.attempts >= 4 }, function() {
        root.check(c.attempts >= 4 && c.status === "disconnected", "gave up after " + c.attempts + " refused attempts, status " + c.status)
        root.done()
      }, 6000)
    })
  }

  // Bug: after the daemon was killed, a race left the client with no attempt
  // pending (a reconnect asked for during an attempt that then failed), and
  // nothing ever tried again: "notchd not running" stayed with the daemon back
  // and listening. Here that state is made by hand: every pending attempt
  // stopped while the daemon is away. The launcher must still reconnect.
  function launcherNeverGivesUp() {
    var l = root.launcher('printf "%s\\n" "$NOTCHD_QMLTEST_SOCKET"')
    root.until(function() { return l.client.ready }, function() {
      root.control({ cmd: "restart", downMs: 2500 })
      root.until(function() { return !l.client.ready }, function() {
        root.later(900, function() {
          // The lost-retry state: nothing pending in the client or the launcher.
          l.client.retryTimer.stop()
          l.restartTimer.stop()
          root.until(function() { return l.client.ready }, function() {
            root.check(l.client.ready, "never reconnected: " + l.client.status + " after " + l.client.attempts + " attempts")
            root.done()
          }, 9000)
        })
      })
    })
  }
}
