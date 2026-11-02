//! The differential check: every case in `tests/vectors/rules/*.json` (the
//! same files `tests/vectors-rules.test.mjs` runs against the JS) run against
//! this crate. Each case names a JS function (`fn`) and gives its `args` and
//! `expected` as JSON; `call` below maps the name to the Rust function,
//! adapting the arguments to the Rust types and the result back to JSON.
//!
//! Where the Rust API differs on purpose, the adapter maps it (a JS `-1` is a
//! `None`, a `{ error }` an `Err`, `undefined` a `None`): see the README in
//! `tests/vectors/`. A case with `knownDiff` is a disagreement between JS and
//! Rust that is known and open: it is skipped, and counted in the output.

use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[path = "vectors/agent.rs"]
mod agent;
#[path = "vectors/components.rs"]
mod components;
#[path = "vectors/config.rs"]
mod config;
#[path = "vectors/countdown.rs"]
mod countdown;
#[path = "vectors/live.rs"]
mod live;
#[path = "vectors/navigate.rs"]
mod navigate;
#[path = "vectors/position.rs"]
mod position;
#[path = "vectors/resources.rs"]
mod resources;
#[path = "vectors/settings.rs"]
mod settings;
#[path = "vectors/state.rs"]
mod state;
#[path = "vectors/terminal.rs"]
mod terminal;
#[path = "vectors/usage.rs"]
mod usage;

// ---------------------------------------------------------------------------
// JSON helpers shared by the adapters

/// `{"$undefined": true}`: JS `undefined`, as an argument and as a result.
pub fn undef() -> Value {
    json!({ "$undefined": true })
}

pub fn is_undef(v: &Value) -> bool {
    v.get("$undefined") == Some(&Value::Bool(true))
}

/// Argument `i`, `Null` when absent (the `undefined` marker stays a marker).
pub fn arg(a: &[Value], i: usize) -> &Value {
    a.get(i).unwrap_or(&Value::Null)
}

/// Argument `i`, `None` for an absent one or the `undefined` marker.
pub fn opt(a: &[Value], i: usize) -> Option<&Value> {
    a.get(i).filter(|v| !is_undef(v))
}

/// A result that may be JS `undefined`.
pub fn or_undef(v: Option<Value>) -> Value {
    v.unwrap_or_else(undef)
}

/// A result that is `null` when there is none.
pub fn or_null(v: Option<Value>) -> Value {
    v.unwrap_or(Value::Null)
}

pub fn text(a: &[Value], i: usize) -> String {
    arg(a, i).as_str().unwrap_or("").to_string()
}

/// Argument `i` as JS `String(x)` makes it for the scalars the cases use
/// (`""` for null and the `undefined` marker).
pub fn jstext(a: &[Value], i: usize) -> String {
    match arg(a, i) {
        Value::String(t) => t.clone(),
        Value::Number(x) => x.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

pub fn list(a: &[Value], i: usize) -> Vec<Value> {
    arg(a, i).as_array().cloned().unwrap_or_default()
}

pub fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|l| {
            l.iter()
                .map(|x| x.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub fn num(a: &[Value], i: usize) -> f64 {
    arg(a, i).as_f64().unwrap_or(f64::NAN)
}

pub fn int(a: &[Value], i: usize) -> i64 {
    arg(a, i).as_f64().map_or(0, |f| f as i64)
}

/// A number result as JSON; a whole one as an integer, as `JSON.stringify` writes it.
pub fn n(f: f64) -> Value {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e15 {
        Value::from(f as i64)
    } else {
        serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
    }
}

pub fn to_json<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).expect("serializes")
}

pub fn from_json<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("bad argument {v}: {e}"))
}

/// Equality as the vectors mean it: numbers by value (`1` is `1.0`), objects
/// by key and, when `ordered`, by key order too.
fn same(a: &Value, b: &Value, ordered: bool) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q, ordered))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same(v, w, ordered)))
                && (!ordered || x.keys().eq(y.keys()))
        }
        _ => a == b,
    }
}

// ---------------------------------------------------------------------------

/// The Rust function for `file`'s `fn`, or `None` when there is no adapter.
fn call(file: &str, f: &str, a: &[Value]) -> Option<Value> {
    match file {
        "config.json" => config::call(f, a),
        "settings.json" => settings::call(f, a),
        "state.json" => state::call(f, a),
        "navigate.json" => navigate::call(f, a),
        "position.json" => position::call(f, a),
        "countdown.json" => countdown::call(f, a),
        "usage.json" => usage::call(f, a),
        "resources.json" => resources::call(f, a),
        "agent.json" => agent::call(f, a),
        "live.json" => live::call(f, a),
        "components.json" => components::call(f, a),
        "terminal.json" => terminal::call(f, a),
        _ => None,
    }
}

#[test]
fn rules_agree_with_the_vectors() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/vectors/rules");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/vectors/rules exists")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no rule vectors found");

    let mut failures: Vec<String> = Vec::new();
    let (mut ran, mut skipped) = (0, 0);
    for path in files {
        let file = path.file_name().unwrap().to_str().unwrap().to_string();
        let suite: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let (mut file_ran, mut file_skipped) = (0, 0);
        for c in suite["cases"].as_array().expect("cases") {
            let (fname, name) = (c["fn"].as_str().unwrap(), c["name"].as_str().unwrap());
            if c.get("knownDiff").is_some() {
                file_skipped += 1;
                // A known difference that went away should lose its mark.
                let args = c["args"].as_array().cloned().unwrap_or_default();
                let ordered = c.get("ordered") == Some(&Value::Bool(true));
                if let Ok(Some(got)) = catch_unwind(AssertUnwindSafe(|| call(&file, fname, &args)))
                    && same(&got, &c["expected"], ordered)
                {
                    failures.push(format!(
                        "{file}: {fname}: {name}: marked knownDiff but now agrees; remove the mark"
                    ));
                }
                continue;
            }
            file_ran += 1;
            let args = c["args"].as_array().cloned().unwrap_or_default();
            let ordered = c.get("ordered") == Some(&Value::Bool(true));
            let got = catch_unwind(AssertUnwindSafe(|| call(&file, fname, &args)));
            match got {
                Err(_) => failures.push(format!("{file}: {fname}: {name}: panicked")),
                Ok(None) => failures.push(format!("{file}: {fname}: {name}: no adapter")),
                Ok(Some(got)) => {
                    if !same(&got, &c["expected"], ordered) {
                        failures.push(format!(
                            "{file}: {fname}: {name}\n    args:     {}\n    expected: {}\n    got:      {got}",
                            Value::Array(args),
                            c["expected"]
                        ));
                    }
                }
            }
        }
        println!("{file}: {file_ran} cases run, {file_skipped} skipped (knownDiff)");
        ran += file_ran;
        skipped += file_skipped;
    }
    println!("rule vectors: {ran} run, {skipped} skipped (knownDiff)");
    assert!(
        failures.is_empty(),
        "{} case(s) disagree:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
