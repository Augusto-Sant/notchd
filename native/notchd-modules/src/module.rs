//! What a module that lives in the daemon is (docs/modules.md, "A module in
//! the daemon"): state in, facts and view trees out. The daemon (`corehost`)
//! owns the clock, the config and the providers and calls it; a module never
//! reads the time or the system itself, so each is tested as a plain value.
//!
//! Every call that can change something (`config`, `tick`, `event`,
//! `command`, `feed`) is followed by the host asking `facts`, `view` per slot
//! and `take_effects`; the host sends the UIs only what changed.

use notchd_proto::{EventValue, Node, Slot};
use serde_json::Value;

/// `NotchModule`'s properties, as the module says them now.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub priority: i32,
    /// Something to show in compact or expanded right now.
    pub active: bool,
    /// A Nerd Font glyph shown closed while active with no compact view.
    pub icon: String,
    pub attention: bool,
    pub expanded_width: u32,
    pub urgent_peek: bool,
    pub peek_on_focused_screen: bool,
}

impl Default for Facts {
    fn default() -> Facts {
        Facts {
            priority: 0,
            active: false,
            icon: String::new(),
            attention: false,
            expanded_width: 400,
            urgent_peek: false,
            peek_on_focused_screen: false,
        }
    }
}

/// What a module asks of the host, besides its views.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `requestPeek(ms)`.
    Peek(u32),
    /// `dismissPeek()`.
    DismissPeek,
    /// A desktop notification (the `notifications` provider).
    Notify { title: String, body: String },
    /// A command for a media player (the `media` provider).
    Media(notchd_providers::media::MediaCommand),
    /// A program in a terminal (the `pty` provider).
    Pty(PtyCmd),
    /// Wants the plan usage asked (the `usage` provider), on a thread of its
    /// own; the answer comes back as `feed("usage", ..)`.
    Usage(crate::plan::Ask),
    /// Acts on the notches that show this module (`module-notch`).
    Notch(notchd_proto::ui::ModuleOp),
}

/// What a module asks of its terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PtyCmd {
    /// Starts the program (a restart stops the old one first). `run` names
    /// the run: the events the host feeds back carry it, so those of a run
    /// that was replaced are told apart.
    Start(PtyStart),
    /// Stops it; its session is gone.
    Stop,
    /// Bytes to it, as typed.
    Write(Vec<u8>),
}

/// A program to run in a terminal. `dir` is the setting as the user wrote it
/// (`~` and the rest are the host's to expand, since the module never reads
/// the environment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyStart {
    pub run: u64,
    pub program: String,
    pub args: Vec<String>,
    pub dir: String,
}

/// What the host knows that a module's rules need.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx {
    /// Some UI connected to the daemon can draw a terminal (`surfaces` of its
    /// `hello`).
    pub terminal_ui: bool,
    /// Some notch is open (a confirmation peek is not needed then).
    pub expanded: bool,
    /// Some notch shows this module's peek.
    pub peeking: bool,
}

/// A module that lives in the daemon.
pub trait CoreModule: Send {
    fn id(&self) -> &'static str;
    /// The title of the tab and the settings page, for a module that has one
    /// of its own (a terminal's); `None`: [`crate::title`] of its id.
    fn title(&self) -> Option<String> {
        None
    }
    /// Standalone: a notch of its own, no place in a notch's module list.
    fn standalone(&self) -> bool {
        false
    }
    fn facts(&self, now: u64) -> Facts;
    /// The tree of a slot now; `None`: no view for it.
    fn view(&self, slot: Slot, now: u64) -> Option<Node>;
    /// The whole config changed (the module reads its own keys: its settings
    /// are `<id>.<key>`, a missing one its default from its JSON file).
    fn config(&mut self, _cfg: &Value, _now: u64) {}
    /// An IPC method for this module (`timer`, `push`, `clear`, `osd`): the
    /// answer, or `None` when the method is not its own.
    fn command(
        &mut self,
        _ctx: &Ctx,
        _method: &str,
        _args: &[String],
        _now: u64,
    ) -> Option<String> {
        None
    }
    /// A primitive's `action` fired in one of its views.
    fn event(&mut self, _ctx: &Ctx, _action: &str, _value: &EventValue, _now: u64) {}
    /// The time `next_due` named has come.
    fn tick(&mut self, _now: u64) {}
    /// When the module wants `tick` next (epoch ms), `None` for never.
    fn next_due(&self, _now: u64) -> Option<u64> {
        None
    }
    /// What it adds to its entry of the IPC `state` (`usage`).
    fn state_extra(&self) -> Option<Value> {
        None
    }
    /// What it asked since the last call.
    fn take_effects(&mut self) -> Vec<Effect> {
        Vec::new()
    }
    /// A provider's data (`resources`): the module takes what is its own.
    fn feed(&mut self, _provider: &str, _data: &Value, _now: u64) {}
    /// The test hook `test-<id>`: demo data in place of the system's.
    fn demo(&mut self, _args: &[String], _now: u64) -> Option<String> {
        None
    }
}
