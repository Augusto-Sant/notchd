# Theming

`core/Theme.qml` is a singleton every view reads. The core never reads a desktop's theme itself: the host sets `Theme.provider` to an object with any of the keys below, and everything else falls back to defaults. Bindings follow the provider live, so a theme switch restyles the notch at once.

## Provider keys

| Key | Default | |
|---|---|---|
| `foreground` | `#cacccc` | text and icons |
| `background` | `#101315` | the notch and windows |
| `accent` | the foreground | selection, progress, focus |
| `danger` | `#e06c75` | errors, invalid input |
| `success` | `#98c379` | working, all good (the claude and codex modules' dot while the agent works) |
| `fontFamily` | `monospace` | use a Nerd Font, so icons render |
| `fontCaption`, `fontBodySmall`, `fontBody`, `fontHeading`, `fontDisplay` | 10, 11, 12, 16, 24 | pixel sizes |
| `spacingScale` | 1 | multiplier behind `Theme.space()` |
| `terminalPalette` | `[]` (the terminal's own) | the 16 ANSI colors (`#rrggbb`, color 0 first) of a terminal in the notch; the Omarchy host reads its theme's `ghostty.conf` |

The Omarchy host maps its `Color`/`Style`; the standalone host reads the config's `theme` object. The GNOME extension takes the top bar's colors and scales the five text sizes, and the open notch's width, by `text-scaling-factor`: Settings, Accessibility, Large Text makes the notch's text bigger with the rest of the Shell's.

The keys below and the derived tokens are the **token set** every UI implements: the daemon never sends a color, only a token name (a view's `tone`, a canvas's token fill) and the list the UI must know (`welcome.tokens`, [architecture.md](architecture.md#handshake-and-version)). A UI for another OS maps the same tokens to its own source. The theme is not part of the protocol.

## Derived tokens

Use these instead of ad-hoc alphas, so the notch, modules and settings shade alike on any theme, light or dark.

| Token | For |
|---|---|
| `muted` | secondary text |
| `faint` | hints, placeholders, disabled |
| `line` | borders and dividers |
| `hover`, `selected` | fills for hovered and selected items |
| `surface` | raised panels (the settings sidebar) |
| `radiusSmall`, `radius` | corner radii |

`Theme.alpha(color, a)` and `Theme.mix(a, b, t)` are there for the rare tone the tokens don't cover.

## Sizes

`Theme.space(px)` turns design pixels into real ones through `spacingScale`. Every size in the core goes through it, so a denser or roomier host keeps the proportions.
