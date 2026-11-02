//! The daemon's core loop over a real socket, in process: the `Server` runs on
//! a thread so a test can feed the hub the module facts the module host will,
//! and `FakeUi` clients are the UIs. Hermetic (temp runtime dir and config),
//! every wait bounded.

use notchd_core::store::ModuleFacts;
use notchd_daemon::hub::Hub;
use notchd_daemon::instance::Instance;
use notchd_daemon::server::Server;
use notchd_proto::ToUi;
use notchd_proto::common::{Edge, Mode};
use notchd_proto::state::State;
use notchd_proto::ui::PointerEv;
use notchd_testkit::fake_ui::{FakeUi, WAIT};
use notchd_testkit::tempdir::TestEnv;
use serde_json::{Value, json};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

struct Rig {
    env: TestEnv,
    hub: Arc<Hub>,
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
    fn ui(&self) -> FakeUi {
        let mut ui = FakeUi::connect_hello(&self.env.socket("default"));
        ui.send_raw(
            r#"{"t":"screens","screens":[{"name":"DP-1","x":0,"y":0,"w":1000,"h":800,"scale":1}],"focused":"DP-1","fullscreen":[],"workspaces":{},"known":[],"supportsWorkspaces":false,"barEdge":null}"#,
        );
        ui
    }

    fn module(&self, id: &str, priority: f64, urgent: bool) {
        self.hub.update(|s, _| {
            let mut m = ModuleFacts::new(id);
            m.info.priority = priority;
            m.info.compact = true;
            m.info.peek = true;
            m.info.urgent_peek = urgent;
            m.expanded = true;
            s.set_module(m);
        });
    }

    fn peek(&self, id: &str, ms: f64) {
        self.hub.update(|s, now| s.request_peek(id, Some(ms), now));
    }
}

fn state_where(ui: &mut FakeUi, pred: impl Fn(&State) -> bool) -> Option<State> {
    match ui.wait_for(WAIT, |m| matches!(m, ToUi::State(s) if pred(s)))? {
        ToUi::State(s) => Some(s),
        _ => None,
    }
}

fn mode_is(m: Mode) -> impl Fn(&State) -> bool {
    move |s| {
        s.notches
            .first()
            .is_some_and(|n| n.copies.first().is_some_and(|c| c.mode == m))
    }
}

fn replied(ui: &mut FakeUi, id: u64) -> ToUi {
    ui.wait_for(WAIT, |m| {
        matches!(m, ToUi::Ok(o) if o.id == id) || matches!(m, ToUi::Err(e) if e.id == Some(id))
    })
    .expect("a reply")
}

const CFG: &str = r#"{"modules":["media","clock"],"hoverOpenDelay":40,"hoverCloseDelay":80}"#;

#[test]
fn hover_enter_opens_after_the_delay_and_leave_closes_it() {
    let r = rig("hover", CFG);
    r.module("media", 50.0, false);
    let mut ui = r.ui();
    assert!(state_where(&mut ui, mode_is(Mode::Compact)).is_some());
    ui.pointer("main", "DP-1", PointerEv::Enter);
    let open = state_where(&mut ui, mode_is(Mode::Expanded)).expect("expanded after the delay");
    ui.pointer("main", "DP-1", PointerEv::Leave);
    let closed =
        state_where(&mut ui, |s| s.rev > open.rev && mode_is(Mode::Compact)(s)).expect("back");
    assert!(closed.rev > open.rev);
}

#[test]
fn leaving_before_the_delay_never_opens() {
    let r = rig("quick", r#"{"hoverOpenDelay":300}"#);
    r.module("media", 50.0, false);
    let mut ui = r.ui();
    assert!(state_where(&mut ui, mode_is(Mode::Compact)).is_some());
    ui.pointer("main", "DP-1", PointerEv::Enter);
    ui.pointer("main", "DP-1", PointerEv::Leave);
    thread::sleep(Duration::from_millis(500));
    // A later request answered is the sync point: nothing came before it.
    let id = ui.ipc("state", &[]);
    replied(&mut ui, id);
    assert!(
        !ui.messages
            .iter()
            .any(|m| matches!(m, ToUi::State(s) if mode_is(Mode::Expanded)(s)))
    );
}

#[test]
fn peek_arbitration_between_two_modules() {
    let r = rig("peek", CFG);
    r.module("media", 50.0, false);
    r.module("clock", 80.0, false);
    let mut ui = r.ui();
    assert!(state_where(&mut ui, mode_is(Mode::Compact)).is_some());
    r.peek("clock", 60_000.0);
    let s = state_where(&mut ui, mode_is(Mode::Peek)).expect("clock peeks");
    assert_eq!(
        s.notches[0].copies[0].peek.as_ref().unwrap().module,
        "clock"
    );
    // media (50) cannot cut clock (80)
    r.peek("media", 60_000.0);
    // a hover drops a plain peek
    ui.pointer("main", "DP-1", PointerEv::Enter);
    let s = state_where(&mut ui, mode_is(Mode::Expanded)).expect("hover opened it");
    assert!(s.notches[0].copies[0].peek.is_none());
    assert!(
        !ui.messages.iter().any(|m| matches!(m, ToUi::State(s)
            if s.notches[0].copies[0].peek.as_ref().is_some_and(|p| p.module == "media"))),
        "the lower priority one never cut in"
    );
}

#[test]
fn a_short_peek_ends_by_itself_and_an_urgent_one_shows_over_a_hover() {
    let r = rig("short", CFG);
    r.module("media", 50.0, false);
    r.module("clock", 80.0, true);
    let mut ui = r.ui();
    assert!(state_where(&mut ui, mode_is(Mode::Compact)).is_some());
    r.peek("media", 400.0);
    assert!(state_where(&mut ui, mode_is(Mode::Peek)).is_some());
    assert!(
        state_where(&mut ui, mode_is(Mode::Compact)).is_some(),
        "the peek clock ran out"
    );
    ui.pointer("main", "DP-1", PointerEv::Enter);
    assert!(state_where(&mut ui, mode_is(Mode::Expanded)).is_some());
    r.peek("clock", 5000.0);
    assert!(
        state_where(&mut ui, mode_is(Mode::Peek)).is_some(),
        "urgent through a hover"
    );
}

#[test]
fn set_removes_the_key_and_empties_parents_on_disk_and_announces_the_config() {
    let r = rig("set", r#"{"theme":{"accent":"red"},"hoverOpenDelay":55}"#);
    let mut ui = r.ui();
    ui.send_raw(r#"{"t":"set","id":1,"key":"theme.accent","value":null}"#);
    assert!(matches!(replied(&mut ui, 1), ToUi::Ok(_)));
    let text = std::fs::read_to_string(&r.env.config).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v,
        json!({"hoverOpenDelay": 55}),
        "the empty parent is gone: {text}"
    );
    let c = ui
        .wait_for(
            WAIT,
            |m| matches!(m, ToUi::Config(c) if c.cfg.get("theme").is_none()),
        )
        .expect("config rebroadcast");
    let ToUi::Config(c) = c else { unreachable!() };
    assert!(c.rev >= 2);
    // back to the default: the key goes, and no default is written
    ui.send_raw(r#"{"t":"set","id":2,"key":"hoverOpenDelay","value":null}"#);
    assert!(matches!(replied(&mut ui, 2), ToUi::Ok(_)));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&r.env.config).unwrap()).unwrap();
    assert_eq!(v, json!({}));
    // our own write is not applied again as an edit from outside
    thread::sleep(Duration::from_millis(350));
    let revs: Vec<u64> = ui
        .messages
        .iter()
        .filter_map(|m| {
            if let ToUi::Config(c) = m {
                Some(c.rev)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(revs, [1, 2, 3], "initial and the two sets");
}

#[test]
fn ipc_set_navigate_and_state_work_over_the_socket() {
    let r = rig("ipc", CFG);
    r.module("media", 50.0, false);
    let mut ui = r.ui();
    let id = ui.ipc("set", &["hoverOpenDelay", "70"]);
    assert!(matches!(replied(&mut ui, id), ToUi::Ok(o) if o.result.as_deref() == Some("ok")));
    assert!(
        ui.wait_for(
            WAIT,
            |m| matches!(m, ToUi::Config(c) if c.cfg["hoverOpenDelay"] == 70)
        )
        .is_some()
    );
    let id = ui.ipc("navigate", &[]);
    assert!(matches!(replied(&mut ui, id), ToUi::Ok(_)));
    let s = state_where(&mut ui, |s| s.navigation.is_some()).expect("navigation");
    assert_eq!(s.navigation.unwrap().selected, 0);
    ui.key("main", "DP-1", "Escape", &[]);
    assert!(state_where(&mut ui, |s| s.navigation.is_none() && s.rev > 1).is_some());
    let id = ui.ipc("state", &[]);
    let ToUi::Ok(o) = replied(&mut ui, id) else {
        panic!()
    };
    let v: Value = serde_json::from_str(&o.result.unwrap()).unwrap();
    assert_eq!(v["notches"][0]["id"], "main");
    let id = ui.ipc("agent", &["claude", "toggle"]);
    assert!(matches!(replied(&mut ui, id), ToUi::Err(e) if e.code == "method"));
}

#[test]
fn an_edit_of_the_file_from_outside_reaches_the_ui_and_a_bad_one_keeps_the_good_config() {
    let r = rig("watch", CFG);
    r.module("media", 50.0, false);
    let mut ui = r.ui();
    assert!(state_where(&mut ui, mode_is(Mode::Compact)).is_some());
    r.env
        .write_config(r#"{"modules":["media"],"position":{"edge":"left"}}"#);
    let s = state_where(&mut ui, |s| {
        s.notches.first().is_some_and(|n| n.edge == Edge::Left)
    })
    .expect("the edit applied");
    assert_eq!(s.notches[0].modules, ["media"]);
    assert!(
        ui.wait_for(
            WAIT,
            |m| matches!(m, ToUi::Config(c) if c.cfg["position"]["edge"] == "left")
        )
        .is_some()
    );
    r.env.write_config("{ not json");
    assert!(
        ui.wait_for(WAIT, |m| matches!(m, ToUi::ConfigError(_)))
            .is_some()
    );
    assert_eq!(
        std::fs::read_to_string(&r.env.config).unwrap(),
        "{ not json",
        "the file is not touched"
    );
    r.env.write_config(r#"{"hoverOpenDelay":10}"#);
    assert!(state_where(&mut ui, |s| s.notches[0].edge == Edge::Top).is_some());
}

#[test]
fn two_uis_get_the_same_state_revs_and_a_late_one_starts_from_the_current() {
    let r = rig("two", CFG);
    r.module("media", 50.0, false);
    let mut a = r.ui();
    let mut b = r.ui();
    assert!(state_where(&mut a, mode_is(Mode::Compact)).is_some());
    assert!(state_where(&mut b, mode_is(Mode::Compact)).is_some());
    a.pointer("main", "DP-1", PointerEv::Enter);
    let open = state_where(&mut a, mode_is(Mode::Expanded)).unwrap().rev;
    assert!(state_where(&mut b, mode_is(Mode::Expanded)).is_some());
    a.pointer("main", "DP-1", PointerEv::Leave);
    assert!(state_where(&mut a, |s| s.rev > open && mode_is(Mode::Compact)(s)).is_some());
    assert!(state_where(&mut b, |s| s.rev > open && mode_is(Mode::Compact)(s)).is_some());
    let revs = |ui: &FakeUi| -> Vec<(u64, Mode)> {
        ui.messages
            .iter()
            .filter_map(|m| {
                if let ToUi::State(s) = m {
                    Some((s.rev, s.notches[0].copies[0].mode))
                } else {
                    None
                }
            })
            .collect()
    };
    let (ra, rb) = (revs(&a), revs(&b));
    assert!(ra.windows(2).all(|w| w[0].0 < w[1].0), "increasing: {ra:?}");
    for x in &ra {
        if let Some(y) = rb.iter().find(|y| y.0 == x.0) {
            assert_eq!(x, y, "one rev, one state");
        }
    }
    let common = ra.iter().filter(|x| rb.contains(x)).count();
    assert!(common >= 2, "both saw the changes: {ra:?} vs {rb:?}");
    let last = *ra.last().unwrap();
    let mut late = r.ui();
    let first = state_where(&mut late, |_| true).unwrap();
    assert_eq!(first.rev, last.0, "a late UI starts from the current rev");
}

fn facts_line(modules: &[(&str, i32)]) -> String {
    let list: Vec<Value> = modules
        .iter()
        .map(|(id, priority)| {
            json!({"id": id, "priority": priority, "active": true, "compact": true,
                   "peek": true, "icon": false, "expanded": true, "attention": false,
                   "urgentPeek": false, "peekOnFocusedScreen": false, "standalone": false})
        })
        .collect();
    json!({"t": "facts", "modules": list}).to_string()
}

#[test]
fn a_ui_reports_facts_peeks_pins_and_drags_and_the_state_follows() {
    let r = rig("inputs", CFG);
    let mut ui = r.ui();
    ui.send_raw(&facts_line(&[("media", 50), ("clock", 80)]));
    let s = state_where(&mut ui, mode_is(Mode::Compact)).expect("facts make a compact view");
    assert_eq!(s.notches[0].copies[0].compact, ["clock", "media"]);
    ui.send_raw(r#"{"t":"peek","module":"clock","ms":60000}"#);
    assert!(state_where(&mut ui, mode_is(Mode::Peek)).is_some());
    ui.send_raw(r#"{"t":"peek-dismiss","module":"clock"}"#);
    assert!(state_where(&mut ui, mode_is(Mode::Compact)).is_some());
    ui.send_raw(r#"{"t":"pin","notch":"main","screen":"","on":true}"#);
    assert!(state_where(&mut ui, mode_is(Mode::Expanded)).is_some());
    ui.send_raw(r#"{"t":"focus","notch":"main","screen":"","active":true}"#);
    assert!(
        state_where(&mut ui, |s| s.notches[0].copies[0].keyboard
            == notchd_proto::common::KeyboardFocus::OnDemand)
        .is_some()
    );
    ui.send_raw(r#"{"t":"focus","notch":"main","screen":"","active":false}"#);
    assert!(
        state_where(&mut ui, mode_is(Mode::Compact)).is_some(),
        "a window took the keyboard: closed"
    );
    ui.send_raw(r#"{"t":"module-notch","module":"clock","op":"open"}"#);
    assert!(state_where(&mut ui, mode_is(Mode::Expanded)).is_some());
    ui.send_raw(r#"{"t":"module-notch","module":"clock","op":"close"}"#);
    assert!(state_where(&mut ui, mode_is(Mode::Compact)).is_some());
    ui.send_raw(r#"{"t":"drag","notch":"main","screen":"","ev":"start"}"#);
    ui.send_raw(
        r#"{"t":"drag","notch":"main","screen":"","ev":"drop","position":{"edge":"bottom","offset":0.5,"screen":""}}"#,
    );
    let s = state_where(&mut ui, |s| s.notches[0].edge == Edge::Bottom).expect("dropped");
    assert_eq!(s.notches[0].copies[0].mode, Mode::Compact);
}
