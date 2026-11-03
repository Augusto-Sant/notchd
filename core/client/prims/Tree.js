// The checks a UI runs on one node of a view tree (docs/architecture.md,
// "Primitive tree (v1)"). Plain ES5 functions, so QML imports them as is.
// `check` looks at one node only: a container's children are checked by their
// own ViewTree. An invalid node renders as an error row, never a crash.

var FILES = {
  row: "RowPrim", column: "ColumnPrim", stack: "StackPrim", spacer: "SpacerPrim", text: "TextPrim",
  icon: "IconPrim", image: "ImagePrim", progress: "ProgressPrim", meter: "MeterPrim",
  equalizer: "EqualizerPrim", list: "ListPrim", button: "ButtonPrim", chip: "ChipPrim",
  canvas: "CanvasPrim", surface: "SurfacePrim"
}

var MAX_DEPTH = 16

// Nodes that fire their own action (the rest get a click area when they
// carry an `action`).
var SELF_ACTING = { button: true, chip: true, progress: true }

function file(t) {
  return Object.prototype.hasOwnProperty.call(FILES, t) ? FILES[t] : ""
}

function selfActing(t) {
  return SELF_ACTING[t] === true
}

function isNumber(v) { return typeof v === "number" && isFinite(v) }
function isString(v) { return typeof v === "string" }
// A Repeater model hands arrays over as sequences, so Array.isArray is not enough.
function isList(v) { return v !== null && typeof v === "object" && typeof v.length === "number" }
function absent(v) { return v === undefined || v === null }

function check(node, depth) {
  if (node === null || typeof node !== "object" || isList(node)) return "a node must be an object"
  if (!isString(node.t)) return "a node needs a `t`"
  if (file(node.t) === "") return "unknown primitive `" + node.t + "`"
  if (depth > MAX_DEPTH) return "tree deeper than " + MAX_DEPTH
  var t = node.t
  if (!absent(node.visible) && typeof node.visible !== "boolean") return t + ": `visible` must be a boolean"
  if (!absent(node.action) && !isString(node.action)) return t + ": `action` must be a string"
  if (!absent(node.fill) && typeof node.fill !== "boolean") return t + ": `fill` must be a boolean"
  if (t === "row" || t === "column" || t === "stack" || t === "list") {
    if (!isList(node.children)) return t + ": `children` must be a list"
  } else if (t === "text") {
    if (!isString(node.text)) return "text: `text` must be a string"
  } else if (t === "icon") {
    if (!absent(node.glyph) && !isString(node.glyph)) return "icon: `glyph` must be a string"
    if (!absent(node.name) && !isString(node.name)) return "icon: `name` must be a string"
    if (!isString(node.glyph) && !isString(node.name)) return "icon: needs a `glyph` or a `name`"
  } else if (t === "image") {
    if (!isString(node.src)) return "image: `src` must be a string"
  } else if (t === "progress") {
    if (!absent(node.value) && (!isNumber(node.value) || node.value < 0 || node.value > 1)) return "progress: `value` must be within 0..1"
    if (!absent(node.anchor) && (typeof node.anchor !== "object" || !isNumber(node.anchor.at) || !isNumber(node.anchor.rate))) return "progress: `anchor` needs `at` and `rate`"
  } else if (t === "meter") {
    if (!isNumber(node.value) || node.value < 0 || node.value > 1) return "meter: `value` must be within 0..1"
  } else if (t === "equalizer") {
    if (typeof node.active !== "boolean") return "equalizer: `active` must be a boolean"
  } else if (t === "button") {
    if (!isString(node.glyph) && !isString(node.name) && !isString(node.label)) return "button: needs a `glyph`, a `name` or a `label`"
  } else if (t === "chip") {
    if (!isString(node.label)) return "chip: `label` must be a string"
  } else if (t === "canvas") {
    if (!isNumber(node.w) || !isNumber(node.h) || node.w <= 0 || node.h <= 0) return "canvas: `w` and `h` must be positive"
    if (!isList(node.cmds)) return "canvas: `cmds` must be a list"
  } else if (t === "surface") {
    if (!isString(node.kind) || !isString(node.ref) || node.ref === "") return "surface: needs a `kind` and a `ref`"
  }
  return ""
}

// The Nerd Font glyph of each named icon (`notchd-proto`'s icons::NAMES, the
// names the official modules send): a code point, or the text a name was drawn
// with before. A UI draws its own symbol for a name; this is the Nerd Font one.
var NAMED_GLYPHS = {
  "media-playback-start": 0xF040A,
  "media-playback-pause": 0xF03E4,
  "media-playback-stop": 0xF04DB,
  "media-skip-forward": 0xF04AD,
  "media-skip-backward": 0xF04AE,
  "audio-x-generic": 0xF075A,
  "audio-input-microphone": 0xF036C,
  "microphone-disabled": 0xF036D,
  "list-add": 0xF0415,
  "view-refresh": 0xF0709,
  "window-close": 0xF0156,
  "alarm": 0xF051B,
  "notification": 0xF009E,
  "dialog-warning": 0xF071,
  "utilities-terminal": ">_",
  "drive-harddisk": 0xF02CA,
  "cpu": 0xF0EE0,
  "memory": 0xF035B,
  "focus": 0xF09D1,
  "coffee": 0xF0176
}

// What a node with a `glyph` and/or a `name` draws: the name's glyph when the
// table knows it (an unknown name is no error), else the `glyph`, else nothing.
function glyphOf(node) {
  var name = node.name
  if (typeof name === "string" && NAMED_GLYPHS.hasOwnProperty(name)) {
    var g = NAMED_GLYPHS[name]
    return typeof g === "number" ? String.fromCodePoint(g) : g
  }
  return typeof node.glyph === "string" ? node.glyph : ""
}

// The `surfaceHost` of the nearest ancestor that has one (the ViewTree a
// module's slot is drawn in, which CoreModule gives its service), or null.
function hostOf(item) {
  var p = item.parent
  while (p) {
    var found = p.surfaceHost
    if (found !== undefined && found !== null) return found
    p = p.parent
  }
  return null
}
