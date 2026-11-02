//! `core/Usage.js` against `notchd_core::usage`. A `null` list is `None`; the
//! typed arguments (`f64`, `&str`) get what JS's `Number(x)` and `String(x)`
//! would make of the case's value.
use super::*;
use notchd_core::usage as u;
use std::collections::BTreeMap;

fn limits(v: &Value) -> Vec<u::Limit> {
    let list: Option<Vec<u::Limit>> = from_json(v);
    list.unwrap_or_default()
}

/// JS `Number(x)` for the scalars the cases use (null is 0).
fn number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::String(t) => t.trim().parse().unwrap_or(f64::NAN),
        _ => v.as_f64().unwrap_or(f64::NAN),
    }
}

fn warned(v: &Value) -> Option<BTreeMap<String, f64>> {
    v.as_object()
        .map(|o| o.iter().map(|(k, x)| (k.clone(), number(x))).collect())
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    let d = arg(a, 0);
    Some(match f {
        "windowLabel" => json!(u::window_label(num(a, 0))),
        "claudeLimits" => or_null(u::claude_limits(d).map(|l| to_json(&l))),
        "codexLimits" => or_null(u::codex_limits(d, number(arg(a, 1))).map(|l| to_json(&l))),
        "limitsOf" => {
            or_null(u::limits_of(&text(a, 0), arg(a, 1), number(arg(a, 2))).map(|l| to_json(&l)))
        }
        "request" => match u::request(
            &text(a, 0),
            arg(a, 1).as_str().unwrap_or(""),
            number(arg(a, 2)),
        ) {
            Ok(r) => to_json(&r),
            Err(e) => to_json(&e),
        },
        "afterStatus" => {
            let retry = match arg(a, 1) {
                Value::String(t) => t.clone(),
                _ => String::new(),
            };
            to_json(&u::after_status(int(a, 0), &retry, number(arg(a, 2))))
        }
        "level" => {
            let limit: u::Limit = from_json(d);
            n(u::level(&limit, number(arg(a, 1))))
        }
        "alerts" => to_json(&u::alerts(
            &limits(d),
            warned(arg(a, 1)).as_ref(),
            number(arg(a, 2)),
        )),
        "resetIn" => json!(u::reset_in(number(d), number(arg(a, 1)))),
        "tightest" => {
            let l = limits(d);
            or_null(u::tightest(&l).map(to_json))
        }
        _ => return None,
    })
}
