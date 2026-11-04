import assert from 'node:assert/strict';
import {test} from 'node:test';

import {hexColor} from '../hosts/gnome/terminalColors.js';

test('GNOME terminal colors use the daemon grid protocol format', () => {
    assert.equal(hexColor({r: 40 / 255, g: 40 / 255, b: 40 / 255, a: 1}), '#282828');
    assert.equal(hexColor({r: 1, g: 0.5, b: 0, a: 0.4}), '#ff8000');
});
