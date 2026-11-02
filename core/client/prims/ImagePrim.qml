import QtQuick
import Quickshell.Widgets
import "../.."
import ".."

// Cover art, as `Art` in core/modules/media.qml draws it: the picture cropped
// into a rounded square over a `selected` fill, the `fallback` icon while it
// loads and when it fails. `radius`: none, small (Theme.radiusSmall) or round
// (Theme.radius, the 58 px cover's). ClippingRectangle rounds it with a
// shader; the software renderer (offscreen renders, QT_QUICK_BACKEND=software)
// has none and would draw nothing, so there the corners stay square.
PrimBase {
  id: prim
  implicitWidth: prim.px(prim.node.w, prim.node.size === undefined ? 38 : prim.node.size)
  implicitHeight: prim.px(prim.node.h, prim.node.size === undefined ? 38 : prim.node.size)
  readonly property real corner: prim.node.radius === "round" ? Theme.radius : prim.node.radius === "small" ? Theme.radiusSmall : 0
  readonly property bool shaders: GraphicsInfo.api !== GraphicsInfo.Software
  readonly property string src: {
    var s = prim.node.src === undefined ? "" : String(prim.node.src)
    return s === "" || s.indexOf("://") > 0 ? s : "file://" + s
  }

  Loader {
    anchors.fill: parent
    sourceComponent: prim.shaders && prim.corner > 0 ? rounded : square
  }
  Component {
    id: rounded
    ClippingRectangle { radius: prim.corner; color: Theme.selected; ImageContent {} }
  }
  Component {
    id: square
    Rectangle { color: Theme.selected; clip: true; ImageContent {} }
  }

  component ImageContent: Item {
    anchors.fill: parent
    Image {
      id: picture
      anchors.fill: parent
      source: prim.src
      fillMode: prim.node.fit === "contain" ? Image.PreserveAspectFit : Image.PreserveAspectCrop
      sourceSize.width: prim.width * 2
      sourceSize.height: prim.height * 2
      asynchronous: true
      mipmap: true
      visible: status === Image.Ready
    }
    // The fallback node (an icon), under the same rules as any node.
    ViewTree {
      anchors.centerIn: parent
      visible: picture.status !== Image.Ready
      tree: prim.node.fallback === undefined ? null : prim.node.fallback
      depth: prim.depth + 1
    }
  }
}
