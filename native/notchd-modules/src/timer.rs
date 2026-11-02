//! The `timer` module: a countdown and a pomodoro. Started from the expanded
//! notch (5, 15 and 25 minutes, or a pomodoro) or over IPC (`timer 25m`,
//! `timer "3m Tea"`, `timer pomodoro`, `timer toggle`, `timer +5m`,
//! `timer skip`, `timer stop`: docs/ipc.md). Closed, it shows the time left
//! while one runs. When the time is up it peeks, even over the open notch,
//! and sends a desktop notification. A pomodoro goes focus, short break,
//! focus... and a long break after the last round, then stops. Ported from
//! `core/modules/timer.qml`; the rules are `notchd-core::countdown`.

use crate::module::{CoreModule, Ctx, Effect, Facts};
use crate::tree::{Settings, column, glyph, node, row, text, with};
use notchd_core::countdown::{self as cd, Command, Kind, Phase, PomodoroSettings, State, Status};
use notchd_proto::{EventValue, Node, Slot};
use serde_json::{Value, json};

/// A done timer goes away after this long; a pomodoro waiting for its next
/// phase stays until you start it or stop it.
const DONE_FOR_MS: u64 = 60_000;

const TIMER: u32 = 0xF051B; // nf-md-timer_outline
const BELL: u32 = 0xF009E; // nf-md-bell_ring
const BRAIN: u32 = 0xF09D1; // nf-md-brain
const COFFEE: u32 = 0xF0176; // nf-md-coffee
const PAUSE: u32 = 0xF03E4;
const PLAY: u32 = 0xF040A;
const PLUS: u32 = 0xF0415;
const SKIP: u32 = 0xF04AD;
const RESTART: u32 = 0xF0709;
const CLOSE: u32 = 0xF0156;
const STOP: u32 = 0xF04DB;

pub struct Timer {
    settings: Settings,
    countdown: State,
    /// When the countdown turned done, for the minute it stays.
    done_at: Option<u64>,
    effects: Vec<Effect>,
}

impl Timer {
    pub fn new() -> Timer {
        Timer {
            settings: Settings::new("timer", include_str!("../../../core/modules/timer.json")),
            countdown: State::idle(),
            done_at: None,
            effects: Vec::new(),
        }
    }

    fn pomodoro(&self) -> PomodoroSettings {
        PomodoroSettings {
            focus: Some(self.settings.get("focus")),
            short_break: Some(self.settings.get("shortBreak")),
            long_break: Some(self.settings.get("longBreak")),
            rounds: Some(self.settings.get("rounds")),
            auto_advance: Some(self.settings.get("autoAdvance")),
        }
    }

    fn status(&self) -> Status {
        self.countdown.status
    }

    fn counting(&self) -> bool {
        matches!(self.status(), Status::Running | Status::Paused)
    }

    fn remaining(&self, now: u64) -> f64 {
        cd::remaining(&self.countdown, now as f64)
    }

    fn clock(&self, now: u64) -> String {
        cd::format(self.remaining(now))
    }

    fn state_glyph(&self) -> String {
        glyph(if self.status() == Status::Done {
            BELL
        } else if self.countdown.kind == Some(Kind::Pomodoro) {
            if self.countdown.phase == Some(Phase::Focus) {
                BRAIN
            } else {
                COFFEE
            }
        } else {
            TIMER
        })
    }

    /// What the closed notch says once the time is up.
    fn done_text(&self) -> String {
        if let Some(next) = &self.countdown.next {
            format!("Next: {}", next.phase.label())
        } else if self.countdown.kind == Some(Kind::Pomodoro) {
            "Pomodoro done".into()
        } else {
            "Time's up".into()
        }
    }

    fn done_detail(&self) -> String {
        match self.countdown.label.as_deref() {
            Some(l) if !l.is_empty() => l.to_string(),
            _ => format!("{} timer", cd::format(self.countdown.duration)),
        }
    }

    fn tone(&self) -> &'static str {
        if self.status() == Status::Paused {
            "muted"
        } else {
            "accent"
        }
    }

    fn ink(&self) -> &'static str {
        if self.status() == Status::Paused {
            "muted"
        } else {
            "foreground"
        }
    }

    fn run(&mut self, cmd: &Command, now: u64) {
        let was = self.status();
        self.countdown = cd::apply(&self.countdown, cmd, now as f64, &self.pomodoro());
        if self.status() == Status::Done && was != Status::Done {
            self.done_at = Some(now);
        }
    }

    /// The notification for a countdown that just ended (`ended`), the state
    /// being what followed it (a pomodoro goes on to its next phase).
    fn notify(&mut self, ended: &State) {
        let pomodoro = ended.kind == Some(Kind::Pomodoro);
        let body = if self.status() == Status::Running {
            format!(
                "{} · {}",
                cd::title(&self.countdown),
                cd::format(self.countdown.duration)
            )
        } else if pomodoro {
            self.done_text()
        } else {
            match ended.label.as_deref() {
                Some(l) if !l.is_empty() => l.to_string(),
                _ => format!("{} timer", cd::format(ended.duration)),
            }
        };
        let head = if pomodoro {
            format!("{} is over", ended.label.clone().unwrap_or_default())
        } else {
            "Time's up".into()
        };
        self.effects.push(Effect::Notify { title: head, body });
    }

    // ----------------------------------------------------------- views

    fn compact(&self, now: u64) -> Value {
        row(
            8.0,
            vec![
                text(&self.state_glyph(), self.tone(), "body"),
                with(
                    text(
                        &if self.status() == Status::Done {
                            self.done_text()
                        } else {
                            self.clock(now)
                        },
                        self.ink(),
                        "bodySmall",
                    ),
                    "tabular",
                    json!(true),
                ),
            ],
        )
    }

    /// Side edge: the icon over the minutes left (seconds in the last one).
    fn vertical_compact(&self, now: u64) -> Value {
        let left = self.remaining(now);
        let mut parts = vec![text(&self.state_glyph(), self.tone(), "body")];
        if self.status() != Status::Done {
            let t = if left >= 60_000.0 {
                format!("{}", (left / 60_000.0).ceil())
            } else {
                format!("{}s", (left / 1000.0).ceil())
            };
            parts.push(text(&t, self.ink(), "caption"));
        }
        with(column(2.0, parts), "align", json!("center"))
    }

    /// As wide as it needs, up to 300 px; a long label elides.
    fn peek(&self, now: u64) -> Value {
        let done = self.status() == Status::Done;
        with(
            row(
                12.0,
                vec![
                    text(&self.state_glyph(), "accent", "heading"),
                    with(
                        column(
                            0.0,
                            vec![
                                text(
                                    &if done {
                                        self.done_text()
                                    } else {
                                        cd::title(&self.countdown)
                                    },
                                    "foreground",
                                    "body",
                                ),
                                text(
                                    &if done {
                                        self.done_detail()
                                    } else {
                                        format!("{} left", self.clock(now))
                                    },
                                    "muted",
                                    "caption",
                                ),
                            ],
                        ),
                        "fill",
                        json!(true),
                    ),
                ],
            ),
            "maxWidth",
            json!(300),
        )
    }

    fn button(&self, code: u32, action: &str, big: bool) -> Value {
        let mut b = json!({"t": "button", "glyph": glyph(code), "action": action});
        if big {
            b["big"] = json!(true);
        }
        b
    }

    fn expanded(&self, now: u64) -> Value {
        if self.status() == Status::Idle {
            // Idle: the quick starts.
            let chips = [5, 15, 25].map(|m| {
                json!({"t": "chip", "label": format!("{m} min"), "action": "start",
                       "value": m.to_string(), "fill": true})
            });
            let mut presets: Vec<Value> = chips.to_vec();
            presets.push(
                json!({"t": "button", "glyph": glyph(BRAIN), "label": "Pomodoro",
                "variant": "outline", "action": "pomodoro", "fill": true}),
            );
            return column(
                10.0,
                vec![column(
                    8.0,
                    vec![
                        row(
                            10.0,
                            vec![
                                text(&glyph(TIMER), "accent", "body"),
                                with(text("Timer", "foreground", "body"), "fill", json!(true)),
                            ],
                        ),
                        row(6.0, presets),
                    ],
                )],
            );
        }
        let done = self.status() == Status::Done;
        let pomodoro = self.countdown.kind == Some(Kind::Pomodoro);
        let heading = format!(
            "{}{}",
            cd::title(&self.countdown),
            if self.status() == Status::Paused {
                " · paused"
            } else {
                ""
            }
        );
        let mut controls = vec![
            text(&self.state_glyph(), self.tone(), "heading"),
            with(
                column(
                    0.0,
                    vec![
                        text(&heading, "muted", "caption"),
                        with(
                            text(
                                &if done {
                                    self.done_text()
                                } else {
                                    self.clock(now)
                                },
                                self.ink(),
                                if done { "heading" } else { "display" },
                            ),
                            "tabular",
                            json!(true),
                        ),
                    ],
                ),
                "fill",
                json!(true),
            ),
        ];
        // Running or paused: pause/resume, then +1 min (timer) or skip
        // (pomodoro), then stop. Done: again or the next phase, then dismiss.
        if self.counting() {
            controls.push(self.button(
                if self.status() == Status::Running {
                    PAUSE
                } else {
                    PLAY
                },
                "toggle",
                true,
            ));
            controls.push(if pomodoro {
                self.button(SKIP, "skip", false)
            } else {
                self.button(PLUS, "add", false)
            });
        }
        if done && (self.countdown.next.is_some() || !pomodoro) {
            controls.push(if self.countdown.next.is_some() {
                self.button(PLAY, "toggle", true)
            } else {
                self.button(RESTART, "restart", true)
            });
        }
        controls.push(self.button(if done { CLOSE } else { STOP }, "stop", false));
        let mut parts = vec![row(12.0, controls)];
        if self.counting() {
            parts.push(json!({"t": "progress",
                "value": cd::progress(&self.countdown, now as f64),
                "tone": self.tone(), "thickness": 4}));
        }
        column(10.0, parts)
    }
}

impl Default for Timer {
    fn default() -> Timer {
        Timer::new()
    }
}

impl CoreModule for Timer {
    fn id(&self) -> &'static str {
        "timer"
    }

    fn facts(&self, _now: u64) -> Facts {
        Facts {
            // Above media: a running timer is what you started on purpose.
            priority: 60,
            // Idle, the expanded notch offers the quick starts, and the
            // closed notch shows the icon (the notch's idleIcons).
            active: true,
            icon: glyph(TIMER),
            // Its end must show even while the notch is open.
            urgent_peek: true,
            ..Facts::default()
        }
    }

    fn view(&self, slot: Slot, now: u64) -> Option<Node> {
        let running = self.status() != Status::Idle;
        Some(node(match slot {
            Slot::Compact if running => self.compact(now),
            Slot::VerticalCompact if running => self.vertical_compact(now),
            Slot::Peek => self.peek(now),
            Slot::Expanded => self.expanded(now),
            _ => return None,
        }))
    }

    fn config(&mut self, cfg: &Value, _now: u64) {
        self.settings.set_config(cfg);
    }

    /// An IPC `timer` argument: "ok" or what was wrong.
    fn command(&mut self, ctx: &Ctx, method: &str, args: &[String], now: u64) -> Option<String> {
        if method != "timer" {
            return None;
        }
        let cmd = match cd::parse_command(args.first().map_or("", String::as_str)) {
            Ok(c) => c,
            Err(e) => return Some(e),
        };
        let starts = matches!(cmd, Command::Start { .. } | Command::Pomodoro);
        if !starts && !matches!(cmd, Command::Stop) && self.status() == Status::Idle {
            return Some("no timer running".into());
        }
        self.run(&cmd, now);
        // A confirmation, unless the notch is open and already shows it.
        if starts && !ctx.expanded {
            self.effects.push(Effect::Peek(2500));
        }
        Some("ok".into())
    }

    fn event(&mut self, _ctx: &Ctx, action: &str, value: &EventValue, now: u64) {
        let cmd = match action {
            "start" => {
                let EventValue::Text(m) = value else { return };
                let Ok(minutes) = m.parse::<f64>() else {
                    return;
                };
                Command::Start {
                    ms: minutes * 60_000.0,
                    label: String::new(),
                }
            }
            "pomodoro" => Command::Pomodoro,
            "toggle" => Command::Toggle,
            "add" => Command::Add { ms: 60_000.0 },
            "skip" => Command::Skip,
            "stop" => Command::Stop,
            "restart" => Command::Start {
                ms: self.countdown.duration,
                label: self.countdown.label.clone().unwrap_or_default(),
            },
            _ => return,
        };
        self.run(&cmd, now);
    }

    fn tick(&mut self, now: u64) {
        let r = cd::tick(&self.countdown, now as f64, &self.pomodoro());
        if r.finished {
            let ended = std::mem::replace(&mut self.countdown, r.state);
            if self.status() == Status::Done {
                self.done_at = Some(now);
            }
            self.effects.push(Effect::Peek(8000));
            if self.settings.get("notify") == json!(true) {
                self.notify(&ended);
            }
        }
        if self.status() == Status::Done
            && self.countdown.next.is_none()
            && self.done_at.is_some_and(|at| now >= at + DONE_FOR_MS)
        {
            self.countdown = State::idle();
            self.done_at = None;
        }
    }

    /// Wakes as the shown second turns (`until_change`): a fixed 250 ms
    /// redrew the notch four times for each change.
    fn next_due(&self, now: u64) -> Option<u64> {
        match self.status() {
            Status::Running => Some(now + cd::until_change(&self.countdown, now as f64) as u64),
            Status::Done if self.countdown.next.is_none() => {
                self.done_at.map(|at| at + DONE_FOR_MS)
            }
            _ => None,
        }
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// `test-timer <state json>`: the countdown set by hand (the renders show
    /// a pomodoro in its second round, a timer that is done).
    fn demo(&mut self, args: &[String], now: u64) -> Option<String> {
        let state: State = serde_json::from_str(args.first()?).ok()?;
        self.countdown = state;
        self.done_at = (self.status() == Status::Done).then_some(now);
        Some("ok".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const T0: u64 = 1_700_000_000_000;

    fn say(m: &mut Timer, cmd: &str, now: u64) -> String {
        m.command(&Ctx::default(), "timer", &[cmd.into()], now)
            .unwrap()
    }

    fn texts(n: &Node) -> Vec<String> {
        fn walk(v: &Value, out: &mut Vec<String>) {
            for key in ["text", "label"] {
                if let Some(t) = v.get(key).and_then(Value::as_str) {
                    out.push(t.into());
                }
            }
            for c in v
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        walk(&serde_json::to_value(n).unwrap(), &mut out);
        out
    }

    fn quiet() -> Timer {
        let mut m = Timer::new();
        m.config(&json!({"timer": {"notify": false}}), 0);
        m
    }

    #[test]
    fn idle_it_rests_but_offers_its_quick_starts_and_its_icon() {
        let m = quiet();
        let f = m.facts(T0);
        assert!(f.active && f.urgent_peek && f.priority == 60 && !f.icon.is_empty());
        assert!(m.view(Slot::Compact, T0).is_none() && m.view(Slot::VerticalCompact, T0).is_none());
        let t = texts(&m.view(Slot::Expanded, T0).unwrap());
        for want in ["Timer", "5 min", "15 min", "25 min", "Pomodoro"] {
            assert!(t.contains(&want.to_string()), "{want} in {t:?}");
        }
    }

    #[test]
    fn a_running_timer_counts_down_in_every_view_and_confirms_its_start_once() {
        let mut m = quiet();
        assert_eq!(say(&mut m, "3m Tea", T0), "ok");
        assert_eq!(m.take_effects(), vec![Effect::Peek(2500)]);
        assert!(texts(&m.view(Slot::Compact, T0 + 1000).unwrap()).contains(&"2:59".to_string()));
        assert_eq!(
            texts(&m.view(Slot::VerticalCompact, T0 + 1000).unwrap())[1],
            "3"
        );
        assert_eq!(
            texts(
                &m.view(Slot::VerticalCompact, T0 + 2 * MIN + 10_000)
                    .unwrap()
            )[1],
            "50s"
        );
        let peek = texts(&m.view(Slot::Peek, T0 + 1000).unwrap());
        assert_eq!(&peek[1..], ["Tea", "2:59 left"]);
        // Open already: no confirmation.
        let ctx = Ctx {
            expanded: true,
            ..Ctx::default()
        };
        m.command(&ctx, "timer", &["5m".into()], T0);
        assert!(m.take_effects().is_empty());
        // A countdown wakes as the shown second turns.
        let due = m.next_due(T0 + 1).unwrap();
        assert!((T0 + 1..=T0 + 1100).contains(&due), "{}", due - T0);
    }

    #[test]
    fn the_end_peeks_notifies_and_goes_back_to_rest_after_a_minute() {
        let mut m = Timer::new();
        say(&mut m, "1m Tea", T0);
        m.take_effects();
        m.tick(T0 + MIN + 5);
        assert_eq!(m.status(), Status::Done);
        let e = m.take_effects();
        assert_eq!(e[0], Effect::Peek(8000));
        assert_eq!(
            e[1],
            Effect::Notify {
                title: "Time's up".into(),
                body: "Tea".into()
            }
        );
        assert_eq!(
            texts(&m.view(Slot::Compact, T0 + MIN + 5).unwrap())[1],
            "Time's up"
        );
        assert_eq!(m.next_due(T0 + MIN + 5), Some(T0 + 2 * MIN + 5));
        m.tick(T0 + 2 * MIN + 5);
        assert_eq!(m.status(), Status::Idle);
        assert!(m.view(Slot::Compact, T0 + 2 * MIN + 5).is_none());
    }

    #[test]
    fn notifying_can_be_switched_off() {
        let mut m = quiet();
        say(&mut m, "1m", T0);
        m.take_effects();
        m.tick(T0 + MIN + 5);
        assert_eq!(m.take_effects(), vec![Effect::Peek(8000)]);
    }

    #[test]
    fn commands_answer_what_was_wrong() {
        let mut m = quiet();
        for c in ["pause", "resume", "+5m", "skip", "toggle", ""] {
            assert_eq!(say(&mut m, c, T0), "no timer running", "{c:?}");
        }
        assert_eq!(say(&mut m, "stop", T0), "ok", "stop is always fine");
        assert!(say(&mut m, "nonsense words", T0).starts_with("usage: timer"));
        assert_eq!(say(&mut m, "+x", T0), "not a duration: x");
        assert_eq!(say(&mut m, "25", T0), "ok");
        assert_eq!(say(&mut m, "pause", T0 + 1000), "ok");
        assert_eq!(m.status(), Status::Paused);
        assert!(
            texts(&m.view(Slot::Expanded, T0 + 1000).unwrap())
                .contains(&"Timer · paused".to_string())
        );
        assert_eq!(say(&mut m, "toggle", T0 + 2000), "ok");
        assert_eq!(m.status(), Status::Running);
        assert!(
            m.command(&Ctx::default(), "push", &["{}".into()], T0)
                .is_none()
        );
    }

    #[test]
    fn a_pomodoro_follows_its_settings_and_waits_when_asked() {
        let mut m = Timer::new();
        m.config(
            &json!({"timer": {"notify": false, "focus": 50, "autoAdvance": false}}),
            0,
        );
        say(&mut m, "pomodoro", T0);
        assert_eq!(m.countdown.duration, 50.0 * 60_000.0);
        assert_eq!(m.state_glyph(), glyph(BRAIN));
        m.tick(T0 + 50 * MIN + 5);
        assert_eq!(m.status(), Status::Done);
        assert_eq!(m.done_text(), "Next: Short break");
        // A pomodoro waiting for its next phase stays; there is nothing to wake for.
        assert_eq!(m.next_due(T0 + 51 * MIN), None);
        m.tick(T0 + 10 * 60 * MIN);
        assert_eq!(m.status(), Status::Done);
        // The play button starts the break.
        m.event(
            &Ctx::default(),
            "toggle",
            &EventValue::Null(()),
            T0 + 51 * MIN,
        );
        assert_eq!(m.status(), Status::Running);
        assert_eq!(m.state_glyph(), glyph(COFFEE));
    }

    #[test]
    fn a_pomodoro_that_goes_on_says_what_began_in_its_notification() {
        let mut m = Timer::new();
        say(&mut m, "pomodoro", T0);
        m.take_effects();
        m.tick(T0 + 25 * MIN + 5);
        let e = m.take_effects();
        assert_eq!(
            e[1],
            Effect::Notify {
                title: "Focus is over".into(),
                body: "Short break · 5:00".into()
            }
        );
    }

    #[test]
    fn the_buttons_of_the_expanded_view_act() {
        let mut m = quiet();
        let null = EventValue::Null(());
        m.event(&Ctx::default(), "start", &EventValue::Text("15".into()), T0);
        assert_eq!(m.countdown.duration, 15.0 * 60_000.0);
        m.event(&Ctx::default(), "add", &null, T0);
        assert_eq!(m.countdown.duration, 16.0 * 60_000.0);
        m.event(&Ctx::default(), "toggle", &null, T0 + 1000);
        assert_eq!(m.status(), Status::Paused);
        m.event(&Ctx::default(), "stop", &null, T0 + 1000);
        assert_eq!(m.status(), Status::Idle);
        m.event(&Ctx::default(), "pomodoro", &null, T0);
        assert_eq!(m.countdown.kind, Some(Kind::Pomodoro));
        m.event(&Ctx::default(), "start", &EventValue::Null(()), T0);
        m.event(&Ctx::default(), "nonsense", &null, T0);
        assert_eq!(
            m.countdown.kind,
            Some(Kind::Pomodoro),
            "bad events change nothing"
        );
    }

    #[test]
    fn a_done_timer_can_start_again_with_its_label() {
        let mut m = quiet();
        say(&mut m, "1m Tea", T0);
        m.tick(T0 + MIN + 5);
        let labels = texts(&m.view(Slot::Expanded, T0 + MIN + 5).unwrap());
        assert!(labels.contains(&"Time's up".to_string()));
        m.event(
            &Ctx::default(),
            "restart",
            &EventValue::Null(()),
            T0 + MIN + 10,
        );
        assert_eq!(
            (m.status(), m.countdown.label.as_deref()),
            (Status::Running, Some("Tea"))
        );
    }

    #[test]
    fn every_slot_builds_a_valid_tree_in_every_state() {
        let mut m = quiet();
        let check = |m: &Timer, now: u64| {
            for slot in Slot::ALL {
                let _ = m.view(slot, now);
            }
        };
        check(&m, T0);
        say(&mut m, "pomodoro", T0);
        check(&m, T0 + 5);
        say(&mut m, "pause", T0 + 10);
        check(&m, T0 + 10);
        say(&mut m, "stop", T0 + 20);
        say(&mut m, "1m", T0 + 20);
        m.tick(T0 + 20 + MIN + 5);
        check(&m, T0 + 20 + MIN + 5);
    }
}
