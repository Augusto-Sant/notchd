// One notch copy on one monitor: the card (its outline drawn with Cairo from
// lib/Outline.js, the same path the QML card draws) and the module views in
// it. It draws the mode the daemon's `state` gives the copy and reports raw
// facts back: the pointer, a press, the keyboard, its measured depths. It
// decides nothing.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import * as Outline from './lib/Outline.js';
import * as Position from './lib/Position.js';
import * as Protocol from './lib/Protocol.js';
import {placeCard} from './layout.js';
import {render} from './view.js';

// The card's motion, as core/NotchCard.qml: the length (x and width) springs
// past its target (460 ms, OutBack with an overshoot of 1.1), the depth (y
// and height) settles without overshoot (380 ms, OutQuint). New content
// fades: the old out in 70 ms, the new in over 180.
const ALONG_MS = 460;
const DEPTH_MS = 380;
const FADE_OUT_MS = 70;
const FADE_IN_MS = 180;
// Held this long, a press picks the card up (NotchCard.qml's holdToDrag);
// moving farther than DRAG_SLOP first makes it a click.
const HOLD_MS = 250;
const DRAG_SLOP = 8;

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

// What a side edge's compact view is for a module without a
// `verticalCompact`: a dot (NotchCard.qml's dotView).
const DOT = 'dot';

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
    // `guest`: the visitor or a standalone module's own notch, unseen while
    // closed (Notch.qml's `guest`).
    _init(host, notchId, {guest = false} = {}) {
        // Clipped: while the card grows, its content never shows outside it.
        super._init({name: 'notchd-copy', reactive: true, can_focus: true, x: 0, y: 0, clip_to_allocation: true});
        this._host = host;
        this.notchId = notchId;
        this.guest = guest;
        this._covered = false;
        this._mode = 'rest';
        // The mode the content on screen belongs to: it trails `_mode` by
        // the fade, and the card's shape follows it.
        this._shown = 'rest';
        this._wireScreen = '';
        this._monitorIndex = 0;
        // The edge the card hangs from (the daemon's, or a drag's preview),
        // and whether the content on screen was laid out for a side edge.
        this._edge = 'top';
        this._shownVertical = false;
        this._shownVerticalPeek = false;
        // While dragged: where the card would land ({edge, offset, monitor,
        // screen}); after the drop, until the daemon's state has it.
        this._drag = null;
        this._modal = null;
        this._viewKey = '';
        this._cardAlong = 0;
        this._cardDepth = 0;
        this._closedDepth = 0;
        this._rowDepth = 0;
        this._contentShift = 0;
        this._cornerInsets = {start: 0, end: 0};
        this._startSide = {kind: 'shoulder'};
        this._endSide = {kind: 'shoulder'};
        this._lastMeasure = '';

        this._bg = new St.DrawingArea({x: 0, y: 0});
        this._bg.connect('repaint', area => this._paint(area));
        this.add_child(this._bg);
        this._hits = [new St.Widget({reactive: true, x: 0, y: 0})];
        this._hits.forEach(hit => this.add_child(hit));
        // What takes the keyboard and the grabs: a non-reactive card gets no
        // events of its own.
        this._keys = new St.Widget({reactive: true, can_focus: true, x: 0, y: 0, width: 1, height: 1});
        this.add_child(this._keys);
        this._content = new St.Bin({x: 0, y: 0, opacity: 0});
        this.add_child(this._content);
        // The content stays against the edge and centered along it as the
        // card springs, as the QML content does.
        const resized = () => {
            this._bg.set_size(this.width, this.height);
            this._layoutHits();
            this._placeContent();
        };
        this.connect('notify::width', resized);
        this.connect('notify::height', resized);

        // The pointer and presses reach the card through its hit areas; a
        // button of a view takes its own (it is no hit area's child), and
        // the keyboard and the drag's grab go to `_keys`.
        for (const hit of [...this._hits, this._keys]) {
            hit.connect('enter-event', () => this._hoverChanged());
            hit.connect('leave-event', () => this._hoverChanged());
            hit.connect('button-press-event', (actor, event) => {
                this._pressed(event);
                return Clutter.EVENT_STOP;
            });
            hit.connect('motion-event', (actor, event) => this._moved(event));
            hit.connect('button-release-event', (actor, event) => this._released(event));
        }
        this._keys.connect('key-press-event', (actor, event) => this._key(event));
        // The card's own: under its modal grab a press outside it comes
        // here (the click away), and from GNOME 49 a view's button lets its
        // press bubble here, which is the button's, not the card's.
        this.connect('button-press-event', (actor, event) => {
            if (this._onButton(event))
                return Clutter.EVENT_PROPAGATE;
            this._pressed(event);
            return Clutter.EVENT_STOP;
        });
        this.connect('motion-event', (actor, event) => this._moved(event));
        this.connect('button-release-event', (actor, event) => this._released(event));

        Main.layoutManager.addTopChrome(this, {trackFullscreen: true});
        // A guest starts unseen: it shows when it peeks.
        if (guest) {
            this._seen = false;
            this.opacity = 0;
            this._setReactive(false);
            this.hide();
        }
    }

    // The pointer is on the notch while it is on the card or its terminal:
    // one `enter` and one `leave` for both. Not while dragged: the card jumps under the pointer and
    // away from it, and each enter and leave made the daemon send a state.
    _hoverChanged() {
        if (this._dragging)
            return;
        const on = this._pointerOnCard() || !!this._termHover;
        // While on it, looked at again every 100 ms: the pointer may leave
        // over a view's button, which tells the hit areas nothing.
        if (on && !this._hoverPoll) {
            this._hoverPoll = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
                if (this._pointerOnCard() || this._termHover)
                    return GLib.SOURCE_CONTINUE;
                this._hoverPoll = 0;
                this._hoverChanged();
                return GLib.SOURCE_REMOVE;
            });
        }
        if (on === this._sentHover)
            return;
        this._sentHover = on;
        const ev = on ? 'enter' : 'leave';
        this._send(Protocol.pointer(this.notchId, this._pointerScreen(ev), ev));
    }

    // The pointer on the visible card, within its rectangle.
    _pointerOnCard() {
        if (!this.visible || this.opacity === 0 || this._hits.every(h => !h.reactive))
            return false;
        const [px, py] = global.get_pointer();
        return px >= this.x && px < this.x + this.width && py >= this.y && py < this.y + this.height;
    }

    // ------------------------------------------------------------- terminal
    // The view's terminal surface, as the open notch shows it: the frame
    // view.js drew (`_surface`), or null.
    _terminalActor() {
        if (this._shown !== 'expanded' || this._pending || !this._host.terminals)
            return null;
        return this._viewActors(a => a._surface)[0] ?? null;
    }

    // The current terminal actor is already inside the view. Give it focus
    // when the view becomes visible under the notch's keyboard grab.
    _placeTerminal() {
        this._termRef = this._terminalActor()?._surface.ref ?? null;
        if (this._modal) {
            const terminal = this._terminalActor()?._terminal;
            terminal?.focusInput();
        }
    }

    _hideTerminal() {
        this._termRef = null;
        this._termHover = false;
        this._hoverChanged();
    }

    // Forward the terminal actor's actions to its module.
    onLine(line) {
        if (line === 'pointer in' || line === 'pointer out') {
            this._termHover = line === 'pointer in';
            this._hoverChanged();
            return;
        }
        const module = this._termRef?.replace(/^term:/, '');
        if (module && ['typed', 'leave', 'keyboard', 'restart'].includes(line))
            this._send(Protocol.event(module, 'expanded', line, null));
    }

    // The hit area covers the card; the terminal actor takes events within
    // its frame as a child of the content.
    _layoutHits() {
        const boxes = [[0, 0, this.width, this.height]];
        this._hits.forEach((hit, i) => {
            const [x1, y1, x2, y2] = boxes[i] ?? [0, 0, 0, 0];
            hit.set_position(x1, y1);
            hit.set_size(Math.max(0, x2 - x1), Math.max(0, y2 - y1));
            hit.visible = x2 > x1 && y2 > y1;
        });
    }

    // Whether the card takes the pointer at all (an unseen guest does not).
    _setReactive(on) {
        this._hits.forEach(hit => {
            hit.reactive = on;
        });
    }

    _redrawCard() {
        this._bg.queue_repaint();
        this._layoutHits();
        this.reactive = true;
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

    // A destroyed copy says nothing: a fade it had running stops with it
    // and would lay out and measure again, over the new copy's facts.
    _send(msg) {
        if (!this._destroyed)
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
    // `highlight`: while navigating, what this notch's highlight says
    // ({number, selected, mode, count, screens}), else null.
    update(notch, copy, highlight = null) {
        // A guest's copies are one per screen, each named by its screen.
        this._wireScreen = this.guest ? copy.screen ?? '' : notch.screen ?? '';
        this._notch = notch;
        this._copy = copy;
        this._navigating = !!highlight;
        this._settleDrop(notch);
        this._sync();
        this._showOrConceal();
        this._keyboard(copy.keyboard ?? 'none', copy.mode);
        const nav = highlight ? {...highlight, tabs: (copy.tabs?.length ?? 0) > 1} : null;
        const navKey = JSON.stringify([nav, this.theme.generation]);
        if (navKey !== this._navKey) {
            this._navKey = navKey;
            this._nav = nav;
            this._drawNavLabel();
            this._redrawCard();
        }
    }

    // The view and the place for the copy's state, or for a drag's preview.
    _sync() {
        const copy = this._copy;
        if (!copy)
            return;
        this._monitorIndex = this._drag?.monitor ?? this._host.monitorFor(copy.screen);
        this._edge = this._drag?.edge ?? this._notch?.edge ?? 'top';
        const vertical = Position.isVertical(this._edge);
        const mode = copy.mode;
        const parts = this._parts(copy, vertical);
        const tabs = this._tabs(copy);
        // A theme change (colors, text scaling) draws the view again too.
        const key = JSON.stringify([mode, vertical, parts, tabs, this.theme.generation]);
        if (key !== this._viewKey) {
            this._viewKey = key;
            // The same parts, tabs and theme as last time: the parts' actors
            // are updated in place; otherwise drawn again.
            const shape = JSON.stringify([mode, vertical, parts.map(([module, slot]) => [module, slot]), tabs, this.theme.generation]);
            if (this._pending || shape !== this._shape || !this._updateParts(parts))
                this._replace(mode, parts, tabs, vertical);
            this._shape = shape;
        }
        this._mode = mode;
        // Dragged, the card follows the pointer at once (only its size
        // springs in NotchCard.qml): a spring restarted at every motion
        // lagged behind it.
        if (!this._pending)
            this._layout(!this._dragging);
    }

    // ---------------------------------------------------------- concealment
    // Whether the card shows: a guest only while it peeks or is open (and
    // through its fade out), a notch of the config unless a shown guest
    // covers it on its edge and it is not open itself (NotchWindow.qml's
    // `concealed`). Faded in 160 ms, as the QML; unseen, it takes no input.
    get open() {
        return this._mode === 'peek' || this._mode === 'expanded';
    }

    get hidden() {
        return !this.open && !this._navigating &&
            (!!this._copy?.covered || !!global.display.get_monitor_in_fullscreen(this._monitorIndex));
    }

    // Seen, or fading out (a guest whose peek just ended).
    get seenOrFading() {
        return !!this._seen || this.opacity > 0;
    }

    // Where the card is going: its monitor, edge and rectangle.
    get place() {
        const t = this._target ?? {x: this.x, y: this.y, w: this.width, h: this.height};
        return {monitor: this._monitorIndex, edge: this._edge, ...t};
    }

    _effectiveInsets(monitor) {
        const insets = this._insets(monitor);
        return Position.isVertical(this._edge)
            ? {...insets, top: insets.top + this._cornerInsets.start,
                bottom: insets.bottom + this._cornerInsets.end}
            : insets;
    }

    _positionFor(offset, monitor, insets) {
        const shoulder = this.theme.space(CARD.shoulder);
        return placeCard(Position, {edge: this._edge, offset, monitor, insets,
            along: this._cardAlong, depth: this._hitDepth, shoulder,
            snap: shoulder + this.theme.space(4)});
    }

    get packItem() {
        const monitor = Main.layoutManager.monitors[this._monitorIndex];
        if (!monitor || !this._cardAlong)
            return null;
        const stretch = Position.stretch({edge: this._edge, width: monitor.width,
            height: monitor.height, insets: this._effectiveInsets(monitor)});
        const offset = this._drag?.offset ?? this._notch?.offset ?? 0.5;
        return {group: `${this._monitorIndex}|${this._edge}`,
            center: stretch.start + offset * (stretch.end - stretch.start),
            size: this._cardAlong, start: stretch.start, end: stretch.end,
            snap: this.theme.space(CARD.shoulder + 4), open: this.open,
            skip: this.guest || !!this._dragging || this.hidden};
    }

    get cornerItem() {
        const spot = this._plannedSpot ?? this._naturalSpot ?? {};
        return {screen: this._monitorIndex, edge: this._edge,
            atStart: !!spot.atStart, atEnd: !!spot.atEnd, depth: this._cardDepth,
            shown: !this.guest && !this.hidden && (!this._covered || this.open) && !this._dragging};
    }

    setCornerInsets(insets) {
        this._cornerInsets = insets;
    }

    get closedDepth() { return this._closedDepth; }

    setRowDepth(depth) {
        if (this._rowDepth === depth)
            return false;
        this._rowDepth = depth;
        this._layout(true);
        return true;
    }

    planPacking(packed) {
        const monitor = Main.layoutManager.monitors[this._monitorIndex];
        const item = this.packItem;
        if (!monitor || !item) return;
        const span = item.end - item.start;
        const offset = span > 0 ? (item.center + packed.shift - item.start) / span : 0.5;
        const {spot, target} = this._positionFor(offset, monitor, this._effectiveInsets(monitor));
        this._plannedSpot = spot;
        this._plannedTarget = target;
        this._packed = packed;
    }

    applyPacking(startNeighbor, endNeighbor, span) {
        if (!this._plannedTarget) return;
        const spot = this._plannedSpot;
        this._startSide = startNeighbor ? {kind: 'joined', depth: startNeighbor._cardDepth}
            : spot.atStart ? {kind: 'corner', depth: span.start} : {kind: 'shoulder'};
        this._endSide = endNeighbor ? {kind: 'joined', depth: endNeighbor._cardDepth}
            : spot.atEnd ? {kind: 'corner', depth: span.end} : {kind: 'shoulder'};
        this._moveTo(this._plannedTarget, !this._dragging);
        this._redrawCard();
    }

    setCovered(covered) {
        if (covered === this._covered)
            return;
        this._covered = covered;
        this._showOrConceal();
        this._host.requestLayout?.();
    }

    _showOrConceal() {
        const seen = this.guest ? this.open || this._shown === 'peek' || this._shown === 'expanded'
            : !this.hidden && (!this._covered || this.open);
        if (seen === this._seen)
            return;
        this._seen = seen;
        this.remove_transition('opacity');
        if (seen) {
            this.show();
            this._setReactive(true);
            this.ease({opacity: 255, duration: animations() ? 160 : 0, mode: Clutter.AnimationMode.LINEAR});
        } else {
            this._setReactive(false);
            this.ease({
                opacity: 0, duration: animations() && this.mapped ? 160 : 0, mode: Clutter.AnimationMode.LINEAR,
                onComplete: () => {
                    if (!this._seen)
                        this.hide();
                },
            });
        }
    }

    // ------------------------------------------------------------------ drag
    // As NotchCard.qml's pressArea: a click toggles the pin (sent on the
    // release, as a click is); held for HOLD_MS without moving, a press of
    // a draggable notch picks it up. Dragged, the card jumps to the edge
    // nearest the pointer, its middle under it, on the monitor the pointer
    // is on (Position.dragTarget, screenAt), and the release drops it
    // there: the daemon stores the place. The motion is this copy's own.
    _pressed(event) {
        this._cancelHold();
        const [x, y] = event.get_coords();
        this._press = {x, y};
        if (this._notch?.draggable) {
            this._holdId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, HOLD_MS, () => {
                this._holdId = 0;
                this._startDrag();
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    _moved(event) {
        const [x, y] = event.get_coords();
        if (this._dragging) {
            this._dragTo(x, y);
            return Clutter.EVENT_STOP;
        }
        if (this._press && Math.hypot(x - this._press.x, y - this._press.y) > DRAG_SLOP)
            this._cancelHold();
        return Clutter.EVENT_PROPAGATE;
    }

    _released(event) {
        this._cancelHold();
        if (this._dragging) {
            this._endDrag(true);
            return Clutter.EVENT_STOP;
        }
        if (!this._press)
            return Clutter.EVENT_PROPAGATE;
        this._press = null;
        if (event.get_button() === Clutter.BUTTON_PRIMARY)
            this._send(Protocol.pointer(this.notchId, this._pointerScreen('press'), 'press'));
        return Clutter.EVENT_STOP;
    }

    _cancelHold() {
        if (this._holdId)
            GLib.source_remove(this._holdId);
        this._holdId = 0;
    }

    _startDrag() {
        if (this._destroyed || !this._press)
            return;
        this._press = null;
        this._dragging = true;
        this._dropped = null;
        // The pointer stays with the card while the button is down, over
        // any monitor.
        this._dragGrab = global.stage.grab(this._keys);
        this._send(Protocol.drag(this.notchId, this._wireScreen, 'start'));
        const [x, y] = global.get_pointer();
        this._dragTo(x, y);
    }

    // The monitors as Position.js's screens: name and rectangle.
    _screenRects() {
        const names = this._host.names();
        return Main.layoutManager.monitors.map((m, i) => ({
            name: names[i] ?? `monitor-${i}`, index: i, x: m.x, y: m.y, width: m.width, height: m.height,
        }));
    }

    _dragTo(x, y) {
        const screens = this._screenRects();
        const name = Position.screenAt(screens, x, y);
        const screen = screens.find(r => r.name === name);
        if (!screen)
            return;
        const monitor = Main.layoutManager.monitors[screen.index];
        const place = Position.dragTarget({
            x: x - screen.x, y: y - screen.y, width: screen.width, height: screen.height,
            insets: this._insets(monitor), magnet: this.theme.space(24),
        });
        const drag = {edge: place.edge, offset: place.offset, monitor: screen.index, screen: name};
        const was = this._drag;
        if (was && was.edge === drag.edge && was.offset === drag.offset && was.monitor === drag.monitor)
            return;
        this._drag = drag;
        // Along the same edge of the same monitor only the place changes:
        // moved there, with no new view (a view key per motion stringified
        // every tree).
        if (was && was.edge === drag.edge && was.monitor === drag.monitor && !this._pending)
            this._layout(false);
        else
            this._sync();
    }

    // Dropped (`drop`) or let go of: the grab goes, and the daemon is told
    // where it landed, or that it did not move. A notch on every screen
    // takes only the edge and offset; with one screen there is no screen to
    // pin it to.
    _endDrag(drop) {
        this._dragging = false;
        this._dragGrab?.dismiss();
        this._dragGrab = null;
        // The hover it kept quiet about while dragged.
        this._hoverChanged();
        const drag = this._drag;
        if (!drop || !drag) {
            this._drag = null;
            this._send(Protocol.drag(this.notchId, this._wireScreen, 'cancel'));
            this._sync();
            return;
        }
        const screen = this._notch?.screen === Position.ALL_SCREENS ? Position.ALL_SCREENS
            : Main.layoutManager.monitors.length > 1 ? drag.screen : '';
        this._dropped = {edge: drag.edge, offset: drag.offset};
        this._send(Protocol.drag(this.notchId, this._wireScreen, 'drop', {edge: drag.edge, offset: drag.offset, screen}));
        // Held where it was dropped until the daemon's state has it there,
        // so it does not spring back first; never longer than a second.
        this._dropTimeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
            this._dropTimeout = 0;
            this._dropped = null;
            this._drag = null;
            this._sync();
            return GLib.SOURCE_REMOVE;
        });
    }

    // The daemon's state has the notch where it was dropped: the preview
    // is no longer needed.
    _settleDrop(notch) {
        const d = this._dropped;
        if (!d || notch.edge !== d.edge || Math.abs((notch.offset ?? 0.5) - d.offset) > 1e-6)
            return;
        this._dropped = null;
        this._drag = null;
        if (this._dropTimeout)
            GLib.source_remove(this._dropTimeout);
        this._dropTimeout = 0;
    }

    // What the top panel and a dock take of a monitor (insets.js): the
    // notch hangs past them and runs beside them (Position.place's
    // `insets`).
    _insets(monitor) {
        return this._host.insets(monitor);
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
    // the right has no room; on the right edge, left of it, the open side.
    // Kept on the monitor.
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
        if (this._edge === 'right' || (this._edge !== 'left' && x + w > monitor.x + monitor.width))
            x = t.x - gap - w;
        x = Math.max(monitor.x, Math.min(x, monitor.x + monitor.width - w));
        const wishY = this._edge === 'top' ? t.y + gap : t.y;
        const y = Math.max(monitor.y, Math.min(wishY, monitor.y + monitor.height - h));
        label.set_position(Math.round(x), Math.round(y));
        Main.layoutManager.uiGroup.set_child_above_sibling(label, null);
    }

    // New content: the old fades out, then the new is drawn, the card
    // springs to it and it fades in. Without animations, at once.
    _replace(mode, parts, tabs, vertical) {
        const fading = !!this._pending;
        this._pending = {mode, parts, tabs, vertical};
        // The terminal fades out with the content it belongs to, not after
        // it: gone before the card shrinks past its window.
        if (this._termRef)
            this._hideTerminal();
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
        this._content.child = this._renderParts(pending.mode, pending.parts, pending.tabs, pending.vertical);
        old?.destroy();
        this._shown = pending.mode;
        this._shownVertical = pending.vertical;
        this._showOrConceal();
        this._shownVerticalPeek = pending.mode === 'peek' && pending.parts.some(([, slot]) => slot === 'verticalPeek');
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
    // side by side (down a side edge, each its `verticalCompact` or a dot),
    // the peek's view (the daemon names `verticalPeek` on a side edge; a
    // module without one shows its `peek`), or the open notch's expanded
    // views stacked (every module, or with tabs the picked one: `shown`).
    _parts(copy, vertical) {
        const views = this._host.store().views;
        const part = (module, slot) => {
            const tree = views[`${module}/${slot}`]?.tree ?? null;
            return tree ? [module, slot, tree] : null;
        };
        let parts = [];
        switch (copy.mode) {
        case 'compact':
            parts = (copy.compact ?? []).map(m => {
                const flat = part(m, 'compact');
                return flat && vertical ? part(m, 'verticalCompact') ?? [m, DOT, DOT] : flat;
            });
            break;
        case 'peek':
            parts = copy.peek ? [part(copy.peek.module, copy.peek.view) ?? part(copy.peek.module, 'peek')] : [];
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
    _renderParts(mode, parts, tabs = [], vertical = false) {
        this._partCtx = parts.map(([module, slot]) => ({
            theme: this.theme,
            onAction: (action, value) => this._send(Protocol.event(module, slot, action, value)),
            terminals: this._host.terminals,
            onTerminal: line => this.onLine(line),
        }));
        this._partActors = parts.map(([, slot, tree], i) => (slot === DOT ? this._dot() : render(tree, this._partCtx[i])));
        this._partBox = null;
        this._partHolder = null;
        this._scrollView = null;
        this._tabButtons = [];
        const area = this._partsArea(mode, this._partActors.filter(a => a), vertical);
        let content = area;
        if (mode === 'expanded' && area) {
            const scroll = new St.ScrollView({overlay_scrollbars: true, style: 'min-height: 0px;'});
            scroll.set_policy(St.PolicyType.NEVER, St.PolicyType.AUTOMATIC);
            scroll.set_child(area);
            scroll.x_expand = true;
            scroll.y_expand = true;
            this._scrollView = scroll;
            if (!this._partBox)
                this._partHolder = scroll;
            content = scroll;
        }
        if (tabs.length === 0)
            return content;
        const column = new St.BoxLayout({style: 'spacing: 10px;'});
        if ('orientation' in column)
            column.orientation = Clutter.Orientation.VERTICAL;
        else
            column.vertical = true;
        column.add_child(this._tabRow(tabs));
        if (content)
            column.add_child(content);
        if (!this._partBox && !this._partHolder)
            this._partHolder = column;
        return column;
    }

    // The parts' actors side by side (closed) or stacked (open, or closed
    // down a side edge): one alone as it is, several in a box.
    _partsArea(mode, actors, sideEdge) {
        if (actors.length === 0)
            return null;
        if (actors.length === 1)
            return actors[0];
        const stacked = mode === 'expanded' || (mode === 'compact' && sideEdge);
        const box = new St.BoxLayout({style: `spacing: ${stacked ? 14 : 12}px;`});
        if ('orientation' in box)
            box.orientation = stacked ? Clutter.Orientation.VERTICAL : Clutter.Orientation.HORIZONTAL;
        else
            box.vertical = stacked;
        for (const a of actors) {
            if (!stacked)
                a.y_align = Clutter.ActorAlign.CENTER;
            else if (mode === 'compact')
                a.x_align = Clutter.ActorAlign.CENTER;
            box.add_child(a);
        }
        this._partBox = box;
        return box;
    }

    // A side edge's compact view for a module without a `verticalCompact`:
    // an accent dot (NotchCard.qml's dotView).
    _dot() {
        const s = v => this.theme.space(v);
        const holder = new St.Widget({layout_manager: new Clutter.BinLayout(), width: s(6), height: s(24)});
        holder.add_child(new St.Widget({
            width: s(6), height: s(6),
            style: `background-color: ${this.theme.css('accent')}; border-radius: ${s(3)}px;`,
            x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
        }));
        return holder;
    }

    // New trees for the parts drawn last time: each actor updated in place
    // where it can, else replaced where it was. False when a part appears
    // or goes (a tree now invisible, or visible again): drawn again then.
    _updateParts(parts) {
        for (let i = 0; i < parts.length; i++) {
            const prev = this._partActors[i];
            if (parts[i][1] === DOT)
                continue;
            const actor = render(parts[i][2], this._partCtx[i], 0, prev);
            if (actor === prev)
                continue;
            if (!prev || !actor)
                return false;
            this._partActors[i] = actor;
            const parent = this._partBox ?? this._partHolder;
            if (parent) {
                actor.x_align = prev.x_align;
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

    // The card in two axes, as NotchCard.qml: `along` the edge and `depth`
    // away from it. Rest, compact and a vertical peek run along the edge,
    // so on a side edge their padding turns with it; other peeks and the
    // open notch stay wide on every edge and keep their padding by screen
    // axis. The shoulders flare out at both ends along the edge.
    _layout(animate) {
        const monitor = Main.layoutManager.monitors[this._monitorIndex] ?? Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;
        const s = v => this.theme.space(v);
        const edge = this._edge;
        const vertical = Position.isVertical(edge);
        const shoulder = s(CARD.shoulder);
        const pads = this._pads();
        const shown = this._shown;
        const turned = !vertical || shown === 'rest' || shown === 'compact' || (shown === 'peek' && this._shownVerticalPeek);
        const alongPad = turned ? pads.side : pads.far;
        const nearPad = turned ? pads.near : pads.side;
        const farPad = turned ? pads.far : pads.side;
        let along, depth, cw = 0, ch = 0;
        const child = this._content.child;
        if (shown === 'rest' || !child) {
            along = s(CARD.restLength);
            depth = s(CARD.restDepth);
        } else {
            const [, natW] = child.get_preferred_width(-1);
            // Mutter reports logical monitor pixels, including fractional
            // scaling. Keep the open card (padding and shoulders included)
            // inside the available edge when a module asks for more room.
            const insets = this._effectiveInsets(monitor);
            const availableW = vertical
                ? monitor.width - insets.left - insets.right - nearPad - farPad
                // Leave room beyond the outline's shoulders so a centered
                // card does not snap to a corner on a narrow screen.
                : monitor.width - insets.left - insets.right - 2 * (alongPad + shoulder + s(5));
            const maxW = shown === 'expanded'
                ? Math.min(s(this._expandedWidth()), Math.max(1, availableW))
                : monitor.width / 2;
            cw = shown === 'expanded' ? maxW : Math.min(natW, maxW);
            [, ch] = child.get_preferred_height(cw);
            ch = Math.min(ch, monitor.height - s(64));
            child.set_size(cw, ch);
            along = (vertical ? ch : cw) + 2 * alongPad;
            depth = (vertical ? cw : ch) + nearPad + farPad;
        }
        const naturalDepth = depth;
        if (shown === 'rest' || shown === 'compact')
            this._closedDepth = naturalDepth;
        depth = Math.max(depth, Math.min(this._rowDepth, vertical ? monitor.width : monitor.height));
        this._contentShift = Math.max(0, depth - naturalDepth) / 2;
        this._cardAlong = along;
        this._cardDepth = depth;
        this._radius = depth > naturalDepth ? s(CARD.compactRadius) : pads.radius;
        this._contentBox = {w: cw, h: ch, near: nearPad};
        // The rest lip is thin: its hit area reaches a little deeper.
        const hit = shown === 'rest' ? Math.max(depth, s(CARD.hitDepth)) : depth;
        this._hitDepth = hit;
        // Below the top panel on the monitor it is on (the panel's own
        // strut keeps windows under it; the notch hangs from it), and down
        // a side edge from there.
        const {spot, target} = this._positionFor(this._drag?.offset ?? this._notch?.offset ?? 0.5,
            monitor, this._effectiveInsets(monitor));
        this._naturalSpot = spot;
        this._naturalTarget = target;
        this._moveTo(target, animate);
        this._placeContent();
        this._redrawCard();
        this._placeNavLabel();
        this._measure();
        this._placeTerminal();
        this._host.requestLayout?.();
    }

    // Against the edge past its near padding, centered along it, on whole
    // pixels, at the card's current (springing) size.
    _placeContent() {
        const b = this._contentBox;
        if (!b)
            return;
        const W = this.width, H = this.height;
        let x = (W - b.w) / 2, y = (H - b.h) / 2;
        if (this._edge === 'top')
            y = b.near + this._contentShift;
        else if (this._edge === 'bottom')
            y = H - b.near - b.h - this._contentShift;
        else if (this._edge === 'left')
            x = b.near + this._contentShift;
        else
            x = W - b.near - b.w - this._contentShift;
        this._content.set_position(Math.round(x), Math.round(y));
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
        // Already there: nothing to spring (a timeline of 460 ms standing
        // still, after a layout that changed nothing).
        if (same(from)) {
            this._placeTerminal();
            return;
        }
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
            this._placeTerminal();
        });
        this._motion = {to, timeline};
        timeline.start();
    }

    // What this copy measures of itself, for the daemon's reserve.
    _measure() {
        // A guest reserves nothing.
        if (this.guest)
            return;
        const s = v => this.theme.space(v);
        const compact = this._shown === 'compact' ? this._cardDepth : 0;
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
        const vertical = Position.isVertical(this._edge);
        // The card grows with the actor while it eases: draw at the
        // actor's current size, not the target.
        const w = Math.max(0, (vertical ? ah : aw) - 2 * shoulder);
        const depth = vertical ? aw : ah;
        const h = this._shown === 'rest' ? Math.min(depth, this._cardDepth) : depth;
        const paths = Outline.path({w, h, shoulder, radius: this._radius ?? 0,
            start: this._startSide, end: this._endSide});
        const c = this.theme.colors;
        // Outline.js draws for the top edge (x along, y depth); turned onto
        // the card's edge as NotchCard.qml's transform does: bottom (x, H -
        // y), left (y, x), right (W - y, x), past the start's shoulder.
        if (this._edge === 'bottom') {
            cr.translate(shoulder, ah);
            cr.scale(1, -1);
        } else if (this._edge === 'left') {
            cr.translate(0, shoulder);
            cr.rotate(Math.PI / 2);
            cr.scale(1, -1);
        } else if (this._edge === 'right') {
            cr.translate(aw, shoulder);
            cr.rotate(Math.PI / 2);
        } else {
            cr.translate(shoulder, 0);
        }
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
                // Grab the card and give its terminal actor the keys.
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
                const terminal = this._terminalActor()?._terminal;
                if (terminal)
                    terminal.focusInput();
                else
                    this._keys.grab_key_focus();
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
        const vertical = Position.isVertical(this._edge);
        const shoulder = this.theme.space(CARD.shoulder);
        const target = this._target ?? {x: this.x, y: this.y, w: this.width, h: this.height};
        const bodyStart = (vertical ? target.y : target.x) + shoulder;
        const adjustment = this._scrollView?.get_vadjustment();
        return {
            // The mode drawn, which the geometry below belongs to; `wanted`,
            // the daemon's, leads it by a fade.
            mode: this._shown, wanted: this._mode, screen: this._copy?.screen ?? '', monitor: this._monitorIndex,
            edge: this._edge, dragging: !!this._dragging, seen: !!this._seen, covered: this._covered,
            ...target,
            card: {along: this._cardAlong ?? 0, depth: this._cardDepth ?? 0,
                start: bodyStart, end: bodyStart + (this._cardAlong ?? 0),
                startSide: this._startSide.kind, endSide: this._endSide.kind}, modal: !!this._modal,
            // The terminal belongs to this card's content tree.
            terminal: this._termRef ? {ref: this._termRef, embedded: true, focus: !!this._modal} : null,
            scroll: adjustment ? {value: adjustment.value, upper: adjustment.upper,
                pageSize: adjustment.page_size, height: this._scrollView.height} : null,
            hasContent: !!this._content.child, shown: this._copy?.shown ?? [],
            navLabel: this._navLabel ? this._navLabel.get_children().flatMap(c => (c instanceof St.Label ? [c.text] : c.get_children().map(l => l.text))) : null,
            // The view's progress bars and pictures (a test presses a bar
            // where it is and reads what a picture shows).
            bars: this._viewActors(a => a._track).map(a => ({...this._boxOf(a._track), seek: !!a.reactive, shown: a._shownFraction ?? null})),
            texts: this._viewActors(a => a instanceof St.Label).map(a => a.text),
            // Each text's size in pixels, as its style sets it.
            textSizes: this._viewActors(a => a instanceof St.Label).map(a => Number(/font-size: ([\d.]+)px/.exec(a.style ?? '')?.[1] ?? 0)),
            canvases: this._viewActors(a => a._canvas).map(a => ({w: a.width, h: a.height, cmds: a._canvas.cmds.length})),
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
        const terminal = this._terminalActor()?._terminal;
        if (terminal)
            return terminal.key(event);
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
        if (this._hoverPoll)
            GLib.source_remove(this._hoverPoll);
        this._hoverPoll = 0;
        this._cancelHold();
        if (this._dropTimeout)
            GLib.source_remove(this._dropTimeout);
        this._dropTimeout = 0;
        this._dragGrab?.dismiss();
        this._dragGrab = null;
        this._pending = null;
        this._motion?.timeline.stop();
        this._motion = null;
        this._navLabel?.destroy();
        this._navLabel = null;
        // Its terminal actor goes with the content tree.
        this._destroying = true;
        // Its last word: the keyboard it held is let go, and its terminal
        // actor is destroyed with the content tree.
        this._hideTerminal();
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
