// Runs the rule vectors (tests/vectors/rules/) against the JS they were taken
// from. native/notchd-core/tests/vectors.rs runs the same files against the
// Rust port: while both exist, they must agree (the differential check).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { SOURCES, call, has } from './vectors/rules/adapt.mjs';

const dir = new URL('./vectors/rules/', import.meta.url);

for (const file of Object.keys(SOURCES)) {
  const suite = JSON.parse(fs.readFileSync(new URL(file, dir), 'utf8'));
  assert.ok(suite.cases.length > 0, `${file} has cases`);
  const names = new Set();
  let skipped = 0;
  for (const c of suite.cases) {
    if (c.knownDiff || !has(file, c.fn)) {
      skipped++;
      continue;
    }
    test(`rules ${file}: ${c.fn}: ${c.name}`, () => {
      assert.ok(!names.has(c.fn + c.name), 'case names are unique per function');
      names.add(c.fn + c.name);
      const got = call(file, c.fn, c.args);
      assert.deepStrictEqual(got, c.expected);
      // `ordered`: the keys come out in the same order too (config edits).
      if (c.ordered) assert.equal(JSON.stringify(got), JSON.stringify(c.expected));
    });
  }
  if (skipped) console.log(`rules ${file}: ${skipped} case(s) skipped, knownDiff or a function the JS no longer has`);
}
