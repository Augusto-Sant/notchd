// What the shell's own chrome takes of each monitor's edges: the top panel
// and any dock (the Ubuntu Dock, Dash to Dock, Dash to Panel...), wherever
// it is. A notch hangs past them and runs beside them (Position.place's
// `insets`), as the standalone host does with its bar, and its reserve adds
// to theirs.
//
// Read from the chrome actors that affect the struts, as GNOME reads them
// to make the work area: an actor along a whole edge of a monitor takes
// that edge, wide ones the top or bottom, tall ones a side. The extension's
// own actors (named notchd-*) are left out, so its reserve never counts
// itself. A dock that hides itself (autohide, intellihide) reserves
// nothing and takes nothing here: the notch may then be under it while it
// shows, as a window would.

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

// The actor's rectangle on the stage, its allocations summed (transformed
// extents are NaN in GNOME 46's headless shell).
function stageBox(actor) {
    let x = 0, y = 0;
    for (let a = actor; a && a !== global.stage; a = a.get_parent()) {
        const box = a.get_allocation_box();
        x += box.x1;
        y += box.y1;
    }
    const own = actor.get_allocation_box();
    return {x1: x, y1: y, x2: x + own.x2 - own.x1, y2: y + own.y2 - own.y1};
}

// {top, bottom, left, right} in pixels for a monitor of the layout manager.
export function edgeInsets(monitor) {
    const out = {top: 0, bottom: 0, left: 0, right: 0};
    if (!monitor)
        return out;
    const mx2 = monitor.x + monitor.width, my2 = monitor.y + monitor.height;
    for (const tracked of Main.layoutManager._trackedActors ?? []) {
        const actor = tracked.actor;
        if (!tracked.affectsStruts || !actor || !actor.visible || actor.name?.startsWith('notchd-'))
            continue;
        const {x1, y1, x2, y2} = stageBox(actor);
        if (!(x2 > x1 && y2 > y1) || x2 <= monitor.x || x1 >= mx2 || y2 <= monitor.y || y1 >= my2)
            continue;
        const wide = x2 - x1 >= y2 - y1;
        if (wide && y1 <= monitor.y)
            out.top = Math.max(out.top, Math.min(y2, my2) - monitor.y);
        else if (wide && y2 >= my2)
            out.bottom = Math.max(out.bottom, my2 - Math.max(y1, monitor.y));
        else if (!wide && x1 <= monitor.x)
            out.left = Math.max(out.left, Math.min(x2, mx2) - monitor.x);
        else if (!wide && x2 >= mx2)
            out.right = Math.max(out.right, mx2 - Math.max(x1, monitor.x));
    }
    return out;
}
