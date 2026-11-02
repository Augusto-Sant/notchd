// Config rules: reading and editing the JSON config, and resolving it into
// the values the service runs on. Pure functions with no QML globals, so
// Node tests them too (tests/config.test.mjs). Keep it plain ES5 functions:
// QML imports this file as is and Node evaluates it in a vm context.

var DEFAULTS = {
  hoverOpenDelay: 120,
  hoverCloseDelay: 300,
  reserveSpace: false,
  draggable: true,
  tabs: true,
  idleIcons: true
}

// A config file's text as an object. Anything but a JSON object is an error;
// the caller keeps the last good config.
function parse(text) {
  var cfg
  try {
    cfg = JSON.parse(String(text || "{}"))
  } catch (e) {
    return { ok: false, error: String(e && e.message || e) }
  }
  if (!cfg || typeof cfg !== "object" || Array.isArray(cfg))
    return { ok: false, error: "the config must be a JSON object" }
  return { ok: true, config: cfg }
}

// A key's value; dotted keys reach into objects ("theme.accent").
function valueAt(cfg, key) {
  var parts = String(key).split(".")
  var node = cfg
  for (var i = 0; i < parts.length; i++) {
    if (!node || typeof node !== "object") return undefined
    node = node[parts[i]]
  }
  return node
}

// A copy of cfg with one key set. `undefined` removes the key, and objects
// it leaves empty go too, so the file never keeps `"theme": {}` around. A
// numeric part indexes a list ("notches.1.position.edge"); an entry of a
// list is never pruned, since an empty notch `{}` still means a notch, and
// removing one ("notches.1") takes it out of the list.
// The value is copied as plain JSON: a list that went through a QML `var`
// signal or property is a sequence, which Array.isArray (and so resolve)
// would not take for a list.
function withValue(cfg, key, value) {
  var out = JSON.parse(JSON.stringify(cfg || {}))
  if (value !== undefined) value = JSON.parse(JSON.stringify(value))
  var parts = String(key).split(".")
  var chain = [out]
  var node = out
  for (var i = 0; i < parts.length - 1; i++) {
    if (!node[parts[i]] || typeof node[parts[i]] !== "object") {
      if (value === undefined) return out
      node[parts[i]] = {}
    }
    node = node[parts[i]]
    chain.push(node)
  }
  var leaf = parts[parts.length - 1]
  if (value !== undefined) node[leaf] = value
  else if (Array.isArray(node)) { if (/^[0-9]+$/.test(leaf)) node.splice(Number(leaf), 1) }
  else delete node[leaf]
  for (var j = chain.length - 1; j > 0; j--) {
    var parent = chain[j - 1]
    if (Array.isArray(parent) || Array.isArray(chain[j])) break
    if (Object.keys(chain[j]).length === 0) delete parent[parts[j - 1]]
  }
  return out
}

function delay(value, fallback) {
  if (value === undefined) return fallback
  return Math.max(0, Number(value) || 0)
}

// The workspaces the notch hangs on: ids or names, or null for every one
// (also for an empty list).
function workspaceList(value) {
  if (!Array.isArray(value)) return null
  var out = value.filter(function(w) {
    return (typeof w === "number" && isFinite(w)) || (typeof w === "string" && w !== "")
  })
  return out.length > 0 ? out : null
}

// The keys that apply to the whole service, as it uses them. A missing key
// falls back to its default, so deleting it (by hand or in the settings
// window) resets it.
function resolve(cfg) {
  cfg = cfg || {}
  return {
    hoverOpenDelay: delay(cfg.hoverOpenDelay, DEFAULTS.hoverOpenDelay),
    hoverCloseDelay: delay(cfg.hoverCloseDelay, DEFAULTS.hoverCloseDelay)
  }
}

// ---------------------------------------------------------------- notches
// The config's `notches` lists the notches, each an object with any of
// these keys. A key a notch leaves out comes from the same key at the top
// of the config, then its default; `position` merges key by key. Without
// `notches` (or with an empty list) there is one notch, made of the
// top-level keys: a config from before there were several still works.
var NOTCH_KEYS = ["modules", "reserveSpace", "draggable", "tabs", "idleIcons", "workspaces", "position"]

// Whether the config lists its notches (else the top level is the one).
function hasNotchList(cfg) {
  return !!cfg && Array.isArray(cfg.notches) && cfg.notches.length > 0
}

// The id a notch gets without one of its own: "main" first, then notch-2...
function defaultNotchId(index) { return index === 0 ? "main" : "notch-" + (index + 1) }

// Every notch, resolved:
//   id          its own, or defaultNotchId; unique (a repeat gets a suffix)
//   index       its place in the list
//   prefix      what goes before a notch key in the config: "" for the
//               top-level notch, "notches.<index>." otherwise
//   moduleIds, reserveSpace, draggable, workspaces
//               as the service's keys of those names
//   tabs        whether the open notch shows one module at a time, picked
//               from tabs (State.shownModules); on unless false
//   idleIcons   whether an active module with nothing to show closed shows
//               its icon (State.compactModules)
//   position    the raw `position` value, top-level keys merged under the
//               notch's (Position.resolve reads it)
function notches(cfg, defaultModules) {
  cfg = cfg || {}
  var listed = hasNotchList(cfg)
  var entries = listed ? cfg.notches : [{}]
  var used = {}
  var out = []
  for (var i = 0; i < entries.length; i++) {
    var entry = entries[i] && typeof entries[i] === "object" && !Array.isArray(entries[i]) ? entries[i] : {}
    var pick = function(key) { return entry[key] !== undefined ? entry[key] : cfg[key] }
    var id = typeof entry.id === "string" && entry.id.trim() !== "" ? entry.id.trim() : defaultNotchId(i)
    var unique = id
    for (var n = 2; used[unique]; n++) unique = id + "-" + n
    used[unique] = true
    var modules = pick("modules")
    var top = cfg.position && typeof cfg.position === "object" ? cfg.position : null
    var own = entry.position && typeof entry.position === "object" ? entry.position : null
    out.push({
      id: unique,
      index: i,
      prefix: listed ? "notches." + i + "." : "",
      moduleIds: Array.isArray(modules) ? modules.map(String) : (defaultModules || []).slice(),
      reserveSpace: pick("reserveSpace") === true,
      draggable: pick("draggable") !== false,
      tabs: pick("tabs") !== false,
      idleIcons: pick("idleIcons") !== false,
      workspaces: workspaceList(pick("workspaces")),
      position: top || own ? Object.assign({}, top || {}, own || {}) : undefined
    })
  }
  return out
}

// Every module any notch shows, in order of first appearance, then the
// `extra` ones no notch lists.
function moduleUnion(specs, extra) {
  var out = []
  for (var i = 0; i < specs.length; i++)
    for (var j = 0; j < specs[i].moduleIds.length; j++)
      if (out.indexOf(specs[i].moduleIds[j]) === -1) out.push(specs[i].moduleIds[j])
  for (var k = 0; k < (extra || []).length; k++)
    if (out.indexOf(String(extra[k])) === -1) out.push(String(extra[k]))
  return out
}

// The modules to load: every one a notch shows, in order of first
// appearance, except standalone ones, which no notch can show; then the
// standalone ones switched on (`enabled`).
function moduleLoadList(specs, standalone, enabled) {
  standalone = standalone || []
  var out = moduleUnion(specs).filter(function(id) { return standalone.indexOf(id) === -1 })
  for (var i = 0; i < (enabled || []).length; i++)
    if (standalone.indexOf(enabled[i]) !== -1 && out.indexOf(enabled[i]) === -1) out.push(enabled[i])
  return out
}

// The id for a new notch: notch-<n>, the lowest n not taken.
function freeNotchId(ids) {
  for (var n = 2; ; n++)
    if ((ids || []).indexOf("notch-" + n) === -1) return "notch-" + n
}

// A copy of cfg with `entry` added as a notch. A config without a list gets
// one first, its top-level notch as an empty entry that inherits it all.
function withNotchAdded(cfg, entry) {
  var out = JSON.parse(JSON.stringify(cfg || {}))
  if (!hasNotchList(out)) out.notches = [{}]
  out.notches.push(JSON.parse(JSON.stringify(entry || {})))
  return out
}

// A copy of cfg without the notch at `index`. When one notch is left, its
// keys move to the top level and the list goes, so the config reads as it
// did before there were several. The last notch is never removed.
function withoutNotch(cfg, index) {
  var out = JSON.parse(JSON.stringify(cfg || {}))
  if (!hasNotchList(out) || out.notches.length < 2 || index < 0 || index >= out.notches.length) return out
  out.notches.splice(index, 1)
  if (out.notches.length > 1) return out
  var last = out.notches[0] && typeof out.notches[0] === "object" ? out.notches[0] : {}
  delete out.notches
  for (var i = 0; i < NOTCH_KEYS.length; i++) {
    var key = NOTCH_KEYS[i]
    if (last[key] === undefined) continue
    if (key === "position" && out.position && typeof out.position === "object" && typeof last.position === "object")
      out.position = Object.assign(out.position, last.position)
    else out[key] = last[key]
  }
  return out
}

// The `workspaces` list with workspace `ws` ({ id, name }) let in, for a
// notch dropped on a screen showing it. null (every workspace) and a list
// that already has it come back unchanged; otherwise it is added as the
// settings window stores it: a number for a numbered workspace, else its
// name, the list in order (numbers, then names). `list` may be a sequence
// (a list that went through a QML var property).
function withWorkspace(list, ws) {
  if (!list || !ws) return list
  for (var i = 0; i < list.length; i++)
    if (String(list[i]) === String(ws.id) || String(list[i]) === String(ws.name)) return list
  var name = String(ws.name === undefined || ws.name === null ? "" : ws.name)
  var value = /^[0-9]+$/.test(name) ? Number(name) : (name !== "" ? name : ws.id)
  var out = Array.prototype.slice.call(list)
  out.push(value)
  return out.sort(function(a, b) {
    var an = typeof a === "number", bn = typeof b === "number"
    if (an && bn) return a - b
    if (an !== bn) return an ? -1 : 1
    return String(a) < String(b) ? -1 : String(a) > String(b) ? 1 : 0
  })
}

// ---------------------------------------------------------------- terminals
// Terminals of your own: `"terminals": { "<id>": { "command", "title" } }`.
// Each is a module that runs `command` in a terminal in the notch
// (a module of the daemon, native/notchd-modules/src/terminal.rs), shown by a notch that lists its
// id like any module, with its settings under its id ("btop": { "width" }).
// The id is a config key then, so it can't be one the config already has
// (RESERVED_KEYS, and the host's `taken`: its modules and its own keys),
// nor have a dot in it.
var RESERVED_KEYS = ["modules", "notches", "position", "theme", "terminals"]
  .concat(Object.keys(DEFAULTS), NOTCH_KEYS)
var TERMINAL_ID = /^[A-Za-z][A-Za-z0-9_-]*$/

// { list: [{ id, title, command }], errors: ["<id>: why"] }: the entries
// that make a terminal, in the file's order; each one that doesn't, with
// why. `title` is "" when the entry has none.
function terminals(cfg, taken) {
  var out = { list: [], errors: [] }
  var all = cfg && cfg.terminals
  if (all === undefined) return out
  if (!all || typeof all !== "object" || Array.isArray(all)) {
    out.errors.push("terminals: not an object of terminals by id")
    return out
  }
  var reserved = RESERVED_KEYS.concat(Array.prototype.slice.call(taken || []))
  Object.keys(all).forEach(function(id) {
    var entry = all[id]
    if (!TERMINAL_ID.test(id)) { out.errors.push(id + ": an id is a letter, then letters, digits, - or _"); return }
    if (reserved.indexOf(id) !== -1) { out.errors.push(id + ": taken, by a config key or a module"); return }
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) { out.errors.push(id + ": not an object"); return }
    var command = typeof entry.command === "string" ? entry.command.trim() : ""
    if (command === "") { out.errors.push(id + ": needs a command"); return }
    out.list.push({ id: id, title: typeof entry.title === "string" ? entry.title.trim() : "", command: command })
  })
  return out
}

// A copy of cfg with a new terminal: { config, id }. Its id is "terminal",
// else terminal-<n>, the first one no config key, terminal or `taken` id
// has; it runs your shell, for its page to change.
function withTerminalAdded(cfg, taken) {
  var out = JSON.parse(JSON.stringify(cfg || {}))
  var used = RESERVED_KEYS.concat(Array.prototype.slice.call(taken || []), Object.keys(out),
                                  out.terminals && typeof out.terminals === "object" ? Object.keys(out.terminals) : [])
  var n = 1
  while (used.indexOf(n === 1 ? "terminal" : "terminal-" + n) !== -1) n++
  var id = n === 1 ? "terminal" : "terminal-" + n
  if (!out.terminals || typeof out.terminals !== "object" || Array.isArray(out.terminals)) out.terminals = {}
  out.terminals[id] = { title: n === 1 ? "Terminal" : "Terminal " + n, command: "$SHELL" }
  return { config: out, id: id }
}

// A copy of cfg without terminal `id`: its entry, its settings (the key of
// its id) and its place in every notch's `modules`. `terminals` goes once
// empty.
function withoutTerminal(cfg, id) {
  var out = JSON.parse(JSON.stringify(cfg || {}))
  if (!out.terminals || typeof out.terminals !== "object" || Array.isArray(out.terminals) || !(id in out.terminals)) return out
  delete out.terminals[id]
  if (Object.keys(out.terminals).length === 0) delete out.terminals
  delete out[id]
  var drop = function(entry) {
    if (entry && Array.isArray(entry.modules)) entry.modules = entry.modules.filter(function(m) { return m !== id })
  }
  drop(out)
  if (Array.isArray(out.notches)) out.notches.forEach(drop)
  return out
}
