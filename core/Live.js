// The rules of a spoken conversation with an agent (LiveModule.qml): the
// bridge's command line, what its lines mean, the conversation's state and
// what the notch says about it, when a session runs, and the IPC commands.
// The bridge is core/live/codex-live.py; its lines are documented there.
// Plain functions: QML imports this file as is and the Node tests run it.

// The transcript keeps this many lines; older ones go.
var MAX_LINES = 40
// Codex's voice, in dB, above which it is speaking.
var SPEAKING_DB = -60

// The bridge's command line: `bridge` is the script (or a test's fake).
// `opts`: { codex, cwd, voice, access }; an empty folder is the bridge's
// own default (notchd's folder in your Documents), "~" is your `home`.
function bridgeArgs(bridge, opts, home) {
  var b = String(bridge || "")
  var run = /\.py$/.test(b) ? ["python3", b] : [b]
  opts = opts || {}
  var codex = String(opts.codex || "").trim() || "codex"
  run = run.concat(["--codex", codex])
  var cwd = String(opts.cwd || "").trim()
  if (cwd === "~" || cwd.indexOf("~/") === 0) cwd = String(home || "") + cwd.slice(1)
  if (cwd !== "") run = run.concat(["--cwd", cwd])
  if (opts.voice) run = run.concat(["--voice", String(opts.voice)])
  run = run.concat(["--access", opts.access === "workspace-write" ? "workspace-write" : "read-only"])
  return run
}

// The same for the local bridge (core/live/local-live.py: whisper.cpp, an
// agent's text session, Piper). `opts`: { agent, command, session, cwd,
// access, whisperModel, voice, lang }; what is empty is the bridge's own
// default (the models in notchd's data folder, the language pt, the
// agent's most recent session in its folder); "~" is your `home`.
function localBridgeArgs(bridge, opts, home) {
  var b = String(bridge || "")
  var run = /\.py$/.test(b) ? ["python3", b] : [b]
  opts = opts || {}
  function path(v) {
    var p = String(v || "").trim()
    if (p === "~" || p.indexOf("~/") === 0) p = String(home || "") + p.slice(1)
    return p
  }
  run = run.concat(["--agent", opts.agent === "claude" ? "claude" : "codex"])
  var command = String(opts.command || "").trim()
  if (command !== "") run = run.concat(["--command", command])
  run = run.concat(["--session", String(opts.session || "").trim() || "last"])
  if (path(opts.cwd) !== "") run = run.concat(["--cwd", path(opts.cwd)])
  run = run.concat(["--access", opts.access === "workspace-write" ? "workspace-write" : "read-only"])
  if (path(opts.whisperModel) !== "") run = run.concat(["--whisper-model", path(opts.whisperModel)])
  if (path(opts.voice) !== "") run = run.concat(["--voice", path(opts.voice)])
  var lang = String(opts.lang || "").trim()
  if (lang !== "") run = run.concat(["--lang", lang])
  return run
}

// A line of the bridge -> its event, or null for anything else.
function parseLine(line) {
  var e
  try { e = JSON.parse(String(line)) } catch (err) { return null }
  return e && typeof e === "object" && !Array.isArray(e) ? e : null
}

// The state with no session: `status` is "off", "starting", "connecting",
// "live" or "closed"; `transcript` [{ role, text, done }]; `level` Codex's
// voice in dB as it plays, `mic` what the microphone sends.
function initialState() {
  return { status: "off", error: "", packages: [], warning: "", transcript: [],
           working: false, muted: false, level: -100, mic: -100 }
}

// A new session: the old one's transcript stays, its lines closed (one it
// left open would take the next session's words), its errors gone.
function starting(state) {
  var s = state || initialState()
  return Object.assign({}, s, {
    transcript: (s.transcript || []).map(function(l) { return l.done ? l : { role: l.role, text: l.text, done: true } }),
    status: "starting", error: "", packages: [], warning: "", working: false, muted: false, level: -100, mic: -100
  })
}

// The state after one of the bridge's events.
function reduce(state, e) {
  var s = Object.assign({}, state || initialState())
  if (!e) return s
  if (typeof e.status === "string") {
    s.status = e.status
    if (e.status === "closed") { s.working = false; s.level = -100; s.mic = -100 }
  }
  if (typeof e.error === "string") {
    s.error = e.error
    s.packages = Array.isArray(e.packages) ? e.packages.map(String) : []
  }
  if (typeof e.warning === "string") s.warning = e.warning
  if (typeof e.working === "boolean") s.working = e.working
  if (typeof e.muted === "boolean") s.muted = e.muted
  if (typeof e.level === "number") s.level = e.level
  if (typeof e.mic === "number") s.mic = e.mic
  if (e.transcript && typeof e.transcript === "object") s.transcript = addTranscript(s.transcript, e.transcript)
  return s
}

// A piece of the transcript into the lines: a delta goes on its role's open
// line (or starts one), "done" sets that line's whole text. Both sides can
// be open at once (Codex starts answering before what you said is done).
function addTranscript(lines, t) {
  var role = t.role === "user" ? "user" : "assistant"
  var out = (lines || []).slice()
  var open = -1
  for (var i = out.length - 1; i >= 0; i--)
    if (out[i].role === role && !out[i].done) { open = i; break }
  if (t.done) {
    var text = String(t.text || "").trim()
    if (open >= 0) out[open] = { role: role, text: text, done: true }
    else if (text !== "") out.push({ role: role, text: text, done: true })
  } else {
    var delta = String(t.delta || "")
    if (delta === "") return out
    if (open >= 0) out[open] = { role: role, text: out[open].text + delta, done: false }
    else out.push({ role: role, text: delta.replace(/^\s+/, ""), done: false })
  }
  return out.slice(Math.max(0, out.length - MAX_LINES))
}

function speaking(state) { return !!state && state.status === "live" && state.level > SPEAKING_DB }
// How loud the microphone is, 0 (silence, the room) .. 1, for the dot that
// shows it hears you: -60 dB and under is nothing, -20 dB is full.
function micLevel(state) {
  if (!state || state.status !== "live" || state.muted) return 0
  return Math.max(0, Math.min(1, (Number(state.mic) + 60) / 40))
}
function running(state) {
  return !!state && (state.status === "starting" || state.status === "connecting" || state.status === "live")
}

// One word or two for the notch: what the conversation is doing.
function statusText(state) {
  if (!state) return ""
  if (state.error) return "Stopped"
  switch (state.status) {
  case "starting":
  case "connecting": return "Connecting…"
  case "live":
    if (speaking(state)) return "Speaking"
    if (state.working) return "Working"
    return state.muted ? "Muted" : "Listening"
  case "closed": return "Ended"
  default: return "Off"
  }
}

// What went wrong, with what to install when that's why.
function errorText(state) {
  if (!state || !state.error) return ""
  // The local bridge's says what to install and how, itself.
  if (state.packages && state.packages.length > 0 && state.error.indexOf("(make ") === -1)
    return state.error + " Install " + state.packages.join(", ") + " (make live-deps)."
  return state.error
}

// Whether a session should run: the notch is open on the module (pinned:
// a click or a shortcut, not a hover) and you haven't stopped it since it
// opened.
function wanted(open, stopped) { return !!open && !stopped }

// An IPC command: "start" (opens its notch, which starts a session),
// "stop", "toggle", "new" (a new session, the transcript cleared), "mute",
// or "unmute". { action, text }, action "usage" for anything else.
var COMMANDS = ["start", "stop", "toggle", "new", "mute", "unmute"]
function parseCommand(raw) {
  var s = String(raw || "").trim()
  if (COMMANDS.indexOf(s) !== -1) return { action: s, text: "" }
  return { action: "usage", text: "" }
}

// The bridge's file as a path: Qt.resolvedUrl gives file:// urls, or under
// Quickshell `qs:@/qs/<path>` (resolved against `shellDir`) and
// `qs:///<absolute path>` (the Omarchy shell, for a plugin's core). "" for
// anything else.
function localPath(url, shellDir) {
  var s = String(url || "")
  var path = ""
  if (s.indexOf("file://") === 0) path = s.slice(7)
  else if (s.indexOf("qs:@/qs/") === 0 && shellDir) path = String(shellDir).replace(/\/$/, "") + "/" + s.slice(8)
  else if (s.indexOf("qs:///") === 0) path = s.slice(5)
  else return ""
  return decodeURIComponent(path)
}
