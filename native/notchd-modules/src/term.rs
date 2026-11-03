//! A module that runs a program's own terminal UI in the notch: a coding
//! agent (`claude`, `codex`) or a terminal of your own (the config's
//! `terminals`). Ported from `core/TerminalModule.qml`; the rules are
//! `notchd-core::agent` and `::terminal`, the plan usage is [`crate::plan`].
//!
//! The program runs in the daemon (the `pty` provider), so it outlives a
//! shell reload: the UI's terminal view only attaches to it (a `surface`
//! node, docs/architecture.md, "Free surface"). The program starts the first
//! time a view attaches, and keeps running while the notch is closed, so the
//! session and whatever it is doing go on (unless `idleMinutes` says to stop
//! it); the restart button starts a new one. The closed notch shows the
//! session's title and a dot while the program works, both read from the
//! terminal's title, which the program sets itself.
//!
//! The host feeds it what happens to its terminal (`feed("pty", ..)`): the
//! views attached (`views`), the title, the end of the program. It answers
//! with [`Effect::Pty`] commands.

use crate::module::{CoreModule, Ctx, Effect, Facts, PtyCmd, PtyStart};
use crate::plan::Plan;
use crate::tree::{column, icon_button, node, row, text, with};
use notchd_core::agent::{self, Action, AgentTitle, PeekState};
use notchd_core::js;
use notchd_core::terminal::shell_args;
use notchd_core::usage::{Alert, reset_in};
use notchd_proto::ui::ModuleOp;
use notchd_proto::{EventValue, Node, Slot};
use serde_json::{Value, json};

/// How the module reads its terminal's title: `agent::claude_title`...
pub type ParseTitle = fn(&str) -> AgentTitle;

/// What distinguishes one terminal module from another.
#[derive(Clone)]
pub struct Kind {
    pub id: &'static str,
    /// The program's name, in the notch's texts (a terminal's: its title, else
    /// its id).
    pub app: String,
    pub parse_title: ParseTitle,
    /// Run when the `command` setting is empty.
    pub default_command: String,
    /// The mark, a file of `core/modules/` the UI bundles; without one, a
    /// prompt drawn in a tile.
    pub logo: Option<&'static str>,
    /// Without a `directory`, start in notchd's own folder, as an agent does;
    /// off, in the home.
    pub own_dir: bool,
    /// The command is a shell script ("a; b", "cd x && y"), run as is.
    pub script: bool,
    /// The IPC method that reaches it, for its usage text: `agent`, or
    /// `terminal` for one of the config's.
    pub ipc: &'static str,
    /// Where its plan usage comes from (`claude`, `codex`).
    pub usage: Option<&'static str>,
    /// A terminal of the config: its command and title are its entry's
    /// (`terminals.<id>`).
    pub custom: bool,
}

/// What the peek says: the agent finished, or a limit passed the warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PeekReason {
    Done,
    Usage,
}

pub struct TermModule {
    kind: Kind,
    cfg: Value,
    /// The run now or last (0: none yet). Part of the surface's id, so a
    /// view of an older run is replaced.
    run: u64,
    /// A program runs (or is starting).
    running: bool,
    /// The program ended; its session stays, with what it printed.
    exited: bool,
    raw_title: String,
    status: AgentTitle,
    /// You typed to it: from then on, the end of its work is news.
    typed: bool,
    /// How many views are attached to its terminal.
    views: u32,
    attention: bool,
    reason: PeekReason,
    alert: Option<Alert>,
    plan: Plan,
    /// What `ask` types once the program is up, and when to look.
    pending_ask: Option<(String, u64)>,
    /// Closed and not working since: it stops after `idleMinutes`.
    idle_since: Option<u64>,
    /// The minute the "resets in" texts are kept fresh by.
    minute_at: Option<u64>,
    effects: Vec<Effect>,
}

impl TermModule {
    pub fn new(kind: Kind) -> TermModule {
        let plan = Plan::new(kind.usage);
        TermModule {
            kind,
            cfg: Value::Null,
            run: 0,
            running: false,
            exited: false,
            raw_title: String::new(),
            status: AgentTitle {
                working: false,
                title: String::new(),
            },
            typed: false,
            views: 0,
            attention: false,
            reason: PeekReason::Done,
            alert: None,
            plan,
            pending_ask: None,
            idle_since: None,
            minute_at: None,
            effects: Vec::new(),
        }
    }

    // ------------------------------------------------------------ settings

    fn setting(&self, key: &str) -> Option<&Value> {
        self.cfg
            .get(self.kind.id)
            .and_then(|m| m.get(key))
            .filter(|v| !v.is_null())
    }

    fn number(&self, key: &str, default: f64) -> f64 {
        let n = self.setting(key).map_or(f64::NAN, js::number);
        if n.is_finite() && n != 0.0 {
            n
        } else {
            default
        }
    }

    fn text_setting(&self, key: &str) -> String {
        self.setting(key)
            .map(|v| js::trim(&js::string_or_empty(v)).to_string())
            .unwrap_or_default()
    }

    /// `Math.max(320, Number(width) || 760)`.
    pub fn expanded_width(&self) -> u32 {
        self.number("width", 760.0).max(320.0) as u32
    }

    /// `Math.max(120, Number(height) || 420)`.
    pub fn terminal_height(&self) -> u32 {
        self.number("height", 420.0).max(120.0) as u32
    }

    fn command_line(&self) -> String {
        let own = self.text_setting("command");
        if own.is_empty() {
            self.kind.default_command.clone()
        } else {
            own
        }
    }

    fn idle_minutes(&self) -> f64 {
        self.number("idleMinutes", 0.0)
    }

    // --------------------------------------------------------------- state

    pub fn session_title(&self) -> &str {
        &self.status.title
    }

    fn working(&self) -> bool {
        self.status.working
    }

    /// The run the surface stands for: the one now, or the one the next view
    /// will start, so the first view of a new run is not replaced by a second
    /// when the program starts.
    fn surface_run(&self) -> u64 {
        if self.has_session() {
            self.run
        } else {
            self.run + 1
        }
    }

    /// There is a session: a program running, or one that ended and left
    /// what it printed.
    fn has_session(&self) -> bool {
        self.running || self.exited
    }

    fn set_raw_title(&mut self, raw: &str, now: u64) {
        self.raw_title = raw.to_string();
        let next = (self.kind.parse_title)(raw);
        let was = self.status.working;
        self.status = next;
        if self.status.working != was {
            self.working_changed(now);
        }
        self.refresh_idle(now);
    }

    fn working_changed(&mut self, now: u64) {
        if self.working() {
            self.attention = false;
        } else if self.typed {
            // A task ended: its usage moved.
            if let Some(ask) = self.plan.refresh(now, false) {
                self.effects.push(Effect::Usage(ask));
            }
        }
        let done = agent::done_peek(&PeekState {
            was_working: !self.working(),
            working: self.working(),
            open: self.views > 0,
            typed: self.typed,
        });
        if done {
            // Done while you weren't looking: news until its view opens.
            self.attention = true;
            self.reason = PeekReason::Done;
            self.effects.push(Effect::Peek(5000));
        }
    }

    /// Starts the program unless there is a session already.
    fn ensure_session(&mut self, now: u64) {
        if self.has_session() {
            return;
        }
        self.start(now);
    }

    fn start(&mut self, now: u64) {
        self.typed = false;
        self.run += 1;
        self.running = true;
        self.exited = false;
        self.raw_title.clear();
        self.status = AgentTitle {
            working: false,
            title: String::new(),
        };
        let dir = self.text_setting("directory");
        let args = shell_args(
            &self.command_line(),
            &[],
            dir.is_empty() && self.kind.own_dir,
            self.kind.script,
        );
        self.effects.push(Effect::Pty(PtyCmd::Start(PtyStart {
            run: self.run,
            program: "/bin/sh".into(),
            args,
            dir,
        })));
        self.refresh_idle(now);
    }

    /// A new session: the old program stops with its terminal.
    fn restart(&mut self, now: u64) {
        self.stop(now);
        self.start(now);
    }

    fn stop(&mut self, now: u64) {
        // A new number: what the run that is ending still says (its exit,
        // arriving late) is not about the session that follows.
        self.run += 1;
        self.attention = false;
        self.pending_ask = None;
        if self.has_session() {
            self.effects.push(Effect::Pty(PtyCmd::Stop));
        }
        self.running = false;
        self.exited = false;
        self.typed = false;
        self.raw_title.clear();
        self.status = AgentTitle {
            working: false,
            title: String::new(),
        };
        self.refresh_idle(now);
    }

    /// Closed and not working for `idleMinutes`, it stops (an agent takes
    /// hundreds of MB); the next view starts a new one. 0 keeps it running.
    fn refresh_idle(&mut self, now: u64) {
        let idle =
            self.idle_minutes() > 0.0 && self.has_session() && !self.working() && self.views == 0;
        if !idle {
            self.idle_since = None;
        } else if self.idle_since.is_none() {
            self.idle_since = Some(now);
        }
    }

    fn idle_due(&self) -> Option<u64> {
        self.idle_since
            .map(|s| s + (self.idle_minutes() * 60_000.0).max(1.0) as u64)
    }

    fn views_changed(&mut self, n: u32, now: u64) {
        self.views = n;
        if n > 0 {
            self.attention = false;
            self.ensure_session(now);
        }
        self.refresh_idle(now);
        self.refresh_minute(now);
    }

    fn refresh_minute(&mut self, now: u64) {
        let want = self.views > 0 && self.plan.shows();
        if !want {
            self.minute_at = None;
        } else if self.minute_at.is_none() {
            self.minute_at = Some(now + 30_000);
        }
    }

    // ------------------------------------------------------------------ IPC

    /// "ok", or what was wrong (`agent::parse_command`).
    fn command_text(&mut self, ctx: &Ctx, text: &str, now: u64) -> String {
        let cmd = agent::parse_command(text);
        match cmd.action {
            Action::Toggle => {
                // By the notch state, not by the views attached: a terminal
                // attaches a moment after its notch opens, and a toggle in
                // between must still close it.
                self.effects.push(Effect::Notch(if ctx.visible {
                    ModuleOp::Close
                } else {
                    ModuleOp::Open
                }));
                "ok".into()
            }
            Action::New => {
                self.restart(now);
                "ok".into()
            }
            Action::Ask => self.ask(ctx, &cmd.text, now),
            Action::Usage => format!(
                "usage: {} {} toggle | new | \"ask <question>\"",
                self.kind.ipc, self.kind.id
            ),
        }
    }

    /// Opens the notch and types `text` into the program, then Enter. A
    /// program that is still starting gets it once it has set its first title
    /// (it is drawing its UI by then), and a moment later.
    fn ask(&mut self, ctx: &Ctx, text: &str, now: u64) -> String {
        if !ctx.terminal_ui {
            return "no UI connected draws a terminal".into();
        }
        self.effects.push(Effect::Notch(ModuleOp::Open));
        if text.is_empty() {
            return "ok".into();
        }
        if self.exited {
            self.restart(now);
        } else {
            self.ensure_session(now);
        }
        self.pending_ask = Some((text.to_string(), now + 500));
        "ok".into()
    }

    fn deliver_ask(&mut self, now: u64) {
        let Some((text, at)) = self.pending_ask.clone() else {
            return;
        };
        if now < at {
            return;
        }
        if !self.running {
            self.pending_ask = None;
            return;
        }
        if self.raw_title.is_empty() {
            self.pending_ask = Some((text, now + 500));
            return;
        }
        self.effects
            .push(Effect::Pty(PtyCmd::Write(format!("{text}\r").into_bytes())));
        self.typed = true;
        self.pending_ask = None;
    }

    // ---------------------------------------------------------------- views

    fn app_name(&self) -> &str {
        &self.kind.app
    }

    fn title_or_app(&self) -> &str {
        if self.session_title().is_empty() {
            self.app_name()
        } else {
            self.session_title()
        }
    }

    /// The mark at `size` px: the bundled image, else a prompt in a rounded
    /// tile in the theme's colors.
    fn logo(&self, size: f64) -> Value {
        match self.kind.logo {
            Some(file) => json!({"t": "image", "src": format!("asset:{file}"), "size": size,
                "fit": "contain", "fallback": {"t": "icon", "glyph": ">_", "name": "utilities-terminal", "tone": "muted"}}),
            None => json!({"t": "canvas", "w": size, "h": size, "cmds": [
                {"c": "rect", "x": 0, "y": 0, "w": size, "h": size, "r": (size / 4.0).round(),
                 "fill": "foreground"},
                {"c": "text", "x": (size * 0.14).round(), "y": (size * 0.2).round(),
                 "text": ">_", "tone": "background", "size": "caption"}]}),
        }
    }

    /// A dot: green while it works, the accent once it is done and you
    /// haven't looked; its place stays while it is off.
    fn dot(&self) -> Value {
        let tone = if self.working() { "success" } else { "accent" };
        let shown = self.working() || self.attention;
        let cmds = if shown {
            json!([{"c": "circle", "x": 3.5, "y": 3.5, "r": 3.5, "fill": tone}])
        } else {
            json!([])
        };
        json!({"t": "canvas", "w": 7, "h": 7, "cmds": cmds})
    }

    fn compact(&self) -> Value {
        row(
            8.0,
            vec![
                self.logo(15.0),
                with(
                    with(
                        text(
                            self.title_or_app(),
                            if self.session_title().is_empty() {
                                "muted"
                            } else {
                                "foreground"
                            },
                            "bodySmall",
                        ),
                        "maxWidth",
                        json!(220),
                    ),
                    "elide",
                    json!("end"),
                ),
                self.dot(),
            ],
        )
    }

    /// Down a side edge: the mark, the dot a badge on its corner.
    fn vertical_compact(&self) -> Value {
        let shown = self.working() || self.attention;
        let tone = if self.working() { "success" } else { "accent" };
        let cmds = if shown {
            json!([{"c": "circle", "x": 18.5, "y": 18.5, "r": 5.5, "fill": tone,
                    "stroke": "background", "width": 2}])
        } else {
            json!([])
        };
        json!({"t": "stack", "children": [
            self.logo(15.0),
            {"t": "canvas", "w": 24, "h": 24, "cmds": cmds},
        ]})
    }

    fn peek(&self, now: u64) -> Value {
        let alert = if self.reason == PeekReason::Usage {
            self.alert.as_ref()
        } else {
            None
        };
        let app = self.app_name();
        let title = match alert {
            None => format!("{app} is done"),
            Some(a) if a.level >= 100.0 => format!("{app}: {} limit reached", a.limit.label),
            Some(a) => format!(
                "{app}: {} limit at {} %",
                a.limit.label,
                js::num_str(js::round(a.limit.percent))
            ),
        };
        let sub = match alert {
            Some(a) if a.limit.resets_at > 0.0 => {
                format!("Resets in {}", reset_in(a.limit.resets_at, now as f64))
            }
            Some(_) => String::new(),
            None => self.session_title().to_string(),
        };
        let mut lines = vec![text(
            &title,
            if alert.is_some_and(|a| a.level >= 100.0) {
                "danger"
            } else {
                "foreground"
            },
            "body",
        )];
        if !sub.is_empty() {
            lines.push(with(
                with(text(&sub, "muted", "caption"), "maxWidth", json!(260)),
                "elide",
                json!("end"),
            ));
        }
        row(12.0, vec![self.logo(24.0), column(0.0, lines)])
    }

    fn usage_bars(&self, now: u64) -> Value {
        let mut parts: Vec<Value> = self
            .plan
            .limits
            .iter()
            .map(|l| {
                let hot = l.percent >= 100.0
                    || (self.plan.threshold < 100.0 && l.percent >= self.plan.threshold);
                let tone = if self.plan.stale { "faint" } else { "muted" };
                let reset = if l.resets_at > 0.0 {
                    format!(" · {}", reset_in(l.resets_at, now as f64))
                } else {
                    String::new()
                };
                row(
                    6.0,
                    vec![
                        text(&l.label, tone, "caption"),
                        json!({"t": "progress", "value": (l.percent / 100.0).clamp(0.0, 1.0),
                               "tone": if hot { "danger" } else { "accent" },
                               "thickness": 4, "width": 56}),
                        text(
                            &format!("{} %{reset}", js::num_str(js::round(l.percent))),
                            if hot {
                                "danger"
                            } else if self.plan.stale {
                                "faint"
                            } else {
                                "foreground"
                            },
                            "caption",
                        ),
                    ],
                )
            })
            .collect();
        if self.plan.stale && !self.plan.problem.is_empty() {
            parts.push(text(&self.plan.problem, "faint", "caption"));
        }
        row(16.0, parts)
    }

    fn expanded(&self, now: u64) -> Value {
        let mut header = vec![
            self.logo(16.0),
            with(
                with(
                    text(
                        self.title_or_app(),
                        if self.session_title().is_empty() {
                            "muted"
                        } else {
                            "foreground"
                        },
                        "body",
                    ),
                    "fill",
                    json!(true),
                ),
                "elide",
                json!("end"),
            ),
        ];
        if self.working() {
            header.push(self.dot());
        }
        if self.running && !self.working() {
            header.push(with(
                icon_button(("view-refresh", 0xF0709), "restart"),
                "tooltip",
                json!("Start a new session"),
            ));
        }
        header.push(icon_button(("window-close", 0xF0156), "close"));
        let mut parts = vec![row(10.0, header)];
        if self.plan.shows() {
            parts.push(self.usage_bars(now));
        }
        // The terminal; the UI draws its frame. `fill` takes the width.
        parts.push(row(
            0.0,
            vec![with(
                json!({"t": "surface", "kind": "terminal", "ref": format!("term:{}", self.kind.id),
                       "id": format!("run-{}", self.surface_run()), "h": self.terminal_height()}),
                "fill",
                json!(true),
            )],
        ));
        if self.exited {
            parts.push(json!({"t": "button",
                "label": format!("{} exited. Enter or click to start it again.", self.app_name()),
                "variant": "outline", "action": "restart"}));
        }
        column(10.0, parts)
    }
}

impl CoreModule for TermModule {
    fn id(&self) -> &'static str {
        self.kind.id
    }

    fn title(&self) -> Option<String> {
        self.kind.custom.then(|| self.kind.app.clone())
    }

    fn facts(&self, _now: u64) -> Facts {
        Facts {
            priority: 40,
            active: true,
            attention: self.attention,
            expanded_width: self.expanded_width(),
            ..Facts::default()
        }
    }

    fn view(&self, slot: Slot, now: u64) -> Option<Node> {
        Some(node(match slot {
            Slot::Compact => self.compact(),
            Slot::VerticalCompact => self.vertical_compact(),
            Slot::Peek => self.peek(now),
            Slot::Expanded => self.expanded(now),
            Slot::VerticalPeek => return None,
        }))
    }

    fn config(&mut self, cfg: &Value, now: u64) {
        self.cfg = cfg.clone();
        if self.kind.custom {
            let entry = cfg.get("terminals").and_then(|t| t.get(self.kind.id));
            let text = |k: &str| {
                entry
                    .and_then(|e| e.get(k))
                    .and_then(Value::as_str)
                    .map(|s| js::trim(s).to_string())
                    .unwrap_or_default()
            };
            let title = text("title");
            self.kind.app = if title.is_empty() {
                self.kind.id.to_string()
            } else {
                title
            };
            self.kind.default_command = text("command");
        }
        let enabled = self.setting("usage") != Some(&Value::Bool(false));
        let warn = self.number("usageWarn", 80.0);
        self.plan.configure(enabled, warn, now);
        self.refresh_idle(now);
    }

    fn command(&mut self, ctx: &Ctx, method: &str, args: &[String], now: u64) -> Option<String> {
        // `agent <id> <command>` and `terminal <id> <command>` reach the right
        // module through the host; `claude <command>` and `codex <command>`
        // name it by the method. The command is what follows.
        let text = match method {
            "agent" | "terminal" => args.iter().skip(1).cloned().collect::<Vec<_>>().join(" "),
            m if m == self.kind.id => args.join(" "),
            _ => return None,
        };
        Some(self.command_text(ctx, &text, now))
    }

    fn event(&mut self, _ctx: &Ctx, action: &str, _value: &EventValue, now: u64) {
        match action {
            "typed" => self.typed = true,
            "restart" => self.restart(now),
            "close" | "leave" => self.effects.push(Effect::Notch(ModuleOp::Close)),
            "keyboard" => self.effects.push(Effect::Notch(ModuleOp::Keyboard)),
            _ => {}
        }
    }

    fn tick(&mut self, now: u64) {
        if let Some(ask) = self.plan.tick(now) {
            self.effects.push(Effect::Usage(ask));
        }
        if self.pending_ask.is_some() {
            self.deliver_ask(now);
        }
        if self.idle_due().is_some_and(|at| now >= at) {
            self.stop(now);
        }
        if self.minute_at.is_some_and(|at| now >= at) {
            self.minute_at = Some(now + 30_000);
        }
    }

    fn next_due(&self, _now: u64) -> Option<u64> {
        [
            self.plan.next_due(),
            self.pending_ask.as_ref().map(|(_, at)| *at),
            self.idle_due(),
            self.minute_at,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn state_extra(&self) -> Option<Value> {
        self.usage_state()
    }

    /// What the host knows of the terminal (`pty`) and of the plan usage
    /// (`usage`), each naming its module by `id`.
    fn feed(&mut self, provider: &str, data: &Value, now: u64) {
        if data.get("id").and_then(Value::as_str) != Some(self.kind.id) {
            return;
        }
        match provider {
            "pty" => {
                let run = data.get("run").and_then(Value::as_u64);
                match data.get("event").and_then(Value::as_str) {
                    Some("views") => {
                        let n = data.get("n").and_then(Value::as_u64).unwrap_or(0) as u32;
                        self.views_changed(n, now);
                    }
                    // Of a run that was replaced: not ours any more.
                    _ if run.is_some_and(|g| g != self.run) => {}
                    Some("title") => {
                        if self.running {
                            let t = data.get("title").and_then(Value::as_str).unwrap_or("");
                            self.set_raw_title(t, now);
                        }
                    }
                    Some("exit") => {
                        self.running = false;
                        self.exited = true;
                        self.pending_ask = None;
                        self.set_raw_title("", now);
                    }
                    _ => {}
                }
            }
            "usage" => {
                let alerts = self.plan.answer(data, now);
                self.alerts(alerts);
                self.refresh_minute(now);
            }
            _ => {}
        }
    }

    /// The test hook `test-<id>` (docs/modules.md): `usage <json>` is an answer of the
    /// endpoint, by hand (offscreen, nothing asks the network), `title <text>`
    /// a title the program set.
    fn demo(&mut self, args: &[String], now: u64) -> Option<String> {
        match args.first().map(String::as_str) {
            Some("usage") => {
                let data: Value = serde_json::from_str(args.get(1)?).ok()?;
                match self.plan.apply_json(&data, now) {
                    Some(alerts) => {
                        self.alerts(alerts);
                        self.refresh_minute(now);
                        Some("ok".into())
                    }
                    None => Some("not an answer".into()),
                }
            }
            // `title <text> [attention]`: a title the program set, and, with
            // `attention`, news you haven't seen.
            Some("title") => {
                self.running = true;
                self.set_raw_title(args.get(1).map_or("", String::as_str), now);
                self.attention = args.get(2).map(String::as_str) == Some("attention");
                Some("ok".into())
            }
            // `peek <ms>`: the peek of the last usage warning (or "done"), for that long.
            Some("peek") => {
                let ms = args.get(1).and_then(|m| m.parse::<u32>().ok())?;
                self.effects.push(Effect::Peek(ms));
                Some("ok".into())
            }
            _ => None,
        }
    }
}

impl TermModule {
    fn alerts(&mut self, alerts: Vec<Alert>) {
        for a in alerts {
            self.reason = PeekReason::Usage;
            self.effects
                .push(Effect::Peek(if a.level >= 100.0 { 8000 } else { 6000 }));
            self.alert = Some(a);
        }
    }

    /// The plan usage as `state` says it: `{ limits, stale, problem,
    /// fetchedAt }`, `None` while it is off or has nothing.
    pub fn usage_state(&self) -> Option<Value> {
        self.plan.active().then(|| {
            json!({"limits": self.plan.limits, "stale": self.plan.stale,
                   "problem": self.plan.problem, "fetchedAt": self.plan.fetched_at})
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notchd_proto::Validate;

    fn claude() -> TermModule {
        crate::claude::new()
    }

    fn ctx() -> Ctx {
        Ctx {
            terminal_ui: true,
            ..Ctx::default()
        }
    }

    fn feed(m: &mut TermModule, data: Value, now: u64) {
        m.feed("pty", &data, now);
    }

    fn start_effect(m: &mut TermModule) -> PtyStart {
        let fx = m.take_effects();
        fx.into_iter()
            .find_map(|e| match e {
                Effect::Pty(PtyCmd::Start(s)) => Some(s),
                _ => None,
            })
            .expect("a start")
    }

    fn texts(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                if let Some(Value::String(t)) = o.get("text") {
                    out.push(t.clone());
                }
                if let Some(Value::String(t)) = o.get("label") {
                    out.push(t.clone());
                }
                o.values().for_each(|c| texts(c, out));
            }
            Value::Array(a) => a.iter().for_each(|c| texts(c, out)),
            _ => {}
        }
    }

    fn all_texts(m: &TermModule, slot: Slot, now: u64) -> Vec<String> {
        let tree = serde_json::to_value(m.view(slot, now).expect("a view")).unwrap();
        let mut out = Vec::new();
        texts(&tree, &mut out);
        out
    }

    fn working(m: &mut TermModule, title: &str, now: u64) {
        feed(
            m,
            json!({"id": "claude", "event": "title", "run": m.run, "title": title}),
            now,
        );
    }

    #[test]
    fn the_program_starts_with_the_first_view_and_not_before() {
        let mut m = claude();
        m.config(&json!({}), 0);
        assert!(
            m.take_effects()
                .iter()
                .all(|e| !matches!(e, Effect::Pty(_)))
        );
        feed(
            &mut m,
            json!({"id": "claude", "event": "views", "n": 1}),
            10,
        );
        let s = start_effect(&mut m);
        assert_eq!((s.program.as_str(), s.run), ("/bin/sh", 1));
        assert_eq!(s.args[0], "-c");
        assert!(s.args[1].contains("exec claude \"$@\""), "{:?}", s.args);
        assert!(
            s.args[1].contains("xdg-user-dir"),
            "an agent starts in notchd's own folder"
        );
        // A second view, or the view going and coming back: the same program.
        feed(
            &mut m,
            json!({"id": "claude", "event": "views", "n": 2}),
            20,
        );
        feed(
            &mut m,
            json!({"id": "claude", "event": "views", "n": 0}),
            30,
        );
        feed(
            &mut m,
            json!({"id": "claude", "event": "views", "n": 1}),
            40,
        );
        assert!(
            m.take_effects()
                .iter()
                .all(|e| !matches!(e, Effect::Pty(_)))
        );
    }

    fn surface_id(m: &TermModule) -> String {
        let t = serde_json::to_value(m.view(Slot::Expanded, 0).unwrap()).unwrap();
        let s = t.to_string();
        let at = s.find("\"id\":\"run-").expect("a surface id") + 6;
        s[at..].split('"').next().unwrap().to_string()
    }

    #[test]
    fn the_surfaces_run_changes_with_a_new_session_and_not_when_the_first_view_starts_it() {
        let mut m = claude();
        m.config(&json!({}), 0);
        let before = surface_id(&m);
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 1);
        // Starting the program for the view that just attached: it is not replaced.
        assert_eq!(surface_id(&m), before);
        feed(
            &mut m,
            json!({"id": "claude", "event": "exit", "run": 1}),
            2,
        );
        assert_eq!(surface_id(&m), before, "an ended program keeps its view");
        m.event(&ctx(), "restart", &EventValue::Null(()), 3);
        assert_ne!(surface_id(&m), before, "a new session is a new view");
    }

    #[test]
    fn the_command_folder_and_font_come_from_the_settings() {
        let mut m = claude();
        m.config(
            &json!({"claude": {"command": "claude --permission-mode auto", "directory": "~/work"}}),
            0,
        );
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 1);
        let s = start_effect(&mut m);
        assert!(s.args[1].contains("exec claude --permission-mode auto \"$@\""));
        assert!(
            !s.args[1].contains("xdg-user-dir"),
            "with a folder, not notchd's own"
        );
        assert_eq!(s.dir, "~/work");
    }

    #[test]
    fn the_title_says_when_it_works_and_the_conversations_name() {
        let mut m = claude();
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 1);
        m.take_effects();
        working(&mut m, "✳ Claude Code", 2);
        assert_eq!((m.working(), m.session_title()), (false, ""));
        working(&mut m, "◐ Claude Code", 3);
        assert!(m.working());
        working(&mut m, "◑ Fake conversation", 4);
        assert_eq!(m.session_title(), "Fake conversation");
        assert_eq!(all_texts(&m, Slot::Compact, 5), vec!["Fake conversation"]);
        working(&mut m, "✳ Fake conversation", 5);
        assert!(!m.working());
        // Another run's title is not ours.
        feed(
            &mut m,
            json!({"id": "claude", "event": "title", "run": 99, "title": "◐ x"}),
            6,
        );
        assert!(!m.working());
        // Another module's is not either.
        feed(
            &mut m,
            json!({"id": "codex", "event": "title", "title": "⠋ x"}),
            6,
        );
        assert!(!m.working());
    }

    #[test]
    fn done_while_you_were_not_looking_peeks_once_and_leaves_a_dot_until_the_view_opens() {
        let mut m = claude();
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 1);
        m.event(&ctx(), "typed", &EventValue::Null(()), 2);
        working(&mut m, "◐ Claude Code", 3);
        // The view is open: finishing is not news.
        working(&mut m, "✳ Claude Code", 4);
        assert!(!m.attention);
        assert!(
            m.take_effects()
                .iter()
                .all(|e| !matches!(e, Effect::Peek(_)))
        );
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 0}), 5);
        working(&mut m, "◐ Claude Code", 6);
        assert!(!m.attention, "working: the dot is the green one");
        working(&mut m, "✳ Fake", 7);
        assert!(m.attention);
        assert!(m.take_effects().contains(&Effect::Peek(5000)));
        assert_eq!(all_texts(&m, Slot::Peek, 8), vec!["Claude is done", "Fake"]);
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 9);
        assert!(!m.attention, "its view opened");
    }

    #[test]
    fn without_typing_nothing_is_news() {
        let mut m = claude();
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 0}), 1);
        m.take_effects();
        working(&mut m, "◐ Claude Code", 3);
        working(&mut m, "✳ Claude Code", 4);
        assert!(!m.attention);
    }

    #[test]
    fn it_ends_and_leaves_its_session_until_a_new_one() {
        let mut m = claude();
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 1);
        m.take_effects();
        working(&mut m, "✳ Fake", 2);
        feed(
            &mut m,
            json!({"id": "claude", "event": "exit", "run": 1}),
            3,
        );
        assert!(m.exited && !m.running);
        assert_eq!(m.session_title(), "", "no title once it is gone");
        let t = all_texts(&m, Slot::Expanded, 4);
        assert!(
            t.contains(&"Claude exited. Enter or click to start it again.".to_string()),
            "{t:?}"
        );
        m.event(&ctx(), "restart", &EventValue::Null(()), 5);
        let fx = m.take_effects();
        assert!(matches!(fx[0], Effect::Pty(PtyCmd::Stop)));
        let Effect::Pty(PtyCmd::Start(s)) = &fx[1] else {
            panic!("{fx:?}")
        };
        assert!(s.run > 1, "a new run");
        assert!(!m.exited && m.running);
        // The old run's end, arriving late, changes nothing.
        feed(
            &mut m,
            json!({"id": "claude", "event": "exit", "run": 1}),
            6,
        );
        assert!(m.running && !m.exited);
    }

    #[test]
    fn the_ipc_commands_and_their_answers() {
        let mut m = claude();
        let say =
            |m: &mut TermModule, c: &str| m.command(&ctx(), "claude", &[c.to_string()], 0).unwrap();
        assert_eq!(
            say(&mut m, "nonsense"),
            "usage: agent claude toggle | new | \"ask <question>\""
        );
        // toggle opens, then closes while a view shows it.
        assert_eq!(say(&mut m, "toggle"), "ok");
        assert_eq!(m.take_effects(), vec![Effect::Notch(ModuleOp::Open)]);
        // The notch is open on it, though no view attached yet.
        let shown = Ctx {
            visible: true,
            ..ctx()
        };
        assert_eq!(
            m.command(&shown, "claude", &["toggle".to_string()], 0)
                .unwrap(),
            "ok"
        );
        assert_eq!(m.take_effects(), vec![Effect::Notch(ModuleOp::Close)]);
        // new starts another.
        assert_eq!(say(&mut m, "new"), "ok");
        assert!(matches!(
            m.take_effects().last(),
            Some(Effect::Pty(PtyCmd::Start(_)))
        ));
        // `agent <id> <command>` carries the id first; other ids are not ours.
        assert_eq!(
            m.command(
                &ctx(),
                "agent",
                &["claude".into(), "ask".into(), "a".into(), "b".into()],
                0
            )
            .unwrap(),
            "ok"
        );
        assert_eq!(m.command(&ctx(), "codex", &["new".into()], 0), None);
    }

    #[test]
    fn ask_types_the_question_once_the_program_has_a_title() {
        let mut m = claude();
        let none = Ctx::default();
        assert_eq!(
            m.command(&none, "claude", &["ask hi".into()], 0).unwrap(),
            "no UI connected draws a terminal"
        );
        assert_eq!(
            m.command(
                &ctx(),
                "claude",
                &["ask why is the sky   blue?\n".into()],
                0
            )
            .unwrap(),
            "ok"
        );
        let fx = m.take_effects();
        assert!(fx.contains(&Effect::Notch(ModuleOp::Open)));
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Pty(PtyCmd::Start(_))))
        );
        // Not before it has a title: it is still starting.
        m.tick(500);
        assert!(m.take_effects().is_empty());
        assert_eq!(m.next_due(500), Some(1000));
        working(&mut m, "✳ Claude Code", 600);
        m.tick(1000);
        assert_eq!(
            m.take_effects(),
            vec![Effect::Pty(PtyCmd::Write(
                b"why is the sky   blue?\r".to_vec()
            ))]
        );
        assert!(m.typed);
        assert_eq!(m.next_due(1000), None);
        // `ask` alone only opens it.
        m.command(&ctx(), "claude", &["ask".into()], 2000).unwrap();
        assert_eq!(m.take_effects(), vec![Effect::Notch(ModuleOp::Open)]);
        assert_eq!(m.pending_ask, None);
    }

    #[test]
    fn closed_and_idle_for_idle_minutes_it_stops_and_the_next_view_starts_a_new_one() {
        let mut m = claude();
        m.config(&json!({"claude": {"idleMinutes": 1, "usage": false}}), 0);
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 0);
        m.take_effects();
        assert_eq!(m.next_due(0), None, "open: it does not idle");
        feed(
            &mut m,
            json!({"id": "claude", "event": "views", "n": 0}),
            1000,
        );
        let due = m.next_due(1000).unwrap();
        assert_eq!(due, 1000 + 60_000);
        // Working holds it back.
        working(&mut m, "◐ Claude Code", 2000);
        assert_eq!(m.next_due(2000), None);
        working(&mut m, "✳ Claude Code", 3000);
        let due = m.next_due(3000).unwrap();
        m.tick(due);
        assert_eq!(m.take_effects(), vec![Effect::Pty(PtyCmd::Stop)]);
        assert!(!m.running && !m.exited);
        feed(
            &mut m,
            json!({"id": "claude", "event": "views", "n": 1}),
            due + 5,
        );
        assert!(start_effect(&mut m).run > 1, "a new run");
    }

    #[test]
    fn the_usage_shows_in_the_open_notch_and_a_limit_passing_the_warning_peeks() {
        let mut m = claude();
        m.config(&json!({}), 0);
        let answer = |p: f64| {
            json!({"five_hour": {"utilization": p, "resets_at": "2099-01-01T00:00:00Z"},
                   "seven_day": {"utilization": 10.0, "resets_at": "2099-01-02T00:00:00Z"}})
        };
        assert!(
            !all_texts(&m, Slot::Expanded, 0)
                .iter()
                .any(|t| t.contains('%'))
        );
        let demo = |m: &mut TermModule, p: f64| {
            m.demo(&["usage".into(), answer(p).to_string()], 1).unwrap()
        };
        assert_eq!(demo(&mut m, 10.0), "ok");
        assert!(
            m.take_effects().is_empty(),
            "the first answer is taken as seen"
        );
        demo(&mut m, 85.0);
        assert_eq!(m.take_effects(), vec![Effect::Peek(6000)]);
        let t = all_texts(&m, Slot::Expanded, 2);
        assert!(
            t.iter().any(|t| t.starts_with("85 %")) && t.contains(&"5h".to_string()),
            "{t:?}"
        );
        assert_eq!(all_texts(&m, Slot::Peek, 3)[0], "Claude: 5h limit at 85 %");
        demo(&mut m, 100.0);
        assert_eq!(m.take_effects(), vec![Effect::Peek(8000)]);
        assert_eq!(all_texts(&m, Slot::Peek, 3)[0], "Claude: 5h limit reached");
        assert!(
            m.demo(&["usage".into(), "{\"nope\":1}".into()], 4) == Some("not an answer".into())
        );
        // Switched off, it shows nothing and asks nothing.
        m.config(&json!({"claude": {"usage": false}}), 5);
        assert!(
            !all_texts(&m, Slot::Expanded, 6)
                .iter()
                .any(|t| t.contains('%'))
        );
        assert_eq!(m.plan.next_due(), None);
    }

    #[test]
    fn a_usage_answer_comes_from_the_provider_and_a_finished_task_asks_again() {
        let mut m = claude();
        m.config(&json!({}), 1000);
        assert_eq!(m.next_due(1000), Some(1000));
        m.tick(1000);
        assert!(matches!(m.take_effects()[..], [Effect::Usage(_)]));
        let body = json!({"five_hour": {"utilization": 10.0}, "seven_day": {"utilization": 5.0}});
        m.feed(
            "usage",
            &json!({"id": "claude", "status": 200, "body": body.to_string(), "retry_after": ""}),
            1500,
        );
        assert!(m.plan.shows());
        // A task that ended asks again once a minute has passed.
        feed(
            &mut m,
            json!({"id": "claude", "event": "views", "n": 1}),
            70_000,
        );
        m.take_effects();
        m.event(&ctx(), "typed", &EventValue::Null(()), 70_001);
        working(&mut m, "◐ Claude Code", 70_002);
        working(&mut m, "✳ Claude Code", 70_003);
        assert!(
            m.take_effects()
                .iter()
                .any(|e| matches!(e, Effect::Usage(_)))
        );
    }

    #[test]
    fn the_views_are_valid_trees_in_every_state() {
        let mut m = claude();
        m.config(&json!({}), 0);
        let check = |m: &TermModule| {
            for slot in Slot::ALL {
                if let Some(n) = m.view(slot, 5) {
                    n.validate().unwrap();
                }
            }
        };
        check(&m);
        feed(&mut m, json!({"id": "claude", "event": "views", "n": 1}), 1);
        working(&mut m, "◐ Claude Code", 2);
        check(&m);
        feed(
            &mut m,
            json!({"id": "claude", "event": "exit", "run": 1}),
            3,
        );
        check(&m);
        let t = serde_json::to_value(m.view(Slot::Expanded, 4).unwrap()).unwrap();
        let s = t.to_string();
        assert!(
            s.contains("\"t\":\"surface\"") && s.contains("\"ref\":\"term:claude\""),
            "{s}"
        );
        assert!(m.view(Slot::Compact, 4).is_some() && m.view(Slot::VerticalPeek, 4).is_none());
    }

    #[test]
    fn a_terminal_of_your_own_takes_its_title_and_command_from_its_entry() {
        let mut m = crate::terminal::new("mine");
        m.config(
            &json!({"terminals": {"mine": {"title": "Mine", "command": "cd ~/app && npm run dev"}},
                    "mine": {"directory": "~/app"}}),
            0,
        );
        assert_eq!(m.title().as_deref(), Some("Mine"));
        feed(&mut m, json!({"id": "mine", "event": "views", "n": 1}), 1);
        let s = start_effect(&mut m);
        assert!(
            s.args[1].contains("cd ~/app && npm run dev"),
            "a script, as is: {:?}",
            s.args
        );
        assert!(!s.args[1].contains("exec "), "{:?}", s.args);
        assert!(!s.args[1].contains("xdg-user-dir"));
        // Its title is the program's, never "working".
        feed(
            &mut m,
            json!({"id": "mine", "event": "title", "run": 1, "title": "Own title"}),
            2,
        );
        assert_eq!((m.working(), m.session_title()), (false, "Own title"));
        assert_eq!(
            m.command(&ctx(), "terminal", &["mine".into(), "nope".into()], 0)
                .unwrap(),
            "usage: terminal mine toggle | new | \"ask <question>\""
        );
        // Without a title, its id.
        m.config(&json!({"terminals": {"mine": {"command": "x"}}}), 3);
        assert_eq!(m.title().as_deref(), Some("mine"));
    }
}
