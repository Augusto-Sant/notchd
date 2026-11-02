//! The `media` module: what is playing, from the `media` provider (MPRIS on
//! Linux). Compact: the cover, "title · artist" and the equalizer. It peeks on
//! a new track. Expanded: the cover, title, artist and player, the progress
//! bar and the transport controls. Ported from `core/modules/media.qml`; the
//! players are the provider's (`notchd-providers`), their position runs on in
//! the UI by the `progress` node's `anchor`, and the cover is an `image`.
//!
//! Which player shows: the first that is playing, else the first with a
//! track. It holds the closed notch while playing and for a minute after
//! pausing, so a short pause does not make it jump back to rest.

use crate::module::{CoreModule, Effect, Facts, Shown};
use crate::tree::{Ic, Settings, column, icon, icon_button, node, row, text, with};
use notchd_proto::provider::{MediaData, Player};
use notchd_proto::{EventValue, Node, Slot};
use notchd_providers::media::MediaCommand;
use serde_json::{Value, json};

/// How long a pause keeps the module in the notch.
const PAUSE_GRACE_MS: u64 = 60_000;
/// A new track waits this long to peek, so its cover arrives with its title.
const PEEK_DELAY_MS: u64 = 300;
const PEEK_MS: u32 = 3500;
/// A length of a day or more is a stream's garbage (Chrome sends INT64_MAX):
/// no known length, no progress bar.
const MAX_LENGTH: f64 = 86_400.0;

pub struct Media {
    settings: Settings,
    players: Vec<Player>,
    /// When the players were read (epoch ms): a playing position runs on from it.
    read_at: u64,
    /// The shown track's key, to see it change.
    key: String,
    playing: bool,
    /// Playing, or paused for less than the grace.
    recent: bool,
    grace_due: Option<u64>,
    peek_due: Option<u64>,
    /// Its expanded view is on screen: the only time its clock (what changes
    /// by the second) is worth ticking.
    visible: bool,
    effects: Vec<Effect>,
}

impl Media {
    pub fn new() -> Media {
        let mut m = Media {
            settings: Settings::new("media", include_str!("../modules/media.json")),
            players: Vec::new(),
            read_at: 0,
            key: String::new(),
            playing: false,
            recent: false,
            grace_due: None,
            peek_due: None,
            visible: false,
            effects: Vec::new(),
        };
        m.key = m.track_key();
        m
    }

    /// Prefer whatever is playing; otherwise the first player with a track.
    fn chosen(&self) -> Option<&Player> {
        let mut fallback = None;
        for p in &self.players {
            if p.title.is_empty() && p.artist.is_empty() {
                continue;
            }
            if p.playing {
                return Some(p);
            }
            fallback.get_or_insert(p);
        }
        fallback
    }

    fn track_key(&self) -> String {
        let (title, artist) = self
            .chosen()
            .map_or(("", ""), |p| (p.title.as_str(), p.artist.as_str()));
        format!("{title}\u{1}{artist}")
    }

    fn is_playing(&self) -> bool {
        self.chosen().is_some_and(|p| p.playing)
    }

    fn length(p: &Player) -> f64 {
        let v = p.length.0;
        if v.is_finite() && v > 0.0 && v < MAX_LENGTH {
            v
        } else {
            0.0
        }
    }

    /// Where the player is at `now`: its last reading, run on while playing.
    fn position(&self, p: &Player, now: u64) -> f64 {
        let at_read = if p.position.0.is_finite() && p.position.0 > 0.0 {
            p.position.0
        } else {
            0.0
        };
        let len = Self::length(p);
        let run = if p.playing {
            now.saturating_sub(self.read_at) as f64 / 1000.0
        } else {
            0.0
        };
        let v = at_read + run;
        if len > 0.0 { v.min(len) } else { v }
    }

    fn equalizer(&self, height: f64) -> Value {
        json!({
            "t": "equalizer",
            "active": self.is_playing(),
            "mode": if self.settings.get("equalizer") == json!("light") { "light" } else { "smooth" },
            "height": height,
        })
    }

    fn cover(p: &Player, size: f64, radius: &str) -> Value {
        json!({
            "t": "image", "src": p.art_url, "size": size, "radius": radius, "fit": "crop",
            "fallback": icon(("audio-x-generic", 0xF075A), "muted", json!(size * 0.55)),
        })
    }

    fn clock(seconds: f64) -> String {
        let s = seconds.max(0.0).floor() as u64;
        let (m, ss) = (s / 60, s % 60);
        let h = m / 60;
        if h > 0 {
            format!("{h}:{:02}:{ss:02}", m % 60)
        } else {
            format!("{m}:{ss:02}")
        }
    }

    fn compact(&self, p: &Player) -> Value {
        let mut words = vec![with(
            text(&p.title, "foreground", "bodySmall"),
            "elide",
            json!("end"),
        )];
        if !p.artist.is_empty() {
            words.push(text(&format!(" · {}", p.artist), "muted", "bodySmall"));
        }
        with(
            row(
                10.0,
                vec![
                    Self::cover(p, 16.0, "small"),
                    with(row(0.0, words), "maxWidth", json!(240)),
                    json!({"t": "spacer", "fill": true}),
                    self.equalizer(12.0),
                ],
            ),
            "minWidth",
            json!(220),
        )
    }

    fn vertical_compact(&self, p: &Player) -> Value {
        with(
            column(
                8.0,
                vec![Self::cover(p, 16.0, "small"), self.equalizer(12.0)],
            ),
            "align",
            json!("center"),
        )
    }

    fn peek(&self, p: &Player) -> Value {
        let mut lines = vec![with(
            with(text(&p.title, "foreground", "body"), "bold", json!(true)),
            "maxWidth",
            json!(240),
        )];
        if !p.artist.is_empty() {
            lines.push(with(
                text(&p.artist, "muted", "caption"),
                "maxWidth",
                json!(240),
            ));
        }
        row(
            12.0,
            vec![
                Self::cover(p, 38.0, "small"),
                column(0.0, lines),
                self.equalizer(16.0),
            ],
        )
    }

    fn expanded(&self, p: &Player, now: u64) -> Value {
        let mut about = vec![with(
            text(&p.title, "foreground", "body"),
            "bold",
            json!(true),
        )];
        if !p.artist.is_empty() {
            about.push(text(&p.artist, "muted", "bodySmall"));
        }
        if !p.name.is_empty() {
            about.push(text(&p.name, "faint", "caption"));
        }
        let mut parts = vec![with(
            row(
                14.0,
                vec![
                    Self::cover(p, 58.0, "round"),
                    with(column(2.0, about), "fill", json!(true)),
                    self.equalizer(16.0),
                ],
            ),
            "align",
            json!("start"),
        )];
        let len = Self::length(p);
        if len > 0.0 {
            let pos = self.position(p, now);
            let mut bar = json!({
                "t": "progress",
                "value": (self.position(p, self.read_at) / len).clamp(0.0, 1.0),
                "tone": "foreground",
                "thickness": 4,
            });
            if p.playing {
                bar = with(
                    bar,
                    "anchor",
                    json!({"at": self.read_at, "rate": 1.0 / len}),
                );
            }
            if p.can_seek {
                bar = with(bar, "seek", json!("seek"));
            }
            parts.push(column(
                4.0,
                vec![
                    bar,
                    row(
                        8.0,
                        vec![
                            text(&Self::clock(pos), "muted", "caption"),
                            json!({"t": "spacer", "fill": true}),
                            text(&format!("-{}", Self::clock(len - pos)), "muted", "caption"),
                        ],
                    ),
                ],
            ));
        }
        let button = |g: Ic, action: &str, enabled: bool, big: bool| {
            let mut b = with(icon_button(g, action), "enabled", json!(enabled));
            if big {
                b = with(b, "big", json!(true));
            }
            b
        };
        let spacer = json!({"t": "spacer", "fill": true});
        parts.push(row(
            26.0,
            vec![
                spacer.clone(),
                button(
                    ("media-skip-backward", 0xF04AE),
                    "previous",
                    p.can_prev,
                    false,
                ),
                button(
                    if p.playing {
                        ("media-playback-pause", 0xF03E4)
                    } else {
                        ("media-playback-start", 0xF040A)
                    },
                    "toggle",
                    true,
                    true,
                ),
                button(("media-skip-forward", 0xF04AD), "next", p.can_next, false),
                spacer,
            ],
        ));
        column(10.0, parts)
    }

    fn can_toggle(p: &Player) -> bool {
        p.can_play || p.can_pause
    }
}

impl Default for Media {
    fn default() -> Media {
        Media::new()
    }
}

impl CoreModule for Media {
    fn id(&self) -> &'static str {
        "media"
    }

    fn facts(&self, _now: u64) -> Facts {
        Facts {
            priority: 50,
            active: self
                .chosen()
                .is_some_and(|p| !p.title.is_empty() && (p.playing || self.recent)),
            ..Facts::default()
        }
    }

    fn view(&self, slot: Slot, now: u64) -> Option<Node> {
        let p = self.chosen()?;
        Some(node(match slot {
            Slot::Compact => self.compact(p),
            Slot::VerticalCompact => self.vertical_compact(p),
            Slot::Peek => self.peek(p),
            Slot::VerticalPeek => return None,
            Slot::Expanded => self.expanded(p, now),
        }))
    }

    fn config(&mut self, cfg: &Value, _now: u64) {
        self.settings.set_config(cfg);
    }

    fn feed(&mut self, provider: &str, data: &Value, now: u64) {
        if provider != "media" {
            return;
        }
        let Ok(data) = serde_json::from_value::<MediaData>(data.clone()) else {
            return;
        };
        self.players = data.players;
        self.read_at = now;
        let playing = self.is_playing();
        if playing != self.playing {
            self.playing = playing;
            if playing {
                self.recent = true;
                self.grace_due = None;
            } else {
                self.grace_due = Some(now + PAUSE_GRACE_MS);
            }
        }
        let key = self.track_key();
        if key != self.key {
            self.key = key;
            let titled = self.chosen().is_some_and(|p| !p.title.is_empty());
            if playing && titled {
                self.peek_due = Some(now + PEEK_DELAY_MS);
            }
        }
    }

    fn event(&mut self, _ctx: &crate::module::Ctx, action: &str, value: &EventValue, now: u64) {
        let Some(p) = self.chosen() else { return };
        let player = p.id.clone();
        let command = match action {
            "toggle" if Self::can_toggle(p) => MediaCommand::PlayPause { player },
            "next" if p.can_next => MediaCommand::Next { player },
            "previous" if p.can_prev => MediaCommand::Previous { player },
            "seek" if p.can_seek => {
                let len = Self::length(p);
                let EventValue::Number(f) = value else { return };
                if len <= 0.0 || !f.0.is_finite() {
                    return;
                }
                let seconds = f.0.clamp(0.0, 1.0) * len;
                // The bar follows the press at once, not when the player
                // says where it went.
                if let Some(p) = self.players.iter_mut().find(|p| p.id == player) {
                    p.position.0 = seconds;
                }
                self.read_at = now;
                MediaCommand::Seek { player, seconds }
            }
            _ => return,
        };
        self.effects.push(Effect::Media(command));
    }

    fn shown(&mut self, shown: Shown, _now: u64) {
        self.visible = shown.visible;
    }

    fn tick(&mut self, now: u64) {
        if self.grace_due.is_some_and(|at| at <= now) {
            self.grace_due = None;
            self.recent = false;
        }
        if self.peek_due.is_some_and(|at| at <= now) {
            self.peek_due = None;
            self.effects.push(Effect::Peek(PEEK_MS));
        }
    }

    /// The pause grace, the delayed peek, and, while a track with a known
    /// length plays and the expanded view is on screen, the moment the seconds
    /// on the clock change (a closed notch costs no per-second work).
    fn next_due(&self, now: u64) -> Option<u64> {
        let clock = self
            .chosen()
            .filter(|p| self.visible && p.playing && Self::length(p) > 0.0)
            .map(|p| {
                let ms = (self.position(p, now) * 1000.0) as u64;
                now + (1000 - ms % 1000).max(1)
            });
        [self.grace_due, self.peek_due, clock]
            .into_iter()
            .flatten()
            .min()
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// The test hook `test-media <MediaData JSON>`: these players in place of
    /// the system's (the host stops feeding the provider's).
    fn demo(&mut self, args: &[String], now: u64) -> Option<String> {
        let data: Value = serde_json::from_str(args.first()?).ok()?;
        self.feed("media", &data, now);
        Some("ok".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::Ctx;
    use notchd_proto::Num;

    const T0: u64 = 1_000_000;

    fn player(id: &str, title: &str, playing: bool) -> Value {
        json!({
            "id": id, "name": "Demo", "playing": playing, "title": title, "artist": "Neon Harbor",
            "album": "", "artUrl": "file:///tmp/c.png", "length": 214, "position": 92,
            "canPlay": true, "canPause": true, "canNext": true, "canPrev": true, "canSeek": true,
        })
    }

    fn fed(players: Vec<Value>, now: u64) -> Media {
        let mut m = Media::new();
        m.feed("media", &json!({"players": players}), now);
        m
    }

    fn json_of(m: &Media, slot: Slot, now: u64) -> String {
        serde_json::to_string(&m.view(slot, now).expect("a view")).unwrap()
    }

    #[test]
    fn nothing_playing_is_inactive_and_has_no_views() {
        let m = Media::new();
        assert!(!m.facts(T0).active);
        for s in Slot::ALL {
            assert!(m.view(s, T0).is_none(), "{s:?}");
        }
        assert_eq!(m.next_due(T0), None);
    }

    #[test]
    fn a_playing_track_is_active_and_every_slot_builds() {
        let m = fed(vec![player("a", "Night Drive", true)], T0);
        assert!(m.facts(T0).active);
        assert_eq!(m.facts(T0).priority, 50);
        for (slot, has) in [
            (Slot::Compact, true),
            (Slot::VerticalCompact, true),
            (Slot::Peek, true),
            (Slot::VerticalPeek, false),
            (Slot::Expanded, true),
        ] {
            assert_eq!(m.view(slot, T0).is_some(), has, "{slot:?}");
        }
        let compact = json_of(&m, Slot::Compact, T0);
        assert!(compact.contains("Night Drive") && compact.contains(" · Neon Harbor"));
        assert!(compact.contains("file:///tmp/c.png") && compact.contains(r#""active":true"#));
    }

    #[test]
    fn the_player_that_plays_wins_over_one_that_only_has_a_track() {
        let m = fed(
            vec![
                player("a", "Paused Song", false),
                json!({"id": "b", "name": "B", "playing": true, "title": "", "artist": "", "album": "",
                       "artUrl": "", "length": 0, "position": 0, "canPlay": false, "canPause": false,
                       "canNext": false, "canPrev": false, "canSeek": false}),
                player("c", "Live Song", true),
            ],
            T0,
        );
        assert_eq!(m.chosen().unwrap().id, "c", "b has no track at all");
        let m = fed(
            vec![player("a", "First", false), player("b", "Second", false)],
            T0,
        );
        assert_eq!(m.chosen().unwrap().id, "a");
    }

    #[test]
    fn a_pause_keeps_the_module_for_a_minute_then_it_goes() {
        let mut m = fed(vec![player("a", "Song", true)], T0);
        m.tick(T0 + 300);
        m.feed(
            "media",
            &json!({"players": [player("a", "Song", false)]}),
            T0 + 1000,
        );
        assert!(m.facts(T0 + 1000).active, "paused, still in the notch");
        let due = m.next_due(T0 + 1000).unwrap();
        assert_eq!(due, T0 + 1000 + 60_000);
        m.tick(due);
        assert!(!m.facts(due).active);
        // Playing again brings it back and cancels any wait.
        m.feed(
            "media",
            &json!({"players": [player("a", "Song", true)]}),
            due + 5,
        );
        assert!(m.facts(due + 5).active);
        assert_eq!(m.grace_due, None);
    }

    #[test]
    fn a_track_that_started_paused_is_not_shown() {
        let m = fed(vec![player("a", "Song", false)], T0);
        assert!(!m.facts(T0).active);
        assert!(m.view(Slot::Expanded, T0).is_some(), "but the tab has it");
    }

    #[test]
    fn a_new_track_peeks_after_a_moment_and_the_same_track_does_not() {
        let mut m = fed(vec![player("a", "One", true)], T0);
        assert_eq!(m.next_due(T0).map(|d| d.min(T0 + 300)), Some(T0 + 300));
        m.tick(T0 + 300);
        assert_eq!(m.take_effects(), vec![Effect::Peek(3500)]);
        m.feed(
            "media",
            &json!({"players": [player("a", "One", true)]}),
            T0 + 400,
        );
        m.tick(T0 + 1000);
        assert!(m.take_effects().is_empty(), "same track");
        m.feed(
            "media",
            &json!({"players": [player("a", "Two", true)]}),
            T0 + 5000,
        );
        m.feed(
            "media",
            &json!({"players": [player("a", "Two", true)]}),
            T0 + 5100,
        );
        m.tick(T0 + 5300);
        assert_eq!(m.take_effects(), vec![Effect::Peek(3500)]);
        m.tick(T0 + 5400);
        assert!(m.take_effects().is_empty(), "once");
    }

    #[test]
    fn a_track_that_changes_while_paused_does_not_peek() {
        let mut m = fed(vec![player("a", "One", false)], T0);
        m.tick(T0 + 10_000);
        assert!(m.take_effects().is_empty());
    }

    #[test]
    fn the_bar_runs_on_by_its_anchor_and_the_clock_by_the_time() {
        let m = fed(vec![player("a", "Song", true)], T0);
        let at = T0 + 7000;
        let expanded = json_of(&m, Slot::Expanded, at);
        let v: Value = serde_json::from_str(&expanded).unwrap();
        let bar = &v["children"][1]["children"][0];
        assert_eq!(bar["t"], "progress");
        assert_eq!(bar["anchor"]["at"], json!(T0));
        assert!((bar["anchor"]["rate"].as_f64().unwrap() - 1.0 / 214.0).abs() < 1e-9);
        assert!((bar["value"].as_f64().unwrap() - 92.0 / 214.0).abs() < 1e-9);
        assert_eq!(bar["seek"], "seek");
        // 92 s + 7 s = 1:39, 214 - 99 = 115 s = 1:55 left.
        assert!(
            expanded.contains("1:39") && expanded.contains("-1:55"),
            "{expanded}"
        );
        // The bar's node is the same a second later: only the clock moves.
        let later: Value = serde_json::from_str(&json_of(&m, Slot::Expanded, at + 1000)).unwrap();
        assert_eq!(later["children"][1]["children"][0], *bar);
    }

    #[test]
    fn a_paused_bar_stands_still() {
        let m = fed(vec![player("a", "Song", false)], T0);
        let v: Value = serde_json::from_str(&json_of(&m, Slot::Expanded, T0 + 9000)).unwrap();
        let bar = &v["children"][1]["children"][0];
        assert!(bar.get("anchor").is_none());
        assert!(json_of(&m, Slot::Expanded, T0 + 9000).contains("1:32"));
        assert_eq!(m.next_due(T0), None, "no clock to run");
    }

    #[test]
    fn the_clock_is_due_when_its_seconds_change() {
        let mut m = fed(vec![player("a", "Song", true)], T0);
        m.shown(open(), T0);
        m.tick(T0 + 300);
        // Read at 92.000 s: the next second is 1000 ms on.
        assert_eq!(m.next_due(T0), Some(T0 + 1000));
        assert_eq!(m.next_due(T0 + 300), Some(T0 + 1000));
        assert_eq!(m.next_due(T0 + 1000), Some(T0 + 2000));
    }

    fn open() -> Shown {
        Shown {
            open: true,
            visible: true,
        }
    }

    #[test]
    fn a_closed_notch_has_no_clock_to_tick() {
        let mut m = fed(vec![player("a", "Song", true)], T0);
        // Playing, but no expanded view on screen: no per-second work.
        m.tick(T0 + 300);
        assert_eq!(m.next_due(T0 + 300), None);
        assert_eq!(m.next_due(T0 + 5000), None);
        // Shown: the clock runs; hidden again: it stops.
        m.shown(open(), T0 + 5000);
        assert_eq!(m.next_due(T0 + 5000), Some(T0 + 6000));
        m.shown(Shown::default(), T0 + 6000);
        assert_eq!(m.next_due(T0 + 6000), None);
        // A hover shows without opening: visible alone is enough.
        m.shown(
            Shown {
                open: false,
                visible: true,
            },
            T0 + 6000,
        );
        assert_eq!(m.next_due(T0 + 6000), Some(T0 + 7000));
    }

    #[test]
    fn a_length_that_is_garbage_hides_the_bar() {
        for len in [json!(0), json!(9_223_372_036_854.0), json!(86_400)] {
            let mut p = player("a", "Live", true);
            p["length"] = len;
            let m = fed(vec![p], T0);
            let expanded = json_of(&m, Slot::Expanded, T0);
            assert!(!expanded.contains("progress"), "{expanded}");
            assert_eq!(m.next_due(T0).map(|d| d.min(T0 + 300)), Some(T0 + 300));
        }
    }

    #[test]
    fn a_player_without_a_known_position_starts_at_zero() {
        let mut p = player("a", "Song", false);
        p["position"] = json!(0);
        let m = fed(vec![p], T0);
        assert!(json_of(&m, Slot::Expanded, T0).contains("0:00"));
    }

    #[test]
    fn the_controls_become_commands_for_the_shown_player() {
        let mut m = fed(
            vec![player("a", "Paused", false), player("b", "Live", true)],
            T0,
        );
        let ctx = Ctx::default();
        let null = EventValue::Null(());
        m.event(&ctx, "toggle", &null, T0);
        m.event(&ctx, "next", &null, T0);
        m.event(&ctx, "previous", &null, T0);
        m.event(&ctx, "nonsense", &null, T0);
        m.event(&ctx, "seek", &EventValue::Number(Num(0.5)), T0 + 100);
        m.event(&ctx, "seek", &EventValue::Null(()), T0);
        let b = || "b".to_string();
        assert_eq!(
            m.take_effects(),
            vec![
                Effect::Media(MediaCommand::PlayPause { player: b() }),
                Effect::Media(MediaCommand::Next { player: b() }),
                Effect::Media(MediaCommand::Previous { player: b() }),
                Effect::Media(MediaCommand::Seek {
                    player: b(),
                    seconds: 107.0
                }),
            ]
        );
        // The bar already shows where it was sent.
        assert!(json_of(&m, Slot::Expanded, T0 + 100).contains("1:47"));
    }

    #[test]
    fn a_control_the_player_does_not_offer_does_nothing_and_shows_disabled() {
        let mut p = player("a", "Song", true);
        p["canNext"] = json!(false);
        p["canSeek"] = json!(false);
        p["canPlay"] = json!(false);
        p["canPause"] = json!(false);
        let mut m = fed(vec![p], T0);
        let ctx = Ctx::default();
        for a in ["next", "toggle"] {
            m.event(&ctx, a, &EventValue::Null(()), T0);
        }
        m.event(&ctx, "seek", &EventValue::Number(Num(0.2)), T0);
        assert!(m.take_effects().is_empty());
        let v: Value = serde_json::from_str(&json_of(&m, Slot::Expanded, T0)).unwrap();
        assert!(v["children"][1]["children"][0].get("seek").is_none());
        let controls = v["children"][2]["children"].as_array().unwrap();
        let next = controls.iter().find(|b| b["action"] == "next").unwrap();
        assert_eq!(next["enabled"], json!(false));
    }

    #[test]
    fn the_equalizer_follows_playing_and_the_setting() {
        let mut m = fed(vec![player("a", "Song", true)], T0);
        assert!(json_of(&m, Slot::Compact, T0).contains(r#""mode":"smooth""#));
        m.config(&json!({"media": {"equalizer": "light"}}), T0);
        assert!(json_of(&m, Slot::Compact, T0).contains(r#""mode":"light""#));
        m.feed(
            "media",
            &json!({"players": [player("a", "Song", false)]}),
            T0 + 1,
        );
        assert!(json_of(&m, Slot::Compact, T0 + 1).contains(r#""active":false"#));
    }

    #[test]
    fn a_track_with_no_artist_shows_the_title_alone() {
        let mut p = player("a", "Instrumental", true);
        p["artist"] = json!("");
        let m = fed(vec![p], T0);
        for slot in [Slot::Compact, Slot::Peek, Slot::Expanded] {
            assert!(!json_of(&m, slot, T0).contains(" · "), "{slot:?}");
        }
    }

    #[test]
    fn the_demo_hook_takes_players_and_a_bad_one_is_refused() {
        let mut m = Media::new();
        let data = json!({"players": [player("a", "Song", true)]}).to_string();
        assert_eq!(m.demo(&[data], T0), Some("ok".into()));
        assert!(m.facts(T0).active);
        assert_eq!(m.demo(&["nope".into()], T0), None);
        assert!(m.demo(&[], T0).is_none());
    }

    #[test]
    fn data_that_is_not_media_is_ignored() {
        let mut m = fed(vec![player("a", "Song", true)], T0);
        m.feed("resources", &json!({"players": []}), T0 + 1);
        m.feed("media", &json!({"nope": 1}), T0 + 1);
        assert!(m.facts(T0 + 1).active);
    }
}
