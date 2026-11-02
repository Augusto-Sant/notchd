import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { load, plain } from './load.mjs';

const Settings = load('core/settings/Settings.js');

const core = [
  { id: 'general', label: 'General', icon: 1 },
  { id: 'modules', label: 'Modules', icon: 2 },
];

test('mergePages lets a host relabel a core page and append its own', () => {
  const pages = plain(Settings.mergePages(core, [{ id: 'modules', label: 'Plugins' }, { id: 'extra', label: 'Extra' }]));
  assert.deepEqual(pages.map((p) => [p.id, p.label, p.icon]),
    [['general', 'General', 1], ['modules', 'Plugins', 2], ['extra', 'Extra', undefined]]);
  assert.equal(core[1].label, 'Modules', 'core pages are not mutated');
});

test('visiblePages hides pages without fields; fields default to general', () => {
  const schema = [{ key: 'a' }, { key: 'b', page: 'extra' }];
  const pages = Settings.mergePages(core, [{ id: 'extra' }]);
  assert.deepEqual(Settings.visiblePages(pages, schema).map((p) => p.id), ['general', 'extra']);
});

test('resolvePage falls back to the first visible page', () => {
  assert.equal(Settings.resolvePage(core, 'modules'), 'modules');
  assert.equal(Settings.resolvePage(core, 'gone'), 'general');
  assert.equal(Settings.resolvePage([], 'gone'), '');
});

test('cyclePage wraps both ways', () => {
  assert.equal(Settings.cyclePage(core, 'general', 1), 'modules');
  assert.equal(Settings.cyclePage(core, 'modules', 1), 'general');
  assert.equal(Settings.cyclePage(core, 'general', -1), 'modules');
  assert.equal(Settings.cyclePage([], 'general', 1), '');
});

test('sections groups a page by section in first-seen order', () => {
  const schema = [
    { key: 'a', section: 'Hover' },
    { key: 'b', section: 'Layout' },
    { key: 'c', section: 'Hover' },
    { key: 'd', page: 'modules' },
    { key: 'e' },
  ];
  const out = plain(Settings.sections(schema, 'general'));
  assert.deepEqual(out.map((s) => [s.name, s.fields.map((f) => f.key)]),
    [['Hover', ['a', 'c']], ['Layout', ['b']], ['', ['e']]]);
});

test('toList accepts arrays and array-likes, rejects the rest', () => {
  assert.deepEqual(plain(Settings.toList(['a', 1])), ['a', '1']);
  assert.deepEqual(plain(Settings.toList({ length: 1, 0: 'x' })), ['x']);
  assert.deepEqual(plain(Settings.toList('abc')), []);
  assert.deepEqual(plain(Settings.toList(undefined)), []);
});

test('toggleId and moveId return new lists', () => {
  const ids = ['a', 'b', 'c'];
  assert.deepEqual(plain(Settings.toggleId(ids, 'b')), ['a', 'c']);
  assert.deepEqual(plain(Settings.toggleId(ids, 'd')), ['a', 'b', 'c', 'd']);
  assert.deepEqual(plain(Settings.moveId(ids, 'c', -1)), ['a', 'c', 'b']);
  assert.deepEqual(plain(Settings.moveId(ids, 'a', -1)), ['a', 'b', 'c']);
  assert.deepEqual(plain(Settings.moveId(ids, 'x', 1)), ['a', 'b', 'c']);
  assert.deepEqual(ids, ['a', 'b', 'c']);
});

test('allIds lists enabled ids first, then the other available ones', () => {
  assert.deepEqual(plain(Settings.allIds(['media', 'ghost'], ['activities', 'media', 'osd'])),
    ['media', 'ghost', 'activities', 'osd']);
});

test('workspaceChoices offers 1-10, known and picked ones, numbers first', () => {
  assert.deepEqual(plain(Settings.workspaceChoices(['2', 'code', '12'], ['notes', 3])),
    [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 'code', 'notes']);
  assert.deepEqual(plain(Settings.workspaceChoices(null, null)), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
});

test('toggleWorkspace keeps numbers as numbers and clears to undefined', () => {
  assert.deepEqual(plain(Settings.toggleWorkspace([3, 1], '2')), [1, 2, 3]);
  assert.deepEqual(plain(Settings.toggleWorkspace([1, 2], 2)), [1]);
  assert.deepEqual(plain(Settings.toggleWorkspace(undefined, 'code')), ['code']);
  assert.equal(Settings.toggleWorkspace([1], 1), undefined);
});

test('snapToStep snaps to the step from min, clamps, and drops float noise', () => {
  assert.equal(Settings.snapToStep(0.351, 0, 1, 0.01), 0.35);
  assert.equal(Settings.snapToStep(0.1 + 0.2, 0, 1, 0.01), 0.3);
  assert.equal(Settings.snapToStep(1.4, 0, 1, 0.01), 1);
  assert.equal(Settings.snapToStep(-3, 0, 1, 0.01), 0);
  assert.equal(Settings.snapToStep(124, 0, 1000, 10), 120);
  assert.equal(Settings.snapToStep(7, 5, 100, 10), 5);
  assert.equal(Settings.snapToStep(7, 0, 100, 0), 7);
});

test('storedValue removes the key when an edit picks the default', () => {
  const field = { key: 'position.edge', default: 'top' };
  assert.equal(Settings.storedValue(field, 'top'), undefined);
  assert.equal(Settings.storedValue(field, 'left'), 'left');
  assert.equal(Settings.storedValue({ default: 0.5 }, 0.5), undefined);
  assert.equal(Settings.storedValue({ default: false }, false), undefined);
  assert.equal(Settings.storedValue({}, 'x'), 'x');
  assert.equal(Settings.storedValue({ default: 'top' }, undefined), undefined);
});

test('moduleFields puts a module\'s fields under its id, on its own page', () => {
  const fields = plain(Settings.moduleFields([
    { moduleId: 'resources', title: 'Resources', settings: [
      { key: 'compact', type: 'choice', default: 'always' },
      { key: 'disk', type: 'string', page: 'extra', section: 'Disk' },
      { key: 'color', type: 'color', page: 'appearance' },
      { type: 'note', label: 'Linux only' }
    ] },
    { moduleId: 'media', title: 'Now playing', settings: [] },
    { moduleId: 'bare', settings: [{ key: 'x', type: 'boolean' }] }
  ]));
  assert.deepEqual(fields.map((f) => [f.key, f.page, f.section]), [
    ['resources.compact', 'modules/resources', undefined],
    ['resources.disk', 'extra', 'Disk'],
    ['resources.color', 'appearance', 'Resources'],
    [undefined, 'modules/resources', undefined],
    ['bare.x', 'modules/bare', undefined]
  ]);
  assert.equal(fields[0].default, 'always');
  assert.deepEqual(plain(Settings.moduleFields(undefined)), []);
});

test('modulePages gives each loaded module a page under Modules', () => {
  assert.deepEqual(plain(Settings.modulePages([{ moduleId: 'media', title: 'Now playing' }, { moduleId: 'bare' }, null])),
    [{ id: 'modules/media', label: 'Now playing' }, { id: 'modules/bare', label: 'bare' }]);
});

const withSub = [
  { id: 'general' }, { id: 'modules' }, { id: 'modules/media' }, { id: 'appearance' }
];

test('a module page is a sub-page: off the sidebar, under its parent', () => {
  assert.deepEqual(Settings.navPages(withSub).map((p) => p.id), ['general', 'modules', 'appearance']);
  assert.equal(Settings.parentPage('modules/media'), 'modules');
  assert.equal(Settings.parentPage('general'), 'general');
  assert.equal(Settings.parentPage(undefined), '');
});

test('resolvePage falls back from a gone sub-page to its parent', () => {
  assert.equal(Settings.resolvePage(withSub, 'modules/media'), 'modules/media');
  assert.equal(Settings.resolvePage(withSub, 'modules/timer'), 'modules');
  assert.equal(Settings.resolvePage(withSub, 'gone/x'), 'general');
  assert.equal(Settings.resolvePage([{ id: 'a/b' }], 'x'), 'a/b');
});

test('cyclePage steps through the sidebar, from a sub-page\'s parent', () => {
  assert.equal(Settings.cyclePage(withSub, 'modules', 1), 'appearance');
  assert.equal(Settings.cyclePage(withSub, 'modules/media', 1), 'appearance');
  assert.equal(Settings.cyclePage(withSub, 'modules/media', -1), 'general');
});

test('moduleTitle uses the loaded module\'s title, else its id', () => {
  assert.equal(Settings.moduleTitle('media', { title: 'Now playing' }), 'Now playing');
  assert.equal(Settings.moduleTitle('timer', null), 'Timer');
});

test('moduleStatus says where a module shows, or whether a standalone one is on', () => {
  const one = [{ id: 'main', moduleIds: ['media'] }];
  const two = [{ id: 'main', moduleIds: ['media', 'osd'] }, { id: 'jobs', moduleIds: ['activities', 'media'] }];
  assert.equal(Settings.moduleStatus('media', one), 'On');
  assert.equal(Settings.moduleStatus('timer', one), 'Off');
  assert.equal(Settings.moduleStatus('media', two), 'In main, jobs');
  assert.equal(Settings.moduleStatus('activities', two), 'In jobs');
  assert.equal(Settings.moduleStatus('osd', two, true, true), 'On', 'a notch listing it does not count');
  assert.equal(Settings.moduleStatus('osd', one, true, false), 'Off');
  assert.equal(Settings.moduleStatus('timer', undefined), 'Off');
});

test('catalogPart splits the Modules page into notch modules, agents and standalone ones', () => {
  const catalog = [{ moduleId: 'media' }, { moduleId: 'osd', standalone: true }, { moduleId: 'claude', agent: true },
                   { moduleId: 'timer', standalone: false }];
  assert.deepEqual(plain(Settings.catalogPart(catalog, 'notch').map((m) => m.moduleId)), ['media', 'timer']);
  assert.deepEqual(plain(Settings.catalogPart(catalog).map((m) => m.moduleId)), ['media', 'timer']);
  assert.deepEqual(plain(Settings.catalogPart(catalog, 'standalone').map((m) => m.moduleId)), ['osd']);
  assert.deepEqual(plain(Settings.catalogPart(catalog, 'agent').map((m) => m.moduleId)), ['claude']);
  assert.deepEqual(plain(Settings.catalogPart(undefined, 'agent')), []);
});

test('an agent\'s file brings the fields every agent has, its command the default', () => {
  const info = plain(Settings.parseModuleInfo(JSON.stringify({
    title: 'Aider', agent: { command: ' aider ', help: { idleMinutes: 'It is light.' } },
    settings: [{ key: 'font', type: 'string', label: 'Its font', default: 'Mono' }, { key: 'extra', type: 'boolean', default: true }]
  })));
  assert.equal(info.agent, true);
  assert.deepEqual(info.settings.map((f) => f.key), ['directory', 'command', 'idleMinutes', 'width', 'height', 'font', 'extra']);
  assert.equal(Settings.settingDefault(info.settings, 'command'), 'aider');
  // Empty: notchd's own folder.
  assert.equal(Settings.settingDefault(info.settings, 'directory'), '');
  assert.match(info.settings.find((f) => f.key === 'idleMinutes').help, /Aider stops.*It is light\.$/);
  // Its own field wins over the inherited one.
  assert.equal(info.settings.find((f) => f.key === 'font').label, 'Its font');
});

test('an agent with usage gets its switch and warning, under Usage', () => {
  const info = plain(Settings.parseModuleInfo(JSON.stringify({ title: 'Claude', agent: { command: 'claude', usage: 'claude' } })));
  assert.equal(info.usage, 'claude');
  const usage = info.settings.filter((f) => f.section === 'Usage');
  assert.deepEqual(usage.map((f) => [f.key, f.default]), [['usage', true], ['usageWarn', 80]]);
  const none = plain(Settings.parseModuleInfo(JSON.stringify({ title: 'Aider', agent: { command: 'aider' } })));
  assert.equal(none.usage, undefined);
  assert.ok(!none.settings.some((f) => f.key === 'usage'));
});

test('an agent without a command is an error, not an agent', () => {
  for (const agent of [{}, { command: '' }, { command: 3 }, 'claude', []]) {
    const info = plain(Settings.parseModuleInfo(JSON.stringify({ title: 'X', agent })));
    assert.equal(info.agent, undefined, JSON.stringify(agent));
    assert.equal(info.error, 'agent: needs a command');
  }
});

test('a live module\'s file brings the conversation\'s fields', () => {
  const info = plain(Settings.parseModuleInfo(JSON.stringify({ title: 'Codex Live', live: { command: ' codex ' } })));
  assert.equal(info.live, true);
  assert.equal(info.agent, undefined);
  assert.deepEqual(info.settings.filter((f) => f.key).map((f) => f.key), ['command', 'directory', 'voice', 'access']);
  assert.equal(Settings.settingDefault(info.settings, 'command'), 'codex');
  assert.equal(Settings.settingDefault(info.settings, 'access'), 'read-only');
  // Codex's own voice by default, the others as chips.
  const voice = info.settings.find((f) => f.key === 'voice');
  assert.equal(voice.default, '');
  assert.equal(voice.display, 'chips');
  assert.equal(voice.options[0].value, '');
  assert.ok(voice.options.some((o) => o.value === 'maple' && o.label === 'Maple'));
  // Listed with the agents.
  assert.equal(Settings.moduleKind({ live: true }), 'agent');
  assert.deepEqual(plain(Settings.catalogPart([{ moduleId: 'codex-live', live: true }, { moduleId: 'media' }], 'agent').map((m) => m.moduleId)), ['codex-live']);
});

test('a local live module needs no command: its fields are the agent\'s, the models\'', () => {
  const info = plain(Settings.parseModuleInfo(JSON.stringify({ title: 'Voice', live: { local: true } })));
  assert.equal(info.live, true);
  assert.equal(info.error, undefined);
  assert.deepEqual(info.settings.filter((f) => f.key).map((f) => f.key), ['agent', 'command', 'session', 'directory', 'access', 'language', 'speechModel', 'voiceLanguage', 'voiceModel', 'otherVoiceModel', 'voice', 'localaiUrl']);
  assert.equal(Settings.settingDefault(info.settings, 'agent'), 'codex');
  assert.equal(Settings.settingDefault(info.settings, 'session'), 'last');
  assert.equal(Settings.settingDefault(info.settings, 'language'), '');  // Voxtype's own
  assert.equal(Settings.settingDefault(info.settings, 'command'), '');
  assert.equal(Settings.settingDefault(info.settings, 'voiceModel'), '');
  assert.equal(Settings.settingDefault(info.settings, 'voiceLanguage'), 'pt');
  assert.equal(Settings.settingDefault(info.settings, 'localaiUrl'), '');
});

test('a live module without a command is an error', () => {
  for (const live of [{}, { command: ' ' }, true, []]) {
    const info = plain(Settings.parseModuleInfo(JSON.stringify({ title: 'X', live })));
    assert.equal(info.live, undefined, JSON.stringify(live));
    assert.equal(info.error, 'live: needs a command');
  }
});

test('parseModuleInfo keeps a title and well-formed fields, drops the rest', () => {
  assert.deepEqual(plain(Settings.parseModuleInfo('{ "title": "Timer", "settings": [{ "key": "a", "type": "boolean" }, 3, { "key": "b" }] }')),
    { title: 'Timer', settings: [{ key: 'a', type: 'boolean' }] });
  assert.deepEqual(plain(Settings.parseModuleInfo('{ "settings": {} }')), { title: '', settings: [] });
  assert.equal(Settings.parseModuleInfo('{ "standalone": true }').standalone, true);
  assert.equal(Settings.parseModuleInfo('{ "standalone": "yes" }').standalone, undefined);
  const bad = plain(Settings.parseModuleInfo('{ nope'));
  assert.deepEqual([bad.title, bad.settings], ['', []]);
  assert.ok(bad.error);
  assert.equal(plain(Settings.parseModuleInfo('[]')).error, 'not an object');
});

test('settingDefault finds a field\'s default by key', () => {
  const fields = [{ key: 'focus', default: 25 }, { key: 'notify', default: false }];
  assert.equal(Settings.settingDefault(fields, 'focus'), 25);
  assert.equal(Settings.settingDefault(fields, 'notify'), false);
  assert.equal(Settings.settingDefault(fields, 'gone'), undefined);
  assert.equal(Settings.settingDefault(undefined, 'focus'), undefined);
});

test('notch pages are sub-pages of Notches, by index', () => {
  assert.equal(Settings.notchPageId(2), 'notch/2');
  assert.equal(Settings.notchIndexOfPage('notch/2'), 2);
  assert.equal(Settings.notchIndexOfPage('notch'), -1);
  assert.equal(Settings.notchIndexOfPage('modules/media'), -1);
  assert.equal(Settings.notchIndexOfPage('notch/x'), -1);
  assert.equal(Settings.parentPage(Settings.notchPageId(0)), 'notch');
});

test('notchSummary says where a notch is and what it shows', () => {
  const titles = { media: 'Now playing' };
  assert.equal(Settings.notchSummary({ edge: 'bottom', moduleIds: ['media', 'timer'] }, titles),
    'Bottom edge · Now playing, Timer');
  assert.equal(Settings.notchSummary({ edge: 'top', moduleIds: [] }, titles), 'Top edge · no modules');
  assert.equal(Settings.notchSummary(null, titles), '');
});

// Every module's JSON file parses, and each field the window can save has
// a key, a type and a default within its range.
test('every module JSON file is well-formed', () => {
  const files = ['core/modules', 'hosts/omarchy/modules'].flatMap((dir) =>
    fs.readdirSync(new URL(`../${dir}`, import.meta.url)).filter((f) => f.endsWith('.json')).map((f) => `${dir}/${f}`));
  assert.ok(files.length >= 5, 'files: ' + files);
  for (const file of files) {
    const text = fs.readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');
    const info = plain(Settings.parseModuleInfo(text));
    assert.equal(info.error, undefined, file);
    assert.ok(info.title, `${file}: no title`);
    const keys = Object.keys(JSON.parse(text));
    assert.deepEqual(keys.filter((k) => !['title', 'settings', 'standalone', 'agent', 'live'].includes(k)), [], `${file}: unknown keys`);
    if (info.standalone)
      for (const f of info.settings) assert.ok(!['enabled', 'position'].includes(String(f.key).split('.')[0]), `${file}: ${f.key} is the standalone switch's or position's`);
    const inherited = info.agent ? Settings.agentSettings(JSON.parse(text).agent, info.title).length
      : info.live ? Settings.liveSettings(JSON.parse(text).live, info.title).length : 0;
    assert.ok(info.settings.length >= Math.max(inherited, JSON.parse(text).settings?.length ?? 0), `${file}: a field was dropped`);
    for (const f of info.settings) {
      if (f.type === 'note') continue;
      if (f.type === 'component') { assert.ok(f.component, `${file}: a component row without a component`); continue; }
      if (f.type === 'components') { assert.ok(f.match, `${file}: components without a match`); continue; }
      assert.ok(f.key && f.default !== undefined, `${file}: ${JSON.stringify(f)}`);
      if (f.min !== undefined) assert.ok(f.default >= f.min && f.default <= f.max, `${file}: ${f.key} out of range`);
      if (f.options) assert.ok(f.options.some((o) => o.value === f.default), `${file}: ${f.key} default not an option`);
    }
  }
  const qml = ['core/modules', 'hosts/omarchy/modules'].flatMap((dir) =>
    fs.readdirSync(new URL(`../${dir}`, import.meta.url)).filter((f) => f.endsWith('.qml')).map((f) => `${dir}/${f.replace(/\.qml$/, '.json')}`));
  for (const json of qml) assert.ok(files.includes(json), `${json} is missing`);
});

test('moduleInfoPath finds the JSON file next to a module, file or qs: url', () => {
  assert.equal(Settings.moduleInfoPath('file:///p/core/modules/media.qml', '/c'), '/p/core/modules/media.json');
  assert.equal(Settings.moduleInfoPath('file:///p/my%20mods/a.qml', ''), '/p/my mods/a.json');
  assert.equal(Settings.moduleInfoPath('qs:@/qs/core/modules/timer.qml', '/home/u/.config/qs/'), '/home/u/.config/qs/core/modules/timer.json');
  assert.equal(Settings.moduleInfoPath('qs:@/qs/core/modules/timer.qml', ''), '');
  // The Omarchy shell names a plugin's core modules this way; their files
  // were not found, so their titles and settings were missing there.
  assert.equal(Settings.moduleInfoPath('qs:///home/u/plugins/notchd/core/modules/media.qml', '/usr/share/omarchy/shell'),
    '/home/u/plugins/notchd/core/modules/media.json');
  assert.equal(Settings.moduleInfoPath('https://x/y.qml', '/c'), '');
  assert.equal(Settings.moduleInfoPath('', '/c'), '');
});

test('a standalone module\'s page starts with its switch and position', () => {
  const fields = plain(Settings.moduleFields([
    { moduleId: 'osd', title: 'OSD', standalone: true, settings: [{ key: 'size', type: 'integer', default: 1 }] }
  ], 'edge.qml'));
  assert.deepEqual(fields.map((f) => [f.key, f.page, f.section, f.default]), [
    ['osd.enabled', 'modules/osd', undefined, true],
    ['osd.position.edge', 'modules/osd', 'Position', 'top'],
    ['osd.position.offset', 'modules/osd', 'Position', 0.5],
    ['osd.size', 'modules/osd', undefined, 1]
  ]);
  assert.equal(fields[1].options[0].preview, 'edge.qml');
});

// Its title and command are its entry's, edited in place: a `command`
// under its id as well would be a second command for the same terminal.
test('a terminal of your own: its entry\'s title and command, then the fields of a terminal, its own list', () => {
  const info = plain(Settings.terminalInfo({ id: 'btop', title: 'System', command: 'btop' }));
  assert.equal(info.title, 'System');
  assert.equal(info.terminal, true);
  assert.deepEqual(info.settings.map((f) => f.key),
    ['terminals.btop.title', 'terminals.btop.command', 'directory', 'idleMinutes', 'width', 'height', 'font']);
  assert.ok(info.settings[0].absolute && info.settings[1].absolute && info.settings[1].required);
  assert.ok(info.settings[2].help.includes('Empty is your home'), info.settings[2].help);
  assert.equal(Settings.moduleKind({ terminal: true }), 'terminal');
  assert.equal(Settings.moduleKind({ agent: true }), 'agent');
  // Without a title, the id names it.
  assert.ok(plain(Settings.terminalInfo({ id: 'logs', title: '', command: 'x' })).settings[3].help.includes('logs stops'));
  // On its page, the entry's keys stay as they are; the others go under its id.
  const fields = plain(Settings.moduleFields([{ moduleId: 'btop', title: 'System', terminal: true, settings: info.settings }]));
  assert.deepEqual(fields.map((f) => f.key).slice(0, 3), ['terminals.btop.title', 'terminals.btop.command', 'btop.directory']);
  assert.ok(fields.every((f) => f.page === 'modules/btop'));
});

const C = load('core/Components.js');
const registry = JSON.parse(fs.readFileSync(new URL('../core/components/components.json', import.meta.url), 'utf8')).components;
const voiceInfo = () => plain(Settings.parseModuleInfo(JSON.stringify({ title: 'Voice', live: { local: true } })));
const voiceKey = 'local-live.voiceLanguage';

test('the voice modules\' pages have a row for every component they need, and each exists', () => {
  const ids = (info) => info.settings.filter((f) => f.type === 'component').map((f) => f.component);
  const local = voiceInfo();
  const codex = plain(Settings.parseModuleInfo(JSON.stringify({ title: 'Codex Live', live: { command: 'codex' } })));
  assert.deepEqual(ids(local), ['libpulse', 'voxtype', 'localai']);
  assert.deepEqual(ids(codex), ['codex-live-audio']);
  for (const id of [...ids(local), ...ids(codex)]) assert.ok(registry.some((c) => c.id === id), id);
  // a component row keeps its module's page and sections, and takes no config key
  const fields = plain(Settings.moduleFields([{ moduleId: 'local-live', title: 'Voice', settings: local.settings }], ''));
  const row = fields.find((f) => f.component === 'voxtype');
  assert.equal(row.page, 'modules/local-live');
  assert.equal(row.section, 'Speech to text (STT)');
  assert.equal(row.key, undefined);
});

test('the Voice page keeps speech to text and text to speech apart, each with a note', () => {
  const sections = plain(Settings.sections(plain(Settings.moduleFields([{ moduleId: 'local-live', title: 'Voice', settings: voiceInfo().settings }], '')), 'modules/local-live'));
  assert.deepEqual(sections.map((s) => s.name), ['Agent', 'Audio', 'Speech to text (STT)', 'Text to speech (TTS)']);
  const named = (name) => sections.find((s) => s.name === name).fields;
  for (const name of ['Speech to text (STT)', 'Text to speech (TTS)']) assert.equal(named(name)[0].type, 'note', name);
  const keys = (name) => named(name).filter((f) => f.key).map((f) => f.key.replace('local-live.', ''));
  assert.deepEqual(keys('Speech to text (STT)'), ['language', 'speechModel']);
  assert.deepEqual(keys('Text to speech (TTS)'), ['voiceLanguage', 'voiceModel', 'otherVoiceModel', 'voice', 'localaiUrl']);
  assert.ok(named('Speech to text (STT)').some((f) => f.component === 'voxtype'));
  assert.ok(named('Text to speech (TTS)').some((f) => f.component === 'localai'));
});

test('the languages: one list for what you speak and what speaks, each a choice that keeps its default', () => {
  const local = voiceInfo();
  const stt = local.settings.find((f) => f.key === 'language');
  const tts = local.settings.find((f) => f.key === 'voiceLanguage');
  const codes = (f) => f.options.map((o) => o.value).filter((v) => v !== '' && v !== 'auto');
  assert.deepEqual(codes(stt), codes(tts));
  assert.deepEqual(stt.options.slice(0, 2).map((o) => o.value), ['', 'auto']);
  // every language of the speech models has both engines in the registry, and only those do
  const voices = registry.filter((c) => c.engine);
  assert.deepEqual([...new Set(voices.map((c) => c.language))].sort(), codes(tts).slice().sort());
  for (const code of codes(tts)) assert.deepEqual(voices.filter((c) => c.language === code).map((c) => c.engine).sort(), ['piper', 'pocket'], code);
  // the label is in its own language, and a key the config file already has keeps meaning
  assert.equal(stt.options.find((o) => o.value === 'pt').label, 'Português');
  assert.equal(stt.key, 'language');
});

test('a field with `when` shows only while another field has that value; the default counts', () => {
  const schema = [
    { key: 'lang', type: 'choice', default: 'pt' },
    { key: 'a', when: { key: 'lang', is: 'pt' } },
    { key: 'b', when: { key: 'lang', is: ['en', 'es'] } },
    { key: 'c' }
  ];
  const shown = (stored) => plain(Settings.shownFields(schema, (k) => stored[k])).map((f) => f.key);
  assert.deepEqual(shown({}), ['lang', 'a', 'c']);
  assert.deepEqual(shown({ lang: 'es' }), ['lang', 'b', 'c']);
  assert.deepEqual(shown({ lang: 'xx' }), ['lang', 'c']);
  assert.equal(Settings.effectiveValue(schema, 'lang', () => undefined), 'pt');
  assert.equal(Settings.effectiveValue(schema, 'nope', () => undefined), undefined);
});

test('moduleFields puts the references to a module\'s own fields under its id, as it does their keys', () => {
  const fields = plain(Settings.moduleFields([{ moduleId: 'm', title: 'M', settings: [
    { key: 'lang', type: 'choice', default: 'pt' },
    { key: 'a', type: 'string', default: '', when: { key: 'lang', is: 'pt' } },
    { type: 'components', by: { key: 'lang', field: 'language' }, match: { engine: 'piper' } },
    { key: 'v', type: 'choice', default: '', options: [{ value: 'x', needs: ['id', { engine: 'piper', language: '@lang', plain: 'text' }] }] }
  ] }], ''));
  assert.deepEqual(fields[1].when, { key: 'm.lang', is: 'pt' });
  assert.deepEqual(fields[2].by, { key: 'm.lang', field: 'language' });
  assert.deepEqual(fields[3].options[0].needs, ['id', { engine: 'piper', language: '@m.lang', plain: 'text' }]);
  // the same through the whole chain: the voice page's own references resolve to keys it has
  const voice = plain(Settings.moduleFields([{ moduleId: 'local-live', title: 'Voice', settings: voiceInfo().settings }], ''));
  const keys = voice.map((f) => f.key).filter(Boolean);
  for (const f of voice) {
    if (f.when) assert.ok(keys.includes(f.when.key), 'when ' + f.when.key);
    if (f.by) assert.ok(keys.includes(f.by.key), 'by ' + f.by.key);
    for (const o of f.options || []) for (const n of o.needs || []) for (const v of typeof n === 'string' ? [] : Object.values(n))
      if (String(v).startsWith('@')) assert.ok(keys.includes(v.slice(1)), 'needs ' + v);
  }
});

test('the Voice page shows the voices of the chosen language only, and every language\'s on the components page', () => {
  const base = plain(C.decorate(plain(Settings.moduleFields([{ moduleId: 'local-live', title: 'Voice', settings: voiceInfo().settings }], '')), registry, [], {}));
  const voiceRows = (lang) => plain(Settings.shownFields(base, (k) => (k === voiceKey ? lang : undefined)))
    .filter((f) => f.type === 'component' && registry.find((c) => c.id === f.component).engine).map((f) => f.component);
  for (const code of [...new Set(registry.filter((c) => c.engine).map((c) => c.language))]) {
    const want = registry.filter((c) => c.engine && c.language === code).map((c) => c.id);
    assert.deepEqual(voiceRows(code), want, code);
    assert.equal(want.length, 2);
  }
  assert.deepEqual(voiceRows(undefined), voiceRows('pt'));  // the default language
  assert.deepEqual(voiceRows('xx'), []);
  // the overview lists all of them, a section per language
  const overview = plain(C.overviewFields(registry, {}));
  assert.equal(overview.filter((f) => f.component && registry.find((c) => c.id === f.component).engine).length, registry.filter((c) => c.engine).length);
  assert.ok(new Set(overview.map((f) => f.section)).size > 7);
});

// ------------------------------------------- the daemon's schema, put together
const catalog = [{ moduleId: 'media', title: 'Now playing' }, { moduleId: 'btop', title: 'btop', terminal: true }];
const daemonPages = [
  { id: 'general', label: 'General', icon: 'g', sub: false },
  { id: 'notch/0', label: 'main', icon: '', sub: true },
  { id: 'notch/1', label: 'side', icon: '', sub: true },
  { id: 'modules/media', label: 'Now playing', icon: '', sub: true },
  { id: 'modules/weather', label: 'Weather', icon: '', sub: true },
];

test('windowPages: the host core pages, the daemon notch pages, the QML module pages and the daemon user module pages', () => {
  const pages = plain(Settings.windowPages(core, [], daemonPages, catalog));
  assert.deepEqual(pages.map((p) => p.id),
    ['general', 'modules', 'notch/0', 'notch/1', 'modules/media', 'modules/btop', 'modules/weather']);
  // a module the QML loads keeps its own page, not the daemon's copy
  assert.equal(pages.filter((p) => p.id === 'modules/media').length, 1);
  assert.equal(pages.find((p) => p.id === 'notch/1').label, 'side');
  assert.deepEqual(plain(Settings.windowPages(core, [], null, [])).map((p) => p.id), ['general', 'modules']);
});

test('windowFields: drops the built-in modules\' own fields and the daemon\'s resolved parts, previews the edge cards', () => {
  const edge = { page: 'notch/0', key: 'position.edge', type: 'choice', display: 'cards', value: 'top', visible: true, locked: null,
    options: [{ value: 'top', label: 'Top' }, { value: 'left', label: 'Left' }] };
  const other = { page: 'general', key: 'hoverOpenDelay', type: 'integer', default: 120, value: 120, visible: true, locked: null };
  const media = { page: 'modules/media', key: 'media.x', type: 'boolean' };
  const weather = { page: 'modules/weather', key: 'weather.city', type: 'string' };
  const nested = { page: 'notch/1', key: 'notches.1.position.edge', type: 'choice', display: 'cards', options: [{ value: 'top', label: 'Top' }] };
  const fields = plain(Settings.windowFields([edge, other, media, weather, nested], catalog, 'Edge.qml'));
  assert.deepEqual(fields.map((f) => f.key), ['position.edge', 'hoverOpenDelay', 'weather.city', 'notches.1.position.edge']);
  assert.deepEqual(fields[0].options.map((o) => o.preview), ['Edge.qml', 'Edge.qml']);
  assert.equal(fields[3].options[0].preview, 'Edge.qml');
  assert.ok(!('value' in fields[0]) && !('visible' in fields[0]) && !('locked' in fields[1]));
  assert.equal(fields[1].default, 120);
  assert.equal(other.value, 120, 'the daemon\'s field is not mutated');
  assert.deepEqual(plain(Settings.windowFields(null, catalog, 'Edge.qml')), []);
});

test('windowFields: an array that is a sequence (not Array.isArray) still gets its previews', () => {
  const options = new Proxy([{ value: 'top', label: 'Top' }], { getPrototypeOf: () => Object.prototype });
  const out = plain(Settings.windowFields([{ page: 'notch/0', key: 'position.edge', type: 'choice', display: 'cards', options }], [], 'E.qml'));
  assert.equal(out[0].options[0].preview, 'E.qml');
});
