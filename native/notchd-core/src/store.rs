//! The daemon's in-memory state: the config (with its `rev`), the screens the
//! UI reports, the facts of every module, what each notch copy is doing
//! (hover, pin, peek, tab), navigation, and from all of it the computed
//! `state` entity.
//!
//! Pure: no clock, no socket, no file. Time comes in as `now` (milliseconds,
//! any monotonic clock; peeks' `until` is on that clock, which the daemon
//! makes epoch milliseconds), every decision is `state.rs`, `peek.rs`,
//! `navigate.rs` or `config.rs`, and what has to reach the outside is taken
//! out by [`Store::take_changes`] (messages to broadcast) and
//! [`Store::take_write`] (a config to write). `NotchService.qml`,
//! `Notch.qml` and `NotchCopy.qml` are the model; each section names its
//! source.

use crate::config::{self, NotchSpec};
use crate::navigate::{self, Action, KeyName, NavMode, SHIFT, StepState};
use crate::peek::{self, CopyPeek, Missed};
use crate::position;
use crate::schema;
use crate::settings;
use crate::state::{
    self, KeyboardContext, ModeContext, ModuleInfo, ScreenContext, ScreenState, WorkspaceKey,
};
use crate::timers::Timers;
use notchd_proto::Num;
use notchd_proto::common::{Edge, Mode, Slot};
use notchd_proto::state as ps;
use notchd_proto::ui::{ConfigError, ConfigUpdate, Modifier, PointerEv, Schema, Screens};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// What `NotchService` has when no config lists any (`defaultModules`).
pub const DEFAULT_MODULES: [&str; 2] = ["media", "activities"];
/// `navSettleTimer`: how long after navigation ends a lost keyboard is asked
/// for again instead of closing the notch.
pub const NAV_SETTLE_MS: u64 = 600;
/// `NotchCopy.windowLinger`: how long a guest's window stays after it closes.
pub const WINDOW_LINGER_MS: u64 = 5000;
/// `ReservedSpace.shrinkDelay`: NotchCard's depth animation (380 ms), plus a
/// little.
pub const RESERVE_SHRINK_MS: u64 = 400;
/// The key of the visitor's copies (no module id or notch id can start with
/// `#`).
const VISITOR_KEY: &str = "#visitor";

/// A module's facts as the store needs them: the rules' view of it
/// ([`ModuleInfo`], `id` is its module id) and what only placing it needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleFacts {
    pub info: ModuleInfo,
    /// Has an expanded view.
    pub expanded: bool,
    /// Its peek only shows once, on the focused screen.
    pub peek_on_focused_screen: bool,
    /// Has a notch of its own instead of a place in the notches.
    pub standalone: bool,
}

impl ModuleFacts {
    pub fn new(id: &str) -> ModuleFacts {
        ModuleFacts {
            info: ModuleInfo {
                id: id.to_string(),
                module_id: Some(id.to_string()),
                active: true,
                ..ModuleInfo::default()
            },
            ..ModuleFacts::default()
        }
    }
}

/// A copy is named by its notch (a standalone module's by `@<id>`) and its
/// screen, `""` for the one copy that moves.
type Key = (String, String);

#[derive(Debug, Clone, PartialEq)]
struct PeekState {
    module: String,
    until: u64,
}

/// `NotchCopy`'s own state.
#[derive(Debug, Clone, Default)]
struct CopyState {
    hovered: bool,
    pinned: bool,
    /// The pointer is on the copy (`pointerInside`).
    inside: bool,
    tab: String,
    /// The screen the pointer opened the moving copy on.
    open_screen: String,
    peek: Option<PeekState>,
    missed: Vec<Missed>,
    was_open: bool,
    /// `NotchCopy.lingering`: closed less than `window_linger` ms ago.
    lingering: bool,
    /// What the UI measured, once it has (`measure`).
    measure: Option<Measure>,
    /// `ReservedSpace.deepestCompact`: the deepest compact view seen.
    deepest: f64,
    /// The edge's orientation at the last measure (`onVerticalChanged`).
    vertical: bool,
    /// `ReservedSpace.space` as held now; `None` before the first target.
    space: Option<f64>,
    /// The target `space` last followed, so a timer is only (re)started when
    /// the target moves (`onTargetChanged`).
    target: Option<f64>,
}

/// The depths of one copy the UI reported.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Measure {
    rest: f64,
    baseline: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Timer {
    Open(Key),
    Close(Key),
    Peek(Key),
    /// `lingerTimer`.
    Linger(Key),
    /// `ReservedSpace.shrinkTimer`.
    Shrink(Key),
    NavSettle,
}

#[derive(Debug, Clone)]
struct MoveFrom {
    notch: usize,
    position: Option<Value>,
    workspaces: Option<Value>,
}

#[derive(Debug, Clone)]
struct Nav {
    mode: NavMode,
    selected: i64,
    /// Each copy's tab as navigation started, given back on Escape.
    tabs: Vec<(Key, String)>,
    move_from: Option<MoveFrom>,
}

/// What changed since the last call, to send to every UI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    pub config: Option<ConfigUpdate>,
    pub config_error: Option<ConfigError>,
    pub state: Option<ps::State>,
    pub schema: Option<Schema>,
}

/// A notch as the rules see it right now, derived from config and modules.
struct NotchView {
    key: String,
    id: String,
    /// Its place in the config; `None` for a standalone module's.
    index: Option<usize>,
    guest: bool,
    /// The visitor, a guest that is no standalone module's.
    visitor: bool,
    spec: Option<NotchSpec>,
    pos: position::Position,
    every: bool,
    homed: bool,
    tabs: bool,
    module_ids: Vec<String>,
    compact: Vec<ModuleInfo>,
    expanded: Vec<ModuleInfo>,
    copy_screens: Vec<String>,
}

impl NotchView {
    fn vertical(&self) -> bool {
        position::is_vertical(&self.pos.edge)
    }
}

/// One copy's derived facts.
struct CopyView {
    mode: state::Mode,
    screen: String,
    covered: bool,
    keyboard: bool,
    shown: Vec<String>,
    tab: String,
}

impl CopyView {
    fn open(&self) -> bool {
        matches!(self.mode, state::Mode::Peek | state::Mode::Expanded)
    }
}

#[derive(Debug, Default)]
pub struct Store {
    config: Value,
    config_loaded: bool,
    config_rev: u64,
    config_dirty: bool,
    config_error: Option<String>,
    config_error_dirty: bool,
    write: Option<Value>,

    screens: Option<Screens>,
    modules: BTreeMap<String, ModuleFacts>,
    copies: BTreeMap<Key, CopyState>,
    nav: Option<Nav>,
    nav_settling: bool,
    pub settings_open: bool,
    pub settings_page: String,
    /// The notch the settings window edits, by index (`settingsNotch`).
    settings_notch: usize,
    /// The modules' `{ moduleId, title, settings, standalone }`, as their
    /// JSON files give them (`moduleCatalog`), for the schema.
    module_defs: BTreeMap<String, Value>,
    schema_rev: u64,
    last_schema: Option<Schema>,

    window_linger: u64,
    timers: Timers<Timer>,
    state_rev: u64,
    last_state: Option<ps::State>,
}

fn to_mode(m: state::Mode) -> Mode {
    match m {
        state::Mode::Peek => Mode::Peek,
        state::Mode::Expanded => Mode::Expanded,
        state::Mode::Compact => Mode::Compact,
        state::Mode::Rest => Mode::Rest,
    }
}

fn to_edge(e: &str) -> Edge {
    match e {
        "bottom" => Edge::Bottom,
        "left" => Edge::Left,
        "right" => Edge::Right,
        _ => Edge::Top,
    }
}

fn nav_edge(e: &str) -> navigate::Edge {
    match to_edge(e) {
        Edge::Bottom => navigate::Edge::Bottom,
        Edge::Left => navigate::Edge::Left,
        Edge::Right => navigate::Edge::Right,
        Edge::Top => navigate::Edge::Top,
    }
}

fn edge_name(e: navigate::Edge) -> &'static str {
    match e {
        navigate::Edge::Bottom => "bottom",
        navigate::Edge::Left => "left",
        navigate::Edge::Right => "right",
        navigate::Edge::Top => "top",
    }
}

/// A normalized wire key (`"Escape"`, `"Tab"`, `"h"`, `"1"`, with modifiers) as
/// `Navigate.keyName` reads it (Qt key codes are the rules' own business).
pub fn wire_key(key: &str, mods: &[Modifier]) -> KeyName {
    let code: u32 = match key {
        "Escape" => 0x0100_0000,
        "Tab" => 0x0100_0001,
        "Backtab" | "ISO_Left_Tab" => 0x0100_0002,
        "Return" => 0x0100_0004,
        "Enter" => 0x0100_0005,
        "Left" => 0x0100_0012,
        "Up" => 0x0100_0013,
        "Right" => 0x0100_0014,
        "Down" => 0x0100_0015,
        k if k.chars().count() == 1 && k.is_ascii() => {
            u32::from(k.as_bytes()[0].to_ascii_uppercase())
        }
        _ => return KeyName::Other,
    };
    let mut flags = 0;
    for m in mods {
        flags |= match m {
            Modifier::Shift => SHIFT,
            Modifier::Ctrl => 0x0400_0000,
            Modifier::Alt => 0x0800_0000,
            Modifier::Meta => 0x1000_0000,
        };
    }
    navigate::key_name(code, flags)
}

impl Store {
    pub fn new() -> Store {
        Store {
            config: json!({}),
            settings_page: "general".into(),
            window_linger: WINDOW_LINGER_MS,
            ..Store::default()
        }
    }

    // ------------------------------------------------------------ config

    /// A good config, from the file or an edit: applied at once.
    pub fn set_config(&mut self, cfg: Value) {
        self.config = cfg;
        self.config_loaded = true;
        self.config_rev += 1;
        self.config_dirty = true;
        self.config_error = None;
    }

    /// The file did not parse: the last good config stays.
    pub fn config_failed(&mut self, message: String) {
        if self.config_error.as_deref() != Some(&message) {
            self.config_error = Some(message);
            self.config_error_dirty = true;
        }
    }

    pub fn config(&self) -> &Value {
        &self.config
    }

    pub fn config_error(&self) -> Option<&str> {
        self.config_error.as_deref()
    }

    /// An edit (`set`, an `edit` op, navigation, a drop): applied, and queued
    /// to be written to the file ([`Store::take_write`]).
    pub fn replace_config(&mut self, cfg: Value) {
        self.set_config(cfg.clone());
        self.write = Some(cfg);
    }

    /// `setConfigValue`: `None` removes the key and the objects it empties.
    pub fn set_value(&mut self, key: &str, value: Option<&Value>) {
        let next = config::with_value(&self.config, key, value);
        self.replace_config(next);
    }

    /// The config an edit left to write, once.
    pub fn take_write(&mut self) -> Option<Value> {
        self.write.take()
    }

    fn resolved(&self) -> config::Resolved {
        config::resolve(&self.config)
    }

    /// The `config` message for the config now, `None` before one loaded.
    pub fn config_update(&self) -> Option<ConfigUpdate> {
        if !self.config_loaded {
            return None;
        }
        let r = self.resolved();
        Some(ConfigUpdate {
            rev: self.config_rev,
            cfg: self.config.clone(),
            resolved: json!({
                "hoverCloseDelay": Num(r.hover_close_delay),
                "hoverOpenDelay": Num(r.hover_open_delay),
            }),
        })
    }

    // ----------------------------------------------------- screens, modules

    pub fn set_screens(&mut self, screens: Screens) {
        self.screens = Some(screens);
    }

    pub fn set_module(&mut self, facts: ModuleFacts) {
        self.modules.insert(facts.info.id.clone(), facts);
    }

    pub fn remove_module(&mut self, id: &str) {
        self.modules.remove(id);
        self.dismiss_peek(id);
    }

    pub fn module(&self, id: &str) -> Option<&ModuleFacts> {
        self.modules.get(id)
    }

    fn focused(&self) -> String {
        self.screens
            .as_ref()
            .map(|s| s.focused.clone())
            .unwrap_or_default()
    }

    fn screen_names(&self) -> Vec<String> {
        self.screens
            .as_ref()
            .map(|s| s.screens.iter().map(|x| x.name.clone()).collect())
            .unwrap_or_default()
    }

    fn workspace_of(&self, screen: &str) -> Option<&notchd_proto::ui::Workspace> {
        self.screens.as_ref()?.workspaces.get(screen)
    }

    /// `showsOn(screen)`: whether the notch's workspaces take its screen.
    fn shows_on(&self, workspaces: &Option<Vec<Value>>, screen: &str) -> bool {
        let list: Option<Vec<WorkspaceKey>> = workspaces.as_ref().map(|l| {
            l.iter()
                .map(|v| match v {
                    Value::Number(n) => WorkspaceKey::Number(n.as_f64().unwrap_or(f64::NAN)),
                    other => WorkspaceKey::Text(crate::js::string(other)),
                })
                .collect()
        });
        let ws = self.workspace_of(screen).map(|w| state::Workspace {
            id: WorkspaceKey::Text(w.id.clone()),
            name: w.name.clone(),
        });
        state::workspace_allowed(list.as_deref(), ws.as_ref())
    }

    fn screen_states(&self, workspaces: &Option<Vec<Value>>) -> Vec<ScreenState> {
        self.screen_names()
            .into_iter()
            .map(|name| ScreenState {
                allowed: self.shows_on(workspaces, &name),
                name,
            })
            .collect()
    }

    // ------------------------------------------------------------- views

    fn standalone_enabled(&self, id: &str) -> bool {
        config::value_at(&self.config, &format!("{id}.enabled")) != Some(&Value::Bool(false))
    }

    fn standalone_position(&self, id: &str) -> position::Position {
        let mut p = match config::value_at(&self.config, &format!("{id}.position")) {
            Some(Value::Object(o)) => o.clone(),
            _ => config::Object::new(),
        };
        p.insert("screen".into(), json!(position::ALL_SCREENS));
        position::resolve(&Value::Object(p))
    }

    fn info(&self, id: &str) -> Option<ModuleInfo> {
        self.modules.get(id).map(|m| m.info.clone())
    }

    fn notch_views(&self) -> Vec<NotchView> {
        let mut out = Vec::new();
        let specs = config::notches(&self.config, &DEFAULT_MODULES);
        let screens = self.screen_names();
        for spec in specs {
            let pos = position::resolve(spec.position.as_ref().unwrap_or(&Value::Null));
            let every = pos.screen == position::ALL_SCREENS;
            let homed = !every && !pos.screen.is_empty();
            let module_ids: Vec<String> = spec
                .module_ids
                .iter()
                .filter(|id| !self.modules.get(*id).is_some_and(|m| m.standalone))
                .cloned()
                .collect();
            let loaded: Vec<Option<ModuleInfo>> =
                module_ids.iter().map(|id| self.info(id)).collect();
            let active = state::active_modules(&loaded);
            let compact = state::compact_modules(&active, spec.tabs, spec.idle_icons)
                .into_iter()
                .cloned()
                .collect();
            let expanded = active
                .iter()
                .filter(|m| self.modules.get(&m.id).is_some_and(|f| f.expanded))
                .cloned()
                .collect();
            out.push(NotchView {
                key: spec.id.clone(),
                id: spec.id.clone(),
                index: Some(spec.index),
                guest: false,
                visitor: false,
                tabs: spec.tabs,
                module_ids,
                compact,
                expanded,
                copy_screens: if every {
                    screens.clone()
                } else {
                    vec![String::new()]
                },
                every,
                homed,
                pos,
                spec: Some(spec),
            });
        }
        for (id, facts) in &self.modules {
            if !facts.standalone || !self.standalone_enabled(id) {
                continue;
            }
            out.push(NotchView {
                key: format!("@{id}"),
                id: id.clone(),
                index: None,
                guest: true,
                visitor: false,
                spec: None,
                pos: self.standalone_position(id),
                every: true,
                homed: false,
                tabs: false,
                module_ids: vec![id.clone()],
                compact: Vec::new(),
                expanded: Vec::new(),
                copy_screens: screens.clone(),
            });
        }
        // The visitor (`Notch.qml`, `visitor: true`): every module that is
        // no standalone one, a copy per screen, its position the default.
        out.push(NotchView {
            key: VISITOR_KEY.into(),
            id: String::new(),
            index: None,
            guest: true,
            visitor: true,
            spec: None,
            pos: position::resolve(&json!({"screen": position::ALL_SCREENS})),
            every: true,
            homed: false,
            tabs: false,
            module_ids: self
                .modules
                .iter()
                .filter(|(_, f)| !f.standalone)
                .map(|(id, _)| id.clone())
                .collect(),
            compact: Vec::new(),
            expanded: Vec::new(),
            copy_screens: screens,
        });
        out
    }

    fn focused_copy_screen<'a>(&self, nv: &'a NotchView, focused: &str) -> Option<&'a str> {
        state::peek_copy(&nv.copy_screens, focused)
    }

    fn mode_of(&self, nv: &NotchView, cs: &str, copy: &CopyState, focused: &str) -> state::Mode {
        let navigating = self.nav.is_some() && !nv.guest;
        let selected = self.nav.as_ref().map(|n| n.selected);
        let nav_selected = navigating && nv.index.is_some_and(|i| Some(i as i64) == selected);
        let preview = nav_selected && self.focused_copy_screen(nv, focused) == Some(cs);
        state::mode(&ModeContext {
            peek_module: copy.peek.is_some(),
            compact_module: !nv.compact.is_empty(),
            pinned: copy.pinned,
            hovered: copy.hovered,
            dragging: false,
            guest: nv.guest,
            navigating,
            preview,
        })
    }

    fn copy_view(&self, nv: &NotchView, cs: &str) -> CopyView {
        let focused = self.focused();
        let default = CopyState::default();
        let copy = self
            .copies
            .get(&(nv.key.clone(), cs.to_string()))
            .unwrap_or(&default);
        let mode = self.mode_of(nv, cs, copy, &focused);
        let open = matches!(mode, state::Mode::Peek | state::Mode::Expanded);
        let workspaces = nv.spec.as_ref().and_then(|s| s.workspaces.clone());
        let states = self.screen_states(&workspaces);
        let home = if nv.pos.screen == position::ALL_SCREENS {
            cs.to_string()
        } else {
            nv.pos.screen.clone()
        };
        let allowed = |name: &str| {
            states
                .iter()
                .find(|s| s.name == name)
                .is_some_and(|s| s.allowed)
        };
        let wanted = if !cs.is_empty() {
            if nv.guest {
                if open { cs.to_string() } else { String::new() }
            } else if open || allowed(cs) {
                cs.to_string()
            } else {
                String::new()
            }
        } else {
            state::notch_screen(&ScreenContext {
                screens: states.clone(),
                focused: focused.clone(),
                open,
                override_screen: copy.open_screen.clone(),
                home,
            })
        };
        let screen = if !wanted.is_empty() {
            wanted.clone()
        } else if !cs.is_empty() {
            cs.to_string()
        } else {
            focused.clone()
        };
        let shown = state::shown_modules(&nv.expanded, nv.tabs, &copy.tab);
        let keyboard = state::wants_keyboard(&KeyboardContext {
            pinned: copy.pinned,
            mode: Some(mode),
            navigating: self.nav.is_some(),
            settling: self.nav_settling,
            elsewhere: !focused.is_empty() && screen != focused,
            ..KeyboardContext::default()
        });
        CopyView {
            mode,
            covered: wanted.is_empty(),
            screen,
            keyboard,
            shown: shown.modules.iter().map(|m| m.id.clone()).collect(),
            tab: copy.tab.clone(),
        }
    }

    fn key_of(nv: &NotchView, screen: &str) -> Key {
        (
            nv.key.clone(),
            if nv.every {
                screen.to_string()
            } else {
                String::new()
            },
        )
    }

    fn find_view<'a>(views: &'a [NotchView], notch: &str) -> Option<&'a NotchView> {
        views
            .iter()
            .find(|v| v.id == notch && !v.guest)
            .or_else(|| views.iter().find(|v| v.id == notch))
    }

    // ------------------------------------------------------------- state

    fn compute_state(&self) -> ps::State {
        let views = self.notch_views();
        let mut notches = Vec::new();
        let mut standalone = Vec::new();
        let mut visitor = Vec::new();
        let slot = |nv: &NotchView| {
            if nv.vertical() {
                Slot::VerticalPeek
            } else {
                Slot::Peek
            }
        };
        for nv in &views {
            let mut copies = Vec::new();
            let mut guest_copies = Vec::new();
            for cs in &nv.copy_screens {
                let view = self.copy_view(nv, cs);
                let st = self.copies.get(&(nv.key.clone(), cs.clone()));
                let peek = st.and_then(|c| c.peek.as_ref()).map(|p| ps::Peek {
                    module: p.module.clone(),
                    view: slot(nv),
                    until: p.until,
                });
                let lingering = st.is_some_and(|c| c.lingering);
                // `State.needsWindow`: a notch of the config always has its
                // window; a guest's only while open and for `windowLinger`
                // ms after (`NotchCopy.lingering`).
                let needs_window = state::needs_window(&state::WindowContext {
                    guest: nv.guest,
                    open: view.open(),
                    lingering,
                });
                if nv.guest {
                    guest_copies.push(ps::StandaloneCopy {
                        screen: view.screen,
                        mode: to_mode(view.mode),
                        peek,
                        needs_window,
                    });
                    continue;
                }
                copies.push(ps::Copy {
                    screen: view.screen,
                    mode: to_mode(view.mode),
                    pinned: st.is_some_and(|c| c.pinned),
                    tab: view.tab,
                    peek,
                    shown: view.shown,
                    covered: view.covered,
                    needs_window,
                    keyboard: view.keyboard,
                });
            }
            if nv.visitor {
                visitor = guest_copies;
                continue;
            }
            if nv.guest {
                standalone.push(ps::Standalone {
                    id: nv.id.clone(),
                    copies: guest_copies,
                });
                continue;
            }
            let spec = nv.spec.as_ref();
            notches.push(ps::Notch {
                id: nv.id.clone(),
                edge: to_edge(&nv.pos.edge),
                offset: Num(nv.pos.offset),
                screen: nv.pos.screen.clone(),
                draggable: spec.is_none_or(|s| s.draggable),
                tabs: nv.tabs,
                modules: nv.module_ids.clone(),
                copies,
            });
        }
        let navigation = self.nav.as_ref().map(|n| {
            let tab = usize::try_from(n.selected)
                .ok()
                .and_then(|i| views.iter().find(|v| v.index == Some(i)))
                .and_then(|nv| {
                    let focused = self.focused();
                    let cs = self.focused_copy_screen(nv, &focused)?;
                    Some(self.copy_view(nv, cs).tab)
                })
                .unwrap_or_default();
            ps::Navigation {
                mode: match n.mode {
                    NavMode::Select => ps::NavMode::Select,
                    NavMode::Move => ps::NavMode::Move,
                    NavMode::Remove => ps::NavMode::Remove,
                },
                selected: n.selected.max(0) as u32,
                tab,
            }
        });
        ps::State {
            rev: self.state_rev,
            navigation,
            settings_open: self.settings_open,
            notches,
            reserve: self.reserve_state(&views),
            standalone,
            visitor,
        }
    }

    /// The `state` as it is now, with the `rev` last published (what a UI
    /// that just connected is sent; the daemon publishes after every change).
    pub fn state(&self) -> ps::State {
        self.compute_state()
    }

    /// What changed since the last call. The state's `rev` only moves when
    /// the state differs from the one last published.
    pub fn take_changes(&mut self, now: u64) -> Changes {
        self.settle(now);
        let mut out = Changes::default();
        if self.config_dirty {
            self.config_dirty = false;
            out.config = self.config_update();
        }
        if self.config_error_dirty {
            self.config_error_dirty = false;
            out.config_error = self
                .config_error
                .clone()
                .map(|message| ConfigError { message });
        }
        let schema = self.compute_schema();
        if self.last_schema.as_ref().is_none_or(|last| {
            let mut a = last.clone();
            a.rev = 0;
            a != schema
        }) {
            self.schema_rev += 1;
            let sent = Schema {
                rev: self.schema_rev,
                ..schema
            };
            self.last_schema = Some(sent.clone());
            out.schema = Some(sent);
        }
        let mut now_state = self.compute_state();
        let same = self.last_state.as_ref().is_some_and(|last| {
            let mut a = last.clone();
            a.rev = 0;
            let mut b = now_state.clone();
            b.rev = 0;
            a == b
        });
        if !same {
            self.state_rev += 1;
            now_state.rev = self.state_rev;
            self.last_state = Some(now_state.clone());
            out.state = Some(now_state);
        }
        out
    }

    // ------------------------------------------------------------ timers

    pub fn next_due(&self) -> Option<u64> {
        self.timers.next_due()
    }

    /// Fires every timer due at `now`.
    pub fn advance(&mut self, now: u64) {
        while let Some(t) = self.timers.pop_due(now) {
            match t {
                Timer::Open(key) => {
                    // `if (!copy.dragging)`: a drag only ever ends in a drop here.
                    self.set_hovered(&key, true);
                }
                Timer::Close(key) => self.set_hovered(&key, false),
                Timer::Peek(key) => {
                    if let Some(c) = self.copies.get_mut(&key) {
                        c.peek = None;
                    }
                }
                Timer::Linger(key) => {
                    if let Some(c) = self.copies.get_mut(&key) {
                        c.lingering = false;
                    }
                }
                Timer::Shrink(key) => self.reserve_settle(&key),
                Timer::NavSettle => self.nav_settling = false,
            }
            self.settle(now);
        }
        self.settle(now);
    }

    /// Reacts to copies that closed (`onIsOpenChanged`, `onModeChanged`): the
    /// peeks they could not show come now, and the screen the pointer opened
    /// them on is forgotten. A peek may open a copy, so it goes again.
    fn settle(&mut self, now: u64) {
        self.reserve_follow(now);
        for _ in 0..4 {
            let views = self.notch_views();
            let focused = self.focused();
            let mut closed = Vec::new();
            let mut live = Vec::new();
            for nv in &views {
                for cs in &nv.copy_screens {
                    let key = (nv.key.clone(), cs.clone());
                    let copy = self.copies.get(&key).cloned().unwrap_or_default();
                    let open = matches!(
                        self.mode_of(nv, cs, &copy, &focused),
                        state::Mode::Peek | state::Mode::Expanded
                    );
                    if copy.was_open && !open {
                        closed.push(key.clone());
                    }
                    live.push((key, open));
                }
            }
            // A copy that no longer exists takes its state and timers along.
            let keys: Vec<Key> = live.iter().map(|(k, _)| k.clone()).collect();
            let gone: Vec<Key> = self
                .copies
                .keys()
                .filter(|k| !keys.contains(k))
                .cloned()
                .collect();
            for k in gone {
                self.copies.remove(&k);
                self.timers.cancel(&Timer::Open(k.clone()));
                self.timers.cancel(&Timer::Close(k.clone()));
                self.timers.cancel(&Timer::Peek(k));
            }
            for (key, open) in live {
                let c = self.copies.entry(key).or_default();
                c.was_open = open;
            }
            if closed.is_empty() {
                return;
            }
            for key in closed {
                if let Some(c) = self.copies.get_mut(&key) {
                    c.open_screen.clear();
                    // `onIsOpenChanged`: the window stays `windowLinger` ms.
                    c.lingering = true;
                }
                self.timers
                    .after(Timer::Linger(key.clone()), now, self.window_linger);
                self.peek_missed(&key, now);
            }
        }
    }

    // ------------------------------------------------------------- peeks

    fn peek_copy_state(&self, key: &Key) -> (bool, bool, Option<ModuleInfo>) {
        let c = self.copies.get(key);
        (
            c.is_some_and(|c| c.pinned),
            c.is_some_and(|c| c.hovered),
            c.and_then(|c| c.peek.as_ref())
                .and_then(|p| self.info(&p.module)),
        )
    }

    /// `NotchCopy.requestPeek`.
    fn request_peek_on(&mut self, key: &Key, module: &str, ms: Option<f64>, now: u64) {
        let Some(info) = self.info(module) else {
            return;
        };
        let (pinned, hovered, current) = self.peek_copy_state(key);
        let may = peek::may_peek(
            &info,
            &CopyPeek {
                pinned,
                hovered,
                current: current.as_ref(),
            },
        );
        let c = self.copies.entry(key.clone()).or_default();
        if !may {
            peek::keep_missed(&mut c.missed, module, info.attention, ms);
            return;
        }
        let until = now + peek::duration_ms(ms);
        c.peek = Some(PeekState {
            module: module.to_string(),
            until,
        });
        self.timers.set(Timer::Peek(key.clone()), until);
    }

    /// `NotchCopy.peekMissed`.
    fn peek_missed(&mut self, key: &Key, now: u64) {
        let Some(mut list) = self
            .copies
            .get_mut(key)
            .map(|c| std::mem::take(&mut c.missed))
        else {
            return;
        };
        let lookup = |id: &str| self.info(id);
        let taken = peek::take_missed(&mut list, &lookup);
        if let Some(c) = self.copies.get_mut(key) {
            c.missed = list;
        }
        if let Some(m) = taken {
            self.request_peek_on(key, &m.module, m.ms, now);
        }
    }

    /// `NotchService.requestPeek`: a module asks to peek for `ms`.
    pub fn request_peek(&mut self, module: &str, ms: Option<f64>, now: u64) {
        let Some(facts) = self.modules.get(module).cloned() else {
            return;
        };
        let views = self.notch_views();
        let focused = self.focused();
        if facts.standalone {
            if let Some(nv) = views
                .iter()
                .find(|v| v.guest && !v.visitor && v.id == module)
                && let Some(cs) = self.focused_copy_screen(nv, &focused).map(str::to_string)
            {
                self.request_peek_on(&(nv.key.clone(), cs), module, ms, now);
            }
            self.settle(now);
            return;
        }
        let owners: Vec<&NotchView> = views
            .iter()
            .filter(|v| !v.guest && v.module_ids.iter().any(|m| m == module))
            .collect();
        let places: Vec<state::PeekNotch> = owners
            .iter()
            .map(|nv| {
                let at = if nv.homed {
                    self.copy_view(nv, "").screen
                } else {
                    String::new()
                };
                peek::place(nv.homed, nv.every, &at)
            })
            .collect();
        let route = peek::route(&places, &focused, facts.peek_on_focused_screen);
        for i in route.notches {
            let nv = owners[i];
            if let Some(cs) = self.focused_copy_screen(nv, &focused).map(str::to_string) {
                self.request_peek_on(&(nv.key.clone(), cs), module, ms, now);
            }
        }
        // The visitor: a copy of the default notch on every screen that only
        // shows peeks; its own `Notch.requestPeek` takes the module if it
        // lists it (every module that is no standalone one).
        if route.visitor
            && let Some(nv) = views.iter().find(|v| v.visitor)
            && nv.module_ids.iter().any(|m| m == module)
            && let Some(cs) = self.focused_copy_screen(nv, &focused).map(str::to_string)
        {
            self.request_peek_on(&(nv.key.clone(), cs), module, ms, now);
        }
        self.settle(now);
    }

    /// `NotchService.dismissPeek`: every copy peeking `module` (all peeks for
    /// an empty id) drops it.
    pub fn dismiss_peek(&mut self, module: &str) {
        let keys: Vec<Key> = self
            .copies
            .iter()
            .filter(|(_, c)| {
                c.peek
                    .as_ref()
                    .is_some_and(|p| module.is_empty() || p.module == module)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys {
            self.dismiss_peek_on(&k);
        }
    }

    fn dismiss_peek_on(&mut self, key: &Key) {
        self.timers.cancel(&Timer::Peek(key.clone()));
        if let Some(c) = self.copies.get_mut(key) {
            c.peek = None;
        }
    }

    // ---------------------------------------------------- copy transitions

    /// `onHoveredChanged`.
    fn set_hovered(&mut self, key: &Key, on: bool) {
        let c = self.copies.entry(key.clone()).or_default();
        if c.hovered == on {
            return;
        }
        c.hovered = on;
        let current = self.copies[key]
            .peek
            .as_ref()
            .and_then(|p| self.info(&p.module));
        if peek::hover_dismisses(on, current.as_ref()) {
            self.dismiss_peek_on(key);
        }
    }

    /// `onPinnedChanged`.
    fn set_pinned(&mut self, key: &Key, on: bool, now: u64) {
        let c = self.copies.entry(key.clone()).or_default();
        if c.pinned == on {
            return;
        }
        c.pinned = on;
        if on {
            self.dismiss_peek_on(key);
            if self.nav.is_some() {
                self.end_navigation(false, now);
            }
        }
    }

    fn close_copy(&mut self, key: &Key, now: u64) {
        self.set_pinned(key, false, now);
        self.set_hovered(key, false);
    }

    // ------------------------------------------------------------ input

    /// `pointer`: raw facts; the delays and the mode are ours.
    pub fn pointer(&mut self, notch: &str, screen: &str, ev: PointerEv, now: u64) {
        let views = self.notch_views();
        let Some(nv) = Self::find_view(&views, notch) else {
            return;
        };
        let key = Self::key_of(nv, screen);
        match ev {
            PointerEv::Enter => {
                let r = self.resolved();
                let c = self.copies.entry(key.clone()).or_default();
                let was_inside = c.inside;
                c.inside = true;
                if !screen.is_empty() {
                    c.open_screen = screen.to_string();
                }
                if !was_inside {
                    self.timers.cancel(&Timer::Close(key.clone()));
                    self.timers
                        .after(Timer::Open(key), now, r.hover_open_delay as u64);
                }
            }
            PointerEv::Leave => {
                let r = self.resolved();
                let open = {
                    let focused = self.focused();
                    let copy = self.copies.get(&key).cloned().unwrap_or_default();
                    matches!(
                        self.mode_of(nv, &key.1, &copy, &focused),
                        state::Mode::Peek | state::Mode::Expanded
                    )
                };
                let c = self.copies.entry(key.clone()).or_default();
                let was_inside = c.inside;
                c.inside = false;
                if was_inside {
                    self.timers.cancel(&Timer::Open(key.clone()));
                    self.timers
                        .after(Timer::Close(key.clone()), now, r.hover_close_delay as u64);
                    if !open {
                        self.copies.entry(key).or_default().open_screen.clear();
                    }
                }
            }
            PointerEv::Press => {
                let c = self.copies.entry(key.clone()).or_default();
                if !screen.is_empty() {
                    c.open_screen = screen.to_string();
                }
                let next = !c.pinned;
                self.set_pinned(&key, next, now);
            }
        }
        self.advance(now);
    }

    /// `tab`: the user picked a module's tab.
    pub fn tab(&mut self, notch: &str, screen: &str, module: &str, now: u64) {
        let views = self.notch_views();
        if let Some(nv) = Self::find_view(&views, notch) {
            let key = Self::key_of(nv, screen);
            self.copies.entry(key).or_default().tab = module.to_string();
        }
        self.settle(now);
    }

    /// `onSettingsPageChanged`: showing a notch's page picks the notch the
    /// window edits.
    fn show_settings_page(&mut self, page: String) {
        if let Ok(i) = usize::try_from(settings::notch_index_of_page(&page)) {
            self.settings_notch = i;
        }
        self.settings_page = page;
    }

    pub fn open_settings(&mut self, page: Option<String>) {
        self.settings_open = true;
        if let Some(p) = page {
            self.show_settings_page(p);
        }
    }

    /// `settings`: the window opened or closed (`page`: keep the one shown
    /// when absent).
    pub fn settings(&mut self, open: bool, page: Option<String>) {
        self.settings_open = open;
        if let Some(p) = page {
            self.show_settings_page(p);
        }
    }

    /// `drag` `drop`: the copy goes back to closed (`setDragging(false)`) and
    /// the notch is where the UI put it.
    pub fn drop_notch(&mut self, notch: &str, screen: &str, to: navigate::Position, now: u64) {
        let views = self.notch_views();
        let Some(nv) = Self::find_view(&views, notch) else {
            return;
        };
        let key = Self::key_of(nv, screen);
        self.timers.cancel(&Timer::Open(key.clone()));
        self.set_pinned(&key, false, now);
        self.set_hovered(&key, false);
        if let Some(i) = nv.index {
            self.move_to(i, &to);
        }
        self.advance(now);
    }

    // ------------------------------------------------------------ notches

    fn spec_at(&self, index: usize) -> Option<NotchSpec> {
        config::notches(&self.config, &DEFAULT_MODULES)
            .into_iter()
            .nth(index)
    }

    /// `Notch.moveTo`: to `place`; taken to a screen showing a workspace the
    /// notch leaves out, that workspace is let in first.
    fn move_to(&mut self, index: usize, place: &navigate::Position) {
        let Some(spec) = self.spec_at(index) else {
            return;
        };
        let home = !place.screen.is_empty()
            && place.screen != position::ALL_SCREENS
            && self.screen_names().contains(&place.screen);
        if home && spec.workspaces.is_some() && !self.shows_on(&spec.workspaces, &place.screen) {
            let ws = self
                .workspace_of(&place.screen)
                .map(|w| json!({"id": w.id, "name": w.name}));
            let next =
                config::with_workspace(spec.workspaces.as_deref(), ws.as_ref()).map(Value::Array);
            self.set_value(&format!("{}workspaces", spec.prefix), next.as_ref());
        }
        let inherited = (!spec.prefix.is_empty())
            .then(|| config::value_at(&self.config, "position").cloned())
            .flatten();
        let target =
            json!({"edge": edge_name(place.edge), "offset": place.offset, "screen": place.screen});
        let stored = position::config_value(&target, inherited.as_ref());
        self.set_value(&format!("{}position", spec.prefix), stored.as_ref());
    }

    fn open_notch_at(&mut self, index: usize, tab: Option<&str>, now: u64) -> bool {
        let views = self.notch_views();
        let Some(nv) = views.iter().find(|v| v.index == Some(index)) else {
            return false;
        };
        let focused = self.focused();
        let Some(cs) = self.focused_copy_screen(nv, &focused).map(str::to_string) else {
            return false;
        };
        let key = (nv.key.clone(), if nv.every { cs } else { String::new() });
        if let Some(t) = tab {
            self.copies.entry(key.clone()).or_default().tab = t.to_string();
        }
        self.set_pinned(&key, true, now);
        true
    }

    fn close_notch_at(&mut self, index: usize, now: u64) {
        let views = self.notch_views();
        let Some(nv) = views.iter().find(|v| v.index == Some(index)) else {
            return;
        };
        for cs in &nv.copy_screens {
            self.close_copy(&(nv.key.clone(), cs.clone()), now);
        }
    }

    fn notch_pinned(&self, index: usize) -> bool {
        let views = self.notch_views();
        views
            .iter()
            .find(|v| v.index == Some(index))
            .is_some_and(|nv| {
                nv.copy_screens.iter().any(|cs| {
                    self.copies
                        .get(&(nv.key.clone(), cs.clone()))
                        .is_some_and(|c| c.pinned)
                })
            })
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        config::notches(&self.config, &DEFAULT_MODULES)
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.index)
    }

    fn notch_count(&self) -> usize {
        config::notches(&self.config, &DEFAULT_MODULES).len()
    }

    /// IPC `open`: pin the first notch open on the focused screen.
    pub fn open(&mut self, now: u64) {
        if self.notch_count() > 0 {
            self.open_notch_at(0, None, now);
        }
        self.advance(now);
    }

    /// IPC `close`: close every notch.
    pub fn close(&mut self, now: u64) {
        for i in 0..self.notch_count() {
            self.close_notch_at(i, now);
        }
        self.advance(now);
    }

    /// IPC `toggle`: the first notch.
    pub fn toggle(&mut self, now: u64) {
        if self.notch_count() > 0 {
            self.toggle_at(0, now);
        }
        self.advance(now);
    }

    fn toggle_at(&mut self, index: usize, now: u64) {
        if self.notch_pinned(index) {
            self.close_notch_at(index, now);
        } else {
            self.open_notch_at(index, None, now);
        }
    }

    /// IPC `openNotch`, `closeNotch`, `toggleNotch` by id; false for no such notch.
    pub fn notch_command(&mut self, id: &str, command: &str, now: u64) -> bool {
        let Some(i) = self.index_of(id) else {
            return false;
        };
        match command {
            "open" => {
                self.open_notch_at(i, None, now);
            }
            "close" => self.close_notch_at(i, now),
            _ => self.toggle_at(i, now),
        }
        self.advance(now);
        true
    }

    /// `NotchService.openModule`: the first notch that shows `module` opens on
    /// its tab. False when none does.
    pub fn open_module(&mut self, module: &str, now: u64) -> bool {
        let views = self.notch_views();
        for nv in views.iter().filter(|v| !v.guest) {
            if nv.module_ids.iter().any(|m| m == module) {
                let ok = nv
                    .index
                    .is_some_and(|i| self.open_notch_at(i, Some(module), now));
                self.advance(now);
                return ok;
            }
        }
        false
    }

    /// `NotchService.closeModule`.
    pub fn close_module(&mut self, module: &str, now: u64) {
        let views = self.notch_views();
        for nv in views.iter().filter(|v| !v.guest) {
            if nv.module_ids.iter().any(|m| m == module)
                && let Some(i) = nv.index
            {
                self.close_notch_at(i, now);
            }
        }
        self.advance(now);
    }

    /// IPC `recenter`: every `position` removed.
    pub fn recenter(&mut self) {
        let mut cfg = config::with_value(&self.config, "position", None);
        let n = match cfg.get("notches") {
            Some(Value::Array(l)) if config::has_notch_list(&cfg) => l.len(),
            _ => 0,
        };
        for i in 0..n {
            cfg = config::with_value(&cfg, &format!("notches.{i}.position"), None);
        }
        self.replace_config(cfg);
    }

    /// The list edits of `Settings.js` as intents (`edit`).
    pub fn edit(&mut self, notch: &str, op: &EditOp) -> Result<(), String> {
        let spec = config::notches(&self.config, &DEFAULT_MODULES)
            .into_iter()
            .find(|s| s.id == notch)
            .ok_or_else(|| format!("no notch {notch}"))?;
        match op {
            EditOp::ToggleModule(m) => {
                let next = settings::toggle_id(&spec.module_ids, m);
                self.set_value(&format!("{}modules", spec.prefix), Some(&json!(next)));
            }
            EditOp::MoveModule(m, delta) => {
                let next = settings::move_id(&spec.module_ids, m, *delta);
                self.set_value(&format!("{}modules", spec.prefix), Some(&json!(next)));
            }
            EditOp::ToggleWorkspace(w) => {
                let picked = spec.workspaces.map(Value::Array).unwrap_or(Value::Null);
                let next = settings::toggle_workspace(&picked, &json!(w)).map(Value::Array);
                self.set_value(&format!("{}workspaces", spec.prefix), next.as_ref());
            }
        }
        Ok(())
    }

    // -------------------------------------------------------- navigation

    pub fn navigating(&self) -> bool {
        self.nav.is_some()
    }

    /// IPC `navigate`: starts, or leaves (as Escape does).
    pub fn navigate(&mut self, now: u64) -> String {
        if self.nav.is_some() {
            self.end_navigation(true, now);
            self.advance(now);
            return "ok".into();
        }
        let r = self.start_navigation();
        self.advance(now);
        r
    }

    fn start_navigation(&mut self) -> String {
        let views = self.notch_views();
        let config_notches: Vec<&NotchView> = views.iter().filter(|v| !v.guest).collect();
        if config_notches.is_empty() {
            return "no notches".into();
        }
        let focused = self.focused();
        let infos: Vec<Option<navigate::NotchInfo>> = config_notches
            .iter()
            .map(|nv| {
                let mut info = navigate::NotchInfo::default();
                for cs in &nv.copy_screens {
                    let v = self.copy_view(nv, cs);
                    info.open |= v.open();
                    info.screens.push(v.screen);
                }
                Some(info)
            })
            .collect();
        let selected = navigate::start(&infos, &focused).map_or(-1, |i| i as i64);
        let tabs = self
            .copies
            .iter()
            .map(|(k, c)| (k.clone(), c.tab.clone()))
            .collect();
        self.nav = Some(Nav {
            mode: NavMode::Select,
            selected,
            tabs,
            move_from: None,
        });
        "ok".into()
    }

    /// `endNavigation(restore)`: Escape gives each copy its tab back and a
    /// notch being moved its place.
    fn end_navigation(&mut self, restore: bool, now: u64) {
        let Some(mut nav) = self.nav.take() else {
            return;
        };
        if restore {
            self.undo_move(nav.move_from.take());
        }
        self.nav_settling = true;
        self.timers.after(Timer::NavSettle, now, NAV_SETTLE_MS);
        if restore {
            for (key, tab) in nav.tabs {
                if let Some(c) = self.copies.get_mut(&key) {
                    c.tab = tab;
                }
            }
        }
    }

    fn undo_move(&mut self, from: Option<MoveFrom>) {
        let Some(from) = from else { return };
        let Some(spec) = self.spec_at(from.notch) else {
            return;
        };
        self.set_value(
            &format!("{}workspaces", spec.prefix),
            from.workspaces.as_ref(),
        );
        self.set_value(&format!("{}position", spec.prefix), from.position.as_ref());
    }

    /// `navKey`: a key while navigating; anything else is ignored.
    pub fn key(&mut self, key: &str, mods: &[Modifier], now: u64) {
        let name = wire_key(key, mods);
        self.nav_key(name, now);
        self.advance(now);
    }

    fn nav_key(&mut self, key: KeyName, now: u64) {
        let Some(nav) = self.nav.clone() else { return };
        let views = self.notch_views();
        let notches: Vec<&NotchView> = views.iter().filter(|v| !v.guest).collect();
        let focused = self.focused();
        let notch = usize::try_from(nav.selected)
            .ok()
            .and_then(|i| notches.get(i).copied());
        let copy_screen =
            notch.and_then(|nv| self.focused_copy_screen(nv, &focused).map(str::to_string));
        let copy_key = notch
            .zip(copy_screen.clone())
            .map(|(nv, cs)| Self::key_of(nv, &cs));
        let (tabs, shown, copy_name) = match (notch, copy_screen.as_deref()) {
            (Some(nv), Some(cs)) => {
                let c = self
                    .copies
                    .get(&Self::key_of(nv, cs))
                    .cloned()
                    .unwrap_or_default();
                let s = state::shown_modules(&nv.expanded, nv.tabs, &c.tab);
                (
                    s.tabs.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                    s.modules.first().map(|m| m.id.clone()).unwrap_or_default(),
                    self.copy_view(nv, cs).screen,
                )
            }
            _ => (Vec::new(), String::new(), String::new()),
        };
        let next = navigate::step(
            &StepState {
                mode: nav.mode,
                selected: nav.selected,
                count: notches.len() as i64,
                tabs,
                tab: shown.clone(),
            },
            key,
        );
        let notch_index = notch.and_then(|nv| nv.index);
        let notch_keys: Vec<String> = notch
            .map(|nv| nv.copy_screens.iter().map(|_| nv.key.clone()).collect())
            .unwrap_or_default();
        match next.action {
            Action::Leave => {
                self.end_navigation(true, now);
                return;
            }
            Action::Open => {
                self.end_navigation(false, now);
                for i in 0..notches.len() {
                    if Some(i) != notch_index {
                        self.close_notch_at(i, now);
                    }
                }
                if let Some(i) = notch_index {
                    self.open_notch_at(i, None, now);
                }
                return;
            }
            Action::Settings => {
                let index = nav.selected;
                self.end_navigation(true, now);
                if index >= 0 {
                    self.open_settings(Some(settings::notch_page_id(index as u64)));
                } else {
                    self.open_settings(None);
                }
                return;
            }
            Action::Remove if notch.is_some() => {
                // Its copies go with it: no tab to give back on Escape.
                if let Some(n) = self.nav.as_mut() {
                    n.tabs.retain(|(k, _)| !notch_keys.contains(&k.0));
                }
                let next_cfg = config::without_notch(&self.config, nav.selected);
                self.replace_config(next_cfg);
            }
            Action::MoveStart if notch.is_some() => {
                if let (Some(i), Some(spec)) =
                    (notch_index, notch_index.and_then(|i| self.spec_at(i)))
                {
                    let get = |k: &str| {
                        config::value_at(&self.config, &format!("{}{k}", spec.prefix)).cloned()
                    };
                    let from = MoveFrom {
                        notch: i,
                        position: get("position"),
                        workspaces: get("workspaces"),
                    };
                    if let Some(n) = self.nav.as_mut() {
                        n.move_from = Some(from);
                    }
                }
            }
            Action::Move if notch.is_some() => {
                if let (Some(nv), Some(i)) = (notch, notch_index) {
                    let current = navigate::Position {
                        edge: nav_edge(&nv.pos.edge),
                        offset: nv.pos.offset,
                        screen: nv.pos.screen.clone(),
                    };
                    let target = navigate::move_target(
                        &current,
                        next.movement.as_ref(),
                        &self.screen_names(),
                        &copy_name,
                    );
                    self.move_to(i, &target);
                }
            }
            Action::MoveDone => {
                if let Some(n) = self.nav.as_mut() {
                    n.move_from = None;
                }
            }
            Action::MoveCancel => {
                let from = self.nav.as_mut().and_then(|n| n.move_from.take());
                self.undo_move(from);
            }
            _ => {}
        }
        if let Some(n) = self.nav.as_mut() {
            n.mode = next.mode;
            n.selected = next.selected;
        }
        if !next.tab.is_empty()
            && next.tab != shown
            && let Some(k) = copy_key
        {
            self.copies.entry(k).or_default().tab = next.tab.clone();
        }
    }

    // ------------------------------------------------------------ reserve

    /// The depths the UI measured for a copy (`measure`). The deepest compact
    /// view seen restarts when the baseline or the edge's orientation
    /// changes (`ReservedSpace.onCompactBaselineChanged`,
    /// `onVerticalChanged`) and grows with the compact depth shown
    /// (`onCompactDepthChanged`).
    pub fn measure(&mut self, notch: &str, screen: &str, rest: f64, compact: f64, baseline: f64) {
        let views = self.notch_views();
        let Some(nv) = Self::find_view(&views, notch) else {
            return;
        };
        let key = Self::key_of(nv, screen);
        let vertical = nv.vertical();
        let c = self.copies.entry(key).or_default();
        let next = Measure { rest, baseline };
        if c.measure.is_some_and(|m| m.baseline != baseline) || c.vertical != vertical {
            c.deepest = 0.0;
        }
        c.vertical = vertical;
        if compact > c.deepest {
            c.deepest = compact;
        }
        c.measure = Some(next);
    }

    /// `NotchCopy.reservedScreens`: the screens a copy keeps windows clear
    /// on. A guest reserves nowhere; a copy on its own screen there while it
    /// may hang there; the one copy as `State.reserveScreens` says.
    fn reserved_screens(&self, nv: &NotchView, cs: &str) -> Vec<String> {
        if nv.guest {
            return Vec::new();
        }
        let workspaces = nv.spec.as_ref().and_then(|s| s.workspaces.clone());
        let states = self.screen_states(&workspaces);
        if !cs.is_empty() {
            return if states.iter().any(|s| s.name == cs && s.allowed) {
                vec![cs.to_string()]
            } else {
                Vec::new()
            };
        }
        let home = if nv.pos.screen == position::ALL_SCREENS {
            cs
        } else {
            &nv.pos.screen
        };
        state::reserve_screens(&states, home)
    }

    /// `ReservedSpace.target`: `State.reservedSpace` over what the UI
    /// measured; `None` while it has not.
    fn reserve_target(&self, nv: &NotchView, cs: &str) -> Option<f64> {
        let c = self.copies.get(&(nv.key.clone(), cs.to_string()))?;
        let m = c.measure?;
        Some(state::reserved_space(&state::ReserveContext {
            enabled: nv.spec.as_ref().is_some_and(|s| s.reserve_space),
            compact: !nv.compact.is_empty(),
            baseline: m.baseline,
            tallest: c.deepest,
            rest: m.rest,
        }))
    }

    /// `ReservedSpace.follow`, for every copy whose target moved: `reserveStep`
    /// says grow now, shrink after `RESERVE_SHRINK_MS`, or nothing (which
    /// also cancels a pending shrink). The first target is taken at once
    /// (`Component.onCompleted`).
    fn reserve_follow(&mut self, now: u64) {
        let views = self.notch_views();
        let mut moves: Vec<(Key, f64)> = Vec::new();
        for nv in views.iter().filter(|v| !v.guest) {
            for cs in &nv.copy_screens {
                if let Some(t) = self.reserve_target(nv, cs) {
                    let key = (nv.key.clone(), cs.clone());
                    if self.copies.get(&key).is_some_and(|c| c.target != Some(t)) {
                        moves.push((key, t));
                    }
                }
            }
        }
        for (key, target) in moves {
            let Some(c) = self.copies.get_mut(&key) else {
                continue;
            };
            c.target = Some(target);
            let Some(held) = c.space else {
                c.space = Some(target);
                continue;
            };
            match state::reserve_step(held, target) {
                state::ReserveStep::Later => {
                    self.timers
                        .after(Timer::Shrink(key), now, RESERVE_SHRINK_MS);
                }
                state::ReserveStep::None => {
                    self.timers.cancel(&Timer::Shrink(key));
                }
                state::ReserveStep::Now => {
                    self.timers.cancel(&Timer::Shrink(key.clone()));
                    if let Some(c) = self.copies.get_mut(&key) {
                        c.space = Some(target);
                    }
                }
            }
        }
    }

    /// `shrinkTimer.onTriggered`: `space = target`.
    fn reserve_settle(&mut self, key: &Key) {
        let views = self.notch_views();
        let target = views
            .iter()
            .find(|v| v.key == key.0)
            .and_then(|nv| self.reserve_target(nv, &key.1));
        if let (Some(t), Some(c)) = (target, self.copies.get_mut(key)) {
            c.space = Some(t);
        }
    }

    /// `state.reserve`: `State.reserveZones` over every copy's space, on the
    /// screens it keeps clear, along its notch's edge.
    fn reserve_state(&self, views: &[NotchView]) -> Vec<ps::Reserve> {
        let mut reserves = Vec::new();
        for nv in views.iter().filter(|v| !v.guest) {
            for cs in &nv.copy_screens {
                let space = self
                    .copies
                    .get(&(nv.key.clone(), cs.clone()))
                    .and_then(|c| c.space);
                reserves.push(space.map(|space| state::Reserve {
                    screens: self.reserved_screens(nv, cs),
                    edge: nv.pos.edge.clone(),
                    space,
                }));
            }
        }
        state::reserve_zones(&reserves)
            .into_iter()
            .filter_map(|(zone, depth)| {
                let (screen, edge) = zone.rsplit_once('|')?;
                Some(ps::Reserve {
                    screen: screen.to_string(),
                    edge: to_edge(edge),
                    depth: Num(depth),
                })
            })
            .collect()
    }

    /// How long a guest's window lingers after it closes (5000 ms; tests and
    /// the daemon's own tests shorten it).
    pub fn set_window_linger(&mut self, ms: u64) {
        self.window_linger = ms;
    }

    // ----------------------------------------------------------- schema

    /// A module's settings descriptor, as its JSON file or `module.json`
    /// gives it: `{ moduleId, title, settings, standalone }`.
    pub fn set_module_def(&mut self, def: Value) {
        if let Some(id) = def.get("moduleId").and_then(Value::as_str) {
            self.module_defs.insert(id.to_string(), def.clone());
        }
    }

    /// The schema as last sent (what a UI that just connected gets).
    pub fn schema(&self) -> Option<Schema> {
        self.last_schema.clone()
    }

    pub fn remove_module_def(&mut self, id: &str) {
        self.module_defs.remove(id);
    }

    /// The `schema` for the config, screens and modules now (rev 0: the
    /// rev is given as it is sent).
    pub fn compute_schema(&self) -> Schema {
        let notches = config::notches(&self.config, &DEFAULT_MODULES);
        let catalog: Vec<Value> = self.module_defs.values().cloned().collect();
        let screens: Vec<String> = self.screen_names();
        schema::build(&schema::Inputs {
            config: &self.config,
            notches: &notches,
            settings_notch: self.settings_notch,
            screens: &screens,
            supports_workspaces: self.screens.as_ref().is_some_and(|s| s.supports_workspaces),
            catalog: &catalog,
            default_modules: &DEFAULT_MODULES,
        })
    }

    // -------------------------------------------------------------- ipc

    /// IPC `state` (docs/ipc.md): the JSON string.
    pub fn ipc_state(&self) -> String {
        let views = self.notch_views();
        let copy_json = |nv: &NotchView| -> Vec<Value> {
            nv.copy_screens
                .iter()
                .map(|cs| {
                    let v = self.copy_view(nv, cs);
                    let peek = self
                        .copies
                        .get(&(nv.key.clone(), cs.clone()))
                        .and_then(|c| c.peek.as_ref())
                        .map(|p| json!(p.module))
                        .unwrap_or(Value::Null);
                    json!({"screen": v.screen, "mode": to_mode_str(v.mode), "peek": peek})
                })
                .collect()
        };
        let selected_id = self.nav.as_ref().and_then(|n| {
            views
                .iter()
                .find(|v| !v.guest && v.index.is_some_and(|i| Some(i as i64) == Some(n.selected)))
                .map(|v| v.id.clone())
        });
        let notches: Vec<Value> = views
            .iter()
            .filter(|v| !v.guest)
            .map(|nv| {
                let spec = nv.spec.as_ref();
                json!({
                    "id": nv.id,
                    "reserveSpace": spec.is_some_and(|s| s.reserve_space),
                    "tabs": nv.tabs,
                    "idleIcons": spec.is_none_or(|s| s.idle_icons),
                    "workspaces": spec.and_then(|s| s.workspaces.clone()).map(Value::Array).unwrap_or(Value::Null),
                    "position": {"edge": nv.pos.edge, "offset": nv.pos.offset, "screen": nv.pos.screen},
                    "draggable": spec.is_none_or(|s| s.draggable),
                    "modules": nv.module_ids,
                    "copies": copy_json(nv),
                })
            })
            .collect();
        let standalone: Vec<Value> = views
            .iter()
            .filter(|v| v.guest && !v.visitor)
            .map(|nv| {
                let copies: Vec<Value> = nv
                    .copy_screens
                    .iter()
                    .filter_map(|cs| {
                        let v = self.copy_view(nv, cs);
                        v.open().then(|| json!({"screen": v.screen, "mode": to_mode_str(v.mode)}))
                    })
                    .collect();
                json!({
                    "id": nv.id,
                    "position": {"edge": nv.pos.edge, "offset": nv.pos.offset, "screen": nv.pos.screen},
                    "copies": copies,
                })
            })
            .collect();
        // The visitor's copies that show something (`NotchService.state`).
        let visitor: Vec<Value> = views
            .iter()
            .filter(|v| v.visitor)
            .flat_map(|nv| {
                nv.copy_screens.iter().filter_map(|cs| {
                    let v = self.copy_view(nv, cs);
                    let peek = self
                        .copies
                        .get(&(nv.key.clone(), cs.clone()))
                        .and_then(|c| c.peek.as_ref())
                        .map_or(Value::Null, |p| json!(p.module));
                    v.open().then(
                        || json!({"screen": v.screen, "mode": to_mode_str(v.mode), "peek": peek}),
                    )
                })
            })
            .collect();
        let modules: Vec<Value> = self
            .modules
            .values()
            .map(|m| json!({"id": m.info.id, "active": m.info.active, "priority": Num(m.info.priority)}))
            .collect();
        json!({
            "navigation": selected_id.map(|id| json!({"notch": id})).unwrap_or(Value::Null),
            "notches": notches,
            "visitor": visitor,
            "standalone": standalone,
            "modules": modules,
        })
        .to_string()
    }
}

fn to_mode_str(m: state::Mode) -> &'static str {
    match m {
        state::Mode::Peek => "peek",
        state::Mode::Expanded => "expanded",
        state::Mode::Compact => "compact",
        state::Mode::Rest => "rest",
    }
}

/// An `edit` op, without the wire types.
#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    ToggleModule(String),
    MoveModule(String, i64),
    ToggleWorkspace(String),
}

#[cfg(test)]
mod tests;
