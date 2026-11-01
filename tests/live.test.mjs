import { test } from 'node:test';
import assert from 'node:assert/strict';
import { load, plain } from './load.mjs';

const Live = load('core/Live.js');

function run(events) {
  return events.reduce((s, e) => Live.reduce(s, e), Live.initialState());
}

test('bridgeArgs: python for the script, the fake as is; options in order', () => {
  assert.deepEqual(plain(Live.bridgeArgs('/c/live/codex-live.py', {}, '/home/u')),
    ['python3', '/c/live/codex-live.py', '--codex', 'codex', '--access', 'read-only']);
  assert.deepEqual(plain(Live.bridgeArgs('/t/bin/codex-live', { codex: ' codex -p work ', cwd: '~/code', voice: 'maple', access: 'workspace-write' }, '/home/u')),
    ['/t/bin/codex-live', '--codex', 'codex -p work', '--cwd', '/home/u/code', '--voice', 'maple', '--access', 'workspace-write']);
});

test('bridgeArgs: an empty folder is left to the bridge; an unknown access is read-only', () => {
  const args = plain(Live.bridgeArgs('b', { cwd: '  ', access: 'everything' }, '/h'));
  assert.equal(args.indexOf('--cwd'), -1);
  assert.deepEqual(args.slice(-2), ['--access', 'read-only']);
});

test('localBridgeArgs: defaults are the bridge\'s own; options in order, ~ expanded', () => {
  assert.deepEqual(plain(Live.localBridgeArgs('/c/live/local-live.py', {}, '/home/u')),
    ['python3', '/c/live/local-live.py', '--agent', 'codex', '--session', 'last', '--access', 'read-only']);
  assert.deepEqual(plain(Live.localBridgeArgs('/t/bin/local-live', {
    agent: 'claude', command: ' claude --model x ', session: ' abc ', cwd: '~/code', access: 'workspace-write',
    whisperModel: '~/m/ggml.bin', voice: '/v/pt.onnx', lang: ' en '
  }, '/home/u')), ['/t/bin/local-live', '--agent', 'claude', '--command', 'claude --model x', '--session', 'abc',
    '--cwd', '/home/u/code', '--access', 'workspace-write', '--whisper-model', '/home/u/m/ggml.bin',
    '--voice', '/v/pt.onnx', '--lang', 'en']);
});

test('localBridgeArgs: an unknown agent is codex, an unknown access read-only', () => {
  const args = plain(Live.localBridgeArgs('b', { agent: 'gpt', access: 'all', session: '  ' }, '/h'));
  assert.deepEqual(args, ['b', '--agent', 'codex', '--session', 'last', '--access', 'read-only']);
});

test('errorText: the local bridge says how to install itself, once', () => {
  const state = { error: 'Voice needs packages. Install whisper-cpp (make local-live-deps).', packages: ['whisper-cpp'] };
  assert.equal(Live.errorText(state), state.error);
});

test('parseLine: an object, else null', () => {
  assert.deepEqual(plain(Live.parseLine('{"status": "live"}')), { status: 'live' });
  assert.equal(Live.parseLine('not json'), null);
  assert.equal(Live.parseLine('[1]'), null);
  assert.equal(Live.parseLine('3'), null);
});

test('reduce: the status goes through starting, connecting, live, closed', () => {
  let s = Live.starting(Live.initialState());
  assert.equal(s.status, 'starting');
  assert.equal(Live.running(s), true);
  s = Live.reduce(s, { status: 'live' });
  assert.equal(Live.statusText(s), 'Listening');
  s = Live.reduce(s, { working: true, level: -30 });
  assert.equal(Live.statusText(s), 'Speaking');
  s = Live.reduce(s, { level: -100 });
  assert.equal(Live.statusText(s), 'Working');
  s = Live.reduce(s, { status: 'closed', reason: 'requested' });
  assert.equal(Live.running(s), false);
  assert.equal(s.working, false);
  assert.equal(s.level, -100);
  assert.equal(Live.statusText(s), 'Ended');
});

test('reduce: muted says so while listening', () => {
  const s = run([{ status: 'live' }, { muted: true }]);
  assert.equal(Live.statusText(s), 'Muted');
});

test('reduce: an error, with the packages to install', () => {
  const s = run([{ status: 'starting' }, { error: 'Codex Live needs packages that aren\'t installed.', packages: ['gst-plugins-bad'] }, { status: 'closed' }]);
  assert.equal(Live.statusText(s), 'Stopped');
  assert.match(Live.errorText(s), /Install gst-plugins-bad \(make live-deps\)\.$/);
  assert.equal(Live.errorText(run([{ error: 'No login.' }])), 'No login.');
});

test('starting: a new session keeps the transcript and drops the error', () => {
  const before = run([{ transcript: { role: 'user', text: 'oi', done: true } }, { error: 'x' }, { status: 'closed' }]);
  const s = Live.starting(before);
  assert.equal(s.error, '');
  assert.equal(s.transcript.length, 1);
  assert.equal(s.status, 'starting');
});

test('starting: a line the last session left open is closed, not continued', () => {
  const before = run([{ transcript: { role: 'assistant', delta: 'Ótimo!' } }, { status: 'closed' }]);
  const s = Live.reduce(Live.starting(before), { transcript: { role: 'assistant', delta: 'Oi' } });
  assert.deepEqual(plain(s.transcript), [
    { role: 'assistant', text: 'Ótimo!', done: true },
    { role: 'assistant', text: 'Oi', done: false },
  ]);
});

test('transcript: deltas build a line, done sets its text', () => {
  const s = run([
    { transcript: { role: 'assistant', delta: ' Oi' } },
    { transcript: { role: 'assistant', delta: '! Tudo' } },
    { transcript: { role: 'assistant', delta: ' bem?' } },
  ]);
  assert.deepEqual(plain(s.transcript), [{ role: 'assistant', text: 'Oi! Tudo bem?', done: false }]);
  const done = Live.reduce(s, { transcript: { role: 'assistant', text: ' Oi! Tudo bem? ', done: true } });
  assert.deepEqual(plain(done.transcript), [{ role: 'assistant', text: 'Oi! Tudo bem?', done: true }]);
});

test('transcript: both sides open at once (it answers before what you said is done)', () => {
  const s = run([
    { transcript: { role: 'user', delta: 'Rodei os' } },
    { transcript: { role: 'user', delta: ' testes' } },
    { transcript: { role: 'assistant', delta: 'Ót' } },
    { transcript: { role: 'user', text: 'Rodei os testes.', done: true } },
    { transcript: { role: 'assistant', delta: 'imo!' } },
  ]);
  assert.deepEqual(plain(s.transcript), [
    { role: 'user', text: 'Rodei os testes.', done: true },
    { role: 'assistant', text: 'Ótimo!', done: false },
  ]);
});

test('transcript: a done line after a done line starts a new one; an empty done adds nothing', () => {
  const s = run([
    { transcript: { role: 'user', text: 'um', done: true } },
    { transcript: { role: 'user', text: 'dois', done: true } },
    { transcript: { role: 'user', text: '  ', done: true } },
    { transcript: { role: 'user', delta: '' } },
  ]);
  assert.deepEqual(plain(s.transcript).map(l => l.text), ['um', 'dois']);
});

test('transcript: only the last lines are kept', () => {
  const events = [];
  for (let i = 0; i < Live.MAX_LINES + 5; i++) events.push({ transcript: { role: i % 2 ? 'user' : 'assistant', text: 'l' + i, done: true } });
  const s = run(events);
  assert.equal(s.transcript.length, Live.MAX_LINES);
  assert.equal(s.transcript[s.transcript.length - 1].text, 'l' + (Live.MAX_LINES + 4));
});

test('speaking: only live, above the threshold', () => {
  assert.equal(Live.speaking(run([{ status: 'live' }, { level: -40 }])), true);
  assert.equal(Live.speaking(run([{ status: 'live' }, { level: -70 }])), false);
  assert.equal(Live.speaking(run([{ status: 'connecting' }, { level: -40 }])), false);
});

test('micLevel: 0 at -60 dB and under, 1 at -20 dB; nothing muted or not live', () => {
  assert.equal(Live.micLevel(run([{ status: 'live' }, { mic: -70 }])), 0);
  assert.equal(Live.micLevel(run([{ status: 'live' }, { mic: -40 }])), 0.5);
  assert.equal(Live.micLevel(run([{ status: 'live' }, { mic: -5 }])), 1);
  assert.equal(Live.micLevel(run([{ status: 'live' }, { mic: -30 }, { muted: true }])), 0);
  assert.equal(Live.micLevel(run([{ status: 'connecting' }, { mic: -30 }])), 0);
  assert.equal(run([{ status: 'live' }, { mic: -30 }, { status: 'closed' }]).mic, -100);
});

test('wanted: open, and not stopped since it opened', () => {
  assert.equal(Live.wanted(true, false), true);
  assert.equal(Live.wanted(true, true), false);
  assert.equal(Live.wanted(false, false), false);
});

test('parseCommand: the words; the rest is usage', () => {
  for (const w of ['start', 'stop', 'toggle', 'new', 'mute', 'unmute'])
    assert.deepEqual(plain(Live.parseCommand(' ' + w + ' ')), { action: w, text: '' });
  assert.equal(Live.parseCommand('avatar ~/me.jpg').action, 'usage');
  assert.equal(Live.parseCommand('start now').action, 'usage');
  assert.equal(Live.parseCommand('').action, 'usage');
});

test('localPath: file and qs urls to a path; anything else is no path', () => {
  assert.equal(Live.localPath('file:///p/core/live/codex-live.py', '/c'), '/p/core/live/codex-live.py');
  assert.equal(Live.localPath('qs:@/qs/core/live/a%20b.py', '/cfg/'), '/cfg/core/live/a b.py');
  assert.equal(Live.localPath('qs:///plug/core/live/x.py', ''), '/plug/core/live/x.py');
  assert.equal(Live.localPath('qrc:/x.py', '/c'), '');
  assert.equal(Live.localPath('qs:@/qs/x.py', ''), '');
});
