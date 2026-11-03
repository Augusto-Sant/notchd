//! A small JSON Schema checker: the subset `view.schema.json` uses (`$ref` into
#![allow(dead_code)]
//! `$defs`, `type`, `enum`, `const`, `oneOf`, `anyOf`, `required`, `properties`,
//! `items`, `minItems`, `minLength`, `minimum`, `maximum`, `exclusiveMinimum`).
//! Enough to hold the fixtures to the schema without a heavy dependency.

use serde_json::Value;
use std::path::Path;

pub fn load() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/view.schema.json");
    let text = std::fs::read_to_string(&path).expect("schema file");
    serde_json::from_str(&text).expect("schema is JSON")
}

pub fn validate(root: &Value, schema: &Value, v: &Value) -> Result<(), String> {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r
            .strip_prefix("#/$defs/")
            .ok_or(format!("unsupported $ref {r}"))?;
        let target = root["$defs"].get(name).ok_or(format!("no def {name}"))?;
        return validate(root, target, v);
    }
    if let Some(c) = schema.get("const")
        && c != v
    {
        return Err(format!("{v} is not {c}"));
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array)
        && !e.contains(v)
    {
        return Err(format!("{v} is not one of {e:?}"));
    }
    if let Some(t) = schema.get("type").and_then(Value::as_str) {
        let ok = match t {
            "string" => v.is_string(),
            "number" => v.is_number(),
            "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
            "boolean" => v.is_boolean(),
            "object" => v.is_object(),
            "array" => v.is_array(),
            "null" => v.is_null(),
            other => return Err(format!("unsupported type {other}")),
        };
        if !ok {
            return Err(format!("{v} is not a {t}"));
        }
    }
    if let Some(options) = schema.get("oneOf").and_then(Value::as_array) {
        let matching = options
            .iter()
            .filter(|o| validate(root, o, v).is_ok())
            .count();
        if matching != 1 {
            return Err(format!("{matching} of oneOf match for {v}"));
        }
    }
    if let Some(options) = schema.get("anyOf").and_then(Value::as_array)
        && !options.iter().any(|o| validate(root, o, v).is_ok())
    {
        return Err(format!("none of anyOf match for {v}"));
    }
    if let (Some(n), Some(min)) = (v.as_f64(), schema.get("minimum").and_then(Value::as_f64))
        && n < min
    {
        return Err(format!("{n} < {min}"));
    }
    if let (Some(n), Some(max)) = (v.as_f64(), schema.get("maximum").and_then(Value::as_f64))
        && n > max
    {
        return Err(format!("{n} > {max}"));
    }
    if let (Some(n), Some(min)) = (
        v.as_f64(),
        schema.get("exclusiveMinimum").and_then(Value::as_f64),
    ) && n <= min
    {
        return Err(format!("{n} <= {min}"));
    }
    if let (Some(s), Some(min)) = (v.as_str(), schema.get("minLength").and_then(Value::as_u64))
        && (s.chars().count() as u64) < min
    {
        return Err(format!("{s:?} shorter than {min}"));
    }
    if let Some(obj) = v.as_object() {
        for req in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let key = req.as_str().unwrap_or_default();
            if !obj.contains_key(key) {
                return Err(format!("missing {key}"));
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            // The discriminators first, so a wrong `oneOf` branch fails before
            // it walks the children (else every level multiplies the work).
            let first = ["t", "c"].into_iter().filter(|k| props.contains_key(*k));
            let rest = props
                .keys()
                .map(String::as_str)
                .filter(|k| *k != "t" && *k != "c");
            for key in first.chain(rest) {
                if let Some(child) = obj.get(key) {
                    validate(root, &props[key], child).map_err(|e| format!("{key}: {e}"))?;
                }
            }
        }
    }
    if let Some(items) = v.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64)
            && (items.len() as u64) < min
        {
            return Err(format!("fewer than {min} items"));
        }
        if let Some(sub) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                validate(root, sub, item).map_err(|e| format!("[{i}]: {e}"))?;
            }
        }
    }
    Ok(())
}
