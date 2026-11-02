// A module's view tree (docs/architecture.md, "Primitive tree (v1)") drawn
// with St. The spike draws row, column, text, icon, spacer and progress; any
// other primitive is a visible placeholder, never a blank (the hello's caps
// say which ones this UI has, so the daemon knows too).

import Clutter from 'gi://Clutter';
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as Tree from './lib/Tree.js';

export const PRIMITIVES = ['row', 'column', 'text', 'icon', 'spacer', 'progress'];

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

function box(vertical, gap, theme) {
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

// `ctx`: { theme, onAction(action, value) }.
export function render(node, ctx, depth = 0) {
    if (!node)
        return null;
    const problem = Tree.check(node, depth);
    if (problem)
        return placeholder(`invalid view: ${problem}`, ctx.theme);
    if (node.visible === false)
        return null;
    let actor;
    switch (node.t) {
    case 'row':
    case 'column': {
        actor = box(node.t === 'column', node.gap, ctx.theme);
        const align = {start: Clutter.ActorAlign.START, center: Clutter.ActorAlign.CENTER, end: Clutter.ActorAlign.END}[node.align];
        for (const child of node.children ?? []) {
            const c = render(child, ctx, depth + 1);
            if (!c)
                continue;
            if (align !== undefined) {
                if (node.t === 'row')
                    c.y_align = align;
                else
                    c.x_align = align;
            } else if (node.t === 'row') {
                c.y_align = Clutter.ActorAlign.CENTER;
            }
            if (child.fill) {
                if (node.t === 'row')
                    c.x_expand = true;
                else
                    c.y_expand = true;
            }
            actor.add_child(c);
        }
        break;
    }
    case 'text': {
        const size = fontSize(node.size, ctx.theme, 'body');
        let style = `color: ${ctx.theme.css(node.tone ?? 'foreground')}; font-size: ${size}px;`;
        if (node.bold)
            style += ' font-weight: bold;';
        if (node.tabular)
            style += ' font-feature-settings: "tnum";';
        actor = new St.Label({text: String(node.text ?? ''), style});
        const ct = actor.clutter_text;
        if (node.wrap) {
            ct.line_wrap = true;
            ct.ellipsize = Pango.EllipsizeMode.NONE;
        } else {
            ct.ellipsize = node.elide === 'none' ? Pango.EllipsizeMode.NONE : Pango.EllipsizeMode.END;
        }
        if (node.maxWidth)
            actor.style += ` max-width: ${node.maxWidth}px;`;
        break;
    }
    case 'icon': {
        const size = fontSize(node.size, ctx.theme, 'body');
        const named = iconName(node.name);
        const color = ctx.theme.css(node.tone ?? 'foreground');
        if (named) {
            actor = new St.Icon({icon_name: named, icon_size: Math.round(size * 1.15), style: `color: ${color};`});
        } else {
            const glyph = Tree.glyphOf(node);
            actor = new St.Label({text: glyph || '?', style: `color: ${color}; font-size: ${size}px;`});
        }
        break;
    }
    case 'spacer':
        actor = new St.Widget({width: ctx.theme.space(node.size ?? 0), height: 1});
        if (node.fill)
            actor.x_expand = true;
        break;
    case 'progress': {
        const thickness = Number(node.thickness) || 3;
        actor = new St.Widget({
            style: `background-color: ${ctx.theme.css('line')}; border-radius: ${thickness}px; height: ${thickness}px; min-width: 40px;`,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        if (typeof node.value === 'number') {
            const bar = new St.Widget({style: `background-color: ${ctx.theme.css(node.tone ?? 'accent')}; border-radius: ${thickness}px;`});
            actor.add_child(bar);
            actor.connect('notify::allocation', () => {
                const box = actor.get_allocation_box();
                bar.set_size((box.x2 - box.x1) * node.value, box.y2 - box.y1);
            });
        }
        break;
    }
    default:
        actor = placeholder(`${node.t}: not drawn yet`, ctx.theme);
    }
    if (node.width)
        actor.width = ctx.theme.space(node.width);
    if (node.height)
        actor.height = ctx.theme.space(node.height);
    if (node.minWidth)
        actor.style = `${actor.style ?? ''} min-width: ${node.minWidth}px;`;
    if (node.action) {
        actor.reactive = true;
        actor.connect('button-release-event', () => {
            ctx.onAction(node.action, null);
            return Clutter.EVENT_STOP;
        });
    }
    return actor;
}
