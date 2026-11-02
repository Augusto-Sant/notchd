import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import ".."
import "Settings.js" as Settings
import "../Components.js" as Components

// The settings window: a sidebar of pages and, for the current page, its
// fields grouped by section. Built from NotchService.settingsPages and
// settingsSchema; every change applies at once and is written back to the
// config file (NotchService.setConfigValue), so there is no Apply button.
//
// An ordinary toplevel window, so it works on any compositor and takes
// keyboard input, which the notch overlay never does.
FloatingWindow {
  id: win

  property var svc: null
  // The page's scroller (the visual tests scroll it to reach lower fields).
  readonly property alias pageView: flick

  title: "notchd settings"
  color: Theme.background

  implicitWidth: Theme.space(780)
  implicitHeight: Theme.space(540)
  // A fixed size, so tiling compositors float it like a dialog.
  minimumSize: Qt.size(win.implicitWidth, win.implicitHeight)
  maximumSize: Qt.size(win.implicitWidth, win.implicitHeight)

  onClosed: if (win.svc) win.svc.closeSettings()

  // ----------------------------------------------------------------- pages
  // Settings.js has the rules.
  // What the page shows: the service's schema without the fields that wait
  // on another field's value (`when`), and the choices that wait on the
  // component of the chosen language locked (Components.gate).
  readonly property var schema: {
    var svc = win.svc
    if (!svc) return []
    var base = svc.settingsSchema
    var valueOf = svc.configValue
    return Components.gate(Settings.shownFields(base, valueOf), svc.components.catalog,
                           svc.components.loaded ? svc.components.installed : null,
                           function(key) { return Settings.effectiveValue(base, key, valueOf) })
  }
  readonly property var pages: Settings.visiblePages(win.svc ? win.svc.settingsPages : [], win.schema)
  readonly property var navPages: Settings.navPages(win.pages)
  readonly property string currentPage: Settings.resolvePage(win.pages, win.svc ? win.svc.settingsPage : "")
  // The sidebar entry to mark: the page, or the one a sub-page sits under.
  readonly property string currentNav: Settings.parentPage(win.currentPage)
  readonly property var currentSub: win.currentNav !== win.currentPage ? Settings.findPage(win.pages, win.currentPage) : null
  readonly property var sections: Settings.sections(win.schema, win.currentPage)

  function showPage(id) { if (win.svc) win.svc.settingsPage = id }
  function cyclePage(delta) { win.showPage(Settings.cyclePage(win.pages, win.currentPage, delta)) }
  // Escape leaves a sub-page for its parent, else closes the window.
  function back() {
    if (win.currentSub) win.showPage(win.currentNav)
    else if (win.svc) win.svc.closeSettings()
  }

  // Opens the config file, or its folder while the file doesn't exist yet.
  Process {
    id: openConfig
    command: ["sh", "-c", "[ -e \"$1\" ] && exec xdg-open \"$1\"; d=$(dirname \"$1\"); mkdir -p \"$d\" && exec xdg-open \"$d\"",
              "sh", win.svc ? win.svc.configPath : ""]
  }

  // ---------------------------------------------------------------- layout
  // Painted here as well as on the window, so grabs (tests/visual) match.
  Rectangle {
    anchors.fill: parent
    color: Theme.background
    focus: true
    Keys.onEscapePressed: win.back()
    Keys.onPressed: function(event) {
      if ((event.modifiers & Qt.ControlModifier) && (event.key === Qt.Key_Tab || event.key === Qt.Key_Backtab)) {
        win.cyclePage(event.key === Qt.Key_Backtab || (event.modifiers & Qt.ShiftModifier) ? -1 : 1)
        event.accepted = true
      }
    }

    RowLayout {
      anchors.fill: parent
      spacing: 0

      // ------------------------------------------------------------ sidebar
      Rectangle {
        Layout.fillHeight: true
        Layout.preferredWidth: Theme.space(210)
        color: Theme.surface

        ColumnLayout {
          anchors.fill: parent
          anchors.margins: Theme.space(14)
          spacing: Theme.space(4)

          Label {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.space(10)
            Layout.topMargin: Theme.space(8)
            Layout.bottomMargin: Theme.space(14)
            role: "title"
            text: "Settings"
          }

          Repeater {
            model: win.navPages
            delegate: NavItem {
              required property var modelData
              Layout.fillWidth: true
              icon: modelData.icon
              label: modelData.label
              current: modelData.id === win.currentNav
              onActivated: win.showPage(modelData.id)
            }
          }

          Item { Layout.fillHeight: true }

          Button {
            Layout.fillWidth: true
            icon: 0xF0214  // nf-md-file
            text: "Open config file"
            onClicked: openConfig.running = true
          }
          Label {
            Layout.fillWidth: true
            Layout.topMargin: Theme.space(4)
            Layout.leftMargin: Theme.space(2)
            role: "help"
            color: Theme.faint
            font.pixelSize: Theme.font.caption
            wrapMode: Text.WrapAnywhere
            text: win.svc ? win.svc.configPath.replace(/^\/home\/[^\/]+/, "~") : ""
          }
        }
      }

      Rectangle {
        Layout.fillHeight: true
        Layout.preferredWidth: 1
        color: Theme.line
      }

      // ------------------------------------------------------------ content
      Flickable {
        id: flick
        Layout.fillWidth: true
        Layout.fillHeight: true
        contentHeight: content.implicitHeight + Theme.space(56)
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Connections {
          target: win
          function onCurrentPageChanged() { flick.contentY = 0 }
        }

        ColumnLayout {
          id: content
          x: Theme.space(32)
          y: Theme.space(28)
          width: flick.width - Theme.space(64)
          spacing: Theme.space(30)

          // A sub-page's way back: "‹ Modules / Now playing".
          RowLayout {
            visible: !!win.currentSub
            Layout.fillWidth: true
            Layout.bottomMargin: -Theme.space(10)
            spacing: Theme.space(6)

            Rectangle {
              id: backLink
              Layout.preferredHeight: Theme.space(28)
              Layout.preferredWidth: backRow.implicitWidth + Theme.space(14)
              Layout.leftMargin: -Theme.space(8)
              radius: Theme.radiusSmall
              color: backArea.containsMouse ? Theme.hover : "transparent"
              RowLayout {
                id: backRow
                anchors.centerIn: parent
                spacing: Theme.space(4)
                Icon {
                  glyph: 0xF0141  // nf-md-chevron_left
                  size: Theme.font.heading
                  color: backArea.containsMouse ? Theme.foreground : Theme.muted
                }
                Label {
                  wrapMode: Text.NoWrap
                  color: backArea.containsMouse ? Theme.foreground : Theme.muted
                  text: {
                    var parent = Settings.findPage(win.pages, win.currentNav)
                    return parent ? parent.label : win.currentNav
                  }
                }
              }
              MouseArea {
                id: backArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: win.showPage(win.currentNav)
              }
            }
            Label {
              wrapMode: Text.NoWrap
              color: Theme.faint
              text: "/"
            }
            Label {
              Layout.fillWidth: true
              role: "title"
              wrapMode: Text.NoWrap
              elide: Text.ElideRight
              text: win.currentSub ? win.currentSub.label : ""
            }
          }

          Repeater {
            model: win.sections
            delegate: ColumnLayout {
              id: section
              required property var modelData
              Layout.fillWidth: true
              spacing: Theme.space(16)

              Label {
                visible: section.modelData.name !== ""
                Layout.fillWidth: true
                role: "section"
                text: section.modelData.name
              }

              Repeater {
                model: section.modelData.fields
                delegate: ColumnLayout {
                  id: entry
                  required property var modelData
                  required property int index
                  Layout.fillWidth: true
                  spacing: Theme.space(16)

                  Rectangle {
                    visible: entry.index > 0
                    Layout.fillWidth: true
                    Layout.preferredHeight: 1
                    color: Theme.line
                  }
                  SettingRow {
                    Layout.fillWidth: true
                    field: entry.modelData
                    svc: win.svc
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
