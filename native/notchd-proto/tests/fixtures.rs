//! Round-trip tests against `tests/protocol/`: every valid case parses and
//! re-serializes byte-identically (compact, canonical key order); every invalid
//! one is rejected. Trees are also held to `schema/view.schema.json`.

mod schema;

use notchd_proto::FromModule;
use notchd_proto::provider::{MediaData, NotifyData, OsdData, ResourcesData, UsageData};
use notchd_proto::version::{CANVAS_COMMANDS, PRIMITIVES};
use notchd_proto::{Cmd, Manifest, Node, ToDaemon, ToModule, ToUi, Validate, parse, to_line};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Fixture {
    message: String,
    valid: Vec<Box<RawValue>>,
    invalid: Vec<Invalid>,
}

#[derive(Deserialize)]
struct Invalid {
    why: String,
    json: Box<RawValue>,
    /// The schema cannot express this rule (a limit), so it still accepts it.
    #[serde(default, rename = "schemaOk")]
    schema_ok: bool,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/protocol")
}

fn fixtures(dir: &str) -> Vec<(String, Fixture)> {
    let mut out = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(root().join(dir))
        .unwrap_or_else(|e| panic!("tests/protocol/{dir}: {e}"))
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let fixture: Fixture =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            fixture.message,
            name,
            "{}: message must be the file name",
            path.display()
        );
        assert!(
            !fixture.valid.is_empty(),
            "{dir}/{name}: needs a valid case"
        );
        assert!(
            !fixture.invalid.is_empty(),
            "{dir}/{name}: needs an invalid case"
        );
        out.push((name, fixture));
    }
    out
}

fn check<T>(dir: &str, f: &Fixture)
where
    T: Serialize + DeserializeOwned + Validate,
{
    for (i, raw) in f.valid.iter().enumerate() {
        let line = raw.get();
        let value: T =
            parse(line).unwrap_or_else(|e| panic!("{dir}/{} valid #{i}: {e}\n{line}", f.message));
        assert_eq!(
            to_line(&value),
            line,
            "{dir}/{} valid #{i} does not re-serialize identically",
            f.message
        );
    }
    for inv in &f.invalid {
        let line = inv.json.get();
        assert!(
            parse::<T>(line).is_err(),
            "{dir}/{} invalid ({}) was accepted:\n{line}",
            f.message,
            inv.why
        );
    }
}

/// Every valid case of an enum's fixture carries the message's own `t`.
fn check_tags(dir: &str, f: &Fixture) {
    for raw in &f.valid {
        let v: Value = serde_json::from_str(raw.get()).unwrap();
        assert_eq!(
            v["t"],
            f.message.as_str(),
            "{dir}/{}: t differs from the file name",
            f.message
        );
    }
}

fn names(list: &[(String, Fixture)]) -> BTreeSet<String> {
    list.iter().map(|(n, _)| n.clone()).collect()
}

fn tags(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn enum_dir<T>(dir: &str, all_tags: &[&str])
where
    T: Serialize + DeserializeOwned + Validate,
{
    let list = fixtures(dir);
    assert_eq!(
        names(&list),
        tags(all_tags),
        "{dir}: one fixture file per message"
    );
    for (_, f) in &list {
        check_tags(dir, f);
        check::<T>(dir, f);
    }
}

#[test]
fn core_to_ui() {
    enum_dir::<ToUi>("to-ui", ToUi::TAGS);
}

#[test]
fn ui_to_core() {
    enum_dir::<ToDaemon>("to-daemon", ToDaemon::TAGS);
}

#[test]
fn core_to_module() {
    enum_dir::<ToModule>("to-module", ToModule::TAGS);
}

#[test]
fn module_to_core() {
    enum_dir::<FromModule>("from-module", FromModule::TAGS);
}

#[test]
fn manifest() {
    for (_, f) in &fixtures("manifest") {
        check::<Manifest>("manifest", f);
    }
}

#[test]
fn primitives() {
    let list = fixtures("tree");
    let have = names(&list);
    for p in PRIMITIVES {
        assert!(have.contains(*p), "tree/{p}.json is missing");
    }
    for (_, f) in &list {
        check::<Node>("tree", f);
    }
}

#[test]
fn canvas_commands() {
    let list = fixtures("canvas");
    assert_eq!(
        names(&list),
        tags(CANVAS_COMMANDS),
        "one fixture per canvas command"
    );
    for (_, f) in &list {
        check::<Cmd>("canvas", f);
    }
}

#[test]
fn providers() {
    let list = fixtures("provider");
    assert_eq!(
        names(&list),
        tags(&["media", "resources", "usage", "osd", "notify"])
    );
    for (name, f) in &list {
        match name.as_str() {
            "media" => check::<MediaData>("provider", f),
            "resources" => check::<ResourcesData>("provider", f),
            "usage" => check::<UsageData>("provider", f),
            "osd" => check::<OsdData>("provider", f),
            "notify" => check::<NotifyData>("provider", f),
            other => panic!("unknown provider fixture {other}"),
        }
    }
}

#[test]
fn trees_follow_the_json_schema() {
    let schema = schema::load();
    let node = |v: &Value| schema::validate(&schema, &schema, v);
    for (name, f) in &fixtures("tree") {
        for raw in &f.valid {
            let v: Value = serde_json::from_str(raw.get()).unwrap();
            node(&v).unwrap_or_else(|e| {
                panic!(
                    "tree/{name}: schema rejects a valid case: {e}\n{}",
                    raw.get()
                )
            });
        }
        for inv in f.invalid.iter().filter(|i| !i.schema_ok) {
            let v: Value = serde_json::from_str(inv.json.get()).unwrap();
            assert!(
                node(&v).is_err(),
                "tree/{name}: schema accepts invalid case ({})",
                inv.why
            );
        }
    }
    // The trees inside `view` messages.
    for dir in ["to-ui", "from-module"] {
        let list = fixtures(dir);
        let (_, f) = list.iter().find(|(n, _)| n == "view").unwrap();
        for raw in &f.valid {
            let v: Value = serde_json::from_str(raw.get()).unwrap();
            if !v["tree"].is_null() {
                node(&v["tree"])
                    .unwrap_or_else(|e| panic!("{dir}/view: schema rejects a tree: {e}"));
            }
        }
    }
}
