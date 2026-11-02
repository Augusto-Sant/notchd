// GNOME's look mapped onto the tokens of `welcome.tokens` (the same names
// and derivations as core/Theme.qml): `color-scheme` picks dark or light,
// `accent-color` (GNOME 47+) the accent. On GNOME 46 the accent is
// Adwaita's default blue.

import Gio from 'gi://Gio';
import St from 'gi://St';

const ACCENTS = {
    blue: '#3584e4', teal: '#2190a4', green: '#3a944a', yellow: '#c88800',
    orange: '#ed5b00', red: '#e62d42', pink: '#d56199', purple: '#9141ac', slate: '#6f8396',
};

const DARK = {foreground: '#ffffff', background: '#1e1e1e', danger: '#ff7b63', success: '#8ff0a4'};
const LIGHT = {foreground: '#1e1e1e', background: '#fafafa', danger: '#c01c28', success: '#26a269'};

function parse(hex) {
    const n = parseInt(hex.slice(1), 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function rgba(c, a) {
    return {r: c[0] / 255, g: c[1] / 255, b: c[2] / 255, a};
}

function mix(a, b, t) {
    return [0, 1, 2].map(i => a[i] + (b[i] - a[i]) * t);
}

export class Theme {
    constructor(onChanged) {
        this._settings = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});
        this._hasAccent = this._settings.settings_schema.has_key('accent-color');
        this._ids = [
            this._settings.connect('changed::color-scheme', () => this._update(onChanged)),
        ];
        if (this._hasAccent)
            this._ids.push(this._settings.connect('changed::accent-color', () => this._update(onChanged)));
        this._themeContext = St.ThemeContext.get_for_stage(global.stage);
        this._scaleId = this._themeContext.connect('notify::scale-factor', () => this._update(onChanged));
        this._update(null);
    }

    destroy() {
        for (const id of this._ids)
            this._settings.disconnect(id);
        this._themeContext.disconnect(this._scaleId);
    }

    _update(onChanged) {
        const scheme = this._settings.get_string('color-scheme');
        this.dark = scheme !== 'prefer-light';
        const base = this.dark ? DARK : LIGHT;
        const accentName = this._hasAccent ? this._settings.get_string('accent-color') : 'blue';
        const fg = parse(base.foreground);
        const bg = parse(base.background);
        this.colors = {
            foreground: rgba(fg, 1),
            background: rgba(bg, 1),
            accent: rgba(parse(ACCENTS[accentName] ?? ACCENTS.blue), 1),
            danger: rgba(parse(base.danger), 1),
            success: rgba(parse(base.success), 1),
            muted: rgba(fg, 0.6),
            faint: rgba(fg, 0.35),
            line: rgba(fg, 0.14),
            hover: rgba(fg, 0.06),
            selected: rgba(fg, 0.10),
            surface: rgba(mix(bg, fg, 0.035), 1),
        };
        // St lengths set from code are not scaled by the UI scale factor
        // (CSS lengths are), so design pixels go through space().
        this.scale = this._themeContext.scale_factor;
        this.fonts = {caption: 10, bodySmall: 11, body: 12, heading: 16, display: 24};
        onChanged?.();
    }

    space(px) {
        const n = Number(px) * this.scale;
        return !isFinite(n) || n <= 0 ? 0 : Math.max(1, Math.round(n));
    }

    css(tone) {
        const c = this.colors[tone] ?? this.colors.foreground;
        return `rgba(${Math.round(c.r * 255)},${Math.round(c.g * 255)},${Math.round(c.b * 255)},${c.a})`;
    }
}
