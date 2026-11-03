// What the terminal view (core/terminal/GhosttySurface.qml) needs on the UI
// side: the key that leaves it and Qt's keys as the daemon names them. The
// daemon runs the program and emulates the terminal (native/notchd-daemon/
// src/grid.rs); the view draws what it sends. Plain functions: QML imports
// this file as is and the Node tests load it in a vm.

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

// The terminal view sends keys, not bytes: the daemon encodes them (legacy or the kitty keyboard
// protocol, as the program asked). A key is named as W3C's
// KeyboardEvent.code names it. `gridKey` turns Qt's key code, modifiers and
// text into the `key` event's { key, mods, text }: `key` "" for a key with
// no name here (a shifted symbol: its text goes as it is), `mods` 1 shift,
// 2 alt, 4 ctrl, 8 super, `text` what it types (none with Ctrl, and none for
// a control character). null for a modifier pressed alone.
var GRID_KEYS = {
  0x01000000: "Escape", 0x01000001: "Tab", 0x01000002: "Tab", 0x01000003: "Backspace",
  0x01000004: "Enter", 0x01000005: "NumpadEnter", 0x01000006: "Insert", 0x01000007: "Delete",
  0x01000010: "Home", 0x01000011: "End", 0x01000012: "ArrowLeft", 0x01000013: "ArrowUp",
  0x01000014: "ArrowRight", 0x01000015: "ArrowDown", 0x01000016: "PageUp", 0x01000017: "PageDown",
  0x20: "Space", 0x27: "Quote", 0x2c: "Comma", 0x2d: "Minus", 0x2e: "Period", 0x2f: "Slash",
  0x3b: "Semicolon", 0x3d: "Equal", 0x5b: "BracketLeft", 0x5c: "Backslash", 0x5d: "BracketRight",
  0x60: "Backquote"
}
// Qt's modifier flags as the grid's `mods`: 1 shift, 2 alt, 4 ctrl, 8 super.
function gridMods(modifiers) {
  modifiers = Number(modifiers) || 0
  var mods = 0
  if (modifiers & 0x02000000) mods |= 1
  if (modifiers & 0x08000000) mods |= 2
  if (modifiers & MOD_CONTROL) mods |= 4
  if (modifiers & 0x10000000) mods |= 8
  return mods
}

// A Qt mouse button (Qt.LeftButton 1, Qt.RightButton 2, Qt.MiddleButton 4)
// as the grid's `mouse` event names it; "" for another.
function gridButton(button) {
  return button === 1 ? "left" : button === 2 ? "right" : button === 4 ? "middle" : ""
}

var GRID_MODIFIER_KEYS = [0x01000020, 0x01000021, 0x01000022, 0x01000023, 0x01000024, 0x01000025, 0x01001103]
function gridKey(code, modifiers, text) {
  if (GRID_MODIFIER_KEYS.indexOf(code) >= 0) return null
  modifiers = Number(modifiers) || 0
  var key = GRID_KEYS[code] || ""
  if (key === "" && code >= 0x41 && code <= 0x5a) key = "Key" + String.fromCharCode(code)
  if (key === "" && code >= 0x30 && code <= 0x39) key = "Digit" + String.fromCharCode(code)
  if (key === "" && code >= 0x01000030 && code <= 0x01000047) key = "F" + (code - 0x01000030 + 1)
  var mods = gridMods(modifiers)
  // Backtab is Shift+Tab, whatever Qt reports of the shift.
  if (code === 0x01000002) mods |= 1
  var out = { key: key, mods: mods }
  var t = String(text || "")
  if (t !== "" && !(mods & 4) && t.charCodeAt(0) >= 0x20 && t.charCodeAt(0) !== 0x7f) out.text = t
  if (out.key === "" && out.text === undefined) return null
  return out
}
