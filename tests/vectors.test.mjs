// Runs the language-neutral golden vectors (tests/vectors/) against the JS
// they were taken from. Every UI implementation of the geometry must pass the
// same files; here they prove the vectors are correct today and stay in sync.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { load, plain } from './load.mjs';

const dir = new URL('./vectors/geometry/', import.meta.url);
const files = { 'position.json': load('core/Position.js'), 'outline.json': load('core/Outline.js') };

for (const [file, mod] of Object.entries(files)) {
  const suite = JSON.parse(fs.readFileSync(new URL(file, dir), 'utf8'));
  assert.ok(suite.cases.length > 0, `${file} has cases`);
  const names = new Set();
  for (const c of suite.cases) {
    test(`vectors ${file}: ${c.fn}: ${c.name}`, () => {
      assert.equal(typeof mod[c.fn], 'function', `${c.fn} exists`);
      assert.ok(!names.has(c.fn + c.name), 'case names are unique per function');
      names.add(c.fn + c.name);
      const args = JSON.parse(JSON.stringify(c.args));
      // The JS is held to exact equality; `tolerance` is for other languages.
      assert.deepStrictEqual(plain(mod[c.fn](...args)), c.expected);
    });
  }
}
