import assert from 'node:assert/strict';
import {test} from 'node:test';

import {load} from './load.mjs';
import {placeCard} from '../hosts/gnome/layout.js';

const Position = load('core/Position.js');
const monitor = {x: 100, y: 200, width: 800, height: 600};

test('a GNOME card body reaches a corner while its shoulder stays outside it', () => {
    const top = placeCard(Position, {edge: 'top', offset: 0, monitor,
        insets: {top: 24, bottom: 0, left: 0, right: 0},
        along: 120, depth: 30, shoulder: 10, snap: 14});
    assert.equal(top.spot.atStart, true);
    assert.equal(top.target.x + 10, monitor.x);
    assert.equal(top.target.y, monitor.y + 24);
    assert.equal(top.target.w, 140);

    const side = placeCard(Position, {edge: 'left', offset: 0, monitor,
        insets: {top: 24 + 30 - 1, bottom: 0, left: 0, right: 0},
        along: 120, depth: 30, shoulder: 10, snap: 14});
    assert.equal(side.spot.atStart, true);
    assert.equal(side.target.y + 10, top.target.y + 29);
    assert.equal(side.target.x, monitor.x);
});
