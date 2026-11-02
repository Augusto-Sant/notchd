//! `FakeUi`: a UI client for tests. It connects to a daemon's socket, records
//! every message the daemon sends, and has helpers to send and to wait.
//! [`Daemon`] spawns the real `notchd` binary in a [`TestEnv`] and kills it on
//! drop.

use crate::tempdir::TestEnv;
use notchd_proto::request::Ipc;
use notchd_proto::ui::{Caps, Hello, Key, Modifier, Pointer, PointerEv};
use notchd_proto::{PROTO, ToDaemon, ToUi, to_line};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

pub const WAIT: Duration = Duration::from_secs(5);
/// Process start-up under a loaded machine (parallel `cargo test` binaries):
/// a bound only, never a sleep that a test waits out.
pub const START_WAIT: Duration = Duration::from_secs(30);

/// A running `notchd daemon`, killed and reaped on drop.
pub struct Daemon {
    child: Child,
    pub socket: PathBuf,
}

impl Daemon {
    /// Starts `bin daemon` in `env` and waits (bounded) until it accepts.
    pub fn start(bin: &Path, env: &TestEnv, instance: &str) -> Daemon {
        let mut cmd = Command::new(bin);
        env.apply(&mut cmd);
        cmd.args(["--instance", instance, "daemon"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn().expect("spawn notchd");
        let socket = env.socket(instance);
        let mut daemon = Daemon { child, socket };
        let deadline = Instant::now() + START_WAIT;
        while UnixStream::connect(&daemon.socket).is_err() {
            assert!(Instant::now() < deadline, "the daemon never listened");
            assert!(
                daemon.child.try_wait().expect("wait").is_none(),
                "the daemon exited early"
            );
            thread::sleep(Duration::from_millis(10));
        }
        daemon
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs a command to completion, killing it after `limit`. Returns the exit
/// code (`None` if killed or signalled), stdout and stderr.
pub fn run_bounded(mut cmd: Command, limit: Duration) -> (Option<i32>, String, String) {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn");
    let deadline = Instant::now() + limit;
    while child.try_wait().expect("wait").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let out = child.wait_with_output().expect("collect output");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

enum Ev {
    Msg(ToUi),
    Unparsed(String),
    Closed,
}

pub struct FakeUi {
    stream: UnixStream,
    rx: Receiver<Ev>,
    /// Every message received, in order.
    pub messages: Vec<ToUi>,
    /// Lines that did not parse as a `ToUi`.
    pub unparsed: Vec<String>,
    pub closed: bool,
    next_id: u64,
}

impl FakeUi {
    /// Connects without a handshake.
    pub fn connect(socket: &Path) -> FakeUi {
        let stream = UnixStream::connect(socket).expect("connect to the daemon");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        let (tx, rx) = channel();
        thread::spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let ev = match notchd_proto::parse::<ToUi>(&line) {
                    Ok(m) => Ev::Msg(m),
                    Err(_) => Ev::Unparsed(line),
                };
                if tx.send(ev).is_err() {
                    return;
                }
            }
            let _ = tx.send(Ev::Closed);
        });
        FakeUi {
            stream,
            rx,
            messages: Vec::new(),
            unparsed: Vec::new(),
            closed: false,
            next_id: 1,
        }
    }

    /// Connects and sends `hello` with this proto.
    pub fn connect_with_proto(socket: &Path, proto: u32) -> FakeUi {
        let mut ui = FakeUi::connect(socket);
        ui.send(&ToDaemon::Hello(Hello {
            proto,
            client: "fake-ui".into(),
            client_version: "0".into(),
            instance: "default".into(),
            caps: Caps {
                primitives: ["row", "column", "text"].map(String::from).to_vec(),
                surfaces: Vec::new(),
                webview: false,
                canvas: false,
            },
        }));
        ui
    }

    /// Connects and sends a correct `hello`.
    pub fn connect_hello(socket: &Path) -> FakeUi {
        FakeUi::connect_with_proto(socket, PROTO)
    }

    pub fn send_raw(&mut self, line: &str) {
        self.stream.write_all(line.as_bytes()).expect("send");
        self.stream.write_all(b"\n").expect("send");
    }

    pub fn send(&mut self, msg: &ToDaemon) {
        self.send_raw(&to_line(msg));
    }

    pub fn pointer(&mut self, notch: &str, screen: &str, ev: PointerEv) {
        self.send(&ToDaemon::Pointer(Pointer {
            notch: notch.into(),
            screen: screen.into(),
            ev,
        }));
    }

    pub fn key(&mut self, notch: &str, screen: &str, key: &str, mods: &[Modifier]) {
        self.send(&ToDaemon::Key(Key {
            notch: notch.into(),
            screen: screen.into(),
            key: key.into(),
            mods: mods.to_vec(),
            text: String::new(),
        }));
    }

    /// Sends an `ipc` request and returns its id.
    pub fn ipc(&mut self, method: &str, args: &[&str]) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&ToDaemon::Ipc(Ipc {
            id,
            method: method.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }));
        id
    }

    fn take(&mut self, ev: Ev) {
        match ev {
            Ev::Msg(m) => self.messages.push(m),
            Ev::Unparsed(l) => self.unparsed.push(l),
            Ev::Closed => self.closed = true,
        }
    }

    /// Waits up to `limit` for a message matching `pred`, searching what was
    /// already recorded first. Nothing is consumed.
    pub fn wait_for(&mut self, limit: Duration, pred: impl Fn(&ToUi) -> bool) -> Option<ToUi> {
        let deadline = Instant::now() + limit;
        let mut seen = 0;
        loop {
            while seen < self.messages.len() {
                if pred(&self.messages[seen]) {
                    return Some(self.messages[seen].clone());
                }
                seen += 1;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(ev) => self.take(ev),
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => {
                    self.closed = true;
                    return None;
                }
            }
        }
    }

    /// Waits up to `limit` for the daemon to close the connection.
    pub fn wait_closed(&mut self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while !self.closed {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(ev) => self.take(ev),
                Err(RecvTimeoutError::Timeout) => return false,
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
            }
        }
        true
    }
}

impl Drop for FakeUi {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
