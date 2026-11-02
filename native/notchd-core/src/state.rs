//! Port of `core/State.js`: the notch's state rules (which modules show, which
//! peek wins, which mode the notch is in, keyboard, reserved space, screens).
//!
//! Inputs are typed structs that deserialize from the same camelCase JSON the
//! QML side builds. JS truthiness of module members (`compact`, `peek`, ...)
//! is kept: any non-null, non-false, non-zero, non-empty value counts as set.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// JS truthiness of a JSON value, for members QML fills with objects or strings.
fn truthy<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    use serde_json::Value;
    Ok(match Value::deserialize(d)? {
        Value::Null => false,
        Value::Bool(b) => b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    })
}

/// A module as the rules see it (`active`, `priority`, `compact`, `peek`,
/// `icon`, `attention`, `urgentPeek`, `moduleId`). The view members only
/// matter for whether they are set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModuleInfo {
    pub id: String,
    /// The id a tab is picked by (`shownModules`); absent for none.
    pub module_id: Option<String>,
    pub priority: f64,
    pub active: bool,
    /// Has a compact view.
    #[serde(deserialize_with = "truthy")]
    pub compact: bool,
    /// Has an icon to show closed.
    #[serde(deserialize_with = "truthy")]
    pub icon: bool,
    /// Has a peek view.
    #[serde(deserialize_with = "truthy")]
    pub peek: bool,
    /// Something not seen yet.
    #[serde(deserialize_with = "truthy")]
    pub attention: bool,
    /// Its peek gets through a hovered notch.
    #[serde(deserialize_with = "truthy")]
    pub urgent_peek: bool,
}

/// `activeModules`: the active modules (nulls skipped), highest priority
/// first; equal priorities keep their order.
pub fn active_modules(modules: &[Option<ModuleInfo>]) -> Vec<ModuleInfo> {
    let mut out: Vec<ModuleInfo> = modules
        .iter()
        .flatten()
        .filter(|m| m.active)
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        b.priority
            .partial_cmp(&a.priority)
            .unwrap_or(Ordering::Equal)
    });
    out
}

/// `compactModule`: the first active module with a compact view.
pub fn compact_module(active: &[ModuleInfo]) -> Option<&ModuleInfo> {
    active.iter().find(|m| m.compact)
}

/// `hasIcon`: no compact view, but an icon to show closed.
pub fn has_icon(m: &ModuleInfo) -> bool {
    !m.compact && m.icon
}

/// `compactModules`: the modules the closed notch shows side by side. With
/// `tabs`, every one with a compact view (or, with `icons`, an icon); else
/// just the compact module, or with `icons` the first icon when none has a
/// compact view.
pub fn compact_modules(active: &[ModuleInfo], tabs: bool, icons: bool) -> Vec<&ModuleInfo> {
    if !tabs {
        if let Some(one) = compact_module(active) {
            return vec![one];
        }
        if icons {
            return active.iter().find(|m| has_icon(m)).into_iter().collect();
        }
        return Vec::new();
    }
    active
        .iter()
        .filter(|m| m.compact || (icons && has_icon(m)))
        .collect()
}

/// `attentionPeek`: the first module (nulls skipped) with something unseen
/// and a peek view.
pub fn attention_peek(modules: &[Option<ModuleInfo>]) -> Option<&ModuleInfo> {
    modules.iter().flatten().find(|m| m.attention && m.peek)
}

/// What `canPeek` looks at (`s` in JS).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PeekContext {
    pub pinned: bool,
    pub hovered: bool,
    /// The module peeking, if any. Identity is the `id`.
    pub current: Option<ModuleInfo>,
    /// Whether its peek is still on screen.
    pub current_showing: bool,
}

/// `canPeek`: whether `module` may peek now. A pinned notch never peeks;
/// hover only lets urgent peeks through; a peek on screen is not cut short by
/// a lower-priority one.
pub fn can_peek(module: Option<&ModuleInfo>, s: &PeekContext) -> bool {
    let Some(module) = module else { return false };
    if !module.peek || s.pinned {
        return false;
    }
    if s.hovered && !module.urgent_peek {
        return false;
    }
    if let Some(current) = &s.current
        && current.id != module.id
        && s.current_showing
        && current.priority > module.priority
    {
        return false;
    }
    true
}

/// The notch's mode, from `mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Peek,
    Expanded,
    Compact,
    Rest,
}

/// What `mode` looks at. `peek_module` / `compact_module` are whether a
/// module is peeking / owns the compact slot (the JS passes the module).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModeContext {
    #[serde(deserialize_with = "truthy")]
    pub peek_module: bool,
    #[serde(deserialize_with = "truthy")]
    pub compact_module: bool,
    pub pinned: bool,
    pub hovered: bool,
    pub dragging: bool,
    /// A copy of the visitor: hidden while closed, so it rests.
    pub guest: bool,
    pub navigating: bool,
    /// The notch selected in navigation.
    pub preview: bool,
}

/// `mode`: peek, expanded, compact or rest. Dragged, it stays closed; a guest
/// rests instead of compact; while navigating only the previewed notch is open.
pub fn mode(s: &ModeContext) -> Mode {
    let closed = if s.compact_module && !s.guest {
        Mode::Compact
    } else {
        Mode::Rest
    };
    if s.dragging {
        return closed;
    }
    if s.navigating {
        return if s.preview { Mode::Expanded } else { closed };
    }
    if s.peek_module && !s.pinned {
        return Mode::Peek;
    }
    if s.pinned || s.hovered {
        return Mode::Expanded;
    }
    closed
}

/// What `needsWindow` looks at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowContext {
    pub guest: bool,
    pub open: bool,
    pub lingering: bool,
}

/// `needsWindow`: a notch of the config always has its window; a guest only
/// while open and lingering after.
pub fn needs_window(s: &WindowContext) -> bool {
    !s.guest || s.open || s.lingering
}

/// The result of `shownModules`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShownModules<'a> {
    /// What the open notch shows.
    pub modules: Vec<&'a ModuleInfo>,
    /// The modules to make tabs of; empty for none.
    pub tabs: Vec<&'a ModuleInfo>,
}

/// `shownModules`: all the expanded modules, or with `tabs` and more than one,
/// the one whose `module_id` is `picked`, else the first. (JS `null` modules
/// is an empty slice.)
pub fn shown_modules<'a>(modules: &'a [ModuleInfo], tabs: bool, picked: &str) -> ShownModules<'a> {
    if !tabs || modules.len() < 2 {
        return ShownModules {
            modules: modules.iter().collect(),
            tabs: Vec::new(),
        };
    }
    let one = modules
        .iter()
        .find(|m| m.module_id.as_deref() == Some(picked))
        .unwrap_or(&modules[0]);
    ShownModules {
        modules: vec![one],
        tabs: modules.iter().collect(),
    }
}

/// What `wantsKeyboard`, `keyboardFocus` and `keyboardChange` look at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyboardContext {
    pub pinned: bool,
    /// The copy's mode (`mode`); absent counts as not expanded.
    pub mode: Option<Mode>,
    pub navigating: bool,
    /// Its window had the keyboard since the copy last wanted it.
    pub held: bool,
    /// On a screen without focus.
    pub elsewhere: bool,
    /// Its window has the keyboard now (`keyboardChange`).
    pub active: bool,
    /// Just after navigation ended.
    pub settling: bool,
}

/// `wantsKeyboard`: pinned open, whatever it shows; never on hover alone.
pub fn wants_keyboard(s: &KeyboardContext) -> bool {
    s.pinned && s.mode == Some(Mode::Expanded)
}

/// The keyboard interactivity a copy's window takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyboardFocus {
    None,
    Exclusive,
    #[serde(rename = "ondemand")]
    OnDemand,
}

/// `keyboardFocus`: exclusive until the window has it, then on demand (but
/// exclusive on a screen without focus); none while navigating or unwanted.
pub fn keyboard_focus(s: &KeyboardContext) -> KeyboardFocus {
    if s.navigating || !wants_keyboard(s) {
        return KeyboardFocus::None;
    }
    if s.held && !s.elsewhere {
        KeyboardFocus::OnDemand
    } else {
        KeyboardFocus::Exclusive
    }
}

/// What a copy does as its window gains or loses the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyboardChange {
    None,
    Hold,
    Close,
    Retake,
}

/// `keyboardChange`: hold (it has it), close (a window took it), retake (lost
/// just after navigation ended) or none.
pub fn keyboard_change(s: &KeyboardContext) -> KeyboardChange {
    if s.navigating || !wants_keyboard(s) {
        return KeyboardChange::None;
    }
    if s.active {
        return KeyboardChange::Hold;
    }
    if !s.held {
        return KeyboardChange::None;
    }
    if s.settling {
        KeyboardChange::Retake
    } else {
        KeyboardChange::Close
    }
}

/// What `reservedSpace` looks at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReserveContext {
    pub enabled: bool,
    /// A module shows a compact view.
    pub compact: bool,
    /// The least reserved for a compact view.
    pub baseline: f64,
    /// The tallest compact view seen.
    pub tallest: f64,
    /// The rest lip.
    pub rest: f64,
}

/// `reservedSpace`: how far windows are kept from the notch's edge, in px; 0
/// reserves nothing.
pub fn reserved_space(s: &ReserveContext) -> f64 {
    let num = |x: f64| if x.is_nan() { 0.0 } else { x };
    if !s.enabled {
        return 0.0;
    }
    if !s.compact {
        return num(s.rest).ceil();
    }
    num(s.baseline).max(num(s.tallest)).ceil()
}

/// When the reserved space changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReserveStep {
    None,
    Now,
    Later,
}

/// `reserveStep`: now to grow or drop it altogether, later to shrink, none
/// when nothing changes.
pub fn reserve_step(held: f64, target: f64) -> ReserveStep {
    if target == held {
        ReserveStep::None
    } else if target > held || target == 0.0 {
        ReserveStep::Now
    } else {
        ReserveStep::Later
    }
}

/// A workspace id or name as the config writes it: a number or a string,
/// compared as text (`String(x)` in JS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkspaceKey {
    Number(f64),
    Text(String),
}

impl fmt::Display for WorkspaceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceKey::Number(n) => write!(f, "{n}"),
            WorkspaceKey::Text(s) => f.write_str(s),
        }
    }
}

/// A compositor workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceKey,
    pub name: String,
}

/// `workspaceAllowed`: whether the notch hangs on a workspace. `list` is the
/// config's `workspaces` (None or empty: every one); entries match a
/// workspace's id or name. An unknown workspace is allowed.
pub fn workspace_allowed(list: Option<&[WorkspaceKey]>, ws: Option<&Workspace>) -> bool {
    let (Some(list), Some(ws)) = (list, ws) else {
        return true;
    };
    if list.is_empty() {
        return true;
    }
    let (id, name) = (ws.id.to_string(), &ws.name);
    list.iter().any(|k| {
        let k = k.to_string();
        k == id || k == *name
    })
}

/// A screen and whether its workspace takes the notch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenState {
    pub name: String,
    pub allowed: bool,
}

/// What `notchScreen` looks at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScreenContext {
    pub screens: Vec<ScreenState>,
    pub focused: String,
    /// Peeks or is expanded.
    pub open: bool,
    /// The screen it was opened from (counts only while open); "" for none.
    #[serde(rename = "override")]
    pub override_screen: String,
    /// The screen the config's position names; "" for none.
    pub home: String,
}

/// `notchScreen`: the screen the notch goes on, by name; "" for none. Closed:
/// home if allowed, else the focused screen if allowed, else the first
/// allowed. Open: where the pointer opened it; else where it hangs closed
/// (with a home screen), else the focused screen.
pub fn notch_screen(s: &ScreenContext) -> String {
    let find = |name: &str| s.screens.iter().find(|x| x.name == name);
    let focused = find(&s.focused).or(s.screens.first());
    let home = if s.home.is_empty() {
        None
    } else {
        find(&s.home)
    };
    let closed = || -> String {
        if let Some(h) = home.filter(|h| h.allowed) {
            return h.name.clone();
        }
        if let Some(f) = focused.filter(|f| f.allowed) {
            return f.name.clone();
        }
        s.screens
            .iter()
            .find(|x| x.allowed)
            .map(|x| x.name.clone())
            .unwrap_or_default()
    };
    if !s.open {
        return closed();
    }
    if !s.override_screen.is_empty()
        && let Some(over) = find(&s.override_screen)
    {
        return over.name.clone();
    }
    if let Some(h) = home {
        let c = closed();
        return if c.is_empty() { h.name.clone() } else { c };
    }
    focused.map(|f| f.name.clone()).unwrap_or_default()
}

/// `reserveScreens`: the screens that keep windows clear of the closed notch:
/// just `home` while it can hang there, else every allowed screen.
pub fn reserve_screens(screens: &[ScreenState], home: &str) -> Vec<String> {
    let allowed: Vec<&ScreenState> = screens.iter().filter(|s| s.allowed).collect();
    if !home.is_empty() && allowed.iter().any(|s| s.name == home) {
        return vec![home.to_string()];
    }
    allowed.iter().map(|s| s.name.clone()).collect()
}

/// `peekCopy`: the copy of a notch that shows a peek, by screen name: the one
/// on the focused screen, else the first; None for no copies.
pub fn peek_copy<'a>(copies: &'a [String], focused: &str) -> Option<&'a str> {
    match copies {
        [] => None,
        [one] => Some(one),
        _ => copies
            .iter()
            .find(|c| *c == focused)
            .or(copies.first())
            .map(String::as_str),
    }
}

/// How a notch that shows a module hangs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeekKind {
    /// A copy on every screen.
    All,
    /// It moves to the focused screen.
    Follow,
    /// Kept on a screen.
    Homed,
}

/// A notch that shows a module, for `needsVisitor` and `focusedPeek`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeekNotch {
    pub kind: PeekKind,
    /// Where a `Homed` notch hangs.
    #[serde(default)]
    pub screen: Option<String>,
}

impl PeekNotch {
    fn peeks_on(&self, focused: &str) -> bool {
        self.kind != PeekKind::Homed || self.screen.as_deref() == Some(focused)
    }
}

/// `needsVisitor`: whether a peek also needs the visitor: no notch peeks on
/// the focused screen. Never with the focused screen unknown ("").
pub fn needs_visitor(peeks: &[PeekNotch], focused: &str) -> bool {
    !focused.is_empty() && !peeks.iter().any(|p| p.peeks_on(focused))
}

/// `focusedPeek`: the index of the first notch that peeks on the focused
/// screen; None (JS -1) for the visitor. With the focused screen unknown
/// (""), the first notch; None with none.
pub fn focused_peek(peeks: &[PeekNotch], focused: &str) -> Option<usize> {
    if focused.is_empty() {
        return if peeks.is_empty() { None } else { Some(0) };
    }
    peeks.iter().position(|p| p.peeks_on(focused))
}

/// One notch copy's reserve, for `reserveZones`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reserve {
    pub screens: Vec<String>,
    pub edge: String,
    pub space: f64,
}

/// `reserveZones`: what windows leave free per screen edge: the deepest
/// reserve on each, as `"<screen>|<edge>" -> space`, spaces above 0 only.
pub fn reserve_zones(reserves: &[Option<Reserve>]) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for r in reserves.iter().flatten() {
        if r.space.is_nan() || r.space <= 0.0 {
            continue;
        }
        for screen in &r.screens {
            let slot = out.entry(format!("{screen}|{}", r.edge)).or_insert(0.0_f64);
            *slot = slot.max(r.space);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, priority: f64) -> ModuleInfo {
        ModuleInfo {
            id: id.into(),
            priority,
            active: true,
            peek: true,
            ..Default::default()
        }
    }

    fn ids(list: &[&ModuleInfo]) -> Vec<String> {
        list.iter().map(|m| m.id.clone()).collect()
    }

    fn screens(list: &[(&str, bool)]) -> Vec<ScreenState> {
        list.iter()
            .map(|(n, a)| ScreenState {
                name: (*n).into(),
                allowed: *a,
            })
            .collect()
    }

    fn strs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).into()).collect()
    }

    fn ctx(s: &[ScreenState], focused: &str) -> ScreenContext {
        ScreenContext {
            screens: s.to_vec(),
            focused: focused.into(),
            ..Default::default()
        }
    }

    fn homed(screen: &str) -> PeekNotch {
        PeekNotch {
            kind: PeekKind::Homed,
            screen: Some(screen.into()),
        }
    }

    fn kind(kind: PeekKind) -> PeekNotch {
        PeekNotch { kind, screen: None }
    }

    // activeModules keeps active ones, highest priority first
    #[test]
    fn active_modules_keeps_active_ones_highest_priority_first() {
        let list = vec![
            Some(m("media", 50.0)),
            Some(m("osd", 100.0)),
            Some(ModuleInfo {
                active: false,
                ..m("off", 90.0)
            }),
            None,
        ];
        let got: Vec<String> = active_modules(&list).into_iter().map(|m| m.id).collect();
        assert_eq!(got, ["osd", "media"]);
    }

    // compactModule is the first active module with a compact view
    #[test]
    fn compact_module_is_the_first_active_module_with_a_compact_view() {
        let compact = |id, p| ModuleInfo {
            compact: true,
            ..m(id, p)
        };
        let active = [
            m("osd", 100.0),
            compact("media", 50.0),
            compact("act", 40.0),
        ];
        assert_eq!(compact_module(&active).unwrap().id, "media");
        assert_eq!(compact_module(&[m("osd", 100.0)]), None);
    }

    // canPeek: pinned blocks, hover only lets urgent peeks through
    #[test]
    fn can_peek_pinned_blocks_hover_only_lets_urgent_peeks_through() {
        let media = m("media", 50.0);
        let pinned = PeekContext {
            pinned: true,
            ..Default::default()
        };
        let hovered = PeekContext {
            hovered: true,
            ..Default::default()
        };
        assert!(!can_peek(Some(&media), &pinned));
        assert!(!can_peek(Some(&media), &hovered));
        let osd = ModuleInfo {
            urgent_peek: true,
            ..m("osd", 100.0)
        };
        assert!(can_peek(Some(&osd), &hovered));
        let no_peek = ModuleInfo {
            peek: false,
            ..m("x", 1.0)
        };
        assert!(!can_peek(Some(&no_peek), &PeekContext::default()));
        assert!(!can_peek(None, &PeekContext::default()));
    }

    // canPeek: a lower-priority peek waits for a higher one on screen
    #[test]
    fn can_peek_a_lower_priority_peek_waits_for_a_higher_one_on_screen() {
        let osd = m("osd", 100.0);
        let media = m("media", 50.0);
        let with = |current: &ModuleInfo, showing| PeekContext {
            current: Some(current.clone()),
            current_showing: showing,
            ..Default::default()
        };
        assert!(!can_peek(Some(&media), &with(&osd, true)));
        assert!(can_peek(Some(&media), &with(&osd, false)));
        assert!(can_peek(Some(&osd), &with(&media, true)));
        assert!(can_peek(Some(&osd), &with(&osd, true)));
    }

    // mode: peek beats expanded unless pinned, then compact, then rest
    #[test]
    fn mode_peek_beats_expanded_unless_pinned_then_compact_then_rest() {
        let base = ModeContext::default();
        assert_eq!(
            mode(&ModeContext {
                peek_module: true,
                ..base.clone()
            }),
            Mode::Peek
        );
        assert_eq!(
            mode(&ModeContext {
                peek_module: true,
                pinned: true,
                ..base.clone()
            }),
            Mode::Expanded
        );
        assert_eq!(
            mode(&ModeContext {
                hovered: true,
                ..base.clone()
            }),
            Mode::Expanded
        );
        assert_eq!(
            mode(&ModeContext {
                compact_module: true,
                ..base.clone()
            }),
            Mode::Compact
        );
        assert_eq!(mode(&base), Mode::Rest);
    }

    // mode: a dragged notch stays closed whatever else is going on
    #[test]
    fn mode_a_dragged_notch_stays_closed_whatever_else_is_going_on() {
        let all = ModeContext {
            dragging: true,
            hovered: true,
            pinned: true,
            peek_module: true,
            compact_module: true,
            ..Default::default()
        };
        assert_eq!(mode(&all), Mode::Compact);
        let no_compact = ModeContext {
            dragging: true,
            hovered: true,
            peek_module: true,
            ..Default::default()
        };
        assert_eq!(mode(&no_compact), Mode::Rest);
    }

    // mode: a guest copy rests while closed, and only peeks or opens
    #[test]
    fn mode_a_guest_copy_rests_while_closed_and_only_peeks_or_opens() {
        let guest = ModeContext {
            guest: true,
            compact_module: true,
            ..Default::default()
        };
        assert_eq!(mode(&guest), Mode::Rest);
        assert_eq!(
            mode(&ModeContext {
                dragging: true,
                ..guest.clone()
            }),
            Mode::Rest
        );
        assert_eq!(
            mode(&ModeContext {
                peek_module: true,
                ..guest.clone()
            }),
            Mode::Peek
        );
        assert_eq!(
            mode(&ModeContext {
                hovered: true,
                ..guest.clone()
            }),
            Mode::Expanded
        );
    }

    // mode: navigating, only the notch selected is open, over a peek, not while dragged
    #[test]
    fn mode_navigating_only_the_notch_selected_is_open_over_a_peek_not_while_dragged() {
        let nav = ModeContext {
            navigating: true,
            preview: true,
            ..Default::default()
        };
        assert_eq!(mode(&nav), Mode::Expanded);
        let busy = ModeContext {
            compact_module: true,
            peek_module: true,
            ..nav.clone()
        };
        assert_eq!(mode(&busy), Mode::Expanded);
        let dragged = ModeContext {
            compact_module: true,
            dragging: true,
            ..nav.clone()
        };
        assert_eq!(mode(&dragged), Mode::Compact);
    }

    // mode: navigating, a notch not selected shows closed, even pinned or hovered
    #[test]
    fn mode_navigating_a_notch_not_selected_shows_closed_even_pinned_or_hovered() {
        let nav = ModeContext {
            navigating: true,
            ..Default::default()
        };
        assert_eq!(
            mode(&ModeContext {
                pinned: true,
                compact_module: true,
                ..nav.clone()
            }),
            Mode::Compact
        );
        assert_eq!(
            mode(&ModeContext {
                hovered: true,
                ..nav.clone()
            }),
            Mode::Rest
        );
        assert_eq!(
            mode(&ModeContext {
                compact_module: true,
                peek_module: true,
                ..nav.clone()
            }),
            Mode::Compact
        );
    }

    // needsWindow: a guest only while open or lingering after; a notch always
    #[test]
    fn needs_window_a_guest_only_while_open_or_lingering_after_a_notch_always() {
        assert!(needs_window(&WindowContext::default()));
        assert!(!needs_window(&WindowContext {
            guest: true,
            ..Default::default()
        }));
        assert!(needs_window(&WindowContext {
            guest: true,
            open: true,
            ..Default::default()
        }));
        assert!(needs_window(&WindowContext {
            guest: true,
            lingering: true,
            ..Default::default()
        }));
    }

    // shownModules: every expanded module, or with tabs the picked one
    #[test]
    fn shown_modules_every_expanded_module_or_with_tabs_the_picked_one() {
        let with_id = |id: &str, p| ModuleInfo {
            module_id: Some(id.into()),
            ..m(id, p)
        };
        let media = with_id("media", 60.0);
        let claude = with_id("claude", 40.0);
        let codex = with_id("codex", 40.0);
        let got = |r: ShownModules| (ids(&r.modules), ids(&r.tabs));
        let all = [media, claude.clone(), codex.clone()];
        // Without tabs, all of them, one above the other.
        assert_eq!(
            got(shown_modules(&all, false, "codex")),
            (strs(&["media", "claude", "codex"]), vec![])
        );
        let two = [claude.clone(), codex];
        let tabs = strs(&["claude", "codex"]);
        // With tabs, the picked one; none picked, or one that went, the first.
        assert_eq!(
            got(shown_modules(&two, true, "codex")),
            (strs(&["codex"]), tabs.clone())
        );
        assert_eq!(
            got(shown_modules(&two, true, "")),
            (strs(&["claude"]), tabs.clone())
        );
        assert_eq!(
            got(shown_modules(&two, true, "media")),
            (strs(&["claude"]), tabs)
        );
        // One module needs no tabs.
        assert_eq!(
            got(shown_modules(&[claude], true, "codex")),
            (strs(&["claude"]), vec![])
        );
        assert_eq!(got(shown_modules(&[], true, "")), (vec![], vec![]));
    }

    // compactModules: with tabs every active compact view, else the compact module
    #[test]
    fn compact_modules_with_tabs_every_active_compact_view_else_the_compact_module() {
        let media = ModuleInfo {
            compact: true,
            ..m("media", 60.0)
        };
        let claude = ModuleInfo {
            compact: true,
            ..m("claude", 40.0)
        };
        let plain = m("osd", 100.0);
        let list = [plain.clone(), media, claude];
        assert_eq!(ids(&compact_modules(&list, false, false)), ["media"]);
        assert_eq!(
            ids(&compact_modules(&list, true, false)),
            ["media", "claude"]
        );
        assert!(compact_modules(std::slice::from_ref(&plain), false, false).is_empty());
        assert!(compact_modules(&[plain], true, false).is_empty());
        assert!(compact_modules(&[], true, false).is_empty());
    }

    // compactModules: with icons an idle module shows its icon
    #[test]
    fn compact_modules_with_icons_an_idle_module_shows_its_icon() {
        let claude = ModuleInfo {
            compact: true,
            ..m("claude", 40.0)
        };
        let timer = ModuleInfo {
            icon: true,
            ..m("timer", 60.0)
        }; // idle: no compact view
        let bare = m("resources", 30.0); // no icon: stays out
        // With tabs, every module that has something: its view or its icon.
        let a = [timer.clone(), claude.clone(), bare.clone()];
        assert_eq!(ids(&compact_modules(&a, true, true)), ["timer", "claude"]);
        assert_eq!(ids(&compact_modules(&a, true, false)), ["claude"]);
        // Without tabs, the compact module first; the icon only without one.
        let b = [timer.clone(), claude];
        assert_eq!(ids(&compact_modules(&b, false, true)), ["claude"]);
        let c = [bare, timer.clone()];
        assert_eq!(ids(&compact_modules(&c, false, true)), ["timer"]);
        assert!(compact_modules(&[timer], false, false).is_empty());
    }

    // attentionPeek: the first module with something unseen and a peek
    #[test]
    fn attention_peek_the_first_module_with_something_unseen_and_a_peek() {
        let codex = ModuleInfo {
            attention: false,
            ..m("codex", 40.0)
        };
        let claude = ModuleInfo {
            attention: true,
            ..m("claude", 40.0)
        };
        let no_peek = ModuleInfo {
            attention: true,
            peek: false,
            ..m("silent", 90.0)
        };
        let list = [Some(no_peek), Some(codex.clone()), Some(claude)];
        assert_eq!(attention_peek(&list).unwrap().id, "claude");
        assert_eq!(attention_peek(&[Some(codex)]), None);
        assert_eq!(attention_peek(&[]), None);
    }

    fn open() -> KeyboardContext {
        KeyboardContext {
            pinned: true,
            mode: Some(Mode::Expanded),
            ..Default::default()
        }
    }

    // keyboardFocus: pinned open, whatever it shows; never on hover alone
    #[test]
    fn keyboard_focus_pinned_open_whatever_it_shows_never_on_hover_alone() {
        assert_eq!(keyboard_focus(&open()), KeyboardFocus::Exclusive);
        // hover alone
        assert_eq!(
            keyboard_focus(&KeyboardContext {
                pinned: false,
                ..open()
            }),
            KeyboardFocus::None
        );
        assert_eq!(
            keyboard_focus(&KeyboardContext {
                mode: Some(Mode::Compact),
                ..open()
            }),
            KeyboardFocus::None
        );
    }

    // keyboardFocus: exclusive until the window has the keyboard, then on demand
    #[test]
    fn keyboard_focus_exclusive_until_the_window_has_the_keyboard_then_on_demand() {
        assert_eq!(
            keyboard_focus(&KeyboardContext {
                held: false,
                ..open()
            }),
            KeyboardFocus::Exclusive
        );
        assert_eq!(
            keyboard_focus(&KeyboardContext {
                held: true,
                ..open()
            }),
            KeyboardFocus::OnDemand
        );
        assert_eq!(
            keyboard_focus(&KeyboardContext {
                pinned: false,
                held: true,
                ..open()
            }),
            KeyboardFocus::None
        );
    }

    // keyboardFocus: on a screen without focus, it stays exclusive
    #[test]
    fn keyboard_focus_on_a_screen_without_focus_it_stays_exclusive() {
        let away = KeyboardContext {
            held: true,
            elsewhere: true,
            ..open()
        };
        assert_eq!(keyboard_focus(&away), KeyboardFocus::Exclusive);
        assert_eq!(
            keyboard_focus(&KeyboardContext {
                elsewhere: false,
                ..away
            }),
            KeyboardFocus::OnDemand
        );
    }

    // keyboardChange: losing the keyboard it held closes the notch
    #[test]
    fn keyboard_change_losing_the_keyboard_it_held_closes_the_notch() {
        let c = |active, held| KeyboardContext {
            active,
            held,
            ..open()
        };
        assert_eq!(keyboard_change(&c(true, false)), KeyboardChange::Hold);
        assert_eq!(keyboard_change(&c(false, true)), KeyboardChange::Close);
        // Not had yet (the compositor hasn't given it): nothing to give back.
        assert_eq!(keyboard_change(&c(false, false)), KeyboardChange::None);
        // Closed: the keyboard goes back by itself, no close.
        let closed = KeyboardContext {
            pinned: false,
            ..c(false, true)
        };
        assert_eq!(keyboard_change(&closed), KeyboardChange::None);
    }

    // keyboard: while navigating no notch asks for it, and losing it to navigation closes nothing
    #[test]
    fn keyboard_while_navigating_no_notch_asks_for_it_and_losing_it_closes_nothing() {
        let nav = KeyboardContext {
            navigating: true,
            ..open()
        };
        assert_eq!(keyboard_focus(&nav), KeyboardFocus::None);
        let lost = KeyboardContext {
            active: false,
            held: true,
            ..nav
        };
        assert_eq!(keyboard_change(&lost), KeyboardChange::None);
    }

    // keyboardChange: lost just after navigation ended, it asks again instead of closing
    #[test]
    fn keyboard_change_lost_just_after_navigation_ended_it_asks_again_instead_of_closing() {
        let c = |active, held| KeyboardContext {
            settling: true,
            active,
            held,
            ..open()
        };
        assert_eq!(keyboard_change(&c(false, true)), KeyboardChange::Retake);
        assert_eq!(keyboard_change(&c(false, false)), KeyboardChange::None);
        assert_eq!(keyboard_change(&c(true, false)), KeyboardChange::Hold);
    }

    // reservedSpace: only when enabled; compact never below the baseline
    #[test]
    fn reserved_space_only_when_enabled_compact_never_below_the_baseline() {
        let on = ReserveContext {
            enabled: true,
            compact: true,
            baseline: 36.0,
            tallest: 0.0,
            rest: 5.0,
        };
        assert_eq!(reserved_space(&on), 36.0);
        assert_eq!(
            reserved_space(&ReserveContext {
                tallest: 41.2,
                ..on.clone()
            }),
            42.0
        );
        assert_eq!(
            reserved_space(&ReserveContext {
                tallest: 20.0,
                ..on.clone()
            }),
            36.0
        );
        assert_eq!(
            reserved_space(&ReserveContext {
                enabled: false,
                ..on
            }),
            0.0
        );
    }

    // reservedSpace: at rest, only the lip
    #[test]
    fn reserved_space_at_rest_only_the_lip() {
        let rest = ReserveContext {
            enabled: true,
            compact: false,
            baseline: 36.0,
            tallest: 50.0,
            rest: 5.4,
        };
        assert_eq!(reserved_space(&rest), 6.0);
    }

    // reserveStep: grow or drop now, shrink later, nothing when equal
    #[test]
    fn reserve_step_grow_or_drop_now_shrink_later_nothing_when_equal() {
        assert_eq!(reserve_step(6.0, 33.0), ReserveStep::Now);
        assert_eq!(reserve_step(33.0, 0.0), ReserveStep::Now);
        assert_eq!(reserve_step(33.0, 6.0), ReserveStep::Later);
        assert_eq!(reserve_step(33.0, 33.0), ReserveStep::None);
    }

    // workspaceAllowed matches ids or names; no list or no workspace allows
    #[test]
    fn workspace_allowed_matches_ids_or_names_no_list_or_no_workspace_allows() {
        use WorkspaceKey::{Number, Text};
        let ws = |id: f64, name: &str| Workspace {
            id: Number(id),
            name: name.into(),
        };
        let t = |s: &str| Text(s.into());
        assert!(workspace_allowed(None, Some(&ws(5.0, "5"))));
        assert!(workspace_allowed(Some(&[]), Some(&ws(5.0, "5"))));
        assert!(workspace_allowed(Some(&[Number(1.0), Number(2.0)]), None));
        assert!(workspace_allowed(
            Some(&[Number(1.0), Number(2.0)]),
            Some(&ws(2.0, "2"))
        ));
        assert!(workspace_allowed(Some(&[t("2")]), Some(&ws(2.0, "2"))));
        assert!(workspace_allowed(
            Some(&[t("code")]),
            Some(&ws(7.0, "code"))
        ));
        assert!(!workspace_allowed(
            Some(&[Number(1.0), Number(2.0), Number(3.0)]),
            Some(&ws(4.0, "4"))
        ));
    }

    // notchScreen: closed, the focused screen if allowed, else the first allowed one
    #[test]
    fn notch_screen_closed_the_focused_screen_if_allowed_else_the_first_allowed_one() {
        let s = screens(&[("DP-1", true), ("eDP-1", false)]);
        assert_eq!(notch_screen(&ctx(&s, "DP-1")), "DP-1");
        assert_eq!(notch_screen(&ctx(&s, "eDP-1")), "DP-1");
        assert_eq!(notch_screen(&ctx(&screens(&[("a", false)]), "a")), "");
        assert_eq!(notch_screen(&ctx(&[], "a")), "");
    }

    // notchScreen: open, the screen it was opened from, else the focused one
    #[test]
    fn notch_screen_open_the_screen_it_was_opened_from_else_the_focused_one() {
        let s = screens(&[("DP-1", true), ("eDP-1", false)]);
        let open = |focused: &str| ScreenContext {
            open: true,
            ..ctx(&s, focused)
        };
        assert_eq!(notch_screen(&open("eDP-1")), "eDP-1");
        assert_eq!(
            notch_screen(&ScreenContext {
                override_screen: "DP-1".into(),
                ..open("eDP-1")
            }),
            "DP-1"
        );
        assert_eq!(notch_screen(&open("gone")), "DP-1");
        let closed = |focused: &str, over: &str| ScreenContext {
            override_screen: over.into(),
            ..ctx(&s, focused)
        };
        assert_eq!(notch_screen(&closed("eDP-1", "gone")), "DP-1");
        assert_eq!(notch_screen(&closed("DP-1", "eDP-1")), "DP-1");
    }

    // notchScreen: closed, the notch stays on its home screen whatever has focus
    #[test]
    fn notch_screen_closed_the_notch_stays_on_its_home_screen_whatever_has_focus() {
        let s = screens(&[("DP-1", true), ("eDP-1", true)]);
        let with_home = |s: &[ScreenState], focused: &str, home: &str| ScreenContext {
            home: home.into(),
            ..ctx(s, focused)
        };
        assert_eq!(notch_screen(&with_home(&s, "DP-1", "eDP-1")), "eDP-1");
        // A home whose workspace does not take the notch, or that is gone, falls back.
        let busy = screens(&[("DP-1", true), ("eDP-1", false)]);
        assert_eq!(notch_screen(&with_home(&busy, "eDP-1", "eDP-1")), "DP-1");
        assert_eq!(notch_screen(&with_home(&s, "DP-1", "HDMI-A-1")), "DP-1");
    }

    // reserveScreens: only the home screen while the notch can hang there
    #[test]
    fn reserve_screens_only_the_home_screen_while_the_notch_can_hang_there() {
        let s = screens(&[("DP-1", true), ("eDP-1", true), ("HDMI-A-1", false)]);
        assert_eq!(reserve_screens(&s, "eDP-1"), ["eDP-1"]);
        assert_eq!(reserve_screens(&s, ""), ["DP-1", "eDP-1"]);
        assert_eq!(reserve_screens(&s, "HDMI-A-1"), ["DP-1", "eDP-1"]);
    }

    // peekCopy: the copy on the focused screen, else the first
    #[test]
    fn peek_copy_the_copy_on_the_focused_screen_else_the_first() {
        let two = strs(&["DP-1", "HDMI-A-1"]);
        assert_eq!(peek_copy(&two, "HDMI-A-1"), Some("HDMI-A-1"));
        assert_eq!(peek_copy(&two, ""), Some("DP-1"));
        assert_eq!(peek_copy(&strs(&[""]), "HDMI-A-1"), Some(""));
        assert_eq!(peek_copy(&[], "DP-1"), None);
    }

    // reserveZones: the deepest reserve per screen edge, never the sum
    #[test]
    fn reserve_zones_the_deepest_reserve_per_screen_edge_never_the_sum() {
        let r = |s: &[&str], edge: &str, space| {
            Some(Reserve {
                screens: strs(s),
                edge: edge.into(),
                space,
            })
        };
        let zones = reserve_zones(&[
            r(&["DP-1", "HDMI-A-1"], "top", 10.0),
            r(&["DP-1"], "top", 24.0),
            r(&["DP-1"], "left", 8.0),
            r(&["HDMI-A-1"], "bottom", 0.0),
            None,
        ]);
        let want: BTreeMap<String, f64> = [
            ("DP-1|top", 24.0),
            ("HDMI-A-1|top", 10.0),
            ("DP-1|left", 8.0),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into();
        assert_eq!(zones, want);
    }

    // notchScreen: open, a notch with a home screen stays where it hangs
    #[test]
    fn notch_screen_open_a_notch_with_a_home_screen_stays_where_it_hangs() {
        let s = screens(&[("DP-1", true), ("eDP-1", true)]);
        let open = |s: &[ScreenState], focused: &str, home: &str| ScreenContext {
            open: true,
            home: home.into(),
            ..ctx(s, focused)
        };
        assert_eq!(notch_screen(&open(&s, "eDP-1", "DP-1")), "DP-1");
        // Its home's workspace leaves it out: it opens where it hangs instead.
        let busy = screens(&[("DP-1", false), ("eDP-1", true), ("HDMI-A-1", true)]);
        assert_eq!(notch_screen(&open(&busy, "HDMI-A-1", "DP-1")), "HDMI-A-1");
        // Hanging nowhere, it opens on its home.
        let none = screens(&[("DP-1", false), ("eDP-1", false)]);
        assert_eq!(notch_screen(&open(&none, "eDP-1", "DP-1")), "DP-1");
        // Opened by the pointer on another screen, it stays there.
        let over = ScreenContext {
            override_screen: "eDP-1".into(),
            ..open(&s, "DP-1", "DP-1")
        };
        assert_eq!(notch_screen(&over), "eDP-1");
    }

    // needsVisitor: only when no notch with the module peeks on the focused screen
    #[test]
    fn needs_visitor_only_when_no_notch_with_the_module_peeks_on_the_focused_screen() {
        let dp = homed("DP-1");
        assert!(needs_visitor(std::slice::from_ref(&dp), "eDP-1"));
        assert!(needs_visitor(&[dp.clone(), homed("HDMI-A-1")], "eDP-1"));
        assert!(!needs_visitor(std::slice::from_ref(&dp), "DP-1"));
        assert!(!needs_visitor(&[dp.clone(), homed("eDP-1")], "eDP-1"));
        assert!(!needs_visitor(&[dp, kind(PeekKind::Follow)], "eDP-1"));
        assert!(!needs_visitor(&[kind(PeekKind::All)], "eDP-1"));
        assert!(needs_visitor(&[], "eDP-1"));
        assert!(!needs_visitor(&[homed("DP-1")], ""));
    }

    // focusedPeek: the first notch peeking on the focused screen, else the visitor
    #[test]
    fn focused_peek_the_first_notch_peeking_on_the_focused_screen_else_the_visitor() {
        let dp = homed("DP-1");
        let edp = homed("eDP-1");
        assert_eq!(
            focused_peek(&[dp.clone(), edp.clone(), homed("eDP-1")], "eDP-1"),
            Some(1)
        );
        assert_eq!(
            focused_peek(&[dp.clone(), kind(PeekKind::All)], "eDP-1"),
            Some(1)
        );
        assert_eq!(
            focused_peek(&[dp.clone(), kind(PeekKind::Follow), edp.clone()], "eDP-1"),
            Some(1)
        );
        assert_eq!(focused_peek(std::slice::from_ref(&dp), "eDP-1"), None);
        assert_eq!(focused_peek(std::slice::from_ref(&dp), "DP-1"), Some(0));
        assert_eq!(focused_peek(&[dp, edp], ""), Some(0));
        assert_eq!(focused_peek(&[], ""), None);
    }
}
