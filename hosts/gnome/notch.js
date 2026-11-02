// One notch copy on one monitor: the card (its outline drawn with Cairo from
// lib/Outline.js, the same path the QML card draws) and the module views in
// it. It draws the mode the daemon's `state` gives the copy and reports raw
// facts back: the pointer, a press, the keyboard, its measured depths. It
// decides nothing.

import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import * as Outline from './lib/Outline.js';
import * as Protocol from './lib/Protocol.js';
import {render} from './view.js';

const EASE_MS = 180;

// The card's paddings and sizes in design pixels, as core/NotchCard.qml has
// them for a top edge.
const CARD = {
    restDepth: 5, restLength: 120, shoulder: 10,
    compactPadNear: 4, compactPadFar: 6, compactPadSide: 14, compactRadius: 12,
    peekPadNear: 6, peekPadFar: 8, peekPadSide: 14, peekRadius: 16,
    expandedPadNear: 14, expandedPadFar: 18, expandedPadSide: 20, expandedRadius: 26,
    expandedWidth: 360, hitDepth: 8,
};

// Traces SVG path data of the subset Outline.js emits (M, L, Q, Z) on a
// Cairo context.
function trace(cr, d) {
    const tokens = d.trim().split(/\s+/);
    let i = 0;
    const num = () => Number(tokens[i++]);
    while (i < tokens.length) {
        const op = tokens[i++];
        if (op === 'M') {
            cr.moveTo(num(), num());
        } else if (op === 'L') {
            cr.lineTo(num(), num());
        } else if (op === 'Q') {
            // Cairo has cubic curves only: raise the quadratic.
            const [x0, y0] = cr.getCurrentPoint();
            const cx = num(), cy = num(), x = num(), y = num();
            cr.curveTo(x0 + 2 / 3 * (cx - x0), y0 + 2 / 3 * (cy - y0),
                x + 2 / 3 * (cx - x), y + 2 / 3 * (cy - y), x, y);
        } else if (op === 'Z') {
            cr.closePath();
        }
    }
}

export const NotchCopy = GObject.registerClass(
class NotchCopy extends St.Widget {
    // `host`: { theme, send(msg), store(), monitorFor(name), names() }.
    _init(host, notchId) {
        super._init({name: 'notchd-copy', reactive: true, track_hover: true, can_focus: true, x: 0, y: 0, clip_to_allocation: false});
        this._host = host;
        this.notchId = notchId;
        this._mode = 'rest';
        this._wireScreen = '';
        this._monitorIndex = 0;
        this._modal = null;
        this._viewKey = '';
        this._cardW = 0;
        this._cardH = 0;
        this._lastMeasure = '';

        this._bg = new St.DrawingArea({x: 0, y: 0});
        this._bg.connect('repaint', area => this._paint(area));
        this.add_child(this._bg);
        this._content = new St.Bin({x: 0, y: 0, opacity: 0});
        this.add_child(this._content);
        this.connect('notify::width', () => this._bg.set_size(this.width, this.height));
        this.connect('notify::height', () => this._bg.set_size(this.width, this.height));

        this.connect('notify::hover', () => this._send(Protocol.pointer(this.notchId, this._pointerScreen(this.hover ? 'enter' : 'leave'), this.hover ? 'enter' : 'leave')));
        this.connect('button-press-event', () => {
            this._send(Protocol.pointer(this.notchId, this._pointerScreen('press'), 'press'));
            return Clutter.EVENT_STOP;
        });
        this.connect('key-press-event', (actor, event) => this._key(event));

        Main.layoutManager.addTopChrome(this, {trackFullscreen: true});
    }

    _send(msg) {
        this._host.send(msg);
    }

    // A notch fixed on a screen is named by it; the one that follows focus
    // by "", except on enter and press, which name the screen the pointer
    // is on (the open notch stays there).
    _pointerScreen(ev) {
        if (this._wireScreen || ev === 'leave')
            return this._wireScreen;
        return this._host.names()[this._monitorIndex] ?? '';
    }

    get theme() {
        return this._host.theme;
    }

    // `notch`: the state's notch entry; `copy`: this copy's entry in it.
    update(notch, copy) {
        this._wireScreen = notch.screen ?? '';
        this._monitorIndex = this._host.monitorFor(copy.screen);
        this._notch = notch;
        this._copy = copy;
        const mode = copy.mode;
        const parts = this._parts(copy);
        const key = JSON.stringify([mode, parts]);
        if (key !== this._viewKey) {
            this._viewKey = key;
            this._content.child = this._renderParts(mode, parts);
        }
        this._mode = mode;
        this._layout(true);
        this._keyboard(copy.keyboard ?? 'none', mode);
    }

    // What this copy shows, as [module, slot, tree] parts: the compact views
    // side by side, the peek's view, or the open notch's expanded views
    // stacked (every module, or with tabs the picked one: `shown`).
    _parts(copy) {
        const views = this._host.store().views;
        const part = (module, slot) => {
            const tree = views[`${module}/${slot}`]?.tree ?? null;
            return tree ? [module, slot, tree] : null;
        };
        let parts = [];
        switch (copy.mode) {
        case 'compact':
            parts = (copy.compact ?? []).map(m => part(m, 'compact'));
            break;
        case 'peek':
            parts = copy.peek ? [part(copy.peek.module, copy.peek.view)] : [];
            break;
        case 'expanded':
            parts = (copy.shown ?? []).map(m => part(m, 'expanded'));
            break;
        }
        return parts.filter(p => p);
    }

    // Each part drawn with its own module and slot, so a button fires the
    // event of the module it belongs to.
    _renderParts(mode, parts) {
        if (parts.length === 0)
            return null;
        const actors = parts.map(([module, slot, tree]) => render(tree, {
            theme: this.theme,
            onAction: (action, value) => this._send(Protocol.event(module, slot, action, value)),
        })).filter(a => a);
        if (actors.length === 1)
            return actors[0];
        const vertical = mode === 'expanded';
        const box = new St.BoxLayout({style: `spacing: ${vertical ? 14 : 12}px;`});
        if ('orientation' in box)
            box.orientation = vertical ? Clutter.Orientation.VERTICAL : Clutter.Orientation.HORIZONTAL;
        else
            box.vertical = vertical;
        for (const a of actors) {
            if (!vertical)
                a.y_align = Clutter.ActorAlign.CENTER;
            box.add_child(a);
        }
        return box;
    }

    // The open notch is as wide as the widest module it shows asks for
    // (`module.expandedWidth`, design pixels).
    _expandedWidth() {
        const modules = this._host.store().modules;
        const widths = (this._copy?.shown ?? []).map(id => Number(modules[id]?.expandedWidth) || 0);
        return Math.max(CARD.expandedWidth, ...widths);
    }

    _pads() {
        const m = this._mode;
        const s = v => this.theme.space(v);
        if (m === 'compact')
            return {near: s(CARD.compactPadNear), far: s(CARD.compactPadFar), side: s(CARD.compactPadSide), radius: s(CARD.compactRadius)};
        if (m === 'peek')
            return {near: s(CARD.peekPadNear), far: s(CARD.peekPadFar), side: s(CARD.peekPadSide), radius: s(CARD.peekRadius)};
        if (m === 'expanded')
            return {near: s(CARD.expandedPadNear), far: s(CARD.expandedPadFar), side: s(CARD.expandedPadSide), radius: s(CARD.expandedRadius)};
        return {near: 0, far: 0, side: 0, radius: 0};
    }

    _layout(animate) {
        const monitor = Main.layoutManager.monitors[this._monitorIndex] ?? Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;
        const s = v => this.theme.space(v);
        const shoulder = s(CARD.shoulder);
        const pads = this._pads();
        let w, h;
        const child = this._content.child;
        if (this._mode === 'rest' || !child) {
            w = s(CARD.restLength);
            h = s(CARD.restDepth);
        } else {
            const [, natW] = child.get_preferred_width(-1);
            const maxW = this._mode === 'expanded' ? s(this._expandedWidth()) : monitor.width / 2;
            const innerW = this._mode === 'expanded' ? maxW : Math.min(natW, maxW);
            const [, natH] = child.get_preferred_height(innerW);
            w = innerW + 2 * pads.side;
            h = natH + pads.near + pads.far;
            child.set_size(innerW, natH);
        }
        this._cardW = w;
        this._cardH = h;
        this._radius = pads.radius;
        // Below the top panel on the monitor it is on (the panel's own
        // strut keeps windows under it; the notch hangs from it).
        const panel = monitor.index === Main.layoutManager.primaryIndex ? Main.panel.height : 0;
        const offset = this._notch?.offset ?? 0.5;
        const total = w + 2 * shoulder;
        const x = Math.round(Math.min(Math.max(monitor.x + monitor.width * offset - total / 2, monitor.x), monitor.x + monitor.width - total));
        const y = monitor.y + panel;
        // The rest lip is thin: its hit area reaches a little deeper.
        const height = Math.max(h, this._mode === 'rest' ? s(CARD.hitDepth) : h);
        this._content.set_position(shoulder + pads.side, pads.near);
        const shown = this._mode !== 'rest';
        if (animate) {
            this._target = {x, y, w: total, h: height};
            this.ease({x, y, width: total, height, duration: EASE_MS, mode: Clutter.AnimationMode.EASE_OUT_QUAD});
            this._content.ease({opacity: shown ? 255 : 0, duration: EASE_MS, mode: Clutter.AnimationMode.EASE_OUT_QUAD});
        } else {
            this._target = {x, y, w: total, h: height};
            this.set_position(x, y);
            this.set_size(total, height);
            this._content.opacity = shown ? 255 : 0;
        }
        this._bg.queue_repaint();
        this._measure();
    }

    // What this copy measures of itself, for the daemon's reserve.
    _measure() {
        const s = v => this.theme.space(v);
        const compact = this._mode === 'compact' ? this._cardH : 0;
        const baseline = s(18) + s(CARD.compactPadNear) + s(CARD.compactPadFar);
        const msg = Protocol.measure(this.notchId, this._wireScreen, s(CARD.restDepth), compact, baseline);
        const text = JSON.stringify(msg);
        if (text === this._lastMeasure)
            return;
        this._lastMeasure = text;
        this._send(msg);
    }

    resendMeasure() {
        this._lastMeasure = '';
        this._measure();
    }

    _paint(area) {
        const cr = area.get_context();
        const [aw, ah] = area.get_surface_size();
        const shoulder = this.theme.space(CARD.shoulder);
        // The card grows with the actor while it eases: draw at the
        // actor's current size, not the target.
        const w = Math.max(0, aw - 2 * shoulder);
        const h = this._mode === 'rest' ? Math.min(ah, this._cardH) : ah;
        const paths = Outline.path({w, h, shoulder, radius: this._radius ?? 0, start: {kind: 'shoulder'}, end: {kind: 'shoulder'}});
        const c = this.theme.colors;
        cr.translate(shoulder, 0);
        trace(cr, paths.fill);
        cr.setSourceRGBA(c.background.r, c.background.g, c.background.b, 1);
        cr.fill();
        trace(cr, paths.stroke);
        cr.setSourceRGBA(c.line.r, c.line.g, c.line.b, c.line.a);
        cr.setLineWidth(1);
        cr.stroke();
        cr.$dispose();
    }

    // The keyboard while the daemon asks for it (`copy.keyboard`): a modal
    // grab, so keys reach the notch and not the windows. Escape is the
    // daemon's to handle like any key, but a grab never outlives an open
    // notch: closing drops it.
    _keyboard(want, mode) {
        const open = mode !== 'rest' && mode !== 'compact';
        if (want !== 'none' && open && !this._modal) {
            try {
                this._modal = Main.pushModal(this);
                // Under the grab a click outside the notch reaches no
                // window: it is a click away, which closes the notch as
                // clicking a window does under layer-shell.
                this._capture = global.stage.connect('captured-event', (stage, event) => {
                    if (event.type() !== Clutter.EventType.BUTTON_PRESS || this._inside(event))
                        return Clutter.EVENT_PROPAGATE;
                    this._release();
                    this._send(Protocol.pin(this.notchId, this._wireScreen, false));
                    return Clutter.EVENT_STOP;
                });
                this.grab_key_focus();
                this._send(Protocol.focus(this.notchId, this._wireScreen, true));
            } catch (e) {
                console.warn(`notchd: no keyboard grab: ${e.message}`);
                this._modal = null;
            }
        } else if ((want === 'none' || !open) && this._modal) {
            this._release();
        }
    }

    // For the test hook: what this copy shows and where, the card's target
    // size (not the eased one).
    describe() {
        return {
            mode: this._mode, screen: this._copy?.screen ?? '', monitor: this._monitorIndex,
            ...(this._target ?? {x: this.x, y: this.y, w: this.width, h: this.height}),
            card: {w: this._cardW, h: this._cardH}, modal: !!this._modal,
            hasContent: !!this._content.child,
        };
    }

    release() {
        this._release();
    }

    _release() {
        if (this._capture) {
            global.stage.disconnect(this._capture);
            this._capture = 0;
        }
        if (!this._modal)
            return;
        Main.popModal(this._modal);
        this._modal = null;
        this._send(Protocol.focus(this.notchId, this._wireScreen, false));
    }

    _inside(event) {
        const [x, y] = event.get_coords();
        const [ax, ay] = this.get_transformed_position();
        const [w, h] = this.get_transformed_size();
        return x >= ax && x < ax + w && y >= ay && y < ay + h;
    }

    // Escape closes a notch pinned open (NotchCard's Keys.onEscapePressed);
    // other keys go to the daemon, for a module's text field.
    _key(event) {
        if (!this._modal)
            return Clutter.EVENT_PROPAGATE;
        const wire = wireKey(event);
        if (!wire)
            return Clutter.EVENT_STOP;
        if (wire.key === 'Escape' && wire.mods.length === 0) {
            this._release();
            this._send(Protocol.pin(this.notchId, this._wireScreen, false));
        } else {
            this._send(Protocol.key(this.notchId, this._wireScreen, wire.key, wire.mods, wire.text));
        }
        return Clutter.EVENT_STOP;
    }

    destroy() {
        this._release();
        Main.layoutManager.removeChrome(this);
        super.destroy();
    }
});

const NAMED_KEYS = {
    [Clutter.KEY_Escape]: 'Escape', [Clutter.KEY_Tab]: 'Tab', [Clutter.KEY_ISO_Left_Tab]: 'Tab',
    [Clutter.KEY_Return]: 'Return', [Clutter.KEY_KP_Enter]: 'Return', [Clutter.KEY_BackSpace]: 'Backspace',
    [Clutter.KEY_Left]: 'Left', [Clutter.KEY_Right]: 'Right', [Clutter.KEY_Up]: 'Up', [Clutter.KEY_Down]: 'Down',
    [Clutter.KEY_space]: 'Space', [Clutter.KEY_Home]: 'Home', [Clutter.KEY_End]: 'End',
    [Clutter.KEY_Delete]: 'Delete', [Clutter.KEY_Page_Up]: 'PageUp', [Clutter.KEY_Page_Down]: 'PageDown',
};

// A Clutter key event as the wire's `key`: a normalized name, the
// modifiers and the text it types.
export function wireKey(event) {
    const sym = event.get_key_symbol();
    const state = event.get_state();
    const mods = [];
    if (state & Clutter.ModifierType.SHIFT_MASK)
        mods.push('shift');
    if (state & Clutter.ModifierType.CONTROL_MASK)
        mods.push('ctrl');
    if (state & Clutter.ModifierType.MOD1_MASK)
        mods.push('alt');
    if (state & Clutter.ModifierType.SUPER_MASK)
        mods.push('meta');
    const unicode = event.get_key_unicode();
    const text = unicode && unicode.charCodeAt(0) >= 32 ? unicode : '';
    const named = NAMED_KEYS[sym];
    if (named)
        return {key: named, mods, text: named === 'Space' ? ' ' : ''};
    if (text)
        return {key: text.toLowerCase(), mods, text};
    return null;
}
