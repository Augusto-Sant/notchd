//! Port of the config half of `core/Position.js` (tests: the matching ones of
//! `tests/position.test.mjs`).
//!
//! The config's `position` is `{ edge, offset, screen }`: edge "top" |
//! "bottom" | "left" | "right", offset 0..1 along the edge (left to right, top
//! to bottom), 0.5 the middle, and screen the monitor the closed notch stays
//! on ("" or missing: the focused one; [`ALL_SCREENS`]: a copy of the notch on
//! every monitor). A missing key is top-center, following focus.
//!
//! Only what the config rules need lives here: [`resolve`], [`config_value`]
//! and what they use. The pixel geometry (`place`, `dragTarget`, `pack`, ...)
//! stays UI-side.

use serde::Serialize;
use serde_json::{Map, Number, Value};

use crate::js;

/// The edges a notch can sit on.
pub const EDGES: [&str; 4] = ["top", "bottom", "left", "right"];
/// The edge of a missing or unknown one (`DEFAULT_EDGE`).
pub const DEFAULT_EDGE: &str = "top";
/// The offset of a missing or unusable one (`DEFAULT_OFFSET`).
pub const DEFAULT_OFFSET: f64 = 0.5;
/// A `screen` for a copy of the notch on every monitor (`ALL_SCREENS`).
pub const ALL_SCREENS: &str = "all";

/// A position as the rules see it (`resolve`'s result).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Position {
    /// One of [`EDGES`].
    pub edge: String,
    /// 0 to 1 along the edge.
    pub offset: f64,
    /// A screen name; "" follows focus, [`ALL_SCREENS`] is every screen.
    pub screen: String,
}

/// Whether `edge` is one of [`EDGES`] (`isEdge`).
pub fn is_edge(edge: &str) -> bool {
    EDGES.contains(&edge)
}

/// Left and right: the notch runs down the edge instead of across it
/// (`isVertical`).
pub fn is_vertical(edge: &str) -> bool {
    edge == "left" || edge == "right"
}

/// The config value as a [`Position`]; anything unknown falls back to
/// top-center on the focused screen (`resolve`). `offset` is read as JS
/// `Number(v.offset)` (so `"0.25"` counts); missing, `null` and non-finite
/// ones are the default, others clamp to 0..1.
pub fn resolve(value: &Value) -> Position {
    let v = value.as_object();
    let get = |key: &str| v.and_then(|o| o.get(key));
    let edge = get("edge")
        .and_then(Value::as_str)
        .filter(|e| is_edge(e))
        .unwrap_or(DEFAULT_EDGE);
    let offset = match get("offset") {
        None | Some(Value::Null) => DEFAULT_OFFSET,
        Some(o) => {
            let n = js::number(o);
            if n.is_finite() {
                n.clamp(0.0, 1.0)
            } else {
                DEFAULT_OFFSET
            }
        }
    };
    Position {
        edge: edge.to_owned(),
        offset,
        screen: get("screen")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    }
}

/// A position as the config stores it: what it would get anyway left out, and
/// `None` (no key) when that is all of it (`configValue`). `inherited` is the
/// position it gets without a key of its own: a notch in `notches` takes the
/// top-level `position` key by key, so a key equal to the default but not to
/// that one must be written, or the notch goes back there. Without
/// `inherited`, the defaults: top-center on the focused screen.
pub fn config_value(position: &Value, inherited: Option<&Value>) -> Option<Value> {
    let p = resolve(position);
    let base = resolve(inherited.unwrap_or(&Value::Null));
    let mut out = Map::new();
    if p.edge != base.edge {
        out.insert("edge".into(), Value::String(p.edge));
    }
    if p.offset != base.offset {
        out.insert("offset".into(), number_value(p.offset));
    }
    if p.screen != base.screen {
        out.insert("screen".into(), Value::String(p.screen));
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

/// A whole number as an integer, like JSON.parse of what JS writes.
fn number_value(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        Value::from(f as i64)
    } else {
        Number::from_f64(f).map_or(Value::Null, Value::Number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pos(edge: &str, offset: f64, screen: &str) -> Position {
        Position {
            edge: edge.into(),
            offset,
            screen: screen.into(),
        }
    }

    // Node: resolve: a missing or broken value is top-center
    #[test]
    fn resolve_a_missing_or_broken_value_is_top_center() {
        let center = pos("top", 0.5, "");
        assert_eq!(resolve(&Value::Null), center); // undefined
        assert_eq!(resolve(&json!("left")), center);
        assert_eq!(resolve(&json!({ "edge": "middle", "offset": "x" })), center);
        assert_eq!(resolve(&json!({ "offset": null })), center);
    }

    // Node: resolve keeps a known edge and clamps the offset to 0..1
    #[test]
    fn resolve_keeps_a_known_edge_and_clamps_the_offset_to_0_1() {
        assert_eq!(resolve(&json!({ "edge": "left" })), pos("left", 0.5, ""));
        assert_eq!(
            resolve(&json!({ "edge": "bottom", "offset": 0.2 })),
            pos("bottom", 0.2, "")
        );
        assert_eq!(
            resolve(&json!({ "edge": "right", "offset": 3 })),
            pos("right", 1.0, "")
        );
        assert_eq!(
            resolve(&json!({ "edge": "top", "offset": -1 })),
            pos("top", 0.0, "")
        );
        assert_eq!(resolve(&json!({ "offset": "0.25" })), pos("top", 0.25, ""));
    }

    // Node: isVertical: only the side edges
    #[test]
    fn is_vertical_only_the_side_edges() {
        assert!(is_vertical("left"));
        assert!(is_vertical("right"));
        assert!(!is_vertical("top"));
        assert!(!is_vertical("bottom"));
    }

    // Node: resolve keeps the home screen by name
    #[test]
    fn resolve_keeps_the_home_screen_by_name() {
        assert_eq!(
            resolve(&json!({ "edge": "left", "screen": "eDP-1" })),
            pos("left", 0.5, "eDP-1")
        );
        assert_eq!(resolve(&json!({ "screen": 3 })).screen, "");
        assert_eq!(resolve(&json!({ "screen": ALL_SCREENS })).screen, "all");
    }

    // Node: configValue leaves defaults out; top-center is no key at all
    #[test]
    fn config_value_leaves_defaults_out_top_center_is_no_key_at_all() {
        assert_eq!(
            config_value(&json!({ "edge": "top", "offset": 0.5 }), None),
            None
        );
        assert_eq!(config_value(&Value::Null, None), None);
        assert_eq!(
            config_value(&json!({ "edge": "left", "offset": 0.5 }), None),
            Some(json!({ "edge": "left" }))
        );
        assert_eq!(
            config_value(&json!({ "edge": "top", "offset": 0.2 }), None),
            Some(json!({ "offset": 0.2 }))
        );
        assert_eq!(
            config_value(&json!({ "edge": "right", "offset": 0.75 }), None),
            Some(json!({ "edge": "right", "offset": 0.75 }))
        );
    }

    // Node: configValue writes what differs from the inherited position, defaults included
    // (bug: a notch dropped on the top edge was saved without `edge`, took the
    // top-level position's "left" key by key, and jumped back to the left side)
    #[test]
    fn config_value_writes_what_differs_from_the_inherited_position_defaults_included() {
        let top = json!({ "edge": "left", "offset": 0.957, "screen": "DP-1" });
        assert_eq!(
            config_value(
                &json!({ "edge": "top", "offset": 0.508, "screen": "DP-1" }),
                Some(&top)
            ),
            Some(json!({ "edge": "top", "offset": 0.508 }))
        );
        assert_eq!(
            config_value(
                &json!({ "edge": "top", "offset": 0.5, "screen": "" }),
                Some(&top)
            ),
            Some(json!({ "edge": "top", "offset": 0.5, "screen": "" }))
        );
        assert_eq!(
            config_value(
                &json!({ "edge": "left", "offset": 0.957, "screen": "DP-1" }),
                Some(&top)
            ),
            None
        );
        assert_eq!(
            config_value(
                &json!({ "edge": "right", "offset": 0.957, "screen": "DP-1" }),
                Some(&top)
            ),
            Some(json!({ "edge": "right" }))
        );
    }

    // Node: configValue keeps the home screen, even at top-center
    #[test]
    fn config_value_keeps_the_home_screen_even_at_top_center() {
        assert_eq!(
            config_value(
                &json!({ "edge": "top", "offset": 0.5, "screen": "eDP-1" }),
                None
            ),
            Some(json!({ "screen": "eDP-1" }))
        );
        assert_eq!(
            config_value(&json!({ "edge": "left", "screen": "DP-1" }), None),
            Some(json!({ "edge": "left", "screen": "DP-1" }))
        );
    }

    // (added) a whole offset is written as an integer, like JS
    #[test]
    fn config_value_writes_a_whole_offset_as_an_integer() {
        assert_eq!(
            config_value(&json!({ "offset": 1 }), None),
            Some(json!({ "offset": 1 }))
        );
        assert_eq!(
            config_value(&json!({ "offset": 0 }), None),
            Some(json!({ "offset": 0 }))
        );
    }
}
