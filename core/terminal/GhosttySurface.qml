import QtQuick
import Quickshell
import Quickshell.Io
import ".."
import "../client/Protocol.js" as Protocol
import "../Terminal.js" as Terminal

// A terminal `surface` drawn as a grid (the `ghostty` renderer, experimental):
// the daemon emulates the terminal with Ghostty's libghostty-vt and sends the
// screen as rows of styled runs (`attach` with `render: grid`,
// docs/architecture.md, "Free surface"); this draws them and sends keys,
// which the daemon encodes for the program (legacy, or the kitty keyboard
// protocol when the program asks). Nothing here emulates a terminal, and it
// needs no package: text, rectangles and a socket of its own.
//
// The same contract as TerminalSurface.qml: SurfacePrim sets `host`,
// `reference` and `run`; it fires `typed`, `leave` (Ctrl+Escape),
// `keyboard` (a press) and `restart` (Enter once the program ended).
Item {
  id: surface

  property var host: null
  property string reference: ""
  property string run: ""
  signal fired(string actionId, var value)

  readonly property string moduleId: surface.reference.replace(/^term:/, "")
  readonly property string socketPath: surface.host ? String(surface.host.client.socketPath || "") : ""
  readonly property bool ready: surface.socketPath !== "" && surface.moduleId !== "" && screenArea.width > 0
  readonly property string fontFamily: (surface.host ? String(surface.host.moduleSetting(surface.moduleId, "font") || "") : "") || Theme.font.family

  // The title the program set, and whether it ended.
  property string title: ""
  property bool exited: false
  // Why the daemon refused the view (a daemon built without the renderer).
  property string error: ""

  FontMetrics {
    id: metrics
    font.family: surface.fontFamily
    font.pixelSize: Theme.font.body
  }
  readonly property real cellW: Math.max(1, metrics.advanceWidth("M"))
  readonly property real cellH: Math.max(1, Math.ceil(metrics.height))
  readonly property int cols: Math.max(2, Math.floor(screenArea.width / surface.cellW))
  readonly property int rows: Math.max(2, Math.floor(screenArea.height / surface.cellH))

  // What the daemon last said: the grid's size and the cursor.
  property int gridRows: 0
  property var cursor: null
  // The program asked for the mouse: clicks, drags and the wheel go to it.
  property bool mouse: false
  // Kitty images: where they are (the grid's `images`) and each one's
  // picture by id (`image` events), as data URLs.
  property var images: []
  property var imageSources: ({})

  // ------------------------------------------------------------ the wire
  property string _buffer: ""
  property string _sentSize: ""
  function send(ev) {
    if (!socket.connected) return
    ev.t = "surface"
    ev.ref = surface.reference
    socket.write(Protocol.encode(ev))
    socket.flush()
  }
  function _hex(c) {
    function two(v) { var s = Math.round(v * 255).toString(16); return s.length < 2 ? "0" + s : s }
    return "#" + two(c.r) + two(c.g) + two(c.b)
  }
  // The theme's ANSI colors as plain strings ([] when it has none).
  function _palette() {
    var p = Theme.terminalPalette, out = []
    for (var i = 0; p && i < p.length && i < 16; i++) out.push(String(p[i]))
    return out
  }
  function _onConnected() {
    surface._buffer = ""
    // A new view: the daemon sends every image again.
    surface.images = []
    surface.imageSources = ({})
    surface.exited = false
    surface.error = ""
    surface._sentSize = surface.cols + "x" + surface.rows
    var attach = { t: "attach", ref: surface.reference, cols: surface.cols, rows: surface.rows,
                   render: "grid", fg: surface._hex(Theme.foreground), bg: surface._hex(Theme.background),
                   cw: Math.round(surface.cellW), ch: Math.round(surface.cellH) }
    var palette = surface._palette()
    if (palette.length > 0) attach.palette = palette
    socket.write(Protocol.encode(attach))
    socket.flush()
  }
  function _feed(chunk) {
    var framed = Protocol.frame(surface._buffer, chunk)
    surface._buffer = framed.rest
    for (var i = 0; i < framed.lines.length; i++) {
      var msg = Protocol.parse(framed.lines[i])
      if (msg === null) continue
      if (msg.t === "surface" && msg.ev === "grid") surface._frame(msg)
      else if (msg.t === "surface" && msg.ev === "title") surface.title = String(msg.text || "")
      // The daemon copied (Ctrl+Shift+C, or the program's OSC 52), or wants
      // the clipboard to paste (Ctrl+Shift+V).
      else if (msg.t === "surface" && msg.ev === "copy") Quickshell.clipboardText = String(msg.text || "")
      else if (msg.t === "surface" && msg.ev === "image") {
        var sources = Object.assign({}, surface.imageSources)
        sources[msg.id] = "data:image/png;base64," + String(msg.png || "")
        surface.imageSources = sources
      }
      else if (msg.t === "surface" && msg.ev === "clipboard") surface.send({ ev: "paste", text: String(Quickshell.clipboardText || "") })
      else if (msg.t === "surface-close") surface.exited = true
      else if (msg.t === "err") surface.error = String(msg.message || msg.code || "error")
    }
  }
  function _frame(g) {
    if (surface.gridRows !== g.rows) surface.gridRows = g.rows
    var lines = g.lines || []
    for (var i = 0; i < lines.length; i++) {
      var row = rowRepeater.itemAt(lines[i].y) as GridRow
      if (!row) continue
      row.runs = lines[i].runs || []
      row.sel = lines[i].sel || null
    }
    surface.cursor = g.cursor || null
    surface.mouse = g.mouse === true
    if (g.images !== undefined) surface.images = g.images
  }

  Socket {
    id: socket
    parser: SplitParser {
      splitMarker: ""
      onRead: data => surface._feed(data)
    }
    onConnectedChanged: {
      if (connected) surface._onConnected()
      else if (surface.launchedKey !== "") surface.exited = true
    }
  }

  // A new run (a restart) or a new daemon is a new connection.
  // The theme's colors go with the attach: a new theme is a new connection.
  readonly property string colorsKey: surface._hex(Theme.foreground) + surface._hex(Theme.background)
    + surface._palette().join("")
  onColorsKeyChanged: launchTimer.restart()
  property string launchedKey: ""
  function launch() {
    var key = surface.socketPath + " " + surface.reference + " " + surface.run + " " + surface.colorsKey
    if (!surface.ready || key === surface.launchedKey) return
    surface.launchedKey = key
    socket.connected = false
    socket.path = surface.socketPath
    socket.connected = true
    surface.typedSent = false
  }
  // A Timer of its own, not Qt.callLater (TerminalSurface.qml says why).
  onReadyChanged: launchTimer.restart()
  onRunChanged: launchTimer.restart()
  onReferenceChanged: launchTimer.restart()
  Timer {
    id: launchTimer
    interval: 0
    onTriggered: surface.launch()
  }
  Component.onDestruction: socket.connected = false

  // The view's size, once it settles.
  onColsChanged: resizeTimer.restart()
  onRowsChanged: resizeTimer.restart()
  Timer {
    id: resizeTimer
    interval: 60
    onTriggered: {
      var size = surface.cols + "x" + surface.rows
      if (!socket.connected || size === surface._sentSize) return
      surface._sentSize = size
      surface.send({ ev: "resize", cols: surface.cols, rows: surface.rows,
                     cw: Math.round(surface.cellW), ch: Math.round(surface.cellH) })
    }
  }

  // ------------------------------------------------------------ drawing
  // A row of the grid: its runs, which a frame replaces.
  component GridRow: Item {
    property var runs: []
    // The selected columns, [first, last], or null.
    property var sel: null
  }

  Rectangle {
    anchors.fill: parent
    radius: Theme.radius
    color: Theme.background
    border.width: 1
    border.color: Theme.line
    clip: true

    Item {
      id: screenArea
      anchors.fill: parent
      anchors.margins: Theme.space(8)
      clip: true

      Repeater {
        id: rowRepeater
        model: surface.gridRows
        delegate: GridRow {
          id: rowItem
          required property int index
          y: rowItem.index * surface.cellH
          width: screenArea.width
          height: surface.cellH
          Rectangle {
            visible: rowItem.sel !== null
            x: rowItem.sel ? rowItem.sel[0] * surface.cellW : 0
            width: rowItem.sel ? (rowItem.sel[1] - rowItem.sel[0] + 1) * surface.cellW : 0
            height: parent.height
            color: Theme.alpha(Theme.accent, 0.35)
          }
          Repeater {
            model: rowItem.runs
            delegate: Item {
              id: run
              required property var modelData
              readonly property int attrs: Number(run.modelData.a || 0)
              x: run.modelData.x * surface.cellW
              width: run.modelData.w * surface.cellW
              height: surface.cellH
              Rectangle {
                anchors.fill: parent
                visible: !!run.modelData.bg
                color: run.modelData.bg || "transparent"
              }
              Text {
                height: parent.height
                verticalAlignment: Text.AlignVCenter
                textFormat: Text.PlainText
                text: run.modelData.text
                color: run.modelData.fg || Theme.foreground
                opacity: (run.attrs & 16) ? 0.6 : 1
                font.family: surface.fontFamily
                font.pixelSize: Theme.font.body
                font.bold: (run.attrs & 1) !== 0
                font.italic: (run.attrs & 2) !== 0
                font.underline: (run.attrs & 4) !== 0
                font.strikeout: (run.attrs & 8) !== 0
                font.kerning: false
              }
            }
          }
        }
      }

      // Over the text, or under it (z below 0).
      Repeater {
        model: surface.images
        delegate: Image {
          id: picture
          required property var modelData
          x: picture.modelData.x * surface.cellW + picture.modelData.xo
          y: picture.modelData.y * surface.cellH + picture.modelData.yo
          z: picture.modelData.z < 0 ? -1 : 1
          width: picture.modelData.w
          height: picture.modelData.h
          source: surface.imageSources[picture.modelData.id] || ""
          sourceClipRect: Qt.rect(picture.modelData.sx, picture.modelData.sy, picture.modelData.sw, picture.modelData.sh)
          smooth: true
          mipmap: true
        }
      }

      Rectangle {
        id: cursorBox
        z: 2
        readonly property string style: surface.cursor ? String(surface.cursor.style) : ""
        visible: surface.cursor !== null && !surface.exited
        x: surface.cursor ? surface.cursor.x * surface.cellW : 0
        y: surface.cursor ? surface.cursor.y * surface.cellH + (cursorBox.style === "underline" ? surface.cellH - 2 : 0) : 0
        width: cursorBox.style === "bar" ? 2 : surface.cellW
        height: cursorBox.style === "underline" ? 2 : surface.cellH
        color: cursorBox.style === "hollow" || !keyArea.activeFocus ? "transparent" : Theme.foreground
        opacity: cursorBox.style === "block" ? 0.55 : 1
        border.width: cursorBox.style === "hollow" || !keyArea.activeFocus ? 1 : 0
        border.color: Theme.foreground
      }
    }

    Text {
      visible: surface.error !== ""
      anchors.centerIn: parent
      width: parent.width - Theme.space(24)
      horizontalAlignment: Text.AlignHCenter
      wrapMode: Text.Wrap
      textFormat: Text.PlainText
      text: surface.error
      color: Theme.muted
      font.family: Theme.font.family
      font.pixelSize: Theme.font.body
    }

    MouseArea {
      id: pointer
      anchors.fill: parent
      acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
      // Motion with no button held only matters to a program that asked.
      hoverEnabled: surface.mouse
      // Shift selects even when the program has the mouse.
      cursorShape: surface.mouse && !(pointer.mods & Qt.ShiftModifier) ? Qt.ArrowCursor : Qt.IBeamCursor
      property int mods: 0
      property real wheelRest: 0
      // The last cell a motion was sent for: one event a cell, not a pixel.
      property string lastCell: ""

      // Where, in cells of the grid (screenArea's), fractions kept.
      function cellAt(px, py) {
        var p = pointer.mapToItem(screenArea, px, py)
        return { x: Math.max(0, p.x / surface.cellW), y: Math.max(0, p.y / surface.cellH) }
      }
      function sendMouse(action, button, px, py, modifiers) {
        pointer.mods = modifiers
        var c = pointer.cellAt(px, py)
        var ev = { ev: "mouse", action: action, x: c.x, y: c.y, mods: Terminal.gridMods(modifiers) }
        if (button !== "") ev.button = button
        surface.send(ev)
      }
      // Clicks and drags always go to the daemon: it gives them to the
      // program or makes a selection of them.
      onPressed: function(mouse) {
        surface.fired("keyboard", null)
        keyArea.forceActiveFocus()
        pointer.lastCell = ""
        pointer.sendMouse("press", Terminal.gridButton(mouse.button), mouse.x, mouse.y, mouse.modifiers)
      }
      onReleased: function(mouse) {
        pointer.sendMouse("release", Terminal.gridButton(mouse.button), mouse.x, mouse.y, mouse.modifiers)
      }
      onPositionChanged: function(mouse) {
        var held = pointer.pressedButtons & Qt.LeftButton ? "left"
          : pointer.pressedButtons & Qt.MiddleButton ? "middle"
          : pointer.pressedButtons & Qt.RightButton ? "right" : ""
        if (held === "" && !surface.mouse) return
        var c = pointer.cellAt(mouse.x, mouse.y)
        var cell = Math.floor(c.x) + "," + Math.floor(c.y)
        if (cell === pointer.lastCell) return
        pointer.lastCell = cell
        pointer.sendMouse("motion", held, mouse.x, mouse.y, mouse.modifiers)
      }
      onWheel: function(wheel) {
        // 120 per notch, three rows a notch.
        wheelRest += -wheel.angleDelta.y / 40
        var n = wheelRest > 0 ? Math.floor(wheelRest) : Math.ceil(wheelRest)
        if (n !== 0) {
          wheelRest -= n
          var c = pointer.cellAt(wheel.x, wheel.y)
          surface.send({ ev: "wheel", rows: n, x: c.x, y: c.y })
        }
      }
    }
  }

  // ------------------------------------------------------------ keys
  property bool typedSent: false
  // The keyboard. A TextInput, not an Item: only an item that takes input
  // methods hears what one composes (fcitx, ibus, dead keys: ´ then a is
  // "á", which never comes as a key). Keys go out as `key` events; what the
  // input method commits lands in the text and goes out as typed text.
  TextInput {
    id: keyArea
    objectName: "terminal"
    focus: true
    width: 0
    height: 0
    opacity: 0
    cursorVisible: false
    inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase | Qt.ImhPreferLatin
    Component.onCompleted: keyArea.forceActiveFocus()
    function typed() {
      if (surface.typedSent) return
      surface.typedSent = true
      surface.fired("typed", null)
    }
    onTextChanged: {
      if (keyArea.text === "") return
      var t = keyArea.text
      keyArea.text = ""
      if (surface.exited) return
      surface.send({ ev: "key", key: "", text: t })
      keyArea.typed()
    }
    Keys.priority: Keys.BeforeItem
    Keys.onPressed: function(event) {
      if (Terminal.isLeaveKey(event.key, event.modifiers)) {
        event.accepted = true
        surface.fired("leave", null)
        return
      }
      if (surface.exited) {
        event.accepted = true
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) surface.fired("restart", null)
        return
      }
      var k = Terminal.gridKey(event.key, event.modifiers, event.text)
      // A dead key or one with nothing to send: the input method's.
      if (k === null) {
        event.accepted = false
        return
      }
      event.accepted = true
      k.ev = "key"
      surface.send(k)
      keyArea.typed()
    }
  }
  // Text to the program, as if typed (a test, a command over IPC).
  function write(text) {
    surface.send({ ev: "input", b64: Qt.btoa(String(text)) })
  }
  function forceActiveFocus() { keyArea.forceActiveFocus() }
}
