//! `core/Terminal.js` against `notchd_core::terminal`. A missing path or
//! title is `None`.
use super::*;
use notchd_core::terminal as t;

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    Some(match f {
        "expandHome" => json!(t::expand_home(Some(&jstext(a, 0)), &jstext(a, 1))),
        "shellArgs" => {
            let extra = strs(arg(a, 1));
            json!(t::shell_args(
                &jstext(a, 0),
                &extra,
                arg(a, 2).as_bool().unwrap_or(false),
                arg(a, 3).as_bool().unwrap_or(false)
            ))
        }
        "isLeaveKey" => {
            let modifiers = arg(a, 1).as_f64().unwrap_or(0.0) as u32;
            json!(t::is_leave_key(int(a, 0) as u32, modifiers))
        }
        "plainTitle" => {
            let p = t::plain_title(Some(&jstext(a, 0)));
            json!({ "working": p.working, "title": p.title })
        }
        _ => return None,
    })
}
