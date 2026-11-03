import QtQuick
import ".."

// A terminal `surface` of a module's view (docs/architecture.md, "Free
// surface"): the frame and the terminal view (TerminalView.qml) attached to a
// program the daemon runs. The program lives in the daemon, so this view
// comes and goes with the notch (and with the shell) and the session goes on;
// each new view is told what was drawn so far.
//
// qmltermwidget can only run a program, not be fed bytes, so the view runs
// `notchd attach <ref>`: a small client of the daemon's surface connection
// whose output is the program's and whose input goes to it
// (native/notchd-daemon/src/attach.rs).
//
// It fires `typed` (once, at the first key: from then on the end of the
// program's work is news), `leave` (Ctrl+Escape), `keyboard` (a press: the
// notch's window takes the keyboard) and `restart` (Enter, once the program
// has ended). SurfacePrim sets `host` (the NotchService), `reference`
// (`term:<module>`) and `run` (the node's id: a new run is a new view).
Item {
  id: surface

  property var host: null
  property string reference: ""
  property string run: ""
  signal fired(string actionId, var value)

  readonly property string moduleId: surface.reference.replace(/^term:/, "")
  readonly property string exe: surface.host && surface.host.client.welcome ? String(surface.host.client.welcome.exe || "") : ""
  readonly property string socket: surface.host ? String(surface.host.client.socketPath || "") : ""
  readonly property bool ready: surface.exe !== "" && surface.socket !== "" && surface.moduleId !== ""
  // The terminal view, once it exists.
  readonly property var term: viewLoader.item
  readonly property bool exitedNow: !!surface.term && surface.term.exited

  // What the view was last made for: a new run (a restart) makes a new one.
  property string launchedKey: ""
  function launch() {
    var key = surface.socket + " " + surface.reference + " " + surface.run
    if (!surface.ready || key === surface.launchedKey) return
    surface.launchedKey = key
    viewLoader.active = false
    // The program and its arguments are set before the view starts it.
    viewLoader.setSource(Qt.resolvedUrl("TerminalView.qml"), {
      program: surface.exe,
      programArguments: ["--socket", surface.socket, "attach", surface.reference],
      fontFamily: surface.host ? String(surface.host.moduleSetting(surface.moduleId, "font") || "") : ""
    })
    viewLoader.active = true
    surface.typedSent = false
  }
  // Once every property SurfacePrim sets is in. A Timer of its own, not
  // Qt.callLater: the surface may go before it fires (a tab whose terminal
  // has the other renderer replaces it), and a queued call would then load
  // into a Loader that is gone (the shell crashed so).
  onReadyChanged: launchTimer.restart()
  onRunChanged: launchTimer.restart()
  onReferenceChanged: launchTimer.restart()
  Timer {
    id: launchTimer
    interval: 0
    onTriggered: surface.launch()
  }

  Rectangle {
    anchors.fill: parent
    radius: Theme.radius
    color: Theme.background
    border.width: 1
    border.color: Theme.line
    clip: true

    Loader {
      id: viewLoader
      anchors.fill: parent
      anchors.margins: Theme.space(8)
      anchors.rightMargin: Theme.space(8) + surface.nudge
      active: false
      onLoaded: (item as Item).forceActiveFocus()
    }

    // A press asks the notch's window for the keyboard; it goes on to the
    // terminal.
    MouseArea {
      anchors.fill: parent
      cursorShape: Qt.IBeamCursor
      onPressed: function(mouse) {
        surface.fired("keyboard", null)
        if (surface.term && !surface.term.exited) surface.term.forceActiveFocus()
        mouse.accepted = false
      }
    }
  }

  property bool typedSent: false
  Connections {
    target: surface.term
    ignoreUnknownSignals: true
    function onTyped() {
      if (surface.typedSent) return
      surface.typedSent = true
      surface.fired("typed", null)
    }
    function onLeave() { surface.fired("leave", null) }
  }

  // The program ended: Enter starts it again (the daemon's view has the
  // button too).
  Item {
    id: exitedKeys
    focus: surface.exitedNow
    onFocusChanged: if (focus) forceActiveFocus()
    Keys.onReturnPressed: surface.fired("restart", null)
    Keys.onEnterPressed: surface.fired("restart", null)
  }

  // Once the program is up (its first title), one column narrower and back:
  // the resize makes it draw its whole screen again. Claude Code sends
  // `CSI < u` (kitty keyboard) right after clearing it, which the widget
  // doesn't know and prints as a "u" in the corner.
  property real nudge: 0
  property bool settled: false
  readonly property string title: surface.term ? surface.term.title : ""
  onTitleChanged: if (surface.title !== "" && !surface.settled) { surface.settled = true; settleTimer.restart() }
  onTermChanged: surface.settled = false
  Timer {
    id: settleTimer
    interval: 600
    onTriggered: {
      var t = surface.term
      if (!t || !t.visible) return
      surface.nudge = Math.max(2, t.fontMetrics.width + 1)
      unsettleTimer.restart()
    }
  }
  Timer {
    id: unsettleTimer
    interval: 150
    onTriggered: surface.nudge = 0
  }
}
