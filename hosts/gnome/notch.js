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

// The card's motion, as core/NotchCard.qml: the length (x and width) springs
// past its target (460 ms, OutBack with an overshoot of 1.1), the depth (y
// and height) settles without overshoot (380 ms, OutQuint). New content
// fades: the old out in 70 ms, the new in over 180.
const ALONG_MS = 460;
const DEPTH_MS = 380;
const FADE_OUT_MS = 70;
const FADE_IN_MS = 180;

function outBack(t, overshoot = 1.1) {
    const u = t - 1;
    return u * u * ((overshoot + 1) * u + overshoot) + 1;
}

function outQuint(t) {
    return 1 - Math.pow(1 - t, 5);
}

function animations() {
    return St.Settings.get().enable_animations;
}

// The keys the selected notch's navigation label lists, a line each, as
// NotchCopy.qml's hint(): by the navigation's `mode`, the number of
// notches, whether it has tabs to walk and how many screens there are.
function navHint({mode, count, tabs, screens}) {
    if (mode === 'remove')
        return ['x remove   Esc keep'];
    if (mode === 'move') {
        return [['←↑↓→ edge', '⇧ slide', screens > 1 ? 'Tab screen' : ''].filter(Boolean).join('   '),
            '↵ done   Esc undo'];
    }
    return [[count > 1 ? `1-${Math.min(9, count)} ←→` : '', tabs ? 'Tab' : '', '↵ open', 'Esc'].filter(Boolean).join('   '),
        ['m move', 's settings', count > 1 ? 'x remove' : ''].filter(Boolean).join('   ')];
}

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
        // Clipped: while the card grows, its content never shows outside it.
        super._init({name: 'notchd-copy', reactive: true, track_hover: true, can_focus: true, x: 0, y: 0, clip_to_allocation: true});
        this._host = host;
        this.notchId = notchId;
        this._mode = 'rest';
        // The mode the content on screen belongs to: it trails `_mode` by
        // the fade, and the card's shape follows it.
        this._shown = 'rest';
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
        // Centered along the card as it springs, as the QML content is.
        this._content.add_constraint(new Clutter.AlignConstraint({source: this, align_axis: Clutter.AlignAxis.X_AXIS, factor: 0.5}));
        this.add_child(this._content);
        this.connect('notify::width', () => this._bg.set_size(this.width, this.height));
        this.connect('notify::height', () => this._bg.set_size(this.width, this.height));

        this.connect('notify::hover', () => this._send(Protocol.pointer(this.notchId, this._pointerScreen(this.hover ? 'enter' : 'leave'), this.hover ? 'enter' : 'leave')));
        this.connect('button-press-event', (actor, event) => {
            // A press on a button inside is the button's: from GNOME 49 an
            // St.Button lets it bubble here, and pinning would take a grab
            // that cancels the button's click.
            if (this._onButton(event))
                return Clutter.EVENT_PROPAGATE;
            this._send(Protocol.pointer(this.notchId, this._pointerScreen('press'), 'press'));
            return Clutter.EVENT_STOP;
        });
        this.connect('key-press-event', (actor, event) => this._key(event));

        Main.layoutManager.addTopChrome(this, {trackFullscreen: true});
    }

    // A destroyed copy says nothing: a fade it had running stops with it
    // and would lay out and measure again, over the new copy's facts.
    _send(msg) {
        if (!this._destroyed)
            this._host.send(msg);
    }

    // Whether `event` happened on an St.Button inside this card (a tab, a
    // view's button or chip).
    _onButton(event) {
        let a = global.stage.get_event_actor?.(event) ?? event.get_source?.() ?? null;
        for (; a && a !== this; a = a.get_parent()) {
            if (a instanceof St.Button)
                return true;
        }
        return false;
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
    // `highlight`: while navigating, what this notch's highlight says
    // ({number, selected, mode, count, screens}), else null.
    update(notch, copy, highlight = null) {
        this._wireScreen = notch.screen ?? '';
        this._monitorIndex = this._host.monitorFor(copy.screen);
        this._notch = notch;
        this._copy = copy;
        const mode = copy.mode;
        const parts = this._parts(copy);
        const tabs = this._tabs(copy);
        const key = JSON.stringify([mode, parts, tabs]);
        if (key !== this._viewKey) {
            this._viewKey = key;
            // The same parts, tabs and theme as last time: the parts' actors
            // are updated in place; otherwise drawn again.
            const shape = JSON.stringify([mode, parts.map(([module, slot]) => [module, slot]), tabs, this.theme.generation]);
            if (this._pending || shape !== this._shape || !this._updateParts(parts))
                this._replace(mode, parts, tabs);
            this._shape = shape;
        }
        this._mode = mode;
        if (!this._pending)
            this._layout(true);
        this._keyboard(copy.keyboard ?? 'none', mode);
        const nav = highlight ? {...highlight, tabs: (copy.tabs?.length ?? 0) > 1} : null;
        const navKey = JSON.stringify([nav, this.theme.generation]);
        if (navKey !== this._navKey) {
            this._navKey = navKey;
            this._nav = nav;
            this._drawNavLabel();
            this._bg.queue_repaint();
        }
    }

    // ------------------------------------------------------------ navigation
    // While navigating, as NotchCard.qml: the outline in the accent for the
    // selected notch and muted for the others, and a small label at the
    // card's top right, outside it: its number and name, the selected one's
    // with the keys under it, in the danger color when asking to remove.
    // Outside the clipped card, so an actor of its own; it takes no clicks.
    _drawNavLabel() {
        this._navLabel?.destroy();
        this._navLabel = null;
        const nav = this._nav;
        if (!nav)
            return;
        const theme = this.theme;
        const removing = nav.selected && nav.mode === 'remove';
        const border = removing ? 'danger' : nav.selected ? 'accent' : 'line';
        const label = new St.BoxLayout({
            name: 'notchd-nav-label', reactive: false,
            style: `background-color: ${theme.css('background')}; border: 1px solid ${theme.css(border)};` +
                ` border-radius: ${theme.radiusSmall}px; padding: 3px 6px; spacing: 1px;`,
        });
        if ('orientation' in label)
            label.orientation = Clutter.Orientation.VERTICAL;
        else
            label.vertical = true;
        const top = new St.BoxLayout({style: 'spacing: 5px;'});
        const caption = theme.fonts.caption;
        if (nav.number > 0)
            top.add_child(new St.Label({text: String(nav.number), style: `color: ${theme.css('accent')}; font-size: ${caption}px; font-weight: bold;`}));
        const name = removing ? `remove ${this.notchId}?` : this.notchId;
        const tone = removing ? 'danger' : nav.selected ? 'foreground' : 'muted';
        top.add_child(new St.Label({text: name, style: `color: ${theme.css(tone)}; font-size: ${caption}px;`}));
        label.add_child(top);
        for (const line of nav.selected ? navHint(nav) : [])
            label.add_child(new St.Label({text: line, style: `color: ${theme.css('faint')}; font-size: ${caption - 1}px;`}));
        Main.layoutManager.uiGroup.add_child(label);
        this._navLabel = label;
        this._placeNavLabel();
    }

    // Right of the card past its shoulder, at its top, or left of it where
    // the right has no room; kept on the monitor.
    _placeNavLabel() {
        const label = this._navLabel;
        const t = this._target;
        if (!label || !t)
            return;
        const monitor = Main.layoutManager.monitors[this._monitorIndex] ?? Main.layoutManager.primaryMonitor;
        const gap = this.theme.space(6);
        const [, w] = label.get_preferred_width(-1);
        const [, h] = label.get_preferred_height(w);
        let x = t.x + t.w + gap;
        if (x + w > monitor.x + monitor.width)
            x = t.x - gap - w;
        x = Math.max(monitor.x, Math.min(x, monitor.x + monitor.width - w));
        const y = Math.max(monitor.y, Math.min(t.y + gap, monitor.y + monitor.height - h));
        label.set_position(Math.round(x), Math.round(y));
        Main.layoutManager.uiGroup.set_child_above_sibling(label, null);
    }

    // New content: the old fades out, then the new is drawn, the card
    // springs to it and it fades in. Without animations, at once.
    _replace(mode, parts, tabs) {
        const fading = !!this._pending;
        this._pending = {mode, parts, tabs};
        if (fading)
            return;
        if (animations() && this._content.child && this._content.opacity > 0) {
            this._content.ease({
                opacity: 0, duration: FADE_OUT_MS, mode: Clutter.AnimationMode.LINEAR,
                onStopped: () => this._swap(true),
            });
        } else {
            this._swap(false);
        }
    }

    _swap(fade) {
        const pending = this._pending;
        if (!pending || this._destroyed)
            return;
        this._pending = null;
        // Destroyed, not only unparented: a primitive's timers end with its
        // actor.
        const old = this._content.child;
        this._content.child = this._renderParts(pending.mode, pending.parts, pending.tabs);
        old?.destroy();
        this._shown = pending.mode;
        this._layout(true);
        const shown = this._shown !== 'rest' && !!this._content.child;
        if (fade && shown) {
            this._content.opacity = 0;
            this._content.ease({opacity: 255, duration: FADE_IN_MS, mode: Clutter.AnimationMode.LINEAR});
        } else {
            this._content.opacity = shown ? 255 : 0;
        }
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

    // The open notch's tabs (`copy.tabs`, none for a notch without): each
    // module's title, whether it is the one shown, its `attention`.
    _tabs(copy) {
        if (copy.mode !== 'expanded')
            return [];
        const modules = this._host.store().modules;
        const shown = copy.shown ?? [];
        return (copy.tabs ?? []).map(id => ({
            id, title: modules[id]?.title || id, picked: shown.includes(id), attention: !!modules[id]?.attention,
        }));
    }

    // A tab per module, as NotchCard.qml's tab row: the title, the picked
    // one on the `selected` fill, the others muted with a hover fill, an
    // accent dot for news not seen. A press sends `tab`; the daemon picks.
    _tabRow(tabs) {
        const theme = this.theme;
        const row = new St.BoxLayout({style: 'spacing: 4px;', x_align: Clutter.ActorAlign.START});
        this._tabButtons = [];
        for (const tab of tabs) {
            const look = hover => {
                const fill = tab.picked ? theme.css('selected') : hover ? theme.css('hover') : 'transparent';
                return `background-color: ${fill}; padding: 5px 12px; border-radius: ${theme.radiusSmall}px;`;
            };
            const button = new St.Button({reactive: true, can_focus: true, track_hover: true, style: look(false)});
            button.connect('notify::hover', () => {
                button.style = look(button.hover);
            });
            button.connect('clicked', () => this._send(Protocol.tab(this.notchId, this._wireScreen, tab.id)));
            const inner = new St.Widget({layout_manager: new Clutter.BinLayout()});
            inner.add_child(new St.Label({
                text: tab.title,
                style: `color: ${theme.css(tab.picked ? 'foreground' : 'muted')}; font-size: ${theme.fonts.body}px;`,
                x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
            }));
            if (tab.attention) {
                inner.add_child(new St.Widget({
                    style: `background-color: ${theme.css('accent')}; width: 6px; height: 6px; border-radius: 3px; margin: -1px -8px 0 0;`,
                    x_align: Clutter.ActorAlign.END, y_align: Clutter.ActorAlign.START,
                }));
            }
            button.set_child(inner);
            row.add_child(button);
            this._tabButtons.push({tab, button});
        }
        return row;
    }

    // Each part drawn with its own module and slot, so a button fires the
    // event of the module it belongs to; with tabs, below their row.
    _renderParts(mode, parts, tabs = []) {
        this._partCtx = parts.map(([module, slot]) => ({
            theme: this.theme,
            onAction: (action, value) => this._send(Protocol.event(module, slot, action, value)),
        }));
        this._partActors = parts.map(([, , tree], i) => render(tree, this._partCtx[i]));
        this._partBox = null;
        this._partHolder = null;
        this._tabButtons = [];
        const area = this._partsArea(mode, this._partActors.filter(a => a));
        if (tabs.length === 0)
            return area;
        const column = new St.BoxLayout({style: 'spacing: 10px;'});
        if ('orientation' in column)
            column.orientation = Clutter.Orientation.VERTICAL;
        else
            column.vertical = true;
        column.add_child(this._tabRow(tabs));
        if (area)
            column.add_child(area);
        if (!this._partBox)
            this._partHolder = column;
        return column;
    }

    // The parts' actors side by side (closed) or stacked (open): one alone
    // as it is, several in a box.
    _partsArea(mode, actors) {
        if (actors.length === 0)
            return null;
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
        this._partBox = box;
        return box;
    }

    // New trees for the parts drawn last time: each actor updated in place
    // where it can, else replaced where it was. False when a part appears
    // or goes (a tree now invisible, or visible again): drawn again then.
    _updateParts(parts) {
        for (let i = 0; i < parts.length; i++) {
            const prev = this._partActors[i];
            const actor = render(parts[i][2], this._partCtx[i], 0, prev);
            if (actor === prev)
                continue;
            if (!prev || !actor)
                return false;
            this._partActors[i] = actor;
            const parent = this._partBox ?? this._partHolder;
            if (parent) {
                actor.y_align = prev.y_align;
                parent.replace_child(prev, actor);
            } else {
                this._content.child = actor;
            }
            prev.destroy();
        }
        return true;
    }

    // The open notch is as wide as the widest module it shows asks for
    // (`module.expandedWidth`, design pixels).
    _expandedWidth() {
        const modules = this._host.store().modules;
        const widths = (this._copy?.shown ?? []).map(id => Number(modules[id]?.expandedWidth) || 0);
        return Math.max(CARD.expandedWidth, ...widths);
    }

    _pads() {
        const m = this._shown;
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
        if (this._shown === 'rest' || !child) {
            w = s(CARD.restLength);
            h = s(CARD.restDepth);
        } else {
            const [, natW] = child.get_preferred_width(-1);
            const maxW = this._shown === 'expanded' ? s(this._expandedWidth()) : monitor.width / 2;
            const innerW = this._shown === 'expanded' ? maxW : Math.min(natW, maxW);
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
        const height = Math.max(h, this._shown === 'rest' ? s(CARD.hitDepth) : h);
        this._content.y = pads.near;
        this._moveTo({x, y, w: total, h: height}, animate);
        this._bg.queue_repaint();
        this._placeNavLabel();
        this._measure();
    }

    // The card to `to` ({x, y, w, h}): sprung as NotchCard.qml does, or at
    // once. A motion toward the same target goes on undisturbed.
    _moveTo(to, animate) {
        this._target = to;
        const same = m => m && m.x === to.x && m.y === to.y && m.w === to.w && m.h === to.h;
        if (this._motion && same(this._motion.to))
            return;
        this._motion?.timeline.stop();
        this._motion = null;
        if (!animate || !animations() || !this.mapped) {
            this.set_position(to.x, to.y);
            this.set_size(to.w, to.h);
            return;
        }
        const from = {x: this.x, y: this.y, w: this.width, h: this.height};
        const lerp = (a, b, t) => a + (b - a) * t;
        const timeline = new Clutter.Timeline({actor: this, duration: ALONG_MS});
        timeline.connect('new-frame', () => {
            const ms = timeline.get_elapsed_time();
            const along = outBack(Math.min(1, ms / ALONG_MS));
            const depth = outQuint(Math.min(1, ms / DEPTH_MS));
            this.set_position(Math.round(lerp(from.x, to.x, along)), Math.round(lerp(from.y, to.y, depth)));
            this.set_size(Math.max(0, Math.round(lerp(from.w, to.w, along))), Math.max(0, Math.round(lerp(from.h, to.h, depth))));
        });
        timeline.connect('completed', () => {
            this.set_position(to.x, to.y);
            this.set_size(to.w, to.h);
            this._motion = null;
        });
        this._motion = {to, timeline};
        timeline.start();
    }

    // What this copy measures of itself, for the daemon's reserve.
    _measure() {
        const s = v => this.theme.space(v);
        const compact = this._shown === 'compact' ? this._cardH : 0;
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
        const h = this._shown === 'rest' ? Math.min(ah, this._cardH) : ah;
        const paths = Outline.path({w, h, shoulder, radius: this._radius ?? 0, start: {kind: 'shoulder'}, end: {kind: 'shoulder'}});
        const c = this.theme.colors;
        cr.translate(shoulder, 0);
        trace(cr, paths.fill);
        cr.setSourceRGBA(c.background.r, c.background.g, c.background.b, 1);
        cr.fill();
        trace(cr, paths.stroke);
        const stroke = this._nav?.selected ? c.accent : this._nav ? c.muted : c.line;
        cr.setSourceRGBA(stroke.r, stroke.g, stroke.b, stroke.a);
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
            // The mode drawn, which the geometry below belongs to; `wanted`,
            // the daemon's, leads it by a fade.
            mode: this._shown, wanted: this._mode, screen: this._copy?.screen ?? '', monitor: this._monitorIndex,
            ...(this._target ?? {x: this.x, y: this.y, w: this.width, h: this.height}),
            card: {w: this._cardW, h: this._cardH}, modal: !!this._modal,
            hasContent: !!this._content.child, shown: this._copy?.shown ?? [],
            navLabel: this._navLabel ? this._navLabel.get_children().flatMap(c => (c instanceof St.Label ? [c.text] : c.get_children().map(l => l.text))) : null,
            // The view's progress bars and pictures (a test presses a bar
            // where it is and reads what a picture shows).
            bars: this._viewActors(a => a._track).map(a => ({...this._boxOf(a._track), seek: !!a.reactive, shown: a._shownFraction ?? null})),
            texts: this._viewActors(a => a instanceof St.Label).map(a => a.text),
            images: this._viewActors(a => a._picture).map(a => ({...a._picture, ...this._boxOf(a)})),
            tabs: (this._tabButtons ?? []).filter(({button}) => button.mapped).map(({tab, button}) => {
                // Allocations summed up to the card: the transformed extents
                // are NaN in GNOME 46's headless shell.
                let x = this.x, y = this.y;
                for (let a = button; a && a !== this; a = a.get_parent()) {
                    const box = a.get_allocation_box();
                    x += box.x1;
                    y += box.y1;
                }
                return {id: tab.id, picked: tab.picked, x, y, w: button.width, h: button.height};
            }),
        };
    }

    // The actors under the content `pick` accepts, and where one is: its
    // allocations summed up to the card (transformed extents are NaN in
    // GNOME 46's headless shell).
    _viewActors(pick) {
        const found = [];
        const walk = a => {
            if (pick(a))
                found.push(a);
            a.get_children().forEach(walk);
        };
        if (this._content)
            walk(this._content);
        return found;
    }

    _boxOf(actor) {
        let x = this.x, y = this.y;
        for (let a = actor; a && a !== this; a = a.get_parent()) {
            const box = a.get_allocation_box();
            x += box.x1;
            y += box.y1;
        }
        const b = actor.get_allocation_box();
        return {x, y, w: b.x2 - b.x1, h: b.y2 - b.y1};
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
        this._pending = null;
        this._motion?.timeline.stop();
        this._motion = null;
        this._navLabel?.destroy();
        this._navLabel = null;
        // Its last word: the keyboard it held is let go.
        this._release();
        this._destroyed = true;
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
