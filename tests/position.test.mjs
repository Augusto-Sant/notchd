import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, plain } from './load.mjs';

const Position = load('core/Position.js');

test('resolve: a missing or broken value is top-center', () => {
  assert.deepEqual(plain(Position.resolve(undefined)), { edge: 'top', offset: 0.5, screen: '' });
  assert.deepEqual(plain(Position.resolve('left')), { edge: 'top', offset: 0.5, screen: '' });
  assert.deepEqual(plain(Position.resolve({ edge: 'middle', offset: 'x' })), { edge: 'top', offset: 0.5, screen: '' });
  assert.deepEqual(plain(Position.resolve({ offset: null })), { edge: 'top', offset: 0.5, screen: '' });
});

test('resolve keeps a known edge and clamps the offset to 0..1', () => {
  assert.deepEqual(plain(Position.resolve({ edge: 'left' })), { edge: 'left', offset: 0.5, screen: '' });
  assert.deepEqual(plain(Position.resolve({ edge: 'bottom', offset: 0.2 })), { edge: 'bottom', offset: 0.2, screen: '' });
  assert.deepEqual(plain(Position.resolve({ edge: 'right', offset: 3 })), { edge: 'right', offset: 1, screen: '' });
  assert.deepEqual(plain(Position.resolve({ edge: 'top', offset: -1 })), { edge: 'top', offset: 0, screen: '' });
  assert.deepEqual(plain(Position.resolve({ offset: '0.25' })), { edge: 'top', offset: 0.25, screen: '' });
});

test('isVertical: only the side edges', () => {
  assert.equal(Position.isVertical('left'), true);
  assert.equal(Position.isVertical('right'), true);
  assert.equal(Position.isVertical('top'), false);
  assert.equal(Position.isVertical('bottom'), false);
});

test('insets: only the bar edge, only while the bar shows', () => {
  assert.deepEqual(plain(Position.insets('top', 30, true)), { top: 30, bottom: 0, left: 0, right: 0 });
  assert.deepEqual(plain(Position.insets('left', 28, true)), { top: 0, bottom: 0, left: 28, right: 0 });
  assert.deepEqual(plain(Position.insets('top', 30, false)), { top: 0, bottom: 0, left: 0, right: 0 });
  assert.deepEqual(plain(Position.insets('nowhere', 30, true)), { top: 0, bottom: 0, left: 0, right: 0 });
});

const screen = { width: 1000, height: 600, cardWidth: 200, cardHeight: 40, snap: 10 };
const xy = (p) => ({ x: p.x, y: p.y });
const bar = (edge, size = 30) => Position.insets(edge, size, true);

test('place: top-center hangs under the bar, centered', () => {
  assert.deepEqual(xy(Position.place({ ...screen, edge: 'top', offset: 0.5, insets: bar('top') })), { x: 400, y: 30 });
});

test('place: bottom and the side edges sit against their edge', () => {
  const none = bar('top', 0);
  assert.deepEqual(xy(Position.place({ ...screen, edge: 'bottom', offset: 0.5, insets: none })), { x: 400, y: 560 });
  const side = { ...screen, cardWidth: 40, cardHeight: 200, insets: none };
  assert.deepEqual(xy(Position.place({ ...side, edge: 'left', offset: 0.5 })), { x: 0, y: 200 });
  assert.deepEqual(xy(Position.place({ ...side, edge: 'right', offset: 0.5 })), { x: 960, y: 200 });
});

test('place: a bar on the same edge pushes the notch off it', () => {
  assert.deepEqual(xy(Position.place({ ...screen, edge: 'bottom', offset: 0.5, insets: bar('bottom') })), { x: 400, y: 530 });
  const side = { ...screen, cardWidth: 40, cardHeight: 200 };
  assert.deepEqual(xy(Position.place({ ...side, edge: 'left', offset: 0.5, insets: bar('left', 28) })), { x: 28, y: 200 });
  assert.deepEqual(xy(Position.place({ ...side, edge: 'right', offset: 0.5, insets: bar('right', 28) })), { x: 932, y: 200 });
});

test('place: the offset runs along the stretch the bar leaves free', () => {
  const side = { ...screen, cardWidth: 40, cardHeight: 100, snap: 0 };
  // Bar on top: the left edge runs from 30 to 600; the middle is 315.
  assert.deepEqual(xy(Position.place({ ...side, edge: 'left', offset: 0.5, insets: bar('top') })), { x: 0, y: 265 });
});

// Asked for: at an end of the edge the notch sits flush in the corner
// instead of short of it by its shoulder.
test('place: at either end the card goes flush into the corner', () => {
  assert.deepEqual(plain(Position.place({ ...screen, edge: 'top', offset: 0, insets: bar('top') })),
                   { x: 0, y: 30, atStart: true, atEnd: false });
  assert.deepEqual(plain(Position.place({ ...screen, edge: 'top', offset: 1, insets: bar('top') })),
                   { x: 800, y: 30, atStart: false, atEnd: true });
  // A side edge with the bar on top: flush against the bar, never under it.
  const side = { ...screen, cardWidth: 40, cardHeight: 200 };
  assert.deepEqual(plain(Position.place({ ...side, edge: 'left', offset: 0, insets: bar('top') })),
                   { x: 0, y: 30, atStart: true, atEnd: false });
  assert.deepEqual(plain(Position.place({ ...side, edge: 'right', offset: 1, insets: bar('bottom') })),
                   { x: 960, y: 370, atStart: false, atEnd: true });
});

test('place: within `snap` of an end it goes into the corner, farther it stays', () => {
  // Its left end would be 8 px from the screen's side: no room for the shoulder.
  assert.deepEqual(plain(Position.place({ ...screen, edge: 'top', offset: 0.108, insets: bar('top', 0) })),
                   { x: 0, y: 0, atStart: true, atEnd: false });
  const p = plain(Position.place({ ...screen, edge: 'top', offset: 0.112, insets: bar('top', 0) }));
  assert.equal(Math.round(p.x), 12);
  assert.equal(p.atStart, false);
  assert.equal(Position.place({ ...screen, edge: 'top', offset: 0.5, insets: bar('top', 0) }).atStart, false);
});

test('place: a card longer than the edge is centered on it', () => {
  const big = { ...screen, cardWidth: 40, cardHeight: 700, snap: 0 };
  assert.deepEqual(plain(Position.place({ ...big, edge: 'left', offset: 0, insets: bar('top', 0) })),
                   { x: 0, y: -50, atStart: false, atEnd: false });
});

test('nearestEdge: the closest side of the screen', () => {
  assert.equal(Position.nearestEdge(500, 10, 1000, 600), 'top');
  assert.equal(Position.nearestEdge(500, 590, 1000, 600), 'bottom');
  assert.equal(Position.nearestEdge(20, 300, 1000, 600), 'left');
  assert.equal(Position.nearestEdge(990, 300, 1000, 600), 'right');
  assert.equal(Position.nearestEdge(500, 300, 1000, 600), 'top');
});

const drag = { width: 1000, height: 600, insets: { top: 30, bottom: 0, left: 0, right: 0 }, magnet: 0 };

test('dragTarget: the nearest edge, the notch middle under the pointer', () => {
  assert.deepEqual(plain(Position.dragTarget({ ...drag, x: 250, y: 40 })), { edge: 'top', offset: 0.25 });
  assert.deepEqual(plain(Position.dragTarget({ ...drag, x: 900, y: 590 })), { edge: 'bottom', offset: 0.9 });
  // The left edge runs from the bar (30) to the bottom (600).
  assert.deepEqual(plain(Position.dragTarget({ ...drag, x: 10, y: 315 })), { edge: 'left', offset: 0.5 });
  assert.deepEqual(plain(Position.dragTarget({ ...drag, x: 10, y: 30 + 57 })), { edge: 'left', offset: 0.1 });
});

test('dragTarget: offsets clamp to the ends and round to a thousandth', () => {
  assert.deepEqual(plain(Position.dragTarget({ ...drag, x: -40, y: 5 })), { edge: 'left', offset: 0 });
  assert.deepEqual(plain(Position.dragTarget({ ...drag, x: 333.333, y: 20 })), { edge: 'top', offset: 0.333 });
  assert.deepEqual(plain(Position.dragTarget({ ...drag, x: 5, y: 10, width: 1000, height: 30 })), { edge: 'left', offset: 0.5 });
});

test('dragTarget: the magnet snaps near the middle', () => {
  assert.deepEqual(plain(Position.dragTarget({ ...drag, magnet: 24, x: 520, y: 40 })), { edge: 'top', offset: 0.5 });
  assert.deepEqual(plain(Position.dragTarget({ ...drag, magnet: 24, x: 530, y: 40 })), { edge: 'top', offset: 0.53 });
});

test('configValue leaves defaults out; top-center is no key at all', () => {
  assert.equal(Position.configValue({ edge: 'top', offset: 0.5 }), undefined);
  assert.equal(Position.configValue(undefined), undefined);
  assert.deepEqual(plain(Position.configValue({ edge: 'left', offset: 0.5 })), { edge: 'left' });
  assert.deepEqual(plain(Position.configValue({ edge: 'top', offset: 0.2 })), { offset: 0.2 });
  assert.deepEqual(plain(Position.configValue({ edge: 'right', offset: 0.75 })), { edge: 'right', offset: 0.75 });
});

// The notch's size animates in fractions of a pixel. Rounding its corner
// made one side step a whole pixel at a time while the other glided, which
// read as a low frame rate at the slow end of each animation.
test('place: no rounding, so a growing card grows evenly on both sides', () => {
  const at = (w) => Position.place({ edge: 'top', offset: 0.5, width: 1000, height: 600, cardWidth: w, cardHeight: 40,
                                     insets: { top: 30 }, snap: 10 });
  assert.equal(at(200.4).x, 399.8);
  assert.equal(at(200.4).x + 200.4, 600.2);
  assert.equal(at(200).x, 400);
});

test('resolve keeps the home screen by name', () => {
  assert.deepEqual(plain(Position.resolve({ edge: 'left', screen: 'eDP-1' })), { edge: 'left', offset: 0.5, screen: 'eDP-1' });
  assert.equal(Position.resolve({ screen: 3 }).screen, '');
  assert.equal(Position.resolve({ screen: 'all' }).screen, 'all');
});

// Bug: a notch in `notches` dropped on the top edge was saved without
// `edge` (top is the default), took the top-level position's "left" key by
// key, and jumped back to the left side.
test('configValue writes what differs from the inherited position, defaults included', () => {
  const top = { edge: 'left', offset: 0.957, screen: 'DP-1' };
  assert.deepEqual(plain(Position.configValue({ edge: 'top', offset: 0.508, screen: 'DP-1' }, top)), { edge: 'top', offset: 0.508 });
  assert.deepEqual(plain(Position.configValue({ edge: 'top', offset: 0.5, screen: '' }, top)), { edge: 'top', offset: 0.5, screen: '' });
  assert.equal(Position.configValue({ edge: 'left', offset: 0.957, screen: 'DP-1' }, top), undefined);
  assert.deepEqual(plain(Position.configValue({ edge: 'right', offset: 0.957, screen: 'DP-1' }, top)), { edge: 'right' });
});

test('cornerInput: the full item for the asked kind of edge, else only where it is', () => {
  const top = { edge: 'top', windowScreenName: 'A', cornerItem: { screen: 'A', edge: 'top', atStart: true, depth: 30, shown: true } };
  const side = { edge: 'left', windowScreenName: 'A', cornerItem: { screen: 'A', edge: 'left', atStart: true, depth: 20, shown: true } };
  assert.equal(Position.cornerInput(top, false), top.cornerItem);
  assert.deepEqual(plain(Position.cornerInput(side, false)), { screen: 'A', edge: 'left' });
  assert.equal(Position.cornerInput(side, true), side.cornerItem);
  assert.deepEqual(plain(Position.cornerInput(top, true)), { screen: 'A', edge: 'top' });
});

test('sideInput: the key for the asked kind of edge, else null, without reading it', () => {
  const read = [];
  const win = (edge) => ({ edge, get packItem() { read.push(edge); return { edge }; } });
  assert.deepEqual(plain(Position.sideInput(win('top'), false, 'packItem')), { edge: 'top' });
  assert.deepEqual(plain(Position.sideInput(win('right'), true, 'packItem')), { edge: 'right' });
  assert.equal(Position.sideInput(win('left'), false, 'packItem'), null);
  assert.equal(Position.sideInput(win('bottom'), true, 'packItem'), null);
  assert.deepEqual(read, ['top', 'right']);
});

test('configValue keeps the home screen, even at top-center', () => {
  assert.deepEqual(plain(Position.configValue({ edge: 'top', offset: 0.5, screen: 'eDP-1' })), { screen: 'eDP-1' });
  assert.deepEqual(plain(Position.configValue({ edge: 'left', screen: 'DP-1' })), { edge: 'left', screen: 'DP-1' });
});

test('screenAt: the screen containing a desktop point, else the nearest', () => {
  // DP-1 3440x1440 at 0,0; eDP-1 1600x1000 (scale 1.6) to its right.
  const screens = [{ name: 'DP-1', x: 0, y: 0, width: 3440, height: 1440 },
                   { name: 'eDP-1', x: 3440, y: 0, width: 1600, height: 1000 }];
  assert.equal(Position.screenAt(screens, 100, 100), 'DP-1');
  assert.equal(Position.screenAt(screens, 3500, 500), 'eDP-1');
  assert.equal(Position.screenAt(screens, 3500, 1020), 'eDP-1');   // just below eDP-1
  assert.equal(Position.screenAt(screens, 3500, 1200), 'DP-1');    // nearer DP-1's side
  assert.equal(Position.screenAt(screens, 3300, 1300), 'DP-1');
  assert.equal(Position.screenAt(screens, -50, 20), 'DP-1');
  assert.equal(Position.screenAt([], 1, 1), '');
});

const item = (center, size, extra = {}) => ({ group: 'DP-1|top', center, size, start: 0, end: 1000, ...extra });
const at = (items, gap = 10) => plain(Position.pack(items, gap)).map((r, i) => items[i].center + r.shift);

test('pack: notches apart from each other stay where they are', () => {
  assert.deepEqual(at([item(200, 100), item(600, 100)]), [200, 600]);
});

// Asked for: two notches can't sit on top of each other. Closed, they go
// side by side, flush, each moved as little as it takes.
test('pack: notches on one spot join side by side, around it', () => {
  assert.deepEqual(at([item(500, 100), item(500, 100)]), [450, 550]);
  // Partly over each other: both give way by half.
  assert.deepEqual(at([item(500, 100), item(560, 100)]), [480, 580]);
  // Three in a row, the middle one kept in place.
  assert.deepEqual(at([item(400, 100), item(500, 100), item(600, 100)], 0), [400, 500, 600]);
  assert.deepEqual(at([item(500, 100), item(500, 100), item(500, 100)], 0), [400, 500, 600]);
});

test('pack: a notch that grows pushes its neighbor along', () => {
  // The first one's compact view got wider: it now reaches the second, and
  // both give way by the same amount.
  assert.deepEqual(at([item(500, 400), item(600, 100)]), [425, 675]);
});

test('pack: notches stay inside the edge, pushed back from its ends', () => {
  assert.deepEqual(at([item(50, 100), item(60, 100)]), [50, 150]);
  // The one that wanted 940 stays left of the one that wanted 950.
  assert.deepEqual(at([item(950, 100), item(940, 100)]), [950, 850]);
});

// Asked for: notches next to each other read as one, and one dropped near
// another snaps to it.
test('pack: notches less than `gap` apart join flush; farther ones stay put', () => {
  const near = plain(Position.pack([item(200, 100), item(305, 100)], 10));
  assert.deepEqual(near.map((r, i) => [200, 305][i] + r.shift), [202.5, 302.5]);
  assert.deepEqual(near.map((r) => [r.joinStart, r.joinEnd]), [[-1, 1], [0, -1]]);
  const apart = plain(Position.pack([item(200, 100), item(320, 100)], 10));
  assert.deepEqual(apart.map((r) => r.shift), [0, 0]);
  assert.deepEqual(apart.map((r) => [r.joinStart, r.joinEnd]), [[-1, -1], [-1, -1]]);
});

test('pack: a row near an end of the edge goes into the corner, like one notch', () => {
  assert.deepEqual(at([item(60, 100, { snap: 14 }), item(165, 100, { snap: 14 })]), [50, 150]);
  assert.deepEqual(at([item(940, 100, { snap: 14 }), item(835, 100, { snap: 14 })]), [950, 850]);
  // Farther than `snap`: where it was.
  assert.deepEqual(at([item(300, 100, { snap: 14 })]), [300]);
});

test('pack: only notches on one screen edge push each other', () => {
  assert.deepEqual(at([item(500, 100), item(500, 100, { group: 'DP-1|left' }), item(500, 100, { group: 'eDP-1|top' })]),
                   [500, 500, 500]);
  assert.deepEqual(at([item(500, 100), item(500, 100, { skip: true })]), [500, 500]);
});

test('stretch: the ends of the edge the middle of a notch may take', () => {
  assert.deepEqual(plain(Position.stretch({ edge: 'top', width: 1000, height: 600, insets: { left: 40 } })),
                   { start: 40, end: 1000 });
  assert.deepEqual(plain(Position.stretch({ edge: 'left', width: 1000, height: 600, insets: { top: 30 } })),
                   { start: 30, end: 600 });
});

// Asked for: joined notches of different depths looked odd, a step
// between them. A joined row takes its deepest notch's depth.
test('rowDepths: a joined row is as deep as its deepest notch; the rest keep theirs', () => {
  const packing = [{ joinStart: -1, joinEnd: 1 }, { joinStart: 0, joinEnd: 2 }, { joinStart: 1, joinEnd: -1 },
                   { joinStart: -1, joinEnd: -1 }];
  assert.deepEqual(plain(Position.rowDepths(packing, [20, 28, 5, 40])), [28, 28, 28, 0]);
  assert.deepEqual(plain(Position.rowDepths([], [])), []);
  assert.deepEqual(plain(Position.rowDepths([null], [10])), [0]);
});


// Asked for: a notch that opens (a peek, the pointer) grows to fit it,
// pushing its neighbors where they can go: in the middle of the edge it
// grows evenly and pushes both sides; against the screen's side it grows
// away from it, pushing the others along.
test('pack: an open notch grows from where it is and pushes its row aside', () => {
  // A row of three in the middle; the middle one opens to 300.
  const row = [item(400, 100), item(500, 300, { open: true }), item(600, 100)];
  assert.deepEqual(at(row), [300, 500, 700]);
  const r = plain(Position.pack(row, 10));
  assert.deepEqual(r.map((x) => x.covered), [false, false, false]);
  // Still joined, the open one too: it steps down to its neighbors.
  assert.deepEqual(r.map((x) => [x.joinStart, x.joinEnd]), [[-1, 1], [0, 2], [1, -1]]);
});

test('pack: against the screen side, an open notch grows away from it', () => {
  // The row sits in the corner; the first one opens to 300.
  assert.deepEqual(at([item(50, 300, { open: true, snap: 14 }), item(150, 100, { snap: 14 })]), [150, 350]);
  // The last one, against the far side.
  assert.deepEqual(at([item(850, 100, { snap: 14 }), item(950, 300, { open: true, snap: 14 })]), [650, 850]);
});

test('pack: an open notch reaching one apart pushes it, and joins it while it does', () => {
  const r = plain(Position.pack([item(300, 300, { open: true }), item(505, 100)], 10));
  assert.deepEqual(r.map((x, i) => [300, 505][i] + x.shift), [302.5, 502.5]);
  assert.deepEqual(r.map((x) => [x.joinStart, x.joinEnd]), [[-1, 1], [0, -1]]);
});

test('pack: only notches that do not fit on the edge hide under an open one', () => {
  const r = plain(Position.pack([item(500, 900, { open: true }), item(500, 300)], 10));
  assert.deepEqual(r.map((x) => x.covered), [false, true]);
  assert.deepEqual(r.map((x) => [x.joinStart, x.joinEnd]), [[-1, -1], [-1, -1]]);
});

// Asked for: notches in the same corner of a screen, one on the top (or
// bottom) edge and one on the side, sat on each other. The top or bottom
// one keeps the corner; the side one starts where it ends, meeting it
// (1 px into it, so no seam shows), and gets pushed along as it grows.
const corner = (screen, edge, extra = {}) => ({ screen, edge, atStart: false, atEnd: false, depth: 30, along: 200, shown: true, ...extra });

test('corners: a side notch starts below a top notch in its corner, above a bottom one', () => {
  const r = plain(Position.corners([
    corner('A', 'top', { atStart: true, depth: 40 }),
    corner('A', 'left', { atStart: true, depth: 30 }),
    corner('A', 'bottom', { atStart: true, depth: 50 }),
    corner('A', 'right')
  ]));
  assert.deepEqual(r[1], { start: 39, end: 49 });
  // Nothing in the top-right or bottom-right corner.
  assert.deepEqual(r[3], { start: 0, end: 0 });
});

test('corners: the top or bottom notch leaves the side notch its stretch of its far side', () => {
  const r = plain(Position.corners([
    corner('A', 'top', { atEnd: true, depth: 40 }),
    corner('A', 'right', { atStart: true, depth: 26 }),
    corner('A', 'bottom', { atEnd: true }),
    corner('A', 'right', { atEnd: true, depth: 60 })
  ]));
  // Its far side's end runs into the side notch below it: no hairline there.
  assert.deepEqual(r[0], { start: 0, end: 26 });
  assert.deepEqual(r[2], { start: 0, end: 60 });
});

test('corners: only flush notches on one screen that show count', () => {
  const r = plain(Position.corners([
    corner('A', 'top', { atStart: true, depth: 40 }),
    corner('B', 'left', { atStart: true }),
    corner('A', 'top', { atEnd: true, depth: 40, shown: false }),
    corner('A', 'right'),
    corner('A', 'top', { depth: 40 })
  ]));
  assert.deepEqual(r[1], { start: 0, end: 0 });
  assert.deepEqual(r[3], { start: 0, end: 0 });
});

test('coveredByStandalone: a shown standalone card hides the notches it overlaps there', () => {
  const items = [
    { group: 'A|top', start: 400, end: 600 },                          // under the OSD
    { group: 'A|top', start: 900, end: 1000 },                         // beside it
    { group: 'B|top', start: 400, end: 600 },                          // another screen
    { group: 'A|top', start: 450, end: 550, cover: true, shown: true } // the OSD
  ];
  assert.deepEqual(plain(Position.coveredByStandalone(items)), [true, false, false, false]);
  items[3].shown = false;
  assert.deepEqual(plain(Position.coveredByStandalone(items)), [false, false, false, false]);
  assert.deepEqual(plain(Position.coveredByStandalone([{ group: 'A|top', start: 0, end: 10 }, { group: 'A|top', start: 10, end: 20, cover: true, shown: true }])),
    [false, false], 'touching is not overlapping');
  assert.deepEqual(plain(Position.coveredByStandalone(undefined)), []);
});
