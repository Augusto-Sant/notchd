// The pure half of the daemon client (DaemonClient.qml): line framing,
// message building, and applying an update by `rev`. Plain ES5 functions, so
// QML imports this file as is and Node tests it in a vm
// (tests/client-protocol.test.mjs). The protocol is docs/architecture.md,
// "Protocol: core to UI".

var PROTO = 1
var MAX_LINE = 4 * 1024 * 1024

// ------------------------------------------------------------- framing
// Splits what has been read so far: `buffer` is the unfinished tail kept from
// the last call, `chunk` the new text. Returns { lines, rest, overflow }:
// the complete non-empty lines, the new tail, and whether the tail grew past
// the line limit (the daemon closes on such a line; the client drops it).
function frame(buffer, chunk) {
  var text = (buffer || "") + (chunk || "")
  var parts = text.split("\n")
  var rest = parts.pop()
  var lines = []
  for (var i = 0; i < parts.length; i++) {
    var line = parts[i]
    if (line.length > 0 && line.charAt(line.length - 1) === "\r") line = line.slice(0, -1)
    if (line.length > 0) lines.push(line)
  }
  var overflow = rest.length > MAX_LINE
  return { lines: lines, rest: overflow ? "" : rest, overflow: overflow }
}

// One line to a message object, or null when it is not a JSON object with a
// string `t` (a client ignores such a line and logs once).
function parse(line) {
  var msg
  try { msg = JSON.parse(line) } catch (e) { return null }
  if (msg === null || typeof msg !== "object" || Array.isArray(msg)) return null
  if (typeof msg.t !== "string") return null
  return msg
}

// A message as one wire line.
function encode(msg) { return JSON.stringify(msg) + "\n" }

// ----------------------------------------------------------- building
function hello(client, clientVersion, instance, caps) {
  return { t: "hello", proto: PROTO, client: client, clientVersion: clientVersion, instance: instance, caps: caps }
}

function pointer(notch, screen, ev) { return { t: "pointer", notch: notch, screen: screen, ev: ev } }

function key(notch, screen, name, mods, text) {
  return { t: "key", notch: notch, screen: screen, key: name, mods: mods || [], text: text || "" }
}

// A copy of a notch is named by its notch's id (a standalone module's by its
// module id, the visitor's by "") and its screen ("" for the one that moves).
// Input facts, no reply (docs/architecture.md, "UI to core").
function pin(notch, screen, on) { return { t: "pin", notch: notch, screen: screen, on: !!on } }
function focus(notch, screen, active) { return { t: "focus", notch: notch, screen: screen, active: !!active } }
function tab(notch, screen, module) { return { t: "tab", notch: notch, screen: screen, module: module } }

// `ev` is "start", "drop" (with `position`: { edge, offset, screen }) or "cancel".
function drag(notch, screen, ev, position) {
  var msg = { t: "drag", notch: notch, screen: screen, ev: ev }
  if (position) msg.position = { edge: position.edge, offset: position.offset, screen: position.screen }
  return msg
}

// The depths one copy measured (docs/architecture.md, `measure`).
function measure(notch, screen, rest, compact, baseline) {
  return { t: "measure", notch: notch, screen: screen, rest: rest, compact: compact, baseline: baseline }
}

// What the modules the UI loads say of themselves, `modules` a list of
// { id, priority, active, compact, peek, icon, expanded, attention, urgentPeek,
// peekOnFocusedScreen, standalone }.
function facts(modules) { return { t: "facts", modules: modules } }

// A module asks to peek (`ms` optional) or drops its peek (`module` optional: every peek).
function peek(module, ms) {
  var msg = { t: "peek", module: module }
  if (ms !== undefined && ms !== null) msg.ms = Number(ms)
  return msg
}
function peekDismiss(module) {
  return module ? { t: "peek-dismiss", module: module } : { t: "peek-dismiss" }
}

// A module acts on the notches that show it: `op` is "open", "close" or "keyboard".
function moduleNotch(module, op) { return { t: "module-notch", module: module, op: op } }

// A key the way the daemon reads it (`key`'s `key` and `mods`) out of a Qt
// key code and modifier flags (Qt.Key_*, Qt.*Modifier), so the notch's
// navigation window needs no table of its own: the name ("Escape", "Tab",
// "Backtab", "Return", "Enter", "Left", "Up", "Right", "Down", or one
// printable ASCII character, lower case) and the modifiers held (["shift",
// "ctrl", "alt", "meta"]). `key` is "" for a key the daemon has no name for.
var QT_KEYS = {
  0x01000000: "Escape", 0x01000001: "Tab", 0x01000002: "Backtab", 0x01000004: "Return", 0x01000005: "Enter",
  0x01000012: "Left", 0x01000013: "Up", 0x01000014: "Right", 0x01000015: "Down"
}
function wireKey(code, modifiers) {
  modifiers = Number(modifiers) || 0
  var key = QT_KEYS[code] || ""
  if (key === "" && code >= 0x20 && code <= 0x7e) key = String.fromCharCode(code).toLowerCase()
  var mods = []
  if (modifiers & 0x02000000) mods.push("shift")
  if (modifiers & 0x04000000) mods.push("ctrl")
  if (modifiers & 0x08000000) mods.push("alt")
  if (modifiers & 0x10000000) mods.push("meta")
  return { key: key, mods: mods }
}

// `value` is always present: a missing one goes as null.
function event(module, slot, action, value) {
  return { t: "event", module: module, slot: slot, action: action, value: value === undefined ? null : value }
}

// `undefined` is a removal, which is null on the wire.
function set(id, keyPath, value) {
  return { t: "set", id: id, key: keyPath, value: value === undefined ? null : value }
}

// `fields` (moveModule: { notch, module, delta }, ...) merge in after `op`.
function edit(id, op, fields) {
  var msg = { t: "edit", id: id, op: op }
  for (var k in (fields || {})) msg[k] = fields[k]
  return msg
}

// Arguments are strings on the wire.
function ipc(id, method, args) {
  var list = []
  for (var i = 0; i < (args || []).length; i++) list.push(String(args[i]))
  return { t: "ipc", id: id, method: method, args: list }
}

// ------------------------------------------------------------ entities
// What the client holds: the latest of each entity and the rev it came with.
// `views` is keyed "module/slot", `modules` by id; `revs` by entity key.
function emptyStore() {
  return { config: null, schema: null, state: null, views: {}, modules: {}, revs: {} }
}

function viewKey(module, slot) { return module + "/" + slot }

// The store key a message updates, or "" when it is not an entity.
function entityKey(msg) {
  switch (msg.t) {
  case "config": case "schema": case "state": return msg.t
  case "view": return "view:" + viewKey(msg.module, msg.slot)
  case "module": return "module:" + msg.id
  }
  return ""
}

function copyMap(map, key, value) {
  var out = {}
  for (var k in map) out[k] = map[k]
  out[key] = value
  return out
}

// Applies one entity message. Returns { store, applied, key }: a new store
// (the old one is not touched, so QML sees property changes), whether the
// message was applied, and the entity key. A message that is not an entity, or
// whose `rev` is not newer than the one held, is not applied and the store
// comes back as it was.
function apply(store, msg) {
  var key = entityKey(msg)
  if (key === "" || typeof msg.rev !== "number") return { store: store, applied: false, key: key }
  var held = store.revs[key]
  if (held !== undefined && msg.rev <= held) return { store: store, applied: false, key: key }
  var next = {
    config: store.config, schema: store.schema, state: store.state,
    views: store.views, modules: store.modules, revs: copyMap(store.revs, key, msg.rev)
  }
  switch (msg.t) {
  case "config": case "schema": case "state": next[msg.t] = msg; break
  case "view": next.views = copyMap(store.views, viewKey(msg.module, msg.slot), msg); break
  case "module": next.modules = copyMap(store.modules, msg.id, msg); break
  }
  return { store: next, applied: true, key: key }
}

// --------------------------------------------------------------- replies
function isReply(msg) { return (msg.t === "ok" || msg.t === "err") && typeof msg.id === "number" }

// A refused handshake: the one `err` without an id.
function isRefusal(msg) { return msg.t === "err" && msg.id === undefined && msg.code === "proto" }

// The `err` a pending request gets when the connection drops first.
function droppedReply(id) { return { t: "err", id: id, code: "disconnected", message: "the daemon connection closed" } }

// Milliseconds to wait before reconnect attempt `attempt` (0 is the first):
// doubling from `min`, capped at `max`.
function backoff(attempt, min, max) {
  var delay = min
  for (var i = 0; i < attempt && delay < max; i++) delay *= 2
  return Math.min(delay, max)
}
