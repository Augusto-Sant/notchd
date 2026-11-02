// Named icons (docs/architecture.md, "Primitive tree"): a known `name` draws
// its Nerd Font glyph, an unknown or absent one falls back to `glyph`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { load } from './load.mjs';

const T = load('core/client/prims/Tree.js');

test('a known name draws the table glyph, whatever `glyph` says', () => {
  assert.equal(T.glyphOf({ t: 'icon', name: 'media-playback-start' }), '\u{F040A}');
  assert.equal(T.glyphOf({ t: 'icon', name: 'media-playback-start', glyph: 'x' }), '\u{F040A}');
  assert.equal(T.glyphOf({ t: 'button', name: 'utilities-terminal' }), '>_');
});

test('an unknown or absent name falls back to the glyph, then to nothing', () => {
  assert.equal(T.glyphOf({ t: 'icon', name: 'no-such-icon', glyph: 'g' }), 'g');
  assert.equal(T.glyphOf({ t: 'icon', glyph: 'g' }), 'g');
  assert.equal(T.glyphOf({ t: 'icon', name: 'no-such-icon' }), '');
  assert.equal(T.glyphOf({ t: 'icon', name: 'constructor' }), '');
  assert.equal(T.glyphOf({ t: 'button', label: 'x' }), '');
});

test('the table covers every name the schema lists, and nothing else', () => {
  const schema = JSON.parse(fs.readFileSync(new URL('../native/notchd-proto/schema/view.schema.json', import.meta.url), 'utf8'));
  const names = schema.$defs.IconName.examples;
  assert.deepEqual(Object.keys(T.NAMED_GLYPHS).sort(), [...names].sort());
});

test('an icon needs a glyph or a name; an unknown name is no error', () => {
  assert.equal(T.check({ t: 'icon', name: 'media-playback-start' }, 1), '');
  assert.equal(T.check({ t: 'icon', name: 'from-the-future' }, 1), '');
  assert.equal(T.check({ t: 'icon', glyph: 'x' }, 1), '');
  assert.match(T.check({ t: 'icon' }, 1), /glyph.*name/);
  assert.match(T.check({ t: 'icon', name: 3 }, 1), /name/);
  assert.equal(T.check({ t: 'button', name: 'list-add' }, 1), '');
  assert.match(T.check({ t: 'button' }, 1), /name/);
});
