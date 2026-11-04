import assert from 'node:assert/strict';
import {test} from 'node:test';

import {preferencesGone, requestedPreferences} from '../hosts/gnome/prefsPresence.js';

test('a vanished preferences process releases only an open settings session it owned', () => {
    assert.equal(preferencesGone(false, true), false, 'the initial absent-name callback is harmless');
    assert.equal(preferencesGone(true, false), false, 'a normally closed window is already released');
    assert.equal(preferencesGone(true, true), true, 'a crashed window cannot leave settings open');
});

test('a daemon preferences launch expires and cannot claim a later manual window', () => {
    assert.equal(requestedPreferences(100, 110), true);
    assert.equal(requestedPreferences(100, 161), false);
    assert.equal(requestedPreferences(0, 110), false);
});
