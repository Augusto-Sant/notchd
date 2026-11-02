//! `notchd daemon --ensure` against the real binary: it starts the daemon
//! detached, reuses a running one, and replaces one of another version.
//! Hermetic: temp runtime, state and config dirs; every daemon started is
//! killed at the end (by pid from its log, then the temp dir goes).

use notchd_proto::ToUi;
use notchd_testkit::fake_ui::{FakeUi, START_WAIT, WAIT, run_bounded};
use notchd_testkit::tempdir::TestEnv;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Instant;

const BIN: &str = env!("CARGO_BIN_EXE_notchd");

/// Kills the daemon whose pid the log names, on drop.
struct Reaper(PathBuf);

impl Reaper {
    fn pids(&self) -> Vec<u32> {
        let text = std::fs::read_to_string(&self.0).unwrap_or_default();
        text.lines()
            .filter_map(|l| l.split("(pid ").nth(1)?.split(')').next()?.parse().ok())
            .collect()
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        for pid in self.pids() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }
}

fn ensure(env: &TestEnv) -> (Option<i32>, String, String) {
    let mut cmd = Command::new(BIN);
    env.apply(&mut cmd);
    cmd.args(["daemon", "--ensure"]);
    run_bounded(cmd, START_WAIT)
}

fn log_of(env: &TestEnv) -> PathBuf {
    env.dir.path().join(".local/state/notchd/daemon.log")
}

fn welcome_version(socket: &std::path::Path) -> String {
    let mut ui = FakeUi::connect_hello(socket);
    let Some(ToUi::Welcome(w)) = ui.wait_for(WAIT, |m| matches!(m, ToUi::Welcome(_))) else {
        panic!("no welcome")
    };
    w.daemon
}

#[test]
fn ensure_starts_a_detached_daemon_and_a_second_one_reuses_it() {
    let env = TestEnv::new("ens");
    let reaper = Reaper(log_of(&env));
    let (code, out, err) = ensure(&env);
    assert_eq!(code, Some(0), "{err}");
    let socket = env.socket("default");
    assert_eq!(out.trim(), socket.display().to_string());
    assert_eq!(welcome_version(&socket), env!("CARGO_PKG_VERSION"));

    // Detached: it logs to the state dir and leads its own session, and it
    // is still there now that `--ensure` has exited.
    let pids = reaper.pids();
    assert_eq!(pids.len(), 1, "one daemon in the log");
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pids[0])).expect("daemon alive");
    let after_comm = stat.rsplit(')').next().unwrap();
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    let session: u32 = fields[3].parse().unwrap();
    assert_eq!(session, pids[0], "setsid: its own session");

    let (code, out, err) = ensure(&env);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out.trim(), socket.display().to_string());
    assert_eq!(reaper.pids(), pids, "the running daemon was reused");
}

#[test]
fn shutdown_replies_then_the_daemon_exits_and_frees_its_socket() {
    let env = TestEnv::new("shut");
    let reaper = Reaper(log_of(&env));
    assert_eq!(ensure(&env).0, Some(0));
    let pid = reaper.pids()[0];
    let mut cmd = Command::new(BIN);
    env.apply(&mut cmd);
    cmd.args(["ipc", "shutdown"]);
    let (code, out, err) = run_bounded(cmd, WAIT);
    assert_eq!((code, out.trim()), (Some(0), "ok"), "{err}");
    let deadline = Instant::now() + WAIT;
    while std::path::Path::new(&format!("/proc/{pid}")).exists() || env.socket("default").exists() {
        assert!(Instant::now() < deadline, "the daemon did not exit");
        thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A daemon of another version: answers `hello` with a `welcome` naming it,
/// and on `ipc shutdown` replies `ok` and goes.
fn fake_old_daemon(socket: PathBuf) -> thread::JoinHandle<bool> {
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(&socket).unwrap();
    thread::spawn(move || {
        let mut shut = false;
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                if line.contains(r#""t":"hello""#) {
                    writeln!(
                        out,
                        r#"{{"t":"welcome","proto":1,"daemon":"0.0.1","instance":"default","config":"","tokens":[]}}"#
                    )
                    .unwrap();
                } else if line.contains("shutdown") {
                    writeln!(out, r#"{{"t":"ok","id":1,"result":"ok"}}"#).unwrap();
                    shut = true;
                }
                line.clear();
                if shut {
                    break;
                }
            }
            if shut {
                break;
            }
        }
        let _ = std::fs::remove_file(&socket);
        shut
    })
}

#[test]
fn a_daemon_of_another_version_is_stopped_and_replaced() {
    let env = TestEnv::new("old");
    let reaper = Reaper(log_of(&env));
    let old = fake_old_daemon(env.socket("default"));
    let (code, out, err) = ensure(&env);
    assert_eq!(code, Some(0), "{err}");
    assert!(old.join().unwrap(), "the old daemon was asked to shut down");
    assert_eq!(out.trim(), env.socket("default").display().to_string());
    assert_eq!(
        welcome_version(&env.socket("default")),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(reaper.pids().len(), 1, "a new daemon was started");
}

#[test]
fn a_daemon_that_cannot_be_stopped_is_reported() {
    let env = TestEnv::new("stuck");
    let socket = env.socket("default");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(&socket).unwrap();
    // Answers a welcome of another version, and an err to everything else.
    thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let reply = if line.contains(r#""t":"hello""#) {
                    r#"{"t":"welcome","proto":1,"daemon":"0.0.1","instance":"default","config":"","tokens":[]}"#
                } else {
                    r#"{"t":"err","id":1,"code":"method","message":"unknown method: shutdown"}"#
                };
                let _ = writeln!(out, "{reply}");
                line.clear();
            }
        }
    });
    let (code, _out, err) = ensure(&env);
    assert_eq!(code, Some(1));
    assert!(err.contains("update notchd"), "{err}");
}
