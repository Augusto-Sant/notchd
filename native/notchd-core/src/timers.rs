//! A deterministic timer wheel. Time is passed in (milliseconds on any clock
//! the caller keeps), so a test advances it by hand and the daemon feeds it a
//! monotonic clock. A key has at most one timer: setting it again restarts it
//! (QML `Timer.restart`).

/// Pending timers, each under a key.
#[derive(Debug, Clone)]
pub struct Timers<K> {
    /// (due, sequence, key); `seq` keeps equal due times in insertion order.
    entries: Vec<(u64, u64, K)>,
    seq: u64,
}

impl<K> Default for Timers<K> {
    fn default() -> Self {
        Timers {
            entries: Vec::new(),
            seq: 0,
        }
    }
}

impl<K: PartialEq> Timers<K> {
    pub fn new() -> Timers<K> {
        Timers::default()
    }

    /// Starts (or restarts) the timer of `key` to fire at `due`.
    pub fn set(&mut self, key: K, due: u64) {
        self.cancel(&key);
        self.seq += 1;
        self.entries.push((due, self.seq, key));
    }

    /// Starts `key` to fire `delay` ms after `now`.
    pub fn after(&mut self, key: K, now: u64, delay: u64) {
        self.set(key, now.saturating_add(delay));
    }

    /// Stops the timer of `key`; true if it was running.
    pub fn cancel(&mut self, key: &K) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(_, _, k)| k != key);
        self.entries.len() != before
    }

    pub fn running(&self, key: &K) -> bool {
        self.entries.iter().any(|(_, _, k)| k == key)
    }

    pub fn due_of(&self, key: &K) -> Option<u64> {
        self.entries
            .iter()
            .find(|(_, _, k)| k == key)
            .map(|(d, _, _)| *d)
    }

    /// When the next timer fires, if any.
    pub fn next_due(&self) -> Option<u64> {
        self.entries.iter().map(|(d, _, _)| *d).min()
    }

    /// Takes the one earliest timer due at `now`, so the caller can react to
    /// it before the next (a fired timer may cancel or start others).
    pub fn pop_due(&mut self, now: u64) -> Option<K> {
        let at = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (d, _, _))| *d <= now)
            .min_by_key(|(_, (d, s, _))| (*d, *s))
            .map(|(i, _)| i)?;
        Some(self.entries.remove(at).2)
    }

    /// Removes and returns every key due at `now`, earliest first.
    pub fn fire_due(&mut self, now: u64) -> Vec<K> {
        let mut out = Vec::new();
        while let Some(k) = self.pop_due(now) {
            out.push(k);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_in_order_and_only_once() {
        let mut t = Timers::new();
        t.set("b", 20);
        t.set("a", 10);
        t.set("c", 10);
        assert_eq!(t.next_due(), Some(10));
        assert!(t.fire_due(9).is_empty());
        assert_eq!(t.fire_due(25), vec!["a", "c", "b"]);
        assert!(t.fire_due(100).is_empty());
        assert_eq!(t.next_due(), None);
    }

    #[test]
    fn setting_again_restarts_and_cancel_stops() {
        let mut t = Timers::new();
        t.after("x", 0, 100);
        t.after("x", 50, 100);
        assert_eq!(t.due_of(&"x"), Some(150));
        assert!(t.fire_due(100).is_empty());
        assert!(t.running(&"x"));
        assert!(t.cancel(&"x"));
        assert!(!t.cancel(&"x"));
        assert!(t.fire_due(1000).is_empty());
    }

    #[test]
    fn pop_due_takes_one_at_a_time() {
        let mut t = Timers::new();
        t.set(1, 5);
        t.set(2, 6);
        t.set(3, 99);
        assert_eq!(t.pop_due(10), Some(1));
        assert_eq!(t.pop_due(10), Some(2));
        assert_eq!(t.pop_due(10), None);
        assert!(t.running(&3));
    }

    #[test]
    fn a_zero_delay_is_due_now() {
        let mut t = Timers::new();
        t.after("z", 7, 0);
        assert_eq!(t.fire_due(7), vec!["z"]);
    }
}
