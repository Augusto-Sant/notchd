import QtQuick
import QtQuick.Layouts
import QtQuick.Shapes
import Quickshell
import "core"

// The README image (docs/images/hero.webp): the real NotchCard in four
// states, each driven by its own NotchService with demo data, over a mock
// desktop in Omarchy's Gruvbox theme. scripts/hero.sh runs it offscreen on a
// private D-Bus with demo-player.py, so the media module shows a fictional
// track, with tests/visual/bin's `claude` and `codex` (a finished
// conversation, printed) first on PATH, and passes the wallpaper (a
// gradient stands in without one).
ShellRoot {
  id: root

  readonly property string outDir: Quickshell.env("NOTCHD_VISUAL_DIR")
  readonly property string outFile: Quickshell.env("NOTCHD_HERO_FILE") || (root.outDir + "/hero.png")
  readonly property string wallpaper: Quickshell.env("NOTCHD_HERO_WALLPAPER")
  readonly property color sceneColor: "#1e1e1e"

  // Omarchy's Gruvbox colors, rendered at 2x so the image stays sharp on
  // high-density screens.
  readonly property var palette: ({
    foreground: "#d4be98", background: "#282828", accent: "#7daea3", danger: "#ea6962",
    fontFamily: "JetBrainsMono Nerd Font", spacingScale: 2,
    fontCaption: 20, fontBodySmall: 22, fontBody: 24, fontHeading: 32, fontDisplay: 64
  })
  Component.onCompleted: Theme.provider = root.palette

  component Service: TestService {
    property string tag: ""
    ipcTarget: "notchd-hero-" + tag
    configPath: root.outDir + "/hero-" + tag + ".json"
  }
  Service { id: compactSvc; tag: "compact"; defaultModules: ["claude", "codex"] }
  Service { id: activitySvc; tag: "activity"; defaultModules: ["activities"] }
  Service { id: usageSvc; tag: "usage"; defaultModules: ["claude"] }
  Service { id: expandedSvc; tag: "expanded"; defaultModules: ["claude", "codex", "media"] }

  // The plan's limits, by hand: offscreen nothing asks the network.
  function limits(week) {
    var later = function(mins) { return new Date(Date.now() + mins * 60000).toISOString() }
    return { limits: [
      { kind: "session", group: "session", percent: 42, resets_at: later(130) },
      { kind: "weekly_all", group: "weekly", percent: week, resets_at: later(3 * 1440 + 200) },
      { kind: "weekly_scoped", group: "weekly", percent: 5, resets_at: later(3 * 1440 + 200),
        scope: { model: { display_name: "Fable" } } } ] }
  }

  // A slice of a desktop: wallpaper, a bar, and the notch hanging from it.
  component Screen: Rectangle {
    id: screen
    property var svc: null
    readonly property var copy: screen.svc ? screen.svc.firstCopy : null
    readonly property int barHeight: Theme.space(26)

    readonly property real cornerRadius: Theme.space(14)

    clip: true
    gradient: Gradient {
      GradientStop { position: 0.0; color: "#3c3836" }
      GradientStop { position: 1.0; color: "#504945" }
    }
    // The wallpaper at the tile's width, pinned to the top, so each tile
    // reads as the top of a screen.
    Image {
      id: wallpaper
      visible: root.wallpaper !== ""
      anchors.top: parent.top
      anchors.horizontalCenter: parent.horizontalCenter
      width: screen.width
      height: implicitWidth > 0 ? Math.max(screen.height, screen.width * implicitHeight / implicitWidth) : screen.height
      source: root.wallpaper !== "" ? "file://" + root.wallpaper : ""
      sourceSize.width: 2400
      fillMode: Image.PreserveAspectCrop
      verticalAlignment: Image.AlignTop
    }

    Rectangle {
      id: bar
      width: parent.width
      height: screen.barHeight
      color: Theme.background
      Row {
        anchors.left: parent.left
        anchors.leftMargin: Theme.space(12)
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.space(9)
        Repeater {
          model: ["1", "2", "3", "4"]
          Text {
            required property string modelData
            required property int index
            text: modelData
            color: index === 0 ? Theme.foreground : Theme.faint
            font.family: Theme.font.family
            font.pixelSize: Theme.font.caption
          }
        }
      }
      Text {
        anchors.centerIn: parent
        text: "Saturday 11:49"
        color: Theme.foreground
        font.family: Theme.font.family
        font.pixelSize: Theme.font.caption
      }
      Text {
        anchors.right: parent.right
        anchors.rightMargin: Theme.space(12)
        anchors.verticalCenter: parent.verticalCenter
        text: "󰖩  󰕾  󰁹"
        color: Theme.muted
        font.family: Theme.font.family
        font.pixelSize: Theme.font.caption
      }
    }

    NotchCard {
      notch: screen.copy
      maxWidth: screen.width - Theme.space(24)
      x: Math.round((screen.width - width) / 2)
      y: screen.barHeight
    }

    // Rounded corners. The software renderer can't round-clip an image, so
    // each corner is painted over in the scene color, then outlined.
    Repeater {
      model: 4
      Shape {
        required property int index
        readonly property real r: screen.cornerRadius
        width: r; height: r
        x: index % 2 === 0 ? 0 : screen.width - r
        y: index < 2 ? 0 : screen.height - r
        rotation: [0, 90, 270, 180][index]
        ShapePath {
          fillColor: root.sceneColor
          strokeColor: "transparent"
          startX: 0; startY: 0
          PathLine { x: screen.cornerRadius; y: 0 }
          PathArc { x: 0; y: screen.cornerRadius; radiusX: screen.cornerRadius; radiusY: screen.cornerRadius; direction: PathArc.Counterclockwise }
          PathLine { x: 0; y: 0 }
        }
      }
    }
    Rectangle {
      anchors.fill: parent
      radius: screen.cornerRadius
      color: "transparent"
      border.width: 1
      border.color: Theme.line
    }
  }

  component Caption: ColumnLayout {
    property string title: ""
    property string detail: ""
    spacing: Theme.space(2)
    Text {
      text: parent.title
      color: Theme.foreground
      font.family: Theme.font.family
      font.pixelSize: Theme.font.body
      font.bold: true
    }
    Text {
      Layout.fillWidth: true
      text: parent.detail
      wrapMode: Text.Wrap
      color: Theme.muted
      font.family: Theme.font.family
      font.pixelSize: Theme.font.bodySmall
    }
  }

  FloatingWindow {
    id: win
    implicitWidth: scene.width
    implicitHeight: scene.height
    color: Theme.background

    Rectangle {
      id: scene
      readonly property int pad: Theme.space(40)
      readonly property int gap: Theme.space(20)
      width: Theme.space(1000)
      height: column.implicitHeight + pad * 2
      color: root.sceneColor

      ColumnLayout {
        id: column
        x: scene.pad
        y: scene.pad
        width: scene.width - scene.pad * 2
        spacing: scene.gap

        ColumnLayout {
          Layout.bottomMargin: Theme.space(8)
          spacing: Theme.space(6)
          Text {
            text: "notchd"
            color: Theme.foreground
            font.family: Theme.font.family
            font.pixelSize: Theme.font.display
            font.bold: true
          }
          Text {
            Layout.fillWidth: true
            text: "A dynamic notch for Wayland, built with Quickshell. It hangs from the top edge and shows what your system is doing: Claude Code and Codex at work, media, script activities, or any module you write."
            wrapMode: Text.Wrap
            color: Theme.muted
            font.family: Theme.font.family
            font.pixelSize: Theme.font.body
          }
        }

        RowLayout {
          Layout.fillWidth: true
          spacing: scene.gap
          Repeater {
            model: [
              { svc: compactSvc, title: "Compact", detail: "Claude works; Codex finished while you looked away." },
              { svc: usageSvc, title: "Plan usage", detail: "A limit nears: it warns once, with when it resets." },
              { svc: activitySvc, title: "Peek", detail: "A script pushes an activity, a training run, a build: it pops open for a moment." }
            ]
            ColumnLayout {
              required property var modelData
              Layout.fillWidth: true
              Layout.preferredWidth: 1
              spacing: Theme.space(10)
              Screen {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.space(150)
                svc: parent.modelData.svc
              }
              Caption { Layout.fillWidth: true; title: parent.modelData.title; detail: parent.modelData.detail }
            }
          }
        }

        ColumnLayout {
          Layout.fillWidth: true
          spacing: Theme.space(10)
          Screen {
            Layout.fillWidth: true
            Layout.preferredHeight: Theme.space(390)
            svc: expandedSvc
          }
          Caption {
            Layout.fillWidth: true
            title: "Expanded"
            detail: "Hover or click: the agent in its terminal, the plan's limits above it, the other modules a tab away."
          }
        }
      }
    }
  }

  // Let modules load and the media track arrive (and its peek pass), set
  // each state, let the springs settle, grab.
  Timer {
    interval: 1500; running: true
    onTriggered: {
      activitySvc.moduleById("activities").push(JSON.stringify({
        id: "train", icon: "󰚩", title: "Fine-tuning a LoRA", subtitle: "epoch 2 of 3 · loss 1.42",
        progress: 64, duration: 600000 }))
      // Agents in notchd's folder here, not the real one.
      var services = [compactSvc, usageSvc, expandedSvc]
      for (var i = 0; i < services.length; i++) {
        services[i].setConfigValue("claude.directory", root.outDir)
        services[i].setConfigValue("codex.directory", root.outDir)
      }
      compactSvc.moduleById("claude").status = { working: true, title: "Port to Rust" }
      var codex = compactSvc.moduleById("codex")
      codex.status = { working: false, title: "Fix flaky CI" }
      codex.attention = true
      // The week passes the warning between two answers: the peek.
      var claude = usageSvc.moduleById("claude")
      claude.usage.apply(root.limits(76))
      claude.usage.apply(root.limits(83))
      claude.requestPeek(600000)
      expandedSvc.setConfigValue("claude.width", 700)
      expandedSvc.setConfigValue("claude.height", 215)
      expandedSvc.moduleById("claude").usage.apply(root.limits(83))
      expandedSvc.open()
      expandedSvc.firstCopy.showTab("claude")
      grabTimer.start()
    }
  }
  Timer {
    id: grabTimer
    interval: 4500
    onTriggered: {
      compactSvc.dismissPeek()
      settleTimer.start()
    }
  }
  Timer {
    id: settleTimer
    interval: 1200
    onTriggered: scene.grabToImage(function(result) {
      if (!result.saveToFile(root.outFile)) console.warn("could not save " + root.outFile)
      Qt.quit()
    })
  }
}
