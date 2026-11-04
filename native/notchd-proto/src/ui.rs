//! Core to UI and UI to core: the two message enums and the messages that are
//! not in another module (handshake, config, view, module facts, raw input).

use crate::Validate;
use crate::common::{Edge, Slot};
use crate::num::Num;
use crate::request::{Edit, ErrReply, EventValue, Ipc, OkReply, Set};
use crate::settings::{Field, Page};
use crate::state::State;
use crate::surface::{Attach, SurfaceMsg, SurfaceOpen, SurfaceRef};
use crate::tree::Node;
use crate::version::{MAX_VIEW_BYTES, PROTO};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Everything the daemon sends on a UI connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum ToUi {
    #[serde(rename = "welcome")]
    Welcome(Welcome),
    #[serde(rename = "config")]
    Config(ConfigUpdate),
    #[serde(rename = "config-error")]
    ConfigError(ConfigError),
    #[serde(rename = "schema")]
    Schema(Schema),
    #[serde(rename = "state")]
    State(State),
    #[serde(rename = "module")]
    Module(ModuleInfo),
    #[serde(rename = "view")]
    View(View),
    #[serde(rename = "surface-open")]
    SurfaceOpen(SurfaceOpen),
    #[serde(rename = "surface")]
    Surface(SurfaceMsg),
    #[serde(rename = "surface-close")]
    SurfaceClose(SurfaceRef),
    #[serde(rename = "ok")]
    Ok(OkReply),
    #[serde(rename = "err")]
    Err(ErrReply),
}

impl ToUi {
    /// Every `t`, for the fixture coverage test.
    pub const TAGS: &'static [&'static str] = &[
        "welcome",
        "config",
        "config-error",
        "schema",
        "state",
        "module",
        "view",
        "surface-open",
        "surface",
        "surface-close",
        "ok",
        "err",
    ];
}

impl Validate for ToUi {
    fn validate(&self) -> Result<(), String> {
        match self {
            ToUi::View(v) => v.validate(),
            ToUi::Welcome(w) if w.proto != PROTO => {
                Err(format!("welcome.proto {} is not {PROTO}", w.proto))
            }
            _ => Ok(()),
        }
    }
}

/// Everything a UI, the CLI or a script sends to the daemon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum ToDaemon {
    #[serde(rename = "hello")]
    Hello(Hello),
    #[serde(rename = "screens")]
    Screens(Screens),
    #[serde(rename = "pointer")]
    Pointer(Pointer),
    #[serde(rename = "drag")]
    Drag(Drag),
    #[serde(rename = "pin")]
    Pin(Pin),
    #[serde(rename = "focus")]
    Focus(Focus),
    #[serde(rename = "facts")]
    Facts(Facts),
    #[serde(rename = "peek")]
    Peek(PeekRequest),
    #[serde(rename = "peek-dismiss")]
    PeekDismiss(PeekDismiss),
    #[serde(rename = "module-notch")]
    ModuleNotch(ModuleNotch),
    #[serde(rename = "key")]
    Key(Key),
    #[serde(rename = "event")]
    Event(Event),
    #[serde(rename = "tab")]
    Tab(Tab),
    #[serde(rename = "settings")]
    Settings(SettingsOpen),
    #[serde(rename = "settings-page")]
    SettingsPage(SettingsPage),
    #[serde(rename = "catalog")]
    Catalog(Catalog),
    #[serde(rename = "reserve-applied")]
    ReserveApplied(ReserveApplied),
    #[serde(rename = "measure")]
    Measure(Measure),
    #[serde(rename = "set")]
    Set(Set),
    #[serde(rename = "edit")]
    Edit(Edit),
    #[serde(rename = "ipc")]
    Ipc(Ipc),
    #[serde(rename = "surface")]
    Surface(SurfaceMsg),
    #[serde(rename = "attach")]
    Attach(Attach),
}

impl ToDaemon {
    /// Every `t`, for the fixture coverage test.
    pub const TAGS: &'static [&'static str] = &[
        "hello",
        "screens",
        "pointer",
        "drag",
        "pin",
        "focus",
        "facts",
        "peek",
        "peek-dismiss",
        "module-notch",
        "key",
        "event",
        "tab",
        "settings",
        "settings-page",
        "catalog",
        "reserve-applied",
        "measure",
        "set",
        "edit",
        "ipc",
        "surface",
        "attach",
    ];
}

impl Validate for ToDaemon {
    fn validate(&self) -> Result<(), String> {
        match self {
            ToDaemon::Drag(d) if d.ev == DragEv::Drop && d.position.is_none() => {
                Err("drag drop needs a position".into())
            }
            ToDaemon::Drag(d) if d.ev != DragEv::Drop && d.position.is_some() => {
                Err("only a drop has a position".into())
            }
            _ => Ok(()),
        }
    }
}

/// What the UI can draw. Names are strings so a UI may list primitives a
/// daemon does not know yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Caps {
    pub primitives: Vec<String>,
    pub surfaces: Vec<String>,
    pub webview: bool,
    pub canvas: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub proto: u32,
    pub client: String,
    pub client_version: String,
    pub instance: String,
    pub caps: Caps,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    pub proto: u32,
    pub daemon: String,
    pub instance: String,
    /// The config file's path.
    pub config: String,
    /// The theme token set the UI maps to its own source.
    pub tokens: Vec<String>,
    /// The daemon's own binary (`notchd attach`, a session in a terminal of
    /// your own; see the free surface).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
}

/// The resolved config: `cfg` as in the file, `resolved` with defaults filled
/// in for reading. Both are free-form (keys serialize sorted).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigUpdate {
    pub rev: u64,
    pub cfg: Value,
    pub resolved: Value,
}

/// The file did not parse; the last good config stays and the file is untouched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigError {
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    pub rev: u64,
    pub pages: Vec<Page>,
    pub fields: Vec<Field>,
    /// The modules the settings pages are made of, built-in, host's and
    /// user's alike: `{ moduleId, title, settings, standalone, agent, live,
    /// terminal }` each (the shape of a `catalog` entry). It is how a UI that
    /// does not ship the built-ins' JSON files learns their titles, kinds and
    /// setting defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modules: Option<Vec<Value>>,
}

/// One module slot's tree; `null` when the module has no view for the slot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub module: String,
    pub slot: Slot,
    pub rev: u64,
    pub tree: Option<Node>,
}

impl Validate for View {
    fn validate(&self) -> Result<(), String> {
        let Some(tree) = &self.tree else {
            return Ok(());
        };
        tree.validate()?;
        let bytes = serde_json::to_string(tree).map_or(0, |s| s.len());
        if bytes > MAX_VIEW_BYTES {
            return Err(format!("view is {bytes} bytes, over {MAX_VIEW_BYTES}"));
        }
        Ok(())
    }
}

/// One module's facts (the `NotchModule` properties).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleInfo {
    pub id: String,
    pub rev: u64,
    pub title: String,
    pub priority: i32,
    pub active: bool,
    pub icon: String,
    pub attention: bool,
    pub expanded_width: u32,
    pub urgent_peek: bool,
    pub peek_on_focused_screen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: Num,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
}

/// The adapter contract of `GenericCompositor.qml`, reported instead of called.
/// Sent on hello and on any change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screens {
    pub screens: Vec<ScreenInfo>,
    pub focused: String,
    pub fullscreen: Vec<String>,
    /// Screen name to its active workspace (keys serialize sorted).
    pub workspaces: BTreeMap<String, Workspace>,
    pub known: Vec<String>,
    pub supports_workspaces: bool,
    /// The bar's edge, when there is a bar.
    pub bar_edge: Option<Edge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerEv {
    Enter,
    Leave,
    Press,
}

/// Raw pointer facts; the core owns the delays and decides `mode`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pointer {
    pub notch: String,
    pub screen: String,
    pub ev: PointerEv,
}

/// `start`: the card is picked up (it stays closed and nothing opens it);
/// `drop`: it landed at `position`; `cancel`: it was let go where it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragEv {
    Start,
    Drop,
    Cancel,
}

/// Where a notch was dropped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DropPosition {
    pub edge: Edge,
    pub offset: Num,
    pub screen: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drag {
    pub notch: String,
    pub screen: String,
    pub ev: DragEv,
    /// Only for a `drop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<DropPosition>,
}

/// A copy pinned open or closed by something other than a click on it: a
/// click away from it, Escape, a module that wants the keyboard. `on: false`
/// also ends its hover (`NotchCopy.close`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub notch: String,
    pub screen: String,
    pub on: bool,
}

/// A copy's window gained (`active`) or lost the keyboard. The core keeps
/// whether it had it, and answers with `keyboard` and, when a window took the
/// keyboard from an open notch, by closing it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Focus {
    pub notch: String,
    pub screen: String,
    pub active: bool,
}

/// One module the UI loads itself, as the rules see it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleFacts {
    pub id: String,
    pub priority: i32,
    pub active: bool,
    /// Has a `compact` view.
    pub compact: bool,
    /// Has a `peek` view.
    pub peek: bool,
    /// Has an icon to show closed.
    pub icon: bool,
    /// Has an `expanded` view.
    pub expanded: bool,
    /// News not seen yet.
    pub attention: bool,
    pub urgent_peek: bool,
    pub peek_on_focused_screen: bool,
    /// Has a notch of its own.
    pub standalone: bool,
}

/// The facts of every module the UI loads itself (a user module's come from
/// its process): replaces the last list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    pub modules: Vec<ModuleFacts>,
}

/// `NotchModule.requestPeek`: the module asks to peek for `ms` (the default
/// when absent).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeekRequest {
    pub module: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<Num>,
}

/// `NotchModule.dismissPeek`: the module's peek goes (every peek when
/// absent).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeekDismiss {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleOp {
    /// `openNotch`: the first notch that shows the module opens on its tab.
    Open,
    /// `closeNotch`: every notch that shows it closes.
    Close,
    /// `requestKeyboard`: every copy open on the module pins, so its window
    /// takes the keyboard.
    Keyboard,
}

/// A module acts on the notches that show it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleNotch {
    pub module: String,
    pub op: ModuleOp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Shift,
    Ctrl,
    Alt,
    Meta,
}

/// A key while a notch holds the keyboard: a normalized name (`"a"`, `"Escape"`,
/// `"Tab"`, `"Left"`, `"Return"`, `"1"`), not a Qt key code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub notch: String,
    pub screen: String,
    pub key: String,
    pub mods: Vec<Modifier>,
    /// The typed text, `""` for a key that types none.
    pub text: String,
}

/// A primitive's `action` fired.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub module: String,
    pub slot: Slot,
    pub action: String,
    pub value: EventValue,
}

/// The user picked a tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub notch: String,
    pub screen: String,
    pub module: String,
}

/// The settings window opened or closed, and on which page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsOpen {
    pub open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// Updates the page shown by an existing settings window without opening it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsPage {
    pub page: String,
}

/// The modules of the host that the daemon cannot know (the Omarchy `osd`,
/// a terminal of the config): `{ moduleId, title, settings, standalone, agent,
/// live, terminal }` each, free-form like a module's settings fields. The
/// built-in modules are the daemon's own (their JSON files are embedded), so a
/// UI does not send them; one that does wins on the same id. The daemon
/// builds the settings pages that depend on them (which lists the Modules
/// page has, what a notch's module list says). It replaces the last list; a
/// user module's own entry (from its `module.json`) is not in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub modules: Vec<Value>,
    /// What a notch lists when the config lists none: the host's
    /// `defaultModules` (the daemon's own pair, `media` and `activities`,
    /// when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_modules: Option<Vec<String>>,
    /// Deprecated, still accepted within proto 1: a folder with the sidecars
    /// (`live/`, `components/`) for the daemon to run instead of its own,
    /// which it ships embedded. A UI omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_dir: Option<String>,
}

/// The reserve target the UI now has mapped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReserveApplied {
    pub screen: String,
    pub edge: Edge,
    pub depth: Num,
}

/// The depths the UI measured for one notch copy, which the core cannot know
/// (`ReservedSpace.qml`'s inputs): the core turns them into `state.reserve`
/// with `State.reservedSpace`, `reserveStep` and `reserveZones`. Sent when a
/// depth changes. `screen` is as in `pointer`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub notch: String,
    pub screen: String,
    /// The rest lip's depth in px (`NotchCard.restDepth`).
    pub rest: Num,
    /// The depth of the compact view now shown, 0 when none
    /// (`NotchCard.compactDepth`); the core keeps the deepest seen.
    pub compact: Num,
    /// The least depth reserved for a compact view (`compactBaseline`: the
    /// card's padding and a media view's height; 0 on a side edge).
    pub baseline: Num,
}
