import QtQuick
import Quickshell.Io
import "Protocol.js" as Protocol

// A client of the notchd daemon (docs/architecture.md, "Protocol: core to
// UI"): connects to its socket, says hello, keeps the latest of each entity
// the daemon sends, sends requests with a reply callback, and reconnects when
// the socket drops (the daemon may restart). NotchService holds one, through
// its DaemonLauncher.
//
// The host sets `socketPath`; `status` says where the connection is:
//   "disconnected"  no socket (waiting to retry, or `enabled` is off)
//   "connecting"    socket up, hello sent, no welcome yet
//   "ready"         welcome received; the entities below follow
//   "refused"       the daemon refused the handshake (`error` says why); no retry
QtObject {
  id: client

  // ----------------------------------------------------------- settings
  property string socketPath: ""
  property bool enabled: true
  property string clientName: "quickshell"
  property string clientVersion: ""
  property string instance: "default"
  // What this UI can draw: `caps` of the hello.
  property var caps: ({ primitives: [], surfaces: [], webview: false, canvas: false })
  property int reconnectMin: 200
  property int reconnectMax: 5000

  // ------------------------------------------------------------- status
  readonly property string status: client._status
  readonly property bool ready: client._status === "ready"
  // Why the last connection failed or was refused; "" when none.
  readonly property string error: client._error
  // The `welcome`: { daemon, instance, config (the file's path), tokens }.
  readonly property var welcome: client._welcome
  // Failed connection attempts since the last welcome.
  readonly property int attempts: client._attempts

  // ----------------------------------------------------------- entities
  // The latest message of each entity, `null` or `{}` until it arrives and
  // again after the connection drops (the daemon's revs start over).
  readonly property var config: client._store.config
  readonly property var schema: client._store.schema
  readonly property var state: client._store.state
  // "module/slot" -> view message, and module id -> module message. Read them
  // with view() and module().
  readonly property var views: client._store.views
  readonly property var modules: client._store.modules

  // The tree of a module slot: the `view` message's `tree`, `null` when the
  // module has none for it or has not sent one.
  function view(moduleId, slot) {
    var msg = client._store.views[Protocol.viewKey(moduleId, slot)]
    return msg ? msg.tree : null
  }
  // A module's facts (the `module` message), or null.
  function module(moduleId) { return client._store.modules[moduleId] || null }

  // ------------------------------------------------------------ signals
  // The handshake finished; the entities follow.
  signal welcomed(var welcome)
  // The connection is gone (or never came up).
  signal disconnected()
  // The handshake was refused (`err` code `proto`).
  signal refused(var err)
  // An entity was replaced: its message `t` and key ("config", "view:timer/compact", ...).
  signal updated(string t, string key, var msg)
  // Any other message: `surface-open`, an `err` without an id, a type this client
  // does not know. Every parsed message goes through here after it was applied.
  signal message(var msg)

  // ---------------------------------------------------------- requests
  // Each returns the request id, or -1 when not connected (the callback then
  // gets a `disconnected` err at once). `reply(msg)` gets the `ok` or `err`
  // for the id; a connection that drops first answers `err` code `disconnected`.
  function set(key, value, reply) { return client._request(function(id) { return Protocol.set(id, key, value) }, reply) }
  function edit(op, fields, reply) { return client._request(function(id) { return Protocol.edit(id, op, fields) }, reply) }
  function ipc(method, args, reply) { return client._request(function(id) { return Protocol.ipc(id, method, args) }, reply) }

  // Input facts: no id, no reply. They are dropped while not connected.
  function pointer(notch, screen, ev) { return client._send(Protocol.pointer(notch, screen, ev)) }
  function key(notch, screen, name, mods, text) { return client._send(Protocol.key(notch, screen, name, mods, text)) }
  function event(moduleId, slot, action, value) { return client._send(Protocol.event(moduleId, slot, action, value)) }
  // Any other message of the UI-to-core set (`screens`, `tab`, `settings`, `drag`,
  // `reserve-applied`), built by the caller.
  function send(msg) { return client._send(msg) }

  // Connects now, skipping the backoff. A no-op while connected to `socketPath`.
  function connectNow() {
    if (!client.enabled || client.socketPath === "" || client._status === "refused") return
    // Up (or saying hello) on this path: nothing to do. Not `socket.connected`:
    // that is the state asked for, true during an attempt that may yet fail,
    // and returning on it (with the retry stopped) left the client with no
    // attempt pending at all.
    if (client._status !== "disconnected" && socket.path === client.socketPath) return
    retryTimer.stop()
    client._restarting = true
    socket.connected = false
    client._restarting = false
    socket.path = client.socketPath
    socket.connected = true
  }

  // ------------------------------------------------------------ internals
  property string _status: "disconnected"
  property string _error: ""
  property var _welcome: null
  property int _attempts: 0
  property var _store: Protocol.emptyStore()
  property string _buffer: ""
  property int _nextId: 1
  property var _pending: ({})
  property var _unknownLogged: ({})
  property bool _restarting: false
  property bool _started: false

  function _send(msg) {
    if (!socket.connected) return false
    socket.write(Protocol.encode(msg))
    socket.flush()
    return true
  }

  function _request(build, reply) {
    var id = client._nextId
    if (!client.ready && client._status !== "connecting") {
      if (reply) reply(Protocol.droppedReply(id))
      return -1
    }
    client._nextId = id + 1
    if (reply) client._pending[id] = reply
    if (!client._send(build(id))) {
      delete client._pending[id]
      if (reply) reply(Protocol.droppedReply(id))
      return -1
    }
    return id
  }

  // Answers every request still waiting, as the daemon is gone.
  function _failPending() {
    var waiting = client._pending
    client._pending = ({})
    for (var id in waiting) waiting[id](Protocol.droppedReply(Number(id)))
  }

  function _reset() {
    client._buffer = ""
    client._store = Protocol.emptyStore()
    client._welcome = null
  }

  function _scheduleRetry() {
    if (!client.enabled || client.socketPath === "" || client._status === "refused" || client._restarting || retryTimer.running) return
    retryTimer.interval = Protocol.backoff(client._attempts, client.reconnectMin, client.reconnectMax)
    client._attempts++
    retryTimer.start()
  }

  // Asks for an attempt on the backoff unless one is pending; a no-op while up.
  function ensureRetry() {
    if (client._status === "disconnected") client._scheduleRetry()
  }

  // Drops a connection that said hello and never got a welcome, and tries
  // again: the socket answered but no daemon behind it does.
  function reconnect() {
    if (client.ready || client._status === "refused") return
    client._restarting = true
    socket.connected = false
    client._restarting = false
    client._status = "disconnected"
    client.connectNow()
  }

  function _closed() {
    var wasUp = client._status !== "disconnected"
    if (client._status !== "refused") client._status = "disconnected"
    client._failPending()
    client._reset()
    if (wasUp) client.disconnected()
    client._scheduleRetry()
  }

  function _onConnected() {
    client._status = "connecting"
    client._error = ""
    client._send(Protocol.hello(client.clientName, client.clientVersion, client.instance, client.caps))
  }

  function _feed(chunk) {
    var framed = Protocol.frame(client._buffer, chunk)
    client._buffer = framed.rest
    if (framed.overflow) console.warn("notchd client: a line over the limit was dropped")
    for (var i = 0; i < framed.lines.length; i++) client._handle(framed.lines[i])
  }

  function _handle(line) {
    var msg = Protocol.parse(line)
    if (msg === null) {
      client._logUnknownOnce("unparsable", "notchd client: ignoring a line that is not a message")
      return
    }
    if (msg.t === "welcome") {
      client._welcome = msg
      client._status = "ready"
      client._attempts = 0
      client.welcomed(msg)
      return
    }
    if (Protocol.isRefusal(msg)) {
      client._status = "refused"
      client._error = "the daemon speaks protocol " + msg.want + ", this client " + Protocol.PROTO
      retryTimer.stop()
      client.refused(msg)
      return
    }
    if (Protocol.isReply(msg)) {
      var reply = client._pending[msg.id]
      delete client._pending[msg.id]
      if (reply) reply(msg)
      return
    }
    var applied = Protocol.apply(client._store, msg)
    if (applied.key !== "") {
      if (!applied.applied) return
      client._store = applied.store
      client.updated(msg.t, applied.key, msg)
      return
    }
    if (msg.t === "err") console.warn("notchd client: the daemon answered " + msg.code + ": " + (msg.message || ""))
    client.message(msg)
  }

  function _logUnknownOnce(key, text) {
    if (client._unknownLogged[key]) return
    client._unknownLogged[key] = true
    console.warn(text)
  }

  readonly property Socket socket: Socket {
    id: socket
    parser: SplitParser {
      // No marker: every read arrives as it came, and Protocol.frame splits it.
      splitMarker: ""
      onRead: data => client._feed(data)
    }
    onConnectedChanged: {
      if (connected) client._onConnected()
      else client._closed()
    }
    onError: client._closed()
  }

  readonly property Timer retryTimer: Timer {
    id: retryTimer
    onTriggered: client.connectNow()
  }

  // A new path is a new daemon: a refusal does not carry over.
  onSocketPathChanged: {
    if (!client._started) return
    if (client._status === "refused") client._status = "disconnected"
    client.connectNow()
  }
  onEnabledChanged: {
    if (client.enabled) client.connectNow()
    else {
      retryTimer.stop()
      socket.connected = false
    }
  }
  Component.onCompleted: {
    client._started = true
    client.connectNow()
  }
}
