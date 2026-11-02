//! The sample modules of `docs/examples/modules/`, copied into a temp config
//! folder and run by the real `notchd` binary: each one's view arrives over
//! the socket and what it does works (a click, a setting, a provider). What
//! a user copies into `~/.config/notchd/modules/` is what is tested here.
//! Hermetic (temp config, runtime and state dirs), every wait bounded.

use notchd_proto::common::Slot;
use notchd_proto::request::EventValue;
use notchd_proto::ui::Event;
use notchd_proto::{ToDaemon, ToUi};
use notchd_testkit::fake_ui::{Daemon, FakeUi, START_WAIT, WAIT};
use notchd_testkit::tempdir::TestEnv;
use std::fs;
use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_notchd");

struct Running {
    _env: TestEnv,
    _daemon: Daemon,
    ui: FakeUi,
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            // `fs::copy` keeps the executable bit, as `cp` does.
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Copies the example `name` into a config folder that lists it, and runs the
/// daemon on it.
fn run_example(name: &str, config: &str) -> Running {
    let env = TestEnv::new(name);
    env.write_config(config);
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/modules");
    let modules = env.config.parent().unwrap().join("modules");
    copy_dir(&examples.join(name), &modules.join(name));
    let path = std::env::var("PATH").unwrap_or_default();
    let daemon = Daemon::start_with(BIN.as_ref(), &env, "default", &[("PATH", &path)]);
    let ui = FakeUi::connect_hello(&daemon.socket);
    Running {
        _env: env,
        _daemon: daemon,
        ui,
    }
}

impl Running {
    /// A view of the module whose tree contains `needle`.
    fn view(&mut self, module: &str, slot: Slot, needle: &str) -> ToUi {
        let (module, want) = (module.to_string(), needle.to_string());
        let module_name = module.clone();
        self.ui
            .wait_for(START_WAIT, move |m| {
                matches!(m, ToUi::View(v) if v.module == module && v.slot == slot
                    && serde_json::to_string(&v.tree).unwrap().contains(&want))
            })
            .unwrap_or_else(|| {
                let log = self
                    ._env
                    .dir
                    .path()
                    .join(".local/state/notchd/logs")
                    .join(format!("{module_name}.log"));
                panic!(
                    "no {slot:?} view with {needle:?}; its log: {:?}; the views were {:?}",
                    fs::read_to_string(log).unwrap_or_default(),
                    self.ui
                        .messages
                        .iter()
                        .filter_map(|m| match m {
                            ToUi::View(v) => serde_json::to_string(&v.tree).ok(),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                )
            })
    }

    fn click(&mut self, module: &str, action: &str) {
        self.ui.send(&ToDaemon::Event(Event {
            module: module.into(),
            slot: Slot::Expanded,
            action: action.into(),
            value: EventValue::Null(()),
        }));
    }

    /// `ipc set <key> <json>`, the way the settings window edits a value.
    fn set(&mut self, key: &str, json: &str) {
        let id = self.ui.ipc("set", &[key, json]);
        let ok = self
            .ui
            .wait_for(WAIT, |m| matches!(m, ToUi::Ok(o) if o.id == id));
        assert!(ok.is_some(), "set {key} was refused");
    }
}

#[test]
fn counter_shows_its_count_and_the_buttons_work() {
    let mut r = run_example("counter", r#"{"modules":["counter"]}"#);
    r.view("counter", Slot::Compact, "\"text\":\"0\"");
    r.view("counter", Slot::Expanded, "Count: 0");
    r.click("counter", "add");
    r.click("counter", "add");
    r.view("counter", Slot::Expanded, "Count: 2");
    r.click("counter", "reset");
    r.view("counter", Slot::Expanded, "Count: 0");
}

#[test]
fn cpu_shows_the_resources_the_daemon_reads() {
    let mut r = run_example("cpu", r#"{"modules":["cpu"]}"#);
    // The daemon reads /proc for it only because it subscribed.
    let ToUi::View(v) = r.view("cpu", Slot::Compact, "%") else {
        unreachable!()
    };
    let text = serde_json::to_string(&v.tree).unwrap();
    assert!(text.contains("\"tabular\":true"), "{text}");
    r.view("cpu", Slot::Expanded, "RAM");
}

#[test]
fn greeting_has_a_settings_page_and_hears_an_edit() {
    let mut r = run_example("greeting", r#"{"modules":["greeting"]}"#);
    // A setting the user never touched is its default.
    r.view("greeting", Slot::Compact, "Hello, world!");

    // The schema: the fields are on the module's page, with their defaults.
    let schema = r
        .ui
        .wait_for(WAIT, |m| {
            matches!(m, ToUi::Schema(s) if s.fields.iter().any(|f| f.key.as_deref() == Some("greeting.name")))
        })
        .expect("the module's fields are in the schema");
    let ToUi::Schema(s) = schema else {
        unreachable!()
    };
    assert!(s.pages.iter().any(|p| p.id == "modules/greeting"));
    let keys: Vec<_> = s
        .fields
        .iter()
        .filter(|f| f.page == "modules/greeting")
        .filter_map(|f| f.key.clone())
        .collect();
    assert_eq!(keys, ["greeting.name", "greeting.shout", "greeting.size"]);

    // An edit reaches the module as `config`, defaults filled in.
    r.set("greeting.name", "\"Eryk\"");
    r.view("greeting", Slot::Compact, "Hello, Eryk!");
    r.set("greeting.shout", "true");
    r.view("greeting", Slot::Compact, "HELLO, ERYK!");
    r.set("greeting.size", "\"heading\"");
    r.view("greeting", Slot::Expanded, "\"size\":\"heading\"");
}
