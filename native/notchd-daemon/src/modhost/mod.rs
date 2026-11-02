//! The process host for user modules (docs/architecture.md, "Protocol: core to
//! module"): discovers `<modules_dir>/<id>/module.json`, runs each module's
//! command as a process, turns its stdout into [`ModEvent`]s, keeps its stderr
//! in a log file, restarts it on a crash (with backoff) and on a save, and
//! stops it cleanly.
//!
//! One supervisor thread owns every module. Reader threads (stdout, stderr)
//! and a writer thread per process feed it through a channel, so there are no
//! locks around module state; the caller talks to it through [`ModHost`]'s
//! methods, which only enqueue. The sink is called on the supervisor thread:
//! keep it cheap (the core loop forwards into its own channel).
//!
//! Files: `<log_dir>/<id>.log` holds the module's stderr, its `log` messages
//! and the host's own notes (starts, exits), rotated at 1 MiB with five
//! rotated files kept (`<id>.log.1` newest).

mod discover;
mod logfile;
pub mod proc;
#[cfg(test)]
mod tests;

pub use discover::{fingerprint, load_manifest};
pub use logfile::LogFile;
pub use proc::{LineReader, ReadLine};

use notchd_proto::module::{
    CoreError, Init, LogLevel, ModuleConfig, ModuleSet, Notify, Peek, valid_module_id,
};
use notchd_proto::version::{MAX_MODULE_LINE_BYTES, MAX_VIEW_BYTES, MAX_VIEWS_PER_SECOND, PROTO};
use notchd_proto::{FromModule, Manifest, Node, Slot, ToModule, Token, Validate, to_line};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Everything tunable. [`Options::default`] is the doc's numbers; tests
/// shorten the times.
#[derive(Clone, Debug)]
pub struct Options {
    /// How often the folders are scanned for a save or a new module.
    pub poll_interval: Duration,
    /// A change must hold still this long before the module restarts.
    pub debounce: Duration,
    /// After `stop`: SIGTERM this long after it was sent...
    pub term_after: Duration,
    /// ...and SIGKILL this long after it was sent.
    pub kill_after: Duration,
    /// Restart delay after the first crash; doubles each time.
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    /// A process that stays up this long is healthy: the backoff starts over.
    pub reset_after: Duration,
    /// This many crashes in a row (each within `reset_after`) and the host
    /// stops restarting until a file changes or `restart` is called.
    pub rapid_crash_limit: u32,
    /// No view this long after the start shows a "no view" error row.
    pub first_view_timeout: Duration,
    pub log_max_bytes: u64,
    pub log_keep: usize,
    pub max_views_per_second: u32,
    /// Bad lines in a row (not JSON, overlong, malformed) that count as a crash.
    pub bad_lines_limit: u32,
    /// `${config}` expands to `<config_root>/<id>`, `${cache}` to `<cache_root>/<id>`.
    pub config_root: PathBuf,
    pub cache_root: PathBuf,
    /// Folder names that are config keys or built-in ids and so cannot be modules.
    pub reserved: Vec<String>,
}

impl Options {
    pub fn for_dir(modules_dir: &Path) -> Options {
        let parent = modules_dir.parent().unwrap_or(modules_dir);
        let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect();
        Options {
            poll_interval: Duration::from_millis(300),
            debounce: Duration::from_millis(150),
            term_after: Duration::from_secs(1),
            kill_after: Duration::from_secs(3),
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            reset_after: Duration::from_secs(60),
            rapid_crash_limit: 5,
            first_view_timeout: Duration::from_secs(5),
            log_max_bytes: 1024 * 1024,
            log_keep: 5,
            max_views_per_second: MAX_VIEWS_PER_SECOND,
            bad_lines_limit: 10,
            config_root: parent.join("module-config"),
            cache_root: parent.join("module-cache"),
            reserved: strings(&[
                "modules",
                "reserveSpace",
                "draggable",
                "tabs",
                "idleIcons",
                "workspaces",
                "position",
                "theme",
                "activities",
                "claude",
                "codex",
                "codexLive",
                "localLive",
                "media",
                "osd",
                "resources",
                "terminal",
                "timer",
            ]),
        }
    }
}

/// What went wrong, for a UI that wants to style the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// `module.json` is missing or invalid.
    Manifest,
    /// The process exited, was killed, or could not start.
    Crash,
    /// The first view did not come in time.
    NoView,
    /// The module sent a view that fails validation.
    InvalidView,
    /// The module sent `error`.
    Module,
    /// The folder cannot be a module (name not an id, or reserved).
    Rejected,
}

/// What the host tells the core. Every event names its module.
#[derive(Clone, Debug, PartialEq)]
pub enum ModEvent {
    /// A good manifest was read (also on every reload): the settings page and
    /// the notch properties come from it.
    Loaded {
        module: String,
        manifest: Box<Manifest>,
    },
    /// The folder is gone; the module has been stopped.
    Removed {
        module: String,
    },
    /// A process started.
    Started {
        module: String,
        pid: u32,
    },
    /// A valid `view`. `tree: None` clears the slot.
    View {
        module: String,
        slot: Slot,
        tree: Option<Node>,
    },
    Set {
        module: String,
        set: ModuleSet,
    },
    Peek {
        module: String,
        peek: Peek,
    },
    Notify {
        module: String,
        notify: Notify,
    },
    Open {
        module: String,
    },
    Subscribe {
        module: String,
        name: String,
    },
    Unsubscribe {
        module: String,
        name: String,
    },
    Log {
        module: String,
        level: LogLevel,
        text: String,
    },
    /// Show an error row for the module (see [`error_row`]); `log_path` is
    /// where its log is.
    Error {
        module: String,
        kind: ErrorKind,
        text: String,
        log_path: PathBuf,
    },
    /// The module is healthy again (a valid view arrived, or the manifest was fixed).
    ErrorCleared {
        module: String,
    },
}

pub type Sink = Box<dyn Fn(ModEvent) + Send + 'static>;

/// The doc's error row: a danger icon, the text and the log path, and a
/// Restart button (`event` action `restart`).
pub fn error_row(text: &str, log_path: &Path) -> Node {
    let row = serde_json::json!({
        "t": "row", "gap": 6, "align": "center", "children": [
            {"t": "icon", "glyph": "\u{f071}", "tone": "danger"},
            {"t": "text", "text": format!("{text}  {}", log_path.display()),
             "tone": "danger", "elide": "end", "fill": true},
            {"t": "button", "label": "Restart", "action": "restart", "variant": "outline"}
        ]
    });
    let node: Node = serde_json::from_value(row).expect("the error row is a valid node");
    debug_assert!(node.validate().is_ok());
    node
}

/// Restart delay after the `crashes`-th crash in a row: base, 2x, 4x... capped.
pub fn backoff_delay(opts: &Options, crashes: u32) -> Duration {
    let shift = crashes.saturating_sub(1).min(20);
    opts.backoff_base
        .saturating_mul(1u32 << shift)
        .min(opts.backoff_max)
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Running,
    /// `stop` was sent; waiting for the process to go.
    Stopping,
    /// Crashed; restarts in this long.
    Backoff(Duration),
    /// Too many rapid crashes; waits for a save or `restart`.
    Parked,
    /// No good manifest, so nothing to run.
    Invalid(String),
    Disabled,
    Idle,
}

#[derive(Clone, Debug)]
pub struct ModuleStatus {
    pub id: String,
    pub state: State,
    pub pid: Option<u32>,
    pub manifest: Option<Manifest>,
    pub log_path: PathBuf,
    /// Crashes in a row since the last healthy run.
    pub crashes: u32,
}

enum Msg {
    Line {
        id: String,
        epoch: u64,
        line: String,
    },
    Bad {
        id: String,
        epoch: u64,
        reason: String,
    },
    Eof {
        id: String,
        epoch: u64,
    },
    Send(String, Box<ToModule>),
    Config(String, Value),
    Restart(String),
    Enable(String, bool),
    Rescan(Sender<()>),
    Status(String, Sender<Option<ModuleStatus>>),
    Ids(Sender<Vec<String>>),
    Shutdown(Sender<()>),
}

/// The handle the core loop holds. Dropping it stops every module.
pub struct ModHost {
    tx: Sender<Msg>,
    thread: Mutex<Option<JoinHandle<()>>>,
    wait: Duration,
}

impl ModHost {
    pub fn new(
        modules_dir: impl Into<PathBuf>,
        log_dir: impl Into<PathBuf>,
        sink: impl Fn(ModEvent) + Send + 'static,
    ) -> ModHost {
        let modules_dir = modules_dir.into();
        let opts = Options::for_dir(&modules_dir);
        ModHost::with_options(modules_dir, log_dir, opts, sink)
    }

    pub fn with_options(
        modules_dir: impl Into<PathBuf>,
        log_dir: impl Into<PathBuf>,
        opts: Options,
        sink: impl Fn(ModEvent) + Send + 'static,
    ) -> ModHost {
        let (tx, rx) = mpsc::channel();
        let wait = opts.kill_after + Duration::from_secs(5);
        let sup = Supervisor {
            ctx: Ctx {
                opts,
                sink: Box::new(sink),
                tx: tx.clone(),
                log_dir: log_dir.into(),
                next_epoch: 1,
            },
            modules_dir: modules_dir.into(),
            mods: BTreeMap::new(),
            rejected: BTreeSet::new(),
            last_scan: None,
            shutdown: None,
        };
        let thread = thread::Builder::new()
            .name("notchd-modhost".into())
            .spawn(move || sup.run(rx))
            .expect("spawn the module host thread");
        ModHost {
            tx,
            thread: Mutex::new(Some(thread)),
            wait,
        }
    }

    /// Queues a message to the module's stdin. Dropped when it is not running.
    pub fn send(&self, module: &str, msg: ToModule) {
        let _ = self.tx.send(Msg::Send(module.into(), Box::new(msg)));
    }

    /// Sets the module's config (its settings' values). Sent as `config` to a
    /// running module, and as `init.config` to the next start.
    pub fn set_config(&self, module: &str, config: Value) {
        let _ = self.tx.send(Msg::Config(module.into(), config));
    }

    /// The Restart button: restart now, forgetting the crash count.
    pub fn restart(&self, module: &str) {
        let _ = self.tx.send(Msg::Restart(module.into()));
    }

    /// A module that is not listed by any notch, or is disabled in the config,
    /// is stopped; enabling it starts it.
    pub fn set_enabled(&self, module: &str, enabled: bool) {
        let _ = self.tx.send(Msg::Enable(module.into(), enabled));
    }

    /// Scans the folders now and waits for it, instead of at the next poll.
    pub fn rescan(&self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Msg::Rescan(tx)).is_ok() {
            let _ = rx.recv_timeout(self.wait);
        }
    }

    pub fn status(&self, module: &str) -> Option<ModuleStatus> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(Msg::Status(module.into(), tx)).ok()?;
        rx.recv_timeout(self.wait).ok().flatten()
    }

    /// The ids of the modules found so far.
    pub fn modules(&self) -> Vec<String> {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Msg::Ids(tx)).is_err() {
            return Vec::new();
        }
        rx.recv_timeout(self.wait).unwrap_or_default()
    }

    /// `stop` to every module, SIGTERM and SIGKILL after the grace periods,
    /// and returns when no process (nor anything it started) is left. The host
    /// is unusable afterwards. Safe to call twice.
    pub fn stop_all(&self) {
        let thread = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(thread) = thread else { return };
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Msg::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(self.wait);
        }
        let _ = thread.join();
    }
}

impl Drop for ModHost {
    fn drop(&mut self) {
        self.stop_all();
    }
}

// ---------------------------------------------------------------- supervisor

struct Stopping {
    since: Instant,
    termed: bool,
    killed: bool,
}

struct Proc {
    epoch: u64,
    child: Child,
    pgid: u32,
    stdin: Option<Sender<String>>,
    started: Instant,
    got_view: bool,
    no_view_reported: bool,
    stopping: Option<Stopping>,
    /// Set when the host itself killed the process for misbehaving.
    fail: Option<String>,
    exit: Option<(ExitStatus, Instant)>,
    eof: bool,
}

enum Phase {
    Idle,
    Backoff(Instant),
    Parked,
}

struct Module {
    id: String,
    dir: PathBuf,
    log: std::sync::Arc<LogFile>,
    manifest: Option<Manifest>,
    manifest_error: Option<String>,
    /// The fingerprint the running (or last started) version was read at.
    fp: u64,
    /// A newer fingerprint waiting out the debounce.
    pending: Option<(u64, Instant)>,
    config: Value,
    enabled: bool,
    removed: bool,
    proc: Option<Proc>,
    phase: Phase,
    crashes: u32,
    restart_pending: bool,
    error_shown: bool,
    bad_streak: u32,
    view_window: Instant,
    view_count: u32,
    view_warned: bool,
}

/// Everything but the module map, so a module can be borrowed mutably while
/// the sink, channel and options are used.
struct Ctx {
    opts: Options,
    sink: Sink,
    tx: Sender<Msg>,
    log_dir: PathBuf,
    next_epoch: u64,
}

struct Supervisor {
    ctx: Ctx,
    modules_dir: PathBuf,
    mods: BTreeMap<String, Module>,
    rejected: BTreeSet<String>,
    last_scan: Option<Instant>,
    shutdown: Option<Sender<()>>,
}

const TICK: Duration = Duration::from_millis(20);
const EOF_GRACE: Duration = Duration::from_secs(1);

impl Supervisor {
    fn run(mut self, rx: Receiver<Msg>) {
        loop {
            let mut batch = Vec::new();
            match rx.recv_timeout(TICK) {
                Ok(msg) => {
                    batch.push(msg);
                    while let Ok(m) = rx.try_recv() {
                        batch.push(m);
                        if batch.len() >= 512 {
                            break;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => self.begin_shutdown(None),
            }
            for msg in batch {
                self.handle(msg);
            }
            if self.shutdown.is_none() {
                let due = self
                    .last_scan
                    .is_none_or(|t| t.elapsed() >= self.ctx.opts.poll_interval);
                if due {
                    self.scan();
                }
            }
            self.tick();
            if self.shutdown.is_some() && self.mods.values().all(|m| m.proc.is_none()) {
                if let Some(ack) = self.shutdown.take() {
                    let _ = ack.send(());
                }
                return;
            }
        }
    }

    fn handle(&mut self, msg: Msg) {
        let ctx = &mut self.ctx;
        match msg {
            Msg::Line { id, epoch, line } => {
                if let Some(m) = self.mods.get_mut(&id).filter(|m| is_gen(m, epoch)) {
                    ctx.on_line(m, &line);
                }
            }
            Msg::Bad { id, epoch, reason } => {
                if let Some(m) = self.mods.get_mut(&id).filter(|m| is_gen(m, epoch)) {
                    ctx.bad_line(m, &reason);
                }
            }
            Msg::Eof { id, epoch } => {
                if let Some(p) = self
                    .mods
                    .get_mut(&id)
                    .and_then(|m| m.proc.as_mut())
                    .filter(|p| p.epoch == epoch)
                {
                    p.eof = true;
                }
            }
            Msg::Send(id, msg) => {
                if let Some(p) = self.mods.get(&id).and_then(|m| m.proc.as_ref())
                    && p.stopping.is_none()
                    && let Some(stdin) = &p.stdin
                {
                    let _ = stdin.send(to_line(&*msg));
                }
            }
            Msg::Config(id, config) => {
                if let Some(m) = self.mods.get_mut(&id) {
                    m.config = config.clone();
                    if let Some(p) = m.proc.as_ref().filter(|p| p.stopping.is_none())
                        && let Some(stdin) = &p.stdin
                    {
                        let _ = stdin.send(to_line(&ToModule::Config(ModuleConfig { config })));
                    }
                }
            }
            Msg::Restart(id) => {
                if let Some(m) = self.mods.get_mut(&id) {
                    ctx.reload(m);
                }
            }
            Msg::Enable(id, enabled) => {
                if let Some(m) = self.mods.get_mut(&id) {
                    ctx.set_enabled(m, enabled);
                }
            }
            Msg::Rescan(ack) => {
                if self.shutdown.is_none() {
                    self.scan();
                }
                let _ = ack.send(());
            }
            Msg::Status(id, reply) => {
                let _ = reply.send(self.mods.get(&id).map(status_of));
            }
            Msg::Ids(reply) => {
                let _ = reply.send(self.mods.keys().cloned().collect());
            }
            Msg::Shutdown(ack) => self.begin_shutdown(Some(ack)),
        }
    }

    fn begin_shutdown(&mut self, ack: Option<Sender<()>>) {
        if self.shutdown.is_some() {
            return;
        }
        // Closed channel (no ack): a dummy sender whose receiver is gone.
        self.shutdown = Some(ack.unwrap_or_else(|| mpsc::channel().0));
        for m in self.mods.values_mut() {
            m.enabled = false;
            m.restart_pending = false;
            self.ctx.begin_stop(m);
        }
    }

    fn tick(&mut self) {
        let mut gone = Vec::new();
        for (id, m) in self.mods.iter_mut() {
            self.ctx.tick_module(m);
            if m.removed && m.proc.is_none() {
                gone.push(id.clone());
            }
        }
        for id in gone {
            self.mods.remove(&id);
        }
    }

    /// Looks at the modules folder: new folders become modules, deleted ones
    /// are stopped, changed ones (after the debounce) are reloaded.
    fn scan(&mut self) {
        self.last_scan = Some(Instant::now());
        let mut present: BTreeSet<String> = BTreeSet::new();
        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.modules_dir) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || !entry.path().is_dir() {
                    continue;
                }
                dirs.push((name, entry.path()));
            }
        }
        dirs.sort();
        for (name, path) in dirs {
            let bad = if !valid_module_id(&name) {
                Some(format!(
                    "{name:?} is not a module id (letters, digits, - and _)"
                ))
            } else if self.ctx.opts.reserved.contains(&name) {
                Some(format!("{name:?} is a config key or a built-in module id"))
            } else {
                None
            };
            if let Some(text) = bad {
                present.insert(name.clone());
                if self.rejected.insert(name.clone()) {
                    let log_path = self.ctx.log_dir.join(format!("{name}.log"));
                    (self.ctx.sink)(ModEvent::Error {
                        module: name,
                        kind: ErrorKind::Rejected,
                        text,
                        log_path,
                    });
                }
                continue;
            }
            present.insert(name.clone());
            let fp = discover::fingerprint(&path);
            match self.mods.get_mut(&name) {
                None => {
                    let mut m = self.ctx.new_module(&name, path, fp);
                    self.ctx.load(&mut m, true);
                    self.mods.insert(name, m);
                }
                Some(m) => self.ctx.check_change(m, fp),
            }
        }
        self.rejected.retain(|n| present.contains(n));
        let missing: Vec<String> = self
            .mods
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for id in missing {
            if let Some(m) = self.mods.get_mut(&id)
                && !m.removed
            {
                m.removed = true;
                m.restart_pending = false;
                self.ctx.begin_stop(m);
                (self.ctx.sink)(ModEvent::Removed { module: id });
            }
        }
    }
}

fn is_gen(m: &Module, epoch: u64) -> bool {
    m.proc.as_ref().is_some_and(|p| p.epoch == epoch)
}

fn status_of(m: &Module) -> ModuleStatus {
    let state = if let Some(p) = &m.proc {
        if p.stopping.is_some() {
            State::Stopping
        } else {
            State::Running
        }
    } else if !m.enabled {
        State::Disabled
    } else if let (None, Some(e)) = (&m.manifest, &m.manifest_error) {
        State::Invalid(e.clone())
    } else {
        match m.phase {
            Phase::Backoff(at) => State::Backoff(at.saturating_duration_since(Instant::now())),
            Phase::Parked => State::Parked,
            Phase::Idle => State::Idle,
        }
    };
    ModuleStatus {
        id: m.id.clone(),
        state,
        pid: m.proc.as_ref().map(|p| p.child.id()),
        manifest: m.manifest.clone(),
        log_path: m.log.path().to_path_buf(),
        crashes: m.crashes,
    }
}

impl Ctx {
    fn emit(&self, ev: ModEvent) {
        (self.sink)(ev);
    }

    fn new_module(&self, id: &str, dir: PathBuf, fp: u64) -> Module {
        Module {
            id: id.into(),
            dir,
            log: std::sync::Arc::new(LogFile::new(
                &self.log_dir,
                id,
                self.opts.log_max_bytes,
                self.opts.log_keep,
            )),
            manifest: None,
            manifest_error: None,
            fp,
            pending: None,
            config: Value::Object(Default::default()),
            enabled: true,
            removed: false,
            proc: None,
            phase: Phase::Idle,
            crashes: 0,
            restart_pending: false,
            error_shown: false,
            bad_streak: 0,
            view_window: Instant::now(),
            view_count: 0,
            view_warned: false,
        }
    }

    fn error(&self, m: &mut Module, kind: ErrorKind, text: String) {
        m.error_shown = true;
        self.emit(ModEvent::Error {
            module: m.id.clone(),
            kind,
            text,
            log_path: m.log.path().to_path_buf(),
        });
    }

    fn clear_error(&self, m: &mut Module) {
        if m.error_shown {
            m.error_shown = false;
            self.emit(ModEvent::ErrorCleared {
                module: m.id.clone(),
            });
        }
    }

    /// Reads the manifest. A good one replaces the old; a bad one is an error
    /// row, and a process already running is left alone. Returns whether the
    /// manifest is good.
    fn load(&mut self, m: &mut Module, first: bool) -> bool {
        match discover::load_manifest(&m.dir) {
            Ok(manifest) => {
                m.manifest = Some(manifest.clone());
                m.manifest_error = None;
                self.clear_error(m);
                self.emit(ModEvent::Loaded {
                    module: m.id.clone(),
                    manifest: Box::new(manifest),
                });
                if first {
                    self.start_now(m);
                }
                true
            }
            Err(text) => {
                m.manifest_error = Some(text.clone());
                m.log.note(&format!("manifest: {text}"));
                let text = format!("{}: {text}", m.id);
                self.error(m, ErrorKind::Manifest, text);
                false
            }
        }
    }

    /// A save is noticed: wait for the folder to hold still, then reload.
    fn check_change(&mut self, m: &mut Module, fp: u64) {
        if fp == m.fp {
            m.pending = None;
            return;
        }
        match m.pending {
            Some((p, since)) if p == fp => {
                if since.elapsed() >= self.opts.debounce {
                    m.pending = None;
                    m.fp = fp;
                    m.log.note("files changed, reloading");
                    if self.load(m, false) {
                        self.reload(m);
                    }
                }
            }
            _ => m.pending = Some((fp, Instant::now())),
        }
    }

    /// Restart at once with a clean slate: crash count and parking forgotten.
    fn reload(&mut self, m: &mut Module) {
        m.crashes = 0;
        m.phase = Phase::Idle;
        if m.proc.is_some() {
            m.restart_pending = true;
            self.begin_stop(m);
        } else {
            self.start_now(m);
        }
    }

    fn set_enabled(&mut self, m: &mut Module, enabled: bool) {
        m.enabled = enabled;
        if enabled {
            if m.proc.is_none() && !m.removed {
                self.start_now(m);
            }
        } else {
            m.restart_pending = false;
            m.phase = Phase::Idle;
            self.begin_stop(m);
        }
    }

    fn start_now(&mut self, m: &mut Module) {
        if m.enabled && !m.removed && m.manifest.is_some() && m.proc.is_none() {
            self.spawn(m);
        }
    }

    /// `stop`, and the clock for SIGTERM and SIGKILL starts.
    fn begin_stop(&mut self, m: &mut Module) {
        let Some(p) = m.proc.as_mut() else { return };
        if p.stopping.is_some() {
            return;
        }
        if let Some(stdin) = p.stdin.take() {
            let _ = stdin.send(to_line(&ToModule::Stop {}));
            // Dropping the sender closes the module's stdin once `stop` is
            // written: end-of-input is a stop signal too.
        }
        p.stopping = Some(Stopping {
            since: Instant::now(),
            termed: false,
            killed: false,
        });
    }

    fn tick_module(&mut self, m: &mut Module) {
        if let Phase::Backoff(at) = m.phase
            && m.proc.is_none()
            && Instant::now() >= at
        {
            m.phase = Phase::Idle;
            self.start_now(m);
        }
        let timeout = self.opts.first_view_timeout;
        let (term_after, kill_after) = (self.opts.term_after, self.opts.kill_after);
        let mut no_view = false;
        let mut finished = false;
        if let Some(p) = m.proc.as_mut() {
            if p.exit.is_none()
                && let Ok(Some(status)) = p.child.try_wait()
            {
                // Whatever the module started must not outlive it.
                proc::signal_group(p.pgid, proc::SIGKILL);
                p.exit = Some((status, Instant::now()));
            }
            if let Some((_, at)) = p.exit {
                finished = p.eof || at.elapsed() >= EOF_GRACE;
            } else if let Some(st) = p.stopping.as_mut() {
                let waited = st.since.elapsed();
                if waited >= kill_after && !st.killed {
                    st.killed = true;
                    proc::signal_group(p.pgid, proc::SIGKILL);
                } else if waited >= term_after && !st.termed {
                    st.termed = true;
                    proc::signal_group(p.pgid, proc::SIGTERM);
                }
            } else if !p.got_view && !p.no_view_reported && p.started.elapsed() >= timeout {
                p.no_view_reported = true;
                no_view = true;
            }
        }
        if no_view {
            let text = format!(
                "{}: no view after {} s, log: {}",
                m.id,
                timeout.as_secs_f64().round(),
                m.log.path().display()
            );
            m.error_shown = true;
            self.emit(ModEvent::Error {
                module: m.id.clone(),
                kind: ErrorKind::NoView,
                text,
                log_path: m.log.path().to_path_buf(),
            });
        }
        if finished {
            self.finish(m);
        }
    }

    /// The process is gone and its output drained.
    fn finish(&mut self, m: &mut Module) {
        let Some(p) = m.proc.take() else { return };
        let Some((status, _)) = p.exit else { return };
        let uptime = p.started.elapsed();
        let asked = p.stopping.is_some() && p.fail.is_none();
        if asked || !m.enabled || m.removed {
            m.log.note(&format!("stopped ({})", describe(&status)));
            if m.restart_pending {
                m.restart_pending = false;
                self.start_now(m);
            }
            return;
        }
        let text = match p.fail {
            Some(reason) => format!("{}: {reason}", m.id),
            None => format!("{} {}", m.id, describe(&status)),
        };
        self.crash(m, text, uptime);
    }

    fn crash(&mut self, m: &mut Module, text: String, uptime: Duration) {
        if uptime >= self.opts.reset_after {
            m.crashes = 0;
        }
        m.crashes += 1;
        m.log.note(&text);
        let mut shown = text;
        if m.crashes >= self.opts.rapid_crash_limit {
            m.phase = Phase::Parked;
            shown = format!(
                "{shown}; stopped restarting after {} crashes, save a file to retry",
                m.crashes
            );
        } else {
            m.phase = Phase::Backoff(Instant::now() + backoff_delay(&self.opts, m.crashes));
        }
        self.error(m, ErrorKind::Crash, shown);
    }

    fn spawn(&mut self, m: &mut Module) {
        let Some(manifest) = m.manifest.clone() else {
            return;
        };
        let config_dir = self.opts.config_root.join(&m.id);
        let cache_dir = self.opts.cache_root.join(&m.id);
        let _ = std::fs::create_dir_all(&config_dir);
        let _ = std::fs::create_dir_all(&cache_dir);
        let expand = |s: &str| {
            s.replace("${config}", &config_dir.to_string_lossy())
                .replace("${cache}", &cache_dir.to_string_lossy())
        };
        let argv: Vec<String> = manifest.command.iter().map(|a| expand(a)).collect();
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(&m.dir)
            .envs(manifest.env.iter().map(|(k, v)| (k, expand(v))))
            .env("NOTCHD_MODULE_ID", &m.id)
            .env("NOTCHD_MODULE_DIR", &m.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group: signals reach what the module starts too.
            .process_group(0);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let text = format!("{} failed to start {:?}: {e}", m.id, argv[0]);
                self.crash(m, text, Duration::ZERO);
                return;
            }
        };
        let pgid = child.id();
        let epoch = self.next_epoch;
        self.next_epoch += 1;

        // stdout -> the supervisor
        if let Some(out) = child.stdout.take() {
            let (tx, id) = (self.tx.clone(), m.id.clone());
            thread::spawn(move || read_stdout(out, tx, id, epoch));
        }
        // stderr -> the log file
        if let Some(err) = child.stderr.take() {
            let log = m.log.clone();
            thread::spawn(move || {
                let mut lines = LineReader::new(BufReader::new(err), 64 * 1024);
                while let Ok(l) = lines.next_line() {
                    match l {
                        ReadLine::Line(b) => log.line(&String::from_utf8_lossy(&b)),
                        ReadLine::Overlong(n) => {
                            log.line(&format!("[stderr line of {n} bytes dropped]"))
                        }
                        ReadLine::Eof => break,
                    }
                }
            });
        }
        // supervisor -> stdin, on its own thread so a module that does not
        // read cannot stall the host
        let (stdin_tx, stdin_rx) = mpsc::channel::<String>();
        if let Some(mut stdin) = child.stdin.take() {
            thread::spawn(move || {
                for line in stdin_rx {
                    if writeln!(stdin, "{line}")
                        .and_then(|_| stdin.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
        let init = ToModule::Init(Init {
            proto: PROTO,
            id: m.id.clone(),
            config: m.config.clone(),
            tokens: Token::ALL
                .iter()
                .filter_map(|t| serde_json::to_value(t).ok())
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            slots: Slot::ALL.to_vec(),
            dir: m.dir.to_string_lossy().into_owned(),
        });
        let _ = stdin_tx.send(to_line(&init));

        m.log.note(&format!("started pid {pgid}"));
        m.bad_streak = 0;
        m.proc = Some(Proc {
            epoch,
            child,
            pgid,
            stdin: Some(stdin_tx),
            started: Instant::now(),
            got_view: false,
            no_view_reported: false,
            stopping: None,
            fail: None,
            exit: None,
            eof: false,
        });
        self.emit(ModEvent::Started {
            module: m.id.clone(),
            pid: pgid,
        });
    }

    fn bad_line(&mut self, m: &mut Module, reason: &str) {
        m.log.note(&format!("bad line from the module: {reason}"));
        self.emit(ModEvent::Log {
            module: m.id.clone(),
            level: LogLevel::Warn,
            text: format!("bad line: {reason}"),
        });
        m.bad_streak += 1;
        if m.bad_streak >= self.opts.bad_lines_limit
            && let Some(p) = m.proc.as_mut().filter(|p| p.fail.is_none())
        {
            p.fail = Some(format!("{} bad lines in a row on stdout", m.bad_streak));
            p.stopping.get_or_insert(Stopping {
                since: Instant::now(),
                termed: true,
                killed: true,
            });
            proc::signal_group(p.pgid, proc::SIGKILL);
        }
    }

    fn on_line(&mut self, m: &mut Module, line: &str) {
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return self.bad_line(m, &format!("not JSON ({e}): {}", clip(line))),
        };
        let tag = value.get("t").and_then(Value::as_str).map(String::from);
        match tag {
            None => return self.bad_line(m, &format!("no \"t\": {}", clip(line))),
            // Unknown `t` is ignored, in both directions.
            Some(t) if !FromModule::TAGS.contains(&t.as_str()) => {
                m.bad_streak = 0;
                return;
            }
            Some(_) => {}
        }
        let msg: FromModule = match serde_json::from_value(value) {
            Ok(msg) => msg,
            Err(e) => return self.bad_line(m, &format!("{e}: {}", clip(line))),
        };
        m.bad_streak = 0;
        let module = m.id.clone();
        if let Err(why) = msg.validate() {
            return self.invalid(m, &msg, &why);
        }
        match msg {
            FromModule::View(v) => {
                if line.len() > MAX_VIEW_BYTES {
                    let why = format!("view over {MAX_VIEW_BYTES} bytes");
                    return self.invalid(m, &FromModule::View(v), &why);
                }
                if m.view_window.elapsed() >= Duration::from_secs(1) {
                    m.view_window = Instant::now();
                    m.view_count = 0;
                    m.view_warned = false;
                }
                m.view_count += 1;
                if m.view_count > self.opts.max_views_per_second {
                    if !m.view_warned {
                        m.view_warned = true;
                        let text = format!(
                            "more than {} views per second: dropping the excess",
                            self.opts.max_views_per_second
                        );
                        m.log.note(&text);
                        self.emit(ModEvent::Log {
                            module,
                            level: LogLevel::Warn,
                            text,
                        });
                    }
                    return;
                }
                if let Some(p) = m.proc.as_mut() {
                    p.got_view = true;
                }
                self.clear_error(m);
                self.emit(ModEvent::View {
                    module,
                    slot: v.slot,
                    tree: v.tree,
                });
            }
            FromModule::Set(set) => self.emit(ModEvent::Set { module, set }),
            FromModule::Peek(peek) => self.emit(ModEvent::Peek { module, peek }),
            FromModule::Notify(notify) => self.emit(ModEvent::Notify { module, notify }),
            FromModule::Open {} => self.emit(ModEvent::Open { module }),
            FromModule::Subscribe(p) => self.emit(ModEvent::Subscribe {
                module,
                name: p.name,
            }),
            FromModule::Unsubscribe(p) => self.emit(ModEvent::Unsubscribe {
                module,
                name: p.name,
            }),
            FromModule::Log(l) => {
                m.log.line(&format!("[{}] {}", level_name(l.level), l.text));
                self.emit(ModEvent::Log {
                    module,
                    level: l.level,
                    text: l.text,
                });
            }
            FromModule::Error(e) => {
                m.log.note(&format!("module error: {}", e.message));
                let text = format!("{}: {}", m.id, e.message);
                self.error(m, ErrorKind::Module, text);
            }
        }
    }

    /// A message that parsed but breaks a rule. A view becomes an error row; a
    /// subscribe to an unknown provider is answered with `error`; the module
    /// keeps running either way.
    fn invalid(&mut self, m: &mut Module, msg: &FromModule, why: &str) {
        m.log.note(&format!("invalid message: {why}"));
        match msg {
            FromModule::View(v) => {
                let slot = serde_json::to_value(v.slot)
                    .ok()
                    .and_then(|s| s.as_str().map(String::from))
                    .unwrap_or_default();
                let text = format!("{}: invalid view ({slot}): {why}", m.id);
                self.error(m, ErrorKind::InvalidView, text);
            }
            FromModule::Subscribe(_) | FromModule::Unsubscribe(_) => {
                if let Some(stdin) = m.proc.as_ref().and_then(|p| p.stdin.as_ref()) {
                    let _ = stdin.send(to_line(&ToModule::Error(CoreError {
                        code: "provider".into(),
                        message: Some(why.into()),
                    })));
                }
            }
            _ => {}
        }
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

fn clip(line: &str) -> String {
    let mut s: String = line.chars().take(200).collect();
    if s.len() < line.len() {
        s.push_str("...");
    }
    s
}

fn describe(status: &ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exited ({code})"),
        (None, Some(sig)) => format!("killed by signal {sig}"),
        _ => "exited".into(),
    }
}

fn read_stdout(out: std::process::ChildStdout, tx: Sender<Msg>, id: String, epoch: u64) {
    let mut lines = LineReader::new(BufReader::new(out), MAX_MODULE_LINE_BYTES);
    loop {
        let msg = match lines.next_line() {
            Ok(ReadLine::Line(bytes)) => match String::from_utf8(bytes) {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => Msg::Line {
                    id: id.clone(),
                    epoch,
                    line,
                },
                Err(_) => Msg::Bad {
                    id: id.clone(),
                    epoch,
                    reason: "not UTF-8".into(),
                },
            },
            Ok(ReadLine::Overlong(n)) => Msg::Bad {
                id: id.clone(),
                epoch,
                reason: format!("a line of {n} bytes is over the {MAX_MODULE_LINE_BYTES} limit"),
            },
            Ok(ReadLine::Eof) | Err(_) => {
                let _ = tx.send(Msg::Eof { id, epoch });
                return;
            }
        };
        if tx.send(msg).is_err() {
            return;
        }
    }
}
