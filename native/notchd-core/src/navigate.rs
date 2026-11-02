//! Port of `core/Navigate.js`: navigation mode. A shortcut highlights every
//! notch, numbered in config order, and the keyboard picks one; the selected
//! notch opens as a preview, Tab walks its tabs, Enter pins it, Escape leaves.
//! Keys also act on the selected notch: settings, removing it (asked twice)
//! and moving it (arrows for the edge, Shift to slide, Tab for the monitor).

use crate::js;
use serde::{Deserialize, Serialize};

/// Qt key codes (`Qt.Key_*`), so the rules don't need the QML runtime.
mod key {
    pub const ESCAPE: u32 = 0x0100_0000;
    pub const TAB: u32 = 0x0100_0001;
    pub const BACKTAB: u32 = 0x0100_0002;
    pub const RETURN: u32 = 0x0100_0004;
    pub const ENTER: u32 = 0x0100_0005;
    pub const LEFT: u32 = 0x0100_0012;
    pub const UP: u32 = 0x0100_0013;
    pub const RIGHT: u32 = 0x0100_0014;
    pub const DOWN: u32 = 0x0100_0015;
    pub const DIGIT1: u32 = 0x31;
    pub const DIGIT9: u32 = 0x39;
    pub const H: u32 = 0x48;
    pub const J: u32 = 0x4a;
    pub const K: u32 = 0x4b;
    pub const L: u32 = 0x4c;
    pub const M: u32 = 0x4d;
    pub const S: u32 = 0x53;
    pub const X: u32 = 0x58;
}

/// `Qt.ShiftModifier`.
pub const SHIFT: u32 = 0x0200_0000;
/// Ctrl, Alt, Meta: a key held with one is a shortcut, not navigation; only
/// Escape leaves.
pub const CHORD: u32 = 0x0400_0000 | 0x0800_0000 | 0x1000_0000;

/// What a key press means in navigation (`keyName`'s strings). It serializes
/// as that string: `"1"`..`"9"`, `"previous"`, ..., and `""` for [`Other`].
///
/// [`Other`]: KeyName::Other
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum KeyName {
    /// A notch by its number, 1 to 9.
    Notch(u8),
    /// ← or h.
    Previous,
    /// → or l.
    Next,
    /// ↑ or k.
    Up,
    /// ↓ or j.
    Down,
    /// Shift with ← or ↑.
    SlideBack,
    /// Shift with → or ↓.
    SlideOn,
    PreviousTab,
    NextTab,
    Open,
    Leave,
    /// m.
    Move,
    /// s.
    Settings,
    /// x.
    Remove,
    /// A key navigation doesn't use (`""`), or any unknown name.
    Other,
}

impl From<KeyName> for String {
    fn from(k: KeyName) -> String {
        match k {
            KeyName::Notch(n) => return n.to_string(),
            KeyName::Previous => "previous",
            KeyName::Next => "next",
            KeyName::Up => "up",
            KeyName::Down => "down",
            KeyName::SlideBack => "slideBack",
            KeyName::SlideOn => "slideOn",
            KeyName::PreviousTab => "previousTab",
            KeyName::NextTab => "nextTab",
            KeyName::Open => "open",
            KeyName::Leave => "leave",
            KeyName::Move => "move",
            KeyName::Settings => "settings",
            KeyName::Remove => "remove",
            KeyName::Other => "",
        }
        .to_string()
    }
}

impl From<String> for KeyName {
    fn from(s: String) -> KeyName {
        match s.as_str() {
            "previous" => KeyName::Previous,
            "next" => KeyName::Next,
            "up" => KeyName::Up,
            "down" => KeyName::Down,
            "slideBack" => KeyName::SlideBack,
            "slideOn" => KeyName::SlideOn,
            "previousTab" => KeyName::PreviousTab,
            "nextTab" => KeyName::NextTab,
            "open" => KeyName::Open,
            "leave" => KeyName::Leave,
            "move" => KeyName::Move,
            "settings" => KeyName::Settings,
            "remove" => KeyName::Remove,
            d if d.len() == 1 && matches!(d.as_bytes()[0], b'1'..=b'9') => {
                KeyName::Notch(d.as_bytes()[0] - b'0')
            }
            _ => KeyName::Other,
        }
    }
}

/// `keyName`: what a key press (Qt key code and modifier flags) means.
pub fn key_name(key_code: u32, modifiers: u32) -> KeyName {
    if key_code == key::ESCAPE {
        return KeyName::Leave;
    }
    if modifiers & CHORD != 0 {
        return KeyName::Other;
    }
    let shift = modifiers & SHIFT != 0;
    if shift && (key_code == key::LEFT || key_code == key::UP) {
        return KeyName::SlideBack;
    }
    if shift && (key_code == key::RIGHT || key_code == key::DOWN) {
        return KeyName::SlideOn;
    }
    if key_code == key::BACKTAB || (key_code == key::TAB && shift) {
        return KeyName::PreviousTab;
    }
    if key_code == key::TAB {
        return KeyName::NextTab;
    }
    if shift {
        return KeyName::Other;
    }
    match key_code {
        key::DIGIT1..=key::DIGIT9 => KeyName::Notch((key_code - key::DIGIT1 + 1) as u8),
        key::LEFT | key::H => KeyName::Previous,
        key::RIGHT | key::L => KeyName::Next,
        key::UP | key::K => KeyName::Up,
        key::DOWN | key::J => KeyName::Down,
        key::RETURN | key::ENTER => KeyName::Open,
        key::M => KeyName::Move,
        key::S => KeyName::Settings,
        key::X => KeyName::Remove,
        _ => KeyName::Other,
    }
}

/// A notch as `start` sees it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotchInfo {
    /// Pinned or hovered.
    pub open: bool,
    /// The screens it shows on, by name.
    pub screens: Vec<String>,
}

/// `start`: the notch selected as navigation starts, by index in `notches`
/// (config order, nulls allowed): the first open one, else the first on the
/// focused screen, else the first; None (JS -1) for no notch.
pub fn start(notches: &[Option<NotchInfo>], focused: &str) -> Option<usize> {
    if notches.is_empty() {
        return None;
    }
    let open = notches
        .iter()
        .position(|n| n.as_ref().is_some_and(|n| n.open));
    let on_focused = || {
        if focused.is_empty() {
            return None;
        }
        notches.iter().position(|n| {
            n.as_ref()
                .is_some_and(|n| n.screens.iter().any(|s| s == focused))
        })
    };
    Some(open.or_else(on_focused).unwrap_or(0))
}

/// `wrap`: `i` wrapped into `0..count`.
pub fn wrap(i: i64, count: i64) -> i64 {
    i.rem_euclid(count)
}

/// The mode navigation is in (`step`'s `mode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NavMode {
    /// Picking a notch (the default).
    #[default]
    Select,
    /// Moving the selected notch.
    Move,
    /// Asking before removing it.
    Remove,
}

/// What a key does besides changing the state (`step`'s `action`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Keep navigating (`""`).
    #[default]
    #[serde(rename = "")]
    None,
    /// Pin the selected notch, end navigation.
    Open,
    /// End it, everything as it was.
    Leave,
    /// End navigation, open the selected notch's settings.
    Settings,
    /// Remove the selected notch (`selected` is the one after).
    Remove,
    /// Remember where the selected notch is.
    MoveStart,
    /// Move it as `move` says (`moveTarget`).
    Move,
    /// Keep where it went.
    MoveDone,
    /// Put it back.
    MoveCancel,
}

/// A screen edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// What a move does (`step`'s `move`, `moveTarget`'s `move`): at most one of
/// the three is set by `step`; `moveTarget` applies all that are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Move {
    /// Onto that edge, at the same offset.
    pub edge: Option<Edge>,
    /// -1 or 1: along the edge, by [`SLIDE`].
    pub slide: Option<i32>,
    /// -1 or 1: to the screen before or after.
    pub screen: Option<i32>,
}

/// `step`'s input (`s`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepState {
    pub mode: NavMode,
    /// The selected notch's index.
    pub selected: i64,
    /// How many notches there are.
    pub count: i64,
    /// The selected notch's tabs (module ids), empty for none.
    pub tabs: Vec<String>,
    /// The tab it shows, "" for its own.
    pub tab: String,
}

/// `step`'s result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepResult {
    /// The mode after the key.
    pub mode: NavMode,
    /// The selected notch; -1 when there are none.
    pub selected: i64,
    /// The tab to show, "" for the notch's own (a newly selected notch keeps
    /// the tab it had).
    pub tab: String,
    pub action: Action,
    /// Set with [`Action::Move`].
    #[serde(rename = "move")]
    pub movement: Option<Move>,
}

fn edge_for(key: KeyName) -> Option<Edge> {
    match key {
        KeyName::Previous => Some(Edge::Left),
        KeyName::Next => Some(Edge::Right),
        KeyName::Up => Some(Edge::Top),
        KeyName::Down => Some(Edge::Bottom),
        _ => None,
    }
}

/// `step`: one key (`key_name`) in navigation. Returns the mode after it, the
/// selection, the tab to show, and the action to take. The last notch is never
/// removed.
pub fn step(s: &StepState, key: KeyName) -> StepResult {
    let (count, selected) = (s.count, s.selected);
    let tab = s.tab.as_str();
    let out = |mode, selected, tab: &str, action, movement| StepResult {
        mode,
        selected,
        tab: tab.to_string(),
        action,
        movement,
    };
    let keep = |mode| out(mode, selected, tab, Action::None, None);
    let act = |mode, action| out(mode, selected, tab, action, None);
    if count == 0 {
        return out(NavMode::Select, -1, "", Action::Leave, None);
    }

    match s.mode {
        NavMode::Remove => {
            if key == KeyName::Remove {
                return out(
                    NavMode::Select,
                    selected.min(count - 2),
                    "",
                    Action::Remove,
                    None,
                );
            }
            return keep(NavMode::Select);
        }
        NavMode::Move => {
            let mv = |m| out(NavMode::Move, selected, tab, Action::Move, Some(m));
            if let Some(edge) = edge_for(key) {
                return mv(Move {
                    edge: Some(edge),
                    ..Move::default()
                });
            }
            return match key {
                KeyName::SlideBack | KeyName::SlideOn => mv(Move {
                    slide: Some(if key == KeyName::SlideOn { 1 } else { -1 }),
                    ..Move::default()
                }),
                KeyName::PreviousTab | KeyName::NextTab => mv(Move {
                    screen: Some(if key == KeyName::NextTab { 1 } else { -1 }),
                    ..Move::default()
                }),
                KeyName::Open => act(NavMode::Select, Action::MoveDone),
                KeyName::Leave => act(NavMode::Select, Action::MoveCancel),
                _ => keep(NavMode::Move),
            };
        }
        NavMode::Select => {}
    }

    match key {
        KeyName::Leave => act(NavMode::Select, Action::Leave),
        KeyName::Open => act(NavMode::Select, Action::Open),
        KeyName::Settings => act(NavMode::Select, Action::Settings),
        KeyName::Move => act(NavMode::Move, Action::MoveStart),
        KeyName::Remove => keep(if count > 1 {
            NavMode::Remove
        } else {
            NavMode::Select
        }),
        KeyName::Notch(n) => {
            let n = i64::from(n) - 1;
            if n >= count || n == selected {
                keep(NavMode::Select)
            } else {
                out(NavMode::Select, n, "", Action::None, None)
            }
        }
        KeyName::Previous | KeyName::Next => {
            if count < 2 {
                return keep(NavMode::Select);
            }
            let by = if key == KeyName::Next { 1 } else { -1 };
            out(
                NavMode::Select,
                wrap(selected + by, count),
                "",
                Action::None,
                None,
            )
        }
        KeyName::PreviousTab | KeyName::NextTab => {
            if s.tabs.len() < 2 {
                return keep(NavMode::Select);
            }
            let at = s.tabs.iter().position(|t| t == tab).unwrap_or(0) as i64;
            let by = if key == KeyName::NextTab { 1 } else { -1 };
            let next = &s.tabs[wrap(at + by, s.tabs.len() as i64) as usize];
            out(NavMode::Select, selected, next, Action::None, None)
        }
        _ => keep(NavMode::Select),
    }
}

/// How far a slide moves a notch along its edge.
pub const SLIDE: f64 = 0.05;

/// Where a notch hangs (`moveTarget`'s `position`, resolved).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub edge: Edge,
    /// 0 to 1 along the edge.
    pub offset: f64,
    /// A screen name; "" follows focus, "all" is every screen.
    pub screen: String,
}

/// `moveTarget`: where a move puts a notch at `position`: onto an edge at the
/// same offset; along it by [`SLIDE`], kept within 0 to 1; to the screen before
/// or after in `screens`, counting from the one it names, else the one it is on
/// (`on`). With under two screens, a screen move does nothing.
pub fn move_target(
    position: &Position,
    movement: Option<&Move>,
    screens: &[String],
    on: &str,
) -> Position {
    let mut p = position.clone();
    let Some(mv) = movement else { return p };
    if let Some(edge) = mv.edge {
        p.edge = edge;
    }
    if let Some(slide) = mv.slide.filter(|s| *s != 0) {
        p.offset =
            (js::round((p.offset + f64::from(slide) * SLIDE) * 100.0) / 100.0).clamp(0.0, 1.0);
    }
    if let Some(by) = mv.screen.filter(|s| *s != 0) {
        if screens.len() < 2 {
            return p;
        }
        let find = |name: &str| screens.iter().position(|s| s == name);
        p.screen = match find(&p.screen).or_else(|| find(on)) {
            Some(at) => {
                screens[wrap(at as i64 + i64::from(by), screens.len() as i64) as usize].clone()
            }
            None => screens[0].clone(),
        };
    }
    p
}

/// `hint`: the keys the selected notch's label lists, one string per line, for
/// the `mode`, `count` notches, whether it has `tabs` to walk and how many
/// `screens` there are.
pub fn hint(mode: NavMode, count: i64, tabs: bool, screens: i64) -> Vec<String> {
    let join = |parts: Vec<String>| {
        parts
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("   ")
    };
    let s = |x: &str| x.to_string();
    match mode {
        NavMode::Remove => vec![s("x remove   Esc keep")],
        NavMode::Move => vec![
            join(vec![
                s("←↑↓→ edge"),
                s("⇧ slide"),
                if screens > 1 { s("Tab screen") } else { s("") },
            ]),
            s("↵ done   Esc undo"),
        ],
        NavMode::Select => vec![
            join(vec![
                if count > 1 {
                    format!("1-{} ←→", count.min(9))
                } else {
                    s("")
                },
                if tabs { s("Tab") } else { s("") },
                s("↵ open"),
                s("Esc"),
            ]),
            join(vec![
                s("m move"),
                s("s settings"),
                if count > 1 { s("x remove") } else { s("") },
            ]),
        ],
    }
}

/// `number`: the number a notch shows in navigation: its place in config
/// order, 1 to 9; 0 (no number, reached with the arrows) past that or outside it.
pub fn number(index: i64) -> u32 {
    if (0..9).contains(&index) {
        index as u32 + 1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESCAPE: u32 = key::ESCAPE;
    const TAB: u32 = key::TAB;
    const BACKTAB: u32 = key::BACKTAB;
    const CTRL: u32 = 0x0400_0000;
    const ALT: u32 = 0x0800_0000;
    const META: u32 = 0x1000_0000;
    const DIGIT0: u32 = 0x30;
    const N: u32 = 0x4e;
    const Q: u32 = 0x51;

    // keyName: digits, arrows and hjkl, tabs, Enter and Escape
    #[test]
    fn key_name_digits_arrows_and_hjkl_tabs_enter_and_escape() {
        assert_eq!(key_name(key::DIGIT1, 0), KeyName::Notch(1));
        assert_eq!(key_name(key::DIGIT9, 0), KeyName::Notch(9));
        assert_eq!(key_name(DIGIT0, 0), KeyName::Other);
        assert_eq!(key_name(key::LEFT, 0), KeyName::Previous);
        assert_eq!(key_name(key::H, 0), KeyName::Previous);
        assert_eq!(key_name(key::RIGHT, 0), KeyName::Next);
        assert_eq!(key_name(key::L, 0), KeyName::Next);
        assert_eq!(key_name(key::UP, 0), KeyName::Up);
        assert_eq!(key_name(key::K, 0), KeyName::Up);
        assert_eq!(key_name(key::DOWN, 0), KeyName::Down);
        assert_eq!(key_name(key::J, 0), KeyName::Down);
        assert_eq!(key_name(TAB, 0), KeyName::NextTab);
        assert_eq!(key_name(BACKTAB, SHIFT), KeyName::PreviousTab);
        assert_eq!(key_name(TAB, SHIFT), KeyName::PreviousTab);
        assert_eq!(key_name(key::RETURN, 0), KeyName::Open);
        assert_eq!(key_name(key::ENTER, 0), KeyName::Open);
        assert_eq!(key_name(ESCAPE, 0), KeyName::Leave);
        assert_eq!(key_name(Q, 0), KeyName::Other);
    }

    // keyName: m, s and x act on the selected notch; n does nothing
    #[test]
    fn key_name_m_s_and_x_act_on_the_selected_notch_n_does_nothing() {
        assert_eq!(key_name(N, 0), KeyName::Other);
        assert_eq!(key_name(key::M, 0), KeyName::Move);
        assert_eq!(key_name(key::S, 0), KeyName::Settings);
        assert_eq!(key_name(key::X, 0), KeyName::Remove);
    }

    // keyName: Shift with an arrow slides; Shift with anything else is nothing
    #[test]
    fn key_name_shift_with_an_arrow_slides_shift_with_anything_else_is_nothing() {
        assert_eq!(key_name(key::LEFT, SHIFT), KeyName::SlideBack);
        assert_eq!(key_name(key::UP, SHIFT), KeyName::SlideBack);
        assert_eq!(key_name(key::RIGHT, SHIFT), KeyName::SlideOn);
        assert_eq!(key_name(key::DOWN, SHIFT), KeyName::SlideOn);
        assert_eq!(key_name(key::X, SHIFT), KeyName::Other);
        assert_eq!(key_name(key::H, SHIFT), KeyName::Other);
    }

    // keyName: a key held with Ctrl, Alt or Super is a shortcut, not navigation
    #[test]
    fn key_name_a_key_held_with_ctrl_alt_or_super_is_a_shortcut_not_navigation() {
        assert_eq!(key_name(key::DIGIT1, CTRL), KeyName::Other);
        assert_eq!(key_name(key::RIGHT, ALT), KeyName::Other);
        assert_eq!(key_name(key::X, META), KeyName::Other);
    }

    // keyName: Escape leaves with any modifier, Ctrl+Escape included
    #[test]
    fn key_name_escape_leaves_with_any_modifier_ctrl_escape_included() {
        assert_eq!(key_name(ESCAPE, CTRL), KeyName::Leave);
        assert_eq!(key_name(ESCAPE, META), KeyName::Leave);
    }

    // start: the open notch, else the first on the focused screen, else the first
    #[test]
    fn start_the_open_notch_else_the_first_on_the_focused_screen_else_the_first() {
        let n = |open, screen: &str| {
            Some(NotchInfo {
                open,
                screens: vec![screen.into()],
            })
        };
        let mut notches = vec![n(false, "A"), n(false, "B"), n(true, "B")];
        assert_eq!(start(&notches, "B"), Some(2));
        notches[2] = n(false, "B");
        assert_eq!(start(&notches, "B"), Some(1));
        assert_eq!(start(&notches, "C"), Some(0));
        assert_eq!(start(&notches, ""), Some(0));
        assert_eq!(start(&[], "A"), None);
    }

    fn at(selected: i64) -> StepState {
        StepState {
            selected,
            count: 3,
            ..Default::default()
        }
    }

    fn with_tabs(selected: i64, tabs: &[&str], tab: &str) -> StepState {
        StepState {
            tabs: tabs.iter().map(|t| (*t).into()).collect(),
            tab: tab.into(),
            ..at(selected)
        }
    }

    fn pick(r: &StepResult) -> (NavMode, i64, &str, Action) {
        (r.mode, r.selected, r.tab.as_str(), r.action)
    }

    // step: a digit selects that notch; one past the last does nothing
    #[test]
    fn step_a_digit_selects_that_notch_one_past_the_last_does_nothing() {
        assert_eq!(
            pick(&step(&at(0), KeyName::Notch(3))),
            (NavMode::Select, 2, "", Action::None)
        );
        assert_eq!(
            pick(&step(&at(0), KeyName::Notch(4))),
            (NavMode::Select, 0, "", Action::None)
        );
    }

    // step: a digit for the selected notch keeps its tab
    #[test]
    fn step_a_digit_for_the_selected_notch_keeps_its_tab() {
        assert_eq!(
            step(&with_tabs(1, &["a", "b"], "b"), KeyName::Notch(2)).tab,
            "b"
        );
    }

    // step: previous and next wrap around
    #[test]
    fn step_previous_and_next_wrap_around() {
        assert_eq!(step(&at(2), KeyName::Next).selected, 0);
        assert_eq!(step(&at(0), KeyName::Previous).selected, 2);
        assert_eq!(step(&at(1), KeyName::Next).selected, 2);
        assert_eq!(
            step(
                &StepState {
                    selected: 0,
                    count: 1,
                    ..Default::default()
                },
                KeyName::Next
            )
            .selected,
            0
        );
    }

    // step: moving to another notch leaves its tab to it
    #[test]
    fn step_moving_to_another_notch_leaves_its_tab_to_it() {
        assert_eq!(step(&with_tabs(0, &["a", "b"], "b"), KeyName::Next).tab, "");
    }

    // step: up and down do nothing outside the move mode
    #[test]
    fn step_up_and_down_do_nothing_outside_the_move_mode() {
        assert_eq!(
            pick(&step(&at(1), KeyName::Up)),
            (NavMode::Select, 1, "", Action::None)
        );
    }

    // step: Tab walks the selected notch's tabs, both ways, wrapping
    #[test]
    fn step_tab_walks_the_selected_notchs_tabs_both_ways_wrapping() {
        let s = with_tabs(0, &["codex", "claude", "timer"], "codex");
        assert_eq!(step(&s, KeyName::NextTab).tab, "claude");
        assert_eq!(step(&s, KeyName::PreviousTab).tab, "timer");
        assert_eq!(
            step(
                &StepState {
                    tab: "timer".into(),
                    ..s.clone()
                },
                KeyName::NextTab
            )
            .tab,
            "codex"
        );
        // A tab it doesn't know counts as the first.
        assert_eq!(
            step(
                &StepState {
                    tab: "gone".into(),
                    ..s
                },
                KeyName::NextTab
            )
            .tab,
            "claude"
        );
        // One module or none: nothing to walk.
        assert_eq!(
            step(&with_tabs(0, &["codex"], "codex"), KeyName::NextTab).tab,
            "codex"
        );
        assert_eq!(step(&at(0), KeyName::NextTab).tab, "");
    }

    // step: Enter opens, Escape leaves, an unknown key does nothing
    #[test]
    fn step_enter_opens_escape_leaves_an_unknown_key_does_nothing() {
        assert_eq!(step(&at(1), KeyName::Open).action, Action::Open);
        assert_eq!(step(&at(1), KeyName::Leave).action, Action::Leave);
        assert_eq!(
            pick(&step(&at(1), KeyName::Other)),
            (NavMode::Select, 1, "", Action::None)
        );
        assert_eq!(
            step(
                &StepState {
                    selected: 0,
                    count: 0,
                    ..Default::default()
                },
                KeyName::Next
            )
            .action,
            Action::Leave
        );
    }

    // step: s opens the selected notch's settings
    #[test]
    fn step_s_opens_the_selected_notchs_settings() {
        assert_eq!(step(&at(2), KeyName::Settings).action, Action::Settings);
    }

    // step: x asks first; x again removes it, anything else keeps it
    #[test]
    fn step_x_asks_first_x_again_removes_it_anything_else_keeps_it() {
        let removing = |selected| StepState {
            mode: NavMode::Remove,
            ..at(selected)
        };
        let asked = step(&at(1), KeyName::Remove);
        assert_eq!(pick(&asked), (NavMode::Remove, 1, "", Action::None));
        assert_eq!(
            pick(&step(&removing(1), KeyName::Remove)),
            (NavMode::Select, 1, "", Action::Remove)
        );
        // Removing the last one selects the one before.
        assert_eq!(step(&removing(2), KeyName::Remove).selected, 1);
        // Escape (or any other key) keeps it, and stays in navigation.
        assert_eq!(
            pick(&step(&removing(1), KeyName::Leave)),
            (NavMode::Select, 1, "", Action::None)
        );
        assert_eq!(step(&removing(1), KeyName::Next).action, Action::None);
    }

    // step: the last notch is never removed
    #[test]
    fn step_the_last_notch_is_never_removed() {
        let one = StepState {
            selected: 0,
            count: 1,
            ..Default::default()
        };
        assert_eq!(step(&one, KeyName::Remove).mode, NavMode::Select);
    }

    // step: m moves the selected notch: arrows pick the edge, Shift slides, Tab the screen
    #[test]
    fn step_m_moves_the_selected_notch_arrows_pick_the_edge_shift_slides_tab_the_screen() {
        assert_eq!(
            pick(&step(&at(1), KeyName::Move)),
            (NavMode::Move, 1, "", Action::MoveStart)
        );
        let moving = StepState {
            mode: NavMode::Move,
            ..at(1)
        };
        let mv = |key| step(&moving, key).movement;
        let edge = |e| {
            Some(Move {
                edge: Some(e),
                ..Move::default()
            })
        };
        assert_eq!(mv(KeyName::Up), edge(Edge::Top));
        assert_eq!(mv(KeyName::Down), edge(Edge::Bottom));
        assert_eq!(mv(KeyName::Previous), edge(Edge::Left));
        assert_eq!(mv(KeyName::Next), edge(Edge::Right));
        assert_eq!(
            mv(KeyName::SlideOn),
            Some(Move {
                slide: Some(1),
                ..Move::default()
            })
        );
        assert_eq!(
            mv(KeyName::SlideBack),
            Some(Move {
                slide: Some(-1),
                ..Move::default()
            })
        );
        assert_eq!(
            mv(KeyName::NextTab),
            Some(Move {
                screen: Some(1),
                ..Move::default()
            })
        );
        assert_eq!(
            mv(KeyName::PreviousTab),
            Some(Move {
                screen: Some(-1),
                ..Move::default()
            })
        );
        assert_eq!(step(&moving, KeyName::Up).mode, NavMode::Move);
        // Digits don't select another notch while moving.
        assert_eq!(
            pick(&step(&moving, KeyName::Notch(3))),
            (NavMode::Move, 1, "", Action::None)
        );
    }

    // step: moving, Enter keeps where it went and Escape puts it back, both still navigating
    #[test]
    fn step_moving_enter_keeps_where_it_went_and_escape_puts_it_back_both_still_navigating() {
        let moving = StepState {
            mode: NavMode::Move,
            ..at(1)
        };
        assert_eq!(
            pick(&step(&moving, KeyName::Open)),
            (NavMode::Select, 1, "", Action::MoveDone)
        );
        assert_eq!(
            pick(&step(&moving, KeyName::Leave)),
            (NavMode::Select, 1, "", Action::MoveCancel)
        );
    }

    fn pos(edge: Edge, offset: f64, screen: &str) -> Position {
        Position {
            edge,
            offset,
            screen: screen.into(),
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).into()).collect()
    }

    // moveTarget: onto an edge at the same offset
    #[test]
    fn move_target_onto_an_edge_at_the_same_offset() {
        let p = pos(Edge::Top, 0.3, "A");
        let mv = Move {
            edge: Some(Edge::Left),
            ..Move::default()
        };
        assert_eq!(
            move_target(&p, Some(&mv), &names(&["A", "B"]), "A"),
            pos(Edge::Left, 0.3, "A")
        );
    }

    // moveTarget: a slide moves 5 % along the edge, within the edge
    #[test]
    fn move_target_a_slide_moves_5_percent_along_the_edge_within_the_edge() {
        let p = pos(Edge::Top, 0.5, "");
        let slide = |p: &Position, by| {
            move_target(
                p,
                Some(&Move {
                    slide: Some(by),
                    ..Move::default()
                }),
                &[],
                "",
            )
            .offset
        };
        assert_eq!(slide(&p, 1), 0.55);
        assert_eq!(slide(&p, -1), 0.45);
        assert_eq!(
            slide(
                &Position {
                    offset: 0.98,
                    ..p.clone()
                },
                1
            ),
            1.0
        );
        assert_eq!(slide(&Position { offset: 0.02, ..p }, -1), 0.0);
    }

    // moveTarget: Tab cycles the screens; from following focus or every screen, from where it is
    #[test]
    fn move_target_tab_cycles_the_screens_from_following_focus_or_every_screen_from_where_it_is() {
        let p = pos(Edge::Top, 0.5, "A");
        let screens = names(&["A", "B", "C"]);
        let go = |p: &Position, by, screens: &[String], on| {
            move_target(
                p,
                Some(&Move {
                    screen: Some(by),
                    ..Move::default()
                }),
                screens,
                on,
            )
            .screen
        };
        assert_eq!(go(&p, 1, &screens, "A"), "B");
        assert_eq!(go(&p, -1, &screens, "A"), "C");
        assert_eq!(
            go(
                &Position {
                    screen: "".into(),
                    ..p.clone()
                },
                1,
                &screens,
                "B"
            ),
            "C"
        );
        assert_eq!(
            go(
                &Position {
                    screen: "all".into(),
                    ..p.clone()
                },
                1,
                &screens,
                ""
            ),
            "A"
        );
        // One screen: nowhere to go.
        assert_eq!(
            go(
                &Position {
                    screen: "".into(),
                    ..p
                },
                1,
                &names(&["A"]),
                "A"
            ),
            ""
        );
    }

    // hint: the keys the label lists, by mode
    #[test]
    fn hint_the_keys_the_label_lists_by_mode() {
        assert_eq!(
            hint(NavMode::Select, 3, true, 2),
            [
                "1-3 ←→   Tab   ↵ open   Esc",
                "m move   s settings   x remove"
            ]
        );
        // One notch: no numbers, nothing to remove; no tabs.
        assert_eq!(
            hint(NavMode::Select, 1, false, 1),
            ["↵ open   Esc", "m move   s settings"]
        );
        assert_eq!(
            hint(NavMode::Move, 3, false, 2),
            ["←↑↓→ edge   ⇧ slide   Tab screen", "↵ done   Esc undo"]
        );
        assert_eq!(
            hint(NavMode::Move, 3, false, 1),
            ["←↑↓→ edge   ⇧ slide", "↵ done   Esc undo"]
        );
        assert_eq!(hint(NavMode::Remove, 3, false, 1), ["x remove   Esc keep"]);
    }

    // number: 1 to 9 in config order, none past that
    #[test]
    fn number_1_to_9_in_config_order_none_past_that() {
        assert_eq!(number(0), 1);
        assert_eq!(number(8), 9);
        assert_eq!(number(9), 0);
        assert_eq!(number(-1), 0);
    }
}
