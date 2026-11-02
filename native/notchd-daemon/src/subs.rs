//! Which user module wants which provider (`subscribe` in the manifest, the
//! `subscribe` message), and what each is sent: the latest payload of every
//! provider, throttled so a chatty source never floods a module. Pure: the
//! hub gives it the data and the clock and sends what it returns, so nothing
//! here waits on a module.
//!
//! A payload goes out at once when its provider has been quiet for the
//! provider's interval, and otherwise waits as the one pending payload for
//! its key (a newer one replaces it); [`Subs::flush`] sends it when the
//! interval has passed. A payload equal to the last one is not sent again.

use notchd_proto::ToModule;
use notchd_proto::module::ProviderData;
use notchd_proto::version::SUBSCRIBABLE;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The least time between two payloads of a provider to a module: the
/// doc's numbers (`resources` 1 s; the others are on change, with a floor so
/// a player that writes its position ten times a second is not forwarded
/// that often).
pub fn interval_ms(provider: &str) -> u64 {
    match provider {
        "resources" => 1000,
        "media" => 250,
        "screens" => 100,
        _ => 0,
    }
}

/// One payload for a module: `(module, message)`.
pub type Delivery = (String, ToModule);

#[derive(Default)]
pub struct Subs {
    /// Provider to the modules that asked for it.
    wanted: BTreeMap<String, BTreeSet<String>>,
    /// The latest payload per (provider, key); the key tells apart a
    /// provider's sources (the agent of a `usage` answer), `""` otherwise.
    last: BTreeMap<(String, String), Value>,
    /// When a provider last went out.
    sent_at: BTreeMap<String, u64>,
    /// Payloads waiting out their interval.
    pending: BTreeSet<(String, String)>,
}

impl Subs {
    pub fn new() -> Subs {
        Subs::default()
    }

    /// Whether a provider is one modules can have.
    pub fn known(name: &str) -> bool {
        SUBSCRIBABLE.contains(&name)
    }

    /// `module` now has exactly these subscriptions (a process started: the
    /// manifest's list). Unknown names are ignored here; the manifest's
    /// validation has refused them.
    pub fn reset(&mut self, module: &str, names: &[String]) {
        self.remove(module);
        for n in names {
            self.subscribe(module, n);
        }
    }

    pub fn subscribe(&mut self, module: &str, name: &str) {
        if Subs::known(name) {
            self.wanted
                .entry(name.to_string())
                .or_default()
                .insert(module.to_string());
        }
    }

    pub fn unsubscribe(&mut self, module: &str, name: &str) {
        if let Some(set) = self.wanted.get_mut(name) {
            set.remove(module);
        }
    }

    pub fn remove(&mut self, module: &str) {
        for set in self.wanted.values_mut() {
            set.remove(module);
        }
    }

    /// Whether some module, for which `enabled` holds, wants the provider:
    /// the source reads the system only then.
    pub fn wants(&self, provider: &str, enabled: impl Fn(&str) -> bool) -> bool {
        self.wanted
            .get(provider)
            .is_some_and(|set| set.iter().any(|m| enabled(m)))
    }

    /// Whether the provider has a payload yet.
    pub fn has_data(&self, provider: &str) -> bool {
        self.last.keys().any(|(p, _)| p == provider)
    }

    /// The latest payload of one source of a provider.
    pub fn last_of(&self, provider: &str, key: &str) -> Option<&Value> {
        self.last.get(&(provider.to_string(), key.to_string()))
    }

    /// What `module` is owed on subscribing: the latest payload of each
    /// source of the provider.
    pub fn replay(&self, module: &str, provider: &str) -> Vec<Delivery> {
        self.last
            .iter()
            .filter(|((p, _), _)| p == provider)
            .map(|((p, _), data)| (module.to_string(), message(p, data)))
            .collect()
    }

    /// A provider's new payload: remembered, and returned for each
    /// subscriber now, or held until its interval has passed.
    pub fn publish(&mut self, provider: &str, key: &str, data: Value, now: u64) -> Vec<Delivery> {
        if !Subs::known(provider) {
            return Vec::new();
        }
        let slot = (provider.to_string(), key.to_string());
        if self.last.get(&slot) == Some(&data) {
            return Vec::new();
        }
        self.last.insert(slot.clone(), data);
        let quiet = self
            .sent_at
            .get(provider)
            .is_none_or(|at| now >= at + interval_ms(provider));
        if quiet {
            self.pending.remove(&slot);
            self.sent_at.insert(provider.to_string(), now);
            self.deliveries(&slot)
        } else {
            self.pending.insert(slot);
            Vec::new()
        }
    }

    /// The payloads whose interval has passed.
    pub fn flush(&mut self, now: u64) -> Vec<Delivery> {
        let due: Vec<(String, String)> = self
            .pending
            .iter()
            .filter(|(p, _)| {
                self.sent_at
                    .get(p)
                    .is_none_or(|at| now >= at + interval_ms(p))
            })
            .cloned()
            .collect();
        let mut out = Vec::new();
        for slot in due {
            self.pending.remove(&slot);
            self.sent_at.insert(slot.0.clone(), now);
            out.extend(self.deliveries(&slot));
        }
        out
    }

    /// When the earliest held payload may go (ms), if any is held.
    pub fn next_due(&self) -> Option<u64> {
        self.pending
            .iter()
            .filter_map(|(p, _)| self.sent_at.get(p).map(|at| at + interval_ms(p)))
            .min()
    }

    fn deliveries(&self, slot: &(String, String)) -> Vec<Delivery> {
        let Some(data) = self.last.get(slot) else {
            return Vec::new();
        };
        self.wanted
            .get(&slot.0)
            .into_iter()
            .flatten()
            .map(|m| (m.clone(), message(&slot.0, data)))
            .collect()
    }
}

fn message(provider: &str, data: &Value) -> ToModule {
    ToModule::Provider(ProviderData {
        name: provider.to_string(),
        data: data.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(d: &[Delivery]) -> Vec<&str> {
        d.iter().map(|(m, _)| m.as_str()).collect()
    }

    #[test]
    fn a_payload_reaches_the_subscribers_only() {
        let mut s = Subs::new();
        s.subscribe("a", "resources");
        s.subscribe("b", "media");
        let out = s.publish("resources", "", json!({"cpu": 0.1}), 0);
        assert_eq!(names(&out), ["a"]);
        s.unsubscribe("a", "resources");
        assert!(
            s.publish("resources", "", json!({"cpu": 0.2}), 5000)
                .is_empty()
        );
    }

    #[test]
    fn unknown_names_are_not_subscribed() {
        let mut s = Subs::new();
        s.subscribe("a", "nope");
        s.subscribe("a", "audio");
        assert!(!s.wants("nope", |_| true) && !s.wants("audio", |_| true));
        assert!(s.publish("nope", "", json!(1), 0).is_empty());
    }

    #[test]
    fn a_fast_source_is_held_to_the_interval_and_the_latest_wins() {
        let mut s = Subs::new();
        s.subscribe("a", "resources");
        assert_eq!(s.publish("resources", "", json!({"cpu": 0.1}), 0).len(), 1);
        assert!(
            s.publish("resources", "", json!({"cpu": 0.2}), 100)
                .is_empty()
        );
        assert!(
            s.publish("resources", "", json!({"cpu": 0.3}), 200)
                .is_empty()
        );
        assert_eq!(s.next_due(), Some(1000));
        assert!(s.flush(900).is_empty());
        let out = s.flush(1000);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0].1, ToModule::Provider(p) if p.data == json!({"cpu": 0.3})));
        assert_eq!(s.next_due(), None);
    }

    #[test]
    fn an_unchanged_payload_is_not_sent_again() {
        let mut s = Subs::new();
        s.subscribe("a", "usage");
        assert_eq!(s.publish("usage", "claude", json!({"x": 1}), 0).len(), 1);
        assert!(s.publish("usage", "claude", json!({"x": 1}), 10).is_empty());
        assert_eq!(s.publish("usage", "codex", json!({"x": 1}), 20).len(), 1);
    }

    #[test]
    fn a_new_subscriber_is_owed_the_latest_of_each_source() {
        let mut s = Subs::new();
        s.publish("usage", "claude", json!({"a": 1}), 0);
        s.publish("usage", "codex", json!({"a": 2}), 1);
        assert_eq!(s.replay("late", "usage").len(), 2);
        assert!(s.replay("late", "media").is_empty());
        assert!(s.has_data("usage") && !s.has_data("media"));
    }

    #[test]
    fn reset_replaces_a_modules_subscriptions_and_wants_follows_enabled() {
        let mut s = Subs::new();
        s.reset("a", &["media".into(), "resources".into()]);
        s.reset("a", &["media".into()]);
        assert!(s.wants("media", |_| true) && !s.wants("resources", |_| true));
        assert!(
            !s.wants("media", |m| m != "a"),
            "a disabled module wants nothing"
        );
        s.remove("a");
        assert!(!s.wants("media", |_| true));
    }
}
