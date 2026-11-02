# Golden vectors

Language-neutral test cases for the pixel geometry that stays in each UI
(`core/Position.js` apart from `resolve`/`configValue`, and `core/Outline.js`).
Every UI implementation (QML today, macOS and Windows later) must reimplement
that geometry identically and pass every case here.

Files, under `geometry/`:

- `position.json`: `place`, `pack`, `dragTarget`, `nearestEdge`, `stretch`,
  `insets`, `isVertical`, `screenAt`, `rowDepths`, `corners`,
  `coveredByStandalone`, `cornerInput`.
- `outline.json`: `path`.

Format:

```json
{
  "description": "...",
  "tolerance": 1e-9,
  "cases": [
    { "name": "...", "fn": "place", "args": [ {...} ], "expected": {...}, "source": "tests" }
  ]
}
```

- `fn` is the function name; `args` its positional arguments, as JSON.
- `expected` is the return value. Objects compare by key (order does not
  matter); a key absent from an object argument is absent, not null.
- Numbers compare within `tolerance` (absolute); the JS reference is held to
  exact equality. SVG path strings (`outline.json`) compare exactly: numbers in
  them are rounded to three decimals with trailing zeros dropped.
- `source`: `tests` for a case taken from `tests/position.test.mjs` or
  `tests/outline.test.mjs`, `added` for an edge case the old tests did not hold.
- No Qt types, screen objects or QML values: screens are plain
  `{ name, x, y, width, height }`, insets `{ top, bottom, left, right }`.
- `null` stands for a missing list (`pack`, `corners`, ...), which the JS
  treats as empty.

`tests/vectors.test.mjs` runs every case against the JS. Change a vector only
when the geometry is meant to change, and then in every UI.

## Rules (`rules/`)

The same idea for the decisions in `core/*.js` and `core/settings/Settings.js`,
which have a Rust port in `native/notchd-core/`. While both exist they must
agree: this is the differential check of `docs/architecture.md`, Phase 1.

One file per JS file: `config`, `settings`, `state`, `navigate`, `position`
(the config half: `resolve`, `configValue`, `isEdge`, `isVertical`; the
geometry half is `geometry/position.json`), `countdown`, `usage`,
`resources`, `agent`, `live`, `components`, `terminal`. Each is
`{ description, cases: [{ name, fn, args, expected, source }] }`:

- `fn` is the JS function name; `args` its positional arguments, `expected`
  its return value, both plain JSON. Objects compare by key; numbers by value
  (`1` is `1.0`).
- `source`: `tests` for a case taken from a Node test, `added` for an edge
  case (JS quirks the ports had to follow: `undefined` against `null` against
  a missing key, `Number` and `String` coercion, `Math.round` and `toFixed`,
  `\s` whitespace, key order).
- `ordered: true`: the keys of the result must also come out in the same
  order (the config edits: the user's file is never reordered).
- `knownDiff: "<why>"`: JS and Rust disagree on this case and nobody has
  decided which is right. Both runners skip it and print the count; the Rust
  runner fails when a marked case starts to agree, so the mark gets removed.
- `{"$undefined": true}` stands for JS `undefined`, as an argument and as an
  expected result (`valueAt` of a missing key, `toggleWorkspace` of the last
  workspace).

`tests/vectors-rules.test.mjs` runs every case against the JS;
`native/notchd-core/tests/vectors.rs` runs them against the Rust. The `fn`
dispatch and the argument and result adapters are `tests/vectors/rules/adapt.mjs`
(JS) and `native/notchd-core/tests/vectors/<file>.rs` (Rust). A case never
needs a QML object, a file or an identity check; where the JS takes one, the
adapter stands in:

- Callbacks (`valueOf`) are an object of values (`effectiveValue`,
  `shownFields`, `needed`, `gate`). `sortWorkspaces` returns a sorted copy.
- Modules (`State`) are objects with an `id`, and what returns modules
  returns their ids; in `canPeek`, `current` is the module itself when the ids
  match. Components (`Components`) are the registry's entries, and what
  returns them returns their ids (`parseCatalog`: `{ id, title, kind }`).
- Where the Rust API differs on purpose: a `-1` is a `None` (`indexIn`,
  `focusedPeek`, `start`), `{ error }` is an `Err` (`countdown.parseCommand`,
  `usage.request`), a missing title or path is `""`, `Countdown`'s `null` state
  is the idle one (a `null` `next` and the idle `duration: 0` are not
  compared), `Countdown.add` takes no `now` there, `Live.parseLine` answers
  `true` or `null` (the event's shape is Rust's own), and `Live.reduce` reads
  its event as `parse_line` does.
- Error texts that are the engine's own (V8's and serde's JSON syntax
  errors in `Config.parse` and `parseModuleInfo`) compare as `"syntax"`; the
  rules' own messages compare as they are. A bad percent escape throws a
  `URIError` in JS (`moduleInfoPath`, `localPath`) and is `""` in Rust: both
  sides say `""`.
- Dates without a zone are left out of `Usage` (JS reads them as local time),
  so the cases do not depend on the machine's TZ.

A new function or edge case goes in as a case here, with its `expected` taken
from the JS; run both runners. Remove the JS runner and its half of the check
when the JS rules go.
