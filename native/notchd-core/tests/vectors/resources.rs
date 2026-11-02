//! `core/Resources.js` against `notchd_core::resources`. A `null` sample or
//! result is `None`.
use super::*;
use notchd_core::resources as r;

fn sample(v: &Value) -> Option<r::CpuTimes> {
    from_json(v)
}

/// JS `Number(x) || 0` for the scalars the cases use.
fn number(v: &Value) -> f64 {
    match v {
        Value::String(t) => t.trim().parse().unwrap_or(0.0),
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => v.as_f64().unwrap_or(0.0),
    }
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    let t = arg(a, 0).as_str().unwrap_or("");
    Some(match f {
        "cpuTimes" => or_null(r::cpu_times(t).map(|x| to_json(&x))),
        "cpuCount" => json!(r::cpu_count(t)),
        "cpuUsage" => or_null(r::cpu_usage(sample(arg(a, 0)), sample(arg(a, 1))).map(n)),
        "memory" => or_null(r::memory(t).map(|x| to_json(&x))),
        "disk" => or_null(r::disk(t).map(|x| to_json(&x))),
        "formatBytes" => json!(r::format_bytes(number(arg(a, 0)))),
        "formatUsage" => json!(r::format_usage(number(arg(a, 0)), number(arg(a, 1)))),
        "nextPressure" => {
            let prev: Option<r::Pressure> = from_json(arg(a, 0));
            let value = opt(a, 1).and_then(Value::as_f64);
            let rule: r::PressureRule = from_json(arg(a, 2));
            to_json(&r::next_pressure(prev, value, &rule))
        }
        _ => return None,
    })
}
