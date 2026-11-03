import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load } from './load.mjs';

// What stays in JS of the terminal: the widget's side (core/Terminal.js). The
// command line, the title and the done-peek rules are Rust now
// (native/notchd-core/src/{terminal,agent}.rs, with the vectors of
// tests/vectors/rules/{terminal,agent}.json).
const Terminal = load('core/Terminal.js');

test('isLeaveKey: Ctrl+Escape alone; plain Escape is the agent\'s', () => {
  const ESC = 0x01000000, CTRL = 0x04000000, SHIFT = 0x02000000, KEYPAD = 0x20000000;
  assert.equal(Terminal.isLeaveKey(ESC, CTRL), true);
  assert.equal(Terminal.isLeaveKey(ESC, CTRL | KEYPAD), true);
  assert.equal(Terminal.isLeaveKey(ESC, 0), false);
  assert.equal(Terminal.isLeaveKey(ESC, CTRL | SHIFT), false);
  assert.equal(Terminal.isLeaveKey(0x43, CTRL), false);   // Ctrl+C goes to the agent
});

test('gridKey: Qt keys as W3C codes, the text only when it types', () => {
  const SHIFT = 0x02000000, CTRL = 0x04000000, ALT = 0x08000000;
  const plain = (o) => JSON.parse(JSON.stringify(o));
  assert.deepEqual(plain(Terminal.gridKey(0x41, 0, 'a')), { key: 'KeyA', mods: 0, text: 'a' });
  assert.deepEqual(plain(Terminal.gridKey(0x41, SHIFT, 'A')), { key: 'KeyA', mods: 1, text: 'A' });
  assert.deepEqual(plain(Terminal.gridKey(0x43, CTRL, '\x03')), { key: 'KeyC', mods: 4 });
  assert.deepEqual(plain(Terminal.gridKey(0x01000004, SHIFT, '\r')), { key: 'Enter', mods: 1 });
  assert.deepEqual(plain(Terminal.gridKey(0x01000002, 0, '')), { key: 'Tab', mods: 1 });
  assert.deepEqual(plain(Terminal.gridKey(0x31, ALT, '1')), { key: 'Digit1', mods: 2, text: '1' });
  assert.deepEqual(plain(Terminal.gridKey(0x0100003b, 0, '')), { key: 'F12', mods: 0 });
  // A shifted symbol has no code here: its text goes as it is.
  assert.deepEqual(plain(Terminal.gridKey(0x21, SHIFT, '!')), { key: '', mods: 1, text: '!' });
  assert.deepEqual(plain(Terminal.gridKey(0xe7, 0, 'ç')), { key: '', mods: 0, text: 'ç' });
  assert.equal(Terminal.gridKey(0x01000020, SHIFT, ''), null, 'Shift alone');
  // A dead key (Qt.Key_Dead_Acute) sends nothing: the input method composes
  // it with the next key, and the view sends what it commits ("á").
  assert.equal(Terminal.gridKey(0x01001251, 0, ''), null, 'a dead key');
});

test('gridMods and gridButton: Qt flags as the grid names them', () => {
  assert.equal(Terminal.gridMods(0x02000000 | 0x04000000), 5);
  assert.equal(Terminal.gridMods(0x08000000 | 0x10000000), 10);
  assert.equal(Terminal.gridButton(1), 'left');
  assert.equal(Terminal.gridButton(2), 'right');
  assert.equal(Terminal.gridButton(4), 'middle');
  assert.equal(Terminal.gridButton(8), '');
});
