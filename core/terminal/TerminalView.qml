import QtQuick
import QMLTermWidget
import ".."
import "../Terminal.js" as Terminal

// A terminal running one program, from the optional qmltermwidget package
// (the QML port of qtermwidget). Nothing loads this file before
// Terminal.PROBE has loaded, so a missing package is a message in the
// surface, not a warning. qmllint can't read the package's types, so lint
// skips this file (Makefile); the QML tests load it. TerminalSurface runs
// `notchd attach` in it: the program itself lives in the daemon and this is
// only a window onto it. The program starts with the view, and the view
// ends with it.
QMLTermWidget {
  id: view
  objectName: "terminal"

  property string program: "/bin/sh"
  property var programArguments: []
  property string workingDirectory: ""
  property string fontFamily: ""

  // The title the program sets (OSC 0/2).
  readonly property string title: termSession.title
  // The program is gone; the view stays with what it printed.
  property bool exited: false
  // A key went to the program.
  signal typed()
  // Ctrl+Escape: the way out. The program takes every other key, Escape
  // included, which agents use; this one never reaches it.
  signal leave()

  // A whole font: the widget's own has a point size, and a pixel size set
  // on top of it warns.
  font: Qt.font({ family: view.fontFamily || Theme.font.family, pixelSize: Theme.font.body })
  // The scheme gives the ANSI colors (Terminal.SCHEME); the theme's
  // background and foreground go over it.
  colorScheme: Terminal.SCHEME
  onColorSchemeChanged: view.applyTheme()
  function applyTheme() {
    view.setBackgroundColor(Theme.background)
    view.setForegroundColor(Theme.foreground)
  }
  Connections {
    target: Theme
    function onBackgroundChanged() { view.applyTheme() }
    function onForegroundChanged() { view.applyTheme() }
  }

  // Text to the program, as if typed (a test, a command over IPC).
  function write(text) { termSession.sendText(String(text)) }

  Keys.priority: Keys.BeforeItem
  Keys.onPressed: function(event) {
    if (Terminal.isLeaveKey(event.key, event.modifiers)) {
      event.accepted = true
      view.leave()
      return
    }
    view.typed()
    event.accepted = false
  }

  session: QMLTermSession {
    id: termSession
    initialWorkingDirectory: view.workingDirectory
    shellProgram: view.program
    shellProgramArgs: view.programArguments
    onFinished: view.exited = true
  }

  Component.onCompleted: {
    view.applyTheme()
    termSession.startShellProgram()
  }
}
