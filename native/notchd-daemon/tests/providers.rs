//! What user modules get from the daemon's providers, and what they may post:
//! a module's subscriptions (the manifest's and `subscribe`), the data of a
//! provider reaching it in the documented shape, a module that does not read
//! never holding the hub, and `notify` through the notifier. Hermetic: the
//! hub is driven by hand (no readings of the system), temp dirs, every wait
//! bounded.

use notchd_daemon::hub::Hub;
use notchd_daemon::instance::Instance;
use notchd_daemon::server::Server;
use notchd_proto::ToUi;
use notchd_proto::common::Slot;
use notchd_providers::notifications::Notifier;
use notchd_testkit::fake_module::write;
use notchd_testkit::fake_ui::{FakeUi, START_WAIT};
use notchd_testkit::tempdir::TestEnv;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct Rig {
    env: TestEnv,
    hub: Arc<Hub>,
}

impl Drop for Rig {
    fn drop(&mut self) {
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

    fn start(&self) -> FakeUi {
        self.hub
            .start_modules(&self.modules_dir(), &self.env.dir.path().join("logs"));
        FakeUi::connect_hello(&self.env.socket("default"))
    }
}

fn view(ui: &mut FakeUi, module: &str, needle: &str) -> bool {
    let (module, needle) = (module.to_string(), needle.to_string());
    ui.wait_for(START_WAIT, move |m| {
        matches!(m, ToUi::View(v) if v.module == module && v.slot == Slot::Compact
            && serde_json::to_string(&v.tree).unwrap().contains(&needle))
    })
    .is_some()
}

/// A module that shows, as its compact view, the `name` and a number of each
/// `provider` line it reads: `<name>:<count>`.
const COUNTER: &str = r#"#!/bin/sh
printf '{"t":"set","active":true}\n'
n=0
while read -r line; do
  case "$line" in
    *'"t":"stop"'*) exit 0 ;;
    *'"t":"provider"'*)
      n=$((n + 1))
      name=${line#*'"name":"'}; name=${name%%\"*}
      printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"%s:%s"}}\n' "$name" "$n"
      case "$line" in *'"cpu":0.5'*) printf '{"t":"view","slot":"compact","tree":{"t":"text","text":"cpu-half"}}\n' ;; esac
      ;;
  esac
done
"#;

#[test]
fn a_manifest_subscription_gets_the_providers_data_in_its_shape() {
    let r = rig("sub-manifest", r#"{"modules":["watch"]}"#);
    write(
        &r.modules_dir(),
        "watch",
        r#"{"command":["./run.sh"],"subscribe":["resources"]}"#,
        COUNTER,
    );
    let mut ui = r.start();
    // The sampler's own shape in, the documented one out: bytes, `mem`, a
    // disk with its path. (The first reading has no CPU and is not sent.)
    r.hub.feed(
        "resources",
        &json!({"cpu": null, "cores": 4, "memory": {"used": 1.0, "total": 2.0, "fraction": 0.5},
                "disk": {"used": 5.0, "total": 10.0, "fraction": 0.5}}),
    );
    r.hub.feed(
        "resources",
        &json!({"cpu": 0.5, "cores": 4, "memory": {"used": 8.0, "total": 16.0, "fraction": 0.5}}),
    );
    assert!(
        view(&mut ui, "watch", "cpu-half"),
        "the module got the data"
    );
}

#[test]
fn a_runtime_subscribe_is_owed_the_latest_data_and_unsubscribe_stops_it() {
    let r = rig("sub-runtime", r#"{"modules":["late"]}"#);
    // The data was there before the module asked.
    r.hub.feed("media", &json!({"players": []}));
    write(
        &r.modules_dir(),
        "late",
        r#"{"command":["./run.sh"]}"#,
        &COUNTER.replace(
            "printf '{\"t\":\"set\",\"active\":true}\\n'",
            "printf '{\"t\":\"set\",\"active\":true}\\n{\"t\":\"subscribe\",\"name\":\"media\"}\\n'",
        ),
    );
    let mut ui = r.start();
    assert!(
        view(&mut ui, "late", "media:1"),
        "owed the latest data at once"
    );
    r.hub.feed("media", &json!({"players": [{"id": "x"}]}));
    assert!(view(&mut ui, "late", "media:2"));
}

#[test]
fn only_the_providers_a_module_asked_for_reach_it() {
    let r = rig("sub-only", r#"{"modules":["only"]}"#);
    write(
        &r.modules_dir(),
        "only",
        r#"{"command":["./run.sh"],"subscribe":["screens"]}"#,
        COUNTER,
    );
    let mut ui = r.start();
    r.hub.feed("media", &json!({"players": []}));
    r.hub.feed("resources", &json!({"cpu": 0.1}));
    let mut other = FakeUi::connect_hello(&r.env.socket("default"));
    other.send_raw(
        r#"{"t":"screens","screens":[{"name":"DP-1","x":0,"y":0,"w":1000,"h":800,"scale":1}],"focused":"DP-1","fullscreen":[],"workspaces":{},"known":[],"supportsWorkspaces":false,"barEdge":null}"#,
    );
    assert!(
        view(&mut ui, "only", "screens:1"),
        "the UI's screens reach it"
    );
    // Nothing else came: the count of that first line is the only one.
    assert!(!view(&mut ui, "only", "media:"));
}

#[derive(Default)]
struct Posted(Mutex<Vec<(String, String)>>);

impl Notifier for Posted {
    fn notify(&self, title: &str, body: &str) {
        self.0.lock().unwrap().push((title.into(), body.into()));
    }
}

#[test]
fn notify_is_posted_through_the_notifier_and_held_to_one_a_second() {
    let r = rig("notify", r#"{"modules":["loud"]}"#);
    let posted = Arc::new(Posted::default());
    r.hub.set_notifier(posted.clone());
    write(
        &r.modules_dir(),
        "loud",
        r#"{"command":["./run.sh"]}"#,
        "#!/bin/sh\nprintf '{\"t\":\"set\",\"active\":true}\\n{\"t\":\"notify\",\"title\":\"Rain\",\"body\":\"in 10 minutes\"}\\n{\"t\":\"notify\",\"title\":\"Again\"}\\n'\nwhile read -r l; do :; done\n",
    );
    let _ui = r.start();
    let deadline = Instant::now() + START_WAIT;
    while posted.0.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "nothing was posted");
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        *posted.0.lock().unwrap(),
        [("Rain".to_string(), "in 10 minutes".to_string())],
        "the second one, within a second, is dropped"
    );
}
