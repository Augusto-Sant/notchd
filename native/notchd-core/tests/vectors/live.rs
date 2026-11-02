//! `core/Live.js` against `notchd_core::live`. States come as JSON (complete,
//! as `initialState` makes them); `parseLine` answers `true` for an event and
//! `null` for anything else (the event's own shape is the Rust `Event`);
//! `reduce` reads its event with `parse_line`; a JS `undefined` hint is `None`.
use super::*;
use notchd_core::live as l;

fn role(v: &Value) -> l::Role {
    if v.as_str() == Some("user") {
        l::Role::User
    } else {
        l::Role::Assistant
    }
}

fn lines(v: &Value) -> Vec<l::Line> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| l::Line {
                    role: role(&x["role"]),
                    text: x["text"].as_str().unwrap_or("").to_string(),
                    done: x["done"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn state(v: &Value) -> l::State {
    if v.is_null() {
        return l::initial_state();
    }
    l::State {
        status: v["status"].as_str().unwrap_or("off").to_string(),
        error: v["error"].as_str().unwrap_or("").to_string(),
        packages: strs(&v["packages"]),
        components: strs(&v["components"]),
        warning: v["warning"].as_str().unwrap_or("").to_string(),
        transcript: lines(&v["transcript"]),
        working: v["working"].as_bool().unwrap_or(false),
        muted: v["muted"].as_bool().unwrap_or(false),
        level: v["level"].as_f64().unwrap_or(-100.0),
        mic: v["mic"].as_f64().unwrap_or(-100.0),
    }
}

fn st(s: &l::State) -> Value {
    to_json(s)
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    let s = arg(a, 0);
    Some(match f {
        "bridgeArgs" => json!(l::bridge_args(
            &jstext(a, 0),
            &opts(arg(a, 1)),
            &jstext(a, 2)
        )),
        "voiceEngine" => json!(l::voice_engine(&jstext(a, 0)).unwrap_or("")),
        "localBridgeArgs" => json!(l::local_bridge_args(
            &jstext(a, 0),
            &local_opts(arg(a, 1)),
            &jstext(a, 2)
        )),
        "parseLine" => {
            or_null(l::parse_line(arg(a, 0).as_str().unwrap_or("")).map(|_| json!(true)))
        }
        "initialState" => st(&l::initial_state()),
        "starting" => st(&l::starting(&state(s))),
        "reduce" => {
            let event = if arg(a, 1).is_null() {
                l::Event::default()
            } else {
                l::parse_line(&arg(a, 1).to_string()).expect("an event")
            };
            st(&l::reduce(&state(s), &event))
        }
        "addTranscript" => {
            let piece = l::parse_line(&json!({ "transcript": arg(a, 1) }).to_string())
                .and_then(|e| e.transcript)
                .unwrap_or_default();
            to_json(&l::add_transcript(&lines(s), &piece))
        }
        "speaking" => json!(!s.is_null() && l::speaking(&state(s))),
        "micLevel" => n(if s.is_null() {
            0.0
        } else {
            l::mic_level(&state(s))
        }),
        "running" => json!(!s.is_null() && l::running(&state(s))),
        "statusText" => json!(if s.is_null() {
            ""
        } else {
            l::status_text(&state(s))
        }),
        "errorText" => {
            let hint = arg(a, 1).as_str().filter(|h| !h.is_empty());
            json!(if s.is_null() {
                String::new()
            } else {
                l::error_text(&state(s), hint)
            })
        }
        "wanted" => json!(l::wanted(
            arg(a, 0).as_bool().unwrap_or(false),
            arg(a, 1).as_bool().unwrap_or(false)
        )),
        "parseCommand" => to_json(&l::parse_command(&jstext(a, 0))),
        "localPath" => json!(l::local_path(&jstext(a, 0), &jstext(a, 1))),
        _ => return None,
    })
}

/// A number option is read as text, as `String(x)` does.
fn textify(v: &Value) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        for x in m.values_mut() {
            if let Value::Number(num) = x {
                *x = Value::String(num.to_string());
            }
        }
    }
    v
}

fn opts(v: &Value) -> l::BridgeOpts {
    if v.is_null() {
        l::BridgeOpts::default()
    } else {
        from_json(&textify(v))
    }
}

fn local_opts(v: &Value) -> l::LocalBridgeOpts {
    if v.is_null() {
        l::LocalBridgeOpts::default()
    } else {
        from_json(&textify(v))
    }
}
