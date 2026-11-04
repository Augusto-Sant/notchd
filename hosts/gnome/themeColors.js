// The tones of `welcome.tokens` from a foreground, a background and an
// accent (the same derivations as core/Theme.qml), and the panel's colors
// they come from. No GNOME imports: the Node tests run it as is.

// Adwaita's, for when the panel gives none (yet).
export const DARK = {foreground: '#ffffff', background: '#1e1e1e', danger: '#ff7b63', success: '#8ff0a4'};
export const LIGHT = {foreground: '#1e1e1e', background: '#fafafa', danger: '#c01c28', success: '#26a269'};

export function parse(hex) {
    const n = parseInt(hex.slice(1), 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function rgba(c, a) {
    return {r: c[0] / 255, g: c[1] / 255, b: c[2] / 255, a};
}

function mix(a, b, t) {
    return [0, 1, 2].map(i => a[i] + (b[i] - a[i]) * t);
}

// A color a theme node gives ({red, green, blue, alpha}, 0..255) as
// [r, g, b], or null when it is see-through: the panel is transparent over
// the overview and the lock screen, and the notch keeps the color it had.
export function opaque(color) {
    if (!color || !(Number(color.alpha) >= 128))
        return null;
    return [color.red, color.green, color.blue].map(v => Math.max(0, Math.min(255, Number(v) || 0)));
}

// Whether a background reads as dark (relative luminance under a half).
export function isDark(bg) {
    const [r, g, b] = bg.map(v => {
        const c = v / 255;
        return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5;
}

// The notch's tones. `panel`, {background, foreground} as [r, g, b] or
// null: the top bar's, so the notch hanging from it reads as part of it;
// without them, Adwaita's for the `dark` color scheme. `accent` is a hex.
export function tones(panel, dark, accent) {
    const bg = panel?.background ?? parse((dark ? DARK : LIGHT).background);
    const shade = panel?.background ? isDark(bg) : dark;
    const base = shade ? DARK : LIGHT;
    const fg = panel?.background && panel.foreground ? panel.foreground : parse(base.foreground);
    return {
        dark: shade,
        colors: {
            foreground: rgba(fg, 1),
            background: rgba(bg, 1),
            accent: rgba(parse(accent), 1),
            danger: rgba(parse(base.danger), 1),
            success: rgba(parse(base.success), 1),
            muted: rgba(fg, 0.6),
            faint: rgba(fg, 0.35),
            line: rgba(fg, 0.14),
            hover: rgba(fg, 0.06),
            selected: rgba(fg, 0.10),
            surface: rgba(mix(bg, fg, 0.035), 1),
        },
    };
}
