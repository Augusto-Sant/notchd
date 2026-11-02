//! The hub: the one shared [`Store`] of a daemon instance, its clients, its
//! clock and the config file.
//!
//! Concurrency: a single `Mutex` around the store and the client list. Every
//! change (a message from a connection thread, a timer, an edit of the file)
//! happens under it and ends with one publish, so the `rev`s are issued in
//! one order and every UI is sent the same messages in that order. The state
//! is a few KB and a change is microseconds, so one lock is simpler and
//! cheaper than a channel into an actor; what must not block it, a socket
//! write, never runs under it: each connection has its own channel and writer
//! thread, and the hub only pushes onto the channel.
//!
//! User modules (`modhost`) run as processes under a [`ModHost`]; its events
//! reach the hub on a thread of their own (the sink only forwards into a
//! channel), and apply under the same lock as any other change.
//!
//! The modules that live in the daemon (`corehost`: timer, activities,
//! resources, osd) run under the same lock: the ticker wakes them at their
//! time, an IPC method or an `event` reaches them from a connection thread,
//! and the system's readings arrive from a sampler thread
//! ([`Hub::start_providers`]).
//!
//! One ticker thread sleeps until the store's next timer (or the next poll of
//! the config file, 100 ms) and wakes early when a change moves it.

use crate::corehost::CoreHost;
use crate::modhost::{ModEvent, ModHost};
use crate::modstate::Mods;
use crate::ptyhost::{Attached, Note, Session};
use crate::router;
use notchd_core::store::{Changes, Store};
use notchd_proto::module::ModuleEvent;
use notchd_proto::ui::Welcome;
use notchd_proto::version::PROTO;
use notchd_proto::{ToDaemon, ToModule, ToUi, Token};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often the config file is looked at for an edit from outside.
pub const CONFIG_POLL: Duration = Duration::from_millis(100);
/// How often the system's resources are read.
const RESOURCES_EVERY: Duration = Duration::from_secs(2);

/// Milliseconds since the epoch, advancing monotonically: the wall clock read
/// once, then an `Instant`, so a clock step never moves a timer.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    base_ms: u64,
    start: Instant,
}

impl Clock {
    pub fn new() -> Clock {
        let base_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        Clock {
            base_ms,
            start: Instant::now(),
        }
    }

    pub fn now(&self) -> u64 {
        self.base_ms + self.start.elapsed().as_millis() as u64
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::new()
    }
}

struct Client {
    id: u64,
    tx: Sender<ToUi>,
    /// Said `hello`: gets the broadcasts. A CLI connection gets replies only.
    subscribed: bool,
    /// Its `hello` said it can draw a terminal (`surfaces`).
    terminals: bool,
}

/// What a view attaching to a terminal gets: its session, and the session's
/// side of the attachment (the output so far and the queue of what follows).
pub struct HubAttach {
    pub session: Arc<Session>,
    pub att: Attached,
}

struct Inner {
    store: Store,
    clients: Vec<Client>,
    next_client: u64,
    /// The file's text as last read or written, so our own write is not
    /// applied again as an edit from outside.
    last_text: Option<String>,
    stop: bool,
    /// The user modules the host reported.
    mods: Mods,
    /// The modules that live in the daemon.
    core: CoreHost,
    /// What the host was last told per module: enabled, and its config.
    host_told: BTreeMap<String, (bool, Value)>,
}

pub struct Hub {
    inner: Mutex<Inner>,
    /// The module host, once started ([`Hub::start_modules`]).
    modhost: Mutex<Option<Arc<ModHost>>>,
    wake: Condvar,
    clock: Clock,
    pub instance: String,
    pub config_path: PathBuf,
}

/// The config file's text; a missing file is an empty config.
fn read_config(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Writes the config atomically: a temp file in the same directory, renamed
/// over the file, so a reader never sees half of it.
pub fn write_config(path: &Path, cfg: &Value) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    text.push('\n');
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = dir {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&tmp, &text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })?;
    Ok(text)
}

impl Inner {
    fn broadcast(&self, msg: &ToUi) {
        for c in self.clients.iter().filter(|c| c.subscribed) {
            let _ = c.tx.send(msg.clone());
        }
    }

    fn send_to(&self, client: u64, msg: ToUi) {
        if let Some(c) = self.clients.iter().find(|c| c.id == client) {
            let _ = c.tx.send(msg);
        }
    }

    /// Sends what changed; true when the notch state did.
    fn announce(&self, changes: Changes) -> bool {
        let state = changes.state.is_some();
        if let Some(c) = changes.config {
            self.broadcast(&ToUi::Config(c));
        }
        if let Some(e) = changes.config_error {
            self.broadcast(&ToUi::ConfigError(e));
        }
        if let Some(s) = changes.schema {
            self.broadcast(&ToUi::Schema(s));
        }
        if let Some(s) = changes.state {
            self.broadcast(&ToUi::State(s));
        }
        state
    }
}

impl Hub {
    /// A hub reading the config at `config_path` now.
    pub fn new(instance: &str, config_path: &Path) -> Arc<Hub> {
        Hub::with_core(instance, config_path, CoreHost::default())
    }

    /// A hub whose modules of the daemon run on `core` (tests give it a fake
    /// environment and network).
    pub fn with_core(instance: &str, config_path: &Path, mut core: CoreHost) -> Arc<Hub> {
        let (notes_tx, notes_rx) = channel::<Note>();
        core.set_notes(notes_tx);
        let hub = Arc::new(Hub {
            inner: Mutex::new(Inner {
                store: Store::new(),
                clients: Vec::new(),
                next_client: 1,
                last_text: None,
                stop: false,
                mods: Mods::new(),
                core,
                host_told: BTreeMap::new(),
            }),
            modhost: Mutex::new(None),
            wake: Condvar::new(),
            clock: Clock::new(),
            instance: instance.to_string(),
            config_path: config_path.to_path_buf(),
        });
        let mut g = hub.lock();
        hub.poll_config(&mut g);
        let _ = hub.publish(&mut g);
        drop(g);
        // What the terminals and the usage requests report: fed to the
        // module they belong to, on a thread of its own (the readers of the
        // programs' output never take the hub's lock).
        let weak = Arc::downgrade(&hub);
        thread::Builder::new()
            .name("notchd-notes".into())
            .spawn(move || {
                for note in notes_rx {
                    match weak.upgrade() {
                        Some(hub) => hub.on_note(&note),
                        None => break,
                    }
                }
            })
            .expect("spawn the notes thread");
        hub
    }

    /// A terminal's or a usage request's report, for its module.
    fn on_note(&self, note: &Note) {
        // What the installer said of the components: no module's.
        if note.provider == "components" {
            let mut g = self.lock();
            let Inner { core, store, .. } = &mut *g;
            core.components_note(store, &note.data);
            let _ = self.publish(&mut g);
            drop(g);
            self.wake.notify_all();
            return;
        }
        let Some(id) = note.data.get("id").and_then(Value::as_str) else {
            return;
        };
        let mut g = self.lock();
        let now = self.clock.now();
        let Inner {
            core, store, mods, ..
        } = &mut *g;
        let msgs = core.feed_to(store, mods, id, note.provider, &note.data, now);
        self.announce_core(&mut g, msgs);
        let _ = self.publish(&mut g);
        drop(g);
        self.wake.notify_all();
    }

    // ---------------------------------------------------------- terminals

    /// A view attaches to the terminal `reference` (`term:<module id>`): the
    /// module hears of it (the first view starts its program) and the view
    /// gets the session. `cols` and `rows` are the size of the view, which the
    /// program starts at.
    pub fn pty_attach(
        &self,
        reference: &str,
        cols: Option<u32>,
        rows: Option<u32>,
    ) -> Result<HubAttach, String> {
        let id = reference
            .strip_prefix("term:")
            .ok_or_else(|| format!("no surface {reference}"))?;
        let mut g = self.lock();
        let now = self.clock.now();
        if !g.core.has(id) {
            return Err(format!("no terminal {id}: its module is not loaded"));
        }
        if let (Some(c), Some(r)) = (cols, rows) {
            g.core
                .set_size_hint(c.clamp(1, 1000) as u16, r.clamp(1, 1000) as u16);
        }
        let before = g.core.session(id).map_or(0, |s| s.views());
        let Inner {
            core, store, mods, ..
        } = &mut *g;
        let msgs = core.pty_views(store, mods, id, before + 1, now);
        let session = g
            .core
            .session(id)
            .ok_or_else(|| format!("no terminal {id}: it did not start"))?;
        let att = session.attach();
        self.announce_core(&mut g, msgs);
        let _ = self.publish(&mut g);
        drop(g);
        self.wake.notify_all();
        Ok(HubAttach { session, att })
    }

    /// The view is gone: its session stays.
    pub fn pty_detach(&self, session: &Arc<Session>, sub: u64) {
        session.detach(sub);
        let mut g = self.lock();
        let now = self.clock.now();
        let id = session.id.clone();
        // The module's current session, which may be a newer one.
        let n = g.core.session(&id).map_or(0, |s| s.views());
        let Inner {
            core, store, mods, ..
        } = &mut *g;
        let msgs = core.pty_views(store, mods, &id, n, now);
        self.announce_core(&mut g, msgs);
        let _ = self.publish(&mut g);
        drop(g);
        self.wake.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Reads the file; a text unlike the last one is a good config to apply
    /// or an error to report (the last good config stays).
    fn poll_config(&self, g: &mut Inner) {
        let text = match read_config(&self.config_path) {
            Ok(t) => t,
            Err(message) => {
                g.store.config_failed(message);
                return;
            }
        };
        if g.last_text.as_deref() == Some(&text) {
            return;
        }
        g.last_text = Some(text.clone());
        match notchd_core::config::parse(&text) {
            Ok(map) => g.store.set_config(Value::Object(map)),
            Err(message) => g
                .store
                .config_failed(format!("{}: {message}", self.config_path.display())),
        }
    }

    /// Writes a config an edit left, then announces what changed. A failed
    /// write is returned: the edit stays applied, as `NotchService` does.
    fn publish(&self, g: &mut Inner) -> Result<(), String> {
        let mut result = Ok(());
        if let Some(cfg) = g.store.take_write() {
            match write_config(&self.config_path, &cfg) {
                Ok(text) => g.last_text = Some(text),
                Err(e) => result = Err(e),
            }
        }
        let changes = g.store.take_changes(self.clock.now());
        // A config that changed may enable or disable a module or change its
        // settings' values.
        let config_changed = changes.config.is_some();
        let state_changed = g.announce(changes);
        if config_changed {
            self.sync_host(g);
            self.sync_core(g);
        } else if state_changed {
            self.notches_changed(g);
        }
        result
    }

    /// Loads and unloads the modules of the daemon to match the config, and
    /// announces them: their `module` and `view` messages, then the state
    /// those facts changed.
    fn sync_core(&self, g: &mut Inner) {
        let now = self.clock.now();
        let Inner {
            core, store, mods, ..
        } = &mut *g;
        let msgs = core.sync(store, mods, now);
        self.announce_core(g, msgs);
    }

    /// Sends what a module of the daemon changed, and the state it moved.
    fn announce_core(&self, g: &mut Inner, msgs: Vec<ToUi>) {
        for m in &msgs {
            g.broadcast(m);
        }
        let changes = g.store.take_changes(self.clock.now());
        if g.announce(changes) {
            self.notches_changed(g);
        }
    }

    /// The notch state changed: the modules that follow it (a voice session
    /// follows its notch opening) are told, and what that moves is sent. A
    /// module tells nothing twice, so this settles within a few rounds.
    fn notches_changed(&self, g: &mut Inner) {
        for _ in 0..4 {
            let now = self.clock.now();
            let Inner {
                core, store, mods, ..
            } = &mut *g;
            let msgs = core.notches_changed(store, mods, now);
            if msgs.is_empty() {
                return;
            }
            for m in &msgs {
                g.broadcast(m);
            }
            let changes = g.store.take_changes(self.clock.now());
            if !g.announce(changes) {
                return;
            }
        }
    }

    // ------------------------------------------------------------ clients

    /// A new connection: its id and the receiver its writer thread drains.
    pub fn register(&self) -> (u64, Receiver<ToUi>) {
        let (tx, rx) = channel();
        let mut g = self.lock();
        let id = g.next_client;
        g.next_client += 1;
        g.clients.push(Client {
            id,
            tx,
            subscribed: false,
            terminals: false,
        });
        (id, rx)
    }

    /// Closes a connection's channel; its writer thread then ends.
    pub fn unregister(&self, client: u64) {
        let mut g = self.lock();
        g.clients.retain(|c| c.id != client);
        let any = g.clients.iter().any(|c| c.terminals);
        g.core.set_terminal_ui(any);
    }

    /// The handshake's first half: what the UI can draw (a terminal needs the
    /// UI's say so), then [`Hub::subscribe`].
    pub fn hello(&self, client: u64, hello: &notchd_proto::ui::Hello) {
        {
            let mut g = self.lock();
            if let Some(c) = g.clients.iter_mut().find(|c| c.id == client) {
                c.terminals = hello.caps.surfaces.iter().any(|s| s == "terminal");
            }
            let any = g.clients.iter().any(|c| c.terminals);
            g.core.set_terminal_ui(any);
        }
        self.subscribe(client);
    }

    pub fn send(&self, client: u64, msg: ToUi) {
        self.lock().send_to(client, msg);
    }

    /// The handshake's second half: `welcome`, the config, the state, in one
    /// go under the lock, so nothing changes between them and the broadcasts
    /// that follow.
    pub fn subscribe(&self, client: u64) {
        let mut g = self.lock();
        g.send_to(
            client,
            ToUi::Welcome(Welcome {
                proto: PROTO,
                daemon: VERSION.into(),
                instance: self.instance.clone(),
                config: self.config_path.display().to_string(),
                exe: std::env::current_exe()
                    .ok()
                    .map(|p| p.display().to_string()),
                tokens: Token::ALL
                    .iter()
                    .filter_map(|t| serde_json::to_value(t).ok()?.as_str().map(String::from))
                    .collect(),
            }),
        );
        if let Some(c) = g.store.config_update() {
            g.send_to(client, ToUi::Config(c));
        }
        // A bad file (after a good config or without one): the UI shows the note.
        if let Some(e) = g.store.config_error() {
            let message = e.to_string();
            g.send_to(
                client,
                ToUi::ConfigError(notchd_proto::ui::ConfigError { message }),
            );
        }
        if let Some(schema) = g.store.schema() {
            g.send_to(client, ToUi::Schema(schema));
        }
        let state = g.store.state();
        g.send_to(client, ToUi::State(state));
        for m in g.mods.snapshot() {
            g.send_to(client, m);
        }
        if let Some(c) = g.clients.iter_mut().find(|c| c.id == client) {
            c.subscribed = true;
        }
    }

    // --------------------------------------------------------- messages

    /// Applies a message from a connection, answers a request on that
    /// connection and announces what changed to every UI.
    pub fn handle(&self, client: u64, msg: &ToDaemon) {
        let mut g = self.lock();
        // A primitive's action in a user module's view goes to the module;
        // `restart` (the error row's button) restarts it.
        if let ToDaemon::Event(e) = msg {
            let now = self.clock.now();
            if g.core.has(&e.module) {
                let Inner {
                    core, store, mods, ..
                } = &mut *g;
                let msgs = core.event(store, mods, &e.module, &e.action, &e.value, now);
                self.announce_core(&mut g, msgs);
                drop(g);
                self.wake.notify_all();
            } else if g.mods.knows(&e.module) {
                self.module_event(&e.module, e);
            }
            return;
        }
        // A method of a module of the daemon (`timer`, `push`, `clear`,
        // `osd`, the test hooks): its answer, after the state it made.
        if let ToDaemon::Ipc(r) = msg
            && let Some(answer) = self.core_ipc(&mut g, &r.method, &r.args)
        {
            let reply = router::ok(r.id, Some(answer));
            let _ = self.publish(&mut g);
            g.send_to(client, reply);
            drop(g);
            self.wake.notify_all();
            return;
        }
        let mut replies = router::route(&mut g.store, msg, self.clock.now());
        // The UI's default modules changed what a notch lists.
        if matches!(msg, ToDaemon::Catalog(_)) {
            self.sync_host(&mut g);
            self.sync_core(&mut g);
        }
        if let Err(e) = self.publish(&mut g) {
            for r in &mut replies {
                if let ToUi::Ok(o) = r {
                    *r = router::err(Some(o.id), "config", e.clone());
                }
            }
        }
        for r in replies {
            g.send_to(client, r);
        }
        drop(g);
        self.wake.notify_all();
    }

    /// An `ipc` method one of the daemon's own modules answers, or a test
    /// hook that feeds them: the answer.
    fn core_ipc(&self, g: &mut Inner, method: &str, args: &[String]) -> Option<String> {
        let now = self.clock.now();
        let Inner {
            core, store, mods, ..
        } = &mut *g;
        let (answer, msgs) = if let Some(id) = method
            .strip_prefix("test-")
            .filter(|_| crate::testhooks::enabled())
        {
            core.demo(store, mods, id, args, now)?
        } else {
            core.command(store, mods, method, args, now)?
        };
        self.announce_core(g, msgs);
        Some(answer)
    }

    /// Feeds a provider's data to the modules of the daemon.
    pub fn feed(&self, provider: &str, data: &Value) {
        let mut g = self.lock();
        let now = self.clock.now();
        let Inner {
            core, store, mods, ..
        } = &mut *g;
        let msgs = core.feed(store, mods, provider, data, now);
        self.announce_core(&mut g, msgs);
        let _ = self.publish(&mut g);
        drop(g);
        self.wake.notify_all();
    }

    /// Starts the readings of the system that feed the modules of the daemon
    /// (`resources`: every 2 s while the module is loaded, on a thread of its
    /// own, so a slow `df` never holds the hub's lock). Not started by a
    /// hub that tests drive by hand.
    pub fn start_providers(self: &Arc<Hub>) {
        let weak = Arc::downgrade(self);
        let spawned = thread::Builder::new()
            .name("notchd-resources".into())
            .spawn(move || {
                let Some(mut provider) = notchd_providers::resources::system() else {
                    return;
                };
                loop {
                    let Some(hub) = weak.upgrade() else { return };
                    if hub.lock().stop {
                        return;
                    }
                    if hub.lock().core.wants_resources() {
                        let sample = provider.sample(notchd_modules::resources::DISK_PATH);
                        if let Ok(data) = serde_json::to_value(sample) {
                            // A hook may have taken over while we read.
                            if hub.lock().core.wants_resources() {
                                hub.feed("resources", &data);
                            }
                        }
                    }
                    drop(hub);
                    thread::sleep(RESOURCES_EVERY);
                }
            });
        if let Err(e) = spawned {
            eprintln!("notchd: cannot start the resources reader: {e}");
        }
        // The media players: its own threads read the bus (nothing while the
        // module is not loaded) and hand over each change; the commands go
        // to them on a channel.
        let wanted = Arc::downgrade(self);
        let fed = Arc::downgrade(self);
        let commands = notchd_providers::media::start(
            Box::new(move || {
                wanted
                    .upgrade()
                    .is_some_and(|h| h.lock().core.wants_media())
            }),
            Box::new(move |data| {
                let Some(hub) = fed.upgrade() else { return };
                if let Ok(data) = serde_json::to_value(data) {
                    hub.feed("media", &data);
                }
            }),
        );
        if let Some(tx) = commands {
            self.lock().core.set_media(tx);
        }
    }

    /// Changes the store directly (a module's facts, a peek request: what the
    /// module host will do) and announces the result.
    pub fn update<R>(&self, f: impl FnOnce(&mut Store, u64) -> R) -> R {
        let mut g = self.lock();
        let r = f(&mut g.store, self.clock.now());
        let _ = self.publish(&mut g);
        drop(g);
        self.wake.notify_all();
        r
    }

    // ------------------------------------------------------ user modules

    fn host(&self) -> Option<Arc<ModHost>> {
        self.modhost
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Runs the user modules of `modules_dir` (a folder each, docs/
    /// architecture.md "Protocol: core to module"), logging to `log_dir`.
    /// Their events apply under the hub's lock; stop them with
    /// [`Hub::stop_modules`].
    pub fn start_modules(self: &Arc<Hub>, modules_dir: &Path, log_dir: &Path) {
        let (tx, rx) = channel::<ModEvent>();
        let host = Arc::new(ModHost::new(modules_dir, log_dir, move |e| {
            let _ = tx.send(e);
        }));
        *self.modhost.lock().unwrap_or_else(|p| p.into_inner()) = Some(host);
        let weak = Arc::downgrade(self);
        thread::Builder::new()
            .name("notchd-modules".into())
            .spawn(move || {
                for ev in rx {
                    match weak.upgrade() {
                        Some(hub) => hub.on_mod_event(ev),
                        None => break,
                    }
                }
            })
            .expect("spawn the module event thread");
        // Modules found before the host was stored.
        let mut g = self.lock();
        self.sync_host(&mut g);
    }

    /// One event of the host: the store and the UIs follow.
    fn on_mod_event(&self, ev: ModEvent) {
        let mut g = self.lock();
        let now = self.clock.now();
        let Inner { mods, store, .. } = &mut *g;
        let msgs = mods.apply(store, ev, now);
        for m in &msgs {
            g.broadcast(m);
        }
        self.sync_host(&mut g);
        let _ = self.publish(&mut g);
        drop(g);
        self.wake.notify_all();
    }

    /// Tells the host what each known module should do: run when a notch
    /// lists it (a standalone one: unless switched off) and the config does
    /// not disable it (`"<id>": {"enabled": false}`), and its settings'
    /// values (the config's `"<id>"` object). Only what changed is sent.
    fn sync_host(&self, g: &mut Inner) {
        let Some(host) = self.host() else { return };
        let cfg = g.store.config().clone();
        let listed: Vec<String> = g
            .store
            .notch_specs()
            .into_iter()
            .flat_map(|n| n.module_ids)
            .collect();
        for id in g.mods.ids() {
            let off = notchd_core::config::value_at(&cfg, &format!("{id}.enabled"))
                == Some(&Value::Bool(false));
            let enabled = !off && (g.mods.standalone(&id) || listed.contains(&id));
            let config = match notchd_core::config::value_at(&cfg, &id) {
                Some(v @ Value::Object(_)) => v.clone(),
                _ => json!({}),
            };
            let told = g.host_told.get(&id);
            if told.map(|t| t.0) != Some(enabled) {
                host.set_enabled(&id, enabled);
            }
            if told.map(|t| &t.1) != Some(&config) {
                host.set_config(&id, config.clone());
            }
            g.host_told.insert(id, (enabled, config));
        }
    }

    /// A UI's `event` for a user module's view.
    fn module_event(&self, module: &str, e: &notchd_proto::ui::Event) {
        let Some(host) = self.host() else { return };
        if e.action == "restart" {
            host.restart(module);
            return;
        }
        host.send(
            module,
            ToModule::Event(ModuleEvent {
                slot: e.slot,
                action: e.action.clone(),
                value: e.value.clone(),
            }),
        );
    }

    /// `stop` to every module and waits until no process is left (the
    /// shutdown path). Safe to call twice, or without a host.
    pub fn stop_modules(&self) {
        let host = self
            .modhost
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(host) = host {
            host.stop_all();
        }
        // The terminals' programs end with the daemon.
        self.lock().core.stop_terminals();
    }

    // ----------------------------------------------------------- ticker

    /// Runs timers and polls the file until [`Hub::stop`].
    pub fn run_ticker(&self) {
        let mut g = self.lock();
        loop {
            if g.stop {
                return;
            }
            g.store.advance(self.clock.now());
            self.poll_config(&mut g);
            let _ = self.publish(&mut g);
            let now = self.clock.now();
            {
                let Inner {
                    core, store, mods, ..
                } = &mut *g;
                let msgs = core.tick(store, mods, now);
                if !msgs.is_empty() {
                    self.announce_core(&mut g, msgs);
                    let _ = self.publish(&mut g);
                }
            }
            let now = self.clock.now();
            let due = [g.store.next_due(), g.core.next_due()]
                .into_iter()
                .flatten()
                .min();
            let wait = due.map_or(CONFIG_POLL, |due| {
                Duration::from_millis(due.saturating_sub(now).max(1)).min(CONFIG_POLL)
            });
            g = self
                .wake
                .wait_timeout(g, wait)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    pub fn stopped(&self) -> bool {
        self.lock().stop
    }

    pub fn stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
    }
}
