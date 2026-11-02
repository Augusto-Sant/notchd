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

// A bar on a line-colored track, as ProgressPrim.qml: horizontal, or
// vertical (`orientation: "v"`, filling from the bottom); `value` absent or
// null draws the track only. With an `anchor` ({at, rate}) the bar moves by
// itself, `value + rate * (now - at)` ten times a second while it is on
// screen, so a playing position needs no stream of views. With a `seek`
// action a press, and every motion while it is held, fires it with the
// fraction where the pointer is. The bar sits in a hit area with 5 px
// around it, so it is easy to grab.
const PROGRESS_TICK_MS = 100;
const SEEK_PAD = 5;

function progress(node, ctx, depth, actor) {
    const vertical = node.orientation === 'v';
    const thickness = Number(node.thickness) || 4;
    const hasValue = typeof node.value === 'number';
    const seekable = hasValue && typeof node.seek === 'string' && node.seek !== '';
    // What the actor is made of: another shape is drawn again.
    const shape = JSON.stringify([vertical, hasValue, seekable]);
    if (actor && actor._shape !== shape)
        return null;
    if (!actor) {
        actor = new St.Widget({layout_manager: new Clutter.BinLayout(), reactive: seekable});
        actor._shape = shape;
        actor._track = new St.Widget();
        actor.add_child(actor._track);
        if (hasValue) {
            actor._bar = new St.Widget();
            actor._track.add_child(actor._bar);
            actor._track.connect('notify::allocation', () => sizeBar(actor));
        }
        if (seekable)
            seekHandlers(actor, vertical, ctx);
        actor.connect('destroy', () => stopProgress(actor));
    }
    const track = actor._track;
    actor.style = `padding: ${seekable ? SEEK_PAD : 0}px;`;
    actor._vertical = vertical;
    if (vertical) {
        actor.x_expand = false;
        actor.y_align = Clutter.ActorAlign.CENTER;
        track.x_align = Clutter.ActorAlign.CENTER;
        track.style = `background-color: ${ctx.theme.css('line')}; border-radius: ${thickness / 2}px; width: ${thickness}px; height: ${ctx.theme.space(120)}px;`;
    } else {
        actor.x_expand = true;
        actor.y_align = Clutter.ActorAlign.CENTER;
        track.x_expand = true;
        track.y_align = Clutter.ActorAlign.CENTER;
        track.style = `background-color: ${ctx.theme.css('line')}; border-radius: ${thickness / 2}px; height: ${thickness}px; min-width: 40px;`;
    }
    if (actor._bar) {
        actor._bar.style = `background-color: ${tone(ctx, node.tone, 'accent')}; border-radius: ${thickness / 2}px;`;
        const anchor = node.anchor;
        const rate = anchor && typeof anchor === 'object' ? Number(anchor.rate) : 0;
        actor._value = node.value;
        actor._anchor = rate ? {at: Number(anchor.at) || 0, rate} : null;
        sizeBar(actor);
        if (actor._anchor)
            startProgress(actor);
        else
            stopProgress(actor);
    }
    return actor;
}

// The fraction drawn now: the value, run on by the anchor, clamped.
function progressShown(actor) {
    let v = actor._value;
    if (actor._anchor)
        v += actor._anchor.rate * (Date.now() - actor._anchor.at) / 1000;
    return Math.max(0, Math.min(1, v));
}

function sizeBar(actor) {
    const b = actor._track.get_allocation_box();
    const w = b.x2 - b.x1;
    const h = b.y2 - b.y1;
    const shown = progressShown(actor);
    if (actor._vertical) {
        actor._bar.set_position(0, h * (1 - shown));
        actor._bar.set_size(w, h * shown);
    } else {
        actor._bar.set_position(0, 0);
        actor._bar.set_size(w * shown, h);
    }
    actor._shownFraction = shown;
}

function startProgress(actor) {
    if (actor._tick)
        return;
    actor._tick = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PROGRESS_TICK_MS, () => {
        if (actor.mapped)
            sizeBar(actor);
        return GLib.SOURCE_CONTINUE;
    });
}

function stopProgress(actor) {
    if (actor._tick)
        GLib.source_remove(actor._tick);
    actor._tick = 0;
}

// The press and drag of a seekable bar. The press is the bar's: it stops
// here, so it never reaches the card (whose press pins the notch and takes
// a grab that would cancel the drag). The motion and the release come from
// the stage while the button is held, so a drag may leave the bar.
function seekHandlers(actor, vertical, ctx) {
    const fire = event => {
        const action = actor._notchd?.node.seek;
        if (!action || !actor._track.get_stage())
            return;
        const [x, y] = event.get_coords();
        const [ok, tx, ty] = actor._track.transform_stage_point(x, y);
        const b = actor._track.get_allocation_box();
        const w = b.x2 - b.x1;
        const h = b.y2 - b.y1;
        if (!ok || !(w > 0) || !(h > 0))
            return;
        const f = vertical ? 1 - ty / h : tx / w;
        ctx.onAction(action, Math.max(0, Math.min(1, f)));
    };
    const release = () => {
        if (actor._drag)
            global.stage.disconnect(actor._drag);
        actor._drag = 0;
    };
    actor.connect('destroy', release);
    actor.connect('button-press-event', (a, event) => {
        if (event.get_button() !== Clutter.BUTTON_PRIMARY)
            return Clutter.EVENT_PROPAGATE;
        fire(event);
        release();
        actor._drag = global.stage.connect('captured-event', (stage, e) => {
            const type = e.type();
            if (type === Clutter.EventType.MOTION) {
                fire(e);
            } else if (type === Clutter.EventType.BUTTON_RELEASE) {
                release();
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
        return Clutter.EVENT_STOP;
    });
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

// A picture's source as a local GFile: an absolute path or a file:// URI.
// An http(s):// one is `remoteSource`'s, a bundled `asset:` is not loaded
// yet (the fallback draws).
function pictureFile(src) {
    const s = src === undefined || src === null ? '' : String(src);
    if (s.startsWith('/'))
        return Gio.File.new_for_path(s);
    if (s.startsWith('file://'))
        return Gio.File.new_for_uri(s);
    return null;
}

function remoteSource(src) {
    const s = src === undefined || src === null ? '' : String(src);
    return /^https?:\/\/[^\s]+$/.test(s) ? s : null;
}

// Pictures from the network live in a cache of their own, named by a hash
// of the URL, so the next view (or the next session) has them at once. The
// oldest files go past CACHE_MAX; one is at most CACHE_FILE_MAX bytes.
const CACHE_MAX = 64;
const CACHE_FILE_MAX = 8 * 1024 * 1024;

function cacheDir() {
    return GLib.build_filenamev([GLib.get_user_cache_dir(), 'notchd-gnome']);
}

function cacheFile(url) {
    const hash = GLib.compute_checksum_for_string(GLib.ChecksumType.SHA256, url, -1);
    return Gio.File.new_for_path(GLib.build_filenamev([cacheDir(), hash]));
}

function pruneCache() {
    try {
        const dir = Gio.File.new_for_path(cacheDir());
        const files = [];
        const en = dir.enumerate_children('standard::name,time::modified', Gio.FileQueryInfoFlags.NONE, null);
        for (let info = en.next_file(null); info; info = en.next_file(null))
            files.push({name: info.get_name(), time: info.get_modification_date_time()?.to_unix() ?? 0});
        en.close(null);
        files.sort((a, b) => b.time - a.time);
        for (const f of files.slice(CACHE_MAX))
            dir.get_child(f.name).delete(null);
    } catch (e) {
        // A cache that cannot be trimmed only keeps files a while longer.
    }
}

let soup = null;
let session = null;
let downloads = 0;
// url -> the callbacks waiting for it: a picture asked for twice while it
// downloads is fetched once.
const pending = new Map();

// Calls `done(GFile)` once `url` is in the cache, `done(null)` when it
// cannot be had. Asynchronous all the way (never a network call on the
// main loop); `done` may run at once (a cached file), from the main loop.
function fetchPicture(url, done) {
    const file = cacheFile(url);
    if (file.query_exists(null)) {
        done(file);
        return;
    }
    if (pending.has(url)) {
        pending.get(url).push(done);
        return;
    }
    pending.set(url, [done]);
    const finish = result => {
        const waiting = pending.get(url) ?? [];
        pending.delete(url);
        for (const cb of waiting)
            cb(result);
    };
    (async () => {
        try {
            soup ??= (await import('gi://Soup?version=3.0')).default;
            session ??= new soup.Session({timeout: 15, idle_timeout: 15});
            const msg = soup.Message.new('GET', url);
            const bytes = await new Promise((resolve, reject) => {
                session.send_and_read_async(msg, GLib.PRIORITY_DEFAULT, null, (s, res) => {
                    try {
                        resolve(s.send_and_read_finish(res));
                    } catch (e) {
                        reject(e);
                    }
                });
            });
            if (msg.status_code !== 200 || bytes.get_size() === 0 || bytes.get_size() > CACHE_FILE_MAX)
                throw new Error(`HTTP ${msg.status_code}, ${bytes.get_size()} bytes`);
            GLib.mkdir_with_parents(cacheDir(), 0o755);
            await new Promise((resolve, reject) => {
                file.replace_contents_bytes_async(bytes, null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null, (f, res) => {
                    try {
                        f.replace_contents_finish(res);
                        resolve();
                    } catch (e) {
                        reject(e);
                    }
                });
            });
            if (++downloads % 16 === 1)
                pruneCache();
            finish(file);
        } catch (e) {
            console.warn(`notchd: no picture from ${url}: ${e.message}`);
            finish(null);
        }
    })();
}

// Cover art: the picture cropped (or contained) over a `selected` fill,
// the `fallback` icon while it loads and when it cannot be had. `radius`:
// none, small or round (Theme.qml's radiusSmall and radius). A picture from
// the network (`https://`; the cache above) draws the fallback until it is
// there, if the actor still is. As ImagePrim.qml.
function image(node, ctx, depth) {
    const theme = ctx.theme;
    const size = node.size === undefined ? 38 : Number(node.size);
    const w = Number(node.w) || size;
    const h = Number(node.h) || size;
    const corner = node.radius === 'round' ? theme.radius : node.radius === 'small' ? theme.radiusSmall : 0;
    const contain = node.fit === 'contain';
    const actor = new St.Widget({layout_manager: new Clutter.BinLayout(), width: theme.space(w), height: theme.space(h)});
    let style = `border-radius: ${corner}px;`;
    if (!contain)
        style += ` background-color: ${theme.css('selected')};`;
    const picture = file => `${style} background-image: url("${file.get_uri()}"); background-size: ${contain ? 'contain' : 'cover'}; background-position: center;`;
    // `state` is what a test reads: loading, ready or failed.
    actor._picture = {state: 'failed', src: node.src ?? ''};
    const url = remoteSource(node.src);
    const file = pictureFile(node.src) ?? (url ? cacheFile(url) : null);
    if (file && file.query_exists(null)) {
        style = picture(file);
        actor._picture.state = 'ready';
    } else {
        const fallback = render(node.fallback, ctx, depth + 1);
        if (fallback) {
            fallback.x_align = Clutter.ActorAlign.CENTER;
            fallback.y_align = Clutter.ActorAlign.CENTER;
            actor.add_child(fallback);
        }
        if (url) {
            let dead = false;
            actor.connect('destroy', () => {
                dead = true;
            });
            actor._picture.state = 'loading';
            fetchPicture(url, got => {
                // A destroyed actor is not touched.
                if (dead)
                    return;
                if (!got) {
                    actor._picture.state = 'failed';
                    return;
                }
                actor._picture.state = 'ready';
                fallback?.destroy();
                actor.style = picture(got);
            });
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
