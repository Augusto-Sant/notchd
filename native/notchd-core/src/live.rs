//! Port of `core/Live.js` (tests: `tests/live.test.mjs`).
//!
//! The rules of a spoken conversation with an agent: the bridge's command
//! line, what its lines mean, the conversation's state and what the notch
//! says about it, when a session runs, and the IPC commands. The bridges are
//! `core/live/codex-live.py` and `core/live/local-live.py`; their lines are
//! documented there.
//!
//! The bridge's lines are flat objects whose keys may come together
//! (`{"working": true, "level": -30}`), so [`Event`] is a struct of optional
//! fields rather than an enum of messages; each field is read leniently, a
//! value of the wrong type being ignored, as `reduce` does in JS.

use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// The transcript keeps this many lines; older ones go (`MAX_LINES`).
pub const MAX_LINES: usize = 40;
/// Codex's voice, in dB, above which it is speaking (`SPEAKING_DB`).
pub const SPEAKING_DB: f64 = -60.0;
/// Level of silence, in dB.
const SILENCE_DB: f64 = -100.0;

// ---- JS semantics the wire format leans on --------------------------------

/// JS `String.prototype.trim`: Unicode white space plus the BOM.
fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// JS truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JS `String(value)` of a JSON value.
fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
            _ => n.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                if x.is_null() {
                    String::new()
                } else {
                    js_string(x)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// `String(v || "")`.
fn string_or_empty(v: &Value) -> String {
    if truthy(v) {
        js_string(v)
    } else {
        String::new()
    }
}

/// A field of the type asked for, else `None` (the JS `typeof` guards).
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Ok(T::deserialize(Value::deserialize(d)?).ok())
}

/// `Array.isArray(x) ? x.map(String) : none`.
fn string_list<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => Some(a.iter().map(js_string).collect()),
        _ => None,
    })
}

fn de_truthy<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(truthy(&Value::deserialize(d)?))
}

fn de_string_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(string_or_empty(&Value::deserialize(d)?))
}

// ---- the bridge's command line --------------------------------------------

/// `["python3", b]` for a `.py` script, `[b]` for anything else (a test's fake).
fn runner(bridge: &str) -> Vec<String> {
    if bridge.ends_with(".py") {
        vec!["python3".to_owned(), bridge.to_owned()]
    } else {
        vec![bridge.to_owned()]
    }
}

/// `"~"` and `"~/x"` under `home`.
fn expand_tilde(path: &str, home: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        format!("{home}{}", &path[1..])
    } else {
        path.to_owned()
    }
}

fn is_workspace_write(access: &Option<String>) -> bool {
    access.as_deref() == Some("workspace-write")
}

/// The options of [`bridge_args`] (the Codex realtime bridge). Empty is the
/// bridge's own default.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct BridgeOpts {
    /// The Codex command, `codex` when empty.
    pub codex: Option<String>,
    /// The folder; `~` is the home; empty leaves it to the bridge.
    pub cwd: Option<String>,
    /// The voice's name.
    pub voice: Option<String>,
    /// `workspace-write`, else read-only.
    pub access: Option<String>,
}

/// The bridge's command line (`bridgeArgs`): `bridge` is the script (or a
/// test's fake), `home` expands a leading `~` of the folder.
pub fn bridge_args(bridge: &str, opts: &BridgeOpts, home: &str) -> Vec<String> {
    let mut run = runner(bridge);
    let codex = js_trim(opts.codex.as_deref().unwrap_or(""));
    run.extend([
        "--codex".to_owned(),
        if codex.is_empty() { "codex" } else { codex }.to_owned(),
    ]);
    let cwd = expand_tilde(js_trim(opts.cwd.as_deref().unwrap_or("")), home);
    if !cwd.is_empty() {
        run.extend(["--cwd".to_owned(), cwd]);
    }
    if let Some(voice) = opts.voice.as_deref().filter(|v| !v.is_empty()) {
        run.extend(["--voice".to_owned(), voice.to_owned()]);
    }
    let access = if is_workspace_write(&opts.access) {
        "workspace-write"
    } else {
        "read-only"
    };
    run.extend(["--access".to_owned(), access.to_owned()]);
    run
}

/// The model names an older version stored (the one voice each engine had)
/// and the engine they mean (`LEGACY_VOICE_MODELS`).
pub const LEGACY_VOICE_MODELS: [(&str, &str); 2] = [
    ("voice-pt_BR-faber-medium", "piper"),
    ("pocket-pt", "pocket"),
];

/// The engine a `voiceModel` setting means (`voiceEngine`): `"piper"` and
/// `"pocket"`, or the engine of a legacy model name; `None` (JS `""`) for any
/// other, which is a model by its name.
pub fn voice_engine(value: &str) -> Option<&'static str> {
    match js_trim(value) {
        "piper" => Some("piper"),
        "pocket" => Some("pocket"),
        v => LEGACY_VOICE_MODELS
            .iter()
            .find(|(m, _)| *m == v)
            .map(|(_, e)| *e),
    }
}

/// The options of [`local_bridge_args`] (`core/live/local-live.py`). What is
/// empty is the bridge's own default. `other_voice_model` (a model of your
/// own, by name) wins over `voice_model` (an engine, `piper` or `pocket`,
/// speaking `voice_language`). Keys the bridge doesn't know are ignored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalBridgeOpts {
    /// `claude`, else `codex`.
    pub agent: Option<String>,
    /// The agent's command line.
    pub command: Option<String>,
    /// `last` (default), `new` or an id.
    pub session: Option<String>,
    /// The folder; `~` is the home.
    pub cwd: Option<String>,
    /// `workspace-write`, else read-only.
    pub access: Option<String>,
    /// The speech language.
    pub lang: Option<String>,
    /// Voxtype's model.
    pub speech_model: Option<String>,
    /// An engine, or a model by name.
    pub voice_model: Option<String>,
    /// The voice's language.
    pub voice_language: Option<String>,
    /// A LocalAI model of your own, by name.
    pub other_voice_model: Option<String>,
    /// One of the model's voices.
    pub voice: Option<String>,
    /// LocalAI's address.
    pub url: Option<String>,
}

/// The same for the local bridge (`localBridgeArgs`).
pub fn local_bridge_args(bridge: &str, opts: &LocalBridgeOpts, home: &str) -> Vec<String> {
    let text = |v: &Option<String>| js_trim(v.as_deref().unwrap_or("")).to_owned();
    let mut run = runner(bridge);
    let mut flag = |name: &str, value: String| run.extend([name.to_owned(), value]);
    flag(
        "--agent",
        if opts.agent.as_deref() == Some("claude") {
            "claude"
        } else {
            "codex"
        }
        .to_owned(),
    );
    let command = text(&opts.command);
    if !command.is_empty() {
        flag("--command", command);
    }
    let session = text(&opts.session);
    flag(
        "--session",
        if session.is_empty() {
            "last".to_owned()
        } else {
            session
        },
    );
    let cwd = expand_tilde(&text(&opts.cwd), home);
    if !cwd.is_empty() {
        flag("--cwd", cwd);
    }
    flag(
        "--access",
        if is_workspace_write(&opts.access) {
            "workspace-write"
        } else {
            "read-only"
        }
        .to_owned(),
    );
    for (name, value) in [
        ("--lang", &opts.lang),
        ("--speech-model", &opts.speech_model),
    ] {
        let v = text(value);
        if !v.is_empty() {
            flag(name, v);
        }
    }
    let model = text(&opts.other_voice_model);
    let voice_model = text(&opts.voice_model);
    if !model.is_empty() {
        flag("--voice-model", model);
    } else if let Some(engine) = voice_engine(&voice_model) {
        flag("--voice-engine", engine.to_owned());
    } else if !voice_model.is_empty() {
        flag("--voice-model", voice_model);
    }
    for (name, value) in [
        ("--voice-lang", &opts.voice_language),
        ("--voice", &opts.voice),
        ("--localai-url", &opts.url),
    ] {
        let v = text(value);
        if !v.is_empty() {
            flag(name, v);
        }
    }
    run
}

// ---- the bridge's lines ---------------------------------------------------

/// Which side of the conversation a transcript line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// What you said.
    User,
    /// What the agent said.
    Assistant,
}

fn de_role<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Role>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) if s == "user" => Some(Role::User),
        _ => None,
    })
}

/// One piece of the transcript, as the bridge sends it: a `delta` on the open
/// line of its role, or `done` with the line's whole `text`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct TranscriptPiece {
    /// `user`; anything else is the assistant.
    #[serde(default, deserialize_with = "de_role")]
    pub role: Option<Role>,
    /// Any truthy value closes the line.
    #[serde(default, deserialize_with = "de_truthy")]
    pub done: bool,
    /// The whole text (`String(text || "")`).
    #[serde(default, deserialize_with = "de_string_or_empty")]
    pub text: String,
    /// The words to append (`String(delta || "")`).
    #[serde(default, deserialize_with = "de_string_or_empty")]
    pub delta: String,
}

/// `transcript` is read when it is an object; anything else is ignored (an
/// array changes nothing in JS either).
fn de_transcript<'de, D: Deserializer<'de>>(d: D) -> Result<Option<TranscriptPiece>, D::Error> {
    Ok(match Value::deserialize(d)? {
        v @ Value::Object(_) => TranscriptPiece::deserialize(v).ok(),
        _ => None,
    })
}

/// A line of the bridge, every field optional and of the type the protocol
/// says or else absent.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Event {
    /// `starting`, `connecting`, `live` or `closed` (any string is kept).
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
    /// What went wrong; the packages and components below go with it.
    #[serde(default, deserialize_with = "lenient")]
    pub error: Option<String>,
    /// System packages that would fix the error (each stringified).
    #[serde(default, deserialize_with = "string_list")]
    pub packages: Option<Vec<String>>,
    /// Component ids (`core/components/components.json`) that would fix it.
    #[serde(default, deserialize_with = "string_list")]
    pub components: Option<Vec<String>>,
    /// A warning for the notch.
    #[serde(default, deserialize_with = "lenient")]
    pub warning: Option<String>,
    /// A turn is in flight.
    #[serde(default, deserialize_with = "lenient")]
    pub working: Option<bool>,
    /// The microphone is muted.
    #[serde(default, deserialize_with = "lenient")]
    pub muted: Option<bool>,
    /// The voice as it plays, in dB.
    #[serde(default, deserialize_with = "lenient")]
    pub level: Option<f64>,
    /// What the microphone hears, in dB.
    #[serde(default, deserialize_with = "lenient")]
    pub mic: Option<f64>,
    /// A piece of the transcript.
    #[serde(default, deserialize_with = "de_transcript")]
    pub transcript: Option<TranscriptPiece>,
}

/// A line of the bridge to its event, or `None` for anything else
/// (`parseLine`): invalid JSON, or JSON that is not an object.
pub fn parse_line(line: &str) -> Option<Event> {
    match serde_json::from_str::<Value>(line).ok()? {
        v @ Value::Object(_) => Some(Event::deserialize(v).unwrap_or_default()),
        _ => None,
    }
}

// ---- the conversation's state ---------------------------------------------

/// A line of the transcript.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    /// Whose words.
    pub role: Role,
    /// What was said so far.
    pub text: String,
    /// Whether the line is closed.
    pub done: bool,
}

/// The conversation's state.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct State {
    /// `off`, `starting`, `connecting`, `live` or `closed`.
    pub status: String,
    /// What went wrong, empty when nothing did.
    pub error: String,
    /// Packages to install, from the error.
    pub packages: Vec<String>,
    /// Components to install, from the error.
    pub components: Vec<String>,
    /// A warning, empty when none.
    pub warning: String,
    /// The transcript, oldest first.
    pub transcript: Vec<Line>,
    /// A turn is in flight.
    pub working: bool,
    /// The microphone is muted.
    pub muted: bool,
    /// The agent's voice in dB as it plays.
    pub level: f64,
    /// What the microphone sends, in dB.
    pub mic: f64,
}

impl Default for State {
    /// The state with no session (`initialState`).
    fn default() -> Self {
        State {
            status: "off".to_owned(),
            error: String::new(),
            packages: Vec::new(),
            components: Vec::new(),
            warning: String::new(),
            transcript: Vec::new(),
            working: false,
            muted: false,
            level: SILENCE_DB,
            mic: SILENCE_DB,
        }
    }
}

/// The state with no session (`initialState`).
pub fn initial_state() -> State {
    State::default()
}

/// A new session (`starting`): the old one's transcript stays, its lines
/// closed (one left open would take the next session's words), its errors gone.
pub fn starting(state: &State) -> State {
    State {
        transcript: state
            .transcript
            .iter()
            .map(|l| Line {
                done: true,
                ..l.clone()
            })
            .collect(),
        status: "starting".to_owned(),
        ..State::default()
    }
}

/// The state after one of the bridge's events (`reduce`). A line that is not
/// an event ([`parse_line`] gave `None`) changes nothing: skip it.
pub fn reduce(state: &State, e: &Event) -> State {
    let mut s = state.clone();
    if let Some(status) = &e.status {
        s.status = status.clone();
        if status == "closed" {
            s.working = false;
            s.level = SILENCE_DB;
            s.mic = SILENCE_DB;
        }
    }
    if let Some(error) = &e.error {
        s.error = error.clone();
        s.packages = e.packages.clone().unwrap_or_default();
        s.components = e.components.clone().unwrap_or_default();
    }
    if let Some(w) = &e.warning {
        s.warning = w.clone();
    }
    if let Some(w) = e.working {
        s.working = w;
    }
    if let Some(m) = e.muted {
        s.muted = m;
    }
    if let Some(l) = e.level {
        s.level = l;
    }
    if let Some(m) = e.mic {
        s.mic = m;
    }
    if let Some(t) = &e.transcript {
        s.transcript = add_transcript(&s.transcript, t);
    }
    s
}

/// A piece of the transcript into the lines (`addTranscript`): a delta goes
/// on its role's open line (or starts one), `done` sets that line's whole
/// text. Both sides can be open at once (Codex starts answering before what
/// you said is done). Only the last [`MAX_LINES`] are kept.
pub fn add_transcript(lines: &[Line], t: &TranscriptPiece) -> Vec<Line> {
    let role = t.role.unwrap_or(Role::Assistant);
    let mut out = lines.to_vec();
    let open = out.iter().rposition(|l| l.role == role && !l.done);
    if t.done {
        let text = js_trim(&t.text).to_owned();
        match open {
            Some(i) => {
                out[i] = Line {
                    role,
                    text,
                    done: true,
                }
            }
            None if !text.is_empty() => out.push(Line {
                role,
                text,
                done: true,
            }),
            None => {}
        }
    } else {
        if t.delta.is_empty() {
            return out;
        }
        match open {
            Some(i) => {
                out[i] = Line {
                    role,
                    text: format!("{}{}", out[i].text, t.delta),
                    done: false,
                }
            }
            None => out.push(Line {
                role,
                text: t.delta.trim_start().to_owned(),
                done: false,
            }),
        }
    }
    let keep = out.len().saturating_sub(MAX_LINES);
    out.split_off(keep)
}

/// Whether the agent's voice is playing (`speaking`): live, above the threshold.
pub fn speaking(state: &State) -> bool {
    state.status == "live" && state.level > SPEAKING_DB
}

/// How loud the microphone is, 0 (silence, the room) to 1 (`micLevel`), for
/// the dot that shows it hears you: -60 dB and under is nothing, -20 dB is full.
pub fn mic_level(state: &State) -> f64 {
    if state.status != "live" || state.muted {
        return 0.0;
    }
    ((state.mic + 60.0) / 40.0).clamp(0.0, 1.0)
}

/// Whether a session is on (`running`): starting, connecting or live.
pub fn running(state: &State) -> bool {
    matches!(state.status.as_str(), "starting" | "connecting" | "live")
}

/// One word or two for the notch: what the conversation is doing (`statusText`).
pub fn status_text(state: &State) -> &'static str {
    if !state.error.is_empty() {
        return "Stopped";
    }
    match state.status.as_str() {
        "starting" | "connecting" => "Connecting…",
        "live" if speaking(state) => "Speaking",
        "live" if state.working => "Working",
        "live" if state.muted => "Muted",
        "live" => "Listening",
        "closed" => "Ended",
        _ => "Off",
    }
}

/// What went wrong, with what to install when that's why (`errorText`).
/// `hint` is what to do about the components (`Components.missingHint`),
/// which only the side that has the catalog can build; without it (or empty)
/// the ids are named. Packages without components (an older bridge) are named
/// as they are.
pub fn error_text(state: &State, hint: Option<&str>) -> String {
    if state.error.is_empty() {
        return String::new();
    }
    if !state.components.is_empty() {
        let hint: Cow<str> = match hint.filter(|h| !h.is_empty()) {
            Some(h) => h.into(),
            None => format!("Install {} in the settings.", state.components.join(", ")).into(),
        };
        return format!("{} {hint}", state.error);
    }
    if !state.packages.is_empty() {
        return format!("{} Install {}.", state.error, state.packages.join(", "));
    }
    state.error.clone()
}

/// Whether a session should run (`wanted`): the notch is open on the module
/// (pinned: a click or a shortcut, not a hover) and you haven't stopped it
/// since it opened.
pub fn wanted(open: bool, stopped: bool) -> bool {
    open && !stopped
}

// ---- IPC ------------------------------------------------------------------

/// What an IPC command asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Opens its notch, which starts a session.
    Start,
    /// Stops the session.
    Stop,
    /// Starts or stops.
    Toggle,
    /// A new session, the transcript cleared.
    New,
    /// Mutes the microphone.
    Mute,
    /// Unmutes it.
    Unmute,
    /// Anything else.
    Usage,
}

/// A parsed IPC command (`{ action, text }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Command {
    /// What it asks for.
    pub action: Action,
    /// Always empty today.
    pub text: String,
}

/// An IPC command (`parseCommand`): one of the words, the rest is `Usage`.
pub fn parse_command(raw: &str) -> Command {
    let action = match js_trim(raw) {
        "start" => Action::Start,
        "stop" => Action::Stop,
        "toggle" => Action::Toggle,
        "new" => Action::New,
        "mute" => Action::Mute,
        "unmute" => Action::Unmute,
        _ => Action::Usage,
    };
    Command {
        action,
        text: String::new(),
    }
}

/// `decodeURIComponent`; `None` where JS throws (a bad `%` escape, or bytes
/// that are not UTF-8).
fn decode_uri_component(s: &str) -> Option<String> {
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

/// The bridge's file as a path (`localPath`): `Qt.resolvedUrl` gives `file://`
/// urls, or under Quickshell `qs:@/qs/<path>` (resolved against `shell_dir`,
/// empty for none) and `qs:///<absolute path>` (the Omarchy shell, for a
/// plugin's core). An empty string for anything else, and for a url with a
/// malformed escape (where JS throws).
pub fn local_path(url: &str, shell_dir: &str) -> String {
    let path = if let Some(p) = url.strip_prefix("file://") {
        p.to_owned()
    } else if let (Some(p), false) = (url.strip_prefix("qs:@/qs/"), shell_dir.is_empty()) {
        let dir = shell_dir.strip_suffix('/').unwrap_or(shell_dir);
        format!("{dir}/{p}")
    } else if url.starts_with("qs:///") {
        url[5..].to_owned()
    } else {
        return String::new();
    };
    decode_uri_component(&path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::parse_catalog;
    use serde_json::json;

    fn run(events: &[Value]) -> State {
        events.iter().fold(initial_state(), |s, e| {
            reduce(&s, &Event::deserialize(e.clone()).expect("an event"))
        })
    }

    fn texts(lines: &[Line]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    fn opts(v: Value) -> LocalBridgeOpts {
        serde_json::from_value(v).expect("options")
    }

    // bridgeArgs: python for the script, the fake as is; options in order
    #[test]
    fn bridge_args_python_for_the_script_the_fake_as_is_options_in_order() {
        assert_eq!(
            bridge_args("/c/live/codex-live.py", &BridgeOpts::default(), "/home/u"),
            s(&[
                "python3",
                "/c/live/codex-live.py",
                "--codex",
                "codex",
                "--access",
                "read-only"
            ])
        );
        let o = BridgeOpts {
            codex: Some(" codex -p work ".into()),
            cwd: Some("~/code".into()),
            voice: Some("maple".into()),
            access: Some("workspace-write".into()),
        };
        assert_eq!(
            bridge_args("/t/bin/codex-live", &o, "/home/u"),
            s(&[
                "/t/bin/codex-live",
                "--codex",
                "codex -p work",
                "--cwd",
                "/home/u/code",
                "--voice",
                "maple",
                "--access",
                "workspace-write"
            ])
        );
    }

    // bridgeArgs: an empty folder is left to the bridge; an unknown access is read-only
    #[test]
    fn bridge_args_empty_folder_is_left_to_the_bridge_unknown_access_is_read_only() {
        let o = BridgeOpts {
            cwd: Some("  ".into()),
            access: Some("everything".into()),
            ..Default::default()
        };
        let args = bridge_args("b", &o, "/h");
        assert!(!args.contains(&"--cwd".to_owned()));
        assert_eq!(args[args.len() - 2..], s(&["--access", "read-only"])[..]);
    }

    // localBridgeArgs: defaults are the bridge's own; options in order, ~ expanded
    #[test]
    fn local_bridge_args_defaults_are_the_bridges_own_options_in_order_tilde_expanded() {
        assert_eq!(
            local_bridge_args(
                "/c/live/local-live.py",
                &LocalBridgeOpts::default(),
                "/home/u"
            ),
            s(&[
                "python3",
                "/c/live/local-live.py",
                "--agent",
                "codex",
                "--session",
                "last",
                "--access",
                "read-only"
            ])
        );
        let o = opts(json!({
            "agent": "claude", "command": " claude --model x ", "session": " abc ", "cwd": "~/code",
            "access": "workspace-write", "lang": " en ", "speechModel": " small ", "voiceModel": "pocket",
            "voiceLanguage": " en ", "voice": " rafael ", "url": " http://127.0.0.1:8080 "
        }));
        assert_eq!(
            local_bridge_args("/t/bin/local-live", &o, "/home/u"),
            s(&[
                "/t/bin/local-live",
                "--agent",
                "claude",
                "--command",
                "claude --model x",
                "--session",
                "abc",
                "--cwd",
                "/home/u/code",
                "--access",
                "workspace-write",
                "--lang",
                "en",
                "--speech-model",
                "small",
                "--voice-engine",
                "pocket",
                "--voice-lang",
                "en",
                "--voice",
                "rafael",
                "--localai-url",
                "http://127.0.0.1:8080"
            ])
        );
    }

    fn voice_args(o: Value) -> Vec<String> {
        local_bridge_args("b", &opts(o), "/h")[7..].to_vec()
    }

    // localBridgeArgs: a model of your own wins over the engine; the language goes with the engine
    #[test]
    fn local_bridge_args_own_model_wins_over_the_engine_language_goes_with_the_engine() {
        assert_eq!(
            voice_args(json!({"voiceModel": "pocket", "otherVoiceModel": " my-voice "})),
            s(&["--voice-model", "my-voice"])
        );
        assert_eq!(
            voice_args(json!({"voiceModel": "piper", "otherVoiceModel": ""})),
            s(&["--voice-engine", "piper"])
        );
        assert_eq!(
            voice_args(json!({"voiceModel": "piper", "voiceLanguage": "de"})),
            s(&["--voice-engine", "piper", "--voice-lang", "de"])
        );
        assert_eq!(
            voice_args(json!({"voiceModel": "", "otherVoiceModel": "  "})),
            Vec::<String>::new()
        );
        assert_eq!(
            voice_args(json!({"voiceLanguage": "fr"})),
            s(&["--voice-lang", "fr"])
        );
        assert_eq!(
            voice_args(json!({"voiceModel": "some-model"})),
            s(&["--voice-model", "some-model"])
        );
        assert_eq!(
            voice_args(
                json!({"tts": "chatterbox", "whisperModel": "/m.bin", "reference": "/r.wav"})
            ),
            Vec::<String>::new()
        );
    }

    // localBridgeArgs: the model names an older version stored mean their engine, now in the chosen language
    #[test]
    fn local_bridge_args_legacy_model_names_mean_their_engine_in_the_chosen_language() {
        assert_eq!(
            voice_args(json!({"voiceModel": "voice-pt_BR-faber-medium", "voiceLanguage": "en"})),
            s(&["--voice-engine", "piper", "--voice-lang", "en"])
        );
        assert_eq!(
            voice_args(json!({"voiceModel": "pocket-pt"})),
            s(&["--voice-engine", "pocket"])
        );
        // what they map to are the registry's own Portuguese voices
        let catalog = parse_catalog(include_str!("../../../core/components/components.json"));
        for (model, engine) in LEGACY_VOICE_MODELS {
            let c = catalog
                .iter()
                .find(|c| c.model.as_ref().and_then(|m| m.name.as_deref()) == Some(model))
                .unwrap_or_else(|| panic!("{model}"));
            assert_eq!(
                (c.engine.as_deref(), c.language.as_deref()),
                (Some(engine), Some("pt"))
            );
        }
        assert_eq!(voice_engine(" piper "), Some("piper"));
        assert_eq!(voice_engine("hasOwnProperty"), None);
    }

    // localBridgeArgs: an unknown agent is codex, an unknown access read-only
    #[test]
    fn local_bridge_args_unknown_agent_is_codex_unknown_access_read_only() {
        let args = local_bridge_args(
            "b",
            &opts(json!({"agent": "gpt", "access": "all", "session": "  "})),
            "/h",
        );
        assert_eq!(
            args,
            s(&[
                "b",
                "--agent",
                "codex",
                "--session",
                "last",
                "--access",
                "read-only"
            ])
        );
    }

    // errorText: components named by the bridge become the hint, not a make target
    #[test]
    fn error_text_components_named_by_the_bridge_become_the_hint_not_a_make_target() {
        let state = State {
            error: "Voice needs packages that aren't installed.".into(),
            packages: s(&["voxtype-bin"]),
            components: s(&["voxtype"]),
            ..State::default()
        };
        assert_eq!(
            error_text(
                &state,
                Some("Install Voxtype in Settings \u{203a} Components.")
            ),
            "Voice needs packages that aren't installed. Install Voxtype in Settings \u{203a} Components."
        );
        // without the catalog's hint it still names the ids
        assert_eq!(
            error_text(&state, None),
            "Voice needs packages that aren't installed. Install voxtype in the settings."
        );
        // packages from a bridge that names no components
        let older = State {
            error: "x.".into(),
            packages: s(&["a", "b"]),
            ..State::default()
        };
        assert_eq!(error_text(&older, None), "x. Install a, b.");
        assert!(!error_text(&state, Some("h")).contains("make "));
    }

    // parseLine: an object, else null
    #[test]
    fn parse_line_an_object_else_none() {
        assert_eq!(
            parse_line(r#"{"status": "live"}"#),
            Some(Event {
                status: Some("live".into()),
                ..Event::default()
            })
        );
        assert_eq!(parse_line("not json"), None);
        assert_eq!(parse_line("[1]"), None);
        assert_eq!(parse_line("3"), None);
    }

    // (added) parseLine: a field of the wrong type is ignored, not the whole line
    #[test]
    fn parse_line_a_field_of_the_wrong_type_is_ignored() {
        let e = parse_line(r#"{"status": 3, "level": "loud", "muted": true, "packages": ["a", 2], "transcript": "x"}"#)
            .expect("an object");
        assert_eq!(e.status, None);
        assert_eq!(e.level, None);
        assert_eq!(e.muted, Some(true));
        assert_eq!(e.packages, Some(s(&["a", "2"])));
        assert_eq!(e.transcript, None);
    }

    // reduce: the status goes through starting, connecting, live, closed
    #[test]
    fn reduce_status_goes_through_starting_connecting_live_closed() {
        let ev = |v: Value| Event::deserialize(v).expect("an event");
        let mut st = starting(&initial_state());
        assert_eq!(st.status, "starting");
        assert!(running(&st));
        st = reduce(&st, &ev(json!({"status": "live"})));
        assert_eq!(status_text(&st), "Listening");
        st = reduce(&st, &ev(json!({"working": true, "level": -30})));
        assert_eq!(status_text(&st), "Speaking");
        st = reduce(&st, &ev(json!({"level": -100})));
        assert_eq!(status_text(&st), "Working");
        st = reduce(&st, &ev(json!({"status": "closed", "reason": "requested"})));
        assert!(!running(&st));
        assert!(!st.working);
        assert_eq!(st.level, -100.0);
        assert_eq!(status_text(&st), "Ended");
    }

    // reduce: muted says so while listening
    #[test]
    fn reduce_muted_says_so_while_listening() {
        let st = run(&[json!({"status": "live"}), json!({"muted": true})]);
        assert_eq!(status_text(&st), "Muted");
    }

    // reduce: an error, with the packages and components to install
    #[test]
    fn reduce_an_error_with_the_packages_and_components_to_install() {
        let st = run(&[
            json!({"status": "starting"}),
            json!({"error": "Codex Live needs packages that aren't installed.", "packages": ["gst-plugins-bad"], "components": ["codex-live-audio"]}),
            json!({"status": "closed"}),
        ]);
        assert_eq!(status_text(&st), "Stopped");
        assert_eq!(st.components, s(&["codex-live-audio"]));
        assert!(
            error_text(
                &st,
                Some("Install WebRTC and audio in Settings \u{203a} Components.")
            )
            .ends_with("Install WebRTC and audio in Settings \u{203a} Components.")
        );
        assert_eq!(starting(&st).components, Vec::<String>::new());
        assert_eq!(
            error_text(&run(&[json!({"error": "No login."})]), None),
            "No login."
        );
    }

    // starting: a new session keeps the transcript and drops the error
    #[test]
    fn starting_a_new_session_keeps_the_transcript_and_drops_the_error() {
        let before = run(&[
            json!({"transcript": {"role": "user", "text": "oi", "done": true}}),
            json!({"error": "x"}),
            json!({"status": "closed"}),
        ]);
        let st = starting(&before);
        assert_eq!(st.error, "");
        assert_eq!(st.transcript.len(), 1);
        assert_eq!(st.status, "starting");
    }

    // starting: a line the last session left open is closed, not continued
    #[test]
    fn starting_a_line_the_last_session_left_open_is_closed_not_continued() {
        let before = run(&[
            json!({"transcript": {"role": "assistant", "delta": "Ótimo!"}}),
            json!({"status": "closed"}),
        ]);
        let next = reduce(
            &starting(&before),
            &Event::deserialize(json!({"transcript": {"role": "assistant", "delta": "Oi"}}))
                .unwrap(),
        );
        assert_eq!(
            next.transcript,
            vec![
                Line {
                    role: Role::Assistant,
                    text: "Ótimo!".into(),
                    done: true
                },
                Line {
                    role: Role::Assistant,
                    text: "Oi".into(),
                    done: false
                },
            ]
        );
    }

    // transcript: deltas build a line, done sets its text
    #[test]
    fn transcript_deltas_build_a_line_done_sets_its_text() {
        let st = run(&[
            json!({"transcript": {"role": "assistant", "delta": " Oi"}}),
            json!({"transcript": {"role": "assistant", "delta": "! Tudo"}}),
            json!({"transcript": {"role": "assistant", "delta": " bem?"}}),
        ]);
        assert_eq!(
            st.transcript,
            vec![Line {
                role: Role::Assistant,
                text: "Oi! Tudo bem?".into(),
                done: false
            }]
        );
        let done = reduce(
            &st,
            &Event::deserialize(json!({"transcript": {"role": "assistant", "text": " Oi! Tudo bem? ", "done": true}})).unwrap(),
        );
        assert_eq!(
            done.transcript,
            vec![Line {
                role: Role::Assistant,
                text: "Oi! Tudo bem?".into(),
                done: true
            }]
        );
    }

    // transcript: both sides open at once (it answers before what you said is done)
    #[test]
    fn transcript_both_sides_open_at_once() {
        let st = run(&[
            json!({"transcript": {"role": "user", "delta": "Rodei os"}}),
            json!({"transcript": {"role": "user", "delta": " testes"}}),
            json!({"transcript": {"role": "assistant", "delta": "Ót"}}),
            json!({"transcript": {"role": "user", "text": "Rodei os testes.", "done": true}}),
            json!({"transcript": {"role": "assistant", "delta": "imo!"}}),
        ]);
        assert_eq!(
            st.transcript,
            vec![
                Line {
                    role: Role::User,
                    text: "Rodei os testes.".into(),
                    done: true
                },
                Line {
                    role: Role::Assistant,
                    text: "Ótimo!".into(),
                    done: false
                },
            ]
        );
    }

    // transcript: a done line after a done line starts a new one; an empty done adds nothing
    #[test]
    fn transcript_done_after_done_starts_a_new_line_empty_done_adds_nothing() {
        let st = run(&[
            json!({"transcript": {"role": "user", "text": "um", "done": true}}),
            json!({"transcript": {"role": "user", "text": "dois", "done": true}}),
            json!({"transcript": {"role": "user", "text": "  ", "done": true}}),
            json!({"transcript": {"role": "user", "delta": ""}}),
        ]);
        assert_eq!(texts(&st.transcript), ["um", "dois"]);
    }

    // transcript: only the last lines are kept
    #[test]
    fn transcript_only_the_last_lines_are_kept() {
        let events: Vec<Value> = (0..MAX_LINES + 5)
            .map(|i| json!({"transcript": {"role": if i % 2 == 1 { "user" } else { "assistant" }, "text": format!("l{i}"), "done": true}}))
            .collect();
        let st = run(&events);
        assert_eq!(st.transcript.len(), MAX_LINES);
        assert_eq!(
            st.transcript.last().unwrap().text,
            format!("l{}", MAX_LINES + 4)
        );
    }

    // speaking: only live, above the threshold
    #[test]
    fn speaking_only_live_above_the_threshold() {
        assert!(speaking(&run(&[
            json!({"status": "live"}),
            json!({"level": -40})
        ])));
        assert!(!speaking(&run(&[
            json!({"status": "live"}),
            json!({"level": -70})
        ])));
        assert!(!speaking(&run(&[
            json!({"status": "connecting"}),
            json!({"level": -40})
        ])));
    }

    // micLevel: 0 at -60 dB and under, 1 at -20 dB; nothing muted or not live
    #[test]
    fn mic_level_zero_at_minus_60_one_at_minus_20_nothing_muted_or_not_live() {
        assert_eq!(
            mic_level(&run(&[json!({"status": "live"}), json!({"mic": -70})])),
            0.0
        );
        assert_eq!(
            mic_level(&run(&[json!({"status": "live"}), json!({"mic": -40})])),
            0.5
        );
        assert_eq!(
            mic_level(&run(&[json!({"status": "live"}), json!({"mic": -5})])),
            1.0
        );
        assert_eq!(
            mic_level(&run(&[
                json!({"status": "live"}),
                json!({"mic": -30}),
                json!({"muted": true})
            ])),
            0.0
        );
        assert_eq!(
            mic_level(&run(&[
                json!({"status": "connecting"}),
                json!({"mic": -30})
            ])),
            0.0
        );
        assert_eq!(
            run(&[
                json!({"status": "live"}),
                json!({"mic": -30}),
                json!({"status": "closed"})
            ])
            .mic,
            -100.0
        );
    }

    // wanted: open, and not stopped since it opened
    #[test]
    fn wanted_open_and_not_stopped_since_it_opened() {
        assert!(wanted(true, false));
        assert!(!wanted(true, true));
        assert!(!wanted(false, false));
    }

    // parseCommand: the words; the rest is usage
    #[test]
    fn parse_command_the_words_the_rest_is_usage() {
        for (w, a) in [
            ("start", Action::Start),
            ("stop", Action::Stop),
            ("toggle", Action::Toggle),
            ("new", Action::New),
            ("mute", Action::Mute),
            ("unmute", Action::Unmute),
        ] {
            assert_eq!(
                parse_command(&format!(" {w} ")),
                Command {
                    action: a,
                    text: String::new()
                }
            );
        }
        assert_eq!(parse_command("avatar ~/me.jpg").action, Action::Usage);
        assert_eq!(parse_command("start now").action, Action::Usage);
        assert_eq!(parse_command("").action, Action::Usage);
    }

    // localPath: file and qs urls to a path; anything else is no path
    #[test]
    fn local_path_file_and_qs_urls_to_a_path_anything_else_is_no_path() {
        assert_eq!(
            local_path("file:///p/core/live/codex-live.py", "/c"),
            "/p/core/live/codex-live.py"
        );
        assert_eq!(
            local_path("qs:@/qs/core/live/a%20b.py", "/cfg/"),
            "/cfg/core/live/a b.py"
        );
        assert_eq!(
            local_path("qs:///plug/core/live/x.py", ""),
            "/plug/core/live/x.py"
        );
        assert_eq!(local_path("qrc:/x.py", "/c"), "");
        assert_eq!(local_path("qs:@/qs/x.py", ""), "");
        // (added) where decodeURIComponent throws, no path
        assert_eq!(local_path("file:///a%zz", ""), "");
        assert_eq!(local_path("file:///a%ff", ""), "");
    }
}
