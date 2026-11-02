import QtQuick
import QtQml
import Quickshell
import Quickshell.Io
import "core"
import "core/client"

// Every primitive of the view tree, drawn by core/client/ViewTree.qml from the
// shared fixtures (tests/protocol/tree/*.json, the `valid` entries) plus a few
// composed views (the media card, the Voice disc, the timer's presets) and the
// error rows of an unknown and an invalid node. One PNG per theme:
// primitives-dark and primitives-light. scripts/visual-test.sh runs it as its
// own config, offscreen, with NOTCHD_FIXTURES (tests/protocol), NOTCHD_COVER
// (a generated cover image) and NOTCHD_VISUAL_DIR set.
ShellRoot {
  id: root

  readonly property string outDir: Quickshell.env("NOTCHD_VISUAL_DIR")
  readonly property string fixtures: Quickshell.env("NOTCHD_FIXTURES")
  readonly property string cover: Quickshell.env("NOTCHD_COVER")

  readonly property var themes: [
    { name: "primitives-dark", palette: { foreground: "#cacccc", background: "#101315", accent: "#7aa2f7", fontFamily: "JetBrainsMono Nerd Font" } },
    { name: "primitives-light", palette: { foreground: "#2a2b30", background: "#f6f5f1", accent: "#3b6fd8", fontFamily: "JetBrainsMono Nerd Font" } }
  ]
  property int step: -1

  readonly property var names: ["row", "column", "stack", "spacer", "text", "icon", "image", "progress", "meter",
    "equalizer", "list", "button", "chip", "canvas", "surface"]

  // `valid` entries of each fixture file: [{ name, tree }, ...].
  property var cells: []
  Component.onCompleted: root.cells = root.load()
  function load() {
    var out = []
    for (var i = 0; i < root.names.length; i++) {
      var parsed = JSON.parse((readers.objectAt(i) as FileView).text())
      for (var j = 0; j < parsed.valid.length; j++) {
        var tree = parsed.valid[j]
        // The fixture's image path is a stand-in: the first gets the real cover.
        if (tree.t === "image" && j === 0 && root.cover !== "") tree = Object.assign({}, tree, { src: root.cover, size: 48, radius: "small" })
        // An anchor in the past would show a full bar: anchor it to now.
        if (tree.t === "progress" && tree.anchor) tree = Object.assign({}, tree, { anchor: { at: Date.now(), rate: tree.anchor.rate } })
        // Nothing reaches the network offscreen: a remote source is an empty one (the fallback shows).
        if (tree.t === "image" && tree.src.indexOf("https://") === 0) tree = Object.assign({}, tree, { src: "" })
        out.push({ name: parsed.message + " " + (j + 1), tree: tree })
      }
    }
    return out
  }

  Instantiator {
    id: readers
    model: root.names
    delegate: FileView {
      required property string modelData
      path: root.fixtures + "/tree/" + modelData + ".json"
      blockLoading: true
    }
  }

  // The modules' views, by hand.
  function text(s, tone, size) { return { t: "text", text: s, tone: tone, size: size } }
  function button(glyph, action, extra) { return Object.assign({ t: "button", glyph: glyph, action: action }, extra || {}) }
  readonly property var mediaView: ({ t: "column", gap: 10, children: [
    { t: "row", gap: 14, children: [
      { t: "image", src: root.cover, size: 58, radius: "round", fit: "crop", fallback: { t: "icon", glyph: "󰝚", tone: "muted", size: 24 } },
      { t: "column", fill: true, gap: 2, children: [
        { t: "text", text: "Midnight Signal", size: "body" },
        { t: "text", text: "The Fictional Quartet", tone: "muted", size: "bodySmall" },
        { t: "text", text: "demo player", tone: "faint", size: "caption" } ] },
      { t: "equalizer", active: true } ] },
    { t: "progress", value: 0.4, anchor: { at: Date.now() - 1000, rate: 0.002 }, seek: "seek", tone: "foreground" },
    { t: "row", children: [ root.text("1:12", "faint", "caption"), { t: "spacer", fill: true }, root.text("-2:30", "faint", "caption") ] },
    { t: "row", gap: 26, align: "center", children: [
      root.button("󰒮", "prev"), root.button("󰏤", "toggle"), root.button("󰒭", "next", { enabled: false }) ] } ] })
  readonly property var voiceView: ({ t: "column", gap: 12, align: "center", children: [
    { t: "stack", children: [ { t: "meter", value: 0.7, size: 96, child: { t: "icon", glyph: "󰍬", size: 38 } } ] },
    { t: "row", gap: 6, children: [ root.button("󰍭", "mute", { tone: "muted" }), root.button("󰓛", "stop"), root.button("󰜉", "restart"),
      root.button("󰅖", "close") ] } ] })
  readonly property var timerView: ({ t: "column", gap: 10, children: [
    { t: "text", text: "18:42", size: "display", tabular: true },
    { t: "row", gap: 8, children: [
      { t: "chip", label: "5 min", action: "preset", value: "300" },
      { t: "chip", label: "10 min", action: "preset", value: "600", selected: true },
      { t: "chip", label: "25 min", action: "preset", value: "1500" },
      { t: "button", label: "Start", glyph: "󰐊", action: "start", variant: "outline" } ] } ] })
  readonly property var listView: ({ t: "list", gap: 6, stick: "end", maxHeight: 64, children: [
    root.text("you: what is on my calendar", "muted", "bodySmall"), root.text("agent: two meetings today", "foreground", "bodySmall"),
    root.text("you: move the first one", "muted", "bodySmall"), root.text("agent: moved it to 15:00", "foreground", "bodySmall") ] })
  readonly property var badViews: [
    { t: "star", text: "unknown primitive" },
    { t: "meter", value: 1.5 },
    { t: "row", children: "nope" },
    "not even a node" ]

  readonly property var composed: [
    { name: "media (composed)", tree: root.mediaView },
    { name: "voice (composed)", tree: root.voiceView },
    { name: "timer (composed)", tree: root.timerView },
    { name: "list, stick end", tree: root.listView },
    { name: "unknown / invalid", tree: { t: "column", gap: 4, children: root.badViews } },
    { name: "hidden node (visible: false)", tree: { t: "row", gap: 6, children: [ root.text("before"), { t: "text", text: "HIDDEN", visible: false }, root.text("after") ] } } ]

  component Cell: Rectangle {
    id: cell
    property string caption: ""
    property var tree: null
    implicitWidth: Math.max(Theme.space(150), body.implicitWidth + Theme.space(20), label.implicitWidth + Theme.space(20))
    implicitHeight: body.implicitHeight + label.implicitHeight + Theme.space(24)
    radius: Theme.radiusSmall
    color: Theme.surface
    border.width: 1
    border.color: Theme.line
    Text {
      id: label
      x: Theme.space(10)
      y: Theme.space(6)
      textFormat: Text.PlainText
      text: cell.caption
      color: Theme.faint
      font.family: Theme.font.family
      font.pixelSize: Theme.font.caption
    }
    ViewTree {
      id: body
      x: Theme.space(10)
      y: label.y + label.implicitHeight + Theme.space(8)
      tree: cell.tree
    }
  }

  FloatingWindow {
    id: win
    visible: root.step >= 0
    implicitWidth: scene.width
    implicitHeight: scene.height
    color: Theme.background
    Item {
      id: scene
      width: Theme.space(1040)
      height: flow.implicitHeight + Theme.space(32)
      Flow {
        id: flow
        x: Theme.space(16)
        y: Theme.space(16)
        width: parent.width - Theme.space(32)
        spacing: Theme.space(12)
        Repeater {
          model: root.cells.concat(root.composed)
          delegate: Cell {
            required property var modelData
            caption: modelData.name
            tree: modelData.tree
          }
        }
      }
    }
  }

  // One theme after the other: apply, let it settle (the canvas repaints, the
  // progress interpolates), grab, and only then the next theme.
  function next() {
    root.step++
    if (root.step >= root.themes.length) { quitTimer.start(); return }
    Theme.provider = root.themes[root.step].palette
    settle.start()
  }
  Timer { interval: 700; running: true; onTriggered: root.next() }
  Timer {
    id: settle
    interval: 700
    onTriggered: {
      var name = root.themes[root.step].name
      scene.grabToImage(function(result) {
        if (!result.saveToFile(root.outDir + "/" + name + ".png")) console.warn("could not save " + name)
        root.next()
      })
    }
  }
  Timer { id: quitTimer; interval: 300; onTriggered: Qt.quit() }
}
