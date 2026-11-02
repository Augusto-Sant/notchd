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
