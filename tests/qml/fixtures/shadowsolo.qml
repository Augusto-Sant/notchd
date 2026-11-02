import QtQuick
import "../core"

// A standalone module (shadowsolo.json) for the shadow test: its own notch,
// never in one of the config's.
NotchModule {
  id: m
  title: "Solo"
  peek: Component { Rectangle { implicitWidth: 120; implicitHeight: 30; color: "gray" } }
  expanded: Component { Rectangle { implicitHeight: 40; color: "gray" } }
}
