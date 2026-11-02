import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load } from './load.mjs';

// What stays in JS of the terminal: the widget's side (core/Terminal.js). The
// command line, the title and the done-peek rules are Rust now
// (native/notchd-core/src/{terminal,agent}.rs, with the vectors of
// tests/vectors/rules/{terminal,agent}.json).
const Terminal = load('core/Terminal.js');

// Bug: with Breeze's palette, the messages you sent to Codex sat on an
// orange band (ANSI 8, which Codex uses when it can't ask the background).
// tests/qml/shell.qml checks the color itself where the widget is installed.
test('SCHEME: Tango, whose bright black is gray', () => {
  assert.equal(Terminal.SCHEME, 'Tango');
});

test('PROBE imports only the widget', () => {
  assert.match(Terminal.PROBE, /import QMLTermWidget/);
});

test('isLeaveKey: Ctrl+Escape alone; plain Escape is the agent\'s', () => {
  const ESC = 0x01000000, CTRL = 0x04000000, SHIFT = 0x02000000, KEYPAD = 0x20000000;
  assert.equal(Terminal.isLeaveKey(ESC, CTRL), true);
  assert.equal(Terminal.isLeaveKey(ESC, CTRL | KEYPAD), true);
  assert.equal(Terminal.isLeaveKey(ESC, 0), false);
  assert.equal(Terminal.isLeaveKey(ESC, CTRL | SHIFT), false);
  assert.equal(Terminal.isLeaveKey(0x43, CTRL), false);   // Ctrl+C goes to the agent
});
