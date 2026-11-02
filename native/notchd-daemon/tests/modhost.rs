//! The module host against fake user modules (tests/modules/*): real
//! processes, real files in a temp dir, every wait bounded.

use notchd_daemon::modhost::proc::alive;
use notchd_daemon::modhost::{ErrorKind, ModEvent, ModHost, Options, State, error_row};
use notchd_proto::module::{LogLevel, ModuleEvent};
use notchd_proto::{EventValue, Node, Slot, ToModule};
use notchd_testkit::fake_module::{install, tick_mtime, viewer_script, write, write_script};
use notchd_testkit::tempdir::TempDir;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(8);

type Events = Arc<Mutex<Vec<(Instant, ModEvent)>>>;

struct Harness {
    _dir: TempDir,
    modules: PathBuf,
    logs: PathBuf,
    events: Events,
    host: ModHost,
}

fn fast() -> impl Fn(&mut Options) {
    |o: &mut Options| {
        o.poll_interval = Duration::from_millis(40);
        o.debounce = Duration::from_millis(40);
        o.term_after = Duration::from_millis(150);
        o.kill_after = Duration::from_millis(500);
        o.backoff_base = Duration::from_millis(100);
        o.backoff_max = Duration::from_millis(400);
        o.rapid_crash_limit = 4;
        o.first_view_timeout = Duration::from_millis(400);
    }
}

impl Harness {
    fn new(tweak: impl Fn(&mut Options)) -> Harness {
        let dir = TempDir::new("mh");
        let modules = dir.path().join("config/modules");
        let logs = dir.path().join("state/logs");
        fs::create_dir_all(&modules).unwrap();
        let mut opts = Options::for_dir(&modules);
        fast()(&mut opts);
        tweak(&mut opts);
        let events: Events = Arc::default();
        let sink = events.clone();
        let host = ModHost::with_options(&modules, &logs, opts, move |e| {
            sink.lock().unwrap().push((Instant::now(), e));
        });
        Harness {
            _dir: dir,
            modules,
            logs,
            events,
            host,
        }
    }

    /// The first event after `from` (an index into the log) that matches.
    fn wait_from(
        &self,
        from: usize,
        what: &str,
        pred: impl Fn(&ModEvent) -> bool,
    ) -> (usize, ModEvent) {
        let deadline = Instant::now() + WAIT;
        loop {
            {
                let events = self.events.lock().unwrap();
                if let Some(i) = (from..events.len()).find(|i| pred(&events[*i].1)) {
                    return (i + 1, events[i].1.clone());
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; got {:#?}",
                self.events
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|e| &e.1)
                    .collect::<Vec<_>>()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait(&self, what: &str, pred: impl Fn(&ModEvent) -> bool) -> ModEvent {
        self.wait_from(0, what, pred).1
    }

    fn count(&self, pred: impl Fn(&ModEvent) -> bool) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| pred(&e.1))
            .count()
    }

    fn mark(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    fn log_text(&self, id: &str) -> String {
        fs::read_to_string(self.logs.join(format!("{id}.log"))).unwrap_or_default()
    }

    fn wait_log(&self, id: &str, needle: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let text = self.log_text(id);
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "log of {id} never had {needle:?}: {text}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_dead(&self, pid: u32) {
        let deadline = Instant::now() + WAIT;
        while alive(pid) {
            assert!(Instant::now() < deadline, "pid {pid} is still alive");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn view_text(tree: &Option<Node>) -> String {
    serde_json::to_string(tree).unwrap()
}

fn is_view(module: &'static str, text: &'static str) -> impl Fn(&ModEvent) -> bool {
    move |e| matches!(e, ModEvent::View { module: m, tree, .. } if m == module && view_text(tree).contains(text))
}

fn is_error(module: &'static str, kind: ErrorKind) -> impl Fn(&ModEvent) -> bool {
    move |e| matches!(e, ModEvent::Error { module: m, kind: k, .. } if m == module && *k == kind)
}

fn started(module: &'static str) -> impl Fn(&ModEvent) -> bool {
    move |e| matches!(e, ModEvent::Started { module: m, .. } if m == module)
}

fn click() -> ToModule {
    ToModule::Event(ModuleEvent {
        slot: Slot::Compact,
        action: "go".into(),
        value: EventValue::Null(()),
    })
}

#[test]
fn a_good_module_gets_init_and_its_view_reaches_the_sink() {
    let h = Harness::new(|_| {});
    install(&h.modules, "good", "good");
    h.host.rescan();

    let loaded = h.wait(
        "loaded",
        |e| matches!(e, ModEvent::Loaded { module, .. } if module == "good"),
    );
    let ModEvent::Loaded { manifest, .. } = loaded else {
        unreachable!()
    };
    assert_eq!(manifest.priority, Some(40));
    h.wait("started", started("good"));
    h.wait("view", is_view("good", "hello"));
    let set = h.wait("set", |e| matches!(e, ModEvent::Set { .. }));
    assert!(
        matches!(set, ModEvent::Set { set, .. } if set.active == Some(true) && set.priority == Some(40))
    );
    let log = h.wait(
        "log",
        |e| matches!(e, ModEvent::Log { text, .. } if text.starts_with("pid ")),
    );
    assert!(matches!(
        log,
        ModEvent::Log {
            level: LogLevel::Info,
            ..
        }
    ));

    // `init` went to its stdin (the module echoes it to stderr, so to the log).
    let text = h.wait_log("good", "\"t\":\"init\"");
    assert!(
        text.contains("\"proto\":1") && text.contains("\"id\":\"good\""),
        "{text}"
    );
    assert!(
        text.contains("\"compact\"") && text.contains("\"foreground\""),
        "{text}"
    );

    // Events and config go in, views come out.
    h.host.send("good", click());
    h.wait("expanded view", is_view("good", "clicked"));
    h.host
        .set_config("good", serde_json::json!({"city": "Paris"}));
    h.wait(
        "config answered",
        |e| matches!(e, ModEvent::Log { text, .. } if text == "got config"),
    );

    let status = h.host.status("good").unwrap();
    assert_eq!(status.state, State::Running);
    assert_eq!(status.log_path, h.logs.join("good.log"));
}

#[test]
fn a_crash_shows_an_error_row_with_the_exit_code_and_the_log_path() {
    let h = Harness::new(|_| {});
    install(&h.modules, "crash", "crash");
    h.host.rescan();

    let ev = h.wait("crash row", is_error("crash", ErrorKind::Crash));
    let ModEvent::Error { text, log_path, .. } = ev else {
        unreachable!()
    };
    assert_eq!(text, "crash exited (3)");
    assert_eq!(log_path, h.logs.join("crash.log"));
    let row = error_row(&text, &log_path);
    assert!(
        serde_json::to_string(&row)
            .unwrap()
            .contains(&format!("crash exited (3)  {}", log_path.display()))
    );

    // Its stderr is in the log.
    let log = h.wait_log("crash", "boom: something broke");
    assert!(log.contains("crash exited (3)"), "{log}");
}

#[test]
fn restarts_back_off_and_stop_after_rapid_crashes_until_a_save() {
    let h = Harness::new(|_| {});
    let dir = install(&h.modules, "crash", "crash");
    h.host.rescan();

    // rapid_crash_limit is 4: four starts, gaps of at least 100, 200, 400 ms.
    let mut from = 0;
    let mut starts = Vec::new();
    for _ in 0..4 {
        let (next, _) = h.wait_from(from, "a start", started("crash"));
        from = next;
        starts.push(h.events.lock().unwrap()[next - 1].0);
    }
    let gaps: Vec<u128> = starts
        .windows(2)
        .map(|w| (w[1] - w[0]).as_millis())
        .collect();
    assert!(
        gaps[0] >= 100 && gaps[1] >= 200 && gaps[2] >= 400,
        "{gaps:?}"
    );
    assert!(gaps.iter().all(|g| *g < 1500), "{gaps:?}");

    let parked = h.wait("parked", |e| matches!(e, ModEvent::Error { text, .. } if text.contains("stopped restarting after 4 crashes")));
    assert!(matches!(
        parked,
        ModEvent::Error {
            kind: ErrorKind::Crash,
            ..
        }
    ));
    let status = h.host.status("crash").unwrap();
    assert_eq!(status.state, State::Parked);
    assert_eq!(status.crashes, 4);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(h.count(started("crash")), 4, "parked: no more starts");

    // A save starts it again with a clean slate.
    tick_mtime();
    write_script(&dir, "#!/bin/sh\nread -r init\nexit 7\n");
    let m = h.mark();
    h.wait_from(m, "restart after save", started("crash"));
    let ev = h
        .wait_from(
            m,
            "new crash",
            |e| matches!(e, ModEvent::Error { text, .. } if text == "crash exited (7)"),
        )
        .1;
    assert!(matches!(ev, ModEvent::Error { .. }));
    assert_eq!(h.host.status("crash").unwrap().crashes, 1);
}

#[test]
fn the_restart_button_starts_a_parked_module_at_once() {
    let h = Harness::new(|o| o.rapid_crash_limit = 1);
    install(&h.modules, "crash", "crash");
    h.host.rescan();
    h.wait(
        "parked",
        |e| matches!(e, ModEvent::Error { text, .. } if text.contains("stopped restarting")),
    );
    let m = h.mark();
    h.host.restart("crash");
    h.wait_from(m, "restarted", started("crash"));
}

#[test]
fn saving_a_file_restarts_the_module_and_the_old_process_is_gone() {
    let h = Harness::new(|_| {});
    let dir = write(
        &h.modules,
        "ticker",
        "{\"command\":[\"./run.sh\"]}",
        &viewer_script("one"),
    );
    h.host.rescan();
    h.wait("first view", is_view("ticker", "one"));
    let ModEvent::Started { pid: old, .. } = h.wait("start", started("ticker")) else {
        unreachable!()
    };

    tick_mtime();
    write_script(&dir, &viewer_script("two"));
    h.wait("second view", is_view("ticker", "two"));
    h.wait_dead(old);
    assert_eq!(h.count(started("ticker")), 2);
    let status = h.host.status("ticker").unwrap();
    assert_eq!(status.state, State::Running);
    assert_ne!(status.pid, Some(old));
    // A graceful stop: no crash row for the old one.
    assert_eq!(h.count(is_error("ticker", ErrorKind::Crash)), 0);
}

#[test]
fn a_new_folder_becomes_a_module_and_a_deleted_one_is_stopped() {
    let h = Harness::new(|_| {});
    h.host.rescan();
    assert!(h.host.modules().is_empty());

    let dir = write(
        &h.modules,
        "late",
        "{\"command\":[\"./run.sh\"]}",
        &viewer_script("late"),
    );
    h.wait("new module's view", is_view("late", "late"));
    assert_eq!(h.host.modules(), vec!["late".to_string()]);
    let ModEvent::Started { pid, .. } = h.wait("start", started("late")) else {
        unreachable!()
    };

    fs::remove_dir_all(&dir).unwrap();
    h.wait(
        "removed",
        |e| matches!(e, ModEvent::Removed { module } if module == "late"),
    );
    h.wait_dead(pid);
    h.host.rescan();
    assert!(h.host.modules().is_empty());
}

#[test]
fn an_invalid_tree_is_an_error_row_and_the_module_keeps_running() {
    let h = Harness::new(|_| {});
    install(&h.modules, "badtree", "badtree");
    h.host.rescan();

    let ev = h.wait("invalid view", is_error("badtree", ErrorKind::InvalidView));
    let ModEvent::Error { text, .. } = ev else {
        unreachable!()
    };
    assert!(
        text.contains("invalid view") && text.contains("compact"),
        "{text}"
    );
    assert_eq!(
        h.count(|e| matches!(e, ModEvent::View { .. })),
        0,
        "the bad view is not forwarded"
    );
    assert_eq!(h.host.status("badtree").unwrap().state, State::Running);

    h.host.send("badtree", click());
    h.wait("fixed view", is_view("badtree", "fixed"));
    h.wait(
        "error cleared",
        |e| matches!(e, ModEvent::ErrorCleared { module } if module == "badtree"),
    );
}

#[test]
fn malformed_lines_are_logged_and_skipped_and_the_module_keeps_running() {
    let h = Harness::new(|_| {});
    install(&h.modules, "malformed", "malformed");
    h.host.rescan();

    h.wait("view after the noise", is_view("malformed", "still alive"));
    let warns = h.count(|e| matches!(e, ModEvent::Log { level: LogLevel::Warn, text, .. } if text.starts_with("bad line")));
    assert_eq!(
        warns, 3,
        "not JSON, cut-off, wrong slot; the unknown `t` is ignored"
    );
    let log = h.log_text("malformed");
    assert!(
        log.contains("not JSON") && log.contains("this is not json"),
        "{log}"
    );
    assert_eq!(h.count(is_error("malformed", ErrorKind::Crash)), 0);
    assert_eq!(h.host.status("malformed").unwrap().state, State::Running);
}

#[test]
fn ten_bad_lines_in_a_row_kill_the_module_as_a_crash() {
    let h = Harness::new(|_| {});
    install(&h.modules, "garbage", "garbage");
    h.host.rescan();
    let ev = h.wait("crash", is_error("garbage", ErrorKind::Crash));
    let ModEvent::Error { text, .. } = ev else {
        unreachable!()
    };
    assert!(text.contains("10 bad lines in a row"), "{text}");
}

#[test]
fn a_line_over_one_mib_is_skipped_without_buffering_it() {
    let h = Harness::new(|_| {});
    let script = "#!/bin/sh\nread -r init\nhead -c 1200000 /dev/zero | tr '\\0' 'x'\necho\nprintf '{\"t\":\"view\",\"slot\":\"compact\",\"tree\":{\"t\":\"text\",\"text\":\"after\"}}\\n'\nwhile read -r line; do :; done\n";
    write(&h.modules, "big", "{\"command\":[\"./run.sh\"]}", script);
    h.host.rescan();
    h.wait("view after the long line", is_view("big", "after"));
    h.wait(
        "warning",
        |e| matches!(e, ModEvent::Log { text, .. } if text.contains("over the 1048576 limit")),
    );
}

#[test]
fn a_bad_manifest_is_an_error_row_and_no_process() {
    let h = Harness::new(|_| {});
    let dir = install(&h.modules, "badmanifest", "badmanifest");
    h.host.rescan();

    let ev = h.wait(
        "manifest error",
        is_error("badmanifest", ErrorKind::Manifest),
    );
    let ModEvent::Error { text, log_path, .. } = ev else {
        unreachable!()
    };
    assert!(
        text.contains("module.json") && text.contains("line 4"),
        "{text}"
    );
    assert_eq!(log_path, h.logs.join("badmanifest.log"));
    assert_eq!(h.count(started("badmanifest")), 0);
    assert!(matches!(
        h.host.status("badmanifest").unwrap().state,
        State::Invalid(_)
    ));

    // Fixing it starts the module and clears the row.
    tick_mtime();
    fs::write(dir.join("module.json"), "{\"command\":[\"./run.sh\"]}").unwrap();
    h.wait("started once fixed", started("badmanifest"));
    h.wait(
        "cleared",
        |e| matches!(e, ModEvent::ErrorCleared { module } if module == "badmanifest"),
    );
}

#[test]
fn a_manifest_typo_leaves_the_running_process_alone() {
    let h = Harness::new(|_| {});
    let dir = install(&h.modules, "good", "good");
    h.host.rescan();
    h.wait("view", is_view("good", "hello"));
    let pid = h.host.status("good").unwrap().pid.unwrap();

    tick_mtime();
    fs::write(dir.join("module.json"), "{ \"command\": [").unwrap();
    h.wait("manifest error", is_error("good", ErrorKind::Manifest));
    std::thread::sleep(Duration::from_millis(300));
    let status = h.host.status("good").unwrap();
    assert_eq!((status.state, status.pid), (State::Running, Some(pid)));
    assert_eq!(h.count(started("good")), 1);
}

#[test]
fn a_module_with_no_first_view_gets_a_row_until_it_arrives() {
    let h = Harness::new(|_| {});
    install(&h.modules, "slow", "slow");
    h.host.rescan();
    let ev = h.wait("no view row", is_error("slow", ErrorKind::NoView));
    let ModEvent::Error { text, log_path, .. } = ev else {
        unreachable!()
    };
    assert!(text.starts_with("slow: no view after "), "{text}");
    assert!(
        text.ends_with(&format!("log: {}", log_path.display())),
        "{text}"
    );
    h.wait("view", is_view("slow", "finally"));
    h.wait(
        "cleared",
        |e| matches!(e, ModEvent::ErrorCleared { module } if module == "slow"),
    );
}

#[test]
fn views_beyond_twenty_a_second_are_dropped_with_one_warning() {
    let h = Harness::new(|_| {});
    install(&h.modules, "chatty", "chatty");
    h.host.rescan();
    h.wait(
        "rate warning",
        |e| matches!(e, ModEvent::Log { text, .. } if text.contains("views per second")),
    );
    std::thread::sleep(Duration::from_millis(200));
    let views = h.count(|e| matches!(e, ModEvent::View { .. }));
    assert!((1..=20).contains(&views), "{views} views got through");
    assert_eq!(
        h.count(|e| matches!(e, ModEvent::Log { text, .. } if text.contains("views per second"))),
        1
    );
}

#[test]
fn unknown_provider_is_answered_with_an_error_and_known_ones_are_forwarded() {
    let h = Harness::new(|_| {});
    let script = "#!/bin/sh\nread -r init\necho '{\"t\":\"subscribe\",\"name\":\"nope\"}'\necho '{\"t\":\"subscribe\",\"name\":\"media\"}'\nwhile read -r line; do\n  case \"$line\" in *'\"code\":\"provider\"'*) echo '{\"t\":\"log\",\"level\":\"info\",\"text\":\"told off\"}' ;; esac\ndone\n";
    write(&h.modules, "subs", "{\"command\":[\"./run.sh\"]}", script);
    h.host.rescan();
    h.wait(
        "subscribe media",
        |e| matches!(e, ModEvent::Subscribe { name, .. } if name == "media"),
    );
    h.wait(
        "module told off",
        |e| matches!(e, ModEvent::Log { text, .. } if text == "told off"),
    );
    assert_eq!(h.count(|e| matches!(e, ModEvent::Subscribe { .. })), 1);
}

#[test]
fn folders_that_cannot_be_modules_are_reported_once() {
    let h = Harness::new(|_| {});
    fs::create_dir_all(h.modules.join("modules")).unwrap();
    fs::create_dir_all(h.modules.join("has space")).unwrap();
    fs::create_dir_all(h.modules.join(".hidden")).unwrap();
    h.host.rescan();
    h.host.rescan();
    h.wait("reserved", is_error("modules", ErrorKind::Rejected));
    h.wait("bad name", is_error("has space", ErrorKind::Rejected));
    assert_eq!(h.count(|e| matches!(e, ModEvent::Error { .. })), 2);
    assert!(h.host.modules().is_empty());
}

#[test]
fn disabling_stops_the_process_and_enabling_starts_it() {
    let h = Harness::new(|_| {});
    install(&h.modules, "good", "good");
    h.host.rescan();
    h.wait("view", is_view("good", "hello"));
    let pid = h.host.status("good").unwrap().pid.unwrap();
    h.host.set_enabled("good", false);
    h.wait_dead(pid);
    assert_eq!(h.host.status("good").unwrap().state, State::Disabled);
    assert_eq!(h.count(is_error("good", ErrorKind::Crash)), 0);
    let m = h.mark();
    h.host.set_enabled("good", true);
    h.wait_from(m, "started again", started("good"));
}

#[test]
fn stop_all_kills_a_stubborn_module_and_everything_it_started() {
    let h = Harness::new(|_| {});
    install(&h.modules, "stubborn", "stubborn");
    h.host.rescan();
    install(&h.modules, "good", "good");
    h.host.rescan();
    h.wait("good view", is_view("good", "hello"));
    let ev = h.wait(
        "pids",
        |e| matches!(e, ModEvent::Log { text, .. } if text.starts_with("pids ")),
    );
    let ModEvent::Log { text, .. } = ev else {
        unreachable!()
    };
    let pids: Vec<u32> = text
        .split_whitespace()
        .skip(1)
        .map(|p| p.parse().unwrap())
        .collect();
    assert_eq!(pids.len(), 2);
    let good = h.host.status("good").unwrap().pid.unwrap();
    assert!(pids.iter().all(|p| alive(*p)) && alive(good));

    let t = Instant::now();
    h.host.stop_all();
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
    for pid in pids.iter().chain([&good]) {
        assert!(!alive(*pid), "pid {pid} survived stop_all");
    }
    // Stubborn ignored stop and SIGTERM, so the log says the group was killed.
    assert!(
        h.log_text("stubborn")
            .contains("stopped (killed by signal 9)"),
        "{}",
        h.log_text("stubborn")
    );
    h.host.stop_all(); // twice is fine
}

#[test]
fn dropping_the_host_leaves_no_processes() {
    let h = Harness::new(|_| {});
    install(&h.modules, "good", "good");
    h.host.rescan();
    h.wait("view", is_view("good", "hello"));
    let pid = h.host.status("good").unwrap().pid.unwrap();
    let Harness { host, _dir, .. } = h;
    drop(host);
    assert!(!alive(pid));
}

#[test]
fn a_command_that_does_not_exist_is_a_crash_row_not_a_panic() {
    let h = Harness::new(|_| {});
    write(
        &h.modules,
        "ghost",
        "{\"command\":[\"./nope\"]}",
        "#!/bin/sh\n",
    );
    h.host.rescan();
    let ev = h.wait("start failure", is_error("ghost", ErrorKind::Crash));
    let ModEvent::Error { text, .. } = ev else {
        unreachable!()
    };
    assert!(text.contains("failed to start"), "{text}");
}

#[test]
fn command_placeholders_and_env_reach_the_module() {
    let h = Harness::new(|_| {});
    let manifest =
        r#"{"command":["sh","run.sh","${config}","${cache}"],"env":{"GREETING":"hi ${cache}"}}"#;
    let script = "#!/bin/sh\necho \"args $1 $2 env $GREETING id $NOTCHD_MODULE_ID cwd $(pwd)\" >&2\nwhile read -r l; do :; done\n";
    let dir = write(&h.modules, "env", manifest, script);
    h.host.rescan();
    let log = h.wait_log("env", "args ");
    let root = h.modules.parent().unwrap();
    let (config, cache) = (
        root.join("module-config/env"),
        root.join("module-cache/env"),
    );
    assert!(
        log.contains(&format!("args {} {}", config.display(), cache.display())),
        "{log}"
    );
    assert!(
        log.contains(&format!("env hi {}", cache.display())),
        "{log}"
    );
    assert!(log.contains("id env"), "{log}");
    let cwd = fs::canonicalize(&dir).unwrap();
    assert!(log.contains(&format!("cwd {}", cwd.display())), "{log}");
    assert!(config.is_dir() && cache.is_dir());
}
