//! The plan-usage rules (ported from `core/Usage.js`, gone with 3.5; its tests
//! live on as `tests/vectors/rules/usage.json` and `tests/fixtures/usage`).
//!
//! The plan limits of the coding agents: how much of each window (5 hours,
//! the week) is used and when it resets. The answers of the endpoints the
//! agents' own CLIs call (loose JSON, so [`serde_json::Value`]) made into
//! one shape, and when to warn. Times are epoch ms and numbers `f64`, like
//! the JS; time comes in as a parameter.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::js;

/// `WINDOW_SESSION`: 5 hours, in seconds.
pub const WINDOW_SESSION: f64 = 18000.0;
/// `WINDOW_WEEK`: 7 days, in seconds.
pub const WINDOW_WEEK: f64 = 604800.0;

/// Where Claude's plan usage comes from (`SOURCES.claude.url`).
pub const CLAUDE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Claude's login file, relative to `CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub const CLAUDE_CREDENTIALS: &str = ".credentials.json";
/// Where Codex's plan usage comes from (`SOURCES.codex.url`).
pub const CODEX_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// Codex's login file, relative to `CODEX_HOME`, else `~/.codex`.
pub const CODEX_CREDENTIALS: &str = "auth.json";

/// A limit (`{ id, label, percent, resetsAt, windowSecs }`). `id` stays the
/// same from one answer to the next, so a warning is given once per window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub id: String,
    pub label: String,
    /// 0 to 100.
    pub percent: f64,
    /// Epoch ms; 0 unknown.
    pub resets_at: f64,
    /// 0 unknown.
    pub window_secs: f64,
}

/// `num`: a JSON value as a finite number, NaN when it is none (null,
/// missing, empty string, not finite).
fn num(v: &Value) -> f64 {
    if v.is_null() || v.as_str() == Some("") {
        return f64::NAN;
    }
    let n = js::number(v);
    if n.is_finite() { n } else { f64::NAN }
}

fn clamp_percent(p: f64) -> f64 {
    p.clamp(0.0, 100.0)
}

/// JS `String(v || "")` for a JSON value: the text of a truthy string,
/// number or boolean, else "".
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) if n.as_f64() != Some(0.0) => js::num_str(n.as_f64().unwrap_or(f64::NAN)),
        Value::Bool(true) => "true".to_string(),
        _ => String::new(),
    }
}

/// Days from 1970-01-01 to a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A cursor over the bytes of an ISO date-time.
struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn eat(&mut self, c: u8) -> bool {
        let hit = self.b.get(self.i) == Some(&c);
        if hit {
            self.i += 1;
        }
        hit
    }

    /// Exactly `n` digits.
    fn digits(&mut self, n: usize) -> Option<i64> {
        let part = self.b.get(self.i..self.i + n)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.i += n;
        std::str::from_utf8(part).ok()?.parse().ok()
    }
}

/// `isoMs`: the epoch ms of an ISO 8601 date-time as `Date.parse` reads it
/// (`2026-11-01T16:10:00.707886+00:00`, `...Z`, date only, a space for the
/// `T`); 0 for anything else. Fractions beyond milliseconds are cut. A
/// date-time without an offset is read as UTC (JS reads it in the local
/// time zone), and the legacy non-ISO formats `Date.parse` guesses at are
/// not read.
fn iso_ms(s: &str) -> f64 {
    parse_iso(s).unwrap_or(0.0)
}

fn parse_iso(s: &str) -> Option<f64> {
    let mut c = Cursor {
        b: s.as_bytes(),
        i: 0,
    };
    let year = if c.b.first().is_some_and(|b| *b == b'+' || *b == b'-') {
        let sign = if c.b[0] == b'-' { -1 } else { 1 };
        c.i = 1;
        let y = c.digits(6)?;
        if sign < 0 && y == 0 {
            return None;
        }
        sign * y
    } else {
        c.digits(4)?
    };
    let (mut month, mut day) = (1, 1);
    if c.eat(b'-') {
        month = c.digits(2)?;
        if c.eat(b'-') {
            day = c.digits(2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return None;
    }
    let (mut h, mut mi, mut sec, mut ms) = (0, 0, 0, 0);
    let mut offset_min = 0;
    if c.i < c.b.len() {
        if !(c.eat(b'T') || c.eat(b't') || c.eat(b' ')) {
            return None;
        }
        h = c.digits(2)?;
        if !c.eat(b':') {
            return None;
        }
        mi = c.digits(2)?;
        if c.eat(b':') {
            sec = c.digits(2)?;
            if c.eat(b'.') {
                let start = c.i;
                while c.b.get(c.i).is_some_and(u8::is_ascii_digit) {
                    c.i += 1;
                }
                if c.i == start {
                    return None;
                }
                let frac = &s[start..c.i];
                ms = format!("{frac:0<3}")[..3].parse().ok()?;
            }
        }
        if h > 24 || mi > 59 || sec > 59 || (h == 24 && (mi, sec, ms) != (0, 0, 0)) {
            return None;
        }
        if c.eat(b'Z') || c.eat(b'z') {
        } else if c.b.get(c.i).is_some_and(|b| *b == b'+' || *b == b'-') {
            let sign = if c.b[c.i] == b'-' { -1 } else { 1 };
            c.i += 1;
            let oh = c.digits(2)?;
            c.eat(b':');
            let om = c.digits(2)?;
            if oh > 23 || om > 59 {
                return None;
            }
            offset_min = sign * (oh * 60 + om);
        }
        if c.i != c.b.len() {
            return None;
        }
    }
    let days = days_from_civil(year, month, day);
    let total = ((days * 24 + h) * 60 + mi - offset_min) * 60 + sec;
    let total_ms = total as f64 * 1000.0 + ms as f64;
    (total_ms.abs() <= 8.64e15).then_some(total_ms)
}

/// A JSON string as epoch ms (`isoMs`); 0 for anything but a date string.
fn iso_value(v: &Value) -> f64 {
    v.as_str().map_or(0.0, iso_ms)
}

/// `windowLabel`: a window's name from its length: "5h", "Week", else in
/// hours or days; "Limit" when unknown.
pub fn window_label(secs: f64) -> String {
    if secs == WINDOW_SESSION {
        "5h".to_string()
    } else if secs == WINDOW_WEEK {
        "Week".to_string()
    } else if secs > 0.0 && secs % 86400.0 == 0.0 {
        format!("{}d", js::num_str(secs / 86400.0))
    } else if secs > 0.0 && secs % 3600.0 == 0.0 {
        format!("{}h", js::num_str(secs / 3600.0))
    } else {
        "Limit".to_string()
    }
}

/// `claudeLimits`: Claude's answer (api/oauth/usage): its `limits` list, the
/// session, the whole week and per-model weeks ("Fable week"); without the
/// list, the `five_hour` and `seven_day` windows (utilization in percent).
/// `None` when it is no answer at all: none of them there.
pub fn claude_limits(data: &Value) -> Option<Vec<Limit>> {
    let obj = data.as_object()?;
    let list = obj.get("limits").and_then(Value::as_array);
    if list.is_none() && !obj.contains_key("five_hour") && !obj.contains_key("seven_day") {
        return None;
    }
    let mut out = Vec::new();
    if let Some(list) = list.filter(|l| !l.is_empty()) {
        for l in list.iter().filter(|l| l.is_object()) {
            let p = num(&l["percent"]);
            if p.is_nan() {
                continue;
            }
            let group = text(&l["group"]);
            let secs = match group.as_str() {
                "session" => WINDOW_SESSION,
                "weekly" => WINDOW_WEEK,
                _ => 0.0,
            };
            let kind = text(&l["kind"]);
            let model = text(&l["scope"]["model"]["display_name"]);
            let mut label = window_label(secs);
            if secs == 0.0 && !kind.is_empty() {
                label = kind.clone();
            }
            if !model.is_empty() {
                label = format!("{model} {}", label.to_lowercase());
            }
            let key = if kind.is_empty() { &group } else { &kind };
            let suffix = if model.is_empty() {
                String::new()
            } else {
                format!(":{model}")
            };
            out.push(Limit {
                id: format!("claude:{key}{suffix}"),
                label,
                percent: clamp_percent(p),
                resets_at: iso_value(&l["resets_at"]),
                window_secs: secs,
            });
        }
        return Some(out);
    }
    for (key, secs) in [("five_hour", WINDOW_SESSION), ("seven_day", WINDOW_WEEK)] {
        let b = &data[key];
        if !b.is_object() {
            continue;
        }
        let p = num(&b["utilization"]);
        if p.is_nan() {
            continue;
        }
        out.push(Limit {
            id: format!("claude:{key}"),
            label: window_label(secs),
            percent: clamp_percent(p),
            resets_at: iso_value(&b["resets_at"]),
            window_secs: secs,
        });
    }
    Some(out)
}

/// `codexLimits`: Codex's answer (backend-api/wham/usage): `rate_limit`'s
/// windows, told apart by their length, not their place (OpenAI has sent the
/// week alone as the primary window). `now` (ms) for a reset given as
/// seconds from now. `None` when it is no answer at all.
pub fn codex_limits(data: &Value, now: f64) -> Option<Vec<Limit>> {
    let rl = data.as_object()?.get("rate_limit")?;
    if rl.is_array() {
        return Some(Vec::new());
    }
    if !rl.is_object() {
        return None;
    }
    let mut out: Vec<Limit> = Vec::new();
    for w in [&rl["primary_window"], &rl["secondary_window"]] {
        if !w.is_object() {
            continue;
        }
        let p = num(&w["used_percent"]);
        let secs = num(&w["limit_window_seconds"]);
        if p.is_nan() || secs.is_nan() || out.iter().any(|l| l.window_secs == secs) {
            continue;
        }
        let reset = num(&w["reset_at"]);
        let after = num(&w["reset_after_seconds"]);
        let resets_at = if !reset.is_nan() {
            reset * 1000.0
        } else if !after.is_nan() {
            now + after * 1000.0
        } else {
            0.0
        };
        out.push(Limit {
            id: format!("codex:{}", js::num_str(secs)),
            label: window_label(secs),
            percent: clamp_percent(p),
            resets_at,
            window_secs: secs,
        });
    }
    out.sort_by(|a, b| a.window_secs.total_cmp(&b.window_secs));
    Some(out)
}

/// `limitsOf`: the limits of `provider` ("claude" or "codex") from its
/// answer; `None` for no answer or another provider.
pub fn limits_of(provider: &str, data: &Value, now: f64) -> Option<Vec<Limit>> {
    match provider {
        "claude" => claude_limits(data),
        "codex" => codex_limits(data, now),
        _ => None,
    }
}

/// The request for a provider's usage: where to send it and the headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRequest {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

/// Why there is no request to send (`{ error, expired? }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestError {
    pub error: String,
    /// The Claude login is past its expiry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expired: bool,
}

fn not_signed_in() -> RequestError {
    RequestError {
        error: "not signed in".to_string(),
        expired: false,
    }
}

/// `request`: the request for `provider`'s usage, from the text of its
/// credentials file, or the error to show. The token goes in a header only,
/// and nothing of it is kept. `now` (ms): an expired Claude login isn't
/// sent; the agent renews it the next time it runs.
pub fn request(
    provider: &str,
    credentials_text: &str,
    now: f64,
) -> Result<UsageRequest, RequestError> {
    let url = match provider {
        "claude" => CLAUDE_URL,
        "codex" => CODEX_URL,
        _ => {
            return Err(RequestError {
                error: format!("no usage for {provider}"),
                expired: false,
            });
        }
    };
    let data: Value = serde_json::from_str(credentials_text).map_err(|_| not_signed_in())?;
    if !data.is_object() && !data.is_array() {
        return Err(not_signed_in());
    }
    let mut headers = BTreeMap::new();
    if provider == "claude" {
        let o = &data["claudeAiOauth"];
        let token = text(&o["accessToken"]);
        if token.is_empty() {
            return Err(not_signed_in());
        }
        let exp = num(&o["expiresAt"]);
        if !exp.is_nan() && exp <= now {
            return Err(RequestError {
                error: "login expired".to_string(),
                expired: true,
            });
        }
        headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        headers.insert("anthropic-beta".to_string(), "oauth-2025-04-20".to_string());
    } else {
        let t = &data["tokens"];
        let token = text(&t["access_token"]);
        if token.is_empty() {
            return Err(not_signed_in());
        }
        headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        let account = text(&t["account_id"]);
        if !account.is_empty() {
            headers.insert("ChatGPT-Account-Id".to_string(), account);
        }
    }
    Ok(UsageRequest {
        url: url.to_string(),
        headers,
    })
}

/// What an HTTP status means for the next try (`{ problem, stale, retryMs }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterStatus {
    pub problem: String,
    pub stale: bool,
    pub retry_ms: f64,
}

/// `afterStatus`: 200 is nothing wrong, again in `interval`; 401/403 the
/// login lapsed or was revoked; 429 is left alone for `Retry-After` seconds
/// (`retry_after`, "" when absent), at least 5 minutes; anything else (0 is
/// no answer: offline, DNS) keeps the last limits, marked stale. `interval`
/// is at least a minute and 5 minutes when it is no number.
pub fn after_status(status: i64, retry_after: &str, interval: f64) -> AfterStatus {
    let interval = if interval.is_nan() || interval == 0.0 {
        300_000.0
    } else {
        interval
    }
    .max(60_000.0);
    let stale = |problem: &str, retry_ms: f64| AfterStatus {
        problem: problem.to_string(),
        stale: true,
        retry_ms,
    };
    match status {
        200 => AfterStatus {
            problem: String::new(),
            stale: false,
            retry_ms: interval,
        },
        401 | 403 => stale("login expired", interval),
        429 => {
            let ra = num(&Value::String(retry_after.to_string()));
            stale(
                "rate limited",
                (if ra.is_nan() { 0.0 } else { ra * 1000.0 }).max(300_000.0),
            )
        }
        0 => stale("offline", interval),
        _ => stale(&format!("HTTP {status}"), interval),
    }
}

/// `level`: the warning a limit is due: 100 at the limit, `threshold` past
/// it, else 0.
pub fn level(limit: &Limit, threshold: f64) -> f64 {
    let t = if threshold.is_nan() { 0.0 } else { threshold };
    if limit.percent >= 100.0 {
        100.0
    } else if t > 0.0 && t < 100.0 && limit.percent >= t {
        t
    } else {
        0.0
    }
}

/// A limit to warn about, with the `level` it reached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub level: f64,
    #[serde(flatten)]
    pub limit: Limit,
}

/// The result of [`alerts`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alerts {
    pub alerts: Vec<Alert>,
    /// What was said, per `<id>@<resetsAt>`: only the current windows'.
    pub warned: BTreeMap<String, f64>,
}

/// `alerts`: which limits to warn about now. `warned` (`"<id>@<resetsAt>"`
/// to level) holds what was already said in each window; a new window (a
/// new reset time) starts over. `None` is the first answer: what it already
/// shows is taken as seen, so a restart doesn't warn again.
pub fn alerts(limits: &[Limit], warned: Option<&BTreeMap<String, f64>>, threshold: f64) -> Alerts {
    let mut next = BTreeMap::new();
    let mut out = Vec::new();
    for l in limits {
        let key = format!("{}@{}", l.id, js::num_str(l.resets_at));
        let had = warned.map_or(0.0, |w| {
            w.get(&key).copied().filter(|v| !v.is_nan()).unwrap_or(0.0)
        });
        let lv = level(l, threshold);
        next.insert(key, had.max(lv));
        if warned.is_some() && lv > had {
            out.push(Alert {
                level: lv,
                limit: l.clone(),
            });
        }
    }
    Alerts {
        alerts: out,
        warned: next,
    }
}

/// `resetIn`: time until `resets_at` (ms), short: "45m", "3h 20m", "2d 4h";
/// "" unknown (0).
pub fn reset_in(resets_at: f64, now: f64) -> String {
    if resets_at == 0.0 || resets_at.is_nan() {
        return String::new();
    }
    let mins = ((resets_at - now) / 60000.0 + 0.5).floor().max(0.0) as i64;
    if mins < 60 {
        return format!("{mins}m");
    }
    let h = mins / 60;
    if h < 24 {
        return if mins % 60 != 0 {
            format!("{h}h {}m", mins % 60)
        } else {
            format!("{h}h")
        };
    }
    let d = h / 24;
    if h % 24 != 0 {
        format!("{d}d {}h", h % 24)
    } else {
        format!("{d}d")
    }
}

/// `tightest`: the limit to show first: the most used, the shorter window on
/// a tie.
pub fn tightest(limits: &[Limit]) -> Option<&Limit> {
    let mut best: Option<&Limit> = None;
    for l in limits {
        if best.is_none_or(|b| {
            l.percent > b.percent || (l.percent == b.percent && l.window_secs < b.window_secs)
        }) {
            best = Some(l);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Real answers (2026-11-01), the account's fields taken out.
    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/../../tests/fixtures/usage/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn limit(percent: f64, window_secs: f64) -> Limit {
        Limit {
            id: String::new(),
            label: String::new(),
            percent,
            resets_at: 0.0,
            window_secs,
        }
    }

    // claudeLimits: the session, the week and a model's week, from the limits list
    #[test]
    fn claude_limits_the_session_the_week_and_a_models_week_from_the_limits_list() {
        let limits = claude_limits(&fixture("claude")).unwrap();
        let shown: Vec<(&str, f64, f64)> = limits
            .iter()
            .map(|l| (l.label.as_str(), l.percent, l.window_secs))
            .collect();
        assert_eq!(
            shown,
            [
                ("5h", 8.0, 18000.0),
                ("Week", 2.0, 604800.0),
                ("Fable week", 0.0, 604800.0)
            ]
        );
        assert_eq!(limits[0].resets_at, 1_793_549_400_707.0); // Date.parse('2026-11-01T16:10:00.707886+00:00')
        // Ids stay put between answers: the reset time is not in them.
        let ids: Vec<&str> = limits.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "claude:session",
                "claude:weekly_all",
                "claude:weekly_scoped:Fable"
            ]
        );
    }

    // claudeLimits: without the list, five_hour and seven_day
    #[test]
    fn claude_limits_without_the_list_five_hour_and_seven_day() {
        let mut data = fixture("claude");
        data.as_object_mut().unwrap().shift_remove("limits");
        let shown: Vec<(String, String, f64)> = claude_limits(&data)
            .unwrap()
            .into_iter()
            .map(|l| (l.id, l.label, l.percent))
            .collect();
        assert_eq!(
            shown,
            [
                ("claude:five_hour".to_string(), "5h".to_string(), 8.0),
                ("claude:seven_day".to_string(), "Week".to_string(), 2.0)
            ]
        );
        assert_eq!(claude_limits(&Value::Null), None);
        assert_eq!(claude_limits(&json!([])), None);
        assert_eq!(
            claude_limits(&json!({ "nope": 1 })),
            None,
            "an answer without limits was taken"
        );
        assert_eq!(
            claude_limits(&json!({ "five_hour": null, "seven_day": null })),
            Some(vec![]),
            "a plan without windows"
        );
        assert_eq!(
            claude_limits(&json!({ "limits": [{ "group": "session", "percent": "x" }, null] })),
            Some(vec![])
        );
        assert_eq!(
            claude_limits(&json!({ "limits": [{ "group": "session", "percent": 140 }] })).unwrap()
                [0]
            .percent,
            100.0
        );
    }

    // codexLimits: windows by their length, not their place
    // (The week came alone as the primary window: told apart by its length.)
    #[test]
    fn codex_limits_windows_by_their_length_not_their_place() {
        let limits = codex_limits(&fixture("codex"), 0.0).unwrap();
        assert_eq!(
            limits,
            [Limit {
                id: "codex:604800".into(),
                label: "Week".into(),
                percent: 1.0,
                resets_at: 1_793_794_381_000.0,
                window_secs: 604800.0
            }]
        );
        let both = json!({ "rate_limit": {
            "primary_window": { "used_percent": 30, "limit_window_seconds": 604800, "reset_at": 2000 },
            "secondary_window": { "used_percent": 50, "limit_window_seconds": 18000, "reset_after_seconds": 60 } } });
        let shown: Vec<(String, f64, f64)> = codex_limits(&both, 1000.0)
            .unwrap()
            .into_iter()
            .map(|l| (l.label, l.percent, l.resets_at))
            .collect();
        assert_eq!(
            shown,
            [
                ("5h".to_string(), 50.0, 61000.0),
                ("Week".to_string(), 30.0, 2_000_000.0)
            ]
        );
        assert_eq!(codex_limits(&json!({}), 0.0), None);
        assert_eq!(
            codex_limits(&json!({ "rate_limit": { "primary_window": null } }), 0.0),
            Some(vec![])
        );
    }

    // request: the token in a header; an expired Claude login is not sent
    #[test]
    fn request_the_token_in_a_header_an_expired_claude_login_is_not_sent() {
        let claude =
            json!({ "claudeAiOauth": { "accessToken": "AT", "expiresAt": 5000 } }).to_string();
        let r = request("claude", &claude, 1000.0).unwrap();
        assert_eq!(r.url, "https://api.anthropic.com/api/oauth/usage");
        assert_eq!(r.headers["Authorization"], "Bearer AT");
        assert_eq!(r.headers["anthropic-beta"], "oauth-2025-04-20");
        assert_eq!(
            request("claude", &claude, 6000.0),
            Err(RequestError {
                error: "login expired".into(),
                expired: true
            })
        );
        let codex = json!({ "tokens": { "access_token": "CT", "account_id": "acc" } }).to_string();
        let headers = request("codex", &codex, 0.0).unwrap().headers;
        assert_eq!(
            headers,
            BTreeMap::from([
                ("Authorization".to_string(), "Bearer CT".to_string()),
                ("ChatGPT-Account-Id".to_string(), "acc".to_string())
            ])
        );
        for text in ["", "{", "{}", r#"{"tokens":{}}"#] {
            assert_eq!(
                request("codex", text, 0.0).unwrap_err().error,
                "not signed in",
                "{text}"
            );
        }
        assert_eq!(
            request("claude", r#"{"claudeAiOauth":{}}"#, 0.0)
                .unwrap_err()
                .error,
            "not signed in"
        );
        assert!(
            request("gemini", "{}", 0.0)
                .unwrap_err()
                .error
                .contains("no usage")
        );
    }

    // afterStatus: 429 waits at least 5 minutes; failures keep the limits, stale
    #[test]
    fn after_status_429_waits_at_least_5_minutes_failures_keep_the_limits_stale() {
        let at = |problem: &str, stale, retry_ms| AfterStatus {
            problem: problem.into(),
            stale,
            retry_ms,
        };
        assert_eq!(after_status(200, "", 300_000.0), at("", false, 300_000.0));
        assert_eq!(
            after_status(429, "", 300_000.0),
            at("rate limited", true, 300_000.0)
        );
        assert_eq!(after_status(429, "900", 60_000.0).retry_ms, 900_000.0);
        assert_eq!(after_status(401, "", 300_000.0).problem, "login expired");
        assert_eq!(after_status(0, "", 300_000.0).problem, "offline");
        assert_eq!(after_status(500, "", 300_000.0).problem, "HTTP 500");
        assert_eq!(
            after_status(200, "", 1000.0).retry_ms,
            60_000.0,
            "never more than once a minute"
        );
    }

    // alerts: once per window at the threshold, again at the limit; a new window starts over
    #[test]
    fn alerts_once_per_window_at_the_threshold_again_at_the_limit_a_new_window_starts_over() {
        let at = |percent, resets_at| {
            vec![Limit {
                id: "claude:session".into(),
                label: "5h".into(),
                percent,
                resets_at,
                window_secs: 18000.0,
            }]
        };
        let levels = |r: &Alerts| r.alerts.iter().map(|a| a.level).collect::<Vec<_>>();
        // The first answer is taken as seen: a restart doesn't warn again.
        let mut r = alerts(&at(85.0, 1.0), None, 80.0);
        assert_eq!(r.alerts.len(), 0);
        r = alerts(&at(70.0, 1.0), Some(&BTreeMap::new()), 80.0);
        assert_eq!(r.alerts.len(), 0);
        r = alerts(&at(82.0, 1.0), Some(&r.warned), 80.0);
        assert_eq!(levels(&r), [80.0]);
        r = alerts(&at(90.0, 1.0), Some(&r.warned), 80.0);
        assert_eq!(r.alerts.len(), 0, "warned twice in one window");
        r = alerts(&at(100.0, 1.0), Some(&r.warned), 80.0);
        assert_eq!(levels(&r), [100.0]);
        r = alerts(&at(81.0, 2.0), Some(&r.warned), 80.0);
        assert_eq!(levels(&r), [80.0], "the next window did not warn");
        assert_eq!(
            r.warned.keys().collect::<Vec<_>>(),
            ["claude:session@2"],
            "old windows kept"
        );
        // 100: only at the limit.
        let empty = BTreeMap::new();
        assert_eq!(alerts(&at(95.0, 1.0), Some(&empty), 100.0).alerts.len(), 0);
        assert_eq!(alerts(&at(100.0, 1.0), Some(&empty), 100.0).alerts.len(), 1);
    }

    // resetIn and tightest
    #[test]
    fn reset_in_and_tightest() {
        assert_eq!(reset_in(0.0, 0.0), "");
        assert_eq!(reset_in(45.0 * 60000.0, 0.0), "45m");
        assert_eq!(reset_in(200.0 * 60000.0, 0.0), "3h 20m");
        assert_eq!(reset_in(180.0 * 60000.0, 0.0), "3h");
        assert_eq!(reset_in((2.0 * 24.0 + 4.0) * 3_600_000.0, 0.0), "2d 4h");
        assert_eq!(reset_in(-5.0, 0.0), "0m");
        let limits = [
            limit(8.0, 604800.0),
            limit(8.0, 18000.0),
            limit(2.0, 604800.0),
        ];
        assert_eq!(tightest(&limits).unwrap().window_secs, 18000.0);
        assert_eq!(tightest(&[]), None);
    }

    // Not in the Node tests: the ISO reader stands in for Date.parse.
    #[test]
    fn iso_ms_reads_what_date_parse_reads() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), 0.0);
        assert_eq!(iso_ms("1970-01-01T00:00:01.5Z"), 1500.0);
        assert_eq!(iso_ms("1970-01-02"), 86_400_000.0);
        assert_eq!(iso_ms("1970-01-01T01:00:00+01:00"), 0.0);
        assert_eq!(iso_ms("2024-02-29T00:00:00Z"), 1_709_164_800_000.0);
        assert_eq!(iso_ms("2023-02-29T00:00:00Z"), 0.0);
        assert_eq!(iso_ms(""), 0.0);
        assert_eq!(iso_ms("soon"), 0.0);
    }
}
