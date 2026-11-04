import QtQuick
import "Position.js" as Position

// Where the notch windows go beside each other (NotchSurface's): packed
// side by side on each edge (Position.pack), a joined row's depth
// (Position.rowDepths), the corners top and side notches share
// (Position.corners) and what a standalone card covers
// (Position.coveredByStandalone). No window of its own, so the QML tests
// run it on plain objects with the windows' bindings.
//
// The work runs in two halves, top and bottom edges apart from the sides,
// so no window's place feeds back into itself: a side notch starts where
// the corner's top notch ends (`cornerInsets`), which is where that one was
// packed, how deep its row is and whether it shows; the top notches'
// far sides only draw where the side ones meet them (`cornerSpans`). Notches
// on different edges never push each other, so nothing is lost by it.
//
// Each window has: copy, edge, windowScreenName, packItem, coverItem,
// closedDepth and cornerItem (NotchWindow).
QtObject {
  id: layout

  property var windows: []

  // Notches closer than this join flush into one shape: two shoulders,
  // which flare past the card, and a little air would not fit between them.
  readonly property real packGap: 2 * Theme.space(10) + Theme.space(8)

  function inputs(sides, key) {
    return layout.windows.map(function(w) { return Position.sideInput(w, sides, key) })
  }
  function isSide(w) { return !!w && (w.edge === "left" || w.edge === "right") }
  function indexOf(copy) {
    var list = layout.windows
    for (var i = 0; i < list.length; i++) if (list[i].copy === copy) return i
    return -1
  }

  // ---------------------------------------------------------- side by side
  readonly property var packingAcross: Position.pack(layout.inputs(false, "packItem"), layout.packGap)
  readonly property var packingDown: Position.pack(layout.inputs(true, "packItem"), layout.packGap)
  // A joined row as deep as its deepest notch closed; an open one, deeper,
  // steps down to its neighbors and keeps the row as it was.
  readonly property var rowDepthsAcross: Position.rowDepths(layout.packingAcross, layout.inputs(false, "closedDepth"))
  readonly property var rowDepthsDown: Position.rowDepths(layout.packingDown, layout.inputs(true, "closedDepth"))

  // { shift, covered, joinStart, joinEnd } (Position.pack), joins as
  // indexes into `windows`; null for a window not in the list.
  function packOf(copy) {
    var i = layout.indexOf(copy)
    if (i < 0) return null
    return (layout.isSide(layout.windows[i]) ? layout.packingDown : layout.packingAcross)[i] || null
  }
  function rowDepthOf(copy) {
    var i = layout.indexOf(copy)
    if (i < 0) return 0
    return (layout.isSide(layout.windows[i]) ? layout.rowDepthsDown : layout.rowDepthsAcross)[i] || 0
  }
  function windowAt(index) {
    return index >= 0 && index < layout.windows.length ? layout.windows[index] : null
  }

  // ---------------------------------------------------------------- corners
  // Where side notches start, from the top and bottom ones flush in the
  // corners; and where those meet the side notches, from the side ones.
  readonly property var cornerInsets: Position.corners(layout.windows.map(function(w) { return Position.cornerInput(w, false) }))
  readonly property var cornerSpans: Position.corners(layout.windows.map(function(w) { return Position.cornerInput(w, true) }))
  function cornerOf(copy, list) {
    var i = layout.indexOf(copy)
    return i >= 0 && list[i] ? list[i] : { start: 0, end: 0 }
  }
  function cornerInsetOf(copy) { return layout.cornerOf(copy, layout.cornerInsets) }
  function cornerSpanOf(copy) { return layout.cornerOf(copy, layout.cornerSpans) }

  // ------------------------------------------------------------- standalone
  // A standalone module's card (the OSD) keeps its own place and draws over
  // the notches it overlaps on its edge, which hide while it shows.
  readonly property var coversAcross: Position.coveredByStandalone(layout.inputs(false, "coverItem"))
  readonly property var coversDown: Position.coveredByStandalone(layout.inputs(true, "coverItem"))
  function coveredByStandalone(copy) {
    var i = layout.indexOf(copy)
    if (i < 0) return false
    return !!(layout.isSide(layout.windows[i]) ? layout.coversDown : layout.coversAcross)[i]
  }
}
