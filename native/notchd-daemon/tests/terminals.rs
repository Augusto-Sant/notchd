//! The agents and terminals in the daemon, over a real socket in process: the
//! fake `claude` and `codex` of `tests/qml/fixtures/bin` run in real PTYs, a
//! `FakeUi` is the UI and `attach::Client` the terminal view. Hermetic (temp
//! dirs, a fake network and a scratch home: no real login, no real
//! `~/.config/notchd`), every wait bounded, every program stopped and reaped
//! when the rig goes.

use notchd_daemon::attach::{Client, Event};
use notchd_daemon::corehost::{CoreHost, Env};
use notchd_daemon::hub::Hub;
use notchd_daemon::instance::Instance;
use notchd_daemon::server::Server;
use notchd_proto::ToUi;
use notchd_proto::common::{Mode, Slot};
use notchd_proto::request::OkReply;
use notchd_providers::usage::{Http, Response};
use notchd_testkit::fake_ui::{FakeUi, WAIT};
use notchd_testkit::tempdir::TestEnv;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// What the fake network was asked, and what it answers.
struct FakeNet {
    asked: Mutex<Vec<(String, BTreeMap<String, String>)>>,
    answer: Mutex<Response>,
}

impl Http for FakeNet {
    fn get(&self, url: &str, headers: &BTreeMap<String, String>, _: Duration) -> Response {
        self.asked
            .lock()
            .unwrap()
            .push((url.into(), headers.clone()));
        self.answer.lock().unwrap().clone()
    }
}

struct Rig {
    env: TestEnv,
    hub: Arc<Hub>,
    net: Arc<FakeNet>,
    claude_args: PathBuf,
    codex_args: PathBuf,
    scratch: PathBuf,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/qml/fixtures/bin")
}

fn rig(label: &str, config: impl FnOnce(&Rig) -> Value, offline: bool) -> Rig {
    let env = TestEnv::new(label);
    let scratch = env.dir.path().to_path_buf();
    let home = scratch.join("home");
    std::fs::create_dir_all(home.join("claude")).unwrap();
    let net = Arc::new(FakeNet {
        asked: Mutex::new(Vec::new()),
        answer: Mutex::new(Response {
            status: 200,
            body: json!({"five_hour": {"utilization": 42.0, "resets_at": "2099-01-01T00:00:00Z"},
                         "seven_day": {"utilization": 10.0, "resets_at": "2099-01-02T00:00:00Z"}})
            .to_string(),
            retry_after: String::new(),
        }),
    });
    let h = home.display().to_string();
    let reads: Env = Arc::new(move |k: &str| match k {
        "HOME" => Some(h.clone()),
        "CLAUDE_CONFIG_DIR" => Some(format!("{h}/claude")),
        "CODEX_HOME" => Some(format!("{h}/codex")),
        "NOTCHD_OFFLINE" => offline.then(|| "1".to_string()),
        other => std::env::var(other).ok(),
    });
    let core = CoreHost::new(None)
        .with_env(reads)
        .with_http(Arc::clone(&net) as Arc<dyn Http>);
    let mut r = Rig {
        env,
        hub: Hub::new("x", Path::new("/nonexistent")),
        net,
        claude_args: scratch.join("claude-args"),
        codex_args: scratch.join("codex-args"),
        scratch,
    };
    let cfg = config(&r);
    r.env.write_config(&cfg.to_string());
    let server = Server::bind_with(
        Instance {
            name: "default".into(),
            socket: r.env.socket("default"),
            config: r.env.config.clone(),
        },
        core,
    )
    .expect("bind");
    r.hub = server.hub();
    thread::spawn(move || server.serve());
    r
}

impl Rig {
    fn socket(&self) -> PathBuf {
        self.env.socket("default")
    }

    /// The command that runs a fixture, recording to its args file.
    fn fake(&self, which: &str) -> String {
        let (var, file) = if which == "claude" {
            ("NOTCHD_FAKE_CLAUDE_ARGS", &self.claude_args)
        } else {
            ("NOTCHD_FAKE_CODEX_ARGS", &self.codex_args)
        };
        format!(
            "env {var}={} {}/{which}",
            file.display(),
            fixtures().display()
        )
    }

    /// A UI that can draw terminals, with a screen so the notch exists.
    fn ui(&self) -> FakeUi {
        let mut ui =
            FakeUi::connect_with_surfaces(&self.socket(), notchd_proto::PROTO, &["terminal"]);
        ui.send_raw(
            r#"{"t":"screens","screens":[{"name":"DP-1","x":0,"y":0,"w":1000,"h":800,"scale":1}],"focused":"DP-1","fullscreen":[],"workspaces":{},"known":[],"supportsWorkspaces":false,"barEdge":null}"#,
        );
        ui
    }

    fn ipc(&self, ui: &mut FakeUi, method: &str, args: &[&str]) -> String {
        let id = ui.ipc(method, args);
        let m = ui
            .wait_for(WAIT, |m| {
                matches!(m, ToUi::Ok(o) if o.id == id)
                    || matches!(m, ToUi::Err(e) if e.id == Some(id))
            })
            .expect("a reply");
        match m {
            ToUi::Ok(OkReply { result, .. }) => result.unwrap_or_default(),
            other => panic!("{other:?}"),
        }
    }

    fn args(&self, file: &Path) -> Vec<String> {
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn wait_args(&self, file: &Path, ok: impl Fn(&[String]) -> bool) -> Vec<String> {
        let limit = Instant::now() + WAIT;
        loop {
            let a = self.args(file);
            if ok(&a) || Instant::now() > limit {
                return a;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Every program stops and is reaped. A stopped hub is one the
        // server's watcher leaves alone when this test's dir goes.
        self.hub.stop();
        self.hub.stop_modules();
    }
}

fn tree_text(m: &ToUi) -> String {
    match m {
        ToUi::View(v) => serde_json::to_string(&v.tree).unwrap(),
        _ => String::new(),
    }
}

fn view_has(module: &'static str, slot: Slot, needle: &'static str) -> impl Fn(&ToUi) -> bool {
    move |m| {
        matches!(m, ToUi::View(v) if v.module == module && v.slot == slot)
            && tree_text(m).contains(needle)
    }
}

/// Reads the view's output until `done` accepts the text, or WAIT passes.
fn read_until(c: &mut Client, mut text: String, done: impl Fn(&str) -> bool) -> String {
    let limit = Instant::now() + WAIT;
    while !done(&text) && Instant::now() < limit {
        if let Ok(Some(Event::Data(d))) = c.next(Duration::from_millis(100)) {
            text.push_str(&String::from_utf8_lossy(&d));
        }
    }
    text
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn gone(pid: i32) -> bool {
    let limit = Instant::now() + WAIT;
    while alive(pid) && Instant::now() < limit {
        thread::sleep(Duration::from_millis(20));
    }
    !alive(pid)
}

fn claude_cfg(r: &Rig) -> Value {
    json!({"modules": ["claude"], "claude": {"command": r.fake("claude"),
        "directory": r.scratch.display().to_string(), "usage": false}})
}

#[test]
fn an_agent_starts_with_its_view_and_its_title_drives_the_closed_notch() {
    let r = rig("agent", claude_cfg, true);
    let mut ui = r.ui();
    // Nothing runs until a view attaches.
    assert!(
        ui.wait_for(WAIT, view_has("claude", Slot::Compact, "Claude"))
            .is_some()
    );
    assert!(r.args(&r.claude_args).is_empty());
    let mut view = Client::connect(&r.socket(), "term:claude", 100, 30).expect("attach");
    let text = read_until(&mut view, String::new(), |t| {
        t.contains("fake claude ready")
    });
    assert!(text.contains("fake claude ready"), "{text:?}");
    // The command line, the folder and a terminal size (the program ran in a PTY at the view's size).
    let a = r.wait_args(&r.claude_args, |a| a.len() >= 3);
    assert_eq!(a[1], r.scratch.display().to_string(), "the folder setting");
    // It works: the half circle in its title is the dot, then the name.
    view.input(b"work\n").unwrap();
    let working = |s: &'static str| view_has("claude", Slot::Compact, s);
    assert!(
        ui.wait_for(WAIT, working("success")).is_some(),
        "no green dot while it works"
    );
    assert!(ui.wait_for(WAIT, working("Fake conversation")).is_some());
    let text = read_until(&mut view, text, |t| t.contains("worked"));
    assert!(text.contains("worked"), "{text:?}");
    // What the expanded view shows: the terminal's surface and the title.
    let expanded = ui
        .wait_for(
            WAIT,
            view_has("claude", Slot::Expanded, "Fake conversation"),
        )
        .unwrap();
    let t = tree_text(&expanded);
    assert!(
        t.contains(r#""t":"surface""#) && t.contains(r#""ref":"term:claude""#),
        "{t}"
    );
    // quit: the view hears it end, and the expanded view says so.
    view.input(b"quit\n").unwrap();
    let mut closed = false;
    let limit = Instant::now() + WAIT;
    while !closed && Instant::now() < limit {
        closed = matches!(
            view.next(Duration::from_millis(100)),
            Ok(Some(Event::Close))
        );
    }
    assert!(closed, "the view was not closed when the program ended");
    assert!(
        ui.wait_for(WAIT, view_has("claude", Slot::Expanded, "Claude exited"))
            .is_some()
    );
}

#[test]
fn the_session_survives_the_ui_and_a_new_view_gets_what_was_drawn() {
    let r = rig("survive", claude_cfg, true);
    let pid = {
        let mut ui = r.ui();
        let mut view = Client::connect(&r.socket(), "term:claude", 80, 24).unwrap();
        read_until(&mut view, String::new(), |t| {
            t.contains("fake claude ready")
        });
        view.input(b"work\n").unwrap();
        let text = read_until(&mut view, String::new(), |t| t.contains("worked"));
        assert!(text.contains("worked"), "{text:?}");
        let a = r.wait_args(&r.claude_args, |a| a.len() >= 3);
        let _ = &mut ui;
        a[2].parse::<i32>().unwrap()
        // The view and the UI go: the shell reloaded.
    };
    assert!(alive(pid), "the program did not outlive its UI");
    thread::sleep(Duration::from_millis(200));
    assert!(alive(pid));
    // The UI is back, a new view attaches to the same program.
    let mut ui = r.ui();
    let mut view = Client::connect(&r.socket(), "term:claude", 80, 24).unwrap();
    let text = read_until(&mut view, String::new(), |t| t.contains("worked"));
    assert!(
        text.contains("fake claude ready") && text.contains("worked"),
        "the replay: {text:?}"
    );
    // Still the same program: it answers, and its pid is the one that started.
    view.input(b"hello again\n").unwrap();
    let text = read_until(&mut view, text, |t| t.contains("got: hello again"));
    assert!(text.contains("got: hello again"), "{text:?}");
    assert_eq!(r.args(&r.claude_args)[2].parse::<i32>().unwrap(), pid);
    assert!(
        ui.wait_for(WAIT, view_has("claude", Slot::Compact, "Fake conversation"))
            .is_some()
    );
    // The daemon stopping takes the program with it.
    r.hub.stop_modules();
    assert!(gone(pid), "the program was left behind");
}

#[test]
fn a_task_that_ends_while_the_notch_is_closed_peeks_once() {
    let r = rig("done", claude_cfg, true);
    let mut ui = r.ui();
    {
        let mut view = Client::connect(&r.socket(), "term:claude", 80, 24).unwrap();
        read_until(&mut view, String::new(), |t| {
            t.contains("fake claude ready")
        });
        // A key was pressed (the view says so), then it works and the view goes.
        ui.send_raw(
            r#"{"t":"event","module":"claude","slot":"expanded","action":"typed","value":null}"#,
        );
        view.input(b"work\n").unwrap();
        assert!(
            ui.wait_for(WAIT, view_has("claude", Slot::Compact, "success"))
                .is_some()
        );
    }
    let peeked = ui.wait_for(WAIT, |m| {
        matches!(m, ToUi::State(s) if s.notches.iter().any(|n| n.copies.iter().any(|c|
            c.mode == Mode::Peek && c.peek.as_ref().is_some_and(|p| p.module == "claude"))))
    });
    assert!(peeked.is_some(), "no peek when it finished");
    assert!(
        ui.wait_for(WAIT, view_has("claude", Slot::Peek, "Claude is done"))
            .is_some()
    );
}

#[test]
fn ask_opens_the_notch_and_types_the_question_and_new_starts_another() {
    let r = rig("ask", claude_cfg, true);
    let mut ui = r.ui();
    assert_eq!(
        r.ipc(&mut ui, "claude", &["ask why is the sky blue?"]),
        "ok"
    );
    let a = r.wait_args(&r.claude_args, |a| {
        a.iter().any(|l| l == "got why is the sky blue?")
    });
    assert!(
        a.iter().any(|l| l == "got why is the sky blue?"),
        "the question did not reach it: {a:?}"
    );
    // The notch was opened.
    assert!(ui
        .wait_for(WAIT, |m| matches!(m, ToUi::State(s) if s.notches.iter().any(|n| n.copies.iter().any(|c| c.mode == Mode::Expanded))))
        .is_some());
    let pid = a[2].clone();
    assert_eq!(r.ipc(&mut ui, "agent", &["claude", "new"]), "ok");
    let a = r.wait_args(&r.claude_args, |a| a.len() >= 3 && a[2] != pid);
    assert_ne!(a[2], pid, "new did not start another claude");
    assert!(gone(pid.parse().unwrap()), "the old one is still running");
    // The same words through `agent`, and the answers to what is wrong.
    assert_eq!(
        r.ipc(&mut ui, "agent", &["claude", "nonsense"]),
        "usage: agent claude toggle | new | \"ask <question>\""
    );
    assert_eq!(
        r.ipc(&mut ui, "agent", &["media", "toggle"]),
        "no agent media; agents: claude, codex"
    );
    assert_eq!(
        r.ipc(&mut ui, "codex", &["toggle"]),
        "codex module not loaded"
    );
    assert_eq!(
        r.ipc(&mut ui, "terminal", &["nope", "toggle"]),
        "no terminal nope"
    );
}

#[test]
fn ask_needs_a_ui_that_can_draw_a_terminal() {
    let r = rig("noterm", claude_cfg, true);
    // A UI without the widget (no `surfaces`).
    let mut ui = FakeUi::connect_hello(&r.socket());
    assert_eq!(
        r.ipc(&mut ui, "claude", &["ask hi"]),
        "the terminal needs the qmltermwidget package"
    );
    assert!(r.args(&r.claude_args).is_empty(), "it started anyway");
    // toggle needs nothing of the UI.
    assert_eq!(r.ipc(&mut ui, "claude", &["toggle"]), "ok");
}

#[test]
fn toggle_opens_the_notch_and_closes_it_while_a_view_shows_the_agent() {
    let r = rig("toggle", claude_cfg, true);
    let mut ui = r.ui();
    let open = |s: &notchd_proto::state::State| {
        s.notches
            .iter()
            .any(|n| n.copies.iter().any(|c| c.mode == Mode::Expanded))
    };
    assert_eq!(r.ipc(&mut ui, "claude", &["toggle"]), "ok");
    assert!(
        ui.wait_for(WAIT, |m| matches!(m, ToUi::State(s) if open(s)))
            .is_some()
    );
    let _view = Client::connect(&r.socket(), "term:claude", 80, 24).unwrap();
    // The module hears of the view before the next command is read.
    let n = ui.messages.len();
    assert_eq!(r.ipc(&mut ui, "claude", &["toggle"]), "ok");
    let closed = ui.wait_for(WAIT, |m| matches!(m, ToUi::State(s) if !open(s)));
    assert!(
        closed.is_some(),
        "toggle did not close it ({n} messages before)"
    );
}

#[test]
fn a_terminal_of_your_own_runs_its_command_in_its_folder_and_has_a_pty() {
    let r = rig(
        "own",
        |r| {
            let pwd = r.scratch.join("pwd");
            json!({"modules": ["mine"],
                "terminals": {"mine": {"title": "Mine", "command":
                    format!("printf '\\033]0;Own title\\007'; pwd > {}; stty size; exec cat", pwd.display())}}})
        },
        true,
    );
    let mut ui = r.ui();
    assert!(
        ui.wait_for(WAIT, view_has("mine", Slot::Compact, "Mine"))
            .is_some()
    );
    let mut view = Client::connect(&r.socket(), "term:mine", 90, 20).unwrap();
    let text = read_until(&mut view, String::new(), |t| t.contains("20 90"));
    assert!(
        text.contains("20 90"),
        "the program did not start at the view's size: {text:?}"
    );
    assert!(
        ui.wait_for(WAIT, view_has("mine", Slot::Compact, "Own title"))
            .is_some()
    );
    // In the home (no folder of its own), not notchd's.
    let pwd = std::fs::read_to_string(r.scratch.join("pwd")).unwrap();
    assert_eq!(pwd.trim(), r.scratch.join("home").display().to_string());
    // Typed text comes back (cat), and a resize reaches the program.
    view.input(b"ping\n").unwrap();
    let text = read_until(&mut view, text, |t| t.matches("ping").count() >= 2);
    assert!(text.matches("ping").count() >= 2, "{text:?}");
    assert_eq!(
        r.ipc(&mut ui, "terminal", &["mine", "usage?"]),
        "usage: terminal mine toggle | new | \"ask <question>\""
    );
    assert_eq!(
        r.ipc(&mut ui, "terminal", &["nope", "toggle"]),
        "no terminal nope; terminals: mine"
    );
    assert_eq!(
        r.ipc(&mut ui, "agent", &["mine", "toggle"]),
        "no agent mine; agents: claude, codex"
    );
    // Removed from the config, its program stops.
    let mut v2 = Client::connect(&r.socket(), "term:mine", 90, 20).unwrap();
    v2.resize(100, 33).unwrap();
    r.hub
        .update(|s, _| s.set_value("modules", Some(&json!(["activities"]))));
    let limit = Instant::now() + WAIT;
    let mut closed = false;
    while !closed && Instant::now() < limit {
        closed = matches!(v2.next(Duration::from_millis(100)), Ok(Some(Event::Close)));
    }
    assert!(closed, "the view stayed after its terminal left the config");
}

#[test]
fn a_surface_that_is_not_a_loaded_terminal_is_refused() {
    let r = rig("refuse", claude_cfg, true);
    let mut c = Client::connect(&r.socket(), "term:nope", 80, 24).unwrap();
    let e = c.next(WAIT).unwrap_err();
    assert!(
        e.contains("surface") && e.contains("no terminal nope"),
        "{e}"
    );
    let mut c = Client::connect(&r.socket(), "widget:x", 80, 24).unwrap();
    assert!(c.next(WAIT).unwrap_err().contains("no surface"));
}

#[test]
fn codex_shows_its_thread_title_from_the_terminal_title() {
    let r = rig(
        "codex",
        |r| {
            json!({"modules": ["codex"], "codex": {"command": r.fake("codex"),
            "directory": r.scratch.display().to_string(), "usage": false}})
        },
        true,
    );
    let mut ui = r.ui();
    let mut view = Client::connect(&r.socket(), "term:codex", 80, 24).unwrap();
    read_until(&mut view, String::new(), |t| t.contains("fake codex ready"));
    view.input(b"work\n").unwrap();
    assert!(
        ui.wait_for(WAIT, view_has("codex", Slot::Compact, "success"))
            .is_some()
    );
    assert!(
        ui.wait_for(WAIT, view_has("codex", Slot::Compact, "Fake thread"))
            .is_some()
    );
}

#[test]
fn the_plan_usage_is_asked_with_the_login_shown_and_never_when_offline() {
    let r = rig(
        "usage",
        |r| {
            let mut c = claude_cfg(r);
            c["claude"]["usage"] = json!(true);
            c
        },
        false,
    );
    std::fs::write(
        r.scratch.join("home/claude/.credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"fake-token","expiresAt":99999999999999}}"#,
    )
    .unwrap();
    let mut ui = r.ui();
    // The module asks as soon as it loads; the answer shows in the open view
    // and in `state`.
    assert!(
        ui.wait_for(WAIT, view_has("claude", Slot::Expanded, "42 %"))
            .is_some(),
        "limits never showed"
    );
    {
        let asked = r.net.asked.lock().unwrap();
        assert_eq!(asked[0].0, "https://api.anthropic.com/api/oauth/usage");
        assert_eq!(asked[0].1["Authorization"], "Bearer fake-token");
    }
    let state: Value = serde_json::from_str(&r.ipc(&mut ui, "state", &[])).unwrap();
    let m = state["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "claude")
        .unwrap();
    assert_eq!(m["usage"]["limits"][0]["label"], "5h");
    assert_eq!(m["usage"]["stale"], false);
    // The login is never in anything the UI is sent.
    assert!(
        !ui.messages
            .iter()
            .any(|m| format!("{m:?}").contains("fake-token"))
    );
    drop(ui);
    // Offline (tests, renders): nothing is asked.
    let r2 = rig(
        "offline",
        |r| {
            let mut c = claude_cfg(r);
            c["claude"]["usage"] = json!(true);
            c
        },
        true,
    );
    let mut ui = r2.ui();
    assert!(
        ui.wait_for(WAIT, view_has("claude", Slot::Compact, "Claude"))
            .is_some()
    );
    thread::sleep(Duration::from_millis(300));
    assert!(r2.net.asked.lock().unwrap().is_empty());
}

#[test]
fn a_flood_of_output_reaches_the_view_whole_and_fast_enough() {
    let r = rig(
        "flood",
        |_| json!({"modules": ["mine"], "terminals": {"mine": {"command": "head -c 20000000 /dev/zero | tr '\\0' x; echo; echo flood-end"}}}),
        true,
    );
    let _ui = r.ui();
    let started = Instant::now();
    let mut view = Client::connect(&r.socket(), "term:mine", 80, 24).unwrap();
    let mut n = 0usize;
    let mut tail = String::new();
    let limit = Instant::now() + Duration::from_secs(60);
    while Instant::now() < limit && !tail.contains("flood-end") {
        match view.next(Duration::from_millis(200)) {
            Ok(Some(Event::Data(d))) => {
                n += d.iter().filter(|b| **b == b'x').count();
                tail = String::from_utf8_lossy(&d[d.len().saturating_sub(40)..]).into_owned();
            }
            Ok(Some(Event::Close)) => break,
            _ => {}
        }
    }
    let secs = started.elapsed().as_secs_f64();
    eprintln!(
        "NOTE: 20 MB through the surface connection in {secs:.2} s ({:.0} MB/s)",
        20.0 / secs
    );
    assert_eq!(n, 20_000_000, "bytes were lost or never came");
}

#[test]
fn closed_and_idle_the_program_stops_and_the_next_view_starts_a_new_one() {
    let r = rig(
        "idle",
        |r| {
            let mut c = claude_cfg(r);
            c["claude"]["idleMinutes"] = json!(0.01);
            c
        },
        true,
    );
    let _ui = r.ui();
    let pid = {
        let mut view = Client::connect(&r.socket(), "term:claude", 80, 24).unwrap();
        read_until(&mut view, String::new(), |t| {
            t.contains("fake claude ready")
        });
        r.wait_args(&r.claude_args, |a| a.len() >= 3)[2]
            .parse::<i32>()
            .unwrap()
    };
    // The view is gone: it idles, then stops.
    assert!(gone(pid), "the program was not stopped after idleMinutes");
    let mut view = Client::connect(&r.socket(), "term:claude", 80, 24).unwrap();
    let text = read_until(&mut view, String::new(), |t| {
        t.contains("fake claude ready")
    });
    assert!(text.contains("fake claude ready"), "{text:?}");
    let a = r.wait_args(&r.claude_args, |a| a.len() >= 3 && a[2] != pid.to_string());
    assert_ne!(
        a[2],
        pid.to_string(),
        "the next view did not start a new program"
    );
}

#[test]
fn notchd_attach_exits_promptly_on_hangup_and_term_and_the_session_stays() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    let r = rig("hangup", claude_cfg, true);
    for sig in [libc::SIGHUP, libc::SIGTERM] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_notchd"));
        cmd.args([
            "--socket",
            &r.socket().display().to_string(),
            "attach",
            "term:claude",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
        // The worst case: the parent started it with the signals ignored.
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGHUP, libc::SIG_IGN);
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
                Ok(())
            });
        }
        let mut child = cmd.spawn().expect("spawn notchd attach");
        let a = r.wait_args(&r.claude_args, |a| a.len() >= 3);
        assert!(a.len() >= 3, "the program started: {a:?}");
        let pid = a[2].parse::<i32>().unwrap();
        thread::sleep(Duration::from_millis(100));
        unsafe { libc::kill(child.id() as i32, sig) };
        let t = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            assert!(
                t.elapsed() < Duration::from_secs(1),
                "still running after signal {sig}"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert!(
            status.code() == Some(0) || status.signal().is_some(),
            "{status:?}"
        );
        thread::sleep(Duration::from_millis(200));
        assert!(alive(pid), "the session died with its view (signal {sig})");
        // A new view reaches the same session.
        let mut view = Client::connect(&r.socket(), "term:claude", 80, 24).unwrap();
        let text = read_until(&mut view, String::new(), |t| {
            t.contains("fake claude ready")
        });
        assert!(text.contains("fake claude ready"), "{text:?}");
    }
}
