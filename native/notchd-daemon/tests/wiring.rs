//! The daemon's gaps closed over a real socket: the schema after the
//! handshake, the visitor, the reserve a UI measures, a guest's window
//! linger, and the user-module host wired into the hub (a module's view, its
//! crash, a click, shutdown). Hermetic (temp runtime dir, config, modules and
//! logs), every wait bounded, every module process stopped on drop.

use notchd_core::store::ModuleFacts;
use notchd_daemon::hub::Hub;
use notchd_daemon::instance::Instance;
use notchd_daemon::modhost::proc::alive;
use notchd_daemon::server::Server;
use notchd_proto::common::{Edge, Mode, Slot};
use notchd_proto::request::EventValue;
use notchd_proto::state::State;
use notchd_proto::ui::{Event, Measure};
use notchd_proto::{Node, Num, ToDaemon, ToUi};
use notchd_testkit::fake_module::{install, write};
use notchd_testkit::fake_ui::{Daemon, FakeUi, START_WAIT, WAIT, run_bounded};
use notchd_testkit::tempdir::TestEnv;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_notchd");

struct Rig {
    env: TestEnv,
    hub: Arc<Hub>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        // No module process may outlive the test.
        self.hub.stop();
        self.hub.stop_modules();
    }
}

fn rig(label: &str, config: &str) -> Rig {
    let env = TestEnv::new(label);
    env.write_config(config);
    let server = Server::bind(Instance {
        name: "default".into(),
        socket: env.socket("default"),
        config: env.config.clone(),
    })
    .expect("bind");
    let hub = server.hub();
    thread::spawn(move || server.serve());
    Rig { env, hub }
}

impl Rig {
    fn modules_dir(&self) -> PathBuf {
        self.env.dir.path().join("modules")
    }

    fn start_modules(&self) {
        self.hub
            .start_modules(&self.modules_dir(), &self.env.dir.path().join("logs"));
    }

    fn ui_on(&self, screens: &[&str], focused: &str) -> FakeUi {
        let mut ui = FakeUi::connect_hello(&self.env.socket("default"));
        let list: Vec<String> = screens
            .iter()
            .enumerate()
            .map(|(i, n)| {
                format!(
                    r#"{{"name":"{n}","x":{},"y":0,"w":1000,"h":800,"scale":1}}"#,
                    i * 1000
                )
            })
            .collect();
        ui.send_raw(&format!(
            r#"{{"t":"screens","screens":[{}],"focused":"{focused}","fullscreen":[],"workspaces":{{}},"known":[],"supportsWorkspaces":false,"barEdge":null}}"#,
            list.join(",")
        ));
        ui
    }

    fn ui(&self) -> FakeUi {
        self.ui_on(&["DP-1"], "DP-1")
    }

    fn module(&self, id: &str) {
        self.hub.update(|s, _| {
            let mut m = ModuleFacts::new(id);
            m.info.priority = 50.0;
            m.info.compact = true;
            m.info.peek = true;
            m.expanded = true;
            s.set_module(m);
        });
    }
}

fn state_where(ui: &mut FakeUi, limit: Duration, pred: impl Fn(&State) -> bool) -> Option<State> {
    match ui.wait_for(limit, |m| matches!(m, ToUi::State(s) if pred(s)))? {
        ToUi::State(s) => Some(s),
        _ => None,
    }
}

fn text_of(node: &Option<Node>) -> String {
    serde_json::to_string(node).unwrap()
}

fn view_where(ui: &mut FakeUi, module: &str, slot: Slot, needle: &str) -> Option<ToUi> {
    ui.wait_for(START_WAIT, |m| {
        matches!(m, ToUi::View(v) if v.module == module && v.slot == slot
            && text_of(&v.tree).contains(needle))
    })
}

// ------------------------------------------------------------------ schema

#[test]
fn the_schema_arrives_after_the_config_and_before_the_state() {
    let r = rig("schema", r#"{"hoverOpenDelay":250}"#);
    let mut ui = r.ui();
    assert!(ui.wait_for(WAIT, |m| matches!(m, ToUi::State(_))).is_some());
    let at = |ui: &FakeUi, f: fn(&ToUi) -> bool| ui.messages.iter().position(f).unwrap();
    let (welcome, config, schema, state) = (
        at(&ui, |m| matches!(m, ToUi::Welcome(_))),
        at(&ui, |m| matches!(m, ToUi::Config(_))),
        at(&ui, |m| matches!(m, ToUi::Schema(_))),
        at(&ui, |m| matches!(m, ToUi::State(_))),
    );
    assert!(welcome < config && config < schema && schema < state);
    let ToUi::Schema(s) = &ui.messages[schema] else {
        unreachable!()
    };
    assert!(s.pages.iter().any(|p| p.id == "general"));
    let f = s
        .fields
        .iter()
        .find(|f| f.key.as_deref() == Some("hoverOpenDelay"))
        .expect("a field of the general page");
    assert_eq!(f.value, 250);
}

#[test]
fn a_change_of_the_config_resends_the_schema_with_the_new_value() {
    let r = rig("schema2", "{}");
    let mut ui = r.ui();
    ui.send(&ToDaemon::Set(notchd_proto::request::Set {
        id: 1,
        key: "hoverOpenDelay".into(),
        value: serde_json::json!(300),
    }));
    let got = ui.wait_for(WAIT, |m| {
        matches!(m, ToUi::Schema(s) if s.fields.iter()
            .any(|f| f.key.as_deref() == Some("hoverOpenDelay") && f.value == 300))
    });
    assert!(got.is_some());
}

// ----------------------------------------------------------------- visitor

#[test]
fn the_visitor_peeks_when_a_peek_goes_to_a_screen_no_notch_is_on() {
    let r = rig(
        "visitor",
        r#"{"modules":["media"],"position":{"screen":"DP-2"}}"#,
    );
    r.module("media");
    let mut ui = r.ui_on(&["DP-1", "DP-2"], "DP-1");
    assert!(
        state_where(&mut ui, WAIT, |s| s.visitor.len() == 2).is_some(),
        "a visitor copy per screen"
    );
    r.hub
        .update(|s, now| s.request_peek("media", Some(2000.0), now));
    let st = state_where(&mut ui, WAIT, |s| {
        s.visitor.iter().any(|c| c.mode == Mode::Peek)
    })
    .expect("the visitor peeks");
    let c = st.visitor.iter().find(|c| c.mode == Mode::Peek).unwrap();
    assert_eq!(c.screen, "DP-1", "on the focused screen");
    assert_eq!(c.peek.as_ref().unwrap().module, "media");
    assert!(c.needs_window);
    let id = ui.ipc("state", &[]);
    let ToUi::Ok(ok) = ui
        .wait_for(WAIT, |m| matches!(m, ToUi::Ok(o) if o.id == id))
        .unwrap()
    else {
        unreachable!()
    };
    let json: serde_json::Value = serde_json::from_str(&ok.result.unwrap()).unwrap();
    assert_eq!(
        json["visitor"][0]["peek"], "media",
        "`ipc state` has it too"
    );
}

// ------------------------------------------------------------------ linger

#[test]
fn a_guests_window_lingers_after_the_peek_then_drops() {
    let r = rig(
        "linger",
        r#"{"modules":["media"],"position":{"screen":"DP-2"}}"#,
    );
    r.module("media");
    r.hub.update(|s, _| s.set_window_linger(400));
    let mut ui = r.ui_on(&["DP-1", "DP-2"], "DP-1");
    state_where(&mut ui, WAIT, |s| s.visitor.len() == 2).expect("state");
    r.hub
        .update(|s, now| s.request_peek("media", Some(400.0), now));
    let lingering = state_where(&mut ui, WAIT, |s| {
        s.visitor.iter().all(|c| c.mode == Mode::Rest) && s.visitor.iter().any(|c| c.needs_window)
    })
    .expect("closed, the window stays");
    let closed = Instant::now();
    state_where(&mut ui, WAIT, |s| {
        s.rev > lingering.rev && s.visitor.iter().all(|c| !c.needs_window)
    })
    .expect("the window drops after the linger");
    assert!(
        closed.elapsed() >= Duration::from_millis(300),
        "it lingered ({:?})",
        closed.elapsed()
    );
}

// ----------------------------------------------------------------- reserve

#[test]
fn a_measured_depth_becomes_the_reserve_and_a_shrink_waits() {
    let r = rig("reserve", r#"{"modules":["media"],"reserveSpace":true}"#);
    r.module("media");
    let mut ui = r.ui();
    let measure = |baseline: f64| {
        ToDaemon::Measure(Measure {
            notch: "main".into(),
            screen: "".into(),
            rest: Num(6.0),
            compact: Num(0.0),
            baseline: Num(baseline),
        })
    };
    state_where(&mut ui, WAIT, |s| s.notches.len() == 1).expect("state");
    assert!(
        ui.messages
            .iter()
            .all(|m| !matches!(m, ToUi::State(s) if !s.reserve.is_empty())),
        "nothing before the UI measures"
    );
    ui.send(&measure(30.0));
    let st = state_where(&mut ui, WAIT, |s| !s.reserve.is_empty()).expect("reserve");
    let res = &st.reserve[0];
    assert_eq!(
        (res.screen.as_str(), res.edge, res.depth.0),
        ("DP-1", Edge::Top, 30.0)
    );
    let sent = Instant::now();
    ui.send(&measure(20.0));
    state_where(&mut ui, WAIT, |s| {
        s.reserve.first().is_some_and(|r| r.depth.0 == 20.0)
    })
    .expect("shrinks");
    assert!(
        sent.elapsed() >= Duration::from_millis(300),
        "after the animation ({:?})",
        sent.elapsed()
    );
}

// ------------------------------------------------------------ user modules

const PEEKER: &str = r#"#!/bin/sh
read -r init
printf '{"t":"set","active":true}\n'
printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"c"}}\n'
printf '{"t":"view","slot":"peek","tree":{"t":"text","text":"p"}}\n'
printf '{"t":"peek","ms":900}\n'
while read -r line; do
  case "$line" in *'"t":"stop"'*) exit 0 ;; esac
done
"#;

#[test]
fn a_user_modules_view_and_facts_reach_the_ui_and_the_notch() {
    let r = rig("modview", r#"{"modules":["good"]}"#);
    install(&r.modules_dir(), "good", "good");
    r.start_modules();
    let mut ui = r.ui();
    let v = view_where(&mut ui, "good", Slot::Compact, "hello").expect("the module's view");
    let ToUi::View(v) = v else { unreachable!() };
    assert!(v.rev >= 1);
    let m = ui
        .wait_for(
            START_WAIT,
            |m| matches!(m, ToUi::Module(m) if m.id == "good" && m.active),
        )
        .expect("its facts");
    let ToUi::Module(m) = m else { unreachable!() };
    assert_eq!((m.title.as_str(), m.priority, m.error), ("Good", 40, None));
    // The facts are the notch's: it lists the module and shows it compact.
    let st = state_where(&mut ui, START_WAIT, |s| {
        s.notches.first().is_some_and(|n| {
            n.modules == ["good"] && n.copies.first().is_some_and(|c| c.mode == Mode::Compact)
        })
    });
    assert!(st.is_some(), "the module's compact view shows in the notch");
}

#[test]
fn a_user_modules_settings_page_is_in_the_schema() {
    let r = rig(
        "modschema",
        r#"{"modules":["weather"],"weather":{"city":"Paris"}}"#,
    );
    write(
        &r.modules_dir(),
        "weather",
        r#"{"title":"Weather","command":["./run.sh"],
            "settings":[{"key":"city","type":"string","label":"City","default":"Berlin"}]}"#,
        "#!/bin/sh\nread -r init\nwhile read -r l; do :; done\n",
    );
    r.start_modules();
    let mut ui = r.ui();
    let got = ui
        .wait_for(START_WAIT, |m| {
            matches!(m, ToUi::Schema(s) if s.fields.iter()
                .any(|f| f.key.as_deref() == Some("weather.city") && f.value == "Paris"
                    && f.page == "modules/weather"))
        })
        .is_some();
    assert!(
        got,
        "the field is on the module's page with the config's value"
    );
}

#[test]
fn a_module_that_peeks_peeks_the_notch() {
    let r = rig("modpeek", r#"{"modules":["pk"]}"#);
    write(
        &r.modules_dir(),
        "pk",
        r#"{"title":"Pk","command":["./run.sh"]}"#,
        PEEKER,
    );
    r.start_modules();
    let mut ui = r.ui();
    let st = state_where(&mut ui, START_WAIT, |s| {
        s.notches
            .first()
            .and_then(|n| n.copies.first())
            .is_some_and(|c| c.peek.as_ref().is_some_and(|p| p.module == "pk"))
    })
    .expect("the module's peek reaches the notch's state");
    assert_eq!(st.notches[0].copies[0].mode, Mode::Peek);
}

#[test]
fn a_crashed_module_shows_an_error_row_in_its_slots() {
    let r = rig("modcrash", r#"{"modules":["crash"]}"#);
    install(&r.modules_dir(), "crash", "crash");
    r.start_modules();
    let mut ui = r.ui();
    let v = view_where(&mut ui, "crash", Slot::Compact, "crash exited").expect("an error row");
    let ToUi::View(v) = v else { unreachable!() };
    let row = text_of(&v.tree);
    assert!(row.contains("crash.log"), "with the log path: {row}");
    assert!(row.contains("restart"), "and the Restart button");
    let m = ui
        .wait_for(
            START_WAIT,
            |m| matches!(m, ToUi::Module(m) if m.id == "crash" && m.error.is_some()),
        )
        .expect("the module's error");
    let ToUi::Module(m) = m else { unreachable!() };
    assert!(m.attention);
}

#[test]
fn a_click_in_a_modules_view_reaches_the_module() {
    let r = rig("modclick", r#"{"modules":["good"]}"#);
    install(&r.modules_dir(), "good", "good");
    r.start_modules();
    let mut ui = r.ui();
    view_where(&mut ui, "good", Slot::Compact, "hello").expect("running");
    ui.send(&ToDaemon::Event(Event {
        module: "good".into(),
        slot: Slot::Expanded,
        action: "toggle".into(),
        value: EventValue::Null(()),
    }));
    // The fixture answers any event with an expanded view.
    assert!(view_where(&mut ui, "good", Slot::Expanded, "clicked").is_some());
}

#[test]
fn a_module_not_listed_by_a_notch_is_stopped_and_a_listed_one_runs() {
    let r = rig("modlisted", r#"{"modules":["media"]}"#);
    install(&r.modules_dir(), "good", "good");
    r.start_modules();
    let mut ui = r.ui();
    // Not listed: whatever it showed while it started, it ends stopped.
    let deadline = Instant::now() + START_WAIT;
    loop {
        let status = r.hub_status("good");
        if status == Some(false) {
            break;
        }
        assert!(Instant::now() < deadline, "never stopped: {status:?}");
        thread::sleep(Duration::from_millis(20));
    }
    ui.send(&ToDaemon::Set(notchd_proto::request::Set {
        id: 1,
        key: "modules".into(),
        value: serde_json::json!(["media", "good"]),
    }));
    assert!(view_where(&mut ui, "good", Slot::Compact, "hello").is_some());
}

impl Rig {
    /// Whether the module's process runs, by its log (`pid N`).
    fn hub_status(&self, id: &str) -> Option<bool> {
        let log = self.env.dir.path().join("logs").join(format!("{id}.log"));
        let text = std::fs::read_to_string(log).ok()?;
        let pid: u32 = text
            .lines()
            .rev()
            .find_map(|l| l.split("pid ").nth(1)?.trim().parse().ok())?;
        Some(alive(pid))
    }
}

// ---------------------------------------------------------------- shutdown

#[test]
fn shutdown_stops_the_module_processes_before_the_daemon_exits() {
    let env = TestEnv::new("shut");
    env.write_config(r#"{"modules":["pidw"]}"#);
    let modules = env.dir.path().join("modules");
    write(
        &modules,
        "pidw",
        r#"{"title":"Pidw","command":["./run.sh"]}"#,
        "#!/bin/sh\necho $$ > pid\nread -r init\nprintf '{\"t\":\"view\",\"slot\":\"compact\",\"tree\":{\"t\":\"text\",\"text\":\"up\"}}\\n'\nwhile read -r line; do\n  case \"$line\" in *'\"t\":\"stop\"'*) exit 0 ;; esac\ndone\n",
    );
    let daemon = Daemon::start(BIN.as_ref(), &env, "default");
    let mut ui = FakeUi::connect_hello(&daemon.socket);
    view_where(&mut ui, "pidw", Slot::Compact, "up").expect("the module runs");
    let pid: u32 = std::fs::read_to_string(modules.join("pidw/pid"))
        .expect("the module wrote its pid")
        .trim()
        .parse()
        .unwrap();
    assert!(alive(pid));
    let mut cmd = Command::new(BIN);
    env.apply(&mut cmd);
    cmd.args(["ipc", "shutdown"]);
    let (code, out, _) = run_bounded(cmd, WAIT);
    assert_eq!((code, out.trim()), (Some(0), "ok"));
    // The socket goes last: after the modules are stopped.
    let deadline = Instant::now() + START_WAIT;
    while daemon.socket.exists() {
        assert!(Instant::now() < deadline, "the daemon never exited");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(pid), "no module process outlives the daemon");
}

// ------------------------------------------------- modules of the daemon

fn answer(ui: &mut FakeUi, method: &str, args: &[&str]) -> Result<String, String> {
    let id = ui.ipc(method, args);
    match ui.wait_for(WAIT, |m| {
        matches!(m, ToUi::Ok(o) if o.id == id) || matches!(m, ToUi::Err(e) if e.id == Some(id))
    }) {
        Some(ToUi::Ok(o)) => Ok(o.result.unwrap_or_default()),
        Some(ToUi::Err(e)) => Err(e.message.unwrap_or_default()),
        other => panic!("no answer to {method}: {other:?}"),
    }
}

#[test]
fn the_timer_activities_and_osd_live_in_the_daemon_and_answer_their_ipc() {
    let r = rig(
        "core",
        r#"{"modules":["timer","activities"],"timer":{"notify":false}}"#,
    );
    let mut ui = r.ui_on(&["DP-1"], "DP-1");
    // The host that owns the OSD's IPC target declares the module.
    ui.send_raw(
        r#"{"t":"catalog","modules":[{"moduleId":"osd","title":"OSD","settings":[],"standalone":true}]}"#,
    );
    // Listed, they load: their facts and an idle timer's expanded view arrive.
    assert!(
        ui.wait_for(START_WAIT, |m| matches!(m, ToUi::Module(i)
            if i.id == "timer" && i.title == "Timer" && i.active && i.urgent_peek))
            .is_some()
    );
    assert!(view_where(&mut ui, "timer", Slot::Expanded, "5 min").is_some());
    // The osd is standalone: loaded without a notch listing it.
    assert!(
        ui.wait_for(START_WAIT, |m| matches!(m, ToUi::Module(i)
            if i.id == "osd" && i.priority == 100))
            .is_some()
    );
    // The same answers as the QML modules gave.
    assert_eq!(
        answer(&mut ui, "timer", &["pause"]).unwrap(),
        "no timer running"
    );
    assert_eq!(answer(&mut ui, "timer", &["3m Tea"]).unwrap(), "ok");
    assert!(
        view_where(&mut ui, "timer", Slot::Compact, "2:5").is_some(),
        "counting down"
    );
    let st = state_where(&mut ui, WAIT, |s| {
        s.notches[0].copies[0].mode == Mode::Peek
            && s.notches[0].copies[0]
                .peek
                .as_ref()
                .is_some_and(|p| p.module == "timer")
    });
    assert!(st.is_some(), "a start confirms with a peek");
    assert_eq!(answer(&mut ui, "push", &["nope"]).unwrap(), "invalid json");
    assert_eq!(
        answer(
            &mut ui,
            "push",
            &[r#"{"id":"d","title":"Deploy","sticky":true}"#]
        )
        .unwrap(),
        "ok"
    );
    assert!(view_where(&mut ui, "activities", Slot::Compact, "Deploy").is_some());
    assert_eq!(answer(&mut ui, "clear", &["d"]).unwrap(), "ok");
    // A button of the expanded view is an event.
    ui.send(&ToDaemon::Event(Event {
        module: "timer".into(),
        slot: Slot::Expanded,
        action: "stop".into(),
        value: EventValue::Null(()),
    }));
    assert!(
        ui.wait_for(
            START_WAIT,
            |m| matches!(m, ToUi::View(v) if v.module == "timer"
            && v.slot == Slot::Compact && v.tree.is_none())
        )
        .is_some(),
        "stopped: no compact view"
    );
    // The osd's own target: show peeks in its notch, state says open.
    assert_eq!(answer(&mut ui, "osd", &["state"]).unwrap(), "closed");
    assert_eq!(
        answer(
            &mut ui,
            "osd",
            &["show", r#"{"icon":"volume","value":"40"}"#]
        )
        .unwrap(),
        "ok"
    );
    assert!(view_where(&mut ui, "osd", Slot::Peek, "40%").is_some());
    assert_eq!(answer(&mut ui, "osd", &["state"]).unwrap(), "open");
    assert_eq!(answer(&mut ui, "osd", &["close"]).unwrap(), "ok");
    // A module no notch lists is not loaded.
    r.hub.update(|s, _| {
        s.set_value("modules", Some(&serde_json::json!(["activities"])));
    });
    assert_eq!(
        answer(&mut ui, "timer", &["5m"]).unwrap(),
        "timer module not loaded"
    );
}
