// The terminal view of the GNOME UI for the grid renderer (a terminal with
// "renderer": "ghostty", docs/architecture.md, "The grid renderer"): a GTK 4
// window that draws the screen the daemon sends, with Pango and Cairo, and
// sends it the keys, the pointer and the clipboard. The daemon emulates the
// terminal (libghostty-vt); nothing here does, and it needs no VTE. Same
// contract with the extension as helper.js:
//
//   gjs -m grid.js <exe> <socket> <ref> <background> <foreground> [font]
//
// It says, a line each on its standard output: typed, leave, keyboard,
// pointer in|out, exited, restart (helper.js tells what each means); and
// reads: colors <background> <foreground>, quit.

import Gdk from 'gi://Gdk?version=4.0';
import GdkPixbuf from 'gi://GdkPixbuf';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';
import Pango from 'gi://Pango';
import PangoCairo from 'gi://PangoCairo';

const [, socketPath, ref, background0, foreground0, font] = ARGV;
let background = background0, foreground = foreground0;

let GioUnix = null;
try {
    GioUnix = (await import('gi://GioUnix')).default;
} catch {
    GioUnix = null;
}
const OutputStream = GioUnix?.OutputStream ?? Gio.UnixOutputStream;
const InputStream = GioUnix?.InputStream ?? Gio.UnixInputStream;

const out = new OutputStream({fd: 1, close_fd: false});
function say(line) {
    try {
        out.write_all(new TextEncoder().encode(`${line}\n`), null);
        out.flush(null);
    } catch {
        // The extension went away: nothing to tell.
    }
}

function rgba(text) {
    const c = new Gdk.RGBA();
    if (!c.parse(text ?? ''))
        c.parse('#000000');
    return c;
}

// A key as the daemon reads it: W3C KeyboardEvent.code names
// (docs/architecture.md, "The grid renderer").
const NAMES = {
    Return: 'Enter', KP_Enter: 'NumpadEnter', Escape: 'Escape', Tab: 'Tab', ISO_Left_Tab: 'Tab',
    BackSpace: 'Backspace', Insert: 'Insert', Delete: 'Delete', Home: 'Home', End: 'End',
    Left: 'ArrowLeft', Up: 'ArrowUp', Right: 'ArrowRight', Down: 'ArrowDown',
    Page_Up: 'PageUp', Page_Down: 'PageDown', space: 'Space', minus: 'Minus', equal: 'Equal',
    bracketleft: 'BracketLeft', bracketright: 'BracketRight', backslash: 'Backslash',
    semicolon: 'Semicolon', apostrophe: 'Quote', comma: 'Comma', period: 'Period',
    slash: 'Slash', grave: 'Backquote',
};
const MODIFIERS = new Set(['Shift_L', 'Shift_R', 'Control_L', 'Control_R', 'Alt_L', 'Alt_R',
    'Super_L', 'Super_R', 'Meta_L', 'Meta_R', 'ISO_Level3_Shift', 'Caps_Lock', 'Num_Lock']);

function modsOf(state) {
    let m = 0;
    if (state & Gdk.ModifierType.SHIFT_MASK)
        m |= 1;
    if (state & Gdk.ModifierType.ALT_MASK)
        m |= 2;
    if (state & Gdk.ModifierType.CONTROL_MASK)
        m |= 4;
    if (state & Gdk.ModifierType.SUPER_MASK)
        m |= 8;
    return m;
}

// { key, mods, text? } for the daemon, or null for a modifier alone.
function keyOf(keyval, state) {
    const name = Gdk.keyval_name(keyval) ?? '';
    if (MODIFIERS.has(name))
        return null;
    let key = NAMES[name] ?? '';
    const lower = Gdk.keyval_name(Gdk.keyval_to_lower(keyval)) ?? '';
    if (!key && /^[a-z]$/.test(lower))
        key = `Key${lower.toUpperCase()}`;
    if (!key && /^[0-9]$/.test(name))
        key = `Digit${name}`;
    if (!key && /^F([1-9]|1[0-9]|2[0-4])$/.test(name))
        key = name;
    let mods = modsOf(state);
    if (name === 'ISO_Left_Tab')
        mods |= 1;
    const k = {key, mods};
    const cp = Gdk.keyval_to_unicode(keyval);
    if (cp >= 0x20 && cp !== 0x7f && !(mods & 4))
        k.text = String.fromCodePoint(cp);
    if (!k.key && k.text === undefined)
        return null;
    return k;
}

const app = new Gtk.Application({application_id: null, flags: Gio.ApplicationFlags.NON_UNIQUE});

app.connect('activate', () => {
    const window = new Gtk.Window({
        application: app, decorated: false, resizable: true,
        title: `notchd ${ref}`, default_width: 480, default_height: 240,
    });
    const area = new Gtk.DrawingArea({hexpand: true, vexpand: true, focusable: true, can_focus: true});
    window.set_child(area);
    const PAD = 8;

    // The font and a cell's size.
    const fontDesc = Pango.FontDescription.from_string(font || 'Monospace 11');
    let cellW = 8, cellH = 16;
    const measure = () => {
        const layout = area.create_pango_layout('M');
        layout.set_font_description(fontDesc);
        const [, logical] = layout.get_extents();
        cellW = Math.max(1, logical.width / Pango.SCALE);
        cellH = Math.max(1, Math.ceil(logical.height / Pango.SCALE));
    };
    measure();

    // What the daemon last said.
    let rows = [];
    let gridRows = 0;
    let cursor = null;
    let mouse = false;
    let images = [];
    const pictures = new Map();
    let exited = false;
    let typed = false;
    let error = '';

    // ------------------------------------------------------------ the wire
    let conn = null;
    let ostream = null;
    let reading = null;
    let sentSize = '';
    const send = msg => {
        if (!ostream)
            return;
        msg.t = 'surface';
        msg.ref = ref;
        try {
            ostream.write_all(new TextEncoder().encode(`${JSON.stringify(msg)}\n`), null);
        } catch {
            // The connection went; the read loop says so.
        }
    };
    const size = () => {
        const w = area.get_width(), h = area.get_height();
        return [Math.max(2, Math.floor((w - 2 * PAD) / cellW)), Math.max(2, Math.floor((h - 2 * PAD) / cellH))];
    };

    const frame = g => {
        if (g.rows !== gridRows || g.full) {
            gridRows = g.rows;
            const old = rows;
            rows = Array.from({length: gridRows}, (_, i) => (g.full ? {runs: [], sel: null} : old[i] ?? {runs: [], sel: null}));
        }
        for (const line of g.lines ?? []) {
            if (line.y < rows.length)
                rows[line.y] = {runs: line.runs ?? [], sel: line.sel ?? null};
        }
        cursor = g.cursor ?? null;
        mouse = g.mouse === true;
        if (g.images !== undefined)
            images = g.images;
        area.queue_draw();
    };

    const picture = (id, b64) => {
        try {
            const bytes = GLib.base64_decode(b64);
            const stream = Gio.MemoryInputStream.new_from_bytes(new GLib.Bytes(bytes));
            pictures.set(id, GdkPixbuf.Pixbuf.new_from_stream(stream, null));
            area.queue_draw();
        } catch (e) {
            printerr(`notchd grid: image ${id}: ${e.message}`);
        }
    };

    const clipboard = () => Gdk.Display.get_default().get_clipboard();

    const handle = line => {
        let m;
        try {
            m = JSON.parse(line);
        } catch {
            return;
        }
        if (m.t === 'surface' && m.ev === 'grid') {
            frame(m);
        } else if (m.t === 'surface' && m.ev === 'image') {
            picture(m.id, m.png);
        } else if (m.t === 'surface' && m.ev === 'copy') {
            clipboard().set_text(String(m.text ?? ''));
        } else if (m.t === 'surface' && m.ev === 'clipboard') {
            clipboard().read_text_async(null, (c, res) => {
                let text = '';
                try {
                    text = c.read_text_finish(res) ?? '';
                } catch {
                    text = '';
                }
                if (text)
                    send({ev: 'paste', text});
            });
        } else if (m.t === 'surface-close') {
            exited = true;
            say('exited');
            area.queue_draw();
        } else if (m.t === 'err') {
            error = String(m.message ?? m.code ?? 'error');
            area.queue_draw();
        }
    };

    const connect = () => {
        try {
            conn?.close(null);
        } catch {
            // Gone already.
        }
        reading?.cancel();
        const client = new Gio.SocketClient();
        try {
            conn = client.connect(Gio.UnixSocketAddress.new(socketPath), null);
        } catch (e) {
            error = `cannot reach notchd: ${e.message}`;
            exited = true;
            say('exited');
            return;
        }
        ostream = conn.get_output_stream();
        const input = new Gio.DataInputStream({base_stream: conn.get_input_stream()});
        const cancel = new Gio.Cancellable();
        reading = cancel;
        images = [];
        pictures.clear();
        exited = false;
        error = '';
        const [cols, rws] = size();
        sentSize = `${cols}x${rws}`;
        const attach = {t: 'attach', ref, cols, rows: rws, render: 'grid', fg: foreground, bg: background,
            cw: Math.round(cellW), ch: Math.round(cellH)};
        try {
            ostream.write_all(new TextEncoder().encode(`${JSON.stringify(attach)}\n`), null);
        } catch {
            // Read below says it ended.
        }
        const next = () => input.read_line_async(GLib.PRIORITY_DEFAULT, cancel, (s, res) => {
            let line = null;
            try {
                [line] = s.read_line_finish_utf8(res);
            } catch {
                line = null;
            }
            if (line === null) {
                if (!cancel.is_cancelled() && !exited) {
                    exited = true;
                    say('exited');
                    area.queue_draw();
                }
                return;
            }
            handle(line);
            next();
        });
        next();
    };

    // The size: once known, the connection; then a resize when it changes.
    area.connect('resize', () => {
        const [cols, rws] = size();
        if (!conn) {
            connect();
            return;
        }
        const s = `${cols}x${rws}`;
        if (s !== sentSize) {
            sentSize = s;
            send({ev: 'resize', cols, rows: rws, cw: Math.round(cellW), ch: Math.round(cellH)});
        }
    });

    // ------------------------------------------------------------ drawing
    const setColor = (cr, text, alpha = 1) => {
        const c = rgba(text);
        cr.setSourceRGBA(c.red, c.green, c.blue, alpha * c.alpha);
    };
    area.set_draw_func((a, cr, w, h) => {
        setColor(cr, background);
        cr.paint();
        const ox = PAD, oy = PAD;
        const below = images.filter(i => i.z < 0), above = images.filter(i => i.z >= 0);
        const drawImages = list => {
            for (const im of list) {
                const pix = pictures.get(im.id);
                if (!pix || !im.sw || !im.sh)
                    continue;
                const x = ox + im.x * cellW + im.xo, y = oy + im.y * cellH + im.yo;
                // The image's `sx`, `sy`, `sw`, `sh` over `w` by `h`.
                cr.save();
                cr.rectangle(x, y, im.w, im.h);
                cr.clip();
                const sx = im.w / im.sw, sy = im.h / im.sh;
                cr.translate(x - im.sx * sx, y - im.sy * sy);
                cr.scale(sx, sy);
                Gdk.cairo_set_source_pixbuf(cr, pix, 0, 0);
                cr.paint();
                cr.restore();
            }
        };
        drawImages(below);
        const layout = PangoCairo.create_layout(cr);
        rows.forEach((row, y) => {
            const ry = oy + y * cellH;
            for (const run of row.runs) {
                if (run.bg) {
                    setColor(cr, run.bg);
                    cr.rectangle(ox + run.x * cellW, ry, run.w * cellW, cellH);
                    cr.fill();
                }
            }
            if (row.sel) {
                setColor(cr, foreground, 0.3);
                cr.rectangle(ox + row.sel[0] * cellW, ry, (row.sel[1] - row.sel[0] + 1) * cellW, cellH);
                cr.fill();
            }
            for (const run of row.runs) {
                if (!run.text.trim())
                    continue;
                const a = run.a ?? 0;
                const fd = fontDesc.copy();
                if (a & 1)
                    fd.set_weight(Pango.Weight.BOLD);
                if (a & 2)
                    fd.set_style(Pango.Style.ITALIC);
                layout.set_font_description(fd);
                const attrs = new Pango.AttrList();
                if (a & 4)
                    attrs.insert(Pango.attr_underline_new(Pango.Underline.SINGLE));
                if (a & 8)
                    attrs.insert(Pango.attr_strikethrough_new(true));
                layout.set_attributes(attrs);
                layout.set_text(run.text, -1);
                setColor(cr, run.fg ?? foreground, a & 16 ? 0.6 : 1);
                cr.moveTo(ox + run.x * cellW, ry);
                PangoCairo.show_layout(cr, layout);
            }
        });
        drawImages(above);
        if (cursor && !exited) {
            const x = ox + cursor.x * cellW, y = oy + cursor.y * cellH;
            const focused = area.has_focus;
            setColor(cr, foreground, cursor.style === 'block' && focused ? 0.55 : 1);
            if (cursor.style === 'bar') {
                cr.rectangle(x, y, 2, cellH);
                cr.fill();
            } else if (cursor.style === 'underline') {
                cr.rectangle(x, y + cellH - 2, cellW, 2);
                cr.fill();
            } else if (cursor.style === 'hollow' || !focused) {
                cr.setLineWidth(1);
                cr.rectangle(x + 0.5, y + 0.5, cellW - 1, cellH - 1);
                cr.stroke();
            } else {
                cr.rectangle(x, y, cellW, cellH);
                cr.fill();
            }
        }
        if (error) {
            layout.set_font_description(fontDesc);
            layout.set_attributes(null);
            layout.set_text(error, -1);
            setColor(cr, foreground, 0.6);
            cr.moveTo(PAD, PAD);
            PangoCairo.show_layout(cr, layout);
        }
        cr.$dispose();
    });

    // ------------------------------------------------------------ keys
    const im = new Gtk.IMMulticontext();
    im.set_client_widget(area);
    im.connect('commit', (c, text) => {
        if (exited || !text)
            return;
        send({ev: 'key', key: '', text});
        if (!typed) {
            typed = true;
            say('typed');
        }
    });
    const keys = new Gtk.EventControllerKey();
    keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE);
    keys.connect('key-pressed', (c, keyval, keycode, state) => {
        const mods = modsOf(state);
        if (keyval === Gdk.KEY_Escape && mods === 4) {
            say('leave');
            return true;
        }
        if (exited) {
            if (keyval === Gdk.KEY_Return || keyval === Gdk.KEY_KP_Enter)
                say('restart');
            return true;
        }
        // What an input method composes (dead keys, IBus) comes as `commit`.
        const ev = c.get_current_event();
        if (!(mods & 4) && ev && im.filter_keypress(ev))
            return true;
        const k = keyOf(keyval, state);
        if (!k)
            return false;
        k.ev = 'key';
        send(k);
        if (!typed) {
            typed = true;
            say('typed');
        }
        return true;
    });
    keys.connect('key-released', c => {
        const ev = c.get_current_event();
        if (ev)
            im.filter_keypress(ev);
    });
    area.add_controller(keys);
    const focus = new Gtk.EventControllerFocus();
    focus.connect('enter', () => {
        im.focus_in();
        area.queue_draw();
    });
    focus.connect('leave', () => {
        im.focus_out();
        area.queue_draw();
    });
    area.add_controller(focus);

    // ------------------------------------------------------------ pointer
    const cellAt = (x, y) => ({x: Math.max(0, (x - PAD) / cellW), y: Math.max(0, (y - PAD) / cellH)});
    const BUTTONS = {1: 'left', 2: 'middle', 3: 'right'};
    let held = '';
    let lastCell = '';
    const sendMouse = (action, button, x, y, state) => {
        const c = cellAt(x, y);
        const m = {ev: 'mouse', action, x: c.x, y: c.y, mods: modsOf(state)};
        if (button)
            m.button = button;
        send(m);
    };
    const click = new Gtk.GestureClick({button: 0});
    click.set_propagation_phase(Gtk.PropagationPhase.CAPTURE);
    click.connect('pressed', (g, n, x, y) => {
        say('keyboard');
        area.grab_focus();
        held = BUTTONS[g.get_current_button()] ?? '';
        lastCell = '';
        if (held)
            sendMouse('press', held, x, y, g.get_current_event_state());
    });
    click.connect('released', (g, n, x, y) => {
        const b = BUTTONS[g.get_current_button()] ?? '';
        held = '';
        if (b)
            sendMouse('release', b, x, y, g.get_current_event_state());
    });
    area.add_controller(click);

    const motion = new Gtk.EventControllerMotion();
    motion.connect('enter', () => say('pointer in'));
    motion.connect('leave', () => say('pointer out'));
    motion.connect('motion', (c, x, y) => {
        if (!held && !mouse)
            return;
        const p = cellAt(x, y);
        const cell = `${Math.floor(p.x)},${Math.floor(p.y)}`;
        if (cell === lastCell)
            return;
        lastCell = cell;
        sendMouse('motion', held, x, y, c.get_current_event_state());
    });
    area.add_controller(motion);

    let wheelRest = 0;
    let pointerAt = {x: 0, y: 0};
    motion.connect('motion', (c, x, y) => {
        pointerAt = {x, y};
    });
    const scroll = new Gtk.EventControllerScroll({flags: Gtk.EventControllerScrollFlags.VERTICAL});
    scroll.connect('scroll', (c, dx, dy) => {
        // A notch is 1; three rows a notch.
        wheelRest += dy * 3;
        const n = wheelRest > 0 ? Math.floor(wheelRest) : Math.ceil(wheelRest);
        if (n !== 0) {
            wheelRest -= n;
            const p = cellAt(pointerAt.x, pointerAt.y);
            send({ev: 'wheel', rows: n, x: p.x, y: p.y});
        }
        return true;
    });
    area.add_controller(scroll);

    // ------------------------------------------------------------ the extension
    const input = new Gio.DataInputStream({base_stream: new InputStream({fd: 0, close_fd: false})});
    const next = () => input.read_line_async(GLib.PRIORITY_DEFAULT, null, (s, res) => {
        let line;
        try {
            [line] = s.read_line_finish_utf8(res);
        } catch {
            line = null;
        }
        if (line === null) {
            app.quit();
            return;
        }
        const [cmd, ...args] = line.trim().split(/\s+/);
        if (cmd === 'colors' && args.length === 2) {
            // New colors go with a new attach: the terminal answers them.
            [background, foreground] = args;
            if (conn)
                connect();
            area.queue_draw();
        } else if (cmd === 'quit') {
            app.quit();
        }
        next();
    });
    next();

    window.present();
    area.grab_focus();
});

app.run([]);
