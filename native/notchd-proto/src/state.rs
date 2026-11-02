//! The `state` entity: notches and their copies, navigation and reserve targets.

use crate::common::{Edge, Mode, Slot};
use crate::num::Num;
use serde::{Deserialize, Serialize};

/// Whole replacement of the notch state; `rev` only grows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub rev: u64,
    pub navigation: Option<Navigation>,
    pub settings_open: bool,
    pub notches: Vec<Notch>,
    pub reserve: Vec<Reserve>,
    pub standalone: Vec<Standalone>,
    /// The visitor (`NotchService.visitor`): the default notch, a copy per
    /// screen at the top, hidden until it shows a peek where no notch with
    /// that module does. Its copies are named by the notch id `""` in
    /// `pointer`, `key` and `tab`.
    pub visitor: Vec<StandaloneCopy>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NavMode {
    Select,
    Move,
    Remove,
}

/// Navigation mode (`Navigate.js`): which notch is selected and its tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Navigation {
    pub mode: NavMode,
    /// Index into `notches`.
    pub selected: u32,
    /// The tab shown, `""` for the notch's own.
    pub tab: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notch {
    pub id: String,
    pub edge: Edge,
    pub offset: Num,
    /// `""`: the focused screen.
    pub screen: String,
    pub draggable: bool,
    pub tabs: bool,
    pub modules: Vec<String>,
    pub copies: Vec<Copy>,
}

/// One notch on one screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Copy {
    pub screen: String,
    pub mode: Mode,
    pub pinned: bool,
    pub tab: String,
    pub peek: Option<Peek>,
    pub shown: Vec<String>,
    pub covered: bool,
    pub needs_window: bool,
    pub keyboard: bool,
}

/// While set, the UI shows the module's `view` slot; the core clears it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Peek {
    pub module: String,
    pub view: Slot,
    /// Epoch milliseconds.
    pub until: u64,
}

/// A depth to keep free along an edge of a screen; the UI holds windows for it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reserve {
    pub screen: String,
    pub edge: Edge,
    pub depth: Num,
}

/// A standalone module (one with its own window, like `osd`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Standalone {
    pub id: String,
    pub copies: Vec<StandaloneCopy>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StandaloneCopy {
    pub screen: String,
    pub mode: Mode,
    pub peek: Option<Peek>,
    /// `State.needsWindow` for a guest: only while open and for 5 s after.
    pub needs_window: bool,
}
