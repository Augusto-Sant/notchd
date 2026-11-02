// What the Linux UI's terminal view needs of qmltermwidget (core/terminal/
// TerminalView.qml). The program itself runs in the daemon (the rules for
// its command line and title are native/notchd-core/src/{terminal,agent}.rs);
// this is the widget's side only. Plain functions: QML imports this file as
// is and the Node tests load it in a vm.

// The view imports QMLTermWidget, from the optional qmltermwidget package
// (the QML port of qtermwidget). Without it, loading the view fails with a
// warning; this loads only that import, and Qt.createQmlObject throws on it
// quietly. The service tries it first (`terminalAvailable`) and tells the
// daemon what it found.
var PROBE = "import QtQuick\nimport QMLTermWidget\nItem {}\n"

// The key that leaves the terminal (the notch closes, the program goes on):
// Ctrl+Escape, which neither Claude Code nor Codex binds; plain Escape is
// theirs (it interrupts them). `key` and `modifiers` as Qt gives them
// (Qt.Key_Escape is 0x01000000, Qt.ControlModifier 0x04000000); Shift,
// Alt or Super with it make another key. The keypad flag doesn't count.
var KEY_ESCAPE = 0x01000000
var MOD_CONTROL = 0x04000000
var MOD_KEYPAD = 0x20000000
function isLeaveKey(key, modifiers) {
  return key === KEY_ESCAPE && ((modifiers || 0) & ~MOD_KEYPAD) === MOD_CONTROL
}

// The built-in color scheme behind the ANSI colors; the view sets the
// theme's background and foreground over it. The widget only reads schemes
// from its own package's folder, so the theme can't bring a palette of its
// own. Tango, on any theme: its bright black (ANSI 8) is a dark gray, and
// Codex, which can't ask this terminal for its background (the widget
// doesn't answer OSC 11), paints your messages with it. Breeze's is orange.
var SCHEME = "Tango"
