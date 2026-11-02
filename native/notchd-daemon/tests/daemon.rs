//! The daemon over a real socket: the real `notchd` binary in a temp dir, a
//! `FakeUi` as the client. Hermetic: temp runtime dir and config, every
//! process killed on drop, every wait bounded.

use notchd_proto::ToUi;
use notchd_proto::version::MAX_LINE_BYTES;
use notchd_testkit::fake_ui::{Daemon, FakeUi, START_WAIT, WAIT, run_bounded};
use notchd_testkit::tempdir::TestEnv;
use std::process::Command;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_notchd");

fn start(label: &str, config: Option<&str>) -> (TestEnv, Daemon) {
    let env = TestEnv::new(label);
    if let Some(text) = config {
        env.write_config(text);
    }
    let daemon = Daemon::start(BIN.as_ref(), &env, "default");
    (env, daemon)
}

fn cli(env: &TestEnv, args: &[&str]) -> (Option<i32>, String, String) {
    let mut cmd = Command::new(BIN);
    env.apply(&mut cmd);
    cmd.args(args);
    run_bounded(cmd, WAIT)
}

#[test]
fn hello_gets_welcome_then_config() {
    let (_env, daemon) = start("welcome", Some(r#"{"hoverOpenDelay":200}"#));
    let mut ui = FakeUi::connect_hello(&daemon.socket);
    let welcome = ui
        .wait_for(WAIT, |m| matches!(m, ToUi::Welcome(_)))
        .expect("welcome");
    let ToUi::Welcome(w) = welcome else {
        unreachable!()
    };
    assert_eq!((w.proto, w.instance.as_str()), (1, "default"));
    assert_eq!(w.tokens.len(), 11);
    let ToUi::Config(c) = ui
        .wait_for(WAIT, |m| matches!(m, ToUi::Config(_)))
        .expect("config")
    else {
        unreachable!()
    };
    assert_eq!(c.cfg["hoverOpenDelay"], 200);
    assert_eq!(c.resolved["hoverOpenDelay"], 200);
    assert_eq!(
        c.resolved["hoverCloseDelay"], 300,
        "a missing key means its default"
    );
    let at = |f: fn(&ToUi) -> bool| ui.messages.iter().position(f).unwrap();
    assert!(at(|m| matches!(m, ToUi::Welcome(_))) < at(|m| matches!(m, ToUi::Config(_))));
}

#[test]
fn a_missing_config_file_is_an_empty_config() {
    let (_env, daemon) = start("noconf", None);
    let mut ui = FakeUi::connect_hello(&daemon.socket);
    let ToUi::Config(c) = ui
        .wait_for(WAIT, |m| matches!(m, ToUi::Config(_)))
        .expect("config")
    else {
        unreachable!()
    };
    assert_eq!(c.cfg, serde_json::json!({}));
}

#[test]
fn a_bad_proto_is_refused_and_the_connection_closed() {
    let (_env, daemon) = start("proto", None);
    let mut ui = FakeUi::connect_with_proto(&daemon.socket, 99);
    let ToUi::Err(e) = ui
        .wait_for(WAIT, |m| matches!(m, ToUi::Err(_)))
        .expect("err")
    else {
        unreachable!()
    };
    assert_eq!((e.code.as_str(), e.id, e.want), ("proto", None, Some(1)));
    assert!(ui.wait_closed(WAIT), "the daemon closes after refusing");
    assert!(
        !ui.messages
            .iter()
            .any(|m| matches!(m, ToUi::Welcome(_) | ToUi::Config(_)))
    );
}

#[test]
fn a_malformed_line_gets_an_error_and_the_connection_stays_usable() {
    let (_env, daemon) = start("bad", None);
    let mut ui = FakeUi::connect_hello(&daemon.socket);
    ui.send_raw("this is not json");
    ui.send_raw(r#"{"t":"nonsense"}"#);
    ui.send_raw(r#"{"t":"ipc","id":41,"method":3}"#);
    let id = ui.ipc("no-such-method", &[]);
    let reply = ui
        .wait_for(WAIT, |m| matches!(m, ToUi::Err(e) if e.id == Some(id)))
        .expect("the connection still answers");
    let ToUi::Err(e) = reply else { unreachable!() };
    assert_eq!(e.code, "method");
    let errs: Vec<_> = ui
        .messages
        .iter()
        .filter_map(|m| if let ToUi::Err(e) = m { Some(e) } else { None })
        .collect();
    assert_eq!(errs.len(), 4);
    assert!(errs[..3].iter().all(|e| e.code == "args"));
    assert_eq!(errs[2].id, Some(41), "a request id is kept in the error");
    ui.pointer("main", "DP-1", notchd_proto::ui::PointerEv::Enter);
    assert!(!ui.closed);
}

#[test]
fn a_line_over_the_limit_is_refused_and_closed() {
    let (_env, daemon) = start("big", None);
    let mut ui = FakeUi::connect_hello(&daemon.socket);
    ui.send_raw(&"x".repeat(MAX_LINE_BYTES + 10));
    assert!(
        ui.wait_for(WAIT, |m| matches!(m, ToUi::Err(e) if e.code == "args"))
            .is_some()
    );
    assert!(ui.wait_closed(WAIT));
}

#[test]
fn a_config_parse_error_yields_config_error() {
    let (_env, daemon) = start("conferr", Some("{ not json"));
    let mut ui = FakeUi::connect_hello(&daemon.socket);
    assert!(
        ui.wait_for(WAIT, |m| matches!(m, ToUi::Welcome(_)))
            .is_some()
    );
    let ToUi::ConfigError(e) = ui
        .wait_for(WAIT, |m| matches!(m, ToUi::ConfigError(_)))
        .expect("config-error")
    else {
        unreachable!()
    };
    assert!(!e.message.is_empty());
    assert!(!ui.messages.iter().any(|m| matches!(m, ToUi::Config(_))));
}

#[test]
fn ipc_with_an_unknown_method_prints_the_error_and_exits_non_zero() {
    let (env, _daemon) = start("cli", None);
    let (code, stdout, stderr) = cli(&env, &["ipc", "no-such-method", "a"]);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("method") && stderr.contains("unknown method"),
        "{stderr}"
    );
}

#[test]
fn ipc_without_a_daemon_fails_cleanly() {
    let env = TestEnv::new("nod");
    let (code, _out, stderr) = cli(&env, &["ipc", "state"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("cannot reach the daemon"), "{stderr}");
}

#[test]
fn ensure_returns_at_once_when_a_daemon_runs() {
    let (env, daemon) = start("ensure", None);
    let mut cmd = Command::new(BIN);
    env.apply(&mut cmd);
    cmd.args(["daemon", "--ensure"]);
    let (code, _o, e) = run_bounded(cmd, Duration::from_secs(5));
    assert_eq!(code, Some(0), "{e}");
    drop(daemon);
}

#[test]
fn a_second_daemon_on_the_same_socket_is_refused() {
    let (env, _daemon) = start("twice", None);
    let (code, _o, stderr) = cli(&env, &["daemon"]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("already listens"), "{stderr}");
}

#[test]
fn a_stale_socket_file_is_replaced() {
    let env = TestEnv::new("stale");
    let sock = env.socket("default");
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());
    assert!(sock.exists());
    // The listener's fd can outlive `drop` for a moment: another test thread
    // forking a daemon holds a copy until its exec. A daemon that probed the
    // socket in that window would find it answered and refuse to start. Wait
    // (bounded) until nobody answers: that is what "stale" means.
    let deadline = std::time::Instant::now() + START_WAIT;
    while std::os::unix::net::UnixStream::connect(&sock).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "the stale socket still answers"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let daemon = Daemon::start(BIN.as_ref(), &env, "default");
    let mut ui = FakeUi::connect_hello(&daemon.socket);
    assert!(
        ui.wait_for(START_WAIT, |m| matches!(m, ToUi::Welcome(_)))
            .is_some()
    );
}
