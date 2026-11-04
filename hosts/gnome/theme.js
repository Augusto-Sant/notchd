// GNOME's look mapped onto the tokens of `welcome.tokens` (the same names
// and derivations as core/Theme.qml, themeColors.js): the notch hangs from
// the top bar, so it takes the bar's background and text colors, whatever
// the Shell theme, read from the panel's theme node again when its style
// changes. Over the overview and the lock screen the panel is see-through
// and the notch keeps the colors it had; before the panel has any, it is
// Adwaita's for `color-scheme`. `accent-color` (GNOME 47+) is the accent;
// on GNOME 46 it is Adwaita's default blue.

import Gio from 'gi://Gio';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {opaque, tones} from './themeColors.js';

const ACCENTS = {
    blue: '#3584e4', teal: '#2190a4', green: '#3a944a', yellow: '#c88800',
    orange: '#ed5b00', red: '#e62d42', pink: '#d56199', purple: '#9141ac', slate: '#6f8396',
};

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
        // A Shell theme loaded, or the panel's style changed (it goes
        // see-through over the overview): its colors again, a redraw only
        // when they changed.
        this._themeId = this._themeContext.connect('changed', () => this._panelChanged(onChanged));
        this._panel = Main.panel;
        this._panelStyleId = this._panel?.connect('style-changed', () => this._panelChanged(onChanged)) ?? 0;
        this._panelColors = this._readPanel();
        this._update(null);
    }

    destroy() {
        for (const id of this._ids)
            this._settings.disconnect(id);
        this._themeContext.disconnect(this._scaleId);
        this._themeContext.disconnect(this._themeId);
        if (this._panelStyleId)
            this._panel.disconnect(this._panelStyleId);
    }

    // The panel's {background, foreground} as [r, g, b], or null while it
    // has none to give (see-through, or not styled yet). The text color is
    // a panel button's (the clock's), as the panel's own is not set.
    _readPanel() {
        const node = this._panel?.peek_theme_node?.();
        const background = node ? opaque(node.get_background_color()) : null;
        if (!background)
            return null;
        const button = this._panel.statusArea?.dateMenu;
        const buttonNode = button?.peek_theme_node?.() ?? null;
        const foreground = opaque((buttonNode ?? node).get_foreground_color());
        return {background, foreground};
    }

    _panelChanged(onChanged) {
        const colors = this._readPanel();
        if (!colors || JSON.stringify(colors) === JSON.stringify(this._panelColors))
            return;
        this._panelColors = colors;
        this._update(onChanged);
    }

    _update(onChanged) {
        const scheme = this._settings.get_string('color-scheme');
        const accentName = this._hasAccent ? this._settings.get_string('accent-color') : 'blue';
        const look = tones(this._panelColors, scheme !== 'prefer-light', ACCENTS[accentName] ?? ACCENTS.blue);
        this.dark = look.dark;
        this.colors = look.colors;
        // What the panel gave, for the test hook.
        this.panelColors = this._panelColors;
        // St lengths set from code are not scaled by the UI scale factor
        // (CSS lengths are), so design pixels go through space().
        this.scale = this._themeContext.scale_factor;
        this.fonts = {caption: 10, bodySmall: 11, body: 12, heading: 16, display: 24};
        // core/Theme.qml's radii, in design pixels (CSS lengths).
        this.radiusSmall = 6;
        this.radius = 10;
        // A new look: what was drawn with the last one is drawn again.
        this.generation = (this.generation ?? 0) + 1;
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
