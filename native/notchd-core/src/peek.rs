//! Peek arbitration and the peek timers' rules: what peeks, for how long, what
//! interrupts it. Ported from the decisions that live in QML:
//!
//! - `NotchCopy.qml` `requestPeek`, `dismissPeek`, `keepMissed`, `peekMissed`
//!   and `onHoveredChanged` (a copy's own peek)
//! - `NotchService.qml` `requestPeek` (which notches and the visitor a
//!   module's peek goes to)
//!
//! over `State.js` (`canPeek`, `attentionPeek`, `needsVisitor`,
//! `focusedPeek`), which [`crate::state`] already ports: this composes those,
//! it does not repeat them.

use crate::state::{
    self, ModuleInfo, PeekContext, PeekKind, PeekNotch, attention_peek, focused_peek, needs_visitor,
};

/// The shortest a peek stays (`Math.max(400, ...)`).
pub const MIN_PEEK_MS: u64 = 400;
/// A peek whose module gave no length (`Number(ms) || 2500`).
pub const DEFAULT_PEEK_MS: u64 = 2500;

/// How long a peek lasts: the module's `ms`, 2500 when it gave none (or 0 or
/// NaN, which JS's `||` treats the same), never under 400.
pub fn duration_ms(ms: Option<f64>) -> u64 {
    let n = ms
        .filter(|n| n.is_finite() && *n != 0.0)
        .unwrap_or(DEFAULT_PEEK_MS as f64);
    n.max(MIN_PEEK_MS as f64) as u64
}

/// What a copy knows when a module asks it to peek.
#[derive(Debug, Clone, Copy)]
pub struct CopyPeek<'a> {
    pub pinned: bool,
    pub hovered: bool,
    /// The module whose peek the copy has, if any.
    pub current: Option<&'a ModuleInfo>,
}

/// `NotchCopy.requestPeek`'s test: may `module` peek on this copy now? A peek
/// on screen (`current` set: the timer runs while it shows) is not cut short by
/// a lower-priority one.
pub fn may_peek(module: &ModuleInfo, copy: &CopyPeek) -> bool {
    state::can_peek(
        Some(module),
        &PeekContext {
            pinned: copy.pinned,
            hovered: copy.hovered,
            current: copy.current.cloned(),
            current_showing: copy.current.is_some(),
        },
    )
}

/// `onHoveredChanged`: a copy that becomes hovered drops a peek that is not
/// urgent.
pub fn hover_dismisses(hovered: bool, current: Option<&ModuleInfo>) -> bool {
    hovered && current.is_some_and(|m| !m.urgent_peek)
}

/// Where `NotchService.requestPeek` sends a module's peek: the notches (by
/// their index in the list given) and whether the visitor too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub notches: Vec<usize>,
    pub visitor: bool,
}

/// `NotchService.requestPeek` for a module of the notches (not a standalone
/// one: that goes to its own notch). `places` are the notches that show the
/// module, in order. A module that `on_focused_screen` only peeks once: in the
/// first notch there that shows it, else in the visitor; any other peeks in
/// every notch that shows it, and in the visitor when none peeks on the
/// focused screen.
pub fn route(places: &[PeekNotch], focused: &str, on_focused_screen: bool) -> Route {
    if on_focused_screen {
        return match focused_peek(places, focused) {
            Some(at) => Route {
                notches: vec![at],
                visitor: false,
            },
            None => Route {
                notches: Vec::new(),
                visitor: true,
            },
        };
    }
    Route {
        notches: (0..places.len()).collect(),
        visitor: needs_visitor(places, focused),
    }
}

/// `peekPlace`: how a notch hangs, for [`route`]. `homed_screen` is the screen
/// a homed notch is on now.
pub fn place(homed: bool, every_screen: bool, homed_screen: &str) -> PeekNotch {
    if !homed {
        return PeekNotch {
            kind: if every_screen {
                PeekKind::All
            } else {
                PeekKind::Follow
            },
            screen: None,
        };
    }
    PeekNotch {
        kind: PeekKind::Homed,
        screen: Some(homed_screen.to_string()),
    }
}

/// A peek a copy could not show, of a module with news not seen yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Missed {
    pub module: String,
    pub ms: Option<f64>,
}

/// `keepMissed`: remember a refused peek, but only of a module with
/// `attention`; a module has one entry, the latest.
pub fn keep_missed(list: &mut Vec<Missed>, module: &str, attention: bool, ms: Option<f64>) {
    if module.is_empty() || !attention {
        return;
    }
    list.retain(|e| e.module != module);
    list.push(Missed {
        module: module.to_string(),
        ms,
    });
}

/// `peekMissed`: as the copy closes, drops the entries whose module has seen
/// its news and takes the one that peeks now (`State.attentionPeek`: the first
/// with a peek view). `lookup` gives a module's current facts.
pub fn take_missed(
    list: &mut Vec<Missed>,
    lookup: &dyn Fn(&str) -> Option<ModuleInfo>,
) -> Option<Missed> {
    list.retain(|e| lookup(&e.module).is_some_and(|m| m.attention));
    let modules: Vec<Option<ModuleInfo>> = list.iter().map(|e| lookup(&e.module)).collect();
    let id = attention_peek(&modules)?.id.clone();
    let at = list.iter().position(|e| e.module == id)?;
    Some(list.remove(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(id: &str, priority: f64) -> ModuleInfo {
        ModuleInfo {
            id: id.into(),
            priority,
            active: true,
            peek: true,
            ..ModuleInfo::default()
        }
    }

    fn copy<'a>(current: Option<&'a ModuleInfo>) -> CopyPeek<'a> {
        CopyPeek {
            pinned: false,
            hovered: false,
            current,
        }
    }

    #[test]
    fn duration_defaults_and_floors() {
        assert_eq!(duration_ms(None), 2500);
        assert_eq!(duration_ms(Some(0.0)), 2500);
        assert_eq!(duration_ms(Some(f64::NAN)), 2500);
        assert_eq!(duration_ms(Some(100.0)), 400);
        assert_eq!(duration_ms(Some(-5.0)), 400);
        assert_eq!(duration_ms(Some(6000.0)), 6000);
    }

    #[test]
    fn a_peek_on_screen_is_not_cut_short_by_a_lower_priority_one() {
        let high = module("a", 80.0);
        let low = module("b", 10.0);
        assert!(!may_peek(&low, &copy(Some(&high))));
        assert!(may_peek(&high, &copy(Some(&low))), "higher priority wins");
        assert!(may_peek(&low, &copy(None)));
        assert!(
            may_peek(&high, &copy(Some(&high))),
            "the same module renews"
        );
    }

    #[test]
    fn pinned_never_peeks_and_hover_lets_only_urgent_through() {
        let m = module("a", 1.0);
        let urgent = ModuleInfo {
            urgent_peek: true,
            ..module("u", 1.0)
        };
        let pinned = CopyPeek {
            pinned: true,
            ..copy(None)
        };
        assert!(!may_peek(&urgent, &pinned));
        let hovered = CopyPeek {
            hovered: true,
            ..copy(None)
        };
        assert!(!may_peek(&m, &hovered));
        assert!(may_peek(&urgent, &hovered));
        let no_view = ModuleInfo {
            peek: false,
            ..module("n", 1.0)
        };
        assert!(!may_peek(&no_view, &copy(None)));
    }

    #[test]
    fn hover_drops_a_peek_that_is_not_urgent() {
        let m = module("a", 1.0);
        let urgent = ModuleInfo {
            urgent_peek: true,
            ..m.clone()
        };
        assert!(hover_dismisses(true, Some(&m)));
        assert!(!hover_dismisses(true, Some(&urgent)));
        assert!(!hover_dismisses(false, Some(&m)));
        assert!(!hover_dismisses(true, None));
    }

    fn at(kind: PeekKind, screen: &str) -> PeekNotch {
        PeekNotch {
            kind,
            screen: Some(screen.into()),
        }
    }

    #[test]
    fn a_peek_goes_to_every_notch_and_the_visitor_when_none_is_on_the_focused_screen() {
        let places = [at(PeekKind::Homed, "DP-2"), at(PeekKind::Homed, "DP-3")];
        let r = route(&places, "DP-1", false);
        assert_eq!(r.notches, vec![0, 1]);
        assert!(r.visitor);
        let r = route(&places, "DP-3", false);
        assert!(!r.visitor, "one is there");
        assert!(route(&[], "DP-1", false).visitor, "no notch shows it");
        assert!(!route(&[], "", false).visitor, "no focus known: no visitor");
    }

    #[test]
    fn a_focused_screen_peek_goes_once() {
        let places = [
            at(PeekKind::Homed, "DP-2"),
            PeekNotch {
                kind: PeekKind::Follow,
                screen: None,
            },
            PeekNotch {
                kind: PeekKind::All,
                screen: None,
            },
        ];
        let r = route(&places, "DP-1", true);
        assert_eq!(
            r,
            Route {
                notches: vec![1],
                visitor: false
            }
        );
        let only_away = [at(PeekKind::Homed, "DP-2")];
        let r = route(&only_away, "DP-1", true);
        assert_eq!(
            r,
            Route {
                notches: vec![],
                visitor: true
            }
        );
    }

    #[test]
    fn place_says_how_a_notch_hangs() {
        assert_eq!(place(false, true, "").kind, PeekKind::All);
        assert_eq!(place(false, false, "").kind, PeekKind::Follow);
        let p = place(true, false, "DP-2");
        assert_eq!(
            (p.kind, p.screen.as_deref()),
            (PeekKind::Homed, Some("DP-2"))
        );
    }

    #[test]
    fn missed_peeks_keep_the_latest_per_module_and_only_with_attention() {
        let mut list = Vec::new();
        keep_missed(&mut list, "a", false, Some(1.0));
        assert!(list.is_empty());
        keep_missed(&mut list, "a", true, Some(1000.0));
        keep_missed(&mut list, "b", true, None);
        keep_missed(&mut list, "a", true, Some(2000.0));
        let names: Vec<_> = list.iter().map(|e| e.module.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
        assert_eq!(list[1].ms, Some(2000.0));
    }

    #[test]
    fn take_missed_skips_the_seen_and_takes_the_first_with_a_peek() {
        let mut list = Vec::new();
        for id in ["seen", "a", "b"] {
            keep_missed(&mut list, id, true, None);
        }
        let lookup = |id: &str| {
            Some(ModuleInfo {
                attention: id != "seen",
                ..module(id, 1.0)
            })
        };
        let got = take_missed(&mut list, &lookup).unwrap();
        assert_eq!(got.module, "a");
        assert_eq!(list.len(), 1, "seen is dropped, a is taken, b waits");
        assert_eq!(list[0].module, "b");
        assert!(take_missed(&mut Vec::new(), &lookup).is_none());
    }
}
