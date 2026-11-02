//! An agent's plan usage: how much of each window (5 hours, the week) is
//! used, from the endpoint the agent's CLI calls, with the login the CLI
//! keeps (docs/modules.md, Agents). Ported from `core/AgentUsage.qml`; the
//! rules are `notchd-core::usage`. This is the state and its timing as plain
//! values: the module says when it wants an answer ([`Plan::tick`] returns
//! the request), the host asks the `usage` provider on a thread of its own
//! and gives the answer back ([`Plan::answer`]). The login is only read,
//! never refreshed or written: when it lapses, the last limits stay, marked
//! stale, until the agent renews it by running.
//!
//! It asks every `interval`, when [`Plan::refresh`] is called (an agent that
//! just finished), and after a 429 not before the time it was given. An
//! alert is returned once per window as a limit passes the threshold, and at
//! 100 % ([`notchd_core::usage::alerts`]).

use notchd_core::usage::{self, Alert, Limit};
use serde_json::Value;
use std::collections::BTreeMap;

/// How often it asks.
pub const INTERVAL_MS: u64 = 300_000;
/// Not within this of the last answer, for a refresh that is not forced.
const MIN_GAP_MS: u64 = 60_000;
/// A request that never came back is given up after this.
const GIVE_UP_MS: u64 = 30_000;
/// While there is no (valid) login, how often to look at its file again.
const LOGIN_RECHECK_MS: u64 = 5_000;

/// What the module wants asked: the provider, and, while the login is
/// lapsed, the time its file had then: the host asks the network only once
/// the file has changed (the agent renewed it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ask {
    pub provider: &'static str,
    pub unless_login_is: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Plan {
    /// `"claude"` or `"codex"`; `None` asks nothing.
    pub provider: Option<&'static str>,
    pub enabled: bool,
    pub threshold: f64,
    pub limits: Vec<Limit>,
    /// The limits are from an earlier answer: this one failed.
    pub stale: bool,
    pub problem: String,
    pub fetched_at: u64,
    warned: Option<BTreeMap<String, f64>>,
    busy_since: Option<u64>,
    /// Not before this (ms): after a 429.
    not_before: u64,
    /// The next periodic ask.
    next_at: Option<u64>,
    /// When the login file was last changed, as the last answer saw it (0:
    /// none).
    creds_mtime: u64,
}

impl Plan {
    pub fn new(provider: Option<&'static str>) -> Plan {
        Plan {
            provider,
            enabled: false,
            threshold: 80.0,
            ..Plan::default()
        }
    }

    pub fn active(&self) -> bool {
        self.enabled && self.provider.is_some()
    }

    /// There are limits to show.
    pub fn shows(&self) -> bool {
        self.active() && !self.limits.is_empty()
    }

    pub fn tightest(&self) -> Option<&Limit> {
        usage::tightest(&self.limits)
    }

    /// The settings changed (`usage`, `usageWarn`).
    pub fn configure(&mut self, enabled: bool, threshold: f64, now: u64) {
        let was = self.active();
        self.enabled = enabled;
        self.threshold = threshold;
        if self.active() && !was {
            // Switched on (or loaded): ask at once.
            self.next_at = Some(now);
        } else if !self.active() {
            self.next_at = None;
        }
    }

    /// When it wants [`Plan::tick`].
    pub fn next_due(&self) -> Option<u64> {
        if !self.active() {
            return None;
        }
        self.next_at
    }

    /// The periodic ask, or the one a lapsed login waits for. `Some` is the
    /// provider to ask now.
    pub fn tick(&mut self, now: u64) -> Option<Ask> {
        let due = self.next_at?;
        if now < due {
            return None;
        }
        self.next_at = Some(now + INTERVAL_MS);
        self.ask(now, true)
    }

    fn login_lapsed(&self) -> bool {
        self.problem == "login expired" || self.problem == "not signed in"
    }

    /// Asks now; not forced (an agent that finished a task), not within a
    /// minute of the last answer.
    pub fn refresh(&mut self, now: u64, force: bool) -> Option<Ask> {
        self.ask(now, force)
    }

    fn ask(&mut self, now: u64, force: bool) -> Option<Ask> {
        if !self.active() {
            return None;
        }
        if let Some(since) = self.busy_since {
            if now < since + GIVE_UP_MS {
                return None;
            }
            self.busy_since = None;
        }
        if now < self.not_before {
            return None;
        }
        if !force && now < self.fetched_at + MIN_GAP_MS {
            return None;
        }
        self.busy_since = Some(now);
        Some(Ask {
            provider: self.provider?,
            unless_login_is: self.login_lapsed().then_some(self.creds_mtime),
        })
    }

    /// What the provider found, as the host feeds it back: `{status, body,
    /// retry_after}` for an answer, `{error, expired}` when there was no login
    /// to ask with. The alerts due.
    pub fn answer(&mut self, data: &Value, now: u64) -> Vec<Alert> {
        self.busy_since = None;
        if let Some(m) = data.get("login_mtime").and_then(Value::as_u64) {
            self.creds_mtime = m;
        }
        if data.get("unchanged") == Some(&Value::Bool(true)) {
            self.next_at = Some(now + LOGIN_RECHECK_MS);
            return Vec::new();
        }
        if let Some(error) = data.get("error").and_then(Value::as_str) {
            self.problem = error.to_string();
            self.stale = !self.limits.is_empty();
            // The login is looked at again soon: the agent may renew it.
            self.next_at = Some(now + LOGIN_RECHECK_MS);
            return Vec::new();
        }
        let status = data.get("status").and_then(Value::as_i64).unwrap_or(0);
        let body = data.get("body").and_then(Value::as_str).unwrap_or("");
        let retry_after = data
            .get("retry_after")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut next = usage::after_status(status, retry_after, INTERVAL_MS as f64);
        if status == 429 {
            self.not_before = now + next.retry_ms as u64;
        }
        let mut alerts = Vec::new();
        if status == 200 {
            match serde_json::from_str::<Value>(body)
                .ok()
                .and_then(|d| usage::limits_of(self.provider.unwrap_or(""), &d, now as f64))
            {
                Some(limits) => alerts = self.apply(limits, now),
                None => {
                    next = usage::AfterStatus {
                        problem: "unexpected answer".into(),
                        stale: true,
                        retry_ms: INTERVAL_MS as f64,
                    }
                }
            }
        }
        self.problem = next.problem;
        self.stale = next.stale && !self.limits.is_empty();
        // After a login lapse the file is looked at soon: a renewed login is
        // picked up without waiting five minutes. (A 429 waits: `not_before`.)
        self.next_at = Some(
            now + if self.login_lapsed() {
                LOGIN_RECHECK_MS
            } else {
                INTERVAL_MS
            },
        );
        alerts
    }

    /// An answer (the endpoint's JSON, already read into limits): the
    /// warnings due.
    pub fn apply(&mut self, limits: Vec<Limit>, now: u64) -> Vec<Alert> {
        let r = usage::alerts(&limits, self.warned.as_ref(), self.threshold);
        self.limits = limits;
        self.fetched_at = now;
        self.problem.clear();
        self.stale = false;
        self.warned = Some(r.warned);
        r.alerts
    }

    /// The endpoint's JSON by hand (a test hook): its limits, or `None` when
    /// it is not an answer.
    pub fn apply_json(&mut self, data: &Value, now: u64) -> Option<Vec<Alert>> {
        let limits = usage::limits_of(self.provider?, data, now as f64)?;
        Some(self.apply(limits, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claude(percent: f64) -> Value {
        json!({"five_hour": {"utilization": percent, "resets_at": "2099-01-01T00:00:00Z"},
               "seven_day": {"utilization": 10.0, "resets_at": "2099-01-02T00:00:00Z"}})
    }

    fn ask(provider: &'static str) -> Ask {
        Ask {
            provider,
            unless_login_is: None,
        }
    }

    fn ok(body: &Value) -> Value {
        json!({"status": 200, "body": body.to_string(), "retry_after": ""})
    }

    #[test]
    fn it_asks_when_loaded_then_every_five_minutes_and_not_while_waiting() {
        let mut p = Plan::new(Some("claude"));
        p.configure(true, 80.0, 1000);
        assert_eq!(p.next_due(), Some(1000));
        assert_eq!(p.tick(500), None);
        assert_eq!(p.tick(1000), Some(ask("claude")));
        // Waiting for the answer: nothing else is sent.
        assert_eq!(p.refresh(1100, true), None);
        p.answer(&ok(&claude(10.0)), 1500);
        assert_eq!(p.next_due(), Some(1500 + INTERVAL_MS));
        assert!(p.shows() && !p.stale && p.problem.is_empty());
        // An agent that finished asks again, not within a minute of the last.
        assert_eq!(p.refresh(30_000, false), None);
        assert_eq!(p.refresh(62_000, false), Some(ask("claude")));
    }

    #[test]
    fn nothing_is_asked_without_a_provider_or_when_switched_off() {
        let mut p = Plan::new(None);
        p.configure(true, 80.0, 0);
        assert_eq!(
            (p.next_due(), p.tick(10), p.refresh(10, true)),
            (None, None, None)
        );
        let mut p = Plan::new(Some("codex"));
        p.configure(false, 80.0, 0);
        assert_eq!(p.next_due(), None);
        assert_eq!(p.refresh(10, true), None);
        p.configure(true, 80.0, 5);
        assert_eq!(p.next_due(), Some(5));
    }

    #[test]
    fn a_warning_comes_once_per_window_and_the_first_answer_is_taken_as_seen() {
        let mut p = Plan::new(Some("claude"));
        p.configure(true, 80.0, 0);
        assert!(
            p.answer(&ok(&claude(10.0)), 1).is_empty(),
            "the first answer warns of nothing"
        );
        // 85 % at the first answer is taken as seen: a restart does not warn again.
        let mut q = Plan::new(Some("claude"));
        q.configure(true, 80.0, 0);
        assert!(q.answer(&ok(&claude(85.0)), 1).is_empty());
        assert!(
            q.answer(&ok(&claude(90.0)), 2).is_empty(),
            "already warned in this window"
        );
        let a = p.answer(&ok(&claude(81.0)), 3);
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].level, a[0].limit.label.as_str()), (80.0, "5h"));
        assert!(
            p.answer(&ok(&claude(90.0)), 4).is_empty(),
            "once per window"
        );
        let a = p.answer(&ok(&claude(100.0)), 5);
        assert_eq!(a[0].level, 100.0);
    }

    #[test]
    fn a_failure_keeps_the_last_limits_marked_stale_and_says_why() {
        let mut p = Plan::new(Some("claude"));
        p.configure(true, 80.0, 0);
        p.answer(&json!({"status": 500, "body": "", "retry_after": ""}), 1);
        assert_eq!(
            (p.stale, p.problem.as_str()),
            (false, "HTTP 500"),
            "no limits to be stale"
        );
        p.answer(&ok(&claude(10.0)), 2);
        p.answer(&json!({"status": 0, "body": "", "retry_after": ""}), 3);
        assert_eq!((p.stale, p.problem.as_str()), (true, "offline"));
        assert_eq!(p.limits.len(), 2);
        p.answer(
            &json!({"status": 200, "body": "{\"nope\":1}", "retry_after": ""}),
            4,
        );
        assert_eq!((p.stale, p.problem.as_str()), (true, "unexpected answer"));
        p.answer(&ok(&claude(10.0)), 5);
        assert!(!p.stale && p.problem.is_empty());
    }

    #[test]
    fn a_429_waits_for_retry_after_and_at_least_five_minutes() {
        let mut p = Plan::new(Some("claude"));
        p.configure(true, 80.0, 0);
        p.tick(0);
        p.answer(
            &json!({"status": 429, "body": "", "retry_after": "900"}),
            1000,
        );
        assert_eq!(p.problem, "rate limited");
        assert_eq!(p.refresh(1000 + 899_000, true), None);
        assert_eq!(p.refresh(1000 + 900_000, true), Some(ask("claude")));
        let mut q = Plan::new(Some("claude"));
        q.configure(true, 80.0, 0);
        q.tick(0);
        q.answer(&json!({"status": 429, "body": "", "retry_after": ""}), 0);
        assert_eq!(q.refresh(299_000, true), None);
        assert_eq!(q.refresh(300_000, true), Some(ask("claude")));
    }

    #[test]
    fn without_a_login_it_looks_again_soon_and_asks_again_once_there_is_one() {
        let mut p = Plan::new(Some("claude"));
        p.configure(true, 80.0, 0);
        p.tick(0);
        p.answer(&json!({"error": "not signed in"}), 10);
        assert_eq!(p.problem, "not signed in");
        assert_eq!(p.next_due(), Some(10 + LOGIN_RECHECK_MS));
        // Until the login file changes, only the file is looked at.
        let again = p.tick(10 + LOGIN_RECHECK_MS).unwrap();
        assert_eq!(again.unless_login_is, Some(0));
        p.answer(&json!({"unchanged": true, "login_mtime": 0}), 6000);
        assert_eq!(p.problem, "not signed in");
        assert!(p.tick(6000 + LOGIN_RECHECK_MS).is_some());
        p.answer(&ok(&claude(5.0)), 12_000);
        assert!(p.problem.is_empty() && p.shows());
    }

    #[test]
    fn a_request_that_never_came_back_is_given_up() {
        let mut p = Plan::new(Some("codex"));
        p.configure(true, 80.0, 0);
        assert_eq!(p.tick(0), Some(ask("codex")));
        assert_eq!(p.refresh(29_000, true), None);
        assert_eq!(p.refresh(30_000, true), Some(ask("codex")));
    }
}
