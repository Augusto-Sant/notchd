import QtQuick
import QtQuick.Layouts
import ".."

// `component` field: one optional component (core/components/components.json)
// as a row: what state it is in, a bar while it installs, and the buttons that
// make sense (Install, Cancel, Remove). It has no config value: the state is
// the daemon's, in the field's `row` (the schema), and a button is the `component`
// IPC, so `value` is unused and `edited` never fires; it takes the contract's
// `field`, `value` and `edited` anyway.
// Removing asks twice (a click, then Sure? within three seconds): a 6 GB
// environment is a long download to get back.
ColumnLayout {
  id: control

  property var field: ({})
  property var value
  property var svc: null
  signal edited(var value)

  readonly property string componentId: control.field.component ? String(control.field.component) : ""
  readonly property var info: control.field.row ? control.field.row : null
  readonly property bool working: !!control.info && control.info.state === "working"
  property bool confirming: false

  function act(id) {
    if (!control.svc || control.componentId === "") return
    if (id === "remove" && !control.confirming) { control.confirming = true; confirmTimer.restart(); return }
    control.confirming = false
    control.svc.notchIpc("component", [id === "remove" || id === "cancel" ? id : "install", control.componentId])
  }

  spacing: Theme.space(8)

  Timer {
    id: confirmTimer
    interval: 3000
    onTriggered: control.confirming = false
  }

  RowLayout {
    Layout.fillWidth: true
    visible: !!control.info
    spacing: Theme.space(14)

    Label {
      Layout.fillWidth: true
      wrapMode: Text.Wrap
      text: control.info ? control.info.text : ""
      color: !control.info ? Theme.muted
        : control.info.state === "failed" ? Theme.danger
        : control.info.state === "working" ? Theme.foreground : Theme.muted
    }
    Label {
      visible: !!control.info && control.info.state !== "working"
      wrapMode: Text.NoWrap
      role: "help"
      color: Theme.faint
      text: control.info ? control.info.size : ""
    }
    Repeater {
      model: control.info ? control.info.actions : []
      delegate: Button {
        required property var modelData
        text: modelData.id === "remove" && control.confirming ? "Sure?" : modelData.label
        onClicked: control.act(modelData.id)
      }
    }
  }

  // The bar: the percent when it is known, else a stretch that sweeps.
  Rectangle {
    id: track
    visible: control.working
    Layout.fillWidth: true
    Layout.preferredHeight: Theme.space(4)
    radius: height / 2
    color: Theme.line
    clip: true

    Rectangle {
      id: fill
      readonly property bool known: !!control.info && control.info.percent >= 0
      height: parent.height
      radius: parent.radius
      color: Theme.accent
      width: known ? parent.width * control.info.percent / 100 : parent.width * 0.3
      property real sweep: 0
      x: known ? 0 : sweep
      Behavior on width { enabled: fill.known; NumberAnimation { duration: 250 } }
      SequentialAnimation on sweep {
        running: track.visible && !fill.known
        loops: Animation.Infinite
        NumberAnimation { from: -fill.width; to: track.width; duration: 1400 }
      }
    }
  }
}
