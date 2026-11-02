//! Port of `core/Countdown.js` (tests: `tests/countdown.test.mjs`).
//!
//! Timer and pomodoro rules: durations, commands and the countdown state.
//! Pure functions over a [`State`], with the time passed in (epoch ms, as
//! `f64` like the JS numbers) so there is no hidden clock.
//!
//! The JS "no state" (`null`) maps to [`State::idle`]: every function that
//! tolerated `null` there takes an idle state here and answers the same.

use crate::js;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `status` of a [`State`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Idle,
    Running,
    Paused,
    Done,
}

/// `kind` of a [`State`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Timer,
    Pomodoro,
}

/// A pomodoro phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Focus,
    Short,
    Long,
}

impl Phase {
    /// `PHASE_LABELS`.
    fn label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus",
            Phase::Short => "Short break",
            Phase::Long => "Long break",
        }
    }
}

/// The phase to start next, `next` of a done pomodoro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Next {
    pub phase: Phase,
    pub round: i64,
}

/// The countdown state (the JS state object). Times are ms; `ends_at` is
/// epoch ms. `Default` is idle.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Its full length.
    pub duration: f64,
    /// When it ends (running).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<f64>,
    /// What was left when paused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    /// Pomodoro: the focus round, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounds: Option<i64>,
    /// Done pomodoro: the phase to start next, `None` once the cycle is over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<Next>,
}

impl State {
    /// `IDLE`.
    pub fn idle() -> State {
        State::default()
    }
}

/// The pomodoro settings as the config holds them (loose: numbers or
/// numeric strings, minutes). Missing or invalid values mean the defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PomodoroSettings {
    pub focus: Option<Value>,
    pub short_break: Option<Value>,
    pub long_break: Option<Value>,
    pub rounds: Option<Value>,
    pub auto_advance: Option<Value>,
}

/// [`PomodoroSettings`] with the defaults filled in, minutes as ms.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PomodoroConfig {
    pub focus: f64,
    pub short: f64,
    pub long: f64,
    pub rounds: i64,
    pub auto_advance: bool,
}

/// `pomodoroConfig`: the pomodoro settings with their defaults filled in.
pub fn pomodoro_config(cfg: &PomodoroSettings) -> PomodoroConfig {
    fn minutes(v: &Option<Value>, d: f64) -> f64 {
        let n = v.as_ref().map_or(f64::NAN, js::number);
        (if n.is_finite() && n > 0.0 { n } else { d }) * 60000.0
    }
    let r = cfg.rounds.as_ref().map_or(f64::NAN, js::number);
    let r = if r.is_nan() || r == 0.0 { 4.0 } else { r };
    PomodoroConfig {
        focus: minutes(&cfg.focus, 25.0),
        short: minutes(&cfg.short_break, 5.0),
        long: minutes(&cfg.long_break, 15.0),
        rounds: js::round(r).max(1.0) as i64,
        auto_advance: cfg.auto_advance != Some(Value::Bool(false)),
    }
}

fn positive(ms: f64) -> Option<f64> {
    (ms.is_finite() && ms >= 1000.0).then(|| js::round(ms))
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Length in bytes of the `\d+(?:\.\d+)?` at the start of `s` (0: none).
fn number_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return 0;
    }
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 {
            i = j;
        }
    }
    i
}

/// `parseDuration`: a duration as ms, or `None`: "25" (minutes), "90s",
/// "25m", "1h", "1h30m", "1h 30m", "1:30" (m:ss), "1:00:00" (h:mm:ss).
pub fn parse_duration(text: &str) -> Option<f64> {
    let s = text.trim_matches(js::is_ws).to_lowercase();
    if s.is_empty() {
        return None;
    }
    // Minutes: digits with an optional fraction.
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    if all_digits(int) && frac.is_none_or(all_digits) {
        return positive(s.parse::<f64>().ok()? * 60000.0);
    }
    // Clock: m:ss or h:mm:ss.
    let parts: Vec<&str> = s.split(':').collect();
    if (2..=3).contains(&parts.len())
        && all_digits(parts[0])
        && parts[1..].iter().all(|p| all_digits(p) && p.len() <= 2)
    {
        let nums: Vec<f64> = parts.iter().filter_map(|p| p.parse().ok()).collect();
        if nums[1..].iter().any(|&p| p >= 60.0) {
            return None;
        }
        let secs = nums.iter().fold(0.0, |acc, p| acc * 60.0 + p);
        return positive(secs * 1000.0);
    }
    // Units: every character outside a `<number> <unit>` match must be
    // whitespace (the JS replaces the matches and checks the rest is empty).
    let mut total = 0.0;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let n = number_len(rest);
        if n > 0 {
            let after = &rest[n..];
            let tail = after.trim_start_matches(js::is_ws);
            let factor = match tail.chars().next() {
                Some('h') => Some(3_600_000.0),
                Some('m') => Some(60_000.0),
                Some('s') => Some(1000.0),
                _ => None,
            };
            if let Some(f) = factor {
                total += rest[..n].parse::<f64>().ok()? * f;
                i += n + (after.len() - tail.len()) + 1;
                continue;
            }
        }
        let c = rest.chars().next()?;
        if !js::is_ws(c) {
            return None;
        }
        i += c.len_utf8();
    }
    positive(total)
}

/// What a `timer` IPC argument asks for (`parseCommand`'s `{ action, ms?,
/// label? }`); a bad one is the `Err` of [`parse_command`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Command {
    Toggle,
    Pause,
    Resume,
    Stop,
    Skip,
    Pomodoro,
    /// `+5m`: add time.
    Add {
        ms: f64,
    },
    /// `25m Tea`: start a timer, with a label.
    Start {
        ms: f64,
        label: String,
    },
}

const USAGE: &str = "usage: timer <duration> [label] | pomodoro | pause | resume | toggle | +<duration> | skip | stop";

/// `parseCommand`: the command, or the error text to show (`{ error }`).
pub fn parse_command(text: &str) -> Result<Command, String> {
    let s = text.trim_matches(js::is_ws);
    let words: Vec<&str> = s.split(js::is_ws).filter(|w| !w.is_empty()).collect();
    let word = words.first().map_or(String::new(), |w| w.to_lowercase());
    if s.is_empty() || word == "toggle" {
        return Ok(Command::Toggle);
    }
    if words.len() == 1 {
        match word.as_str() {
            "pause" => return Ok(Command::Pause),
            "resume" => return Ok(Command::Resume),
            "stop" => return Ok(Command::Stop),
            "skip" => return Ok(Command::Skip),
            "pomodoro" => return Ok(Command::Pomodoro),
            _ => {}
        }
    }
    if let Some(rest) = s.strip_prefix('+') {
        return match parse_duration(rest) {
            Some(ms) => Ok(Command::Add { ms }),
            None => Err(format!("not a duration: {rest}")),
        };
    }
    // The duration may have spaces ("1h 30m"): the longest leading run of
    // words that parses is the duration, the rest the label.
    for n in (1..=words.len()).rev() {
        if let Some(ms) = parse_duration(&words[..n].join(" ")) {
            return Ok(Command::Start {
                ms,
                label: words[n..].join(" "),
            });
        }
    }
    Err(USAGE.to_string())
}

/// `format`: remaining time as "25:00", "0:05" or "1:02:03", seconds
/// rounded up so it reads 0:00 only at the end.
pub fn format(ms: f64) -> String {
    let ms = if ms.is_nan() { 0.0 } else { ms };
    let total = (ms / 1000.0).ceil().max(0.0) as i64;
    let (h, m, sec) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// `start`: a running timer from `now`.
pub fn start(now: f64, ms: f64, label: &str) -> State {
    State {
        status: Status::Running,
        kind: Some(Kind::Timer),
        label: Some(label.to_string()),
        duration: ms,
        ends_at: Some(now + ms),
        ..State::default()
    }
}

/// `startPhase`: a running pomodoro phase from `now`.
pub fn start_phase(now: f64, cfg: &PomodoroSettings, phase: Phase, round: i64) -> State {
    let c = pomodoro_config(cfg);
    let ms = match phase {
        Phase::Focus => c.focus,
        Phase::Short => c.short,
        Phase::Long => c.long,
    };
    State {
        status: Status::Running,
        kind: Some(Kind::Pomodoro),
        label: Some(phase.label().to_string()),
        phase: Some(phase),
        round: Some(round),
        rounds: Some(c.rounds),
        duration: ms,
        ends_at: Some(now + ms),
        ..State::default()
    }
}

/// `nextPhase`: the pomodoro phase after `phase`: focus rounds with a short
/// break between them, a long break after the last; `None` after the long
/// break (the cycle is over).
pub fn next_phase(phase: Phase, round: i64, rounds: i64) -> Option<Next> {
    match phase {
        Phase::Focus if round >= rounds => Some(Next {
            phase: Phase::Long,
            round,
        }),
        Phase::Focus => Some(Next {
            phase: Phase::Short,
            round,
        }),
        Phase::Short => Some(Next {
            phase: Phase::Focus,
            round: round + 1,
        }),
        Phase::Long => None,
    }
}

/// `remaining`: ms left at `now`.
pub fn remaining(s: &State, now: f64) -> f64 {
    match s.status {
        Status::Running => (s.ends_at.unwrap_or(f64::NAN) - now).max(0.0),
        Status::Paused => s.left.unwrap_or(f64::NAN),
        _ => 0.0,
    }
}

/// `untilChange`: ms until the clock [`format`] shows changes (5 ms past
/// it, so the rounded-up second has turned); 1 once the time is up; 1000
/// when not running.
pub fn until_change(s: &State, now: f64) -> f64 {
    if s.status != Status::Running {
        return 1000.0;
    }
    let left = s.ends_at.unwrap_or(f64::NAN) - now;
    if left <= 0.0 {
        return 1.0;
    }
    let m = left % 1000.0;
    (if m == 0.0 { 1000.0 } else { m }) + 5.0
}

/// `progress`: how far along, 0 to 1.
pub fn progress(s: &State, now: f64) -> f64 {
    if s.duration == 0.0 || s.duration.is_nan() {
        return 0.0;
    }
    if s.status == Status::Done {
        return 1.0;
    }
    (1.0 - remaining(s, now) / s.duration).clamp(0.0, 1.0)
}

/// `pause`: a running state pauses; anything else is as it was.
pub fn pause(s: &State, now: f64) -> State {
    if s.status != Status::Running {
        return s.clone();
    }
    State {
        status: Status::Paused,
        left: Some(remaining(s, now)),
        ends_at: None,
        ..s.clone()
    }
}

/// `resume`: a paused state runs again.
pub fn resume(s: &State, now: f64) -> State {
    if s.status != Status::Paused {
        return s.clone();
    }
    State {
        status: Status::Running,
        ends_at: Some(now + s.left.unwrap_or(f64::NAN)),
        left: None,
        ..s.clone()
    }
}

/// `toggle`: pause or resume; a done pomodoro starts its next phase; idle or
/// a done timer stays as it is.
pub fn toggle(s: &State, now: f64, cfg: &PomodoroSettings) -> State {
    match (s.status, s.next) {
        (Status::Running, _) => pause(s, now),
        (Status::Paused, _) => resume(s, now),
        (Status::Done, Some(n)) => start_phase(now, cfg, n.phase, n.round),
        _ => s.clone(),
    }
}

/// `add`: more time on a running or paused state (the JS takes `now` too,
/// unused).
pub fn add(s: &State, ms: f64) -> State {
    match s.status {
        Status::Running => State {
            ends_at: s.ends_at.map(|e| e + ms),
            duration: s.duration + ms,
            ..s.clone()
        },
        Status::Paused => State {
            left: s.left.map(|l| l + ms),
            duration: s.duration + ms,
            ..s.clone()
        },
        _ => s.clone(),
    }
}

/// `finish` (internal there): the state once its time is up. A timer is
/// done; a pomodoro moves on to its next phase (running with `autoAdvance`,
/// else done and waiting), or is done after the long break.
fn finish(s: &State, now: f64, cfg: &PomodoroSettings) -> State {
    let done = State {
        status: Status::Done,
        ends_at: None,
        left: None,
        ..s.clone()
    };
    if s.kind != Some(Kind::Pomodoro) {
        return done;
    }
    let c = pomodoro_config(cfg);
    let next = next_phase(
        s.phase.unwrap_or(Phase::Focus),
        s.round.unwrap_or(1),
        c.rounds,
    );
    match next {
        Some(n) if c.auto_advance => start_phase(now, cfg, n.phase, n.round),
        _ => State { next, ..done },
    }
}

/// `skip`: a pomodoro's next phase, running; anything else stops.
pub fn skip(s: &State, now: f64, cfg: &PomodoroSettings) -> State {
    if s.kind != Some(Kind::Pomodoro) || s.status == Status::Idle {
        return State::idle();
    }
    let next = if s.status == Status::Done {
        s.next
    } else {
        next_phase(
            s.phase.unwrap_or(Phase::Focus),
            s.round.unwrap_or(1),
            pomodoro_config(cfg).rounds,
        )
    };
    next.map_or_else(State::idle, |n| start_phase(now, cfg, n.phase, n.round))
}

/// The result of [`tick`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tick {
    pub state: State,
    /// Its time just ran out (the module peeks and notifies then).
    pub finished: bool,
}

/// `tick`: the state at `now`.
pub fn tick(s: &State, now: f64, cfg: &PomodoroSettings) -> Tick {
    if s.status == Status::Running && now >= s.ends_at.unwrap_or(f64::INFINITY) {
        return Tick {
            state: finish(s, now, cfg),
            finished: true,
        };
    }
    Tick {
        state: s.clone(),
        finished: false,
    }
}

/// `apply`: a parsed command on the state. `cfg` holds the pomodoro settings.
pub fn apply(s: &State, cmd: &Command, now: f64, cfg: &PomodoroSettings) -> State {
    match cmd {
        Command::Start { ms, label } => start(now, *ms, label),
        Command::Pomodoro => start_phase(now, cfg, Phase::Focus, 1),
        Command::Pause => pause(s, now),
        Command::Resume => resume(s, now),
        Command::Toggle => toggle(s, now, cfg),
        Command::Add { ms } => add(s, *ms),
        Command::Skip => skip(s, now, cfg),
        Command::Stop => State::idle(),
    }
}

/// `title`: its label, a pomodoro's round, or "Timer".
pub fn title(s: &State) -> String {
    if s.status == Status::Idle {
        return "Timer".to_string();
    }
    let label = s.label.clone().unwrap_or_default();
    if s.kind == Some(Kind::Pomodoro) {
        return match (s.phase, s.round, s.rounds) {
            (Some(Phase::Focus), Some(r), Some(n)) => format!("{label} · {r} of {n}"),
            _ => label,
        };
    }
    if label.is_empty() {
        "Timer".to_string()
    } else {
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MIN: f64 = 60000.0;

    fn settings(v: Value) -> PomodoroSettings {
        serde_json::from_value(v).unwrap()
    }

    // parseDuration reads minutes, units and clock forms
    #[test]
    fn parse_duration_reads_minutes_units_and_clock_forms() {
        assert_eq!(parse_duration("25"), Some(25.0 * MIN));
        assert_eq!(parse_duration("25m"), Some(25.0 * MIN));
        assert_eq!(parse_duration("90s"), Some(90000.0));
        assert_eq!(parse_duration("1h30m"), Some(90.0 * MIN));
        assert_eq!(parse_duration("1h 30m"), Some(90.0 * MIN));
        assert_eq!(parse_duration("1.5h"), Some(90.0 * MIN));
        assert_eq!(parse_duration("1:30"), Some(90000.0));
        assert_eq!(parse_duration("1:00:00"), Some(60.0 * MIN));
        for bad in ["", "tea", "1:75", "0", "10x", "m", "0s"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }

    // parseCommand: actions, added time and a duration with a label
    #[test]
    fn parse_command_actions_added_time_and_a_duration_with_a_label() {
        assert_eq!(parse_command(""), Ok(Command::Toggle));
        assert_eq!(parse_command("PAUSE"), Ok(Command::Pause));
        assert_eq!(parse_command("pomodoro"), Ok(Command::Pomodoro));
        assert_eq!(parse_command("+5m"), Ok(Command::Add { ms: 5.0 * MIN }));
        assert_eq!(
            parse_command("25m"),
            Ok(Command::Start {
                ms: 25.0 * MIN,
                label: "".into()
            })
        );
        assert_eq!(
            parse_command("3m Green tea"),
            Ok(Command::Start {
                ms: 3.0 * MIN,
                label: "Green tea".into()
            })
        );
        assert_eq!(
            parse_command("1h 30m Deep work"),
            Ok(Command::Start {
                ms: 90.0 * MIN,
                label: "Deep work".into()
            })
        );
        assert!(parse_command("soon").is_err());
        assert!(parse_command("+later").is_err());
    }

    // format rounds seconds up and shows hours only when needed
    #[test]
    fn format_rounds_seconds_up_and_shows_hours_only_when_needed() {
        assert_eq!(format(25.0 * MIN), "25:00");
        assert_eq!(format(4001.0), "0:05");
        assert_eq!(format(0.0), "0:00");
        assert_eq!(format(3_723_000.0), "1:02:03");
    }

    // a timer runs, pauses, resumes, takes more time and ends
    #[test]
    fn a_timer_runs_pauses_resumes_takes_more_time_and_ends() {
        let cfg = PomodoroSettings::default();
        let mut s = start(1000.0, 10000.0, "Tea");
        assert_eq!(remaining(&s, 4000.0), 7000.0);
        assert_eq!(progress(&s, 6000.0), 0.5);
        s = pause(&s, 4000.0);
        assert_eq!(s.status, Status::Paused);
        assert_eq!(remaining(&s, 99999.0), 7000.0, "paused time stands still");
        s = toggle(&s, 10000.0, &cfg);
        assert_eq!(s.status, Status::Running);
        assert_eq!(s.ends_at, Some(17000.0));
        s = add(&s, 5000.0);
        assert_eq!(s.ends_at, Some(22000.0));
        assert_eq!(s.duration, 15000.0);
        assert!(!tick(&s, 21999.0, &cfg).finished);
        let t = tick(&s, 22000.0, &cfg);
        assert!(t.finished);
        assert_eq!(t.state.status, Status::Done);
        assert_eq!(t.state.label.as_deref(), Some("Tea"));
        assert!(!tick(&t.state, 30000.0, &cfg).finished, "finishes once");
        assert_eq!(
            toggle(&t.state, 30000.0, &cfg).status,
            Status::Done,
            "a done timer does not restart"
        );
    }

    // pomodoro: focus and short breaks, a long break after the last round, then done
    #[test]
    fn pomodoro_focus_short_breaks_long_break_after_the_last_round_then_done() {
        let cfg = settings(json!({ "focus": 25, "shortBreak": 5, "longBreak": 15, "rounds": 2 }));
        let mut s = apply(&State::idle(), &Command::Pomodoro, 0.0, &cfg);
        assert_eq!(
            (s.phase, s.round, s.duration),
            (Some(Phase::Focus), Some(1), 25.0 * MIN)
        );
        assert_eq!(title(&s), "Focus · 1 of 2");
        let mut phases = vec![];
        for _ in 0..4 {
            let t = tick(&s, s.ends_at.unwrap(), &cfg);
            assert!(t.finished);
            s = t.state;
            phases.push((s.status, s.phase.unwrap(), s.round.unwrap()));
        }
        assert_eq!(
            phases,
            [
                (Status::Running, Phase::Short, 1),
                (Status::Running, Phase::Focus, 2),
                (Status::Running, Phase::Long, 2),
                (Status::Done, Phase::Long, 2),
            ]
        );
        assert_eq!(s.next, None, "the cycle is over");
    }

    // pomodoro without autoAdvance waits for a toggle to start the next phase
    #[test]
    fn pomodoro_without_auto_advance_waits_for_a_toggle_to_start_the_next_phase() {
        let cfg = settings(json!({ "autoAdvance": false }));
        let mut s = start_phase(0.0, &cfg, Phase::Focus, 1);
        s = tick(&s, s.ends_at.unwrap(), &cfg).state;
        assert_eq!(s.status, Status::Done);
        assert_eq!(
            s.next,
            Some(Next {
                phase: Phase::Short,
                round: 1
            })
        );
        s = toggle(&s, 100.0, &cfg);
        assert_eq!(
            (s.status, s.phase, s.duration),
            (Status::Running, Some(Phase::Short), 5.0 * MIN)
        );
    }

    // skip moves a pomodoro on and stops a timer; stop always stops
    #[test]
    fn skip_moves_a_pomodoro_on_and_stops_a_timer_stop_always_stops() {
        let cfg = PomodoroSettings::default();
        let p = start_phase(0.0, &cfg, Phase::Focus, 1);
        assert_eq!(skip(&p, 10.0, &cfg).phase, Some(Phase::Short));
        assert_eq!(
            skip(&start(0.0, 5000.0, ""), 10.0, &cfg).status,
            Status::Idle
        );
        assert_eq!(apply(&p, &Command::Stop, 10.0, &cfg).status, Status::Idle);
        assert_eq!(
            pause(&State::idle(), 0.0).status,
            Status::Idle,
            "idle ignores pause"
        );
    }

    // pomodoroConfig fills in the defaults
    #[test]
    fn pomodoro_config_fills_in_the_defaults() {
        assert_eq!(
            pomodoro_config(&settings(json!({}))),
            PomodoroConfig {
                focus: 25.0 * MIN,
                short: 5.0 * MIN,
                long: 15.0 * MIN,
                rounds: 4,
                auto_advance: true
            }
        );
        assert_eq!(
            pomodoro_config(&settings(json!({ "focus": 50, "rounds": 0 }))).focus,
            50.0 * MIN
        );
        assert_eq!(pomodoro_config(&settings(json!({ "rounds": 0 }))).rounds, 4);
    }

    // untilChange: the time until the shown second changes, and the end itself
    #[test]
    fn until_change_the_time_until_the_shown_second_changes_and_the_end_itself() {
        let now = 1_000_000.0;
        let s = start(now, 2500.0, "");
        assert_eq!(format(remaining(&s, now)), "0:03");
        assert_eq!(until_change(&s, now), 505.0);
        assert_eq!(format(remaining(&s, now + 505.0)), "0:02");
        assert_eq!(until_change(&s, now + 505.0), 1000.0);
        assert_eq!(until_change(&s, now + 1505.0), 1000.0);
        assert!(
            now + 1505.0 + 1000.0 >= s.ends_at.unwrap(),
            "the last wake-up is at or past the end"
        );
        assert_eq!(until_change(&start(now, 2.0 * MIN, ""), now), 1005.0);
        assert_eq!(
            until_change(&s, s.ends_at.unwrap() + 50.0),
            1.0,
            "past the end it wakes at once"
        );
        assert_eq!(until_change(&pause(&s, now), now), 1000.0);
        assert_eq!(until_change(&State::idle(), now), 1000.0);
    }
}
