import QtQuick
import "../core"

// A module that asks for its peek to show once, on the focused screen only
// (peekOnFocusedScreen, as the OSD does), for the peek routing tests.
NotchModule {
  id: m
  title: "Focus"
  active: true
  peekOnFocusedScreen: true
  compact: Component { Rectangle { implicitWidth: 60; implicitHeight: 12; color: "gray" } }
  peek: Component { Rectangle { implicitWidth: 200; implicitHeight: 40; color: "gray" } }
}
