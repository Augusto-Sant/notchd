// The terminal surfaces of the GNOME UI (docs/architecture.md, "Free
// surface", "GNOME UI"): a window per terminal, from the helper in
// terminal/helper.js (GTK 4 and VTE running `notchd attach <ref>`), which
// the extension keeps over the notch: above, on every workspace, out of the
// window list, and where the view's `surface` node is. The notch leaves a
// hole there, so the window shows through it and takes the pointer.
//
// A window is started through Meta.WaylandClient, the way the shell knows a
// window is its own: GNOME 49 and later have `new_subprocess` and the
// window's own `hide_from_window_list`; GNOME 46 has `new` and `spawnv`,
// and hides the window through the client.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// Whether this system can run the helper: gjs, GTK 4 and VTE for GTK 4
// (Ubuntu: gir1.2-vte-3.91; Fedora: vte291-gtk4). `done(bool)`.
export function probe(done) {
    const gjs = GLib.find_program_in_path('gjs');
    if (!gjs) {
        done(false);
        return;
    }
    try {
        const proc = Gio.Subprocess.new([gjs, '-c',
            "imports.gi.versions.Gtk = '4.0'; imports.gi.versions.Vte = '3.91'; imports.gi.Gtk; imports.gi.Vte;"],
        Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
        proc.wait_check_async(null, (p, res) => {
            let ok = false;
            try {
                ok = p.wait_check_finish(res);
            } catch {
                ok = false;
            }
            done(ok);
        });
    } catch {
        done(false);
    }
}

// One helper window, for one run of one surface.
class TerminalWindow {
    // `argv`: the helper's command line. `onLine(line)`: what it says.
    // `onPlaced(bool)`: whether it is on screen exactly where it was put
    // (the notch opens its hole only then).
    constructor(argv, onLine, onPlaced) {
        this._onLine = onLine;
        this._onPlaced = onPlaced;
        this.placed = false;
        this.window = null;
        this._rect = null;
        this._shown = false;
        const launcher = new Gio.SubprocessLauncher({
            flags: Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDOUT_PIPE,
        });
        if (Meta.WaylandClient.new_subprocess) {
            this._client = Meta.WaylandClient.new_subprocess(global.context, launcher, argv);
            this._proc = this._client.get_subprocess();
        } else {
            this._client = Meta.WaylandClient.new(global.context, launcher);
            this._proc = this._client.spawnv(global.display, argv);
        }
        // Taken over once it is shown: moving or restacking a Wayland window
        // before its first frame took the shell down on GNOME 46.
        this._created = global.display.connect('window-created', (d, win) => {
            if (!this._client.owns_window(win))
                return;
            // A helper being let go of may still map its window: never seen.
            if (this._dying) {
                win.get_compositor_private()?.hide();
                return;
            }
            if (this.window || this._pendingWin)
                return;
            // Not seen until it is where it belongs: GTK shows it first at its
            // own size, in the middle of the screen.
            const actor = win.get_compositor_private();
            if (actor) {
                Main.wm.skipNextEffect(actor);
                actor.opacity = 0;
            }
            this._pendingWin = win;
            this._pendingId = win.connect('shown', () => {
                win.disconnect(this._pendingId);
                this._pendingWin = null;
                this._pendingId = 0;
                this._adopt(win);
            });
        });
        this._stdin = this._proc.get_stdin_pipe();
        this._stdout = new Gio.DataInputStream({base_stream: this._proc.get_stdout_pipe()});
        this._cancel = new Gio.Cancellable();
        this._read();
    }

    _read() {
        this._stdout.read_line_async(GLib.PRIORITY_DEFAULT, this._cancel, (s, res) => {
            let line = null;
            try {
                [line] = s.read_line_finish_utf8(res);
            } catch {
                line = null;
            }
            if (line === null)
                return;
            this._onLine(line.trim());
            this._read();
        });
    }

    _adopt(win) {
        this.window = win;
        if (win.hide_from_window_list)
            win.hide_from_window_list();
        else
            this._client.hide_from_window_list?.(win);
        win.make_above();
        win.stick();
        this._unmanaged = win.connect('unmanaged', () => {
            this.window = null;
        });
        // The client sizes its window itself at first (its default size):
        // back over the frame whenever it moves away from it.
        this._moved = [
            win.connect('size-changed', () => this._keep()),
            win.connect('position-changed', () => this._keep()),
        ];
        this._apply();
    }

    // Over `rect` ({x, y, w, h}, stage pixels), or hidden with null.
    place(rect) {
        const same = (a, b) => a === b || (a && b && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h);
        if (same(rect, this._rect) && this.window && !this._wantFocus)
            return;
        this._rect = rect;
        if (!rect)
            this._wantFocus = false;
        this._apply();
    }

    // Hidden, the window's actor is: a window out of the window list cannot
    // be minimized (mutter says no), and a hidden actor is drawn nowhere and
    // takes no pointer. The keyboard goes back to the window that had it.
    // Put somewhere, it stays transparent until its frame is there (the
    // client takes the new size on its next frame).
    _apply() {
        const win = this.window;
        if (!win)
            return;
        const actor = win.get_compositor_private();
        if (!this._rect) {
            this.hidden = true;
            // Out of the way while hidden: below every window, so the
            // shell never picks it to focus (an above window was the one it
            // gave the keyboard to when a click landed on no window).
            win.unmake_above();
            win.lower();
            if (win.has_focus())
                this._giveBackFocus();
            actor?.hide();
            this._setPlaced(false);
            return;
        }
        if (this.hidden || !actor?.visible) {
            if (actor)
                actor.opacity = 0;
            actor?.show();
            win.make_above();
            win.raise();
            this.hidden = false;
        }
        if (!this._matches())
            this._move();
        this._check();
    }

    _matches() {
        const r = this._rect;
        const f = this.window?.get_frame_rect();
        return !!r && !!f && f.x === Math.round(r.x) && f.y === Math.round(r.y) &&
            f.width === Math.round(r.w) && f.height === Math.round(r.h);
    }

    // As a user's move: a program's own placement is kept clear of the
    // struts, which pushed the window out of a notch on a side edge that
    // reserves its strip.
    _move() {
        const r = this._rect;
        this.window.move_resize_frame(true, Math.round(r.x), Math.round(r.y), Math.round(r.w), Math.round(r.h));
    }

    // In place: seen, and the keyboard if it was asked for.
    _check() {
        const win = this.window;
        if (!win || this.hidden || !this._rect)
            return;
        if (!this._matches()) {
            this._setPlaced(false);
            return;
        }
        const actor = win.get_compositor_private();
        if (actor)
            actor.opacity = 255;
        this._setPlaced(true);
        if (this._wantFocus && !win.has_focus()) {
            const before = global.display.focus_window;
            if (before && before !== win)
                this._before = before;
            win.activate(global.get_current_time());
        }
    }

    _setPlaced(placed) {
        if (placed === this.placed)
            return;
        this.placed = placed;
        this._onPlaced(placed);
    }

    giveBackFocus() {
        this._giveBackFocus();
    }

    _giveBackFocus() {
        const before = this._before;
        this._before = null;
        // No window to go back to: the keyboard to no window. Activating the
        // workspace focused its topmost window, which could be this one.
        if (before && !before.minimized && before.get_workspace?.() !== null)
            before.activate(global.get_current_time());
        else
            global.display.unset_input_focus(global.get_current_time());
    }

    // The client moved or sized it (its own size at first): back over the
    // frame, or seen now that it is there.
    _keep() {
        if (!this._rect || this.hidden || !this.window)
            return;
        if (!this._matches())
            this._move();
        this._check();
    }

    // The keyboard to the window: now if it is on screen, else once it is.
    focus() {
        this._wantFocus = true;
        this._apply();
    }

    unfocus() {
        this._wantFocus = false;
        if (this.window?.has_focus())
            this._giveBackFocus();
    }

    get focused() {
        return !!this.window?.has_focus();
    }

    // Its window, also before it is shown and taken over (it may have the
    // keyboard by then).
    owns(win) {
        return !!win && (win === this.window || this._client.owns_window(win));
    }

    send(line) {
        try {
            this._stdin.write_all(new TextEncoder().encode(`${line}\n`), null);
        } catch {
            // The helper is gone.
        }
    }

    // Gone at once: its window hidden now (and any it maps until its
    // process ends: a helper still starting maps one after this), the
    // process told to end and made to after a second.
    destroy() {
        this._dying = true;
        this._cancel.cancel();
        if (this._pendingWin) {
            this._pendingWin.disconnect(this._pendingId);
            this._pendingWin.get_compositor_private()?.hide();
        }
        this._pendingWin = null;
        if (this.window) {
            this.window.disconnect(this._unmanaged);
            for (const id of this._moved ?? [])
                this.window.disconnect(id);
            if (this.window.has_focus())
                this._giveBackFocus();
            this.window.get_compositor_private()?.hide();
        }
        this._setPlaced(false);
        this.send('quit');
        try {
            this._stdin.close(null);
        } catch {
            // Closed already.
        }
        const proc = this._proc;
        const created = this._created;
        proc.send_signal(15);
        const kill = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
            proc.force_exit();
            return GLib.SOURCE_REMOVE;
        });
        proc.wait_async(null, () => {
            GLib.source_remove(kill);
            global.display.disconnect(created);
        });
        this.window = null;
    }
}

// The terminal windows, by surface ref. A notch copy showing a surface owns
// its window while it does (`show`), hears what the window says and hides
// it again (`hide`).
export class Terminals {
    // `host`: { exe(), socket(), theme, helper (the helper's path) }.
    constructor(host) {
        this._host = host;
        this._windows = new Map();
        // Mutter may give a hidden window the keyboard (the topmost window
        // when a click lands on no window): never keep it there.
        this._focusId = global.display.connect('notify::focus-window', () => {
            const win = global.display.focus_window;
            for (const entry of this._windows.values()) {
                if (entry.window.owns(win) && entry.window.hidden)
                    entry.window.giveBackFocus();
            }
        });
    }

    // The helper for the renderer: VTE (helper.js), or the grid's (grid.js,
    // `renderer: ghostty`).
    _argv(ref, renderer) {
        const theme = this._host.theme;
        const color = name => theme.css(name).replace(/\s+/g, '');
        const helper = renderer === 'ghostty' && this._host.grid ? this._host.grid : this._host.helper;
        return [GLib.find_program_in_path('gjs') ?? 'gjs', '-m', helper,
            this._host.exe(), this._host.socket(), ref, color('background'), color('foreground'), ''];
    }

    // The window of `ref` for the run `run` over `rect`, for `owner`
    // ({ onLine(line) }), drawn by `renderer`'s helper: a new run or another
    // renderer starts a new window.
    show(ref, run, rect, owner, renderer = 'widget') {
        let entry = this._windows.get(ref);
        // A new run (restart) is a new window; the old one goes now, and
        // its owner's hole closes until the new one is in place.
        if (entry && (entry.run !== run || entry.renderer !== renderer)) {
            entry.window.destroy();
            this._windows.delete(ref);
            entry = null;
        }
        if (!entry) {
            if (!this._host.exe())
                return;
            entry = {run, owner, renderer};
            // `placed` false comes from a window let go of: its last owner
            // hears it, to close its hole.
            entry.window = new TerminalWindow(this._argv(ref, renderer), line => entry.owner?.onLine(line),
                placed => (entry.owner ?? entry.lastOwner)?.onPlaced(placed));
            this._windows.set(ref, entry);
        }
        entry.owner = owner;
        entry.window.place(rect);
        // A window already in place tells its new owner so.
        if (entry.window.placed)
            owner.onPlaced(true);
    }

    // Hidden, if `owner` still owns it; the program goes on in the daemon
    // and the window stays for next time.
    hide(ref, owner) {
        const entry = this._windows.get(ref);
        if (!entry || (owner && entry.owner !== owner))
            return;
        entry.lastOwner = entry.owner;
        entry.owner = null;
        entry.window.place(null);
    }

    focus(ref) {
        for (const [r, entry] of this._windows)
            entry.window[r === ref ? 'focus' : 'unfocus']();
    }

    unfocus() {
        for (const entry of this._windows.values())
            entry.window.unfocus();
    }

    // For the test hook: each window, where it is and whether it has the
    // keyboard.
    describe() {
        return [...this._windows].map(([ref, entry]) => {
            const win = entry.window.window;
            const r = win?.get_frame_rect();
            return {ref, run: entry.run, window: !!win, hidden: win ? !win.get_compositor_private()?.visible : null, focused: entry.window.focused,
                ...(r ? {x: r.x, y: r.y, w: r.width, h: r.height} : {})};
        });
    }

    owns(win) {
        for (const entry of this._windows.values()) {
            if (entry.window.owns(win))
                return true;
        }
        return false;
    }

    // The theme changed: every window takes its colors.
    recolor() {
        const theme = this._host.theme;
        const color = name => theme.css(name).replace(/\s+/g, '');
        for (const entry of this._windows.values())
            entry.window.send(`colors ${color('background')} ${color('foreground')}`);
    }

    destroy() {
        global.display.disconnect(this._focusId);
        for (const entry of this._windows.values())
            entry.window.destroy();
        this._windows.clear();
    }
}
