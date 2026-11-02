//! A spoken conversation with an agent, in the notch: the `codex-live` and
//! `local-live` modules ([`crate::codex_live`], [`crate::local_live`]). Ported
//! from `core/LiveModule.qml`; the rules (the bridge's command line, its lines,
//! the conversation's state, the texts, the IPC words) are
//! `notchd_core::live`.
//!
//! The voice goes through a bridge (`core/live/*.py`), a program the daemon
//! runs for each session and talks to in JSON lines ([`SidecarCmd`]): it
//! tells the status, the transcript, the levels; it is told `mute` and
//! `stop`. A session runs while a notch is open on the module by a click or a
//! shortcut (the host tells: [`CoreModule::shown`]), from the moment it opens
//! until 1.5 s after it closes, or until you stop it, which holds until the
//! notch closes.
//!
//! The levels come 10 to 25 times a second. They are read only while the
//! module's expanded view is on screen, quantized, and move the view at most
//! every [`LEVEL_EVERY_MS`], so a conversation costs the UI some ten small
//! messages a second while you look at it and none while you don't.

use crate::module::{Bridge, CoreModule, Ctx, Effect, Facts, Shown, SidecarCmd, SidecarStart};
use crate::tree::{column, glyph, node, row, text, with};
use notchd_core::js;
use notchd_core::live::{self, Action, Event, Line, Role, State};
use notchd_core::settings::{parse_module_info, setting_default};
use notchd_proto::ui::ModuleOp;
use notchd_proto::{EventValue, Node, Slot};
use serde_json::{Value, json};

/// The notch shut for a moment (a tab, a shortcut pressed twice) keeps the
/// session this long.
const CLOSE_AFTER_MS: u64 = 1500;
/// A bridge asked to stop that has not, in this long, is sent SIGTERM.
const KILL_AFTER_MS: u64 = 4000;
/// The view follows a level at most this often.
pub const LEVEL_EVERY_MS: u64 = 100;
/// How many steps the levels the view draws are quantized to.
const LEVEL_STEPS: f64 = 20.0;
/// The transcript shows this many lines.
const SHOWN_LINES: usize = 8;

const MIC: u32 = 0xF036C;
const MIC_OFF: u32 = 0xF036D;
const STOP: u32 = 0xF04DB;
const RESTART: u32 = 0xF0709;
const CLOSE: u32 = 0xF0156;

/// What distinguishes the two voice modules.
#[derive(Clone, Copy)]
pub struct Kind {
    pub id: &'static str,
    /// The voice runs on this machine (`local-live.py`), joining the agent's
    /// text session, rather than Codex's realtime voice.
    pub local: bool,
    /// Its JSON file, which gives its settings and their defaults.
    pub info: &'static str,
}

pub struct Voice {
    kind: Kind,
    /// The settings' fields (`Settings.liveSettings`), for the defaults.
    fields: Vec<Value>,
    cfg: Value,
    conv: State,
    /// The run now or last (0: none yet).
    run: u64,
    /// A bridge runs, or is starting, or was asked to stop and has not yet.
    bridge: bool,
    shown: Shown,
    /// Stopped with the notch open: stays stopped until it closes.
    stopped: bool,
    wanted: bool,
    close_at: Option<u64>,
    kill_at: Option<u64>,
    /// A new session once the one running has ended.
    renewing: bool,
    /// The levels the view draws (0..1, in steps), and when they last moved.
    voice: f64,
    mic: f64,
    level_at: u64,
    effects: Vec<Effect>,
}

impl Voice {
    pub fn new(kind: Kind) -> Voice {
        let fields = parse_module_info(kind.info).settings;
        Voice {
            kind,
            fields,
            cfg: Value::Null,
            conv: live::initial_state(),
            run: 0,
            bridge: false,
            shown: Shown::default(),
            stopped: false,
            wanted: false,
            close_at: None,
            kill_at: None,
            renewing: false,
            voice: 0.0,
            mic: 0.0,
            level_at: 0,
            effects: Vec::new(),
        }
    }

    // ------------------------------------------------------------ settings

    /// The config's `<id>.<key>`, else the field's default.
    fn setting(&self, key: &str) -> Option<String> {
        let own = self
            .cfg
            .get(self.kind.id)
            .and_then(|m| m.get(key))
            .filter(|v| !v.is_null())
            .cloned();
        let v = own.or_else(|| setting_default(&self.fields, key))?;
        Some(js::string_or_empty(&v))
    }

    /// The agent's name, in the notch's texts.
    fn app(&self) -> &'static str {
        if self.kind.local && self.setting("agent").as_deref() == Some("claude") {
            "Claude"
        } else {
            "Codex"
        }
    }

    fn title(&self) -> &'static str {
        crate::title(self.kind.id)
    }

    fn bridge_opts(&self) -> Bridge {
        let s = |k: &str| self.setting(k);
        if self.kind.local {
            Bridge::Local(live::LocalBridgeOpts {
                agent: s("agent"),
                command: s("command"),
                session: s("session"),
                cwd: s("directory"),
                access: s("access"),
                lang: s("language"),
                speech_model: s("speechModel"),
                voice_model: s("voiceModel"),
                voice_language: s("voiceLanguage"),
                other_voice_model: s("otherVoiceModel"),
                voice: s("voice"),
                url: s("localaiUrl"),
            })
        } else {
            Bridge::Codex(live::BridgeOpts {
                codex: s("command"),
                cwd: s("directory"),
                voice: s("voice"),
                access: s("access"),
            })
        }
    }

    // --------------------------------------------------------------- state

    fn running(&self) -> bool {
        live::running(&self.conv)
    }

    fn is_live(&self) -> bool {
        self.conv.status == "live"
    }

    /// What went wrong, with the components to install named by their titles.
    fn error_text(&self) -> String {
        let catalog = notchd_core::components::parse_catalog(notchd_core::components::REGISTRY);
        let hint = notchd_core::components::missing_hint(&self.conv.components, &catalog);
        live::error_text(&self.conv, Some(&hint))
    }

    // ------------------------------------------------------------- bridge

    fn start(&mut self) {
        if self.bridge {
            return;
        }
        self.conv = live::starting(&self.conv);
        self.bridge = true;
        self.run += 1;
        self.effects
            .push(Effect::Sidecar(SidecarCmd::Start(Box::new(SidecarStart {
                run: self.run,
                bridge: self.bridge_opts(),
            }))));
    }

    /// Asks the bridge to end (it hangs up and closes the agent's server);
    /// one that does not, in a few seconds, is sent SIGTERM.
    fn stop(&mut self, now: u64) {
        if !self.bridge {
            return;
        }
        self.send(json!({"stop": true}));
        self.kill_at = Some(now + KILL_AFTER_MS);
    }

    fn send(&mut self, obj: Value) {
        if self.bridge {
            self.effects
                .push(Effect::Sidecar(SidecarCmd::Write(obj.to_string())));
        }
    }

    fn set_muted(&mut self, on: bool) {
        if self.is_live() {
            self.send(json!({"mute": on}));
        }
    }

    /// A new session: the transcript cleared, the one running (if any) ended
    /// and another started once it has. `false`: no notch shows it.
    fn renew(&mut self, ctx: &Ctx, now: u64) -> bool {
        self.stopped = false;
        let started = if self.bridge {
            self.renewing = true;
            self.stop(now);
            true
        } else {
            self.conv = live::initial_state();
            if self.shown.open {
                self.start();
                true
            } else {
                self.open_notch(ctx)
            }
        };
        self.follow(now);
        started
    }

    /// Opens the first notch that shows it; `false` when none does.
    fn open_notch(&mut self, ctx: &Ctx) -> bool {
        if ctx.listed {
            self.effects.push(Effect::Notch(ModuleOp::Open));
        }
        ctx.listed
    }

    /// `wanted` followed from `open` and `stopped`: a session starts the
    /// moment it is wanted, and ends 1.5 s after it is not.
    fn follow(&mut self, now: u64) {
        let wanted = live::wanted(self.shown.open, self.stopped);
        if wanted == self.wanted {
            return;
        }
        self.wanted = wanted;
        if wanted {
            self.close_at = None;
            self.start();
        } else {
            self.close_at = Some(now + CLOSE_AFTER_MS);
        }
    }

    fn line(&mut self, line: &str, now: u64) {
        let Some(e) = live::parse_line(line) else {
            return;
        };
        self.apply(&e, now);
    }

    fn apply(&mut self, e: &Event, now: u64) {
        self.conv = live::reduce(&self.conv, e);
        self.levels(now);
    }

    fn exited(&mut self, error: Option<&str>, now: u64) {
        self.kill_at = None;
        self.bridge = false;
        if let Some(why) = error {
            // It never started: say why in the notch.
            self.conv = live::reduce(
                &self.conv,
                &Event {
                    error: Some(why.to_string()),
                    ..Event::default()
                },
            );
        }
        if self.conv.status != "closed" {
            self.apply(
                &Event {
                    status: Some("closed".into()),
                    ..Event::default()
                },
                now,
            );
        }
        if self.renewing {
            self.renewing = false;
            self.conv = live::initial_state();
            if self.wanted {
                self.start();
            }
        }
        self.levels(now);
    }

    // -------------------------------------------------------------- levels

    fn quantize(v: f64) -> f64 {
        (v.clamp(0.0, 1.0) * LEVEL_STEPS).round() / LEVEL_STEPS
    }

    /// What the view would draw now: the voice's swell, the microphone's.
    fn target(&self) -> (f64, f64) {
        let voice = if live::speaking(&self.conv) {
            Self::quantize((self.conv.level + 55.0) / 40.0)
        } else {
            0.0
        };
        (voice, Self::quantize(live::mic_level(&self.conv)))
    }

    /// Moves the levels the view draws toward the conversation's, when it is
    /// on screen and not too soon after the last move.
    fn levels(&mut self, now: u64) {
        let want = if self.shown.visible {
            self.target()
        } else {
            (0.0, 0.0)
        };
        if want == (self.voice, self.mic) {
            return;
        }
        if self.shown.visible && now < self.level_at + LEVEL_EVERY_MS {
            return;
        }
        (self.voice, self.mic) = want;
        self.level_at = now;
    }

    // --------------------------------------------------------------- views

    fn dot(&self, size: f64, swell: f64) -> Value {
        let tone = if self.conv.muted { "muted" } else { "danger" };
        let r = size / 2.0 * (1.0 + swell * 0.6);
        let box_ = size * 1.6;
        json!({"t": "canvas", "w": box_, "h": box_, "cmds": [
            {"c": "circle", "x": box_ / 2.0, "y": box_ / 2.0, "r": r, "fill": tone,
             "opacity": if self.is_live() { 1.0 } else { 0.5 }}]})
    }

    fn compact(&self) -> Value {
        row(
            8.0,
            vec![
                self.dot(8.0, 0.0),
                text(
                    &format!("{} · {}", self.app(), live::status_text(&self.conv)),
                    "foreground",
                    "bodySmall",
                ),
            ],
        )
    }

    /// The mark: a tile with the microphone, in the theme's colors.
    fn logo(size: f64) -> Value {
        json!({"t": "canvas", "w": size, "h": size, "cmds": [
            {"c": "rect", "x": 0, "y": 0, "w": size, "h": size, "r": (size / 4.0).round(),
             "fill": "foreground"},
            {"c": "text", "x": (size * 0.2).round(), "y": (size * 0.16).round(),
             "text": glyph(MIC), "tone": "background", "size": "caption"}]})
    }

    fn button(code: u32, action: &str, tooltip: &str) -> Value {
        json!({"t": "button", "glyph": glyph(code), "action": action, "tooltip": tooltip})
    }

    fn line_node(l: &Line) -> Value {
        let user = l.role == Role::User;
        let mut n = with(
            text(
                &l.text,
                if user { "muted" } else { "foreground" },
                "bodySmall",
            ),
            "wrap",
            json!(true),
        );
        if user {
            n = with(n, "align", json!("end"));
        }
        n
    }

    fn expanded(&self) -> Value {
        let running = self.running();
        let error = !self.conv.error.is_empty();
        let mut header = vec![Self::logo(18.0), text(self.title(), "foreground", "body")];
        if running {
            header.push(self.dot(8.0, self.mic));
        }
        header.push(with(
            with(
                text(
                    if running || error {
                        live::status_text(&self.conv)
                    } else {
                        ""
                    },
                    if error { "danger" } else { "muted" },
                    "caption",
                ),
                "fill",
                json!(true),
            ),
            "elide",
            json!("end"),
        ));
        if self.is_live() {
            header.push(Self::button(
                if self.conv.muted { MIC_OFF } else { MIC },
                "mute",
                if self.conv.muted {
                    "Unmute the microphone"
                } else {
                    "Mute the microphone"
                },
            ));
        }
        if running {
            header.push(Self::button(STOP, "stop", "Stop"));
        }
        if running || !self.conv.transcript.is_empty() {
            header.push(Self::button(RESTART, "new", "A new session"));
        }
        header.push(Self::button(CLOSE, "close", "Close"));

        let spacer = json!({"t": "spacer", "fill": true});
        let mut parts = vec![row(10.0, header)];
        // A disc that swells with the voice.
        parts.push(row(
            0.0,
            vec![
                spacer.clone(),
                with(
                    json!({"t": "stack", "children": [
                        {"t": "meter", "value": self.voice, "style": "disc", "size": 96,
                         "child": {"t": "icon", "glyph": glyph(if self.conv.muted { MIC_OFF } else { MIC }),
                                   "size": 28}}]}),
                    "height",
                    json!(220),
                ),
                spacer.clone(),
            ],
        ));
        // Not talking: how to start, or what went wrong.
        if !running {
            let mut start = Vec::new();
            if error {
                start.push(with(
                    text(&self.error_text(), "danger", "caption"),
                    "wrap",
                    json!(true),
                ));
            }
            start.push(row(
                0.0,
                vec![
                    spacer.clone(),
                    json!({"t": "button", "variant": "outline", "action": "start",
                           "label": if error { "Try again".to_string() }
                                    else { format!("Talk to {}", self.app()) }}),
                    spacer,
                ],
            ));
            parts.push(column(6.0, start));
        }
        // Something to know that doesn't end it.
        if running && !self.conv.warning.is_empty() {
            parts.push(with(
                text(&self.conv.warning, "muted", "caption"),
                "wrap",
                json!(true),
            ));
        }
        // What was said, the latest at the bottom.
        if !self.conv.transcript.is_empty() {
            let lines: Vec<&Line> = self
                .conv
                .transcript
                .iter()
                .rev()
                .take(SHOWN_LINES)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let mut list = json!({"t": "list", "gap": 6, "stick": "end", "maxHeight": 150,
                "children": lines.iter().map(|l| Self::line_node(l)).collect::<Vec<_>>()});
            // The finished lines are plain; the one still being said is fainter.
            if lines.last().is_some_and(|l| !l.done) {
                list["children"][lines.len() - 1]["tone"] = json!("muted");
            }
            parts.push(list);
        }
        column(10.0, parts)
    }
}

impl CoreModule for Voice {
    fn id(&self) -> &'static str {
        self.kind.id
    }

    fn facts(&self, _now: u64) -> Facts {
        Facts {
            priority: 45,
            active: true,
            icon: glyph(MIC),
            expanded_width: 460,
            ..Facts::default()
        }
    }

    fn view(&self, slot: Slot, _now: u64) -> Option<Node> {
        Some(node(match slot {
            Slot::Compact if self.running() => self.compact(),
            Slot::Expanded => self.expanded(),
            _ => return None,
        }))
    }

    fn config(&mut self, cfg: &Value, _now: u64) {
        self.cfg = cfg.clone();
    }

    /// `live <id> start | stop | toggle | new | mute | unmute`.
    fn command(&mut self, ctx: &Ctx, method: &str, args: &[String], now: u64) -> Option<String> {
        if method != "live" {
            return None;
        }
        let word = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        let id = self.kind.id;
        let none = format!("no notch shows {id}; add it to one");
        let answer = match live::parse_command(&word).action {
            Action::Start => self.start_command(ctx, now).unwrap_or(none),
            Action::Stop => {
                self.stopped = true;
                self.stop(now);
                self.follow(now);
                "ok".into()
            }
            Action::Toggle => {
                if self.running() {
                    self.stopped = true;
                    self.stop(now);
                    self.follow(now);
                    "ok".into()
                } else {
                    self.start_command(ctx, now).unwrap_or(none)
                }
            }
            Action::New => {
                if self.renew(ctx, now) {
                    "ok".into()
                } else {
                    none
                }
            }
            Action::Mute | Action::Unmute => {
                if !self.is_live() {
                    "not in a conversation".into()
                } else {
                    self.set_muted(live::parse_command(&word).action == Action::Mute);
                    "ok".into()
                }
            }
            Action::Usage => {
                format!("usage: live {id} start | stop | toggle | new | mute | unmute")
            }
        };
        Some(answer)
    }

    fn event(&mut self, ctx: &Ctx, action: &str, _value: &EventValue, now: u64) {
        match action {
            // A press pins the notch (which starts it, opened by a hover);
            // pinned already, it starts it.
            "start" => {
                self.effects.push(Effect::Notch(ModuleOp::Keyboard));
                if self.start_command(ctx, now).is_none() {
                    self.stopped = false;
                }
            }
            "stop" => {
                self.stopped = true;
                self.stop(now);
                self.follow(now);
            }
            "new" => {
                self.effects.push(Effect::Notch(ModuleOp::Keyboard));
                self.renew(ctx, now);
            }
            "mute" => self.set_muted(!self.conv.muted),
            "close" => self.effects.push(Effect::Notch(ModuleOp::Close)),
            _ => {}
        }
    }

    fn shown(&mut self, shown: Shown, now: u64) {
        let was = self.shown;
        self.shown = shown;
        if !shown.open {
            self.stopped = false;
        }
        self.follow(now);
        if was.visible != shown.visible {
            self.levels(now);
        }
    }

    fn tick(&mut self, now: u64) {
        if self.close_at.is_some_and(|at| now >= at) {
            self.close_at = None;
            if !self.wanted {
                self.stop(now);
            }
        }
        if self.kill_at.is_some_and(|at| now >= at) {
            self.kill_at = None;
            if self.bridge {
                self.effects.push(Effect::Sidecar(SidecarCmd::Term));
            }
        }
        self.levels(now);
    }

    fn next_due(&self, _now: u64) -> Option<u64> {
        let level = (self.shown.visible && self.target() != (self.voice, self.mic))
            .then_some(self.level_at + LEVEL_EVERY_MS);
        [self.close_at, self.kill_at, level]
            .into_iter()
            .flatten()
            .min()
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// What the host knows of the bridge (`sidecar`): a line it printed, its
    /// end, or that it could not start; each names its module by `id` and its
    /// run by `run`.
    fn feed(&mut self, provider: &str, data: &Value, now: u64) {
        if provider != "sidecar" || data.get("id").and_then(Value::as_str) != Some(self.kind.id) {
            return;
        }
        // Of a run that was replaced: not ours any more.
        if data.get("run").and_then(Value::as_u64) != Some(self.run) {
            return;
        }
        match data.get("event").and_then(Value::as_str) {
            Some("line") => {
                if let Some(l) = data.get("line").and_then(Value::as_str) {
                    self.line(l, now);
                }
            }
            Some("exit") => self.exited(data.get("error").and_then(Value::as_str), now),
            _ => {}
        }
    }

    /// The test hook `test-<id>`: `state <json>` is a conversation as it
    /// would be (`status`, `error`, `packages`, `components`, `warning`,
    /// `transcript` as `[{role, text, done}]`, `working`, `muted`, `level`,
    /// `mic`), with no bridge: offscreen, nothing talks.
    fn demo(&mut self, args: &[String], now: u64) -> Option<String> {
        if args.first().map(String::as_str) != Some("state") {
            return None;
        }
        let v: Value = serde_json::from_str(args.get(1)?).ok()?;
        let mut s = live::initial_state();
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let list = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        s.status = if text("status").is_empty() {
            "off".into()
        } else {
            text("status")
        };
        s.error = text("error");
        s.packages = list("packages");
        s.components = list("components");
        s.warning = text("warning");
        s.working = v.get("working").and_then(Value::as_bool).unwrap_or(false);
        s.muted = v.get("muted").and_then(Value::as_bool).unwrap_or(false);
        s.level = v.get("level").and_then(Value::as_f64).unwrap_or(s.level);
        s.mic = v.get("mic").and_then(Value::as_f64).unwrap_or(s.mic);
        s.transcript = v
            .get("transcript")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .map(|x| Line {
                        role: if x.get("role").and_then(Value::as_str) == Some("user") {
                            Role::User
                        } else {
                            Role::Assistant
                        },
                        text: x.get("text").and_then(Value::as_str).unwrap_or("").into(),
                        done: x.get("done").and_then(Value::as_bool).unwrap_or(true),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.conv = s;
        self.level_at = 0;
        self.levels(now);
        Some("ok".into())
    }
}

impl Voice {
    /// `start`: stopped no more; starts at once when a notch is open on it,
    /// else opens one. `None`: no notch shows it.
    fn start_command(&mut self, ctx: &Ctx, now: u64) -> Option<String> {
        self.stopped = false;
        let answer = if self.shown.open {
            self.start();
            Some("ok".to_string())
        } else {
            self.open_notch(ctx).then(|| "ok".to_string())
        };
        self.follow(now);
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notchd_proto::Validate;

    fn codex() -> Voice {
        crate::codex_live::new()
    }

    fn local() -> Voice {
        crate::local_live::new()
    }

    fn open() -> Shown {
        Shown {
            open: true,
            visible: true,
        }
    }

    fn listed() -> Ctx {
        Ctx {
            listed: true,
            ..Ctx::default()
        }
    }

    fn line(v: &mut Voice, run: u64, l: Value, now: u64) {
        let id = v.kind.id;
        v.feed(
            "sidecar",
            &json!({"id": id, "run": run, "event": "line", "line": l.to_string()}),
            now,
        );
    }

    fn say(v: &mut Voice, ctx: &Ctx, word: &str, now: u64) -> String {
        let id = v.kind.id.to_string();
        v.command(ctx, "live", &[id, word.into()], now).unwrap()
    }

    fn started(v: &mut Voice) -> Vec<SidecarStart> {
        v.take_effects()
            .into_iter()
            .filter_map(|e| match e {
                Effect::Sidecar(SidecarCmd::Start(s)) => Some(*s),
                _ => None,
            })
            .collect()
    }

    fn writes(fx: &[Effect]) -> Vec<String> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Sidecar(SidecarCmd::Write(l)) => Some(l.clone()),
                _ => None,
            })
            .collect()
    }

    fn json_of(v: &Voice, slot: Slot) -> String {
        serde_json::to_string(&v.view(slot, 0).expect("a view")).unwrap()
    }

    #[test]
    fn every_view_it_builds_is_a_valid_tree() {
        let mut v = codex();
        for state in [
            json!({}),
            json!({"status": "live", "level": -20, "mic": -30, "muted": true,
                   "transcript": [{"role": "user", "text": "Oi", "done": true},
                                  {"role": "assistant", "text": "Ola", "done": false}],
                   "warning": "no echo cancellation"}),
            json!({"status": "closed", "error": "Codex Live needs packages.",
                   "components": ["codex-live-audio"]}),
        ] {
            v.demo(&["state".into(), state.to_string()], 0).unwrap();
            for slot in Slot::ALL {
                if let Some(n) = v.view(slot, 0) {
                    n.validate().unwrap();
                }
            }
        }
    }

    #[test]
    fn a_session_starts_when_a_notch_opens_on_it_and_ends_after_it_closes() {
        let mut v = codex();
        assert!(started(&mut v).is_empty());
        v.shown(open(), 1000);
        let runs = started(&mut v);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run, 1);
        assert!(matches!(runs[0].bridge, Bridge::Codex(_)));
        assert!(v.running(), "starting counts as a session");
        // The notch shuts: the session goes on for 1.5 s, and a second open
        // inside that keeps it.
        v.shown(Shown::default(), 2000);
        assert_eq!(v.next_due(2000), Some(3500));
        v.shown(open(), 3000);
        v.tick(3600);
        assert!(writes(&v.take_effects()).is_empty(), "kept by the reopen");
        // Shut for good: it is asked to stop once the time has gone.
        v.shown(Shown::default(), 4000);
        v.tick(5000);
        assert!(writes(&v.take_effects()).is_empty());
        v.tick(5500);
        assert_eq!(writes(&v.take_effects()), [r#"{"stop":true}"#]);
        // A bridge that does not stop in 4 s is terminated.
        assert_eq!(v.next_due(5500), Some(9500));
        v.tick(9500);
        assert_eq!(v.take_effects(), vec![Effect::Sidecar(SidecarCmd::Term)]);
    }

    #[test]
    fn a_hover_only_shows_it_and_starts_nothing() {
        let mut v = codex();
        v.shown(
            Shown {
                open: false,
                visible: true,
            },
            0,
        );
        assert!(v.take_effects().is_empty());
        assert!(!v.running());
    }

    #[test]
    fn stop_holds_until_the_notch_closes() {
        let mut v = codex();
        v.shown(open(), 0);
        started(&mut v);
        line(&mut v, 1, json!({"status": "live"}), 10);
        assert_eq!(say(&mut v, &Ctx::default(), "stop", 20), "ok");
        assert_eq!(writes(&v.take_effects()), [r#"{"stop":true}"#]);
        v.exited(None, 30);
        assert_eq!(v.conv.status, "closed");
        // Open still: it does not start again by itself...
        v.shown(open(), 40);
        assert!(started(&mut v).is_empty());
        // ...until the notch has closed and opens again.
        v.shown(Shown::default(), 50);
        v.shown(open(), 60);
        assert_eq!(started(&mut v).len(), 1);
    }

    #[test]
    fn start_opens_a_notch_that_lists_it_or_says_none_does() {
        let mut v = codex();
        assert_eq!(
            say(&mut v, &Ctx::default(), "start", 0),
            "no notch shows codex-live; add it to one"
        );
        assert!(v.take_effects().is_empty());
        assert_eq!(say(&mut v, &listed(), "start", 0), "ok");
        assert_eq!(v.take_effects(), vec![Effect::Notch(ModuleOp::Open)]);
        // Open: it starts at once, no notch to open.
        v.shown(open(), 1);
        let fx = v.take_effects();
        assert!(!fx.contains(&Effect::Notch(ModuleOp::Open)));
        assert_eq!(
            fx.iter()
                .filter(|e| matches!(e, Effect::Sidecar(SidecarCmd::Start(_))))
                .count(),
            1
        );
        assert_eq!(say(&mut v, &listed(), "start", 2), "ok");
        assert!(started(&mut v).is_empty(), "one session at a time");
    }

    #[test]
    fn the_words_and_what_they_answer() {
        let mut v = codex();
        let c = Ctx::default();
        assert_eq!(say(&mut v, &c, "mute", 0), "not in a conversation");
        assert_eq!(
            say(&mut v, &c, "dance", 0),
            "usage: live codex-live start | stop | toggle | new | mute | unmute"
        );
        v.shown(open(), 0);
        started(&mut v);
        line(&mut v, 1, json!({"status": "live"}), 1);
        assert_eq!(say(&mut v, &c, "mute", 2), "ok");
        assert_eq!(writes(&v.take_effects()), [r#"{"mute":true}"#]);
        assert_eq!(say(&mut v, &c, "unmute", 3), "ok");
        assert_eq!(writes(&v.take_effects()), [r#"{"mute":false}"#]);
        // Toggle stops what runs.
        assert_eq!(say(&mut v, &c, "toggle", 4), "ok");
        assert_eq!(writes(&v.take_effects()), [r#"{"stop":true}"#]);
    }

    #[test]
    fn new_ends_the_session_and_starts_another_with_the_transcript_cleared() {
        let mut v = codex();
        v.shown(open(), 0);
        started(&mut v);
        line(&mut v, 1, json!({"status": "live"}), 1);
        line(
            &mut v,
            1,
            json!({"transcript": {"role": "user", "text": "Oi", "done": true}}),
            2,
        );
        assert_eq!(say(&mut v, &Ctx::default(), "new", 3), "ok");
        assert_eq!(writes(&v.take_effects()), [r#"{"stop":true}"#]);
        v.feed(
            "sidecar",
            &json!({"id": "codex-live", "run": 1, "event": "exit"}),
            4,
        );
        let again = started(&mut v);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].run, 2);
        assert!(v.conv.transcript.is_empty() || v.conv.status == "starting");
        assert_eq!(v.conv.status, "starting");
        assert!(v.conv.transcript.is_empty());
    }

    #[test]
    fn what_a_replaced_run_says_is_not_heard() {
        let mut v = codex();
        v.shown(open(), 0);
        started(&mut v);
        line(&mut v, 7, json!({"status": "live"}), 1);
        assert_eq!(v.conv.status, "starting");
        line(&mut v, 1, json!({"status": "live"}), 2);
        assert_eq!(v.conv.status, "live");
        // Another module's lines neither.
        v.feed(
            "sidecar",
            &json!({"id": "local-live", "run": 1, "event": "exit"}),
            3,
        );
        assert!(v.running());
    }

    #[test]
    fn a_bridge_that_ends_closes_the_session_and_keeps_its_error() {
        let mut v = codex();
        v.shown(open(), 0);
        started(&mut v);
        line(
            &mut v,
            1,
            json!({"error": "needs packages", "components": ["codex-live-audio"]}),
            1,
        );
        v.feed(
            "sidecar",
            &json!({"id": "codex-live", "run": 1, "event": "exit"}),
            2,
        );
        assert_eq!(v.conv.status, "closed");
        assert!(
            v.error_text().contains("Install WebRTC and audio"),
            "{}",
            v.error_text()
        );
        assert!(json_of(&v, Slot::Expanded).contains("Try again"));
        // A bridge that could not start says so.
        let mut v = codex();
        v.shown(open(), 0);
        started(&mut v);
        v.feed(
            "sidecar",
            &json!({"id": "codex-live", "run": 1, "event": "exit",
                    "error": "python3: not found"}),
            1,
        );
        assert!(v.conv.error.contains("python3"));
    }

    #[test]
    fn the_command_lines_follow_the_settings_and_their_defaults() {
        let mut v = local();
        v.config(
            &json!({"local-live": {"agent": "claude", "directory": "~/talk", "voiceLanguage": "en",
                                   "otherVoiceModel": "mine"}}),
            0,
        );
        v.shown(open(), 0);
        let runs = started(&mut v);
        let Bridge::Local(o) = &runs[0].bridge else {
            panic!()
        };
        assert_eq!(o.agent.as_deref(), Some("claude"));
        assert_eq!(o.cwd.as_deref(), Some("~/talk"));
        assert_eq!(o.session.as_deref(), Some("last"), "the field's default");
        assert_eq!(o.voice_language.as_deref(), Some("en"));
        assert_eq!(o.other_voice_model.as_deref(), Some("mine"));
        assert_eq!(v.app(), "Claude");
        let c = codex();
        assert_eq!(c.app(), "Codex");
        let Bridge::Codex(o) = c.bridge_opts() else {
            panic!()
        };
        assert_eq!(o.codex.as_deref(), Some("codex"), "the command's default");
        assert_eq!(o.access.as_deref(), Some("read-only"));
    }

    #[test]
    fn the_buttons_of_its_view_are_events() {
        let mut v = codex();
        let c = Ctx::default();
        let null = EventValue::Null(());
        v.event(&c, "close", &null, 0);
        assert_eq!(v.take_effects(), vec![Effect::Notch(ModuleOp::Close)]);
        // A press on Talk pins the notch, which starts it.
        v.event(&listed(), "start", &null, 0);
        assert_eq!(
            v.take_effects(),
            vec![
                Effect::Notch(ModuleOp::Keyboard),
                Effect::Notch(ModuleOp::Open)
            ]
        );
        v.shown(open(), 1);
        started(&mut v);
        line(&mut v, 1, json!({"status": "live"}), 2);
        v.event(&c, "mute", &null, 3);
        assert_eq!(writes(&v.take_effects()), [r#"{"mute":true}"#]);
    }

    #[test]
    fn levels_are_read_only_while_the_view_is_on_screen_and_at_most_ten_a_second() {
        let mut v = codex();
        v.shown(open(), 0);
        started(&mut v);
        line(&mut v, 1, json!({"status": "live"}), 0);
        let drawn = |v: &Voice| (v.voice, v.mic);
        // The first level moves the view at once, quantized.
        line(&mut v, 1, json!({"level": -30.0}), 1000);
        assert_eq!(drawn(&v).0, 0.65);
        // 25 a second after it: the view waits for its time.
        for (i, db) in [-28.0, -26.0, -25.0].iter().enumerate() {
            line(&mut v, 1, json!({"level": db}), 1000 + 30 * (i as u64 + 1));
        }
        assert_eq!(drawn(&v).0, 0.65, "not before 100 ms");
        assert_eq!(v.next_due(1090), Some(1100));
        v.tick(1100);
        assert_eq!(drawn(&v).0, 0.75);
        assert_eq!(v.next_due(1100), None, "nothing pending");
        // A change too small to be a step moves nothing.
        line(&mut v, 1, json!({"level": -24.9}), 1300);
        assert_eq!(drawn(&v).0, 0.75);
        // Not on screen: nothing is drawn, nothing is due.
        v.shown(
            Shown {
                open: true,
                visible: false,
            },
            2000,
        );
        assert_eq!(drawn(&v), (0.0, 0.0));
        line(&mut v, 1, json!({"level": -20.0, "mic": -30.0}), 2100);
        assert_eq!(drawn(&v), (0.0, 0.0));
        assert_eq!(v.next_due(2100), None);
        // On screen again: it catches up.
        v.shown(open(), 3000);
        assert_eq!(drawn(&v), (0.9, 0.75));
    }

    #[test]
    fn the_closed_notch_shows_a_dot_and_what_it_does_only_while_it_talks() {
        let mut v = codex();
        assert!(v.view(Slot::Compact, 0).is_none());
        v.shown(open(), 0);
        started(&mut v);
        line(&mut v, 1, json!({"status": "live"}), 1);
        let compact = json_of(&v, Slot::Compact);
        assert!(compact.contains("Codex · Listening"), "{compact}");
        assert!(v.view(Slot::VerticalCompact, 0).is_none());
        assert!(v.view(Slot::Peek, 0).is_none());
        let f = v.facts(0);
        assert!((f.priority, f.active, f.expanded_width) == (45, true, 460));
    }

    #[test]
    fn the_expanded_view_has_the_disc_the_transcript_and_the_controls_that_fit() {
        let mut v = codex();
        let idle = json_of(&v, Slot::Expanded);
        assert!(idle.contains("Talk to Codex") && idle.contains(r#""t":"meter""#));
        assert!(!idle.contains(r#""action":"stop""#) && !idle.contains(r#""action":"mute""#));
        v.demo(
            &[
                "state".into(),
                json!({"status": "live", "transcript": [
                {"role": "user", "text": "Rodei os testes.", "done": true},
                {"role": "assistant", "text": "Otimo", "done": false}]})
                .to_string(),
            ],
            0,
        );
        let live = json_of(&v, Slot::Expanded);
        for want in [
            r#""action":"stop""#,
            r#""action":"mute""#,
            r#""action":"new""#,
            r#""t":"list""#,
            "Rodei os testes.",
        ] {
            assert!(live.contains(want), "{want} in {live}");
        }
        assert!(!live.contains("Talk to Codex"));
    }
}
