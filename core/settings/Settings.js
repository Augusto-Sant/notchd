// Settings-window rules: pages, sections and list edits. Pure functions with
// no QML globals, so Node tests them too (tests/settings.test.mjs).

// The core pages with the host's applied: a host page with a core id
// replaces its label or icon, any other is appended.
function mergePages(core, host) {
  var out = core.map(function(p) { return Object.assign({}, p) })
  host = host || []
  for (var i = 0; i < host.length; i++) {
    var at = -1
    for (var j = 0; j < out.length; j++) if (out[j].id === host[i].id) at = j
    if (at === -1) out.push(Object.assign({}, host[i]))
    else out[at] = Object.assign(out[at], host[i])
  }
  return out
}

function pageOf(field) { return field.page || "general" }

// A page whose id has a "/" is a sub-page of the part before it
// ("modules/media" of "modules"): it has no sidebar entry of its own, and
// the sidebar marks its parent while it shows.
function parentPage(id) {
  var s = String(id || "")
  var at = s.indexOf("/")
  return at === -1 ? s : s.slice(0, at)
}

function isSubPage(page) { return parentPage(page.id) !== page.id }

// Pages with at least one field, in their declared order.
function visiblePages(pages, schema) {
  return pages.filter(function(p) {
    return schema.some(function(f) { return pageOf(f) === p.id })
  })
}

// The pages the sidebar lists: the visible ones that are not sub-pages.
function navPages(pages) {
  return pages.filter(function(p) { return !isSubPage(p) })
}

function findPage(pages, id) {
  for (var i = 0; i < pages.length; i++) if (pages[i].id === id) return pages[i]
  return null
}

// The page to show: the wanted one if it is visible, else its parent (a
// module's page once the module unloads), else the first.
function resolvePage(pages, wanted) {
  if (findPage(pages, wanted)) return wanted
  var parent = parentPage(wanted)
  if (findPage(pages, parent)) return parent
  var nav = navPages(pages)
  return nav.length ? nav[0].id : (pages.length ? pages[0].id : "")
}

// The sidebar page `delta` steps from the one `current` sits under,
// wrapping around.
function cyclePage(pages, current, delta) {
  var nav = navPages(pages)
  if (!nav.length) return ""
  var here = parentPage(current)
  var at = -1
  for (var i = 0; i < nav.length; i++) if (nav[i].id === here) at = i
  var n = nav.length
  return nav[((at + delta) % n + n) % n].id
}

// A field's value as the settings show it: the config's, else the default of
// the field with that key (`valueOf(key)` is the config's, undefined unset).
function effectiveValue(schema, key, valueOf) {
  var v = valueOf(key)
  if (v !== undefined) return v
  for (var i = 0; i < schema.length; i++) if (schema[i].key === key) return schema[i]["default"]
  return undefined
}

// A field with `when: { key, is }` shows only while the field `key` has the
// value `is` (or one of them, when it is a list): the voices of the chosen
// language, a setting that only means something with another on. The schema
// without the fields that don't show.
function shownFields(schema, valueOf) {
  return schema.filter(function(f) {
    if (!f.when) return true
    var v = effectiveValue(schema, f.when.key, valueOf)
    return Array.isArray(f.when.is) ? f.when.is.indexOf(v) !== -1 : v === f.when.is
  })
}

// A page's fields grouped by section, sections in first-seen order.
function sections(schema, page) {
  var out = []
  var byName = {}
  for (var i = 0; i < schema.length; i++) {
    var f = schema[i]
    if (pageOf(f) !== page) continue
    var name = f.section || ""
    if (!Object.prototype.hasOwnProperty.call(byName, name)) {
      byName[name] = { name: name, fields: [] }
      out.push(byName[name])
    }
    byName[name].fields.push(f)
  }
  return out
}

// A module's own page, under Modules.
function modulePageId(moduleId) { return "modules/" + moduleId }

// A module's JSON file (media.json next to media.qml): its `title`,
// `settings` fields, whether it is `standalone` (no notch shows it; it has
// a place of its own), whether it is an `agent` (a coding agent in a
// terminal: `"agent": { "command": ... }`, which brings the fields every
// agent has, agentSettings; `"usage"` there names where its plan usage
// comes from, native/notchd-core/src/usage.rs, as `usage`) and whether it is `live` (a spoken
// conversation with an agent: `"live": { "command": ... }`, which brings
// liveSettings). Anything malformed is dropped, so a bad file
// costs the module its page and nothing else; `error` says why.
function parseModuleInfo(text) {
  var info = { title: "", settings: [] }
  var data
  try { data = JSON.parse(text) } catch (e) { info.error = String(e.message || e); return info }
  if (!data || typeof data !== "object" || Array.isArray(data)) { info.error = "not an object"; return info }
  if (typeof data.title === "string") info.title = data.title
  if (data.standalone === true) info.standalone = true
  var own = []
  if (Array.isArray(data.settings))
    own = data.settings.filter(function(f) { return f && typeof f === "object" && !Array.isArray(f) && typeof f.type === "string" })
  var inherited = []
  var agent = data.agent
  if (agent !== undefined) {
    if (agent && typeof agent === "object" && !Array.isArray(agent) && typeof agent.command === "string" && agent.command.trim() !== "") {
      info.agent = true
      if (typeof agent.usage === "string" && agent.usage !== "") info.usage = agent.usage
      inherited = agentSettings(agent, info.title)
    } else {
      info.error = "agent: needs a command"
    }
  }
  var live = data.live
  if (live !== undefined) {
    if (live && typeof live === "object" && !Array.isArray(live) && (live.local === true || typeof live.command === "string" && live.command.trim() !== "")) {
      info.live = true
      inherited = liveSettings(live, info.title)
    } else {
      info.error = "live: needs a command"
    }
  }
  // A field of its own replaces an inherited one with the same key.
  var ownKeys = own.map(function(f) { return f.key })
  info.settings = inherited.filter(function(f) { return ownKeys.indexOf(f.key) === -1 }).concat(own)
  return info
}

// The fields every agent has, from its file's `agent`: where it starts,
// the command line (`agent.command` is its default), when it stops, its
// size and font. `agent.help` adds to a field's help, by key: what only
// that agent needs said. `title` names it in the texts.
function agentSettings(agent, title) {
  var name = title || "the agent"
  var extra = agent.help && typeof agent.help === "object" && !Array.isArray(agent.help) ? agent.help : {}
  var fields = terminalFields(agent.command.trim(), name,
    "Empty is notchd's own folder, notchd in your Documents folder, made the first time.",
    "Closed and not working this long, " + name + " stops; the next time the notch opens, a new one starts (/resume brings the session back). 0 keeps it running.")
  // `agent.usage`: the plan's limits, asked the way its CLI does.
  if (typeof agent.usage === "string" && agent.usage !== "") fields.push(
    { key: "usage", type: "boolean", label: "Plan usage", "default": true, section: "Usage",
      help: "How much of the plan's limits (5 hours, the week) is used, in the open notch, asked every few minutes with the login " + name + " keeps. notchd only reads that login; " + name + " renews it when it runs." },
    { key: "usageWarn", type: "integer", label: "Warn at", "default": 80, min: 50, max: 100, step: 5, unit: "%", section: "Usage",
      help: "The notch peeks once when a limit passes this, and again when it is reached. 100 warns only at the limit." })
  fields.forEach(function(f) {
    if (typeof extra[f.key] === "string" && extra[f.key] !== "") f.help = f.help ? f.help + " " + extra[f.key] : extra[f.key]
  })
  return fields
}

// The voices a Codex realtime (v3) conversation speaks in; "" is Codex's choice.
var LIVE_VOICES = ["juniper", "maple", "spruce", "ember", "vale", "breeze", "arbor", "sol", "cove",
  "aube", "bubbie", "haetsal", "hanul", "himari", "leher", "miko", "neer", "porto", "rindo", "rio",
  "rivage", "selva", "sonna", "tinta", "viola"]

// The fields of a spoken conversation (LiveModule), from its file's
// `live`: the agent's command (`live.command` its default), its folder,
// the voice and what it may do there. `title` names it. With `live.local`
// it is the local one (localLiveSettings).
function liveSettings(live, title) {
  var name = title || "the agent"
  var next = "Applies to the next conversation."
  if (live.local === true) return localLiveSettings(name, next)
  return [
    { key: "command", type: "string", label: "Command", "default": live.command.trim(),
      help: "The command that runs Codex; notchd starts its server (app-server) with it. " + next },
    { key: "directory", type: "string", label: "Folder", "default": "",
      help: "Where " + name + " works when you ask it something; ~ is your home. Empty is notchd's own folder, notchd in your Documents folder. " + next },
    { key: "voice", type: "choice", label: "Voice", "default": "", display: "chips",
      options: [{ value: "", label: "Codex's" }].concat(LIVE_VOICES.map(function(v) {
        return { value: v, label: v.charAt(0).toUpperCase() + v.slice(1) }
      })),
      help: "The voice it speaks in. " + next },
    { key: "access", type: "choice", label: "May", "default": "read-only",
      options: [{ value: "read-only", label: "Only read" }, { value: "workspace-write", label: "Change files in its folder" }],
      help: "What " + name + " may do in its folder when you ask it to. It never asks for approval (no one would answer): what it may not do, it doesn't. " + next },
    { type: "component", component: "codex-live-audio", section: "Needs" }
  ]
}

// The languages of the speech models, by the code Voxtype (speech
// recognition) and the registry's voices (core/components/components.json:
// `language`) share, named in their own language.
var SPEECH_LANGUAGES = [
  { value: "pt", label: "Português" }, { value: "en", label: "English" }, { value: "es", label: "Español" },
  { value: "fr", label: "Français" }, { value: "de", label: "Deutsch" }, { value: "it", label: "Italiano" },
  { value: "nl", label: "Nederlands" }
]

// The fields of a local spoken conversation (modules/local-live.json): which
// agent's session it joins, speech to text (Voxtype) and text to speech (a
// LocalAI voice), both on this machine.
function localLiveSettings(name, next) {
  function part(id, section) { return { type: "component", component: id, section: section } }
  var STT = "Speech to text (STT)"
  var TTS = "Text to speech (TTS)"
  return [
    { key: "agent", type: "choice", section: "Agent", label: "Agent", "default": "codex",
      options: [{ value: "codex", label: "Codex" }, { value: "claude", label: "Claude Code" }],
      help: "Whose text session your words go into, the one you also type in. " + next },
    { key: "command", type: "string", section: "Agent", label: "Command", "default": "",
      help: "The command line that runs the agent; empty is codex or claude. " + next },
    { key: "session", type: "string", section: "Agent", label: "Session", "default": "last",
      help: "last: the agent's most recent session in the folder. new: one of its own. Or a session id. " + next },
    { key: "directory", type: "string", section: "Agent", label: "Folder", "default": "",
      help: "Where the agent works, and whose sessions \"last\" looks in; ~ is your home. Empty is notchd's own folder, notchd in your Documents folder. " + next },
    { key: "access", type: "choice", section: "Agent", label: "May", "default": "read-only",
      options: [{ value: "read-only", label: "Only read" }, { value: "workspace-write", label: "Change files in its folder" }],
      help: "What the agent may do in its folder when you ask it to (Claude: plan mode, or accept edits). It never asks for approval (no one would answer). " + next },
    part("libpulse", "Audio"),
    { type: "note", section: STT, label: "Listening",
      help: "Speech to text turns what you say into the words the agent receives. It is Voxtype, on this machine, with the speech model you set up for dictation." },
    part("voxtype", STT),
    { key: "language", type: "choice", section: STT, label: "Language you speak", "default": "", display: "chips",
      options: [{ value: "", label: "Voxtype's own" }, { value: "auto", label: "Auto-detect" }].concat(SPEECH_LANGUAGES),
      help: "Empty is Voxtype's own setting. Any other code Voxtype knows can be written in the config file as language. " + next },
    { key: "speechModel", type: "string", section: STT, label: "Speech model", "default": "",
      help: "A Voxtype model, such as small or large-v3-turbo (voxtype info models lists them); empty is the one Voxtype is set up with. " + next },
    { type: "note", section: TTS, label: "Speaking",
      help: "Text to speech is the voice that answers you: the agent's words, read aloud by a LocalAI voice on this machine. Piper is light and quick; Pocket TTS is more natural and heavier." },
    part("localai", TTS),
    { key: "voiceLanguage", type: "choice", section: TTS, label: "Language it speaks", "default": "pt", display: "chips",
      options: SPEECH_LANGUAGES,
      help: "The language of the voice: the voices below are the ones of this language. " + next },
    { type: "components", section: TTS, match: { engine: ["piper", "pocket"] }, by: { key: "voiceLanguage", field: "language" } },
    { key: "voiceModel", type: "choice", section: TTS, label: "Speaks with", "default": "",
      options: [{ value: "", label: "First installed" },
                { value: "piper", label: "Piper", needs: [{ engine: "piper", language: "@voiceLanguage" }] },
                { value: "pocket", label: "Pocket TTS", needs: [{ engine: "pocket", language: "@voiceLanguage" }] }],
      help: "Which voice engine speaks, in the language above: the first installed is Piper, else Pocket TTS. " + next },
    { key: "otherVoiceModel", type: "string", section: TTS, label: "Another voice model", "default": "",
      help: "Any text-to-speech model of the LocalAI you use, by its name; it takes the place of the choice above. " + next },
    { key: "voice", type: "string", section: TTS, label: "Voice", "default": "",
      help: "A voice of that model, for those that have several: Pocket TTS has alba, azelma, rafael and more. Empty is the model's own. " + next },
    { key: "localaiUrl", type: "string", section: TTS, label: "LocalAI address", "default": "",
      help: "Where LocalAI answers. Empty is notchd's own, http://127.0.0.1:18080, which it starts when a conversation needs it and stops after; an address where one is already running is used as it is. " + next }
  ]
}

// The fields of a program in a terminal (native/notchd-modules/src/term.rs), an agent or one
// of your own: folder, command (`command` its default), when it stops,
// size, font. `name` names it in the texts; `emptyDir` says where it
// starts without a folder, `idle` what stopping when idle does.
function terminalFields(command, name, emptyDir, idle) {
  var later = "Applies the next time it starts (the restart button)."
  return [
    { key: "directory", type: "string", label: "Folder", "default": "",
      help: "Where " + name + " starts; ~ is your home. " + emptyDir + " " + later },
    { key: "command", type: "string", label: "Command", "default": command,
      help: "The command line that runs " + name + ". " + later },
    { key: "idleMinutes", type: "integer", label: "Stop when idle", "default": 0, min: 0, max: 240, step: 15, unit: "min",
      help: idle },
    { key: "width", type: "integer", label: "Width", "default": 760, min: 320, max: 1600, step: 20, unit: "px" },
    { key: "height", type: "integer", label: "Terminal height", "default": 420, min: 120, max: 1000, step: 20, unit: "px" },
    { key: "font", type: "string", label: "Font", "default": "",
      help: "A monospace font family for the terminal; empty uses the theme's. " + later }
  ]
}

// The module info of a terminal of your own (Config.terminals' { id,
// title, command }), as parseModuleInfo gives a module's file: its title
// and the fields of a program in a terminal, `terminal` set. Its title and
// command are its entry's, edited there (`absolute` keys): the command is
// `required`, since an entry without one is no terminal.
function terminalInfo(t) {
  var name = t.title || t.id
  var entry = "terminals." + t.id + "."
  var later = "Applies the next time it starts (the restart button)."
  var own = [
    { key: entry + "title", absolute: true, type: "string", label: "Title", "default": "",
      help: "Its name, in the notch and here; empty is its id, " + t.id + "." },
    { key: entry + "command", absolute: true, required: true, type: "string", label: "Command",
      help: "The command line it runs, in a shell: \"btop\", \"cd ~/app && npm run dev\". " + later }
  ]
  var fields = terminalFields(t.command, name, "Empty is your home.",
    "Closed this long, " + name + " stops; the next time the notch opens, it starts again. 0 keeps it running.")
  return {
    title: t.title, terminal: true,
    settings: own.concat(fields.filter(function(f) { return f.key !== "command" }))
  }
}

// The JSON file of the module at `url` (its .qml): the same path, .json.
// Quickshell names files under the shell's folder `qs:@/qs/<path>`, which
// resolve against `shellDir`, and others `qs:///<absolute path>` (the
// Omarchy shell, for a plugin's core); "" for anything else.
function moduleInfoPath(url, shellDir) {
  var s = String(url || "")
  if (!/\.qml$/.test(s)) return ""
  var path = ""
  if (s.indexOf("file://") === 0) path = s.slice(7)
  else if (s.indexOf("qs:@/qs/") === 0 && shellDir) path = String(shellDir).replace(/\/$/, "") + "/" + s.slice(8)
  else if (s.indexOf("qs:///") === 0) path = s.slice(5)
  else return ""
  return decodeURIComponent(path).replace(/\.qml$/, ".json")
}

// A module setting's default, from its fields.
function settingDefault(fields, key) {
  fields = fields || []
  for (var i = 0; i < fields.length; i++)
    if (fields[i] && fields[i].key === key) return fields[i]["default"]
  return undefined
}

// A module's title: its own (the loaded module's, or its JSON file's),
// else its id capitalized.
function moduleTitle(id, module) {
  if (module && module.title) return module.title
  var s = String(id)
  return s.charAt(0).toUpperCase() + s.slice(1)
}

// Which list of the Modules page a module is in: "standalone" (a place of
// its own), "agent" (a coding agent, or a conversation with one: `live`),
// "terminal" (one of the config's `terminals`), else "notch".
function moduleKind(m) {
  if (m && m.standalone) return "standalone"
  if (m && (m.agent || m.live)) return "agent"
  if (m && m.terminal) return "terminal"
  return "notch"
}

// One of the Modules page's lists (`part`, a moduleKind), in catalog order.
function catalogPart(catalog, part) {
  return Array.prototype.filter.call(catalog || [], function(m) {
    return !!m && moduleKind(m) === (part || "notch")
  })
}

// Where a module shows, for the Modules page: "On" or "Off" for a
// standalone one, whose `enabled` says whether it is on (its own list says
// it is standalone); "On" or "Off" with one notch, else the notches that
// list it.
function moduleStatus(id, specs, standalone, enabled) {
  if (standalone) return enabled === false ? "Off" : "On"
  specs = specs || []
  var where = []
  for (var i = 0; i < specs.length; i++)
    if (indexIn(specs[i].moduleIds, id) !== -1) where.push(specs[i].id)
  if (where.length === 0) return "Off"
  return specs.length > 1 ? "In " + where.join(", ") : "On"
}

function indexIn(list, id) {
  return Array.prototype.indexOf.call(list || [], id)
}

// A notch's own page, under Notches, by its index.
function notchPageId(index) { return "notch/" + index }

// The notch index a page is for: "notch/2" is 2; -1 for any other page.
function notchIndexOfPage(id) {
  var m = /^notch\/([0-9]+)$/.exec(String(id || ""))
  return m ? Number(m[1]) : -1
}

// One line about a notch, for the Notches page: where it is and what it
// shows. `notch`: { edge, moduleIds }; `titles`: id -> title.
function notchSummary(notch, titles) {
  if (!notch) return ""
  titles = titles || {}
  var edge = String(notch.edge || "top")
  var where = edge.charAt(0).toUpperCase() + edge.slice(1) + " edge"
  var ids = toList(notch.moduleIds)
  var shows = ids.length === 0 ? "no modules" : ids.map(function(id) { return titles[id] || moduleTitle(id, null) }).join(", ")
  return where + " · " + shows
}

// A list value as a plain array of strings. Values that went through a
// Repeater model arrive as sequences, which Array.isArray rejects.
function toList(value) {
  if (!value || typeof value === "string" || typeof value.length !== "number") return []
  return Array.prototype.map.call(value, String)
}

// `ids` with `id` added at the end, or removed if present.
function toggleId(ids, id) {
  var next = ids.slice()
  var at = next.indexOf(id)
  if (at === -1) next.push(id)
  else next.splice(at, 1)
  return next
}

// `ids` with `id` moved `delta` places; unchanged if it would leave the list.
function moveId(ids, id, delta) {
  var next = ids.slice()
  var at = next.indexOf(id)
  var to = at + delta
  if (at === -1 || to < 0 || to >= next.length) return next
  next.splice(at, 1)
  next.splice(to, 0, id)
  return next
}

// Every id to list: enabled ones in their order, then the rest available.
function allIds(enabled, available) {
  var out = enabled.slice()
  for (var i = 0; i < available.length; i++)
    if (out.indexOf(available[i]) === -1) out.push(available[i])
  return out
}

// A workspace as the config stores it: a number for a numbered one, else
// its name.
function workspaceValue(w) {
  var s = String(w)
  return /^[0-9]+$/.test(s) ? Number(s) : s
}

function sortWorkspaces(list) {
  return list.sort(function(a, b) {
    var an = typeof a === "number", bn = typeof b === "number"
    if (an && bn) return a - b
    if (an !== bn) return an ? -1 : 1
    return a < b ? -1 : a > b ? 1 : 0
  })
}

// The workspaces to offer: 1 to 10, the ones the compositor knows and the
// ones already picked; numbers in order, then names.
function workspaceChoices(known, picked) {
  var out = []
  function add(w) {
    if (String(w) === "") return
    var v = workspaceValue(w)
    if (out.indexOf(v) === -1) out.push(v)
  }
  for (var i = 1; i <= 10; i++) add(i)
  toList(known).forEach(add)
  toList(picked).forEach(add)
  return sortWorkspaces(out)
}

// `picked` with `w` added or removed, in order; undefined once empty, which
// removes the key (every workspace).
function toggleWorkspace(picked, w) {
  var list = toList(picked).map(workspaceValue)
  var v = workspaceValue(w)
  var at = list.indexOf(v)
  if (at === -1) list.push(v)
  else list.splice(at, 1)
  return list.length > 0 ? sortWorkspaces(list) : undefined
}

// A slider value: `v` snapped to `step` from `min` and kept within
// [min, max], without float noise (0.35, not 0.35000000000000003).
function snapToStep(v, min, max, step) {
  step = Number(step) > 0 ? Number(step) : 1
  var snapped = min + Math.round((v - min) / step) * step
  var decimals = (String(step).split(".")[1] || "").length
  return Math.max(min, Math.min(max, Number(snapped.toFixed(decimals))))
}

// What a control's edit writes: undefined (remove the key) when it equals
// the field's default, so the config never stores defaults.
function storedValue(field, value) {
  return field && field["default"] !== undefined && value === field["default"] ? undefined : value
}

// ------------------------------------------------- the daemon's schema
// The daemon builds the settings schema (the `schema` message: pages and
// fields) from the config, the screens and the modules it knows: the core's
// pages, a page per notch, and a user module's own. What it does not carry
// yet the QML adds (NotchService): the Components page, the built-in modules'
// pages (their choices wait on components: `needs`), the host's own fields
// and the edge cards' previews. These put the halves together.

// The module a page is the settings of ("modules/media" is "media"), else "".
function pageModule(id) {
  var m = /^modules\/(.+)$/.exec(String(id || ""))
  return m ? m[1] : ""
}

// The pages of the window: the core's and the host's, then a page per notch
// (the daemon's), the built-in modules' (from the catalog) and the daemon's
// pages of a user module. `daemonPages` is the schema's `pages`.
function windowPages(core, host, daemonPages, catalog) {
  var subs = Array.prototype.filter.call(daemonPages || [], function(p) { return !!p && !!p.sub })
  function page(p) { return { id: p.id, label: p.label, icon: p.icon } }
  var notches = subs.filter(function(p) { return notchIndexOfPage(p.id) !== -1 })
  var modules = subs.filter(function(p) { return pageModule(p.id) !== "" })
  return mergePages(core, host).concat(notches.map(page), modules.map(page))
}

// The fields of the window the daemon's schema gives: all but the built-in
// modules' own, as the loose descriptors the controls read (without the
// daemon's resolved `value`, `visible` and `locked`: the window reads the
// config for them). The edge cards get `preview`, a QML file the daemon
// cannot name.
function windowFields(daemonFields, catalog, edgePreview) {
  var out = []
  Array.prototype.forEach.call(daemonFields || [], function(f) {
    var field = Object.assign({}, f)
    delete field.value
    delete field.visible
    delete field.locked
    if (field.type === "choice" && field.display === "cards" && field.options
        && /(^|\.)position\.edge$/.test(String(field.key || "")))
      field.options = Array.prototype.map.call(field.options, function(o) { return Object.assign({}, o, { preview: edgePreview }) })
    out.push(field)
  })
  return out
}
