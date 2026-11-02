// A module's view tree (docs/architecture.md, "Primitive tree (v1)") drawn
// with St. Every v1 primitive but canvas and surface is drawn; those are a
// visible placeholder, never a blank (the hello's caps say which ones this
// UI has, so the daemon knows too).
//
// A new `view` updates the actors it drew last time where it can, as the
// QML's Repeaters keep their items: a node equal to the last is kept as is,
// one of the same primitive and `id` that knows how is updated in place
// (containers reconcile their children by position), any other is drawn
// again. So an equalizer keeps moving and a meter eases from where it was
// while a module sends views many times a second.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Graphene from 'gi://Graphene';
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as Tree from './lib/Tree.js';

export const PRIMITIVES = ['row', 'column', 'stack', 'text', 'icon', 'spacer', 'progress', 'button', 'chip', 'image', 'meter', 'equalizer', 'list'];

// Named icons that are not freedesktop names, mapped to Adwaita's nearest
// symbolic icon. A freedesktop name is used as `<name>-symbolic`.
const ADWAITA = {
    cpu: 'computer-symbolic',
    memory: 'media-flash-symbolic',
    focus: 'find-location-symbolic',
    coffee: 'weather-few-clouds-symbolic',
    alarm: 'alarm-symbolic',
    notification: 'preferences-system-notifications-symbolic',
    'microphone-disabled': 'microphone-disabled-symbolic',
};

let iconTheme = null;
function hasIcon(name) {
    try {
        iconTheme ??= new St.IconTheme();
        return iconTheme.has_icon(name);
    } catch (e) {
        return false;
    }
}

export function iconName(name) {
    if (!name)
        return null;
    const candidate = ADWAITA[name] ?? `${name}-symbolic`;
    return hasIcon(candidate) ? candidate : null;
}

const ALIGN = {start: Clutter.ActorAlign.START, center: Clutter.ActorAlign.CENTER, end: Clutter.ActorAlign.END};

function box(vertical, gap) {
    const b = new St.BoxLayout({style: `spacing: ${Number(gap) || 0}px;`});
    if ('orientation' in b)
        b.orientation = vertical ? Clutter.Orientation.VERTICAL : Clutter.Orientation.HORIZONTAL;
    else
        b.vertical = vertical;
    return b;
}

function fontSize(size, theme, fallback) {
    if (typeof size === 'number')
        return size;
    return theme.fonts[size] ?? theme.fonts[fallback];
}

function placeholder(text, theme) {
    return new St.Label({text, style: `color: ${theme.css('danger')}; font-size: ${theme.fonts.caption}px;`});
}

// A `tone` as CSS. On an accent fill (a meter's child), `foreground`
// becomes the background, as PrimBase.qml's `onAccent`.
const TONES = ['muted', 'faint', 'accent', 'danger', 'success', 'line'];
function tone(ctx, name, fallback = 'foreground') {
    const n = name ?? fallback;
    if (TONES.includes(n))
        return ctx.theme.css(n);
    return ctx.theme.css(ctx.onAccent ? 'background' : 'foreground');
}

// A named icon (Adwaita symbolic) or the node's glyph as text, `px` tall.
function glyphActor(node, color, px) {
    const named = iconName(node.name);
    if (named)
        return new St.Icon({icon_name: named, icon_size: Math.round(px * 1.15), style: `color: ${color};`});
    const glyph = Tree.glyphOf(node);
    return glyph ? new St.Label({text: glyph, style: `color: ${color}; font-size: ${px}px;`, y_align: Clutter.ActorAlign.CENTER}) : null;
}

// Children reconciled by position: each new child node is drawn over the
// actor at its place (kept, updated or replaced), the rest go. `place(actor,
// node)` sets what the parent decides of a child (alignment, expansion).
function reconcile(parent, nodes, ctx, depth, place) {
    const old = parent.get_children();
    let k = 0;
    let at = 0;
    for (const child of nodes ?? []) {
        const prev = old[k] ?? null;
        const actor = render(child, ctx, depth + 1, prev);
        if (!actor)
            continue;
        if (actor === prev) {
            k++;
        } else {
            if (prev) {
                prev.destroy();
                k++;
            }
            parent.insert_child_at_index(actor, at);
        }
        place?.(actor, child);
        at++;
    }
    for (; k < old.length; k++)
        old[k].destroy();
}

// --- Primitives. Each has a `build(node, ctx, depth)` and may have an
// `update(actor, node, ctx, depth)` that returns false when it cannot.

function rowColumn(node, ctx, depth, actor) {
    const vertical = node.t === 'column';
    actor ??= box(vertical, node.gap);
    actor.style = `spacing: ${Number(node.gap) || 0}px;`;
    const align = ALIGN[node.align];
    reconcile(actor, node.children, ctx, depth, (c, child) => {
        if (vertical) {
            c.x_align = align ?? Clutter.ActorAlign.FILL;
            c.y_expand = !!child.fill;
        } else {
            c.y_align = align ?? Clutter.ActorAlign.CENTER;
            c.x_expand = !!child.fill;
        }
    });
    return actor;
}

// Overlay: every child on top of the others, centered unless `align` says
// start or end. The size is the largest child's. As StackPrim.qml.
function stack(node, ctx, depth, actor) {
    actor ??= new St.Widget({layout_manager: new Clutter.BinLayout()});
    const align = ALIGN[node.align] ?? Clutter.ActorAlign.CENTER;
    reconcile(actor, node.children, ctx, depth, c => {
        c.x_align = align;
        c.y_align = align;
    });
    return actor;
}

function textStyle(node, ctx) {
    let style = `color: ${tone(ctx, node.tone)}; font-size: ${fontSize(node.size, ctx.theme, 'body')}px;`;
    if (node.bold)
        style += ' font-weight: bold;';
    if (node.tabular)
        style += ' font-feature-settings: "tnum";';
    if (node.maxWidth)
        style += ` max-width: ${node.maxWidth}px;`;
    return style;
}

function text(node, ctx, depth, actor) {
    actor ??= new St.Label();
    actor.text = String(node.text ?? '');
    actor.style = textStyle(node, ctx);
    const ct = actor.clutter_text;
    ct.line_wrap = !!node.wrap;
    ct.ellipsize = node.wrap || node.elide === 'none' ? Pango.EllipsizeMode.NONE : Pango.EllipsizeMode.END;
    return actor;
}

function icon(node, ctx) {
    const color = tone(ctx, node.tone);
    return glyphActor(node, color, fontSize(node.size, ctx.theme, 'body')) ??
        new St.Label({text: '?', style: `color: ${color};`});
}

function spacer(node, ctx) {
    const actor = new St.Widget({width: ctx.theme.space(node.size ?? 0), height: 1});
    actor.x_expand = !!node.fill;
    return actor;
}

// A bar on a line-colored track; `value` absent or null draws the track only.
function progress(node, ctx, depth, actor) {
    const thickness = Number(node.thickness) || 3;
    const hasValue = typeof node.value === 'number';
    if (actor && !!actor._bar !== hasValue)
        return null;
    if (!actor) {
        actor = new St.Widget({x_expand: true, y_align: Clutter.ActorAlign.CENTER});
        if (hasValue) {
            actor._bar = new St.Widget();
            actor.add_child(actor._bar);
            actor.connect('notify::allocation', () => sizeBar(actor));
        }
    }
    actor.style = `background-color: ${ctx.theme.css('line')}; border-radius: ${thickness}px; height: ${thickness}px; min-width: 40px;`;
    if (actor._bar) {
        actor._bar.style = `background-color: ${tone(ctx, node.tone, 'accent')}; border-radius: ${thickness}px;`;
        actor._value = Math.max(0, Math.min(1, node.value));
        sizeBar(actor);
    }
    return actor;
}

function sizeBar(actor) {
    const b = actor.get_allocation_box();
    actor._bar.set_size((b.x2 - b.x1) * actor._value, b.y2 - b.y1);
}

// A pressable St.Button whose fill follows hover and `selected`: `look(hover)`
// gives its style. St styles set from code have no :hover, so it restyles.
function pressable(look, live, onPress) {
    const button = new St.Button({reactive: live, can_focus: live, track_hover: live, style: look(false)});
    if (live) {
        button.connect('notify::hover', () => {
            button.style = look(button.hover);
        });
        button.connect('clicked', onPress);
    }
    return button;
}

// The modules' IconButton (a 28 px round hover fill around a heading-size
// glyph; `big`: 36 px) and, with a `label` or `variant: "outline"`, the
// timer's bordered button (a chip's shape). Dimmed when disabled; the
// press fires `action` with null. As core/client/prims/ButtonPrim.qml.
function button(node, ctx) {
    const theme = ctx.theme;
    const hasLabel = typeof node.label === 'string' && node.label !== '';
    const outline = node.variant === 'outline';
    const round = !hasLabel && !outline;
    const big = round && node.big === true;
    const usable = node.enabled !== false;
    const live = usable && typeof node.action === 'string' && node.action !== '';
    const selected = node.selected === true;
    const look = hover => {
        const fill = selected || (hover && live) ? theme.css('selected') : 'transparent';
        let style = `background-color: ${fill};`;
        if (round) {
            const d = big ? 36 : 28;
            style += ` width: ${d}px; height: ${d}px; border-radius: ${d / 2}px;`;
        } else {
            style += ` height: 32px; padding: 0 10px; border-radius: ${theme.radiusSmall}px;`;
            if (outline)
                style += ` border: 1px solid ${selected ? theme.css('accent') : theme.css('line')};`;
        }
        return style;
    };
    const actor = pressable(look, live, () => ctx.onAction(node.action, null));
    actor.opacity = usable ? 255 : Math.round(255 * 0.35);
    const content = box(false, 6);
    const glyphPx = hasLabel ? theme.fonts.bodySmall : big ? theme.fonts.heading * 1.25 : theme.fonts.heading;
    const glyph = glyphActor(node, tone(ctx, node.tone, hasLabel ? 'accent' : 'foreground'), glyphPx);
    if (glyph)
        content.add_child(glyph);
    if (hasLabel) {
        const color = glyph ? tone(ctx, 'foreground') : tone(ctx, node.tone);
        content.add_child(new St.Label({text: node.label, style: `color: ${color}; font-size: ${theme.fonts.bodySmall}px;`, y_align: Clutter.ActorAlign.CENTER}));
    }
    content.x_align = Clutter.ActorAlign.CENTER;
    content.y_align = Clutter.ActorAlign.CENTER;
    actor.set_child(content);
    return actor;
}

// The timer's preset chip: a 32 px bordered pill, a hover fill, `selected`
// in the accent. The press fires `action` with `value` (null without one).
// As core/client/prims/ChipPrim.qml.
function chip(node, ctx) {
    const theme = ctx.theme;
    const live = typeof node.action === 'string' && node.action !== '';
    const selected = node.selected === true;
    const look = hover => {
        const fill = selected ? theme.css('selected') : hover && live ? theme.css('hover') : 'transparent';
        return `background-color: ${fill}; height: 32px; padding: 0 10px; border-radius: ${theme.radiusSmall}px;` +
            ` border: 1px solid ${selected ? theme.css('accent') : theme.css('line')};`;
    };
    const actor = pressable(look, live, () => ctx.onAction(node.action, node.value === undefined ? null : node.value));
    actor.set_child(new St.Label({
        text: node.label === undefined ? '' : String(node.label),
        style: `color: ${tone(ctx, 'foreground')}; font-size: ${theme.fonts.bodySmall}px;`,
        x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
    }));
    return actor;
}

// A local picture path or file:// URI as a GFile, or null (an https:// one
// and a bundled `asset:` are not loaded yet: the fallback draws).
function pictureFile(src) {
    const s = src === undefined || src === null ? '' : String(src);
    if (s.startsWith('/'))
        return Gio.File.new_for_path(s);
    if (s.startsWith('file://'))
        return Gio.File.new_for_uri(s);
    return null;
}

// Cover art: the picture cropped (or contained) over a `selected` fill,
// the `fallback` icon when it cannot be drawn. `radius`: none, small or
// round (Theme.qml's radiusSmall and radius). As ImagePrim.qml.
function image(node, ctx, depth) {
    const theme = ctx.theme;
    const size = node.size === undefined ? 38 : Number(node.size);
    const w = Number(node.w) || size;
    const h = Number(node.h) || size;
    const corner = node.radius === 'round' ? theme.radius : node.radius === 'small' ? theme.radiusSmall : 0;
    const contain = node.fit === 'contain';
    const actor = new St.Widget({layout_manager: new Clutter.BinLayout(), width: theme.space(w), height: theme.space(h)});
    const file = pictureFile(node.src);
    let style = `border-radius: ${corner}px;`;
    if (!contain)
        style += ` background-color: ${theme.css('selected')};`;
    if (file && file.query_exists(null)) {
        style += ` background-image: url("${file.get_uri()}"); background-size: ${contain ? 'contain' : 'cover'}; background-position: center;`;
    } else {
        const fallback = render(node.fallback, ctx, depth + 1);
        if (fallback) {
            fallback.x_align = Clutter.ActorAlign.CENTER;
            fallback.y_align = Clutter.ActorAlign.CENTER;
            actor.add_child(fallback);
        }
    }
    actor.style = style;
    return actor;
}

const METER_EASE_MS = 140;

// The Voice disc: an accent disc with a line border that swells with the
// level (`1 + value * .35` over 140 ms, from the scale it has), its `child`
// inside on the accent. It takes the room of the fully swollen disc. As
// MeterPrim.qml.
function meter(node, ctx, depth, actor) {
    const theme = ctx.theme;
    const disc = Number(node.size) > 0 ? Number(node.size) : 96;
    if (actor && actor._disc !== disc)
        return null;
    const target = 1 + (typeof node.value === 'number' ? Math.max(0, Math.min(1, node.value)) : 0) * 0.35;
    if (!actor) {
        const room = theme.space(Math.ceil(disc * 1.35));
        actor = new St.Widget({layout_manager: new Clutter.BinLayout(), width: room, height: room});
        const d = theme.space(disc);
        actor._circle = new St.Widget({
            layout_manager: new Clutter.BinLayout(),
            width: d, height: d,
            x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
            pivot_point: new Graphene.Point({x: 0.5, y: 0.5}),
            scale_x: target, scale_y: target,
        });
        actor._disc = disc;
        actor.add_child(actor._circle);
    }
    const circle = actor._circle;
    circle.style = `background-color: ${theme.css('accent')}; border: 1px solid ${theme.css('line')}; border-radius: ${disc / 2}px;`;
    // Longer than the time between levels (at most ten a second): the next
    // level comes while it moves, so the disc never stops between steps.
    circle.ease({scale_x: target, scale_y: target, duration: METER_EASE_MS, mode: Clutter.AnimationMode.EASE_OUT_CUBIC});
    reconcile(circle, node.child ? [node.child] : [], {...ctx, onAccent: true}, depth, c => {
        c.x_align = Clutter.ActorAlign.CENTER;
        c.y_align = Clutter.ActorAlign.CENTER;
    });
    return actor;
}

// The media module's decorative bars: they wander while `active` and settle
// low when not. Smooth (the default) eases toward new targets 60 times a
// second; light jumps about six times a second. A fixed-height box, so the
// motion never resizes the notch. The motion is the actor's own: a new view
// only says whether it runs. As EqualizerPrim.qml.
const EQ_REST = 0.2;

function equalizer(node, ctx, depth, actor) {
    const theme = ctx.theme;
    const count = Math.max(1, Math.round(Number(node.bars) || 4));
    const height = theme.space(Number(node.height) || 14);
    if (actor && (actor._eq.bars.length !== count || actor._eq.height !== height))
        return null;
    const active = node.active === true;
    if (!actor) {
        actor = box(false, 2);
        actor.height = height;
        const width = theme.space(3);
        const eq = {bars: [], height, width, ticks: 0, id: 0, smooth: true, active};
        eq.levels = Array.from({length: count}, () => (active ? 0.25 + Math.random() * 0.75 : EQ_REST));
        eq.targets = eq.levels.slice();
        for (let i = 0; i < count; i++) {
            const bar = new St.Widget({width, y_align: Clutter.ActorAlign.END});
            eq.bars.push(bar);
            actor.add_child(bar);
        }
        actor._eq = eq;
        actor.connect('destroy', () => stopEqualizer(eq));
        drawEqualizer(eq);
    }
    const eq = actor._eq;
    for (const bar of eq.bars)
        bar.style = `background-color: ${theme.css('accent')}; border-radius: ${eq.width / 2}px;`;
    const smooth = node.mode !== 'light';
    if (smooth !== eq.smooth)
        stopEqualizer(eq);
    eq.smooth = smooth;
    eq.active = active;
    startEqualizer(actor);
    return actor;
}

function drawEqualizer(eq) {
    eq.bars.forEach((bar, i) => {
        bar.height = Math.max(eq.width, Math.round(eq.height * eq.levels[i]));
    });
}

function startEqualizer(actor) {
    const eq = actor._eq;
    if (eq.id)
        return;
    eq.id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, eq.smooth ? 16 : 160, () => {
        if (!actor.mapped)
            return GLib.SOURCE_CONTINUE;
        let t = eq.targets;
        if (!eq.active)
            t = t.map(() => EQ_REST);
        else if (!eq.smooth || ++eq.ticks % 10 === 0)
            t = t.map(v => (!eq.smooth || Math.random() < 0.6 ? 0.25 + Math.random() * 0.75 : v));
        eq.targets = t;
        // Smooth closes ~23 % of the gap per step: a ~60 ms time constant.
        eq.levels = eq.levels.map((v, i) => (eq.smooth ? v + (t[i] - v) * 0.23 : t[i]));
        drawEqualizer(eq);
        if (!eq.active && eq.levels.every(v => Math.abs(v - EQ_REST) < 0.01)) {
            eq.id = 0;
            return GLib.SOURCE_REMOVE;
        }
        return GLib.SOURCE_CONTINUE;
    });
}

function stopEqualizer(eq) {
    if (eq.id)
        GLib.source_remove(eq.id);
    eq.id = 0;
}

// A scrolling column, as tall as its content up to `maxHeight`. `stick:
// "end"` keeps the newest line in view until the user scrolls up;
// scrolling back to the end sticks again. As ListPrim.qml.
function list(node, ctx, depth, actor) {
    if (!actor) {
        actor = new St.ScrollView({
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
            overlay_scrollbars: true,
            x_expand: true,
        });
        actor._content = box(true, 8);
        if (actor.set_child)
            actor.set_child(actor._content);
        else
            actor.add_actor(actor._content);
        actor._pinned = true;
        const adj = actor.vadjustment ?? actor.vscroll.adjustment;
        const toEnd = () => {
            if (actor._stickEnd && actor._pinned)
                adj.value = adj.upper - adj.page_size;
        };
        adj.connect('notify::upper', toEnd);
        adj.connect('notify::page-size', toEnd);
        // A scroll by the user decides whether it sticks.
        actor.connect('scroll-event', () => {
            GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                actor._pinned = adj.value >= adj.upper - adj.page_size - 1;
                return GLib.SOURCE_REMOVE;
            });
            return Clutter.EVENT_PROPAGATE;
        });
    }
    actor._stickEnd = node.stick === 'end';
    actor.style = Number(node.maxHeight) > 0 ? `max-height: ${Number(node.maxHeight)}px;` : '';
    actor._content.style = `spacing: ${Number(node.gap ?? 8) || 0}px;`;
    reconcile(actor._content, node.children, ctx, depth, (c, child) => {
        c.x_expand = ['text', 'row', 'column', 'progress'].includes(child.t);
        c.x_align = c.x_expand ? Clutter.ActorAlign.FILL : Clutter.ActorAlign.START;
    });
    return actor;
}

const BUILD = {
    row: rowColumn, column: rowColumn, stack, text, icon, spacer, progress,
    button, chip, image, meter, equalizer, list,
};
// The primitives that update an actor in place (the others draw again).
const UPDATES = new Set(['row', 'column', 'stack', 'text', 'progress', 'meter', 'equalizer', 'list']);

// `ctx`: { theme, onAction(action, value), onAccent }. `prev`: the actor
// drawn at this place last time, or null; the result may be it.
export function render(node, ctx, depth = 0, prev = null) {
    if (!node)
        return null;
    const problem = Tree.check(node, depth);
    if (problem)
        return placeholder(`invalid view: ${problem}`, ctx.theme);
    if (node.visible === false)
        return null;
    const json = JSON.stringify(node);
    const last = prev?._notchd;
    if (last && last.json === json && last.onAccent === !!ctx.onAccent)
        return prev;
    let actor = null;
    if (last && last.t === node.t && last.id === (node.id ?? null) && UPDATES.has(node.t))
        actor = BUILD[node.t](node, ctx, depth, prev);
    const updated = !!actor;
    if (!actor)
        actor = BUILD[node.t]?.(node, ctx, depth, null) ?? placeholder(`${node.t}: not drawn yet`, ctx.theme);
    actor._notchd = {t: node.t, id: node.id ?? null, json, node, onAccent: !!ctx.onAccent};
    common(actor, node, ctx, updated ? last.node : null);
    return actor;
}

// The fields every node has: a fixed size, a least width, a press action
// (a button's and a chip's are their own). `was`: the node an updated actor
// drew before, whose fixed size goes when the new one has none.
function common(actor, node, ctx, was) {
    if (node.width)
        actor.width = ctx.theme.space(node.width);
    else if (was?.width)
        actor.width = -1;
    if (node.height)
        actor.height = ctx.theme.space(node.height);
    else if (was?.height)
        actor.height = -1;
    if (node.minWidth)
        actor.style = `${actor.style ?? ''} min-width: ${node.minWidth}px;`;
    if (node.t === 'button' || node.t === 'chip' || actor._pressConnected)
        return;
    if (node.action) {
        actor.reactive = true;
        actor._pressConnected = true;
        actor.connect('button-release-event', () => {
            const action = actor._notchd?.node.action;
            if (action)
                ctx.onAction(action, null);
            return action ? Clutter.EVENT_STOP : Clutter.EVENT_PROPAGATE;
        });
    }
}
