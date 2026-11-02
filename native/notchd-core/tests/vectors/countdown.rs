//! `core/Countdown.js` against `notchd_core::countdown`. The JS "no state"
//! (`null`) is the Rust idle state; `add` drops the unused `now`;
//! `parseCommand`'s `{ error }` is an `Err`, `parseDuration`'s `null` a `None`.
use super::*;
use notchd_core::countdown as c;

fn state(v: &Value) -> c::State {
    if v.is_null() {
        c::State::idle()
    } else {
        from_json(v)
    }
}

/// A state as JSON. Rust's idle state carries `duration: 0`; the JS one is
/// `{ status: "idle" }`.
fn st(v: &c::State) -> Value {
    let mut j = to_json(v);
    if v.status == c::Status::Idle && v.duration == 0.0 {
        j.as_object_mut().unwrap().remove("duration");
    }
    j
}

/// JS `String(x || "")` for a label.
fn label(v: &Value) -> String {
    match v {
        Value::String(t) => t.clone(),
        Value::Number(x) if x.as_f64() != Some(0.0) => x.to_string(),
        Value::Bool(true) => "true".into(),
        _ => String::new(),
    }
}

fn settings(v: Option<&Value>) -> c::PomodoroSettings {
    match v {
        None | Some(Value::Null) => c::PomodoroSettings::default(),
        Some(v) => from_json(v),
    }
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    let s = arg(a, 0);
    Some(match f {
        "pomodoroConfig" => to_json(&c::pomodoro_config(&settings(Some(s)))),
        "parseDuration" => {
            let t = match s {
                Value::String(t) => t.clone(),
                Value::Number(x) => x.to_string(),
                _ => String::new(),
            };
            or_null(c::parse_duration(&t).map(n))
        }
        "parseCommand" => {
            let t = match s {
                Value::String(t) => t.clone(),
                Value::Number(x) => x.to_string(),
                _ => String::new(),
            };
            match c::parse_command(&t) {
                Ok(cmd) => to_json(&cmd),
                Err(e) => json!({ "error": e }),
            }
        }
        "format" => {
            let ms = match s {
                Value::String(t) => t.trim().parse().unwrap_or(f64::NAN),
                _ => s.as_f64().unwrap_or(f64::NAN),
            };
            json!(c::format(ms))
        }
        "start" => st(&c::start(num(a, 0), num(a, 1), &label(arg(a, 2)))),
        "startPhase" => st(&c::start_phase(
            num(a, 0),
            &settings(opt(a, 1)),
            from_json(arg(a, 2)),
            int(a, 3),
        )),
        "nextPhase" => to_json(&c::next_phase(from_json(s), int(a, 1), int(a, 2))),
        "remaining" => n(c::remaining(&state(s), num(a, 1))),
        "untilChange" => n(c::until_change(&state(s), num(a, 1))),
        "progress" => n(c::progress(&state(s), num(a, 1))),
        "pause" => st(&c::pause(&state(s), num(a, 1))),
        "resume" => st(&c::resume(&state(s), num(a, 1))),
        "toggle" => st(&c::toggle(&state(s), num(a, 1), &settings(opt(a, 2)))),
        "add" => st(&c::add(&state(s), num(a, 2))),
        "tick" => {
            let t = c::tick(&state(s), num(a, 1), &settings(opt(a, 2)));
            json!({ "state": st(&t.state), "finished": t.finished })
        }
        "skip" => st(&c::skip(&state(s), num(a, 1), &settings(opt(a, 2)))),
        "apply" => {
            let cmd: c::Command = from_json(arg(a, 1));
            st(&c::apply(&state(s), &cmd, num(a, 2), &settings(opt(a, 3))))
        }
        "title" => json!(c::title(&state(s))),
        _ => return None,
    })
}
