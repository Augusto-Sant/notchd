//! Version and limit constants of the protocol.

/// The protocol's major version: `hello.proto`, `welcome.proto`, `init.proto`.
/// A different major is refused (`err` code `proto`). Minor additions are new
/// optional fields and new `t`s, so they do not change this number.
pub const PROTO: u32 = 1;

/// Longest line on the core to UI socket, in bytes (4 MiB).
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
/// Longest line from a user module, in bytes (1 MiB).
pub const MAX_MODULE_LINE_BYTES: usize = 1024 * 1024;
/// Views a module may send per second before the core drops the excess.
pub const MAX_VIEWS_PER_SECOND: u32 = 20;
/// The fastest a `canvas` may animate.
pub const MAX_CANVAS_FPS: u32 = 30;
/// Tree limits (Open question 13's suggestion): nodes, nesting, serialized bytes.
pub const MAX_TREE_NODES: usize = 2000;
pub const MAX_TREE_DEPTH: usize = 16;
pub const MAX_VIEW_BYTES: usize = 256 * 1024;

/// `err` codes the daemon uses.
pub const ERR_CODES: &[&str] = &["proto", "module", "method", "args", "key", "config"];

/// Every primitive `t`, in the order of the doc's table.
pub const PRIMITIVES: &[&str] = &[
    "row",
    "column",
    "stack",
    "spacer",
    "text",
    "icon",
    "image",
    "progress",
    "meter",
    "equalizer",
    "list",
    "button",
    "chip",
    "canvas",
    "surface",
];

/// Every canvas command `c`.
pub const CANVAS_COMMANDS: &[&str] = &["rect", "circle", "line", "path", "text", "clip", "unclip"];

/// The IPC methods of docs/ipc.md that the `ipc` request carries.
pub const IPC_METHODS: &[&str] = &[
    "push",
    "clear",
    "open",
    "toggle",
    "close",
    "openNotch",
    "closeNotch",
    "toggleNotch",
    "agent",
    "claude",
    "codex",
    "terminal",
    "timer",
    "live",
    "navigate",
    "settings",
    "recenter",
    "set",
    "state",
    "component",
];

/// Provider names (`subscribe`, `provider`).
pub const PROVIDERS: &[&str] = &[
    "media",
    "resources",
    "screens",
    "usage",
    "audio",
    "notifications",
    "components",
    "osd",
];

/// The providers a user module can subscribe to: the ones the daemon has
/// data for. The rest of [`PROVIDERS`] are named by the protocol but have no
/// feed for modules yet, so a subscribe to them is answered with `error`.
pub const SUBSCRIBABLE: &[&str] = &["media", "resources", "screens", "usage"];
