//! The JavaScript semantics the ported rules lean on, once: `Number(x)`,
//! `String(x)`, truthiness, `===`, `trim`, `Math.round`, `toFixed` and
//! `decodeURIComponent`. Each is the faithful behaviour for the JSON values
//! the rules see, not a Rust idiom.

use serde_json::Value;

/// JS `\s` / `String.prototype.trim` whitespace (differs from Rust's
/// `char::is_whitespace`: U+FEFF counts, U+0085 does not).
pub(crate) fn is_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// JS `String.prototype.trim`.
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_ws)
}

/// JS `Number(string)`: empty is 0, hex/octal/binary prefixes, decimal and
/// exponent forms, `Infinity`; anything else is NaN.
pub(crate) fn str_to_number(s: &str) -> f64 {
    let t = trim(s);
    if t.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return u64::from_str_radix(rest, radix).map_or(f64::NAN, |n| n as f64);
        }
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust also parses "inf" and "nan"; JS does not.
    if t.chars().any(|c| c.is_alphabetic() && c != 'e' && c != 'E') {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// JS `Number(value)` for a JSON value (arrays and objects are NaN here).
pub(crate) fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => str_to_number(s),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Null => 0.0,
        _ => f64::NAN,
    }
}

/// JS `Number(value) || 0`.
pub(crate) fn number_or_zero(v: &Value) -> f64 {
    let n = number(v);
    if n.is_nan() { 0.0 } else { n }
}

/// JS truthiness of a JSON value.
pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_none_or(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JS truthiness of an optional value (`undefined` is falsy).
pub(crate) fn truthy_opt(v: Option<&Value>) -> bool {
    v.is_some_and(truthy)
}

/// A number as JS prints it: integers without a fraction, an exponent from
/// 1e21 and below 1e-6, `-0` as "0".
pub(crate) fn num_str(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_string()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else if n == 0.0 {
        "0".to_string()
    } else if n.abs() >= 1e21 || n.abs() < 1e-6 {
        let s = format!("{n:e}");
        match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        }
    } else if n.abs() >= 1e15 {
        // Whole and large: the shortest digits that round-trip, padded with
        // zeros (not the exact value, which `{:.0}` would print).
        let s = format!("{:e}", n.abs());
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let digits = m.replace('.', "");
        let len = e.parse::<usize>().unwrap_or(0) + 1;
        let sign = if n < 0.0 { "-" } else { "" };
        format!("{sign}{digits:0<len$}")
    } else if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

/// JS `String(value)` for a JSON value.
pub(crate) fn string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => num_str(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                if x.is_null() {
                    String::new()
                } else {
                    string(x)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// JS `String(v || "")`.
pub(crate) fn string_or_empty(v: &Value) -> String {
    if truthy(v) { string(v) } else { String::new() }
}

/// JS `===` between two values (numbers compare as numbers, so 1 equals 1.0).
pub(crate) fn eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// JS `===` between two possibly-`undefined` values.
pub(crate) fn eq_opt(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => eq(x, y),
        _ => false,
    }
}

/// JS `Math.round`: halves round toward +infinity.
pub(crate) fn round(x: f64) -> f64 {
    let r = x.floor();
    if x - r >= 0.5 { r + 1.0 } else { r }
}

/// JS `Number.prototype.toFixed`: rounds half up (away from zero) on the
/// exact decimal value; Rust's formatting rounds ties to even.
pub(crate) fn to_fixed(v: f64, digits: usize) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v < 0.0 {
        return format!("-{}", to_fixed(-v, digits));
    }
    // The exact expansion of a double, long enough to decide any tie.
    let exact = format!("{v:.80}");
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let round_up = frac.as_bytes().get(digits).is_some_and(|d| *d >= b'5');
    let kept = format!("{int}{}", &frac[..digits]);
    let Ok(n) = kept.parse::<u128>() else {
        return format!("{v:.digits$}");
    };
    let s = (n + u128::from(round_up)).to_string();
    if digits == 0 {
        return s;
    }
    let s = format!("{s:0>width$}", width = digits + 1);
    format!("{}.{}", &s[..s.len() - digits], &s[s.len() - digits..])
}

/// JS `decodeURIComponent`, or `None` where JS throws (a bad `%` escape, or
/// bytes that are not UTF-8).
pub(crate) fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whitespace_is_the_js_set() {
        assert!(is_ws('\u{feff}') && is_ws('\u{a0}') && is_ws('\u{2028}'));
        assert!(!is_ws('\u{85}') && !is_ws('\u{180e}') && !is_ws('a'));
        assert_eq!(trim("\u{feff} a b\u{3000}\n"), "a b");
        assert_eq!(trim("\u{85}x"), "\u{85}x");
    }

    #[test]
    fn number_of_a_string_like_js() {
        assert_eq!(str_to_number(""), 0.0);
        assert_eq!(str_to_number("  "), 0.0);
        assert_eq!(str_to_number(" 12 "), 12.0);
        assert_eq!(str_to_number("1e3"), 1000.0);
        assert_eq!(str_to_number(".5"), 0.5);
        assert_eq!(str_to_number("-1.5"), -1.5);
        assert_eq!(str_to_number("0x1f"), 31.0);
        assert_eq!(str_to_number("0b101"), 5.0);
        assert_eq!(str_to_number("0o17"), 15.0);
        assert_eq!(str_to_number("Infinity"), f64::INFINITY);
        assert_eq!(str_to_number("-Infinity"), f64::NEG_INFINITY);
        for bad in ["abc", "1px", "inf", "nan", "NaN", "-0x10", "1 2", "0x"] {
            assert!(str_to_number(bad).is_nan(), "{bad}");
        }
    }

    #[test]
    fn number_of_a_value_like_js() {
        assert_eq!(number(&json!(null)), 0.0);
        assert_eq!(number(&json!(true)), 1.0);
        assert_eq!(number(&json!("7")), 7.0);
        assert_eq!(number(&json!(2.5)), 2.5);
        assert!(number(&json!({})).is_nan());
        assert_eq!(number_or_zero(&json!("x")), 0.0);
        assert_eq!(number_or_zero(&json!("3")), 3.0);
        assert_eq!(number_or_zero(&json!([1])), 0.0);
    }

    #[test]
    fn truthiness_like_js() {
        for falsy in [json!(null), json!(false), json!(0), json!(0.0), json!("")] {
            assert!(!truthy(&falsy), "{falsy}");
        }
        for t in [
            json!(true),
            json!(1),
            json!(-1),
            json!("0"),
            json!([]),
            json!({}),
        ] {
            assert!(truthy(&t), "{t}");
        }
        assert!(!truthy_opt(None));
        assert!(truthy_opt(Some(&json!("a"))));
    }

    #[test]
    fn numbers_print_like_js() {
        assert_eq!(num_str(5.0), "5");
        assert_eq!(num_str(-0.0), "0");
        assert_eq!(num_str(1.5), "1.5");
        assert_eq!(num_str(0.1), "0.1");
        assert_eq!(num_str(1e21), "1e+21");
        assert_eq!(num_str(1.5e-7), "1.5e-7");
        assert_eq!(num_str(123456789012345680000.0), "123456789012345680000");
        assert_eq!(num_str(f64::NAN), "NaN");
        assert_eq!(num_str(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn string_of_a_value_like_js() {
        assert_eq!(string(&json!(null)), "null");
        assert_eq!(string(&json!(true)), "true");
        assert_eq!(string(&json!(3)), "3");
        assert_eq!(string(&json!(3.0)), "3");
        assert_eq!(string(&json!(0.5)), "0.5");
        assert_eq!(string(&json!([1, null, "a", [2, 3]])), "1,,a,2,3");
        assert_eq!(string(&json!({"a": 1})), "[object Object]");
        assert_eq!(string_or_empty(&json!(0)), "");
        assert_eq!(string_or_empty(&json!(null)), "");
        assert_eq!(string_or_empty(&json!(4)), "4");
    }

    #[test]
    fn strict_equality_like_js() {
        assert!(eq(&json!(1), &json!(1.0)));
        assert!(!eq(&json!(1), &json!("1")));
        assert!(eq(&json!("a"), &json!("a")));
        assert!(eq_opt(None, None));
        assert!(!eq_opt(None, Some(&json!(null))));
        assert!(eq_opt(Some(&json!(2)), Some(&json!(2.0))));
    }

    #[test]
    fn math_round_halves_go_up() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(-0.4), 0.0);
        assert_eq!(round(1.4999), 1.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert!(round(f64::NAN).is_nan());
        assert_eq!(round(f64::INFINITY), f64::INFINITY);
    }

    #[test]
    fn to_fixed_rounds_like_js() {
        assert_eq!(to_fixed(1.005, 2), "1.00"); // the double is below 1.005
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(0.5, 0), "1");
        assert_eq!(to_fixed(1.45, 1), "1.4"); // the double is below 1.45
        assert_eq!(to_fixed(7.75, 1), "7.8");
        assert_eq!(to_fixed(0.0, 2), "0.00");
        assert_eq!(to_fixed(-1.5, 0), "-2");
        assert_eq!(to_fixed(f64::NAN, 1), "NaN");
    }

    #[test]
    fn decode_uri_component_like_js() {
        assert_eq!(decode_uri_component("a%20b%C3%A9").as_deref(), Some("a bé"));
        assert_eq!(decode_uri_component("plain+x").as_deref(), Some("plain+x"));
        assert_eq!(decode_uri_component("%"), None);
        assert_eq!(decode_uri_component("%zz"), None);
        assert_eq!(decode_uri_component("%+f"), None);
        assert_eq!(decode_uri_component("%FF"), None);
    }
}
