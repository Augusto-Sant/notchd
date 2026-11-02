// The JS side of the rule vectors: which JS file each vector file is about,
// and the few functions whose arguments or result are adapted so a case is
// plain JSON (see tests/vectors/README.md, "Rules"). Shared by
// tests/vectors-rules.test.mjs and by whatever generates a case's `expected`.
import { load, plain } from '../../load.mjs';

export const SOURCES = {
  'config.json': 'core/Config.js',
  'settings.json': 'core/settings/Settings.js',
  'position.json': 'core/Position.js',
};

// `{"$undefined": true}` stands for JS `undefined`, as an argument and as a result.
const isUndef = (v) => v !== null && typeof v === 'object' && !Array.isArray(v) && v.$undefined === true;
const decode = (v) => (isUndef(v) ? undefined : v);
export const encode = (v) => (v === undefined ? { $undefined: true } : plain(v));

// What a parse error says is the engine's own (V8's, serde's); only whether it
// is one of the rules' fixed messages is compared.
const FIXED = ['the config must be a JSON object', 'not an object', 'agent: needs a command', 'live: needs a command'];
const message = (e) => (e === undefined || FIXED.includes(e) ? e : 'syntax');

const before = {
  'config.json': {
    reservedKeys: (m) => () => m.RESERVED_KEYS
  },
  'settings.json': {
    effectiveValue: (m) => (schema, key, values) => m.effectiveValue(schema, key, (k) => values[k]),
    shownFields: (m) => (schema, values) => m.shownFields(schema, (k) => values[k]),
    // A malformed percent escape throws a URIError in JS; the Rust port answers "".
    moduleInfoPath: (m) => (url, dir) => { try { return m.moduleInfoPath(url, dir); } catch (e) { if (e.name !== 'URIError') throw e; return ''; } },
    sortWorkspaces: (m) => (list) => m.sortWorkspaces(list.slice())
  },
};
const after = {
  'config.json': { parse: (r) => (r.ok ? r : { ok: false, error: message(r.error) }) },
  'settings.json': { parseModuleInfo: (r) => (r.error === undefined ? r : { ...r, error: message(r.error) }) }
};

const mods = {};
// Whether the JS still has the function: the cases of one that went (the Rust
// port is the only one left) are run by the Rust runner alone.
export function has(file, fn) {
  const m = (mods[file] ||= load(SOURCES[file]));
  return before[file]?.[fn] !== undefined || typeof m[fn] === 'function';
}
export function call(file, fn, args) {
  const m = (mods[file] ||= load(SOURCES[file]));
  const wrap = before[file]?.[fn];
  const f = wrap ? wrap(m) : m[fn];
  if (typeof f !== 'function') throw new Error(`${file}: no function ${fn}`);
  const result = f(...JSON.parse(JSON.stringify(args)).map(decode));
  const post = after[file]?.[fn];
  return encode(post ? post(plain(result)) : result);
}
