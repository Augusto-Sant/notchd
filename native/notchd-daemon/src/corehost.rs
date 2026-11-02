//! The modules that live in the daemon (`notchd-modules`: timer, activities,
//! resources, osd), driven the way the module host drives a user module's
//! process: they are loaded while a notch lists them (a standalone one:
//! unless switched off), told the config, the time, a provider's data, an
//! IPC command or an `event`, and after each call their facts, views and
//! effects (a peek, a notification) go to the store and the UIs, only what
//! changed. Pure over a [`Store`] and [`Mods`] and a time, like `modstate`;
//! the hub owns the lock, the clock and the providers.

use crate::modstate::Mods;
use crate::ptyhost::{Note, PtyHost, Session};
use crate::sidecars::Sidecars;
use notchd_core::store::Store;
use notchd_modules::module::{Bridge, PtyCmd, Shown, SidecarCmd};
use notchd_modules::plan::Ask;
use notchd_modules::{CoreModule, Ctx, Effect};
use notchd_proto::common::Mode;
use notchd_proto::{EventValue, Slot, ToUi};
use notchd_providers::media::MediaCommand;
use notchd_providers::notifications::Notifier;
use notchd_providers::pty::PtySpec;
use notchd_providers::usage::{self as usage_provider, Curl, Http};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::mpsc::Sender;

/// The IPC methods a module of the daemon answers, by module. A method of
/// one that is not loaded is answered `<id> module not loaded`.
const METHODS: &[(&str, &str)] = &[
    ("push", "activities"),
    ("clear", "activities"),
    ("timer", "timer"),
    ("osd", "osd"),
    ("claude", "claude"),
    ("codex", "codex"),
];

/// The agents `agent <id> ...` takes, in the order they are listed.
const AGENTS: &[&str] = &["claude", "codex"];

/// The spoken conversations `live <id> ...` takes.
const LIVES: &[&str] = &["codex-live", "local-live"];

/// The modules told how the notches show them (`CoreModule::shown`): the
/// conversations follow their notch opening, media ticks its clock only while
/// its expanded view is on screen.
const SHOWN_AWARE: &[&str] = &["codex-live", "local-live", "media"];

/// How often the installer is asked for its state while the settings are
/// open and something installs.
const COMPONENT_POLL_MS: u64 = 1000;

/// How the daemon reads its environment: a closure, so tests give it their
/// own (a scratch `HOME`, no real login).
pub type Env = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The module that owns an IPC method.
pub fn owner(method: &str) -> Option<&'static str> {
    METHODS
        .iter()
        .find(|(m, _)| *m == method)
        .map(|(_, id)| *id)
}

pub struct CoreHost {
    loaded: BTreeMap<&'static str, Box<dyn CoreModule>>,
    /// When each wants `tick`, as it said after its last change.
    due: BTreeMap<&'static str, u64>,
    notifier: Option<Box<dyn Notifier>>,
    /// The media provider's commands (its thread makes the D-Bus calls, never
    /// this one, which runs under the hub's lock).
    media: Option<Sender<MediaCommand>>,
    /// The players of the system feed `media`; off once a test hook gave it
    /// demo players.
    media_live: bool,
    /// The system's readings feed `resources`; off once a test hook gave it
    /// demo values.
    pub sampling: bool,
    /// The programs of the terminal modules, and what they (and the plan
    /// usage's requests) tell their modules.
    ptys: PtyHost,
    notes: Option<Sender<Note>>,
    env: Env,
    http: Arc<dyn Http>,
    /// Some UI can draw a terminal (`surfaces` of its `hello`).
    terminal_ui: bool,
    /// The voice's bridges, and what the notches showed of each module last.
    sidecars: Sidecars,
    shown_seen: BTreeMap<&'static str, Shown>,
    /// The optional components: when the installer is asked next, whether a
    /// `status` is out, and whether the settings were open at the last look.
    comp_next: u64,
    comp_busy: bool,
    comp_settings: bool,
}

impl Default for CoreHost {
    fn default() -> CoreHost {
        CoreHost::new(notchd_providers::notifications::system())
    }
}

impl CoreHost {
    pub fn new(notifier: Option<Box<dyn Notifier>>) -> CoreHost {
        CoreHost {
            loaded: BTreeMap::new(),
            due: BTreeMap::new(),
            notifier,
            media: None,
            media_live: true,
            sampling: true,
            ptys: PtyHost::default(),
            notes: None,
            env: Arc::new(|k| std::env::var(k).ok()),
            http: Arc::new(Curl),
            terminal_ui: false,
            sidecars: Sidecars::default(),
            shown_seen: BTreeMap::new(),
            comp_next: 0,
            comp_busy: false,
            comp_settings: false,
        }
    }

    /// Where `media`'s commands go. A module that loaded before this asks for the players now.
    pub fn set_media(&mut self, commands: Sender<MediaCommand>) {
        self.media = Some(commands);
        if self.loaded.contains_key("media") {
            self.send_media(MediaCommand::Refresh);
        }
    }

    /// Whether the module `media` wants the system's players.
    pub fn wants_media(&self) -> bool {
        self.media_live && self.loaded.contains_key("media")
    }

    /// The environment the terminals' programs and the usage requests read
    /// (`HOME`, `CLAUDE_CONFIG_DIR`, `NOTCHD_OFFLINE`).
    pub fn with_env(mut self, env: Env) -> CoreHost {
        self.env = env;
        self
    }

    /// What asks the network for the plan usage.
    pub fn with_http(mut self, http: Arc<dyn Http>) -> CoreHost {
        self.http = http;
        self
    }

    /// Where the modules' terminals and usage requests report to: the hub
    /// feeds each note to its module.
    pub fn set_notes(&mut self, notes: Sender<Note>) {
        self.ptys.set_notes(notes.clone());
        self.sidecars.set_notes(notes.clone());
        self.notes = Some(notes);
    }

    /// Some UI connected can (not) draw a terminal.
    pub fn set_terminal_ui(&mut self, on: bool) {
        self.terminal_ui = on;
    }

    /// The session of a terminal module, for a view to attach to.
    pub fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.ptys.session(id)
    }

    /// The size a program starts at: the first view's.
    pub fn set_size_hint(&mut self, cols: u16, rows: u16) {
        self.ptys.hint = (cols, rows);
    }

    /// Stops every terminal's program (the daemon is going).
    pub fn stop_terminals(&mut self) {
        self.ptys.stop_all();
        self.sidecars.stop_all();
    }

    pub fn has(&self, id: &str) -> bool {
        self.loaded.contains_key(id)
    }

    /// Whether the module `id` wants the system's readings.
    pub fn wants_resources(&self) -> bool {
        self.sampling && self.loaded.contains_key("resources")
    }

    /// What a module's rules need to know of the notches.
    fn ctx(store: &Store, id: &str) -> Ctx {
        let st = store.state();
        let mut seen: Vec<(Mode, Option<String>)> = Vec::new();
        for c in st.notches.iter().flat_map(|n| n.copies.iter()) {
            seen.push((c.mode, c.peek.as_ref().map(|p| p.module.clone())));
        }
        for c in st
            .standalone
            .iter()
            .flat_map(|n| n.copies.iter())
            .chain(st.visitor.iter())
        {
            seen.push((c.mode, c.peek.as_ref().map(|p| p.module.clone())));
        }
        let mut ctx = Ctx {
            listed: st.notches.iter().any(|n| n.modules.iter().any(|m| m == id)),
            visible: Self::shown_of(store, id).visible,
            ..Ctx::default()
        };
        for (mode, peek) in seen {
            ctx.expanded |= mode == Mode::Expanded;
            ctx.peeking |= mode == Mode::Peek && peek.as_deref() == Some(id);
        }
        ctx
    }

    /// How the notches show module `id` now: open on it by a click or a
    /// shortcut (pinned), and its expanded view on screen at all.
    fn shown_of(store: &Store, id: &str) -> Shown {
        let st = store.state();
        let mut out = Shown::default();
        let mut see = |mode: Mode, pinned: bool, shown: &[String]| {
            if mode == Mode::Expanded && shown.iter().any(|m| m == id) {
                out.visible = true;
                out.open |= pinned;
            }
        };
        for c in st.notches.iter().flat_map(|n| n.copies.iter()) {
            see(c.mode, c.pinned, &c.shown);
        }
        for c in st
            .standalone
            .iter()
            .flat_map(|n| n.copies.iter())
            .chain(st.visitor.iter())
        {
            see(c.mode, c.pinned, &c.shown);
        }
        out
    }

    /// The notch state changed: tells the modules whose answer did how the
    /// notches show them now (a voice session follows its notch opening).
    pub fn notches_changed(&mut self, store: &mut Store, mods: &mut Mods, now: u64) -> Vec<ToUi> {
        let ids: Vec<&'static str> = self.loaded.keys().copied().collect();
        let mut out = Vec::new();
        for id in ids {
            if !SHOWN_AWARE.contains(&id) {
                continue;
            }
            let shown = Self::shown_of(store, id);
            let before = self.shown_seen.get(id).copied().unwrap_or_default();
            if before == shown {
                continue;
            }
            self.shown_seen.insert(id, shown);
            if let Some(m) = self.loaded.get_mut(id) {
                m.shown(shown, now);
            }
            out.extend(self.settle(store, mods, id, now));
        }
        out
    }

    /// Loads and unloads to match the config, hands the loaded modules the
    /// config, and returns what the UIs need. A module that is not listed
    /// anywhere is gone with its state, as an unloaded QML module was.
    pub fn sync(&mut self, store: &mut Store, mods: &mut Mods, now: u64) -> Vec<ToUi> {
        let cfg = store.config().clone();
        let listed: Vec<String> = store
            .notch_specs()
            .into_iter()
            .flat_map(|n| n.module_ids)
            .collect();
        let mut out = Vec::new();
        // The modules of the daemon, and a terminal for each entry of the
        // config's `terminals`; one that left the config is unloaded.
        let dynamic: Vec<&'static str> = notchd_modules::terminal::ids(&cfg)
            .iter()
            .map(|i| notchd_modules::terminal::intern(i))
            .collect();
        let mut candidates: BTreeSet<&'static str> = notchd_modules::IDS.iter().copied().collect();
        candidates.extend(dynamic.iter().copied());
        candidates.extend(self.loaded.keys().copied());
        let make = |id: &str| -> Option<Box<dyn CoreModule>> {
            notchd_modules::create(id).or_else(|| {
                dynamic
                    .contains(&id)
                    .then(|| Box::new(notchd_modules::terminal::new(id)) as Box<dyn CoreModule>)
            })
        };
        for id in candidates {
            let known = notchd_modules::IDS.contains(&id) || dynamic.contains(&id);
            let probe = known
                && self
                    .loaded
                    .get(id)
                    .map(|m| m.standalone())
                    .or_else(|| make(id).map(|m| m.standalone()))
                    .unwrap_or(false);
            let off = notchd_core::config::value_at(&cfg, &format!("{id}.enabled"))
                == Some(&Value::Bool(false));
            // A standalone module belongs to a host (the OSD to the one that
            // forwards its IPC target): it runs once the host declares it.
            let wanted = known
                && if probe {
                    !off && store.ui_declares(id)
                } else {
                    listed.iter().any(|l| l == id)
                };
            if wanted && !self.loaded.contains_key(id) {
                if let Some(m) = make(id) {
                    self.loaded.insert(id, m);
                    if id == "media" {
                        // It wants the players as they are now.
                        self.send_media(MediaCommand::Refresh);
                    }
                }
            } else if !wanted && self.loaded.remove(id).is_some() {
                self.due.remove(id);
                // Its program goes with it, as its terminal did.
                self.ptys.stop(id);
                self.sidecars.stop(id);
                self.shown_seen.remove(id);
                store.set_module_extra(id, None);
                out.extend(mods.apply(
                    store,
                    crate::modhost::ModEvent::Removed { module: id.into() },
                    now,
                ));
            }
        }
        let ids: Vec<&'static str> = self.loaded.keys().copied().collect();
        for id in ids {
            if let Some(m) = self.loaded.get_mut(id) {
                m.config(&cfg, now);
            }
            out.extend(self.settle(store, mods, id, now));
        }
        out.extend(self.notches_changed(store, mods, now));
        out
    }

    fn send_media(&self, c: MediaCommand) {
        if let Some(tx) = &self.media {
            let _ = tx.send(c);
        }
    }

    /// A module's facts, views and effects, after something changed it.
    fn settle(&mut self, store: &mut Store, mods: &mut Mods, id: &str, now: u64) -> Vec<ToUi> {
        let Some(m) = self.loaded.get_mut(id) else {
            return Vec::new();
        };
        let facts = m.facts(now);
        let views = Slot::ALL.iter().map(|s| (*s, m.view(*s, now))).collect();
        let effects = m.take_effects();
        match m.next_due(now) {
            Some(at) => self.due.insert(m.id(), at),
            None => self.due.remove(m.id()),
        };
        let title = m
            .title()
            .unwrap_or_else(|| notchd_modules::title(id).to_string());
        let standalone = m.standalone();
        store.set_module_extra(id, m.state_extra());
        let out = mods.set_builtin(store, id, &title, standalone, &facts, views);
        for e in effects {
            match e {
                Effect::Peek(ms) => store.request_peek(id, Some(f64::from(ms)), now),
                Effect::DismissPeek => store.dismiss_peek(id),
                Effect::Media(c) => self.send_media(c),
                Effect::Notify { title, body } => {
                    if let Some(n) = &self.notifier {
                        n.notify(&title, &body);
                    }
                }
                Effect::Pty(cmd) => self.pty_command(id, cmd),
                Effect::Sidecar(cmd) => self.sidecar_command(store, id, cmd),
                Effect::Usage(ask) => self.ask_usage(id, ask),
                Effect::Notch(op) => store.module_op(id, op, now),
            }
        }
        out
    }

    /// Where a file of the UI's `core/` is: its folder (`catalog`), else
    /// `NOTCHD_CORE_DIR`.
    fn core_file(&self, store: &Store, rel: &str) -> String {
        let dir = store
            .core_dir()
            .map(String::from)
            .or_else(|| (self.env)("NOTCHD_CORE_DIR"))
            .unwrap_or_default();
        format!("{}/{rel}", dir.trim_end_matches('/'))
    }

    /// What a voice module asked of its bridge.
    fn sidecar_command(&mut self, store: &Store, id: &str, cmd: SidecarCmd) {
        match cmd {
            SidecarCmd::Start(s) => {
                let home = (self.env)("HOME").unwrap_or_default();
                let argv = match &s.bridge {
                    Bridge::Codex(o) => {
                        let path = (self.env)("NOTCHD_LIVE_BRIDGE")
                            .unwrap_or_else(|| self.core_file(store, "live/codex-live.py"));
                        notchd_core::live::bridge_args(&path, o, &home)
                    }
                    Bridge::Local(o) => {
                        let path = (self.env)("NOTCHD_LOCAL_LIVE_BRIDGE")
                            .unwrap_or_else(|| self.core_file(store, "live/local-live.py"));
                        notchd_core::live::local_bridge_args(&path, o, &home)
                    }
                };
                self.sidecars.start(id, s.run, &argv);
            }
            SidecarCmd::Write(line) => self.sidecars.write(id, &line),
            SidecarCmd::Term => self.sidecars.terminate(id),
        }
    }

    // ---------------------------------------------------------- components

    /// The installer's command line (`NOTCHD_COMPONENTS_INSTALLER`, else
    /// `core/components/install.py` of the UI's core).
    fn installer(&self, store: &Store, args: &[&str]) -> Vec<String> {
        let script = (self.env)("NOTCHD_COMPONENTS_INSTALLER")
            .unwrap_or_else(|| self.core_file(store, "components/install.py"));
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        notchd_core::components::installer_command(&script, &args)
    }

    /// Runs the installer on a thread of its own (an action, then a `status`
    /// to see what it did), and tells the hub what `status` said.
    fn run_installer(&mut self, store: &Store, action: Option<(&str, &str)>) {
        let Some(notes) = self.notes.clone() else {
            return;
        };
        let status = self.installer(store, &["status"]);
        let act = action.map(|(verb, id)| self.installer(store, &[verb, id]));
        self.comp_busy = true;
        let _ = std::thread::Builder::new()
            .name("notchd-components".into())
            .spawn(move || {
                use notchd_providers::components::{ACTION_LIMIT, STATUS_LIMIT, run};
                if let Some(act) = act {
                    let _ = run(&act, &[], ACTION_LIMIT);
                }
                let text = run(&status, &[], STATUS_LIMIT)
                    .map(|r| r.stdout)
                    .unwrap_or_default();
                let _ = notes.send(Note {
                    provider: "components",
                    data: json!({"status": text}),
                });
            });
    }

    /// What the installer said of the components (a `components` note).
    pub fn components_note(&mut self, store: &mut Store, data: &Value) {
        self.comp_busy = false;
        let text = data.get("status").and_then(Value::as_str).unwrap_or("");
        let states = notchd_core::components::parse_status(text);
        if !states.is_empty() {
            store.set_component_states(states);
        }
    }

    /// Asks the installer for its state while the settings are open and it
    /// is not known yet, the window just opened, or something installs.
    fn poll_components(&mut self, store: &Store, now: u64) {
        let open = store.settings_open;
        let opened = open && !self.comp_settings;
        self.comp_settings = open;
        if !open || self.comp_busy {
            return;
        }
        let active = store
            .component_states()
            .is_none_or(notchd_core::components::any_active);
        if opened || (active && now >= self.comp_next) {
            self.comp_next = now + COMPONENT_POLL_MS;
            self.run_installer(store, None);
        }
    }

    /// `component status [id] | install <id> | cancel <id> | remove <id>`.
    fn component_command(&mut self, store: &mut Store, args: &[String]) -> String {
        use notchd_core::components as c;
        let usage = "usage: component status [id] | install <id> | cancel <id> | remove <id>";
        let verb = args.first().map_or("", String::as_str);
        let id = args.get(1).map_or("", String::as_str);
        let known = c::by_id(store.component_catalog(), id).is_some();
        let unknown = {
            let ids: Vec<&str> = store
                .component_catalog()
                .iter()
                .map(|k| k.id.as_str())
                .collect();
            format!("no component {id}; components: {}", ids.join(", "))
        };
        match verb {
            "status" => {
                if !id.is_empty() && !known {
                    return unknown;
                }
                if store.component_states().is_none() {
                    let argv = self.installer(store, &["status"]);
                    if let Ok(r) = notchd_providers::components::run(
                        &argv,
                        &[],
                        notchd_providers::components::STATUS_LIMIT,
                    ) {
                        let states = c::parse_status(&r.stdout);
                        if !states.is_empty() {
                            store.set_component_states(states);
                        }
                    }
                }
                let states = store.component_states().cloned().unwrap_or_default();
                let installed = c::installed_ids(&states);
                store
                    .component_catalog()
                    .iter()
                    .filter(|k| id.is_empty() || k.id == id)
                    .map(|k| {
                        let shared = c::shares_backend(k, store.component_catalog(), &installed);
                        format!("{}: {}", k.id, c::view(k, states.get(&k.id), shared).text)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "install" | "cancel" | "remove" => {
                if id.is_empty() {
                    return usage.into();
                }
                if !known {
                    return unknown;
                }
                self.run_installer(store, Some((verb, id)));
                "ok".into()
            }
            _ => usage.into(),
        }
    }

    /// What a terminal module asked of its program.
    fn pty_command(&mut self, id: &str, cmd: PtyCmd) {
        match cmd {
            PtyCmd::Start(s) => {
                let home = (self.env)("HOME").unwrap_or_default();
                let spec = PtySpec {
                    program: s.program,
                    args: s.args,
                    cwd: Some(notchd_core::terminal::expand_home(Some(&s.dir), &home)),
                    env: vec![("TERM".into(), "xterm-256color".into())],
                    cols: 0,
                    rows: 0,
                };
                self.ptys.start(id, s.run, spec);
            }
            PtyCmd::Stop => self.ptys.stop(id),
            PtyCmd::Write(bytes) => self.ptys.write(id, &bytes),
        }
    }

    /// Asks the plan usage on a thread of its own, so a slow network never
    /// holds the hub; the answer comes back as a note. Offscreen runs and
    /// tests (`NOTCHD_OFFLINE`) never ask.
    fn ask_usage(&self, id: &str, ask: Ask) {
        if (self.env)("NOTCHD_OFFLINE").as_deref() == Some("1") {
            return;
        }
        let Some(notes) = self.notes.clone() else {
            return;
        };
        let (env, http, id) = (
            Arc::clone(&self.env),
            Arc::clone(&self.http),
            id.to_string(),
        );
        let _ = std::thread::Builder::new()
            .name("notchd-usage".into())
            .spawn(move || {
                let mut data = usage_provider::fetch(
                    ask.provider,
                    ask.unless_login_is,
                    usage_provider::epoch_ms(),
                    &*env,
                    &*http,
                );
                data["id"] = json!(id);
                let _ = notes.send(Note {
                    provider: "usage",
                    data,
                });
            });
    }

    /// An IPC method of a module of the daemon: the answer and what changed.
    /// `None` when no module of the daemon owns the method.
    pub fn command(
        &mut self,
        store: &mut Store,
        mods: &mut Mods,
        method: &str,
        args: &[String],
        now: u64,
    ) -> Option<(String, Vec<ToUi>)> {
        let id = match method {
            // `agent <id> ...` and `terminal <id> ...` name the module in
            // their first argument.
            "agent" => {
                let want = args.first().map_or("", String::as_str);
                match AGENTS.iter().find(|a| **a == want) {
                    Some(a) => *a,
                    None => {
                        return Some((
                            format!("no agent {want}; agents: {}", AGENTS.join(", ")),
                            Vec::new(),
                        ));
                    }
                }
            }
            "live" => {
                let want = args.first().map_or("", String::as_str);
                match LIVES.iter().find(|a| **a == want) {
                    Some(a) => *a,
                    None => {
                        return Some((
                            format!("no live module {want}; live modules: {}", LIVES.join(", ")),
                            Vec::new(),
                        ));
                    }
                }
            }
            "component" => return Some((self.component_command(store, args), Vec::new())),
            "terminal" => {
                let want = args.first().map_or("", String::as_str);
                let list = notchd_modules::terminal::ids(store.config());
                if !list.iter().any(|t| t == want) {
                    let named = if list.is_empty() {
                        String::new()
                    } else {
                        format!("; terminals: {}", list.join(", "))
                    };
                    return Some((format!("no terminal {want}{named}"), Vec::new()));
                }
                notchd_modules::terminal::intern(want)
            }
            _ => owner(method)?,
        };
        let mut ctx = Self::ctx(store, id);
        ctx.terminal_ui = self.terminal_ui;
        let Some(m) = self.loaded.get_mut(id) else {
            return Some((format!("{id} module not loaded"), Vec::new()));
        };
        let answer = m.command(&ctx, method, args, now)?;
        Some((answer, self.settle(store, mods, id, now)))
    }

    /// A primitive's `action` in a module's view.
    pub fn event(
        &mut self,
        store: &mut Store,
        mods: &mut Mods,
        id: &str,
        action: &str,
        value: &EventValue,
        now: u64,
    ) -> Vec<ToUi> {
        let ctx = Self::ctx(store, id);
        let Some(m) = self.loaded.get_mut(id) else {
            return Vec::new();
        };
        m.event(&ctx, action, value, now);
        self.settle(store, mods, id, now)
    }

    /// A provider's data, for the modules that take it.
    pub fn feed(
        &mut self,
        store: &mut Store,
        mods: &mut Mods,
        provider: &str,
        data: &Value,
        now: u64,
    ) -> Vec<ToUi> {
        if provider == "media" && !self.media_live {
            return Vec::new();
        }
        let ids: Vec<&'static str> = self.loaded.keys().copied().collect();
        let mut out = Vec::new();
        for id in ids {
            if let Some(m) = self.loaded.get_mut(id) {
                m.feed(provider, data, now);
            }
            out.extend(self.settle(store, mods, id, now));
        }
        out
    }

    /// What happened to one module's terminal or usage request (`pty`,
    /// `usage`): fed to that module alone.
    pub fn feed_to(
        &mut self,
        store: &mut Store,
        mods: &mut Mods,
        id: &str,
        provider: &str,
        data: &Value,
        now: u64,
    ) -> Vec<ToUi> {
        let Some(m) = self.loaded.get_mut(id) else {
            return Vec::new();
        };
        m.feed(provider, data, now);
        self.settle(store, mods, id, now)
    }

    /// The views attached to a terminal module's session are `n` now (the
    /// first one starts the program).
    pub fn pty_views(
        &mut self,
        store: &mut Store,
        mods: &mut Mods,
        id: &str,
        n: usize,
        now: u64,
    ) -> Vec<ToUi> {
        let data = json!({"id": id, "event": "views", "n": n});
        self.feed_to(store, mods, id, "pty", &data, now)
    }

    /// Runs the modules whose time has come.
    pub fn tick(&mut self, store: &mut Store, mods: &mut Mods, now: u64) -> Vec<ToUi> {
        self.poll_components(store, now);
        let due: Vec<&'static str> = self
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        let mut out = Vec::new();
        for id in due {
            if let Some(m) = self.loaded.get_mut(id) {
                m.tick(now);
            }
            out.extend(self.settle(store, mods, id, now));
        }
        out
    }

    /// When the first module wants `tick`.
    pub fn next_due(&self) -> Option<u64> {
        self.due.values().copied().min()
    }

    /// The test hook `test-<id>`: demo data in a module's place of the
    /// system's. `None` when there is no such hook.
    pub fn demo(
        &mut self,
        store: &mut Store,
        mods: &mut Mods,
        id: &str,
        args: &[String],
        now: u64,
    ) -> Option<(String, Vec<ToUi>)> {
        if id == "resources" {
            self.sampling = false;
        }
        if id == "media" {
            self.media_live = false;
        }
        let m = self.loaded.get_mut(id)?;
        let answer = m.demo(args, now)?;
        Some((answer, self.settle(store, mods, id, now)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct Spy(Arc<Mutex<Vec<(String, String)>>>);
    impl Notifier for Spy {
        fn notify(&self, title: &str, body: &str) {
            self.0.lock().unwrap().push((title.into(), body.into()));
        }
    }

    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    fn rig(cfg: Value) -> (CoreHost, Store, Mods, Seen) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut store = Store::new();
        store.set_config(cfg);
        (
            CoreHost::new(Some(Box::new(Spy(seen.clone())))),
            store,
            Mods::new(),
            seen,
        )
    }

    fn say(
        h: &mut CoreHost,
        s: &mut Store,
        m: &mut Mods,
        method: &str,
        args: &[&str],
        now: u64,
    ) -> String {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        h.command(s, m, method, &args, now).expect("an owner").0
    }

    #[test]
    fn a_module_loads_while_a_notch_lists_it_and_is_gone_with_its_state_when_not() {
        let (mut h, mut s, mut m, _) = rig(json!({"modules": ["timer"]}));
        let out = h.sync(&mut s, &mut m, 0);
        assert!(h.has("timer") && !h.has("activities"));
        assert!(
            out.iter()
                .any(|u| matches!(u, ToUi::Module(i) if i.id == "timer" && i.title == "Timer"))
        );
        assert_eq!(say(&mut h, &mut s, &mut m, "timer", &["5m"], 1000), "ok");
        assert_eq!(
            say(&mut h, &mut s, &mut m, "push", &["{}"], 0),
            "activities module not loaded"
        );
        s.set_config(json!({"modules": ["activities"]}));
        let out = h.sync(&mut s, &mut m, 2000);
        assert!(!h.has("timer") && h.has("activities"));
        assert!(
            out.iter()
                .any(|u| matches!(u, ToUi::View(v) if v.module == "timer" && v.tree.is_none()))
        );
        assert!(s.module("timer").is_none(), "its facts leave the store");
        s.set_config(json!({"modules": ["timer"]}));
        h.sync(&mut s, &mut m, 3000);
        assert_eq!(
            say(&mut h, &mut s, &mut m, "timer", &["pause"], 3000),
            "no timer running"
        );
    }

    #[test]
    fn a_standalone_module_loads_unless_switched_off() {
        let (mut h, mut s, mut m, _) = rig(json!({"modules": []}));
        h.sync(&mut s, &mut m, 0);
        assert!(!h.has("osd"), "no host declared it");
        s.set_ui_catalog(
            vec![json!({"moduleId": "osd", "title": "OSD", "settings": [], "standalone": true})],
            None,
        );
        h.sync(&mut s, &mut m, 0);
        assert!(h.has("osd"));
        s.set_config(json!({"modules": [], "osd": {"enabled": false}}));
        h.sync(&mut s, &mut m, 0);
        assert!(!h.has("osd"));
        assert_eq!(
            say(&mut h, &mut s, &mut m, "osd", &["ping"], 0),
            "osd module not loaded"
        );
    }

    #[test]
    fn a_views_change_goes_to_the_ui_once_and_the_peek_to_the_store() {
        let (mut h, mut s, mut m, _) = rig(json!({"modules": ["activities"]}));
        h.sync(&mut s, &mut m, 0);
        let json = r#"{"id":"a","title":"Deploy","sticky":true}"#;
        let args = [json.to_string()];
        let (answer, out) = h.command(&mut s, &mut m, "push", &args, 5).unwrap();
        assert_eq!(answer, "ok");
        assert!(out.iter().any(|u| matches!(u, ToUi::Module(i) if i.active)));
        let slots: Vec<Slot> = out
            .iter()
            .filter_map(|u| match u {
                ToUi::View(v) => Some(v.slot),
                _ => None,
            })
            .collect();
        for slot in [
            Slot::Compact,
            Slot::VerticalCompact,
            Slot::Peek,
            Slot::Expanded,
        ] {
            assert!(slots.contains(&slot), "{slot:?} in {slots:?}");
        }
        // The same push again changes nothing the UIs see.
        let (_, out) = h.command(&mut s, &mut m, "push", &args, 6).unwrap();
        assert!(out.is_empty(), "{out:?}");
        assert!(s.module("activities").unwrap().info.compact);
    }

    #[test]
    fn the_clock_drives_a_running_timer_and_its_end_notifies() {
        let (mut h, mut s, mut m, seen) = rig(json!({"modules": ["timer"]}));
        h.sync(&mut s, &mut m, 0);
        say(&mut h, &mut s, &mut m, "timer", &["1m Tea"], 10_000);
        let due = h.next_due().unwrap();
        assert!(due > 10_000 && due <= 11_100);
        let out = h.tick(&mut s, &mut m, due);
        let compact = out.iter().find_map(|u| match u {
            ToUi::View(v) if v.slot == Slot::Compact => v.tree.clone(),
            _ => None,
        });
        assert!(
            serde_json::to_string(&compact.unwrap())
                .unwrap()
                .contains("0:59")
        );
        h.tick(&mut s, &mut m, 70_010);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![("Time's up".to_string(), "Tea".to_string())]
        );
        assert_eq!(
            h.next_due(),
            Some(70_010 + 60_000),
            "a done timer goes after a minute"
        );
    }

    #[test]
    fn a_button_of_a_view_is_an_event_and_resources_take_a_providers_data() {
        let (mut h, mut s, mut m, _) = rig(json!({"modules": ["timer", "resources"]}));
        h.sync(&mut s, &mut m, 0);
        let out = h.event(
            &mut s,
            &mut m,
            "timer",
            "start",
            &EventValue::Text("5".into()),
            1000,
        );
        assert!(
            out.iter()
                .any(|u| matches!(u, ToUi::View(v) if v.slot == Slot::Compact && v.tree.is_some()))
        );
        let sample = json!({"cpu": 0.5, "cores": 4, "memory": {"used": 1.0, "total": 2.0, "fraction": 0.5}, "disk": null});
        let out = h.feed(&mut s, &mut m, "resources", &sample, 2000);
        assert!(out.iter().any(
            |u| matches!(u, ToUi::View(v) if v.module == "resources" && v.slot == Slot::Compact)
        ));
        assert!(
            h.demo(
                &mut s,
                &mut m,
                "resources",
                &["0.1".into(), "0.2".into(), "0.3".into()],
                3000
            )
            .is_some()
        );
        assert!(!h.wants_resources(), "demo values stop the readings");
    }

    #[test]
    fn media_asks_for_the_players_when_it_loads_and_its_buttons_reach_the_provider() {
        use std::sync::mpsc::channel;
        let (mut h, mut s, mut m, _) = rig(json!({"modules": ["media"]}));
        let (tx, rx) = channel();
        h.set_media(tx);
        assert!(!h.wants_media(), "not loaded yet");
        h.sync(&mut s, &mut m, 0);
        assert!(h.wants_media());
        assert_eq!(rx.try_recv(), Ok(MediaCommand::Refresh));
        let player = json!({"id": "p", "name": "P", "playing": true, "title": "Song", "artist": "A",
            "album": "", "artUrl": "", "length": 100, "position": 10, "canPlay": true, "canPause": true,
            "canNext": true, "canPrev": true, "canSeek": true});
        let out = h.feed(&mut s, &mut m, "media", &json!({"players": [player]}), 1000);
        assert!(
            out.iter()
                .any(|u| matches!(u, ToUi::Module(i) if i.id == "media" && i.active))
        );
        let null = EventValue::Null(());
        h.event(&mut s, &mut m, "media", "next", &null, 1100);
        let half = EventValue::Number(notchd_proto::Num(0.5));
        h.event(&mut s, &mut m, "media", "seek", &half, 1200);
        assert_eq!(rx.try_recv(), Ok(MediaCommand::Next { player: "p".into() }));
        assert_eq!(
            rx.try_recv(),
            Ok(MediaCommand::Seek {
                player: "p".into(),
                seconds: 50.0
            })
        );
        // Demo players stop the system's.
        let demo = json!({"players": []}).to_string();
        h.demo(&mut s, &mut m, "media", &[demo], 2000).unwrap();
        assert!(!h.wants_media());
        let out = h.feed(&mut s, &mut m, "media", &json!({"players": [player]}), 3000);
        assert!(out.is_empty());
    }

    #[test]
    fn the_context_says_whether_a_notch_is_open() {
        let (mut h, mut s, mut m, _) = rig(json!({"modules": ["timer"]}));
        h.sync(&mut s, &mut m, 0);
        assert!(!CoreHost::ctx(&s, "timer").expanded);
    }
}
