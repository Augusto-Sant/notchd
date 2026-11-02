//! `core/Navigate.js` against `notchd_core::navigate`. A `-1` is a `None`,
//! a key name is its string, `""` for a key that is not used.
use super::*;
use notchd_core::navigate as nav;

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    Some(match f {
        "keyName" => {
            let modifiers = arg(a, 1).as_f64().unwrap_or(0.0) as u32;
            to_json(&nav::key_name(int(a, 0) as u32, modifiers))
        }
        "start" => {
            let notches: Option<Vec<Option<nav::NotchInfo>>> = from_json(arg(a, 0));
            json!(nav::start(&notches.unwrap_or_default(), &text(a, 1)).map_or(-1, |i| i as i64))
        }
        "wrap" => json!(nav::wrap(int(a, 0), int(a, 1))),
        "step" => {
            let state: nav::StepState = from_json(arg(a, 0));
            let key = nav::KeyName::from(text(a, 1));
            let mut out = to_json(&nav::step(&state, key));
            // A `Move` has only the keys it sets; Rust writes the others as null.
            if let Some(Value::Object(m)) = out.get_mut("move") {
                m.retain(|_, v| !v.is_null());
            }
            out
        }
        "moveTarget" => {
            let position: nav::Position = from_json(arg(a, 0));
            let movement: Option<nav::Move> = from_json(arg(a, 1));
            let screens: Option<Vec<String>> = from_json(arg(a, 2));
            to_json(&nav::move_target(
                &position,
                movement.as_ref(),
                &screens.unwrap_or_default(),
                &text(a, 3),
            ))
        }
        "hint" => {
            let mode: nav::NavMode = arg(a, 0)
                .as_str()
                .map_or(nav::NavMode::Select, |m| from_json(&json!(m)));
            json!(nav::hint(
                mode,
                int(a, 1),
                arg(a, 2).as_bool().unwrap_or(false),
                int(a, 3)
            ))
        }
        "number" => json!(nav::number(int(a, 0))),
        _ => return None,
    })
}
