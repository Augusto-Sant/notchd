import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import "Live.js" as Live

// Base type for a spoken conversation with an agent (modules/codex-live.qml):
// you talk, it answers aloud, and what it is asked to do it does in its
// folder. The voice goes through a bridge (core/live/codex-live.py: the
// agent's server, WebRTC, the microphone and the speakers); its lines are
// the conversation's state (Live.reduce). A disc swells with the voice as
// it plays.
//
// A conversation runs while a notch is open on the module by a click or a
// shortcut (pinned; a hover only shows it), from the moment it opens until
// it closes, or until you stop it (the button, IPC). The microphone is on
// only then; the closed notch shows a red dot while it is. Settings: its
// JSON file says `"live": { "command": ... }` and gets command, directory,
// voice and access (Settings.liveSettings). Over IPC (`live
// <id> <command>`), command(): "start", "stop", "toggle", "new", "mute",
// "unmute" (Live.parseCommand).
NotchModule {
  id: m
  priority: 45
  active: true
  icon: String.fromCodePoint(0xF036C)  // nf-md-microphone
  expandedWidth: 460

  // ------------------------------------------------------------ the module
  // The agent's name, in the notch's texts.
  property string appName: ""
  // The mark: an image, or the agent's initial in a tile without one.
  property url logoSource: ""
  // The voice runs on this machine (core/live/local-live.py) and joins the
  // agent's text session, instead of Codex's realtime voice.
  property bool localVoice: false

  // ----------------------------------------------------------------- state
  // The conversation (Live.initialState): its status, transcript, voice.
  property var conv: Live.initialState()
  readonly property bool running: Live.running(m.conv)
  readonly property bool live: m.conv.status === "live"
  readonly property bool speaking: Live.speaking(m.conv)
  readonly property bool working: m.running && m.conv.working
  readonly property string statusText: Live.statusText(m.conv)

  // A notch is open on it by a click or a shortcut (NotchService.liveOpen).
  readonly property bool open: !!m.notch && m.notch.liveOpen(m)
  // You stopped it with the notch open: it stays stopped until the notch
  // closes.
  property bool stopped: false
  readonly property bool wanted: Live.wanted(m.open, m.stopped)
  onOpenChanged: if (!m.open) m.stopped = false
  onWantedChanged: {
    if (m.wanted) { closeTimer.stop(); m.start() }
    else closeTimer.restart()
  }
  // The notch shut for a moment (a tab, a shortcut pressed twice) keeps it.
  Timer {
    id: closeTimer
    interval: 1500
    onTriggered: if (!m.wanted) m.stop()
  }

  // ---------------------------------------------------------- the bridge
  property var bridge: null
  function start() {
    if (m.bridge || !m.notch) return
    m.conv = Live.starting(m.conv)
    m.bridge = bridgeProcess.createObject(m, {
      command: m.localVoice
        ? Live.localBridgeArgs(m.notch.localLiveBridge, {
            agent: m.setting("agent"), command: m.setting("command"), session: m.setting("session"),
            cwd: m.setting("directory"), access: m.setting("access"), lang: m.setting("language"),
            whisperModel: m.setting("whisperModel"), voice: m.setting("voice")
          }, Quickshell.env("HOME"))
        : Live.bridgeArgs(m.notch.liveBridge, {
            codex: m.setting("command"), cwd: m.setting("directory"),
            voice: m.setting("voice"), access: m.setting("access")
          }, Quickshell.env("HOME")),
      running: true
    })
  }
  // Asks the bridge to end (it hangs up and closes the agent's server);
  // one that doesn't, in a few seconds, is made to.
  function stop() {
    if (!m.bridge) return
    m.send({ stop: true })
    killTimer.restart()
  }
  // A new session: the transcript cleared, the one running (if any) ended
  // and another started once it has (bridgeExited).
  property bool renewing: false
  function renew() {
    m.stopped = false
    if (m.bridge) { m.renewing = true; m.stop(); return true }
    m.conv = Live.initialState()
    if (m.open) { m.start(); return true }
    return m.openNotch()
  }
  function send(obj) { if (m.bridge) m.bridge.write(JSON.stringify(obj) + "\n") }
  function setMuted(on) { if (m.live) m.send({ mute: !!on }) }
  Timer {
    id: killTimer
    interval: 4000
    onTriggered: if (m.bridge) m.bridge.signal(15)
  }
  function bridgeLine(line) {
    var e = Live.parseLine(line)
    if (e) m.conv = Live.reduce(m.conv, e)
  }
  function bridgeExited(proc, code) {
    killTimer.stop()
    if (m.conv.status !== "closed") m.conv = Live.reduce(m.conv, { status: "closed" })
    // Not a fault of notchd's (no packages, no login): the view says it; the
    // log keeps it for an IPC caller.
    if (m.conv.error) console.info((m.notch ? m.notch.logName : "notchd") + ": " + m.moduleId + ": " + Live.errorText(m.conv))
    if (m.bridge === proc) m.bridge = null
    proc.destroy()
    if (m.renewing) {
      m.renewing = false
      m.conv = Live.initialState()
      if (m.wanted) m.start()
    }
  }
  Component {
    id: bridgeProcess
    Process {
      id: proc
      stdinEnabled: true
      stdout: SplitParser { onRead: function(line) { m.bridgeLine(line) } }
      onExited: function(code) { m.bridgeExited(proc, code) }
    }
  }
  Component.onDestruction: if (m.bridge) m.bridge.signal(15)

  // --------------------------------------------------------------- IPC
  function command(text) {
    var cmd = Live.parseCommand(text)
    switch (cmd.action) {
    case "start":
      m.stopped = false
      if (m.open) { m.start(); return "ok" }
      return m.openNotch() ? "ok" : "no notch shows " + m.moduleId + "; add it to one"
    case "stop":
      m.stopped = true
      m.stop()
      return "ok"
    case "toggle":
      return m.command(m.running ? "stop" : "start")
    case "new":
      return m.renew() ? "ok" : "no notch shows " + m.moduleId + "; add it to one"
    case "mute":
    case "unmute":
      if (!m.live) return "not in a conversation"
      m.setMuted(cmd.action === "mute")
      return "ok"
    }
    return "usage: live " + m.moduleId + " start | stop | toggle | new | mute | unmute"
  }

  // --------------------------------------------------------------- views
  // Closed, while it talks: a red dot (the microphone is on) and what it does.
  compact: m.running ? compactView : null
  Component {
    id: compactView
    Row {
      spacing: Theme.space(8)
      MicDot { anchors.verticalCenter: parent.verticalCenter }
      Text {
        anchors.verticalCenter: parent.verticalCenter
        textFormat: Text.PlainText
        text: m.appName + " · " + m.statusText
        color: Theme.foreground
        font.family: Theme.font.family
        font.pixelSize: Theme.font.bodySmall
      }
    }
  }

  expanded: Component {
    ColumnLayout {
      id: view
      spacing: Theme.space(10)

      // Header: the mark, the name, what it does, mute, stop or start, close.
      RowLayout {
        Layout.fillWidth: true
        spacing: Theme.space(10)
        Logo { size: Theme.font.heading }
        Text {
          textFormat: Text.PlainText
          text: m.title || m.appName
          color: Theme.foreground
          font.family: Theme.font.family
          font.pixelSize: Theme.font.body
        }
        MicDot { visible: m.running }
        Text {
          Layout.fillWidth: true
          textFormat: Text.PlainText
          text: m.running || m.conv.error ? m.statusText : ""
          elide: Text.ElideRight
          color: m.conv.error ? Theme.danger : Theme.muted
          font.family: Theme.font.family
          font.pixelSize: Theme.font.caption
        }
        IconButton {
          visible: m.live
          glyph: m.conv.muted ? 0xF036D : 0xF036C  // nf-md-microphone_off / microphone
          onActivated: m.setMuted(!m.conv.muted)
        }
        IconButton {
          visible: m.running
          glyph: 0xF04DB  // nf-md-stop
          onActivated: m.command("stop")
        }
        // A new session, the transcript cleared.
        IconButton {
          visible: m.running || m.conv.transcript.length > 0
          glyph: 0xF0709  // nf-md-restart
          onActivated: { m.requestKeyboard(); m.renew() }
        }
        IconButton {
          glyph: 0xF0156  // nf-md-close
          onActivated: m.closeNotch()
        }
      }

      // A disc that swells with the voice.
      Item {
        Layout.fillWidth: true
        Layout.preferredHeight: Theme.space(220)

        Rectangle {
          anchors.centerIn: parent
          readonly property real voice: m.speaking ? Math.min(1, Math.max(0, (m.conv.level + 55) / 40)) : 0
          width: Theme.space(96)
          height: width
          radius: width / 2
          scale: 1 + voice * .35
          Behavior on scale { NumberAnimation { duration: 90 } }
          color: m.running ? Theme.accent : Theme.surface
          opacity: m.running ? (m.live ? 1 : .5) : 1
          border.width: 1
          border.color: Theme.line
          Text {
            anchors.centerIn: parent
            textFormat: Text.PlainText
            text: String.fromCodePoint(m.conv.muted ? 0xF036D : 0xF036C)
            color: m.running ? Theme.background : Theme.muted
            font.family: Theme.font.family
            font.pixelSize: Theme.font.heading * 1.6
          }
        }
      }

      // Not talking: how to start, or what went wrong.
      ColumnLayout {
        Layout.fillWidth: true
        visible: !m.running
        spacing: Theme.space(6)
        Text {
          Layout.fillWidth: true
          visible: m.conv.error !== ""
          textFormat: Text.PlainText
          text: Live.errorText(m.conv)
          wrapMode: Text.Wrap
          color: Theme.danger
          font.family: Theme.font.family
          font.pixelSize: Theme.font.caption
        }
        Rectangle {
          Layout.alignment: Qt.AlignHCenter
          implicitWidth: startLabel.implicitWidth + Theme.space(32)
          implicitHeight: Theme.space(34)
          radius: Theme.radius
          color: startMouse.containsMouse ? Theme.selected : Theme.hover
          border.width: 1
          border.color: Theme.line
          Text {
            id: startLabel
            anchors.centerIn: parent
            textFormat: Text.PlainText
            text: m.conv.error ? "Try again" : "Talk to " + m.appName
            color: Theme.foreground
            font.family: Theme.font.family
            font.pixelSize: Theme.font.bodySmall
          }
          MouseArea {
            id: startMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            // A press pins the notch (which starts it, opened by a hover);
            // pinned already, it starts it.
            onClicked: { m.requestKeyboard(); m.command("start") }
          }
        }
      }

      // Something to know that doesn't end it (no echo cancellation, a
      // microphone that sends only silence).
      Text {
        Layout.fillWidth: true
        visible: m.running && m.conv.warning !== ""
        textFormat: Text.PlainText
        text: m.conv.warning
        wrapMode: Text.Wrap
        color: Theme.muted
        font.family: Theme.font.family
        font.pixelSize: Theme.font.caption
      }

      // What was said, the latest at the bottom: yours on the right.
      Item {
        Layout.fillWidth: true
        Layout.preferredHeight: Theme.space(150)
        visible: m.conv.transcript.length > 0
        clip: true
        Column {
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.bottom: parent.bottom
          spacing: Theme.space(6)
          Repeater {
            model: m.conv.transcript.slice(-8)
            delegate: Text {
              required property var modelData
              width: parent ? parent.width : 0
              textFormat: Text.PlainText
              text: modelData.text
              wrapMode: Text.Wrap
              horizontalAlignment: modelData.role === "user" ? Text.AlignRight : Text.AlignLeft
              color: modelData.role === "user" ? Theme.muted : Theme.foreground
              opacity: modelData.done ? 1 : .8
              font.family: Theme.font.family
              font.pixelSize: Theme.font.bodySmall
            }
          }
        }
      }
    }
  }

  // ---------------------------------------------------------- components
  component Logo: Item {
    id: logo
    property real size: Theme.font.body * 1.25
    implicitWidth: Math.round(logo.size)
    implicitHeight: implicitWidth
    Image {
      visible: m.logoSource.toString() !== ""
      anchors.fill: parent
      source: m.logoSource
      sourceSize.width: Math.round(logo.size) * 2
      sourceSize.height: Math.round(logo.size) * 2
      fillMode: Image.PreserveAspectFit
      smooth: true
      // Out of the window-sized texture atlas (docs/modules.md).
      mipmap: true
    }
    Rectangle {
      visible: m.logoSource.toString() === ""
      anchors.fill: parent
      radius: Math.round(logo.size / 4)
      color: Theme.foreground
      Text {
        anchors.centerIn: parent
        textFormat: Text.PlainText
        text: String.fromCodePoint(0xF036C)
        color: Theme.background
        font.family: Theme.font.family
        font.pixelSize: Math.max(6, Math.round(logo.size * 0.6))
      }
    }
  }
  // The microphone is on: red, swelling as it hears you; muted, faint.
  component MicDot: Item {
    implicitWidth: Theme.space(8)
    implicitHeight: implicitWidth
    Rectangle {
      anchors.centerIn: parent
      width: parent.width
      height: width
      radius: width / 2
      color: m.conv.muted ? Theme.muted : Theme.danger
      opacity: m.live ? 1 : .5
      scale: 1 + Live.micLevel(m.conv) * .6
      Behavior on scale { NumberAnimation { duration: 90 } }
    }
  }
  component IconButton: Item {
    id: btn
    property int glyph: 0
    signal activated()
    implicitWidth: Theme.space(28)
    implicitHeight: implicitWidth
    Rectangle {
      anchors.fill: parent
      radius: width / 2
      color: btnMouse.containsMouse ? Theme.selected : "transparent"
    }
    Text {
      anchors.centerIn: parent
      textFormat: Text.PlainText
      text: String.fromCodePoint(btn.glyph)
      color: Theme.foreground
      font.family: Theme.font.family
      font.pixelSize: Theme.font.heading
    }
    MouseArea {
      id: btnMouse
      anchors.fill: parent
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      onClicked: btn.activated()
    }
  }
}
