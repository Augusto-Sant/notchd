import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { load, plain } from './load.mjs';

const P = load('core/client/Protocol.js');
const fixture = (dir, name, i = 0) =>
  JSON.parse(fs.readFileSync(new URL(`./protocol/${dir}/${name}.json`, import.meta.url), 'utf8')).valid[i];

test('frame: whole lines, and the unfinished tail kept for the next read', () => {
  assert.deepEqual(plain(P.frame('', '{"a":1}\n{"b":2}\n')), { lines: ['{"a":1}', '{"b":2}'], rest: '', overflow: false });
  let r = P.frame('', '{"a":1}\n{"b"');
  assert.deepEqual(plain(r), { lines: ['{"a":1}'], rest: '{"b"', overflow: false });
  r = P.frame(r.rest, ':2}\n');
  assert.deepEqual(plain(r), { lines: ['{"b":2}'], rest: '', overflow: false });
});

test('frame: a line cut at every byte comes out whole', () => {
  const text = '{"t":"state","rev":1,"x":"é\\n"}\n{"t":"ok","id":1}\n';
  const bytes = [...text];
  for (let cut = 0; cut <= bytes.length; cut++) {
    const a = P.frame('', bytes.slice(0, cut).join(''));
    const b = P.frame(a.rest, bytes.slice(cut).join(''));
    assert.deepEqual(plain(a.lines.concat(b.lines)), ['{"t":"state","rev":1,"x":"é\\n"}', '{"t":"ok","id":1}'], `cut at ${cut}`);
    assert.equal(b.rest, '');
  }
});

test('frame: blank lines and a carriage return are dropped', () => {
  assert.deepEqual(plain(P.frame('', '\n\n{"a":1}\r\n\n').lines), ['{"a":1}']);
  assert.deepEqual(plain(P.frame(undefined, undefined)), { lines: [], rest: '', overflow: false });
});

test('frame: a tail past the 4 MiB limit is dropped and reported', () => {
  const r = P.frame('', 'x'.repeat(4 * 1024 * 1024 + 1));
  assert.equal(r.overflow, true);
  assert.equal(r.rest, '');
  assert.equal(P.frame('', 'x'.repeat(4 * 1024 * 1024)).overflow, false);
});

test('parse: a message has an object shape and a string t', () => {
  assert.deepEqual(plain(P.parse('{"t":"ok","id":1}')), { t: 'ok', id: 1 });
  for (const bad of ['', 'nope', '[1]', '3', 'null', '{}', '{"t":1}', '{"t":"ok"']) assert.equal(P.parse(bad), null, bad);
});

test('building: every client message matches its fixture shape', () => {
  assert.deepEqual(plain(P.hello('quickshell-omarchy', '0.4.0', 'default', fixture('to-daemon', 'hello').caps)), fixture('to-daemon', 'hello'));
  assert.deepEqual(plain(P.pointer('main', 'DP-1', 'enter')), fixture('to-daemon', 'pointer', 0));
  assert.deepEqual(plain(P.key('main', 'DP-1', 'Tab', ['shift'], '')), fixture('to-daemon', 'key', 0));
  assert.deepEqual(plain(P.key('main', 'DP-1', 'a')), { t: 'key', notch: 'main', screen: 'DP-1', key: 'a', mods: [], text: '' });
  assert.deepEqual(plain(P.set(1, 'hoverOpenDelay', 200)), fixture('to-daemon', 'set', 0));
  assert.deepEqual(plain(P.ipc(6, 'timer', ['3m Tea'])), fixture('to-daemon', 'ipc', 0));
  assert.deepEqual(plain(P.ipc(7, 'state', [])), fixture('to-daemon', 'ipc', 1));
});

test('building: a removal and a missing value go as null, ipc arguments as strings', () => {
  assert.equal(P.encode(P.set(2, 'position', undefined)), '{"t":"set","id":2,"key":"position","value":null}\n');
  assert.equal(P.encode(P.event('timer', 'expanded', 'toggle')), '{"t":"event","module":"timer","slot":"expanded","action":"toggle","value":null}\n');
  assert.deepEqual(plain(P.event('media', 'expanded', 'seek', 0.42).value), 0.42);
  assert.deepEqual(plain(P.ipc(1, 'timer', ['3m', 5]).args), ['3m', '5']);
  assert.deepEqual(plain(P.ipc(1, 'state').args), []);
});

test('building: edit puts its fields after the op', () => {
  assert.equal(P.encode(P.edit(3, 'moveModule', { notch: 'main', module: 'media', delta: 1 })),
    '{"t":"edit","id":3,"op":"moveModule","notch":"main","module":"media","delta":1}\n');
});

test('apply: each entity message lands where it belongs', () => {
  let store = P.emptyStore();
  for (const [dir, name, i] of [['to-ui', 'config', 0], ['to-ui', 'schema', 0], ['to-ui', 'state', 0], ['to-ui', 'module', 0], ['to-ui', 'view', 0], ['to-ui', 'view', 1]]) {
    const msg = fixture(dir, name, i);
    const r = P.apply(store, msg);
    assert.equal(r.applied, true, `${name} ${i}`);
    store = r.store;
  }
  assert.equal(store.config.rev, 1);
  assert.equal(store.schema.t, 'schema');
  assert.equal(store.state.rev, 41);
  assert.equal(store.modules.media.title, 'Now playing');
  assert.equal(store.views['timer/compact'].tree.t, 'row');
  assert.equal(store.views['timer/verticalPeek'].tree, null);
  assert.deepEqual(plain(store.revs), { config: 1, schema: store.schema.rev, state: 41, 'module:media': 7, 'view:timer/compact': 112, 'view:timer/verticalPeek': 113 });
});

test('apply: a rev that is not newer than the one held changes nothing', () => {
  const state = (rev, marker) => ({ t: 'state', rev, marker });
  let r = P.apply(P.emptyStore(), state(5, 'five'));
  const held = r.store;
  for (const stale of [state(4, 'four'), state(5, 'again'), state(0, 'zero')]) {
    r = P.apply(held, stale);
    assert.equal(r.applied, false);
    assert.equal(r.store, held);
    assert.equal(r.store.state.marker, 'five');
  }
  r = P.apply(held, state(6, 'six'));
  assert.equal(r.applied, true);
  assert.equal(r.store.state.marker, 'six');
});

test('apply: revs are per entity, and the old store is left alone', () => {
  const view = (module, slot, rev) => ({ t: 'view', module, slot, rev, tree: null });
  let store = P.apply(P.emptyStore(), view('timer', 'compact', 10)).store;
  const before = store;
  // A lower rev for another slot or module is its own entity: applied.
  store = P.apply(store, view('timer', 'peek', 1)).store;
  store = P.apply(store, view('media', 'compact', 1)).store;
  assert.deepEqual(Object.keys(store.views).sort(), ['media/compact', 'timer/compact', 'timer/peek']);
  assert.equal(P.apply(store, view('timer', 'compact', 9)).applied, false);
  assert.deepEqual(Object.keys(before.views), ['timer/compact']);
  assert.notEqual(store.views, before.views);
});

test('apply: what is not an entity, or has no numeric rev, is not applied', () => {
  const store = P.emptyStore();
  for (const msg of [{ t: 'ok', id: 1 }, { t: 'surface-open', module: 'x' }, { t: 'state' }, { t: 'state', rev: '3' }, { t: 'future', rev: 1 }]) {
    const r = P.apply(store, msg);
    assert.equal(r.applied, false, msg.t);
    assert.equal(r.store, store);
  }
});

test('replies and refusal', () => {
  assert.equal(P.isReply(fixture('to-ui', 'ok')), true);
  assert.equal(P.isReply(fixture('to-ui', 'err', 0)), true);
  assert.equal(P.isReply(fixture('to-ui', 'err', 1)), false);
  assert.equal(P.isRefusal(fixture('to-ui', 'err', 1)), true);
  assert.equal(P.isRefusal(fixture('to-ui', 'err', 0)), false);
  assert.equal(P.isRefusal({ t: 'err', code: 'args' }), false);
  assert.deepEqual(plain(P.droppedReply(4)), { t: 'err', id: 4, code: 'disconnected', message: 'the daemon connection closed' });
});

test('backoff doubles from the minimum and stops at the maximum', () => {
  const seq = [0, 1, 2, 3, 4, 5, 6].map((n) => P.backoff(n, 200, 5000));
  assert.deepEqual(seq, [200, 400, 800, 1600, 3200, 5000, 5000]);
  assert.equal(P.backoff(100, 200, 5000), 5000);
});
