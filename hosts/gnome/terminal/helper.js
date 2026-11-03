// The terminal view of the GNOME UI (docs/architecture.md, "Free surface"):
// a GTK 4 window with a VTE terminal running `notchd attach <ref>`, the
// daemon's own client of a terminal surface, as the QML view runs it in
// qmltermwidget. The program lives in the daemon; this is only a window
// onto it. The extension starts it (hosts/gnome/terminal.js) and keeps the
// window over the notch: above, on every workspace, out of the window list.
//
//   gjs -m helper.js <exe> <socket> <ref> <background> <foreground> [font]
//
// It tells the extension what happens, a line each on its standard output:
//   typed            a key went to the program (once)
//   leave            Ctrl+Escape: the way out (the program never sees it)
//   keyboard         a press in the terminal: the notch takes the keyboard
//   pointer in|out   the pointer entered or left the window
//   exited           the program ended; Enter then says `restart`
//   restart
// and reads, a line each on its standard input:
//   colors <background> <foreground>
//   quit

import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';
import Pango from 'gi://Pango';
import Vte from 'gi://Vte?version=3.91';

const [exe, socket, ref, background, foreground, font] = ARGV;

// Tango, as the QML view (core/Terminal.js's SCHEME): its bright black is a
// dark gray, which Codex paints your messages with.
const TANGO = [
    '#000000', '#cc0000', '#4e9a06', '#c4a000', '#3465a4', '#75507b', '#06989a', '#d3d7cf',
    '#555753', '#ef2929', '#8ae234', '#fce94f', '#729fcf', '#ad7fa8', '#34e2e2', '#eeeeec',
];

// GioUnix from GLib 2.80; Gio's own before (and deprecated after).
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
    if (!c.parse(text))
        c.parse('#000000');
    return c;
}

const app = new Gtk.Application({
    application_id: null,
    flags: Gio.ApplicationFlags.NON_UNIQUE,
});

app.connect('activate', () => {
    const window = new Gtk.Window({
        application: app, decorated: false, resizable: true,
        title: `notchd ${ref}`, default_width: 480, default_height: 240,
    });
    const term = new Vte.Terminal({
        hexpand: true, vexpand: true, scrollback_lines: 5000,
        margin_top: 8, margin_bottom: 8, margin_start: 8, margin_end: 8,
    });
    if (font)
        term.set_font(Pango.FontDescription.from_string(font));
    const paint = (bg, fg) => {
        term.set_colors(rgba(fg), rgba(bg), TANGO.map(rgba));
        const css = new Gtk.CssProvider();
        const rule = `window { background-color: ${bg}; }`;
        // GTK 4.12 and later take a string; before, bytes.
        if (css.load_from_string)
            css.load_from_string(rule);
        else
            css.load_from_data(rule, -1);
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), css,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);
    };
    paint(background, foreground);
    window.set_child(term);

    let typed = false;
    let exited = false;
    const keys = new Gtk.EventControllerKey();
    keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE);
    keys.connect('key-pressed', (c, keyval, keycode, state) => {
        const mods = state & (Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.SHIFT_MASK |
            Gdk.ModifierType.ALT_MASK | Gdk.ModifierType.SUPER_MASK);
        if (keyval === Gdk.KEY_Escape && mods === Gdk.ModifierType.CONTROL_MASK) {
            say('leave');
            return true;
        }
        if (exited) {
            if (keyval === Gdk.KEY_Return || keyval === Gdk.KEY_KP_Enter)
                say('restart');
            return true;
        }
        if (!typed) {
            typed = true;
            say('typed');
        }
        return false;
    });
    window.add_controller(keys);

    const motion = new Gtk.EventControllerMotion();
    motion.connect('enter', () => say('pointer in'));
    motion.connect('leave', () => say('pointer out'));
    window.add_controller(motion);

    const click = new Gtk.GestureClick();
    click.set_propagation_phase(Gtk.PropagationPhase.CAPTURE);
    click.connect('pressed', () => {
        say('keyboard');
        term.grab_focus();
    });
    window.add_controller(click);

    term.connect('child-exited', () => {
        exited = true;
        say('exited');
    });
    term.spawn_async(Vte.PtyFlags.DEFAULT, null, [exe, '--socket', socket, 'attach', ref], null,
        GLib.SpawnFlags.DEFAULT, null, -1, null, (t, pid, error) => {
            if (error) {
                printerr(`notchd terminal: ${error.message}`);
                exited = true;
                say('exited');
            }
        });

    // The extension's commands.
    const input = new Gio.DataInputStream({base_stream: new InputStream({fd: 0, close_fd: false})});
    const next = () => input.read_line_async(GLib.PRIORITY_DEFAULT, null, (s, res) => {
        let line;
        try {
            [line] = s.read_line_finish_utf8(res);
        } catch {
            line = null;
        }
        if (line === null) {
            // The extension is gone: so is its window.
            app.quit();
            return;
        }
        const [cmd, ...args] = line.trim().split(/\s+/);
        if (cmd === 'colors' && args.length === 2)
            paint(args[0], args[1]);
        else if (cmd === 'quit')
            app.quit();
        next();
    });
    next();

    window.present();
    term.grab_focus();
});

app.run([]);
