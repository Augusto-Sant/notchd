import assert from 'node:assert/strict';
import {test} from 'node:test';

import {FONTS, expandedWidth, fonts, isDark, opaque, parse, textScale, tones} from '../hosts/gnome/themeColors.js';

const hex = c => '#' + [c.r, c.g, c.b].map(v => Math.round(v * 255).toString(16).padStart(2, '0')).join('');

test('GNOME notch takes the top bar colors, whatever the color scheme', () => {
    // A bar of any color, dark or light: the notch hanging from it is the same.
    for (const [bg, fg] of [[[0, 0, 0], [255, 255, 255]], [[40, 44, 52], [220, 223, 228]], [[246, 245, 244], [46, 52, 54]]]) {
        for (const dark of [true, false]) {
            const look = tones({background: bg, foreground: fg}, dark, '#3584e4');
            assert.deepEqual(parse(hex(look.colors.background)), bg);
            assert.deepEqual(parse(hex(look.colors.foreground)), fg);
            assert.equal(look.colors.muted.a, 0.6);
            assert.equal(look.dark, isDark(bg), `the shade follows the bar, not the scheme (${bg})`);
        }
    }
});

test('GNOME notch without the bar colors is Adwaita for the scheme', () => {
    assert.equal(hex(tones(null, true, '#3584e4').colors.background), '#1e1e1e');
    assert.equal(hex(tones(null, false, '#3584e4').colors.background), '#fafafa');
    // A bar with a background but no text color keeps the background.
    const look = tones({background: [0, 0, 0], foreground: null}, false, '#3584e4');
    assert.equal(hex(look.colors.background), '#000000');
    assert.equal(hex(look.colors.foreground), '#ffffff');
});

test('a see-through bar color gives none, an opaque one its channels', () => {
    assert.equal(opaque({red: 0, green: 0, blue: 0, alpha: 0}), null);
    assert.equal(opaque({red: 0, green: 0, blue: 0, alpha: 100}), null);
    assert.equal(opaque(null), null);
    assert.deepEqual(opaque({red: 12, green: 34, blue: 56, alpha: 255}), [12, 34, 56]);
});

test('GNOME notch text follows the desktop text scaling', () => {
    assert.deepEqual(fonts(1), FONTS);
    assert.deepEqual(fonts(1.25), {caption: 13, bodySmall: 14, body: 15, heading: 20, display: 30});
    assert.deepEqual(fonts(1.5).body, 18);
    // An unreadable factor keeps the design sizes.
    for (const bad of [0, -1, NaN, Infinity, undefined, null, '1.5'])
        assert.deepEqual(fonts(bad), FONTS, `factor ${bad}`);
});

test('the open GNOME notch widens with the text scaling, never narrows', () => {
    assert.equal(expandedWidth(400, 1), 400);
    assert.equal(expandedWidth(400, 1.25), 500);
    assert.equal(expandedWidth(360, 2), 720);
    // Smaller text keeps the module's width; a bad factor is 1.
    assert.equal(expandedWidth(400, 0.8), 400);
    for (const bad of [0, -1, NaN, undefined, '2'])
        assert.equal(expandedWidth(400, bad), 400, `factor ${bad}`);
    assert.equal(textScale(1.5), 1.5);
    assert.equal(textScale(null), 1);
});
