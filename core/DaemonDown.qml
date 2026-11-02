import QtQuick
import Quickshell
import Quickshell.Wayland

// "notchd not running", on screen while the daemon is away
// (NotchService.daemonDown): a small row at the top of the screen you work
// on, under the host's bar, with no input of its own. The service keeps its
// last config meanwhile and the launcher brings the daemon back
// (client/DaemonLauncher.qml); the row goes when it answers.
PanelWindow {
  id: win

  property var svc: null
  // The host's bar thickness when it is on the top edge, as NotchSurface has it.
  property int barHeight: 0

  readonly property bool shown: !!win.svc && win.svc.daemonDown
  visible: win.shown
  screen: {
    var screens = Quickshell.screens || []
    var focused = win.svc && win.svc.compositor ? win.svc.compositor.focusedScreenName : ""
    for (var i = 0; i < screens.length; i++) if (screens[i].name === focused) return screens[i]
    return screens.length > 0 ? screens[0] : null
  }

  anchors.top: true
  margins.top: win.svc && win.svc.barEdge === "top" ? win.barHeight : 0
  implicitWidth: down.implicitWidth
  implicitHeight: down.implicitHeight
  color: "transparent"
  exclusionMode: ExclusionMode.Ignore
  WlrLayershell.namespace: "notchd-down"
  WlrLayershell.layer: WlrLayer.Overlay
  mask: Region {}

  DaemonDownRow { id: down }
}
