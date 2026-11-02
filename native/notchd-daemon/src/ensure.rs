//! `notchd daemon --ensure`: one daemon per instance, started on demand.
//!
//! A compatible daemon answering is enough. Otherwise the daemon starts
//! detached (its own session, stdio to the log file, so it is no child of
//! the shell that asked and outlives it) and this returns once the socket
//! answers, within a bound. A daemon of another version is asked to stop
//! (`ipc shutdown`) and replaced.

use crate::hub::VERSION;
use crate::instance::Instance;
use crate::ipc;
use notchd_proto::ui::{Caps, Hello};
use notchd_proto::{PROTO, ToDaemon, ToUi, to_line};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// How long a new daemon has to answer, and an old one to go away.
const START_WAIT: Duration = Duration::from_secs(10);
const PROBE_WAIT: Duration = Duration::from_secs(2);

unsafe extern "C" {
    fn setsid() -> i32;
}

/// Whether a daemon answers on the instance's socket.
pub fn running(instance: &Instance) -> bool {
    UnixStream::connect(&instance.socket).is_ok()
}

/// What a running daemon says it is, from its `welcome`: its version, or
/// `None` when it refuses the handshake or does not answer in time.
pub fn daemon_version(instance: &Instance) -> Option<String> {
    let mut stream = UnixStream::connect(&instance.socket).ok()?;
    stream.set_read_timeout(Some(PROBE_WAIT)).ok()?;
    stream.set_write_timeout(Some(PROBE_WAIT)).ok()?;
    let hello = ToDaemon::Hello(Hello {
        proto: PROTO,
        client: "notchd-ensure".into(),
        client_version: VERSION.into(),
        instance: instance.name.clone(),
        caps: Caps {
            primitives: Vec::new(),
            surfaces: Vec::new(),
            webview: false,
            canvas: false,
        },
    });
    writeln!(stream, "{}", to_line(&hello)).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while reader.read_line(&mut line).ok()? > 0 {
        match notchd_proto::parse::<ToUi>(line.trim_end()) {
            Ok(ToUi::Welcome(w)) => return Some(w.daemon),
            Ok(ToUi::Err(_)) => return None,
            _ => line.clear(),
        }
    }
    None
}

/// `$XDG_STATE_HOME/notchd/daemon.log` (`daemon-<instance>.log` for another
/// instance), `~/.local/state` without the variable.
pub fn log_path(instance: &Instance, get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let get = |k: &str| get(k).filter(|v| !v.is_empty());
    let base = match get("XDG_STATE_HOME") {
        Some(x) => PathBuf::from(x),
        None => Path::new(&get("HOME")?).join(".local").join("state"),
    };
    let file = if instance.name == crate::instance::DEFAULT_INSTANCE {
        "daemon.log".to_string()
    } else {
        format!("daemon-{}.log", instance.name)
    };
    Some(base.join("notchd").join(file))
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while !done() {
        if Instant::now() > deadline {
            return false;
        }
        sleep(Duration::from_millis(20));
    }
    true
}

/// Starts this binary as the instance's daemon, detached, and waits for its
/// socket.
fn spawn(instance: &Instance) -> Result<(), String> {
    let log = log_path(instance, &|k| std::env::var(k).ok())
        .ok_or("no XDG_STATE_HOME or HOME for the daemon's log")?;
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let open = || {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| format!("{}: {e}", log.display()))
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("--instance")
        .arg(&instance.name)
        .arg("--socket")
        .arg(&instance.socket)
        .arg("--config")
        .arg(&instance.config)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(open()?)
        .stderr(open()?);
    // SAFETY: setsid is async-signal-safe and touches no memory of ours.
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start notchd: {e}"))?;
    let timed_out = !wait_until(START_WAIT, || {
        running(instance) || child.try_wait().ok().flatten().is_some()
    });
    // Another `--ensure` may have won the race: its daemon answering is fine.
    if running(instance) {
        return Ok(());
    }
    let why = if timed_out {
        "it did not answer"
    } else {
        "it exited"
    };
    Err(format!(
        "the daemon did not start ({why}); see {}",
        log.display()
    ))
}

/// Makes sure a daemon of this version answers on the instance's socket and
/// returns the socket path.
pub fn ensure(instance: Instance) -> Result<String, String> {
    if running(&instance) {
        match daemon_version(&instance) {
            Some(v) if v == VERSION => return Ok(instance.socket.display().to_string()),
            other => {
                let was = other.unwrap_or_else(|| "an unknown version".into());
                ipc::call(&instance.socket, "shutdown", &[]).map_err(|e| {
                    format!("notchd {was} is running and cannot be stopped ({e}): update notchd")
                })?;
                if !wait_until(START_WAIT, || !running(&instance)) {
                    return Err(format!("notchd {was} did not stop: update notchd"));
                }
            }
        }
    }
    spawn(&instance)?;
    Ok(instance.socket.display().to_string())
}
