//! What the daemon knows of its user modules, between the module host's
//! events ([`ModEvent`]) and the two places they go: the store (a module's
//! facts, a peek or an open request, its settings page) and the UIs (the
//! `module` and `view` messages). Pure over a [`Store`]; the hub owns the
//! lock, the socket and the host.
//!
//! The views a UI is sent are the module's, except while it has an error
//! (a crash, a manifest, an invalid tree, an `error` of its own): then its
//! `compact`, `verticalCompact` and `expanded` slots hold the host's error row
//! (`modhost::error_row`) and `attention` is set, until the host says it is
//! healthy again.

use crate::modhost::{ModEvent, error_row};
use notchd_core::store::{ModuleFacts, Store};
use notchd_proto::module::ModuleSet;
use notchd_proto::ui::{ModuleInfo, View};
use notchd_proto::{Manifest, Node, Slot, ToUi};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// `NotchModule.expandedWidth`'s default.
const DEFAULT_EXPANDED_WIDTH: u32 = 400;

/// The slots an error row takes the place of.
const ERROR_SLOTS: [Slot; 3] = [Slot::Compact, Slot::VerticalCompact, Slot::Expanded];

#[derive(Debug, Clone, Default)]
struct ModState {
    title: String,
    priority: i32,
    active: bool,
    icon: String,
    /// The module's own `attention`; an error sets it on top.
    attention: bool,
    expanded_width: u32,
    urgent_peek: bool,
    peek_on_focused_screen: bool,
    standalone: bool,
    /// The error text, and the row that shows it.
    error: Option<(String, Node)>,
    /// What the module sent.
    views: Vec<(Slot, Node)>,
    /// What the UIs were last sent.
    sent_views: Vec<(Slot, Node)>,
    sent_info: Option<ModuleInfo>,
    rev: u64,
    view_rev: u64,
}

impl ModState {
    fn from_manifest(id: &str, m: &Manifest) -> ModState {
        ModState {
            title: m.title.clone().unwrap_or_else(|| id.to_string()),
            priority: m.priority.unwrap_or(0),
            icon: m.icon.clone().unwrap_or_default(),
            expanded_width: m.expanded_width.unwrap_or(DEFAULT_EXPANDED_WIDTH),
            urgent_peek: m.urgent_peek.unwrap_or(false),
            peek_on_focused_screen: m.peek_on_focused_screen.unwrap_or(false),
            standalone: m.standalone.unwrap_or(false),
            ..ModState::default()
        }
    }

    /// The tree a slot shows now: the error row in place of the module's.
    fn effective(&self, slot: Slot) -> Option<&Node> {
        if let Some((_, row)) = &self.error
            && ERROR_SLOTS.contains(&slot)
        {
            return Some(row);
        }
        self.views.iter().find(|(s, _)| *s == slot).map(|(_, n)| n)
    }

    fn facts(&self, id: &str) -> ModuleFacts {
        let mut f = ModuleFacts::new(id);
        f.info.priority = f64::from(self.priority);
        f.info.active = self.active;
        f.info.compact = self.effective(Slot::Compact).is_some();
        f.info.icon = !self.icon.is_empty();
        f.info.peek = self.effective(Slot::Peek).is_some();
        f.info.attention = self.attention || self.error.is_some();
        f.info.urgent_peek = self.urgent_peek;
        f.expanded = self.effective(Slot::Expanded).is_some();
        f.peek_on_focused_screen = self.peek_on_focused_screen;
        f.standalone = self.standalone;
        f
    }

    fn info(&self, id: &str) -> ModuleInfo {
        ModuleInfo {
            id: id.to_string(),
            rev: self.rev,
            title: self.title.clone(),
            priority: self.priority,
            active: self.active,
            icon: self.icon.clone(),
            attention: self.attention || self.error.is_some(),
            expanded_width: self.expanded_width,
            urgent_peek: self.urgent_peek,
            peek_on_focused_screen: self.peek_on_focused_screen,
            status: None,
            error: self.error.as_ref().map(|(text, _)| text.clone()),
        }
    }
}

/// The user modules the host reported.
#[derive(Debug, Default)]
pub struct Mods {
    mods: BTreeMap<String, ModState>,
}

/// `{ moduleId, title, settings, standalone }`: what the schema builds a
/// module's page from (`Settings.moduleFields`).
fn settings_def(id: &str, m: &Manifest) -> Value {
    json!({
        "moduleId": id,
        "title": m.title.clone().unwrap_or_else(|| id.to_string()),
        "settings": serde_json::to_value(&m.settings).unwrap_or_else(|_| json!([])),
        "standalone": m.standalone.unwrap_or(false),
    })
}

impl Mods {
    pub fn new() -> Mods {
        Mods::default()
    }

    pub fn knows(&self, id: &str) -> bool {
        self.mods.contains_key(id)
    }

    pub fn ids(&self) -> Vec<String> {
        self.mods.keys().cloned().collect()
    }

    pub fn standalone(&self, id: &str) -> bool {
        self.mods.get(id).is_some_and(|m| m.standalone)
    }

    /// Applies an event from the host: the store follows, and the `module`
    /// and `view` messages the UIs need are returned.
    pub fn apply(&mut self, store: &mut Store, ev: ModEvent, now: u64) -> Vec<ToUi> {
        match ev {
            ModEvent::Loaded { module, manifest } => {
                let old = self.mods.remove(&module);
                let mut m = ModState::from_manifest(&module, &manifest);
                if let Some(old) = old {
                    // A save: the module's own state carries over until it
                    // sends again.
                    m.active = old.active;
                    m.attention = old.attention;
                    m.error = old.error;
                    m.views = old.views;
                    m.sent_views = old.sent_views;
                    m.sent_info = old.sent_info;
                    m.rev = old.rev;
                    m.view_rev = old.view_rev;
                }
                store.set_module_def(settings_def(&module, &manifest));
                self.mods.insert(module.clone(), m);
                self.sync(store, &module)
            }
            ModEvent::Removed { module } => {
                let Some(old) = self.mods.remove(&module) else {
                    return Vec::new();
                };
                store.remove_module(&module);
                store.remove_module_def(&module);
                // The slots it had go empty.
                old.sent_views
                    .iter()
                    .map(|(slot, _)| {
                        ToUi::View(View {
                            module: module.clone(),
                            slot: *slot,
                            rev: old.view_rev + 1,
                            tree: None,
                        })
                    })
                    .collect()
            }
            ModEvent::View { module, slot, tree } => {
                let m = self.entry(&module);
                m.views.retain(|(s, _)| *s != slot);
                if let Some(tree) = tree {
                    m.views.push((slot, tree));
                }
                self.sync(store, &module)
            }
            ModEvent::Set { module, set } => {
                apply_set(self.entry(&module), &set);
                self.sync(store, &module)
            }
            ModEvent::Peek { module, peek } => {
                // `NotchModule.requestPeek(ms)`; the facts are current.
                let out = self.sync(store, &module);
                store.request_peek(&module, Some(f64::from(peek.ms)), now);
                out
            }
            ModEvent::Open { module } => {
                let out = self.sync(store, &module);
                store.open_module(&module, now);
                out
            }
            ModEvent::Error {
                module,
                text,
                log_path,
                ..
            } => {
                let row = error_row(&text, &log_path);
                self.entry(&module).error = Some((text, row));
                self.sync(store, &module)
            }
            ModEvent::ErrorCleared { module } => {
                self.entry(&module).error = None;
                self.sync(store, &module)
            }
            // Providers, notifications and the log are the host's and the
            // providers' business, not the notch's state.
            ModEvent::Started { .. }
            | ModEvent::Notify { .. }
            | ModEvent::Subscribe { .. }
            | ModEvent::Unsubscribe { .. }
            | ModEvent::Log { .. } => Vec::new(),
        }
    }

    fn entry(&mut self, id: &str) -> &mut ModState {
        self.mods.entry(id.to_string()).or_default()
    }

    /// Puts a module's facts in the store and returns what changed for the
    /// UIs: a `module` when its facts differ from the last sent, a `view` per
    /// slot whose tree does.
    fn sync(&mut self, store: &mut Store, id: &str) -> Vec<ToUi> {
        let Some(m) = self.mods.get_mut(id) else {
            return Vec::new();
        };
        store.set_module(m.facts(id));
        let mut out = Vec::new();
        let mut info = m.info(id);
        let changed = m.sent_info.as_ref().is_none_or(|last| {
            let mut a = last.clone();
            a.rev = info.rev;
            a != info
        });
        if changed {
            m.rev += 1;
            info.rev = m.rev;
            m.sent_info = Some(info.clone());
            out.push(ToUi::Module(info));
        }
        for slot in Slot::ALL {
            let now = m.effective(slot).cloned();
            let before = m
                .sent_views
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, n)| n);
            if now.as_ref() == before {
                continue;
            }
            m.sent_views.retain(|(s, _)| *s != slot);
            if let Some(n) = &now {
                m.sent_views.push((slot, n.clone()));
            }
            m.view_rev += 1;
            out.push(ToUi::View(View {
                module: id.to_string(),
                slot,
                rev: m.view_rev,
                tree: now,
            }));
        }
        out
    }

    /// What a UI that just connected is sent: every module's facts and one
    /// `view` per slot in use.
    pub fn snapshot(&self) -> Vec<ToUi> {
        let mut out = Vec::new();
        for (id, m) in &self.mods {
            if let Some(info) = &m.sent_info {
                out.push(ToUi::Module(info.clone()));
            }
            for (slot, tree) in &m.sent_views {
                out.push(ToUi::View(View {
                    module: id.clone(),
                    slot: *slot,
                    rev: m.view_rev,
                    tree: Some(tree.clone()),
                }));
            }
        }
        out
    }
}

fn apply_set(m: &mut ModState, set: &ModuleSet) {
    if let Some(v) = set.active {
        m.active = v;
    }
    if let Some(v) = set.priority {
        m.priority = v;
    }
    if let Some(v) = &set.icon {
        m.icon = v.clone();
    }
    if let Some(v) = set.attention {
        m.attention = v;
    }
    if let Some(v) = set.urgent_peek {
        m.urgent_peek = v;
    }
    if let Some(v) = set.expanded_width {
        m.expanded_width = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modhost::ErrorKind;
    use notchd_proto::module::Peek;
    use serde_json::json;

    fn manifest() -> Manifest {
        serde_json::from_value(json!({
            "title": "Weather", "command": ["./run.sh"], "priority": 40, "icon": "W",
            "settings": [{"key": "city", "type": "string", "label": "City", "default": "Berlin"}]
        }))
        .unwrap()
    }

    fn text(s: &str) -> Node {
        serde_json::from_value(json!({"t": "text", "text": s})).unwrap()
    }

    fn loaded() -> (Mods, Store) {
        let mut mods = Mods::new();
        let mut store = Store::new();
        store.set_config(json!({"modules": ["weather"]}));
        let out = mods.apply(
            &mut store,
            ModEvent::Loaded {
                module: "weather".into(),
                manifest: Box::new(manifest()),
            },
            0,
        );
        assert!(matches!(&out[..], [ToUi::Module(m)] if m.title == "Weather" && m.rev == 1));
        (mods, store)
    }

    fn view(slot: Slot, s: &str) -> ModEvent {
        ModEvent::View {
            module: "weather".into(),
            slot,
            tree: Some(text(s)),
        }
    }

    #[test]
    fn loaded_gives_the_store_a_module_with_the_manifests_facts_and_a_settings_page() {
        let (_, store) = loaded();
        let f = store.module("weather").unwrap();
        assert_eq!(f.info.priority, 40.0);
        assert!(!f.info.active, "a module says when it is active");
        assert!(f.info.icon);
        assert!(
            store
                .compute_schema()
                .fields
                .iter()
                .any(|f| f.key.as_deref() == Some("weather.city"))
        );
    }

    #[test]
    fn a_view_reaches_the_ui_once_and_sets_the_facts_that_depend_on_it() {
        let (mut mods, mut store) = loaded();
        let out = mods.apply(&mut store, view(Slot::Compact, "18"), 0);
        let [ToUi::View(v)] = &out[..] else {
            panic!("{out:?}")
        };
        assert_eq!(
            (v.module.as_str(), v.slot, v.rev),
            ("weather", Slot::Compact, 1)
        );
        assert!(store.module("weather").unwrap().info.compact);
        // The same tree again: nothing to send.
        let out = mods.apply(&mut store, view(Slot::Compact, "18"), 0);
        assert!(out.is_empty(), "{out:?}");
        let out = mods.apply(&mut store, view(Slot::Compact, "19"), 0);
        assert!(matches!(&out[..], [ToUi::View(v)] if v.rev == 2));
        let out = mods.apply(
            &mut store,
            ModEvent::View {
                module: "weather".into(),
                slot: Slot::Compact,
                tree: None,
            },
            0,
        );
        assert!(matches!(&out[..], [ToUi::View(v)] if v.tree.is_none()));
    }

    #[test]
    fn set_moves_the_facts_and_a_peek_goes_to_the_store() {
        let (mut mods, mut store) = loaded();
        mods.apply(&mut store, view(Slot::Peek, "rain"), 0);
        let set: ModuleSet =
            serde_json::from_value(json!({"active": true, "priority": 90})).unwrap();
        let out = mods.apply(
            &mut store,
            ModEvent::Set {
                module: "weather".into(),
                set,
            },
            0,
        );
        assert!(
            matches!(&out[..], [ToUi::Module(m)] if m.active && m.priority == 90 && m.rev == 2)
        );
        store.set_screens(notchd_proto::ui::Screens {
            screens: vec![notchd_proto::ui::ScreenInfo {
                name: "DP-1".into(),
                x: 0,
                y: 0,
                w: 1000,
                h: 800,
                scale: notchd_proto::Num(1.0),
            }],
            focused: "DP-1".into(),
            fullscreen: vec![],
            workspaces: Default::default(),
            known: vec![],
            supports_workspaces: false,
            bar_edge: None,
        });
        mods.apply(
            &mut store,
            ModEvent::Peek {
                module: "weather".into(),
                peek: Peek {
                    ms: 1500,
                    slot: None,
                },
            },
            10,
        );
        let st = store.state();
        let p = st.notches[0].copies[0].peek.as_ref().expect("peeking");
        assert_eq!((p.module.as_str(), p.until), ("weather", 1510));
    }

    #[test]
    fn an_error_puts_its_row_in_the_slots_and_clearing_it_brings_the_views_back() {
        let (mut mods, mut store) = loaded();
        mods.apply(&mut store, view(Slot::Compact, "18"), 0);
        mods.apply(&mut store, view(Slot::Peek, "rain"), 0);
        let out = mods.apply(
            &mut store,
            ModEvent::Error {
                module: "weather".into(),
                kind: ErrorKind::Crash,
                text: "weather exited (1)".into(),
                log_path: "/logs/weather.log".into(),
            },
            0,
        );
        let [
            ToUi::Module(info),
            ToUi::View(compact),
            ToUi::View(_vertical),
            ToUi::View(expanded),
        ] = &out[..]
        else {
            panic!("{out:?}")
        };
        assert_eq!(info.error.as_deref(), Some("weather exited (1)"));
        assert!(info.attention, "an error asks for the eye");
        assert_eq!(
            (compact.slot, expanded.slot),
            (Slot::Compact, Slot::Expanded)
        );
        let row = serde_json::to_string(compact.tree.as_ref().unwrap()).unwrap();
        assert!(row.contains("weather exited (1)") && row.contains("/logs/weather.log"));
        assert!(row.contains("restart"), "the Restart button");
        assert!(
            store.module("weather").unwrap().expanded,
            "the row shows expanded too"
        );
        let out = mods.apply(
            &mut store,
            ModEvent::ErrorCleared {
                module: "weather".into(),
            },
            0,
        );
        let [
            ToUi::Module(info),
            ToUi::View(compact),
            ToUi::View(_vertical),
            ToUi::View(expanded),
        ] = &out[..]
        else {
            panic!("{out:?}")
        };
        assert!(info.error.is_none() && !info.attention);
        assert_eq!(compact.tree, Some(text("18")));
        assert!(expanded.tree.is_none());
    }

    #[test]
    fn a_new_ui_is_sent_the_facts_and_the_views_in_use() {
        let (mut mods, mut store) = loaded();
        mods.apply(&mut store, view(Slot::Compact, "18"), 0);
        let snap = mods.snapshot();
        assert!(matches!(&snap[..], [ToUi::Module(_), ToUi::View(v)] if v.slot == Slot::Compact));
    }

    #[test]
    fn a_save_keeps_what_the_module_sent_and_a_removed_module_empties_its_slots() {
        let (mut mods, mut store) = loaded();
        let set: ModuleSet = serde_json::from_value(json!({"active": true})).unwrap();
        mods.apply(
            &mut store,
            ModEvent::Set {
                module: "weather".into(),
                set,
            },
            0,
        );
        mods.apply(&mut store, view(Slot::Compact, "18"), 0);
        let mut m2 = manifest();
        m2.priority = Some(10);
        let out = mods.apply(
            &mut store,
            ModEvent::Loaded {
                module: "weather".into(),
                manifest: Box::new(m2),
            },
            0,
        );
        assert!(matches!(&out[..], [ToUi::Module(m)] if m.priority == 10 && m.active));
        let out = mods.apply(
            &mut store,
            ModEvent::Removed {
                module: "weather".into(),
            },
            0,
        );
        assert!(matches!(&out[..], [ToUi::View(v)] if v.tree.is_none()));
        assert!(store.module("weather").is_none());
        assert!(
            !store
                .compute_schema()
                .pages
                .iter()
                .any(|p| p.id == "modules/weather")
        );
    }
}
