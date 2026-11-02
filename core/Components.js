// The rules of the optional components (what the voice modules need beyond
// the app: LocalAI and its voices, Voxtype, system packages): what a
// settings row says about one, which buttons it has, which choices wait on an
// installation, and what a module's error says to do. The registry is
// core/components/components.json; the state comes from the installer
// (core/components/install.py status). Settings and ComponentStore.qml bind
// these. Plain functions: QML imports this file as is and the Node tests run
// it (tests/components.test.mjs).

// The settings page that lists every component, and what a module's error
// points to.
var PAGE = "components"
var WHERE = "Settings › Components"

// The registry's file as a catalog: the entries that have an id, a title and
// a known kind, in order. Anything else (a bad file) is an empty catalog.
function parseCatalog(text) {
  var data
  try { data = JSON.parse(String(text)) } catch (e) { return [] }
  var list = data && Array.isArray(data.components) ? data.components : []
  return list.filter(function(c) {
    return c && typeof c.id === "string" && c.id !== "" && typeof c.title === "string"
      && (c.kind === "files" || c.kind === "localai" || c.kind === "system")
  })
}

// The installer's `status` output as { id: state }; {} for anything else.
function parseStatus(text) {
  var data
  try { data = JSON.parse(String(text)) } catch (e) { return {} }
  return data && typeof data === "object" && !Array.isArray(data) ? data : {}
}

function byId(catalog, id) {
  for (var i = 0; i < catalog.length; i++) if (catalog[i].id === id) return catalog[i]
  return null
}

// The installer's command line: `script` is the file (or a test's fake).
function installerCommand(script, args) {
  var s = String(script || "")
  return (/\.py$/.test(s) ? ["python3", s] : [s]).concat(args)
}

// A size for people: 950 B, 63 MB, 1.5 GB (decimal, as downloads are).
function formatBytes(n) {
  n = Number(n)
  if (!isFinite(n) || n <= 0) return "0 B"
  if (n < 1000) return Math.round(n) + " B"
  var units = ["kB", "MB", "GB", "TB"]
  var v = n
  var i = -1
  while (v >= 1000 && i < units.length - 1) { v /= 1000; i++ }
  var text = v >= 100 || i === 0 ? String(Math.round(v)) : (Math.round(v * 10) / 10).toFixed(1).replace(/\.0$/, "")
  return text + " " + units[i]
}

// What installing a component takes: its size, less what another installed
// component of the same backend (`backend_dir`: the Piper and Pocket TTS
// languages) already holds, its `shared_size`.
function sizeToInstall(component, shared) {
  var n = Number(component.size) || 0
  return shared ? Math.max(0, n - (Number(component.shared_size) || 0)) : n
}

// Whether an installed component holds the backend `component` runs on too.
function sharesBackend(component, catalog, installed) {
  if (!component.backend_dir) return false
  return (catalog || []).some(function(o) {
    return o.id !== component.id && o.backend_dir === component.backend_dir
      && (installed || []).indexOf(o.id) !== -1
  })
}

// What a row shows of one component: { state, percent, text, size, actions }.
//   state    "unknown" (not read yet), "missing", "working", "installed", "failed"
//   percent  0..100 while working and known, else -1 (a bar that only moves)
//   text     the state in words
//   size     "about 488 MB" (what it takes to install) or what it uses
//   actions  [{ id: "install" | "cancel" | "remove", label }]
function view(component, st, shared) {
  var size = "about " + formatBytes(sizeToInstall(component, shared))
  var out = { state: "unknown", percent: -1, text: "Checking…", size: size, actions: [] }
  if (!st) return out
  var removable = st.removable !== false && component.kind !== "system"
  if (st.active) {
    out.state = "working"
    out.percent = typeof st.percent === "number" ? st.percent : -1
    var pct = out.percent >= 0 ? " " + out.percent + " %" : ""
    if (st.phase === "downloading") {
      var bytes = st.total > 0 ? " (" + formatBytes(st.done) + " of " + formatBytes(st.total) + ")" : ""
      out.text = "Downloading" + (pct !== "" ? pct + bytes : "…")
    } else if (st.phase === "queued") {
      out.text = "Starting…"
    } else {
      out.text = "Installing" + pct + (st.detail ? ": " + st.detail : "…")
    }
    out.actions = [{ id: "cancel", label: "Cancel" }]
    return out
  }
  if (st.installed) {
    out.state = "installed"
    out.text = component.kind === "system" ? "Installed by the system"
      : "Installed" + (st.disk > 0 ? " (" + formatBytes(st.disk) + ")" : "")
    out.size = component.kind === "system" ? size : formatBytes(st.disk)
    out.actions = removable ? [{ id: "remove", label: "Remove" }] : []
    return out
  }
  if (st.phase === "failed") {
    out.state = "failed"
    out.text = "Failed: " + (st.error || "it stopped")
    out.actions = [{ id: "install", label: "Try again" }]
    if (removable && st.disk > 0) out.actions.push({ id: "remove", label: "Clean up" })
    return out
  }
  out.state = "missing"
  out.text = "Not installed"
  out.actions = [{ id: "install", label: "Install" }]
  return out
}

// The ids of the installed components, sorted: a key that changes only when
// the set does, so what depends on it is not rebuilt at every progress tick.
function installedIds(states) {
  return Object.keys(states || {}).filter(function(id) { return states[id] && states[id].installed }).sort()
}

// Whether any component is being installed (the window polls while so).
function anyActive(states) {
  return Object.keys(states || {}).some(function(id) { return states[id] && states[id].active })
}

// "Voice and Codex Live" of the module ids that need a component, by their
// `titles` ({ id: title }; an id without one is its own).
function usedBy(component, titles) {
  var ids = component.needed_by || []
  var names = ids.map(function(id) { return (titles && titles[id]) || id })
  if (names.length === 0) return ""
  if (names.length === 1) return names[0]
  return names.slice(0, -1).join(", ") + " and " + names[names.length - 1]
}

// The help under a row's title: what it is, what it takes, who needs it.
function helpText(component, titles) {
  var by = usedBy(component, titles)
  return component.description + (by ? " For " + by + "." : "")
}

// A settings field for a component, on `page` under `section`.
function field(page, section, id) {
  return { page: page, section: section, type: "component", component: id }
}

// The fields of the overview page: a note with what is installed, then every
// component under its group. `titles` names the modules ({ id: title }).
function overviewFields(catalog, states) {
  var out = [{ page: PAGE, type: "note", label: "Optional components",
               help: "Nothing here ships with notchd: install what a module needs, and remove it when you stop using it. " + diskText(catalog, states) }]
  catalog.forEach(function(c) { out.push(field(PAGE, c.group || "Other", c.id)) })
  return out
}

// "Installed components use 1.6 GB.", or "" with none.
function diskText(catalog, states) {
  var total = 0
  catalog.forEach(function(c) {
    var st = states && states[c.id]
    if (st && c.kind !== "system") total += Number(st.disk) || 0
  })
  return total > 0 ? "They use " + formatBytes(total) + " in notchd's folders." : ""
}

// The registry's components that match `where`: every key of it equals the
// component's (or is one of the values, when it is a list).
function matching(catalog, where) {
  return catalog.filter(function(c) {
    return Object.keys(where || {}).every(function(k) {
      var want = where[k]
      return Array.isArray(want) ? want.indexOf(c[k]) !== -1 : c[k] === want
    })
  })
}

// A field of type "components" ({ match, by: { key, field } }) becomes a row
// per component that matches, each shown while the field `by.key` has the
// value of the component's `by.field` (the voices of the chosen language),
// or always without `by`.
function expand(f, catalog) {
  return matching(catalog, f.match).map(function(c) {
    var row = { type: "component", component: c.id }
    if (f.section) row.section = f.section
    if (f.page) row.page = f.page
    if (f.by) row.when = { key: f.by.key, is: c[f.by.field] }
    return row
  })
}

// The ids an option's `needs` name for one step of the gating: with
// `strings`, those that are ids (null when there is none); else those that are
// registry matches, whose values may be "@key" (that field's value, `valueOf`),
// as the ids of what matches (none: an empty list), null when there is no match.
function needed(option, catalog, valueOf, strings) {
  var out = []
  var any = false
  ;(option.needs || []).forEach(function(n) {
    if (typeof n === "string") {
      if (strings) { out.push(n); any = true }
      return
    }
    if (strings) return
    any = true
    var where = {}
    Object.keys(n).forEach(function(k) {
      var v = n[k]
      where[k] = typeof v === "string" && v.charAt(0) === "@" && valueOf ? valueOf(v.slice(1)) : v
    })
    matching(catalog, where).forEach(function(c) { out.push(c.id) })
  })
  return any ? out : null
}

// A choice with the options that wait on a component marked `disabled`, and
// its help saying which; the field itself when nothing waits.
function lock(f, catalog, installed, idsOf) {
  var notes = []
  var options = f.options.map(function(o) {
    var ids = idsOf(o)
    if (ids === null) return o
    var missing = ids.filter(function(id) { return installed.indexOf(id) === -1 })
    if (ids.length > 0 && missing.length === 0) return o
    missing.forEach(function(id) {
      var c = byId(catalog, id)
      if (c && notes.indexOf(c.title) === -1) notes.push(c.title)
    })
    return Object.assign({}, o, { disabled: true })
  })
  if (options.every(function(o, i) { return o === f.options[i] })) return f
  var out = Object.assign({}, f, { options: options })
  if (notes.length > 0) {
    var note = "Needs " + notes.join(" and ") + ": install " + (notes.length > 1 ? "them" : "it") + " in " + WHERE + "."
    out.help = (f.help ? f.help + " " : "") + note
  }
  return out
}

// The schema with its component fields filled in (title as `label`, `help`;
// a "components" field is a row per component it matches) and its choices
// that wait on a component marked: an option with `needs: [ids]` is
// `disabled` while one of them isn't installed, and its field's help says
// which. Nothing is gated until the state has been read (`installed` null),
// so a choice isn't locked while it loads. A `needs` that is a match
// ({ engine, language: "@voiceLanguage" }) waits on a field's value: `gate`.
function decorate(schema, catalog, installed, titles) {
  var out = []
  schema.forEach(function(f) {
    if (f.type === "components") { out = out.concat(expand(f, catalog)); return }
    out.push(f)
  })
  return out.map(function(f) {
    if (f.type === "component") {
      var c = byId(catalog, f.component)
      if (!c) return f
      return Object.assign({}, f, { label: f.label || c.title, help: f.help || helpText(c, titles) })
    }
    if (!f.options || installed === null || installed === undefined) return f
    return lock(f, catalog, installed, function(o) { return needed(o, catalog, null, true) })
  })
}

// The second step of the gating, for what depends on a field's value: the
// options whose `needs` match the registry by "@key" are disabled while the
// matching component (the voice of the chosen language) isn't installed.
// `valueOf(key)` is a field's effective value. Run on a schema `decorate`
// made; it leaves the options without such a `needs` alone.
function gate(schema, catalog, installed, valueOf) {
  if (installed === null || installed === undefined) return schema
  return schema.map(function(f) {
    if (!f.options) return f
    return lock(f, catalog, installed, function(o) { return needed(o, catalog, valueOf, false) })
  })
}

// What a module's error adds when components are missing: "Install X and Y
// in Settings > Components." `ids` are the bridge's; titles come from the
// catalog, an unknown id stays as it is.
function missingHint(ids, catalog) {
  var names = (ids || []).map(function(id) {
    var c = byId(catalog || [], id)
    return c ? c.title : id
  })
  if (names.length === 0) return ""
  var list = names.length === 1 ? names[0] : names.slice(0, -1).join(", ") + " and " + names[names.length - 1]
  return "Install " + list + " in " + WHERE + "."
}
