import QtQuick
import Quickshell
import Quickshell.Io

// Starts the notchd daemon with the shell and connects a DaemonClient to it
// (docs/hosts.md, "The daemon"). It runs `scripts/ensure-daemon.sh`, which
// finds the binary, starts the daemon detached unless one runs, and prints
// the socket path; on failure it prints one line of JSON, {"error": "..."}.
//
// While the QML service still decides everything (the daemon runs in shadow)
// the client is only connected: nothing is shown, a failure is logged once.
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

  // "idle" until started, "starting", "ready" (the socket is known, the
  // client connects), "failed" (`error` says why).
  readonly property string state: launcher._state
  readonly property string error: launcher._error
  readonly property string socketPath: launcher._socketPath
  readonly property DaemonClient client: connectionClient

  property string _state: "idle"
  property string _error: ""
  property string _socketPath: ""
  property bool _exited: false
  property int _code: 0
  property bool _collected: false
  property string _out: ""

  // Runs the script (again, after a failure, say).
  function start() {
    if (!launcher.enabled || launcher._state === "starting") return
    launcher._state = "starting"
    launcher._exited = false
    launcher._collected = false
    starterProcess.running = true
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
      return
    }
    var why = last
    try { why = JSON.parse(last).error || last } catch (e) {}
    launcher._error = why !== "" ? why : "the start script exited with " + launcher._code
    launcher._state = "failed"
    console.warn(launcher.logName + ": the notchd daemon did not start: " + launcher._error)
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

  Component.onCompleted: launcher.start()
}
