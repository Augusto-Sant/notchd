//! The config half of `core/Position.js` against `notchd_core::position`.
use super::*;
use notchd_core::position as p;

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    Some(match f {
        "isEdge" => json!(p::is_edge(&text(a, 0))),
        "isVertical" => json!(p::is_vertical(&text(a, 0))),
        "resolve" => {
            let r = p::resolve(arg(a, 0));
            json!({ "edge": r.edge, "offset": n(r.offset), "screen": r.screen })
        }
        "configValue" => or_undef(p::config_value(arg(a, 0), opt(a, 1))),
        _ => return None,
    })
}
