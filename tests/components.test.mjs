import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { load, plain } from './load.mjs';

const C = load('core/Components.js');
const catalog = C.parseCatalog(fs.readFileSync(new URL('../core/components/components.json', import.meta.url), 'utf8'));
const find = (id) => catalog.find((c) => c.id === id);

test('parseCatalog: the shipped registry, and nothing malformed', () => {
  assert.ok(find('localai') && find('voxtype') && find('voice-pocket-pt') && find('voice-piper-pt-br') && find('codex-live-audio'));
  assert.ok(!catalog.some((c) => /whisper|chatterbox|^piper$/.test(c.id)));  // LocalAI and Voxtype replaced them
  assert.deepEqual(plain(C.parseCatalog('not json')), []);
  assert.deepEqual(plain(C.parseCatalog('{"components": [{"id": "a", "title": "A", "kind": "files"}, {"id": "b"}, {"id": "c", "title": "C", "kind": "magic"}, null]}')).map((c) => c.id), ['a']);
});

test('parseStatus: an object, else nothing', () => {
  assert.deepEqual(plain(C.parseStatus('{"a": {"installed": true}}')), { a: { installed: true } });
  assert.deepEqual(plain(C.parseStatus('[1]')), {});
  assert.deepEqual(plain(C.parseStatus('')), {});
});

test('installerCommand: a .py runs under python3, a fake as it is', () => {
  assert.deepEqual(plain(C.installerCommand('/x/install.py', ['install', 'a'])), ['python3', '/x/install.py', 'install', 'a']);
  assert.deepEqual(plain(C.installerCommand('/t/fake', ['status'])), ['/t/fake', 'status']);
});

test('formatBytes: sizes for people', () => {
  assert.equal(C.formatBytes(0), '0 B');
  assert.equal(C.formatBytes(950), '950 B');
  assert.equal(C.formatBytes(63_000_000), '63 MB');
  assert.equal(C.formatBytes(488_000_000), '488 MB');
  assert.equal(C.formatBytes(1_533_000_000), '1.5 GB');
  assert.equal(C.formatBytes(6_000_000_000), '6 GB');
  assert.equal(C.formatBytes(148_000_000), '148 MB');
  assert.equal(C.formatBytes(-4), '0 B');
});

const small = find('localai');  // a download: 197 MB
const pulse = find('libpulse');

test('view: not read yet', () => {
  const v = plain(C.view(small, undefined));
  assert.equal(v.state, 'unknown');
  assert.deepEqual(v.actions, []);
});

test('view: not installed offers Install and the size', () => {
  const v = plain(C.view(small, { installed: false, active: false, phase: '' }));
  assert.equal(v.state, 'missing');
  assert.equal(v.text, 'Not installed');
  assert.equal(v.size, 'about 197 MB');
  assert.deepEqual(v.actions.map((a) => a.id), ['install']);
});

test('view: a download shows percent and bytes, and can be cancelled', () => {
  const v = plain(C.view(small, { active: true, phase: 'downloading', percent: 42, done: 83_000_000, total: 196_718_754 }));
  assert.equal(v.state, 'working');
  assert.equal(v.percent, 42);
  assert.equal(v.text, 'Downloading 42 % (83 MB of 197 MB)');
  assert.deepEqual(v.actions.map((a) => a.id), ['cancel']);
});

test('view: work without a percent has a bar that only moves', () => {
  const v = plain(C.view(find('voice-piper-pt-br'), { active: true, phase: 'installing', percent: null, detail: 'Installing piper-tts' }));
  assert.equal(v.percent, -1);
  assert.equal(v.text, 'Installing: Installing piper-tts');
  assert.equal(plain(C.view(small, { active: true, phase: 'queued' })).text, 'Starting…');
  assert.equal(plain(C.view(small, { active: true, phase: 'downloading' })).text, 'Downloading…');
  assert.equal(plain(C.view(find('voice-piper-pt-br'), { active: true, phase: 'installing', percent: 30 })).text, 'Installing 30 %…');
});

test('view: installed says what it uses and offers Remove', () => {
  const v = plain(C.view(small, { installed: true, disk: 196_718_754, removable: true }));
  assert.equal(v.state, 'installed');
  assert.equal(v.text, 'Installed (197 MB)');
  assert.deepEqual(v.actions.map((a) => a.id), ['remove']);
});

test('view: a system package is installed by the system, never removed here', () => {
  const v = plain(C.view(pulse, { installed: true, removable: false, disk: 0 }));
  assert.equal(v.text, 'Installed by the system');
  assert.deepEqual(v.actions, []);
  assert.deepEqual(plain(C.view(pulse, { installed: false, phase: '' })).actions.map((a) => a.id), ['install']);
});

test('view: a failure says why, offers a retry, and cleaning up when something is left', () => {
  const st = { installed: false, active: false, phase: 'failed', error: 'The server answered 404 Not Found.', disk: 0, removable: true };
  const v = plain(C.view(small, st));
  assert.equal(v.state, 'failed');
  assert.equal(v.text, 'Failed: The server answered 404 Not Found.');
  assert.deepEqual(v.actions.map((a) => a.id), ['install']);
  assert.deepEqual(plain(C.view(small, { ...st, disk: 5000 })).actions.map((a) => a.id), ['install', 'remove']);
  assert.deepEqual(plain(C.view(pulse, { ...st, disk: 0, removable: false })).actions.map((a) => a.id), ['install']);
});

test('installedIds and anyActive', () => {
  const states = { b: { installed: true }, a: { installed: true }, c: { installed: false, active: true } };
  assert.deepEqual(plain(C.installedIds(states)), ['a', 'b']);
  assert.equal(C.anyActive(states), true);
  assert.equal(C.anyActive({ a: { installed: true } }), false);
  assert.equal(C.anyActive({}), false);
  assert.deepEqual(plain(C.installedIds(undefined)), []);
});

test('usedBy and helpText name the modules', () => {
  assert.equal(C.usedBy(small, { 'local-live': 'Voice' }), 'Voice');
  assert.equal(C.usedBy(pulse, { 'local-live': 'Voice', 'codex-live': 'Codex Live' }), 'Voice and Codex Live');
  assert.equal(C.usedBy(pulse, {}), 'local-live and codex-live');
  assert.match(C.helpText(small, { 'local-live': 'Voice' }), /^The engine the voices run on.* For Voice\.$/);
});

test('overviewFields: a note, then every component under its group', () => {
  const fields = plain(C.overviewFields(catalog, { localai: { installed: true, disk: 197_000_000 }, libpulse: { installed: true, disk: 0 } }));
  assert.equal(fields[0].type, 'note');
  assert.match(fields[0].help, /They use 197 MB/);
  assert.equal(fields.length, catalog.length + 1);
  assert.ok(fields.slice(1).every((f) => f.type === 'component' && f.page === 'components'));
  // the tools, then a group per language, then Codex Live's
  const groups = [...new Set(fields.slice(1).map((f) => f.section))];
  assert.equal(groups[0], 'Voice');
  assert.equal(groups[groups.length - 1], 'Codex Live');
  assert.equal(groups.length, 2 + new Set(catalog.filter((c) => c.engine).map((c) => c.language)).size);
  for (const g of groups.slice(1, -1)) assert.match(g, /^Voices: /);
  assert.doesNotMatch(plain(C.overviewFields(catalog, {}))[0].help, /They use/);
});

test('decorate: component fields get their title and help', () => {
  const out = plain(C.decorate([C.field('p', 'S', 'localai')], catalog, [], { 'local-live': 'Voice' }));
  assert.equal(out[0].label, 'LocalAI');
  assert.match(out[0].help, /For Voice\.$/);
  // an unknown component stays as it is
  assert.deepEqual(plain(C.decorate([C.field('p', 'S', 'nope')], catalog, [], {}))[0].label, undefined);
});

const tts = { key: 'voiceModel', type: 'choice', help: 'Pick.', options: [{ value: '', label: 'First' }, { value: 'pocket-pt', label: 'Pocket TTS', needs: ['voice-pocket-pt'] }] };
const byLanguage = { key: 'voiceModel', type: 'choice', help: 'Pick.', options: [
  { value: '', label: 'First' },
  { value: 'piper', label: 'Piper', needs: [{ engine: 'piper', language: '@lang' }] },
  { value: 'pocket', label: 'Pocket TTS', needs: [{ engine: 'pocket', language: '@lang' }] }] };

test('decorate: a choice that needs a component is disabled until it is installed', () => {
  const locked = plain(C.decorate([tts], catalog, ['localai'], {}))[0];
  assert.equal(locked.options[0].disabled, undefined);
  assert.equal(locked.options[1].disabled, true);
  assert.equal(locked.help, 'Pick. Needs Voice: Pocket TTS, Portuguese (Brazil): install it in Settings › Components.');
  const open = plain(C.decorate([tts], catalog, ['localai', 'voice-pocket-pt'], {}))[0];
  assert.equal(open.options[1].disabled, undefined);
  assert.equal(open.help, 'Pick.');
  // the state not read yet: nothing is locked
  assert.equal(plain(C.decorate([tts], catalog, null, {}))[0].options[1].disabled, undefined);
});

test('decorate: leaves the schema it was given alone', () => {
  const before = JSON.stringify(tts);
  C.decorate([tts], catalog, [], {});
  assert.equal(JSON.stringify(tts), before);
});

test('missingHint points to the settings, not to make', () => {
  assert.equal(C.missingHint(['localai'], catalog), 'Install LocalAI in Settings › Components.');
  assert.equal(C.missingHint(['voxtype', 'libpulse', 'localai'], catalog), 'Install Voxtype, Audio tools and LocalAI in Settings › Components.');
  assert.equal(C.missingHint(['mystery'], catalog), 'Install mystery in Settings › Components.');
  assert.equal(C.missingHint([], catalog), '');
});

test('the registry names modules that exist, and no two components share a file', () => {
  const modules = fs.readdirSync(new URL('../core/modules/', import.meta.url)).filter((f) => f.endsWith('.json')).map((f) => f.replace('.json', ''));
  for (const c of catalog) for (const m of c.needed_by) assert.ok(modules.includes(m), `${c.id} needed by ${m}`);
  const files = catalog.flatMap((c) => (c.files || []).map((f) => `${c.dir}/${f.name}`));
  assert.equal(files.length, new Set(files).size);
});

const voices = plain(catalog).filter((c) => c.engine);
const languages = [...new Set(voices.map((c) => c.language))];

test('the voices: every language has a Piper and a Pocket TTS voice, consistent with its gallery entry and backend', () => {
  assert.ok(languages.length >= 7 && languages.includes('pt') && languages.includes('en'));
  const ids = new Set();
  for (const code of languages) {
    assert.deepEqual(voices.filter((c) => c.language === code).map((c) => c.engine), ['piper', 'pocket'], code);
  }
  for (const c of voices) {
    assert.equal(c.kind, 'localai');
    assert.ok(!ids.has(c.id), 'duplicate ' + c.id); ids.add(c.id);
    assert.equal(c.requires, 'localai');
    // the gallery entry, its backend and the folder the backend lands in agree with the engine
    const backend = c.engine === 'piper' ? 'cpu-piper' : 'cpu-pocket-tts';
    assert.equal(c.backend, 'localai@' + backend);
    assert.equal(c.backend_dir, backend);
    assert.equal(c.model.overrides.backend, backend);
    assert.match(c.model.gallery, /^localai@/);
    assert.ok(c.paths.includes('backends/' + backend), c.id);
    assert.ok(c.size > c.shared_size && c.shared_size > 0, c.id);
    assert.ok(c.warmup, c.id);
    assert.ok(c.title.includes(c.engine === 'piper' ? 'Piper' : 'Pocket TTS'), c.title);
    assert.equal(c.group, find(voices.find((v) => v.language === c.language).id).group);
  }
  // a model name is the one thing LocalAI knows it by: no two voices share one
  const names = voices.map((c) => c.model.name);
  assert.equal(new Set(names).size, names.length);
  // Piper: the gallery's voice is the model, named by its language; its files are its own
  for (const c of voices.filter((v) => v.engine === 'piper')) {
    assert.equal(c.model.gallery, 'localai@' + c.model.name);
    assert.ok(c.paths.filter((p) => p.startsWith('models/')).length >= 2, c.id);
    for (const p of c.paths.filter((x) => x.startsWith('models/'))) assert.ok(!voices.some((o) => o !== c && o.paths.includes(p)), p);
  }
  // Pocket TTS: one model of the shared gallery entry per language, the language and alba in its options, weights in the shared cache
  for (const c of voices.filter((v) => v.engine === 'pocket')) {
    assert.equal(c.model.gallery, 'localai@pocket-tts');
    assert.equal(c.model.name, 'pocket-' + c.language);
    assert.ok(c.model.overrides.options.includes('language:' + c.hf_language), c.id);
    assert.ok(c.model.overrides.options.includes('default_voice:alba'), c.id);
    assert.match(c.hf_language, /_24l$/);
    assert.ok(c.paths.includes('hf'));
    assert.ok(languages.includes(c.language));
  }
  assert.equal(new Set(voices.filter((v) => v.engine === 'pocket').map((c) => c.hf_language)).size, languages.length);
});

test('the engine\'s backend is shared: the others of its language engine are what the size and the first install count', () => {
  for (const engine of ['piper', 'pocket']) {
    const list = voices.filter((c) => c.engine === engine);
    for (const c of list) {
      const others = list.filter((o) => o !== c).map((o) => o.id);
      assert.equal(C.sharesBackend(c, catalog, []), false);
      assert.equal(C.sharesBackend(c, catalog, [c.id]), false);       // itself doesn't count
      assert.equal(C.sharesBackend(c, catalog, ['localai', 'voxtype']), false);
      for (const o of others) assert.equal(C.sharesBackend(c, catalog, [o]), true, `${c.id} with ${o}`);
      // the other engine's voices share nothing with it
      for (const o of voices.filter((v) => v.engine !== engine)) assert.equal(C.sharesBackend(c, catalog, [o.id]), false);
      assert.equal(C.sizeToInstall(c, true), c.size - c.shared_size);
      assert.equal(C.sizeToInstall(c, false), c.size);
      assert.match(C.view(c, { installed: false, active: false, phase: '' }, true).size, /^about /);
      assert.ok(C.view(c, { installed: false, active: false, phase: '' }, true).size !== C.view(c, { installed: false, active: false, phase: '' }, false).size);
    }
  }
  assert.equal(C.sharesBackend(find('localai'), catalog, voices.map((v) => v.id)), false);  // no backend of its own
});

test('a "components" field is a row per voice that matches, shown with the language the field says', () => {
  const field = { type: 'components', section: 'TTS', page: 'p', match: { engine: ['piper', 'pocket'] }, by: { key: 'lang', field: 'language' } };
  const rows = plain(C.decorate([field], catalog, [], {}));
  assert.deepEqual(rows.map((r) => r.component), voices.map((c) => c.id));
  for (const r of rows) {
    const c = find(r.component);
    assert.deepEqual([r.type, r.section, r.page, r.label], ['component', 'TTS', 'p', c.title]);
    assert.deepEqual(r.when, { key: 'lang', is: c.language });
  }
  // narrower matches, and no `by`: always shown
  const piper = plain(C.decorate([{ type: 'components', match: { engine: 'piper' } }], catalog, [], {}));
  assert.equal(piper.length, languages.length);
  assert.ok(piper.every((r) => r.when === undefined));
  assert.deepEqual(plain(C.decorate([{ type: 'components', match: { engine: 'none' } }], catalog, [], {})), []);
});

test('gate: a choice that needs the voice of a chosen language waits on that language\'s component alone', () => {
  const lang = (code) => (key) => (key === 'lang' ? code : undefined);
  for (const code of languages) {
    const pocket = voices.find((c) => c.language === code && c.engine === 'pocket').id;
    const piper = voices.find((c) => c.language === code && c.engine === 'piper').id;
    const gated = (installed, l = code) => plain(C.gate([byLanguage], catalog, installed, lang(l)))[0];
    // nothing installed: both engines wait, "first installed" never does
    assert.deepEqual(gated([]).options.map((o) => !!o.disabled), [false, true, true], code);
    // the language's own voice opens its engine, no other language's does
    assert.deepEqual(gated([piper]).options.map((o) => !!o.disabled), [false, false, true], code);
    assert.deepEqual(gated([pocket]).options.map((o) => !!o.disabled), [false, true, false], code);
    for (const other of languages.filter((l) => l !== code))
      assert.deepEqual(gated(voices.filter((c) => c.language === other).map((c) => c.id)).options.map((o) => !!o.disabled), [false, true, true], `${code} with ${other}`);
    // the help names what is missing, in that language
    assert.match(gated([]).help, new RegExp('Needs ' + find(piper).title.replace(/[()]/g, '\\$&') + ' and ' + find(pocket).title.replace(/[()]/g, '\\$&')));
    assert.doesNotMatch(gated([piper, pocket]).help, /Needs/);
  }
  // an unknown language matches nothing: both wait, nothing to install is named
  const unknown = plain(C.gate([byLanguage], catalog, ['localai'], lang('xx')))[0];
  assert.deepEqual(unknown.options.map((o) => !!o.disabled), [false, true, true]);
  assert.doesNotMatch(unknown.help, /Needs/);
  // the state not read yet: nothing is locked; decorate leaves these needs to gate
  assert.deepEqual(plain(C.gate([byLanguage], catalog, null, lang('pt')))[0].options.map((o) => !!o.disabled), [false, false, false]);
  assert.deepEqual(plain(C.decorate([byLanguage], catalog, [], {}))[0].options.map((o) => !!o.disabled), [false, false, false]);
  // and gate leaves the plain ids to decorate, and fields without options alone
  assert.deepEqual(plain(C.gate([tts, { key: 'x' }], catalog, [], lang('pt'))), plain([tts, { key: 'x' }]));
});
