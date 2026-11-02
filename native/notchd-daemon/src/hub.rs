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
//! One ticker thread sleeps until the store's next timer (or the next poll of
//! the config file, 100 ms) and wakes early when a change moves it.

use crate::modhost::{ModEvent, ModHost};
use crate::modstate::Mods;
use crate::router;
use notchd_core::store::{Changes, DEFAULT_MODULES, Store};
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

    fn announce(&self, changes: Changes) {
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
    }
}

impl Hub {
    /// A hub reading the config at `config_path` now.
    pub fn new(instance: &str, config_path: &Path) -> Arc<Hub> {
        let hub = Arc::new(Hub {
            inner: Mutex::new(Inner {
                store: Store::new(),
                clients: Vec::new(),
                next_client: 1,
                last_text: None,
                stop: false,
                mods: Mods::new(),
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
        hub
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
        g.announce(changes);
        if config_changed {
            self.sync_host(g);
        }
        result
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
        });
        (id, rx)
    }

    /// Closes a connection's channel; its writer thread then ends.
    pub fn unregister(&self, client: u64) {
        self.lock().clients.retain(|c| c.id != client);
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
            if g.mods.knows(&e.module) {
                self.module_event(&e.module, e);
            }
            return;
        }
        let mut replies = router::route(&mut g.store, msg, self.clock.now());
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
        let listed: Vec<String> = notchd_core::config::notches(&cfg, &DEFAULT_MODULES)
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
            let wait = g.store.next_due().map_or(CONFIG_POLL, |due| {
                Duration::from_millis(due.saturating_sub(now).max(1)).min(CONFIG_POLL)
            });
            g = self
                .wake
                .wait_timeout(g, wait)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    pub fn stop(&self) {
        self.lock().stop = true;
        self.wake.notify_all();
    }
}
