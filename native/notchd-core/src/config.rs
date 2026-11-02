//! Port of `core/Config.js` (tests: `tests/config.test.mjs`).
//!
//! Config rules: reading and editing the JSON config, and resolving it into
//! the values the service runs on. The config is loose JSON, so it travels
//! as [`serde_json::Value`]; a JS `undefined` (as opposed to `null`) is an
//! `Option::None`. A missing key means its default, and defaults are never
//! written to the user's file ([`DEFAULTS`] holds them, once).
//!
//! Objects keep their file order (serde_json's `preserve_order`, set in the
//! workspace), as JS objects do: editing never reorders the user's file.

use std::sync::LazyLock;

use serde_json::{Map, Number, Value, json};

/// A JSON object, as the config uses them.
pub type Object = Map<String, Value>;

/// `DEFAULTS`: the service-wide defaults, the only place they live.
pub static DEFAULTS: LazyLock<Object> = LazyLock::new(|| {
    let Value::Object(map) = json!({
        "hoverOpenDelay": 120,
        "hoverCloseDelay": 300,
        "reserveSpace": false,
        "draggable": true,
        "tabs": true,
        "idleIcons": true
    }) else {
        unreachable!("an object literal")
    };
    map
});

/// `NOTCH_KEYS`: the keys a notch may override from the top level.
pub const NOTCH_KEYS: [&str; 7] = [
    "modules",
    "reserveSpace",
    "draggable",
    "tabs",
    "idleIcons",
    "workspaces",
    "position",
];

// ------------------------------------------------------------ JS helpers

/// `node[key]` on an object or (canonical numeric key) an array.
fn get<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    match node {
        Value::Object(map) => map.get(key),
        Value::Array(list) => index_of(key).and_then(|i| list.get(i)),
        _ => None,
    }
}

fn get_mut<'a>(node: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    match node {
        Value::Object(map) => map.get_mut(key),
        Value::Array(list) => index_of(key).and_then(|i| list.get_mut(i)),
        _ => None,
    }
}

/// A canonical array index ("0", "12"; not "01" or "-1").
fn index_of(key: &str) -> Option<usize> {
    let i: usize = key.parse().ok()?;
    (i.to_string() == key).then_some(i)
}

/// `node[key] = value`; an array takes a numeric key, padding holes with null.
fn put(node: &mut Value, key: &str, value: Value) {
    match node {
        Value::Object(map) => {
            map.insert(key.to_owned(), value);
        }
        Value::Array(list) => {
            // JS would make a sparse array for a huge index; refuse instead.
            if let Some(i) = index_of(key).filter(|&i| i <= list.len() + (1 << 16)) {
                if i >= list.len() {
                    list.resize(i + 1, Value::Null);
                }
                list[i] = value;
            }
        }
        _ => {}
    }
}

fn is_object_like(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Object(_) | Value::Array(_)))
}

/// JS `String(value)`.
fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.to_string(),
            (_, Some(u)) => u.to_string(),
            _ => n.as_f64().map_or_else(String::new, |f| f.to_string()),
        },
        Value::String(s) => s.clone(),
        Value::Array(list) => list
            .iter()
            .map(|e| {
                if e.is_null() {
                    String::new()
                } else {
                    js_string(e)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// JS `Number(value)` (NaN as NaN); objects and arrays simplified to NaN.
fn js_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                0.0
            } else if s == "Infinity" || s == "+Infinity" {
                f64::INFINITY
            } else if s == "-Infinity" {
                f64::NEG_INFINITY
            } else if s.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c)) {
                s.parse().unwrap_or(f64::NAN)
            } else {
                f64::NAN
            }
        }
        _ => f64::NAN,
    }
}

/// The JS `x || {}` then a deep copy: a falsy config is an empty object.
fn copy_cfg(cfg: &Value) -> Value {
    let falsy = match cfg {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        _ => false,
    };
    if falsy {
        Value::Object(Object::new())
    } else {
        cfg.clone()
    }
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn number_value(f: f64) -> Value {
    if f.fract() == 0.0 && (0.0..9.0e15).contains(&f) {
        Value::from(f as u64)
    } else {
        Number::from_f64(f).map_or(Value::Null, Value::Number)
    }
}

// ---------------------------------------------------------- read and edit

/// `parse(text)`: a config file's text as an object. Anything but a JSON
/// object is an error message; the caller keeps the last good config. Empty
/// text is an empty config. (The messages are serde_json's, not V8's.)
pub fn parse(text: &str) -> Result<Object, String> {
    let text = if text.is_empty() { "{}" } else { text };
    match serde_json::from_str::<Value>(text) {
        Err(e) => Err(e.to_string()),
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err("the config must be a JSON object".into()),
    }
}

/// `valueAt(cfg, key)`: a key's value; dotted keys reach into objects
/// (`"theme.accent"`) and lists by index. `None` is JS `undefined`; a stored
/// `null` comes back as `Some(Null)`.
pub fn value_at<'a>(cfg: &'a Value, key: &str) -> Option<&'a Value> {
    let mut node = cfg;
    for part in key.split('.') {
        if !is_object_like(Some(node)) {
            return None;
        }
        node = get(node, part)?;
    }
    Some(node)
}

/// `withValue(cfg, key, value)`: a copy of cfg with one key set. `None`
/// (`undefined`) removes the key, and the objects it leaves empty go too, so
/// the file never keeps `"theme": {}`. A numeric part indexes a list
/// (`"notches.1.position.edge"`); an entry of a list is never pruned, and
/// removing one (`"notches.1"`) takes it out of the list.
pub fn with_value(cfg: &Value, key: &str, value: Option<&Value>) -> Value {
    let mut out = copy_cfg(cfg);
    let parts: Vec<&str> = key.split('.').collect();
    set_path(&mut out, &parts, value);
    out
}

/// Sets or removes along `parts`, pruning emptied objects on the way back.
/// False when it stopped early (nothing to remove under a missing parent).
fn set_path(node: &mut Value, parts: &[&str], value: Option<&Value>) -> bool {
    let Some((key, rest)) = parts.split_first() else {
        return false;
    };
    if rest.is_empty() {
        match value {
            Some(v) => put(node, key, v.clone()),
            None => match node {
                Value::Array(list) => {
                    if is_digits(key)
                        && let Ok(i) = key.parse::<usize>()
                        && i < list.len()
                    {
                        list.remove(i);
                    }
                }
                Value::Object(map) => {
                    map.remove(*key);
                }
                _ => {}
            },
        }
        return true;
    }
    if !is_object_like(get(node, key)) {
        if value.is_none() {
            return false;
        }
        put(node, key, Value::Object(Object::new()));
    }
    let Some(child) = get_mut(node, key) else {
        return false;
    };
    if !set_path(child, rest, value) {
        return false;
    }
    let emptied = matches!(get(node, key), Some(Value::Object(m)) if m.is_empty());
    if emptied && let Value::Object(map) = node {
        map.remove(*key);
    }
    true
}

/// `delay(value, fallback)`: a non-negative delay; `undefined` is the
/// fallback, anything that is no number is 0.
fn delay(value: Option<&Value>, fallback: f64) -> f64 {
    let Some(v) = value else { return fallback };
    let n = js_number(v);
    if n.is_nan() { 0.0 } else { n.max(0.0) }
}

/// `workspaceList(value)`: the workspaces the notch hangs on, ids or names,
/// or `None` for every one (also for an empty or non-list value).
pub fn workspace_list(value: Option<&Value>) -> Option<Vec<Value>> {
    let Some(Value::Array(list)) = value else {
        return None;
    };
    let out: Vec<Value> = list
        .iter()
        .filter(|w| match w {
            Value::Number(_) => true,
            Value::String(s) => !s.is_empty(),
            _ => false,
        })
        .cloned()
        .collect();
    (!out.is_empty()).then_some(out)
}

/// The service-wide keys, as [`resolve`] returns them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    pub hover_open_delay: f64,
    pub hover_close_delay: f64,
}

/// `resolve(cfg)`: the keys that apply to the whole service. A missing key
/// falls back to its default.
pub fn resolve(cfg: &Value) -> Resolved {
    let default = |key: &str| DEFAULTS.get(key).map_or(0.0, js_number);
    Resolved {
        hover_open_delay: delay(get(cfg, "hoverOpenDelay"), default("hoverOpenDelay")),
        hover_close_delay: delay(get(cfg, "hoverCloseDelay"), default("hoverCloseDelay")),
    }
}

// --------------------------------------------------------------- notches

/// `hasNotchList(cfg)`: whether the config lists its notches (else the top
/// level is the one).
pub fn has_notch_list(cfg: &Value) -> bool {
    matches!(get(cfg, "notches"), Some(Value::Array(l)) if !l.is_empty())
}

/// `defaultNotchId(index)`: "main" first, then notch-2...
pub fn default_notch_id(index: usize) -> String {
    if index == 0 {
        "main".into()
    } else {
        format!("notch-{}", index + 1)
    }
}

/// One notch, resolved (the JS object `notches()` returns).
#[derive(Debug, Clone, PartialEq)]
pub struct NotchSpec {
    /// Its own id or [`default_notch_id`]; unique (a repeat gets a suffix).
    pub id: String,
    /// Its place in the list.
    pub index: usize,
    /// What goes before a notch key in the config: "" for the top-level
    /// notch, "notches.<index>." otherwise.
    pub prefix: String,
    pub module_ids: Vec<String>,
    pub reserve_space: bool,
    pub draggable: bool,
    /// Whether the open notch shows one module at a time; on unless false.
    pub tabs: bool,
    /// Whether an idle active module shows its icon; on unless false.
    pub idle_icons: bool,
    pub workspaces: Option<Vec<Value>>,
    /// The raw `position`, top-level keys merged under the notch's.
    pub position: Option<Value>,
}

/// `notches(cfg, defaultModules)`: every notch, resolved. A key a notch
/// leaves out comes from the top level, then its default; `position` merges.
pub fn notches(cfg: &Value, default_modules: &[&str]) -> Vec<NotchSpec> {
    let empty = Value::Object(Object::new());
    let cfg = if cfg.is_null() { &empty } else { cfg };
    let listed = has_notch_list(cfg);
    let single = [empty.clone()];
    let entries: &[Value] = match get(cfg, "notches") {
        Some(Value::Array(l)) if listed => l,
        _ => &single,
    };
    let mut used: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (i, raw) in entries.iter().enumerate() {
        let entry = if raw.is_object() { raw } else { &empty };
        let pick = |key: &str| get(entry, key).or_else(|| get(cfg, key));
        let id = match get(entry, "id") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
            _ => default_notch_id(i),
        };
        let mut unique = id.clone();
        let mut n = 2;
        while used.contains(&unique) {
            unique = format!("{id}-{n}");
            n += 1;
        }
        used.push(unique.clone());
        let top = get(cfg, "position").and_then(Value::as_object);
        let own = get(entry, "position").and_then(Value::as_object);
        let position = (top.is_some() || own.is_some()).then(|| {
            let mut merged = top.cloned().unwrap_or_default();
            merged.extend(own.cloned().unwrap_or_default());
            Value::Object(merged)
        });
        out.push(NotchSpec {
            id: unique,
            index: i,
            prefix: if listed {
                format!("notches.{i}.")
            } else {
                String::new()
            },
            module_ids: match pick("modules") {
                Some(Value::Array(l)) => l.iter().map(js_string).collect(),
                _ => default_modules.iter().map(|s| (*s).to_owned()).collect(),
            },
            reserve_space: pick("reserveSpace") == Some(&Value::Bool(true)),
            draggable: pick("draggable") != Some(&Value::Bool(false)),
            tabs: pick("tabs") != Some(&Value::Bool(false)),
            idle_icons: pick("idleIcons") != Some(&Value::Bool(false)),
            workspaces: workspace_list(pick("workspaces")),
            position,
        });
    }
    out
}

/// `moduleUnion(specs, extra)`: every module any notch shows, in order of
/// first appearance, then the `extra` ones no notch lists.
pub fn module_union(specs: &[NotchSpec], extra: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let all = specs
        .iter()
        .flat_map(|s| s.module_ids.iter().map(String::as_str))
        .chain(extra.iter().copied());
    for id in all {
        if !out.iter().any(|o| o == id) {
            out.push(id.to_owned());
        }
    }
    out
}

/// `moduleLoadList(specs, standalone, enabled)`: the modules to load: every
/// one a notch shows except standalone ones, then the standalone ones
/// switched on.
pub fn module_load_list(specs: &[NotchSpec], standalone: &[&str], enabled: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = module_union(specs, &[])
        .into_iter()
        .filter(|id| !standalone.contains(&id.as_str()))
        .collect();
    for id in enabled {
        if standalone.contains(id) && !out.iter().any(|o| o == id) {
            out.push((*id).to_owned());
        }
    }
    out
}

/// `freeNotchId(ids)`: notch-<n>, the lowest n (from 2) not taken.
pub fn free_notch_id(ids: &[&str]) -> String {
    (2..)
        .map(|n| format!("notch-{n}"))
        .find(|id| !ids.contains(&id.as_str()))
        .expect("an unbounded range")
}

/// `withNotchAdded(cfg, entry)`: a copy of cfg with `entry` added as a notch.
/// A config without a list gets one first, its top-level notch as an empty
/// entry that inherits it all.
pub fn with_notch_added(cfg: &Value, entry: &Value) -> Value {
    let mut out = copy_cfg(cfg);
    let had_list = has_notch_list(&out);
    if let Value::Object(map) = &mut out {
        if !had_list {
            map.insert("notches".into(), json!([{}]));
        }
        if let Some(Value::Array(list)) = map.get_mut("notches") {
            list.push(copy_cfg(entry));
        }
    }
    out
}

/// `withoutNotch(cfg, index)`: a copy of cfg without the notch at `index`.
/// When one notch is left, its keys move to the top level and the list goes.
/// The last notch is never removed; a bad index changes nothing.
pub fn without_notch(cfg: &Value, index: i64) -> Value {
    let mut out = copy_cfg(cfg);
    let Some(list_len) = get(&out, "notches")
        .and_then(Value::as_array)
        .map(Vec::len)
        .filter(|&n| has_notch_list(&out) && n >= 2)
    else {
        return out;
    };
    let Some(index) = usize::try_from(index).ok().filter(|&i| i < list_len) else {
        return out;
    };
    let Value::Object(map) = &mut out else {
        return out;
    };
    let Some(Value::Array(list)) = map.get_mut("notches") else {
        return out;
    };
    list.remove(index);
    if list.len() > 1 {
        return out;
    }
    let last = match list.first() {
        Some(Value::Object(o)) => o.clone(),
        _ => Object::new(),
    };
    map.remove("notches");
    for key in NOTCH_KEYS {
        let Some(v) = last.get(key) else { continue };
        match (key, map.get_mut(key), v) {
            ("position", Some(Value::Object(top)), Value::Object(own)) => top.extend(own.clone()),
            // Object.assign(top, null) changes nothing.
            ("position", Some(Value::Object(_)), Value::Null) => {}
            _ => {
                map.insert(key.to_owned(), v.clone());
            }
        }
    }
    out
}

/// `withWorkspace(list, ws)`: the `workspaces` list with workspace `ws`
/// (`{ id, name }`) let in, for a notch dropped on a screen showing it.
/// `None` (every workspace) and a list that already has it come back
/// unchanged; otherwise it is added as the settings window stores it: a
/// number for a numbered workspace, else its name, in order (numbers, then
/// names). (The JS also took array-likes; Rust has no such thing.)
pub fn with_workspace(list: Option<&[Value]>, ws: Option<&Value>) -> Option<Vec<Value>> {
    let list = list?;
    let Some(ws) = ws.filter(|w| !w.is_null()) else {
        return Some(list.to_vec());
    };
    let id = get(ws, "id");
    let name = get(ws, "name");
    let text = |v: Option<&Value>| v.map_or_else(|| "undefined".to_owned(), js_string);
    if list.iter().any(|e| {
        let e = js_string(e);
        e == text(id) || e == text(name)
    }) {
        return Some(list.to_vec());
    }
    let name = name
        .filter(|n| !n.is_null())
        .map(js_string)
        .unwrap_or_default();
    let value = if is_digits(&name) {
        number_value(name.parse().unwrap_or(f64::NAN))
    } else if !name.is_empty() {
        Value::String(name)
    } else {
        id.cloned().unwrap_or(Value::Null)
    };
    let mut out = list.to_vec();
    out.push(value);
    out.sort_by(|a, b| {
        match (
            a.is_number().then(|| a.as_f64()),
            b.is_number().then(|| b.as_f64()),
        ) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => js_string(a).cmp(&js_string(b)),
        }
    });
    Some(out)
}

// -------------------------------------------------------------- terminals

/// `RESERVED_KEYS`: the ids a terminal can't take, being keys the config
/// already has.
pub fn reserved_keys() -> Vec<String> {
    ["modules", "notches", "position", "theme", "terminals"]
        .into_iter()
        .map(str::to_owned)
        .chain(DEFAULTS.keys().cloned())
        .chain(NOTCH_KEYS.iter().map(|k| (*k).to_owned()))
        .collect()
}

/// `TERMINAL_ID`: a letter, then letters, digits, `-` or `_`.
fn is_terminal_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A terminal of your own, as [`terminals`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub id: String,
    /// "" when the entry has none.
    pub title: String,
    pub command: String,
}

/// What [`terminals`] returns: `{ list, errors }`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Terminals {
    pub list: Vec<Terminal>,
    /// `"<id>: why"`, one per entry that makes no terminal.
    pub errors: Vec<String>,
}

/// `terminals(cfg, taken)`: the entries of `terminals` that make a terminal,
/// each other one with why. `taken` is the host's module and key ids.
pub fn terminals(cfg: &Value, taken: &[&str]) -> Terminals {
    let mut out = Terminals::default();
    let Some(all) = get(cfg, "terminals") else {
        return out;
    };
    let Value::Object(all) = all else {
        out.errors
            .push("terminals: not an object of terminals by id".into());
        return out;
    };
    let mut reserved = reserved_keys();
    reserved.extend(taken.iter().map(|t| (*t).to_owned()));
    for (id, entry) in all {
        if !is_terminal_id(id) {
            out.errors.push(format!(
                "{id}: an id is a letter, then letters, digits, - or _"
            ));
            continue;
        }
        if reserved.contains(id) {
            out.errors
                .push(format!("{id}: taken, by a config key or a module"));
            continue;
        }
        if !entry.is_object() {
            out.errors.push(format!("{id}: not an object"));
            continue;
        }
        let text = |key: &str| get(entry, key).and_then(Value::as_str).map(str::trim);
        let command = text("command").unwrap_or("");
        if command.is_empty() {
            out.errors.push(format!("{id}: needs a command"));
            continue;
        }
        out.list.push(Terminal {
            id: id.clone(),
            title: text("title").unwrap_or("").to_owned(),
            command: command.to_owned(),
        });
    }
    out
}

/// What [`with_terminal_added`] returns: `{ config, id }`.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalAdded {
    pub config: Value,
    pub id: String,
}

/// `withTerminalAdded(cfg, taken)`: a copy of cfg with a new terminal, id
/// "terminal" else terminal-<n>, the first no config key, terminal or `taken`
/// id has; it runs your shell, for its page to change.
pub fn with_terminal_added(cfg: &Value, taken: &[&str]) -> TerminalAdded {
    let mut out = copy_cfg(cfg);
    let mut used = reserved_keys();
    used.extend(taken.iter().map(|t| (*t).to_owned()));
    if let Value::Object(map) = &out {
        used.extend(map.keys().cloned());
        if let Some(Value::Object(t)) = map.get("terminals") {
            used.extend(t.keys().cloned());
        }
    }
    let name = |n: u32| {
        if n == 1 {
            "terminal".to_owned()
        } else {
            format!("terminal-{n}")
        }
    };
    let n = (1..)
        .find(|&n| !used.contains(&name(n)))
        .expect("an unbounded range");
    let id = name(n);
    if let Value::Object(map) = &mut out {
        if !matches!(map.get("terminals"), Some(Value::Object(_))) {
            map.insert("terminals".into(), json!({}));
        }
        if let Some(Value::Object(t)) = map.get_mut("terminals") {
            let title = if n == 1 {
                "Terminal".to_owned()
            } else {
                format!("Terminal {n}")
            };
            t.insert(id.clone(), json!({ "title": title, "command": "$SHELL" }));
        }
    }
    TerminalAdded { config: out, id }
}

/// `withoutTerminal(cfg, id)`: a copy of cfg without terminal `id`: its
/// entry, its settings (the key of its id) and its place in every notch's
/// `modules`. `terminals` goes once empty. Not a terminal: unchanged.
pub fn without_terminal(cfg: &Value, id: &str) -> Value {
    let mut out = copy_cfg(cfg);
    let Value::Object(map) = &mut out else {
        return out;
    };
    let Some(Value::Object(terms)) = map.get_mut("terminals") else {
        return out;
    };
    if terms.remove(id).is_none() {
        return out;
    }
    if terms.is_empty() {
        map.remove("terminals");
    }
    map.remove(id);
    let drop_from = |entry: &mut Value| {
        if let Some(Value::Array(modules)) = entry.get_mut("modules") {
            modules.retain(|m| m.as_str() != Some(id));
        }
    };
    drop_from(&mut out);
    if let Some(Value::Array(list)) = out.get_mut("notches") {
        list.iter_mut().for_each(drop_from);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first notch of `cfg`, with `["media"]` as the default modules.
    fn one(cfg: Value) -> NotchSpec {
        notches(&cfg, &["media"]).remove(0)
    }

    // parse accepts a JSON object and rejects everything else
    #[test]
    fn parse_accepts_a_json_object_and_rejects_everything_else() {
        assert_eq!(
            Value::Object(parse(r#"{"a":1}"#).unwrap()),
            json!({ "a": 1 })
        );
        assert_eq!(parse("").unwrap(), Object::new());
        assert!(parse("[1]").is_err());
        assert!(parse(r#""x""#).is_err());
        assert!(parse("null").is_err());
        assert!(parse("{oops").is_err());
    }

    // valueAt follows dotted keys and tolerates missing parents
    #[test]
    fn value_at_follows_dotted_keys_and_tolerates_missing_parents() {
        let cfg = json!({ "theme": { "accent": "#fff" }, "modules": ["media"] });
        assert_eq!(value_at(&cfg, "theme.accent"), Some(&json!("#fff")));
        assert_eq!(value_at(&cfg, "theme.missing"), None);
        assert_eq!(value_at(&cfg, "nope.deeper.still"), None);
        assert_eq!(value_at(&cfg, "modules.0"), Some(&json!("media")));
    }

    // withValue sets nested keys without touching the input
    #[test]
    fn with_value_sets_nested_keys_without_touching_the_input() {
        let cfg = json!({ "hoverOpenDelay": 100 });
        let next = with_value(&cfg, "theme.accent", Some(&json!("#abc")));
        assert_eq!(
            next,
            json!({ "hoverOpenDelay": 100, "theme": { "accent": "#abc" } })
        );
        assert_eq!(cfg, json!({ "hoverOpenDelay": 100 }));
    }

    // withValue stores a plain copy of the value
    #[test]
    fn with_value_stores_a_plain_copy_of_the_value() {
        let mut list = json!([1, 2]);
        let next = with_value(&json!({}), "workspaces", Some(&list));
        list.as_array_mut().unwrap().push(json!(3));
        assert_eq!(next, json!({ "workspaces": [1, 2] }));
        assert!(next["workspaces"].is_array());
    }

    // withValue with undefined removes the key and emptied parents
    #[test]
    fn with_value_with_undefined_removes_the_key_and_emptied_parents() {
        let cfg = json!({ "theme": { "accent": "#abc" }, "topOffset": 4 });
        assert_eq!(
            with_value(&cfg, "theme.accent", None),
            json!({ "topOffset": 4 })
        );
        assert_eq!(
            with_value(&cfg, "topOffset", None),
            json!({ "theme": { "accent": "#abc" } })
        );
    }

    // withValue removing a key under a missing parent changes nothing
    #[test]
    fn with_value_removing_a_key_under_a_missing_parent_changes_nothing() {
        assert_eq!(
            with_value(&json!({ "a": 1 }), "theme.accent", None),
            json!({ "a": 1 })
        );
    }

    // resolve: the service-wide keys, defaults for missing or bad ones
    #[test]
    fn resolve_the_service_wide_keys_defaults_for_missing_or_bad_ones() {
        assert_eq!(
            resolve(&json!({})),
            Resolved {
                hover_open_delay: 120.0,
                hover_close_delay: 300.0
            }
        );
        assert_eq!(
            resolve(&json!({ "hoverOpenDelay": -50, "hoverCloseDelay": "x" })),
            Resolved {
                hover_open_delay: 0.0,
                hover_close_delay: 0.0
            }
        );
    }

    // notches: without a list, the top level is the one notch, at its defaults
    #[test]
    fn notches_without_a_list_the_top_level_is_the_one_notch_at_its_defaults() {
        assert_eq!(
            notches(&json!({}), &["media"]),
            vec![NotchSpec {
                id: "main".into(),
                index: 0,
                prefix: String::new(),
                module_ids: vec!["media".into()],
                reserve_space: false,
                draggable: true,
                tabs: true,
                idle_icons: true,
                workspaces: None,
                position: None,
            }]
        );
        assert_eq!(notches(&json!({ "notches": [] }), &[]).len(), 1);
    }

    // notches: reads and clamps the notch keys
    #[test]
    fn notches_reads_and_clamps_the_notch_keys() {
        let n = notches(
            &json!({ "modules": ["osd", 3], "reserveSpace": true, "draggable": false,
                     "tabs": false, "workspaces": [1, "code"] }),
            &[],
        )
        .remove(0);
        assert_eq!(n.module_ids, ["osd", "3"]);
        assert!(n.reserve_space);
        assert!(!n.draggable);
        assert!(!n.tabs);
        assert!(one(json!({ "tabs": "no" })).tabs);
        assert!(!one(json!({ "idleIcons": false })).idle_icons);
        assert!(one(json!({ "idleIcons": "no" })).idle_icons);
        assert_eq!(n.workspaces, Some(vec![json!(1), json!("code")]));
        // The compact style left with the notch in the bar: a config that
        // still has one changes nothing.
        assert_eq!(one(json!({ "compactStyle": "bar" })), one(json!({})));
        assert!(!one(json!({ "reserveSpace": "yes" })).reserve_space);
        assert!(one(json!({ "draggable": "no" })).draggable);
        assert_eq!(one(json!({ "workspaces": [] })).workspaces, None);
        assert_eq!(one(json!({ "workspaces": 3 })).workspaces, None);
        assert_eq!(
            one(json!({ "workspaces": [2, "", null, "x"] })).workspaces,
            Some(vec![json!(2), json!("x")])
        );
    }

    // notches: each listed notch inherits the top-level keys it leaves out
    #[test]
    fn notches_each_listed_notch_inherits_the_top_level_keys_it_leaves_out() {
        let cfg = json!({
            "modules": ["media"], "reserveSpace": true,
            "position": { "edge": "left", "offset": 0.2 },
            "notches": [{}, { "id": "jobs", "modules": ["activities"], "reserveSpace": false,
                              "tabs": false, "position": { "offset": 0.8, "screen": "all" } }]
        });
        let specs = notches(&cfg, &["osd"]);
        let (main, jobs) = (&specs[0], &specs[1]);
        assert_eq!([main.tabs, jobs.tabs], [true, false]);
        assert_eq!(
            (
                main.id.as_str(),
                main.prefix.as_str(),
                &main.module_ids,
                main.reserve_space
            ),
            ("main", "notches.0.", &vec!["media".to_owned()], true)
        );
        assert_eq!(
            main.position,
            Some(json!({ "edge": "left", "offset": 0.2 }))
        );
        assert_eq!(
            (
                jobs.id.as_str(),
                jobs.index,
                jobs.prefix.as_str(),
                &jobs.module_ids,
                jobs.reserve_space
            ),
            (
                "jobs",
                1,
                "notches.1.",
                &vec!["activities".to_owned()],
                false
            )
        );
        assert_eq!(
            jobs.position,
            Some(json!({ "edge": "left", "offset": 0.8, "screen": "all" }))
        );
    }

    // notches: ids default by place and never repeat
    #[test]
    fn notches_ids_default_by_place_and_never_repeat() {
        let cfg = json!({ "notches": [{}, {}, { "id": "main" }, { "id": " x " }, { "id": "x" }, "junk"] });
        let ids: Vec<String> = notches(&cfg, &[]).into_iter().map(|n| n.id).collect();
        assert_eq!(ids, ["main", "notch-2", "main-2", "x", "x-2", "notch-6"]);
    }

    // moduleUnion: every module a notch shows, once, in first-seen order
    #[test]
    fn module_union_every_module_a_notch_shows_once_in_first_seen_order() {
        let cfg = json!({ "notches": [{ "modules": ["osd", "media"] }, { "modules": ["activities", "osd"] }] });
        let specs = notches(&cfg, &[]);
        assert_eq!(module_union(&specs, &[]), ["osd", "media", "activities"]);
        // The host's system modules load whether a notch lists them or not.
        assert_eq!(
            module_union(&specs, &["osd", "clock"]),
            ["osd", "media", "activities", "clock"]
        );
    }

    // freeNotchId: the lowest notch-<n> not taken
    #[test]
    fn free_notch_id_the_lowest_notch_n_not_taken() {
        assert_eq!(free_notch_id(&["main"]), "notch-2");
        assert_eq!(free_notch_id(&["main", "notch-2", "notch-4"]), "notch-3");
    }

    // withNotchAdded turns the top-level notch into the first entry
    #[test]
    fn with_notch_added_turns_the_top_level_notch_into_the_first_entry() {
        let cfg = json!({ "modules": ["media"], "hoverOpenDelay": 50 });
        let next = with_notch_added(&cfg, &json!({ "id": "jobs", "modules": [] }));
        assert_eq!(
            next,
            json!({ "modules": ["media"], "hoverOpenDelay": 50,
                    "notches": [{}, { "id": "jobs", "modules": [] }] })
        );
        assert_eq!(
            with_notch_added(&next, &json!({ "id": "c" }))["notches"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(cfg, json!({ "modules": ["media"], "hoverOpenDelay": 50 }));
    }

    // withoutNotch: the last one left goes back to the top level
    #[test]
    fn without_notch_the_last_one_left_goes_back_to_the_top_level() {
        let cfg = json!({
            "modules": ["media"], "position": { "edge": "left", "offset": 0.2 },
            "notches": [{}, { "id": "jobs", "modules": ["activities"], "position": { "offset": 0.9 } }]
        });
        assert_eq!(
            without_notch(&cfg, 0),
            json!({ "modules": ["activities"], "position": { "edge": "left", "offset": 0.9 } })
        );
        assert_eq!(
            without_notch(&cfg, 1),
            json!({ "modules": ["media"], "position": { "edge": "left", "offset": 0.2 } })
        );
        let three = json!({ "notches": [{ "id": "a" }, { "id": "b" }, { "id": "c" }] });
        assert_eq!(
            without_notch(&three, 1),
            json!({ "notches": [{ "id": "a" }, { "id": "c" }] })
        );
        // The last notch stays.
        assert_eq!(
            without_notch(&json!({ "modules": [] }), 0),
            json!({ "modules": [] })
        );
        assert_eq!(without_notch(&three, 7), three);
    }

    // withValue reaches into the notch list and never prunes an entry
    #[test]
    fn with_value_reaches_into_the_notch_list_and_never_prunes_an_entry() {
        let cfg = json!({ "notches": [{}, { "id": "jobs", "position": { "edge": "left" } }] });
        assert_eq!(
            with_value(&cfg, "notches.1.position.edge", Some(&json!("right")))["notches"][1],
            json!({ "id": "jobs", "position": { "edge": "right" } })
        );
        assert_eq!(
            with_value(&cfg, "notches.0.reserveSpace", Some(&json!(true)))["notches"][0],
            json!({ "reserveSpace": true })
        );
        // Emptying an entry keeps it: an empty notch inherits everything.
        let emptied = with_value(
            &json!({ "notches": [{ "draggable": false }, {}] }),
            "notches.0.draggable",
            None,
        );
        assert_eq!(emptied, json!({ "notches": [{}, {}] }));
        let moved = with_value(&cfg, "notches.1.position.edge", None);
        assert_eq!(moved, json!({ "notches": [{}, { "id": "jobs" }] }));
        // Removing an entry by index takes it out of the list, no hole.
        assert_eq!(
            with_value(&cfg, "notches.0", None),
            json!({ "notches": [{ "id": "jobs", "position": { "edge": "left" } }] })
        );
    }

    // Bug: a notch dragged onto a screen showing a workspace left out of
    // `workspaces` landed there and went straight back to the other screen.
    // The drop lets that workspace in.
    // withWorkspace adds the workspace a notch was dropped on
    #[test]
    fn with_workspace_adds_the_workspace_a_notch_was_dropped_on() {
        let add = |list: Value, ws: Value| -> Value {
            json!(with_workspace(
                list.as_array().map(Vec::as_slice),
                Some(&ws)
            ))
        };
        assert_eq!(
            add(json!([1, 2, 3]), json!({ "id": 4, "name": "4" })),
            json!([1, 2, 3, 4])
        );
        assert_eq!(
            add(json!([3, "code"]), json!({ "id": 1, "name": "1" })),
            json!([1, 3, "code"])
        );
        assert_eq!(
            add(json!([1]), json!({ "id": 7, "name": "mail" })),
            json!([1, "mail"])
        );
        assert_eq!(
            add(json!([1, 2]), json!({ "id": 2, "name": "2" })),
            json!([1, 2])
        );
        assert_eq!(
            add(json!(["web"]), json!({ "id": 9, "name": "web" })),
            json!(["web"])
        );
        assert_eq!(
            with_workspace(None, Some(&json!({ "id": 4, "name": "4" }))),
            None
        );
        assert_eq!(
            with_workspace(Some(&[json!(1)]), None),
            Some(vec![json!(1)])
        );
        // The JS test's array-like (a QML var property) has no Rust shape:
        // a slice is the only list there is.
    }

    // moduleLoadList: what notches show, never a standalone module, then the standalone ones switched on
    #[test]
    fn module_load_list_what_notches_show_never_a_standalone_module_then_the_standalone_ones_switched_on()
     {
        let specs = vec![
            NotchSpec {
                module_ids: vec!["osd".into(), "media".into()],
                ..one(json!({}))
            },
            NotchSpec {
                module_ids: vec!["activities".into(), "media".into()],
                ..one(json!({}))
            },
        ];
        assert_eq!(
            module_load_list(&specs, &["osd", "clock"], &["osd", "clock"]),
            ["media", "activities", "osd", "clock"]
        );
        // osd is off, even listed
        assert_eq!(
            module_load_list(&specs, &["osd", "clock"], &["clock"]),
            ["media", "activities", "clock"]
        );
        // enabled only counts for standalone ones
        assert_eq!(
            module_load_list(&specs, &[], &["media"]),
            ["osd", "media", "activities"]
        );
        assert!(module_load_list(&[], &[], &[]).is_empty());
    }

    // terminals: each entry with a command, in order, its title or ""
    #[test]
    fn terminals_each_entry_with_a_command_in_order_its_title_or_empty() {
        let r = terminals(
            &json!({ "terminals": {
                "btop": { "title": "System", "command": " btop " },
                "logs": { "command": "journalctl -f" }
            } }),
            &[],
        );
        let terminal = |id: &str, title: &str, command: &str| Terminal {
            id: id.into(),
            title: title.into(),
            command: command.into(),
        };
        assert_eq!(
            r,
            Terminals {
                list: vec![
                    terminal("btop", "System", "btop"),
                    terminal("logs", "", "journalctl -f")
                ],
                errors: vec![],
            }
        );
        assert_eq!(terminals(&json!({}), &[]), Terminals::default());
    }

    // terminals: an id that is taken or no key, or an entry without a command, is left out with why
    //
    // An id is a config key too ("btop": { "width" }): one the config already
    // has, a module's, or one with a dot would read or write something else.
    #[test]
    fn terminals_an_id_that_is_taken_or_no_key_or_an_entry_without_a_command_is_left_out_with_why()
    {
        let r = terminals(
            &json!({ "terminals": {
                "theme": { "command": "x" }, "tabs": { "command": "x" }, "media": { "command": "x" },
                "a.b": { "command": "x" }, "1x": { "command": "x" },
                "empty": { "command": "  " }, "none": {}, "bad": "btop", "ok": { "command": "x" }
            } }),
            &["media"],
        );
        assert_eq!(
            r.list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["ok"]
        );
        assert_eq!(r.errors.len(), 8, "{}", r.errors.join("\n"));
        assert!(r.errors[0].starts_with("theme: taken"));
        assert!(r.errors[2].starts_with("media: taken"));
        assert_eq!(
            terminals(&json!({ "terminals": ["btop"] }), &[])
                .errors
                .len(),
            1
        );
    }

    // withTerminalAdded: a free id, running your shell
    #[test]
    fn with_terminal_added_a_free_id_running_your_shell() {
        let first = with_terminal_added(&json!({ "modules": ["media"] }), &[]);
        assert_eq!(first.id, "terminal");
        assert_eq!(
            first.config,
            json!({ "modules": ["media"],
                    "terminals": { "terminal": { "title": "Terminal", "command": "$SHELL" } } })
        );
        // Taken by a terminal, a config key or a module: the next number.
        let next = with_terminal_added(
            &json!({ "terminals": { "terminal": { "command": "x" } }, "terminal-2": { "width": 1 } }),
            &["terminal-3"],
        );
        assert_eq!(next.id, "terminal-4");
        assert_eq!(
            next.config["terminals"]["terminal-4"],
            json!({ "title": "Terminal 4", "command": "$SHELL" })
        );
        assert!(terminals(&next.config, &[]).errors.is_empty());
    }

    // withoutTerminal: its entry, its settings and its place in every notch go
    #[test]
    fn without_terminal_its_entry_its_settings_and_its_place_in_every_notch_go() {
        let cfg = json!({
            "modules": ["media", "btop"],
            "notches": [{}, { "id": "tools", "modules": ["btop", "dev"] }],
            "terminals": { "btop": { "command": "btop" }, "dev": { "command": "npm run dev" } },
            "btop": { "width": 900 }
        });
        assert_eq!(
            without_terminal(&cfg, "btop"),
            json!({
                "modules": ["media"],
                "notches": [{}, { "id": "tools", "modules": ["dev"] }],
                "terminals": { "dev": { "command": "npm run dev" } }
            })
        );
        let none = without_terminal(&without_terminal(&cfg, "btop"), "dev");
        assert!(none.get("terminals").is_none());
        // Not a terminal: nothing changes, a module's settings least of all.
        assert_eq!(
            without_terminal(&json!({ "media": { "equalizer": "light" } }), "media"),
            json!({ "media": { "equalizer": "light" } })
        );
    }
}
