// A terminal is a Shell actor inside the surface node. The daemon owns the
// PTY and sends an already-emulated grid on this actor's own socket.
import Cairo from 'cairo';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import PangoCairo from 'gi://PangoCairo';
import St from 'gi://St';

import {hexColor} from './terminalColors.js';

const PAD = 8;

function rgba(value) {
    const m = String(value).match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\)$/);
    if (m)
        return [Number(m[1]) / 255, Number(m[2]) / 255, Number(m[3]) / 255, Number(m[4] ?? 1)];
    const hex = String(value).match(/^#([0-9a-f]{6})$/i);
    return hex ? [1, 3, 5].map(i => parseInt(hex[1].slice(i - 1, i + 1), 16) / 255).concat(1) : [0, 0, 0, 1];
}

function modifiers(state) {
    let mods = 0;
    if (state & Clutter.ModifierType.SHIFT_MASK) mods |= 1;
    if (state & Clutter.ModifierType.MOD1_MASK) mods |= 2;
    if (state & Clutter.ModifierType.CONTROL_MASK) mods |= 4;
    if (state & Clutter.ModifierType.SUPER_MASK) mods |= 8;
    return mods;
}

const KEY_NAMES = {
    [Clutter.KEY_Return]: 'Enter', [Clutter.KEY_KP_Enter]: 'NumpadEnter',
    [Clutter.KEY_Escape]: 'Escape', [Clutter.KEY_Tab]: 'Tab', [Clutter.KEY_ISO_Left_Tab]: 'Tab',
    [Clutter.KEY_BackSpace]: 'Backspace', [Clutter.KEY_Insert]: 'Insert',
    [Clutter.KEY_Delete]: 'Delete', [Clutter.KEY_Home]: 'Home', [Clutter.KEY_End]: 'End',
    [Clutter.KEY_Left]: 'ArrowLeft', [Clutter.KEY_Up]: 'ArrowUp',
    [Clutter.KEY_Right]: 'ArrowRight', [Clutter.KEY_Down]: 'ArrowDown',
    [Clutter.KEY_Page_Up]: 'PageUp', [Clutter.KEY_Page_Down]: 'PageDown',
    [Clutter.KEY_space]: 'Space',
    [Clutter.KEY_minus]: 'Minus', [Clutter.KEY_equal]: 'Equal',
    [Clutter.KEY_bracketleft]: 'BracketLeft', [Clutter.KEY_bracketright]: 'BracketRight',
    [Clutter.KEY_backslash]: 'Backslash', [Clutter.KEY_semicolon]: 'Semicolon',
    [Clutter.KEY_apostrophe]: 'Quote', [Clutter.KEY_comma]: 'Comma',
    [Clutter.KEY_period]: 'Period', [Clutter.KEY_slash]: 'Slash',
    [Clutter.KEY_grave]: 'Backquote',
};

export const TerminalSurface = GObject.registerClass(
class TerminalSurface extends St.DrawingArea {
    _init(host, ref, run, onLine) {
        super._init({reactive: true, can_focus: true, clip_to_allocation: true,
            layout_manager: new Clutter.BinLayout()});
        this._host = host;
        this._ref = ref;
        this._run = run;
        this._onLine = onLine;
        this._rows = [];
        this._cursor = null;
        this._images = [];
        this._pictures = new Map();
        this._mouse = false;
        this._exited = false;
        this._typed = false;
        this._cellW = 8;
        this._cellH = 16;
        this._sentSize = '';
        this._font = null;
        this.updateFont();
        this._cancel = null;
        this._retryId = 0;
        this._retries = 0;
        this._conn = null;
        this._lastCell = '';
        this._held = '';
        this._wheelRest = 0;
        this._pointer = [0, 0];
        this._enterKeyAt = 0;
        // Clutter.Text owns the input method (dead keys, IBus and fcitx).
        // It stays mapped for focus, but its glyphs are drawn only by the
        // terminal grid. Each committed string goes to the daemon once.
        this._input = new St.Entry({width: 1, height: 1, opacity: 0,
            x_align: Clutter.ActorAlign.START, y_align: Clutter.ActorAlign.START,
            style: 'min-width: 1px; min-height: 1px; padding: 0; border: 0;'});
        this.add_child(this._input);
        this._input.clutter_text.connect('key-press-event', (actor, event) => this._inputKey(event));
        this._input.clutter_text.connect('text-changed', () => this._committed());
        // GNOME 51 handles Enter inside Clutter.Text and emits `activate`
        // without a key-press-event. GNOME 46 still sends the key event.
        this._input.clutter_text.connect('activate', () => this._activated());
        this.connect('repaint', () => this._paint());
        this.connect('notify::width', () => this._resize());
        this.connect('notify::height', () => this._resize());
        this.connect('button-press-event', (a, e) => this._press(e));
        this.connect('button-release-event', (a, e) => this._release(e));
        this.connect('motion-event', (a, e) => this._motion(e));
        this.connect('scroll-event', (a, e) => this._scroll(e));
        this.connect('enter-event', () => { this._onLine('pointer in'); });
        this.connect('leave-event', () => { this._onLine('pointer out'); });
        this.connect('key-press-event', (a, e) => this.focused ? Clutter.EVENT_PROPAGATE : this.key(e));
    }

    _colors() {
        const {background, foreground} = this._host.theme.colors;
        return [hexColor(background), hexColor(foreground)];
    }

    updateFont() {
        const id = this._ref.replace(/^term:/, '');
        const configured = this._host.config()?.[id]?.font;
        const family = typeof configured === 'string' && configured.trim() ? configured.trim() : 'Monospace';
        const size = this._host.theme.fonts.body;
        if (family === this._fontFamily && size === this._fontSize)
            return;
        this._fontFamily = family;
        this._fontSize = size;
        this._font = Pango.FontDescription.from_string('Monospace 12');
        this._font.set_family(family);
        this._font.set_absolute_size(size * Pango.SCALE);
        this.queue_repaint();
    }

    _send(message) {
        if (!this._out) return;
        try {
            this._out.write_all(new TextEncoder().encode(`${JSON.stringify({t: 'surface', ref: this._ref, ...message})}\n`), null);
        } catch { /* The read loop reconnects. */ }
    }

    _size() {
        return [Math.max(2, Math.floor((this.width - PAD * 2) / this._cellW)),
            Math.max(2, Math.floor((this.height - PAD * 2) / this._cellH))];
    }

    _resize() {
        if (this._destroyed || this.width <= 0 || this.height <= 0)
            return;
        if (!this._cancel) {
            this._connect();
            return;
        }
        const [cols, rows] = this._size();
        const next = `${cols}x${rows}@${Math.round(this._cellW)}x${Math.round(this._cellH)}`;
        if (this._out && this._sentSize !== next) {
            this._sentSize = next;
            this._send({ev: 'resize', cols, rows, cw: Math.round(this._cellW), ch: Math.round(this._cellH)});
        }
    }

    _connect() {
        if (this._destroyed) return;
        if (this._retryId) {
            GLib.source_remove(this._retryId);
            this._retryId = 0;
        }
        this._cancel?.cancel();
        try { this._conn?.close(null); } catch { /* Already closed. */ }
        this._out = null;
        const client = new Gio.SocketClient();
        const cancel = new Gio.Cancellable();
        this._cancel = cancel;
        client.connect_async(Gio.UnixSocketAddress.new(this._host.socket()), cancel, (c, result) => {
            if (cancel.is_cancelled()) return;
            try { this._conn = c.connect_finish(result); } catch (e) {
                this._error = `cannot reach notchd: ${e.message}`;
                this.queue_repaint();
                this._retry();
                return;
            }
            this._error = '';
            this._out = this._conn.get_output_stream();
            const input = new Gio.DataInputStream({base_stream: this._conn.get_input_stream()});
            const [cols, rows] = this._size();
            this._sentSize = `${cols}x${rows}@${Math.round(this._cellW)}x${Math.round(this._cellH)}`;
            const [bg, fg] = this._colors();
            try {
                this._out.write_all(new TextEncoder().encode(`${JSON.stringify({t: 'attach', ref: this._ref,
                    cols, rows, render: 'grid', bg, fg, cw: Math.round(this._cellW), ch: Math.round(this._cellH)})}\n`), null);
            } catch (e) {
                this._error = `cannot attach terminal: ${e.message}`;
                this._retry();
                return;
            }
            this._pictures.clear();
            this._images = [];
            this._exited = false;
            const next = () => input.read_line_async(GLib.PRIORITY_DEFAULT, cancel, (stream, res) => {
                let line = null;
                try { [line] = stream.read_line_finish_utf8(res); } catch { /* Disconnected. */ }
                if (cancel.is_cancelled()) return;
                if (line === null) { if (!this._exited) this._retry(); return; }
                this._retries = 0;
                this._handle(line);
                next();
            });
            next();
        });
    }

    _retry() {
        if (this._retryId || this._destroyed || this._exited) return;
        this._retryId = GLib.timeout_add(GLib.PRIORITY_DEFAULT,
            Math.min(5000, 250 * 2 ** this._retries++), () => {
                this._retryId = 0;
                this._connect();
                return GLib.SOURCE_REMOVE;
            });
    }

    _handle(line) {
        let m;
        try { m = JSON.parse(line); } catch { return; }
        if (m.t === 'surface' && m.ev === 'grid') {
            if (m.full || m.rows !== this._rows.length)
                this._rows = Array.from({length: m.rows}, () => ({runs: [], sel: null}));
            for (const row of m.lines ?? [])
                if (row.y >= 0 && row.y < this._rows.length)
                    this._rows[row.y] = {runs: row.runs ?? [], sel: row.sel ?? null};
            this._cursor = m.cursor ?? null;
            this._mouse = m.mouse === true;
            if (m.images !== undefined) this._images = m.images;
            this.queue_repaint();
        } else if (m.t === 'surface' && m.ev === 'image') {
            // GJS Cairo accepts a PNG filename; keep only a decoded surface.
            let path = null;
            try {
                const [fd, temporary] = GLib.file_open_tmp('notchd-image-XXXXXX');
                path = temporary;
                GLib.close(fd);
                GLib.file_set_contents(path, GLib.base64_decode(m.png));
                const image = Cairo.ImageSurface.createFromPNG(path);
                this._pictures.set(m.id, image);
                this.queue_repaint();
            } catch (e) { console.warn(`notchd: terminal image: ${e.message}`); }
            finally { if (path) GLib.unlink(path); }
        } else if (m.t === 'surface' && m.ev === 'title') {
            this._title = String(m.text ?? '');
        } else if (m.t === 'surface' && m.ev === 'copy') {
            St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, String(m.text ?? ''));
        } else if (m.t === 'surface' && m.ev === 'clipboard') {
            St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, (clip, text) => {
                if (text) this._send({ev: 'paste', text});
            });
        } else if (m.t === 'surface-close') {
            this._exited = true;
            this._onLine('exited');
            this.queue_repaint();
        } else if (m.t === 'err') {
            this._error = String(m.message ?? m.code ?? 'error');
            this.queue_repaint();
        }
    }

    _paint() {
        const cr = this.get_context();
        const [width, height] = this.get_surface_size();
        const [bg, fg] = this._colors();
        const color = (v, alpha = 1) => {
            const [r, g, b, a] = rgba(v);
            cr.setSourceRGBA(r, g, b, a * alpha);
        };
        cr.setOperator(Cairo.Operator.CLEAR);
        cr.paint();
        cr.setOperator(Cairo.Operator.OVER);
        const x = 1, y = 1, w = Math.max(0, width - 2), h = Math.max(0, height - 2);
        const r = Math.max(0, Math.min(this._host.theme.radius - 1, w / 2, h / 2));
        cr.newPath();
        cr.arc(x + w - r, y + r, r, -Math.PI / 2, 0);
        cr.arc(x + w - r, y + h - r, r, 0, Math.PI / 2);
        cr.arc(x + r, y + h - r, r, Math.PI / 2, Math.PI);
        cr.arc(x + r, y + r, r, Math.PI, 3 * Math.PI / 2);
        cr.closePath();
        color(bg); cr.fill();
        cr.save();
        cr.rectangle(PAD, PAD, Math.max(0, width - 2 * PAD), Math.max(0, height - 2 * PAD));
        cr.clip();
        cr.translate(PAD, PAD);
        const layout = PangoCairo.create_layout(cr);
        layout.set_font_description(this._font);
        layout.set_text('M', -1);
        const [, logical] = layout.get_extents();
        const cw = Math.max(1, logical.width / Pango.SCALE);
        const ch = Math.max(1, Math.ceil(logical.height / Pango.SCALE));
        if (cw !== this._cellW || ch !== this._cellH) {
            this._cellW = cw; this._cellH = ch;
            GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => { this._resize(); return GLib.SOURCE_REMOVE; });
        }
        const images = list => {
            for (const im of list) {
                const pix = this._pictures.get(im.id);
                if (!pix || !im.sw || !im.sh) continue;
                const x = im.x * cw + im.xo, y = im.y * ch + im.yo;
                const sx = im.w / im.sw, sy = im.h / im.sh;
                cr.save(); cr.rectangle(x, y, im.w, im.h); cr.clip();
                cr.translate(x - im.sx * sx, y - im.sy * sy); cr.scale(sx, sy);
                cr.setSourceSurface(pix, 0, 0); cr.paint(); cr.restore();
            }
        };
        images(this._images.filter(i => i.z < 0));
        this._rows.forEach((row, y) => {
            const ry = y * ch;
            for (const run of row.runs) if (run.bg) {
                color(run.bg); cr.rectangle(run.x * cw, ry, run.w * cw, ch); cr.fill();
            }
            if (row.sel) {
                color(fg, 0.3);
                cr.rectangle(row.sel[0] * cw, ry, (row.sel[1] - row.sel[0] + 1) * cw, ch); cr.fill();
            }
            for (const run of row.runs) {
                if (!run.text?.trim()) continue;
                const a = run.a ?? 0;
                const font = this._font.copy();
                if (a & 1) font.set_weight(Pango.Weight.BOLD);
                if (a & 2) font.set_style(Pango.Style.ITALIC);
                layout.set_font_description(font);
                const attrs = new Pango.AttrList();
                if (a & 4) attrs.insert(Pango.attr_underline_new(Pango.Underline.SINGLE));
                if (a & 8) attrs.insert(Pango.attr_strikethrough_new(true));
                layout.set_attributes(attrs);
                layout.set_text(run.text, -1);
                color(run.fg ?? fg, a & 16 ? 0.6 : 1);
                cr.moveTo(run.x * cw, ry); PangoCairo.show_layout(cr, layout);
            }
        });
        images(this._images.filter(i => i.z >= 0));
        const cursor = this._cursor;
        if (cursor && !this._exited) {
            const x = cursor.x * cw, y = cursor.y * ch;
            color(fg, cursor.style === 'block' && this.focused ? 0.55 : 1);
            if (cursor.style === 'bar') { cr.rectangle(x, y, 2, ch); cr.fill(); }
            else if (cursor.style === 'underline') { cr.rectangle(x, y + ch - 2, cw, 2); cr.fill(); }
            else if (cursor.style === 'hollow' || !this.focused) {
                cr.setLineWidth(1); cr.rectangle(x + 0.5, y + 0.5, cw - 1, ch - 1); cr.stroke();
            } else { cr.rectangle(x, y, cw, ch); cr.fill(); }
        }
        if (this._error) {
            layout.set_font_description(this._font); layout.set_attributes(null);
            layout.set_text(this._error, -1); color(fg, 0.6);
            cr.moveTo(0, 0); PangoCairo.show_layout(cr, layout);
        }
        cr.restore();
        cr.$dispose();
    }

    key(event) {
        const sym = event.get_key_symbol(), mods = modifiers(event.get_state());
        if (sym === Clutter.KEY_Return || sym === Clutter.KEY_KP_Enter)
            this._enterKeyAt = GLib.get_monotonic_time();
        if (sym === Clutter.KEY_Escape && mods === 4) { this._onLine('leave'); return Clutter.EVENT_STOP; }
        if (this._exited) {
            if (sym === Clutter.KEY_Return || sym === Clutter.KEY_KP_Enter) this._onLine('restart');
            return Clutter.EVENT_STOP;
        }
        let key = KEY_NAMES[sym] ?? '';
        const text = event.get_key_unicode() ?? '';
        const cp = text.codePointAt(0) ?? 0;
        const ascii = sym < 128 ? String.fromCharCode(sym) : '';
        if (!key && /^[a-z]$/i.test(ascii)) key = `Key${ascii.toUpperCase()}`;
        if (!key && /^[0-9]$/.test(ascii)) key = `Digit${ascii}`;
        if (!key && sym >= Clutter.KEY_F1 && sym <= Clutter.KEY_F24) key = `F${sym - Clutter.KEY_F1 + 1}`;
        if (!key && (!cp || cp < 32)) return Clutter.EVENT_STOP;
        this._send({ev: 'key', key, mods: sym === Clutter.KEY_ISO_Left_Tab ? mods | 1 : mods,
            ...(cp >= 32 && cp !== 127 && !(mods & 4) ? {text} : {})});
        this._didType();
        return Clutter.EVENT_STOP;
    }

    _didType() {
        if (!this._typed) { this._typed = true; this._onLine('typed'); }
    }

    _inputKey(event) {
        const sym = event.get_key_symbol();
        const mods = modifiers(event.get_state());
        const level3 = !!(event.get_state() & Clutter.ModifierType.MOD5_MASK);
        // While composing, Enter and arrows may select a candidate. Let the
        // input method finish; its committed text arrives in _committed().
        if (this._input.clutter_text.has_preedit?.() && (!(mods & 14) || level3)) {
            this._rememberInput({key: '', mods: 0, text: ''});
            return Clutter.EVENT_PROPAGATE;
        }
        // These must carry their physical code to the terminal (including
        // Ctrl+Shift+C/V and the kitty keyboard protocol).
        const text = event.get_key_unicode() ?? '';
        const composing = sym >= 0xfe50 && sym <= 0xfe6f || sym === Clutter.KEY_Multi_key;
        if (this._exited || ((mods & 14) && !level3) || (KEY_NAMES[sym] && sym !== Clutter.KEY_space) ||
            (!text && !composing))
            return this.key(event);
        const ascii = sym < 128 ? String.fromCharCode(sym) : '';
        this._rememberInput({
            key: /^[a-z]$/i.test(ascii) ? `Key${ascii.toUpperCase()}`
                : /^[0-9]$/.test(ascii) ? `Digit${ascii}` : KEY_NAMES[sym] ?? '',
            mods: level3 ? 0 : mods, text,
        });
        return Clutter.EVENT_PROPAGATE;
    }

    _rememberInput(pending) {
        this._pendingInput = pending;
        // A composition can commit later, outside the key event. It must
        // then carry text alone rather than an unrelated physical key.
        GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            if (this._pendingInput === pending)
                this._pendingInput = null;
            return GLib.SOURCE_REMOVE;
        });
    }

    _committed() {
        if (this._clearingInput) return;
        const text = this._input.clutter_text.get_text();
        if (!text) return;
        this._clearingInput = true;
        this._input.clutter_text.set_text('');
        this._clearingInput = false;
        const pending = this._pendingInput;
        this._pendingInput = null;
        if (this._exited) return;
        const sameKey = pending && text === pending.text;
        this._send({ev: 'key', key: sameKey ? pending.key : '',
            mods: sameKey ? pending.mods : 0, text});
        this._didType();
    }

    _activated() {
        // A legacy key-press can be followed by activate in the same turn.
        if (GLib.get_monotonic_time() - this._enterKeyAt < 100000)
            return;
        if (this._exited) {
            this._onLine('restart');
            return;
        }
        const state = global.get_pointer()[2];
        this._send({ev: 'key', key: 'Enter', mods: modifiers(state)});
        this._didType();
    }

    focusInput() {
        this._input.clutter_text.grab_key_focus();
        this.queue_repaint();
    }

    _cell(x, y) { return {x: Math.max(0, (x - PAD) / this._cellW), y: Math.max(0, (y - PAD) / this._cellH)}; }
    _mouseEvent(action, button, event) {
        const [stageX, stageY] = event.get_coords();
        const [x, y] = this.transform_stage_point(stageX, stageY).slice(1);
        this._send({ev: 'mouse', action, ...this._cell(x, y), mods: modifiers(event.get_state()),
            ...(button ? {button} : {})});
    }
    _button(event) { return ({1: 'left', 2: 'middle', 3: 'right'})[event.get_button()] ?? ''; }
    _press(event) {
        this._onLine('keyboard');
        this.focusInput();
        this._held = this._button(event);
        if (this._held) this._mouseEvent('press', this._held, event);
        return Clutter.EVENT_STOP;
    }
    _release(event) {
        const button = this._button(event);
        if (button) this._mouseEvent('release', button, event);
        this._held = '';
        return Clutter.EVENT_STOP;
    }
    _motion(event) {
        const [sx, sy] = event.get_coords();
        const [ok, x, y] = this.transform_stage_point(sx, sy);
        if (!ok) return Clutter.EVENT_PROPAGATE;
        this._pointer = [x, y];
        const cell = this._cell(x, y);
        const id = `${Math.floor(cell.x)},${Math.floor(cell.y)}`;
        if ((this._held || this._mouse) && id !== this._lastCell) {
            this._lastCell = id;
            this._send({ev: 'mouse', action: 'motion', ...cell, mods: modifiers(event.get_state()),
                ...(this._held ? {button: this._held} : {})});
        }
        return Clutter.EVENT_STOP;
    }
    _scroll(event) {
        const direction = event.get_scroll_direction();
        const delta = direction === Clutter.ScrollDirection.SMOOTH
            ? event.get_scroll_delta()[1]
            : direction === Clutter.ScrollDirection.UP ? -1 : direction === Clutter.ScrollDirection.DOWN ? 1 : 0;
        this._wheelRest += delta * 3;
        const rows = this._wheelRest > 0 ? Math.floor(this._wheelRest) : Math.ceil(this._wheelRest);
        if (rows) {
            this._wheelRest -= rows;
            this._send({ev: 'wheel', rows, ...this._cell(...this._pointer)});
        }
        return Clutter.EVENT_STOP;
    }
    recolor() {
        if (this.width > 0 && this.height > 0)
            this._connect();
        this.queue_repaint();
    }
    get focused() { return global.stage.get_key_focus() === this._input.clutter_text; }
    destroy() {
        if (this._destroyed)
            return;
        this._destroyed = true;
        if (this._retryId) GLib.source_remove(this._retryId);
        this._cancel?.cancel();
        try { this._conn?.close(null); } catch { /* Already closed. */ }
        super.destroy();
    }
});

export class Terminals {
    constructor(host) { this._host = host; this._surfaces = new Set(); }
    create(ref, run, onLine) {
        const surface = new TerminalSurface(this._host, ref, run, onLine);
        this._surfaces.add(surface);
        surface.connect('destroy', () => this._surfaces.delete(surface));
        return surface;
    }
    recolor() { for (const s of this._surfaces) s.recolor(); }
    updateFonts() { for (const s of this._surfaces) s.updateFont(); }
    describe() { return [...this._surfaces].map(s => ({ref: s._ref, run: s._run, actor: true,
        mapped: s.mapped, w: s.width, h: s.height, rows: s._rows.length, focused: s.focused,
        pictures: s._pictures.size, images: s._images.length,
        font: s._fontFamily, fontSize: s._fontSize, title: s._title ?? '',
        text: s._rows.map(row => row.runs.map(run => run.text).join('')).join('\n')})); }
    destroy() { for (const s of [...this._surfaces]) s.destroy(); this._surfaces.clear(); }
}
