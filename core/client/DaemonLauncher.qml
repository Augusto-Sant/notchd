import QtQuick
import Quickshell
import Quickshell.Io

// Starts the notchd daemon with the shell and connects a DaemonClient to it
// (docs/hosts.md, "The daemon"). It runs `scripts/ensure-daemon.sh`, which
// finds the binary, starts the daemon detached unless one runs, and prints
// the socket path; on failure it prints one line of JSON, {"error": "..."}.
//
// The daemon is the authority for the config, the settings schema and IPC
// (NotchService binds to this launcher's client), so the launcher also brings
// it back: when the connection drops, or the script fails, the script runs
// again after a short backoff (`restartMin`, doubling up to `restartMax`) for
// as long as the shell runs. The script is idempotent: a daemon that is up is
// reported, not replaced. Meanwhile the service shows "notchd not running"
// (NotchService.daemonDown).
QtObject {
  id: launcher

  // The ensure script, as a file URL (`Qt.resolvedUrl("../../scripts/ensure-daemon.sh")`
  // from the host). `command` overrides what runs (tests).
  property url script
  property var command: ["bash", launcher.scriptPath]
  readonly property string scriptPath: decodeURIComponent(String(launcher.script).replace(/^file:\/\//, ""))
  // Which daemon: the instance of the hello. The script reads NOTCHD_INSTANCE
  // from the same environment, so the two agree; `default` unless a dev
  // run says otherwise.
  property string instance: Quickshell.env("NOTCHD_INSTANCE") || "default"
  property string clientName: "quickshell"
  property string clientVersion: ""
  property string logName: "notchd"
  property bool enabled: true
  // The backoff between runs of the script once the daemon went away.
  property int restartMin: 300
  property int restartMax: 5000

  // "idle" until started, "starting", "ready" (the socket is known, the
  // client connects), "failed" (`error` says why).
  readonly property string state: launcher._state
  readonly property string error: launcher._error
  readonly property string socketPath: launcher._socketPath
  // Runs of the script since the daemon was last up.
  readonly property int restarts: launcher._restarts
  readonly property DaemonClient client: connectionClient

  property string _state: "idle"
  property string _error: ""
  property string _socketPath: ""
  property bool _exited: false
  property int _code: 0
  property bool _collected: false
  property string _out: ""
  property int _restarts: 0
  property string _logged: ""

  // Runs the script (again, after a failure, say).
  function start() {
    if (!launcher.enabled || launcher._state === "starting") return
    launcher._state = "starting"
    launcher._exited = false
    launcher._collected = false
    starterProcess.running = true
  }

  // Runs the script again after the backoff: the daemon is gone or never
  // came up. A run in flight, or one already waiting, makes this a no-op.
  function _respawn() {
    if (!launcher.enabled || launcher._state === "starting" || restartTimer.running) return
    var delay = launcher.restartMin
    for (var i = 0; i < launcher._restarts && delay < launcher.restartMax; i++) delay *= 2
    restartTimer.interval = Math.min(delay, launcher.restartMax)
    launcher._restarts++
    restartTimer.start()
  }

  // Once the script has exited and its output is in: the last line is the
  // socket path, or the JSON error.
  function _finish() {
    if (!launcher._exited || !launcher._collected) return
    var lines = launcher._out.trim().split("\n")
    var last = lines[lines.length - 1].trim()
    if (launcher._code === 0 && last !== "") {
      launcher._error = ""
      launcher._socketPath = last
      launcher._state = "ready"
      // The path may be the one the client already has (a daemon that came
      // back where it was): it retries on its own backoff, so nudge it.
      connectionClient.connectNow()
      return
    }
    var why = last
    try { why = JSON.parse(last).error || last } catch (e) {}
    launcher._error = why !== "" ? why : "the start script exited with " + launcher._code
    launcher._state = "failed"
    // One line per kind of failure, not one per retry.
    if (launcher._error !== launcher._logged) console.warn(launcher.logName + ": the notchd daemon did not start: " + launcher._error)
    launcher._logged = launcher._error
    launcher._respawn()
  }

  readonly property Timer restartTimer: Timer {
    id: restartTimer
    onTriggered: launcher.start()
  }

  readonly property Process starter: Process {
    id: starterProcess
    command: launcher.command
    stdout: StdioCollector {
      onStreamFinished: {
        launcher._out = text
        launcher._collected = true
        launcher._finish()
      }
    }
    onExited: (code, status) => {
      launcher._code = code
      launcher._exited = true
      launcher._finish()
    }
  }

  readonly property DaemonClient connection: DaemonClient {
    id: connectionClient
    clientName: launcher.clientName
    clientVersion: launcher.clientVersion
    instance: launcher.instance
    socketPath: launcher._socketPath
  }

  // The daemon went away: bring it back. A client that keeps failing to
  // connect to a socket nobody answers on says so too (the daemon died
  // while the script ran, or never came up).
  readonly property Connections watcher: Connections {
    target: connectionClient
    function onDisconnected() { launcher._respawn() }
    function onWelcomed(welcome) { launcher._restarts = 0; launcher._logged = "" }
    function onAttemptsChanged() {
      if (connectionClient.attempts >= 2 && launcher._state === "ready") launcher._respawn()
    }
  }

  // While the daemon is away there is always an attempt pending: whatever
  // race left the client with none (a reconnect asked for while an attempt
  // that then failed was in flight), it tries again within a few seconds.
  readonly property Timer keepTrying: Timer {
    interval: 2000
    repeat: true
    running: launcher.enabled && !connectionClient.ready && launcher._state === "ready"
    property int helloTicks: 0
    onRunningChanged: helloTicks = 0
    onTriggered: {
      // Hello sent two ticks ago and still no welcome: start over.
      helloTicks = connectionClient.status === "connecting" ? helloTicks + 1 : 0
      if (helloTicks >= 2) { helloTicks = 0; connectionClient.reconnect() }
      else connectionClient.ensureRetry()
    }
  }

  Component.onCompleted: launcher.start()
}
