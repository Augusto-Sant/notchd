import QtQuick
import "../core"

// A module whose facts the shadow test sets (tests/qml/shadow.qml) on both the
// QML service and the daemon: active, priority, urgentPeek, attention,
// peekOnFocusedScreen, an icon, and whether it has a compact and a peek view.
NotchModule {
  id: m
  property bool hasCompact: true
  property bool hasPeek: true
  title: "Shadow"
  compact: m.hasCompact ? compactView : null
  peek: m.hasPeek ? peekView : null
  expanded: Component { Rectangle { implicitHeight: 40; color: "gray" } }
  readonly property Component compactView: Component { Rectangle { implicitWidth: 60; implicitHeight: 12; color: "gray" } }
  readonly property Component peekView: Component { Rectangle { implicitWidth: 200; implicitHeight: 40; color: "gray" } }
}
