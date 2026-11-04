// Where the notch sits: a screen edge and how far along it. Pure functions
// with no QML globals, so Node tests them too (tests/position.test.mjs).
//
// The config's `position` is { edge, offset, screen }: edge "top" |
// "bottom" | "left" | "right", offset 0..1 along the edge (left to right,
// top to bottom), 0.5 the middle, and screen the monitor the closed notch
// stays on ("" or missing: the focused one, State.notchScreen; ALL_SCREENS:
// a copy of the notch on every monitor). A missing key is top-center,
// following focus.

var EDGES = ["top", "bottom", "left", "right"]
var DEFAULT_EDGE = "top"
var DEFAULT_OFFSET = 0.5
var ALL_SCREENS = "all"

function isEdge(edge) { return EDGES.indexOf(edge) !== -1 }

// Left and right: the notch runs down the edge instead of across it.
function isVertical(edge) { return edge === "left" || edge === "right" }

// The config value as { edge, offset, screen }; anything unknown falls back
// to top-center on the focused screen.
function resolve(value) {
  var v = value && typeof value === "object" ? value : {}
  var offset = Number(v.offset)
  return {
    edge: isEdge(v.edge) ? v.edge : DEFAULT_EDGE,
    offset: v.offset === undefined || v.offset === null || !isFinite(offset)
      ? DEFAULT_OFFSET : Math.max(0, Math.min(1, offset)),
    screen: typeof v.screen === "string" ? v.screen : ""
  }
}

// The card's top-left corner on screen, in px. `s`:
//   edge, offset        the position
//   width, height       the screen
//   cardWidth, cardHeight
//   insets              { top, bottom, left, right }: what the host's bar
//                       takes on each edge; the card hangs from its edge's
//                       inset and its offset runs between the other two
//   snap                within this many px of either end of the stretch,
//                       the card goes flush against it (its shoulder would
//                       not fit there): the corner
// The card's middle goes at `offset` along the free stretch, clamped so the
// card stays inside it; a card longer than the stretch is centered on it.
// Returns { x, y, atStart, atEnd }: atStart and atEnd say the card is flush
// with that end (the left or top one is the start), where NotchCard draws
// it into the corner. Not rounded: the card's size animates in fractions of
// a pixel, and a rounded corner would step a pixel at a time while the
// opposite one glides (NotchCard snaps its content to whole pixels instead).
function place(s) {
  var edge = isEdge(s.edge) ? s.edge : DEFAULT_EDGE
  var offset = resolve({ offset: s.offset }).offset
  var insets = s.insets || {}
  function inset(e) { return Math.max(0, Number(insets[e]) || 0) }
  var snap = Math.max(0, Number(s.snap) || 0)
  var vertical = isVertical(edge)
  var length = vertical ? s.height : s.width
  var along = vertical ? s.cardHeight : s.cardWidth
  var start = vertical ? inset("top") : inset("left")
  var end = length - (vertical ? inset("bottom") : inset("right"))
  var lo = start
  var hi = end - along
  var pos, atStart = false, atEnd = false
  if (hi < lo) pos = (lo + hi) / 2
  else {
    pos = Math.max(lo, Math.min(hi, start + offset * (end - start) - along / 2))
    if (pos - lo <= snap && pos - lo <= hi - pos) { pos = lo; atStart = true }
    else if (hi - pos <= snap) { pos = hi; atEnd = true }
  }
  var depth
  if (edge === "top") depth = inset("top")
  else if (edge === "bottom") depth = s.height - inset("bottom") - s.cardHeight
  else if (edge === "left") depth = inset("left")
  else depth = s.width - inset("right") - s.cardWidth
  return vertical ? { x: depth, y: pos, atStart: atStart, atEnd: atEnd }
    : { x: pos, y: depth, atStart: atStart, atEnd: atEnd }
}

// What the host's bar takes on each edge: `size` on `barEdge` while it is
// shown, nothing elsewhere.
function insets(barEdge, size, shown) {
  var out = { top: 0, bottom: 0, left: 0, right: 0 }
  if (shown && isEdge(barEdge)) out[barEdge] = Math.max(0, Number(size) || 0)
  return out
}

// The screen edge nearest to a point.
function nearestEdge(x, y, width, height) {
  var d = { top: y, bottom: height - y, left: x, right: width - x }
  var best = "top"
  for (var i = 1; i < EDGES.length; i++)
    if (d[EDGES[i]] < d[best]) best = EDGES[i]
  return best
}

// Where a notch dragged to the pointer goes: the nearest edge, its middle
// under the pointer along that edge (the inverse of place). Within
// `magnet` px of the middle of the free stretch it snaps to 0.5 (place's
// `snap` takes it into a corner). `s`: x, y (the pointer), width, height,
// insets and magnet, as place.
function dragTarget(s) {
  var edge = nearestEdge(s.x, s.y, s.width, s.height)
  var insets = s.insets || {}
  function inset(e) { return Math.max(0, Number(insets[e]) || 0) }
  var vertical = isVertical(edge)
  var p = vertical ? s.y : s.x
  var start = vertical ? inset("top") : inset("left")
  var end = vertical ? s.height - inset("bottom") : s.width - inset("right")
  if (end <= start) return { edge: edge, offset: DEFAULT_OFFSET }
  if (Math.abs(p - (start + end) / 2) <= Math.max(0, Number(s.magnet) || 0))
    return { edge: edge, offset: DEFAULT_OFFSET }
  var offset = Math.max(0, Math.min(1, (p - start) / (end - start)))
  return { edge: edge, offset: Math.round(offset * 1000) / 1000 }
}

// A position as the config stores it: what it would get anyway left out,
// and undefined (no key) when that is all of it. `inherited` is the
// position it gets without a key of its own: a notch in `notches` takes the
// top-level `position` key by key (Config.notches), so a key equal to the
// default but not to that one must be written, or the notch goes back
// there. Without `inherited`, the defaults: top-center on the focused screen.
function configValue(position, inherited) {
  var p = resolve(position)
  var base = resolve(inherited)
  var out = {}
  if (p.edge !== base.edge) out.edge = p.edge
  if (p.offset !== base.offset) out.offset = p.offset
  if (p.screen !== base.screen) out.screen = p.screen
  return Object.keys(out).length > 0 ? out : undefined
}

// The screen a point of the desktop (global layout coordinates) is on, by
// name: the one containing it, else the nearest (a point in a gap between
// screens). `screens` are { name, x, y, width, height }; "" for none.
function screenAt(screens, x, y) {
  var best = ""
  var bestDistance = Infinity
  for (var i = 0; i < (screens || []).length; i++) {
    var s = screens[i]
    var dx = Math.max(s.x - x, 0, x - (s.x + s.width))
    var dy = Math.max(s.y - y, 0, y - (s.y + s.height))
    var d = dx * dx + dy * dy
    if (d < bestDistance) { best = s.name; bestDistance = d }
  }
  return best
}

// The stretch of an edge the notch's middle may take (place's rules): its
// two ends, { start, end }, in px along the edge. `s` as place.
function stretch(s) {
  var edge = isEdge(s.edge) ? s.edge : DEFAULT_EDGE
  var insets = s.insets || {}
  function inset(e) { return Math.max(0, Number(insets[e]) || 0) }
  var vertical = isVertical(edge)
  var length = vertical ? s.height : s.width
  return {
    start: vertical ? inset("top") : inset("left"),
    end: length - (vertical ? inset("bottom") : inset("right"))
  }
}

// Notches sharing an edge of a screen, side by side: none sits on another,
// and ones that come close join into one. `items`, each { group, center,
// size, start, end, snap, open, skip }: the group ("<screen>|<edge>"),
// where its middle wants to be and its length along the edge as it is now
// (an open one's as it grows), the stretch it must stay in (stretch) and
// how near an end of it takes it into the corner (place's `snap`), whether
// it is open, and whether it takes no part (hidden, being dragged). In each
// group, in order of `center` (ties: list order), notches that would be
// less than `gap` apart join: flush against each other, the row as near
// where its notches want to be as it can, taken into a corner like a single
// notch. The others stay where they are. So a notch that opens grows from
// where it is and pushes its neighbors where they can go: evenly to both
// sides in the middle of the edge, away from the screen's side against it.
// Returns, per item:
//   shift      to add to its place along the edge
//   covered    a closed notch pushed off the edge by an open one, which
//              should hide while it is open
//   joinStart, joinEnd
//              the index of the notch flush against its start (left or top)
//              and end, -1 for none; never with a covered one
function pack(items, gap) {
  items = items || []
  gap = Math.max(0, Number(gap) || 0)
  var out = items.map(function() {
    return { shift: 0, covered: false, joinStart: -1, joinEnd: -1 }
  })
  var groups = {}
  for (var i = 0; i < items.length; i++) {
    if (!items[i] || items[i].skip) continue
    var g = String(items[i].group)
    if (!groups[g]) groups[g] = []
    groups[g].push(i)
  }
  Object.keys(groups).forEach(function(g) {
    var order = groups[g].slice().sort(function(a, b) {
      return items[a].center - items[b].center || a - b
    })
    // Rows of joined notches: `rel` is each one's middle from the row's
    // first, `pos` the first's middle.
    var blocks = []
    function want(block) {
      var sum = 0, snap = 0
      for (var k = 0; k < block.idx.length; k++) {
        sum += items[block.idx[k]].center - block.rel[k]
        snap = Math.max(snap, Number(items[block.idx[k]].snap) || 0)
      }
      var pos = sum / block.idx.length
      var lo = -Infinity, hi = Infinity
      for (var m = 0; m < block.idx.length; m++) {
        var it = items[block.idx[m]]
        lo = Math.max(lo, it.start + it.size / 2 - block.rel[m])
        hi = Math.min(hi, it.end - it.size / 2 - block.rel[m])
      }
      if (lo > hi) { block.pos = block.free = (lo + hi) / 2; return }
      // `free`: before the corner takes it, which is what joining goes by,
      // so a notch near the corner still joins the one beside it.
      pos = block.free = Math.max(lo, Math.min(hi, pos))
      if (pos - lo <= snap && pos - lo <= hi - pos) pos = lo
      else if (hi - pos <= snap) pos = hi
      block.pos = pos
    }
    function lastEnd(block) {
      var k = block.idx.length - 1
      return block.free + block.rel[k] + items[block.idx[k]].size / 2
    }
    for (var n = 0; n < order.length; n++) {
      var block = { idx: [order[n]], rel: [0], pos: 0 }
      want(block)
      blocks.push(block)
      while (blocks.length > 1) {
        var prev = blocks[blocks.length - 2]
        var last = blocks[blocks.length - 1]
        if (lastEnd(prev) + gap + items[last.idx[0]].size / 2 <= last.free + 1e-9) break
        var base = prev.rel[prev.rel.length - 1] + items[prev.idx[prev.idx.length - 1]].size / 2
          + items[last.idx[0]].size / 2
        for (var q = 0; q < last.idx.length; q++) {
          prev.idx.push(last.idx[q])
          prev.rel.push(base + last.rel[q])
        }
        blocks.pop()
        want(prev)
      }
    }
    blocks.forEach(function(b) {
      for (var k = 0; k < b.idx.length; k++)
        out[b.idx[k]].shift = b.pos + b.rel[k] - items[b.idx[k]].center
    })
    // A closed notch an open one pushed off the edge, or still over, hides.
    var opened = order.filter(function(o) { return items[o].open })
    if (opened.length > 0) order.forEach(function(c) {
      if (items[c].open) return
      var cm = items[c].center + out[c].shift, half = items[c].size / 2
      var off = cm - half < items[c].start - 0.5 || cm + half > items[c].end + 0.5
      var under = opened.some(function(o) {
        return Math.abs(items[o].center + out[o].shift - cm) < (items[o].size + items[c].size) / 2 - 0.5
      })
      if (off || under) out[c].covered = true
    })
    // Neighbors in a row join while both show.
    blocks.forEach(function(b) {
      for (var k = 0; k + 1 < b.idx.length; k++) {
        var x = b.idx[k], y = b.idx[k + 1]
        if (out[x].covered || out[y].covered) continue
        out[x].joinEnd = y
        out[y].joinStart = x
      }
    })
  })
  return out
}

// The depth each notch takes, from pack's result and each notch's own
// depth: a joined row as deep as its deepest notch, so the row reads as
// one shape; 0 (its own) for a notch joined to none. Give an open notch
// its depth closed: it is deeper than its row and steps down to it, and
// the row stays as it was through it.
function rowDepths(packing, depths) {
  packing = packing || []
  var out = []
  for (var i = 0; i < packing.length; i++) {
    var p = packing[i]
    if (!p || (p.joinStart < 0 && p.joinEnd < 0)) { out.push(0); continue }
    var first = i
    while (packing[first] && packing[first].joinStart >= 0) first = packing[first].joinStart
    var deepest = 0
    for (var k = first; k >= 0; k = packing[k] ? packing[k].joinEnd : -1)
      deepest = Math.max(deepest, Number(depths[k]) || 0)
    out.push(deepest)
  }
  return out
}

// Notches in the same corner of a screen, one on the top or bottom edge and
// one on a side edge, would sit on each other: the top or bottom one keeps
// the corner and the side one starts where it ends, meeting it. `items`,
// each { screen, edge, atStart, atEnd, depth, shown }: where it is, whether
// it sits flush in the corner at either end of its edge (place's atStart,
// atEnd), its depth as it is now and whether it shows. Returns, per item,
// { start, end }:
//   on a side edge   how far in from each end (top, bottom) its edge now
//                    starts: the depth of the notch keeping that corner,
//                    less 1 px so they meet without a seam
//   on top or bottom how far along its far side, from each end (left,
//                    right), the side notch below it reaches: where it
//                    draws no hairline
// One window's item for corners(), for one half of the work: its full
// cornerItem if it is on the `sides` kind of edge (left/right when true,
// top/bottom when false), else only where it is. `w` has edge,
// windowScreenName and cornerItem. The edge is checked before cornerItem
// is read: a QML binding that reads a property depends on it even if the
// value is thrown away, and a side notch's cornerItem comes from its place,
// which comes from the corners the top and bottom ones give it. Reading it
// for every window looped (~1300 warnings at start, notches on a top and a
// side edge of one screen).
function cornerInput(w, sides) {
  var side = w.edge === "left" || w.edge === "right"
  return side === sides ? w.cornerItem : { screen: w.windowScreenName, edge: w.edge }
}

// One window's `key` (packItem, coverItem, closedDepth) for the work on one
// kind of edge (left/right when `sides`, top/bottom otherwise), else null,
// which pack, rowDepths and coveredByStandalone pass over. The edge is
// checked before the key is read, as in cornerInput: a side notch's place
// comes from the corners, which come from how the top and bottom ones were
// packed, so a packing that read every window's packItem fed back into
// itself (~840 warnings when a notch was added on the top edge beside a
// side one, and the new one drawn empty).
function sideInput(w, sides, key) {
  var side = w.edge === "left" || w.edge === "right"
  return side === sides ? w[key] : null
}

function corners(items) {
  items = items || []
  function deepest(screen, edge, end) {
    var d = 0
    for (var i = 0; i < items.length; i++) {
      var it = items[i]
      if (!it || !it.shown || it.screen !== screen || it.edge !== edge) continue
      if (end === "start" ? it.atStart : it.atEnd) d = Math.max(d, Number(it.depth) || 0)
    }
    return d
  }
  return items.map(function(it) {
    if (!it) return { start: 0, end: 0 }
    var side = it.edge === "left" ? "start" : "end"
    if (it.edge === "left" || it.edge === "right")
      return { start: Math.max(0, deepest(it.screen, "top", side) - 1),
               end: Math.max(0, deepest(it.screen, "bottom", side) - 1) }
    var near = it.edge === "top" ? "start" : "end"
    return { start: deepest(it.screen, "left", near), end: deepest(it.screen, "right", near) }
  })
}

// Which notches a standalone module's card covers while it shows: it draws
// in its own place, over whatever hangs there, and the notches under it
// hide until it goes. `items`, each { group, start, end, cover, shown }:
// the group ("<screen>|<edge>"), its span along the edge, whether it is a
// standalone card and whether it shows. Returns, per item, whether a shown
// cover in its group overlaps it; never for a cover.
function coveredByStandalone(items) {
  items = items || []
  return items.map(function(it) {
    if (!it || it.cover) return false
    return items.some(function(c) {
      return !!c && c.cover && c.shown && c.group === it.group && c.start < it.end && it.start < c.end
    })
  })
}
