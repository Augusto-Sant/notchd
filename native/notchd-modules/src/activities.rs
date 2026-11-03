//! The `activities` module: whatever scripts push over `push`/`clear` (a
//! deploy, a build, a download). Ported from `core/modules/activities.qml`;
//! the fields of a push are in docs/ipc.md.

use crate::module::{CoreModule, Ctx, Effect, Facts};
use crate::tree::{Ic, column, icon, node, row, text, with};
use notchd_proto::{Node, Slot};
use serde_json::{Value, json};

const DEFAULT_ICON: Ic = ("notification", 0xF02FC);

#[derive(Clone, Debug, PartialEq)]
struct Item {
    id: String,
    /// A glyph the pusher gave; none draws the default named icon.
    icon: Option<String>,
    title: String,
    subtitle: String,
    /// 0..=100, or -1 for no bar.
    progress: f64,
}

#[derive(Default)]
pub struct Activities {
    /// The sticky ones, in the order they were last pushed.
    items: Vec<Item>,
    /// The last one pushed, sticky or not: what the peek shows.
    last: Option<Item>,
    effects: Vec<Effect>,
}

impl Activities {
    pub fn new() -> Activities {
        Activities::default()
    }

    /// `push(json)`: "ok" or "invalid json".
    fn push(&mut self, json: &str, now: u64) -> String {
        let Ok(Value::Object(o)) = serde_json::from_str::<Value>(json) else {
            return "invalid json".into();
        };
        let progress = match o.get("progress") {
            None | Some(Value::Null) => -1.0,
            Some(v) => notchd_core::js::number(v),
        };
        let str_of = |key: &str, default: String| match o.get(key) {
            Some(v) if notchd_core::js::truthy(v) => notchd_core::js::string(v),
            _ => default,
        };
        let item = Item {
            id: str_of("id", format!("activity-{now}")),
            icon: Some(str_of("icon", String::new())).filter(|s| !s.is_empty()),
            title: str_of("title", String::new()),
            subtitle: str_of("subtitle", String::new()),
            progress: if progress.is_finite() && progress >= 0.0 {
                progress.min(100.0)
            } else {
                -1.0
            },
        };
        let sticky = o.get("sticky") == Some(&json!(true));
        self.items.retain(|x| x.id != item.id);
        if sticky {
            self.items.push(item.clone());
        }
        self.last = Some(item);
        if o.get("peek") != Some(&json!(false)) {
            let ms = o.get("duration").map_or(0.0, notchd_core::js::number);
            self.effects
                .push(Effect::Peek(if ms.is_finite() && ms >= 1.0 {
                    ms.min(f64::from(u32::MAX)) as u32
                } else {
                    3000
                }));
        }
        "ok".into()
    }

    /// `clear(id)`: `""` removes all.
    fn clear(&mut self, id: &str) -> String {
        if id.is_empty() {
            self.items.clear();
        } else {
            self.items.retain(|x| x.id != id);
        }
        if self
            .last
            .as_ref()
            .is_some_and(|l| id.is_empty() || l.id == id)
        {
            self.effects.push(Effect::DismissPeek);
        }
        "ok".into()
    }
}

fn percent(item: &Item) -> Option<Value> {
    (item.progress >= 0.0).then(|| {
        with(
            text(&format!("{}%", item.progress.round()), "muted", "caption"),
            "tabular",
            json!(true),
        )
    })
}

/// The item's mark in the size a text size names, as the modules draw theirs:
/// the glyph it was pushed with, else the default named icon.
fn mark(item: &Item, size: &str) -> Value {
    match &item.icon {
        Some(g) => json!({"t": "icon", "glyph": g, "tone": "accent", "size": size}),
        None => icon(DEFAULT_ICON, "accent", json!(size)),
    }
}

fn bar(item: &Item) -> Option<Value> {
    (item.progress >= 0.0).then(|| {
        json!({"t": "progress", "value": item.progress / 100.0, "tone": "accent", "thickness": 4})
    })
}

/// The row of an item (`ActivityRow`): icon, title and subtitle, the
/// percentage, and under them the bar (`full`: the peek's and the expanded
/// view's, with the subtitle).
fn item_row(item: &Item, full: bool) -> Value {
    let mut lines = vec![with(
        text(&item.title, "foreground", "body"),
        "fill",
        json!(true),
    )];
    if full && !item.subtitle.is_empty() {
        lines.push(text(&item.subtitle, "muted", "caption"));
    }
    let mut head = vec![
        mark(item, if full { "heading" } else { "body" }),
        with(column(0.0, lines), "fill", json!(true)),
    ];
    head.extend(percent(item));
    let mut parts = vec![row(12.0, head)];
    if full {
        parts.extend(bar(item));
    }
    column(5.0, parts)
}

impl CoreModule for Activities {
    fn id(&self) -> &'static str {
        "activities"
    }

    fn facts(&self, _now: u64) -> Facts {
        Facts {
            priority: 70,
            active: !self.items.is_empty(),
            ..Facts::default()
        }
    }

    fn view(&self, slot: Slot, _now: u64) -> Option<Node> {
        let item = self.items.last();
        Some(node(match slot {
            Slot::Compact => with(item_row(item?, false), "width", json!(220)),
            // Side edge: the icon, and the progress as a bar filling upwards.
            Slot::VerticalCompact => {
                let item = item?;
                let mut parts = vec![mark(item, "body")];
                if item.progress >= 0.0 {
                    parts.push(
                        json!({"t": "progress", "orientation": "v", "value": item.progress / 100.0,
                        "tone": "accent", "thickness": 4, "height": 28}),
                    );
                }
                with(column(6.0, parts), "align", json!("center"))
            }
            Slot::Peek => {
                let item = self.last.as_ref()?;
                with(
                    with(item_row(item, true), "minWidth", json!(220)),
                    "maxWidth",
                    json!(340),
                )
            }
            Slot::VerticalPeek => return None,
            Slot::Expanded => column(8.0, self.items.iter().map(|i| item_row(i, true)).collect()),
        }))
    }

    fn command(&mut self, _ctx: &Ctx, method: &str, args: &[String], now: u64) -> Option<String> {
        let arg = args.first().map_or("", String::as_str);
        match method {
            "push" => Some(self.push(arg, now)),
            "clear" => Some(self.clear(arg)),
            _ => None,
        }
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(m: &mut Activities, v: Value) -> String {
        m.command(&Ctx::default(), "push", &[v.to_string()], 1000)
            .unwrap()
    }

    fn texts(n: &Node) -> Vec<String> {
        let v = serde_json::to_value(n).unwrap();
        let mut out = Vec::new();
        fn walk(v: &Value, out: &mut Vec<String>) {
            if let Some(t) = v.get("text").and_then(Value::as_str) {
                out.push(t.into());
            }
            for c in v
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                walk(c, out);
            }
        }
        walk(&v, &mut out);
        out
    }

    #[test]
    fn a_sticky_push_is_an_item_a_plain_one_only_peeks() {
        let mut m = Activities::new();
        assert!(!m.facts(0).active);
        assert_eq!(
            push(&mut m, json!({"id": "a", "title": "Build", "peek": false})),
            "ok"
        );
        assert!(!m.facts(0).active && m.take_effects().is_empty());
        assert!(
            m.view(Slot::Peek, 0).is_some(),
            "the peek shows the last push"
        );
        assert_eq!(
            push(
                &mut m,
                json!({"id": "b", "title": "Deploy", "sticky": true, "progress": 40})
            ),
            "ok"
        );
        assert!(m.facts(0).active);
        assert_eq!(m.take_effects(), vec![Effect::Peek(3000)]);
        assert_eq!(texts(&m.view(Slot::Compact, 0).unwrap()), ["Deploy", "40%"]);
    }

    #[test]
    fn the_same_id_updates_in_place_and_comes_last() {
        let mut m = Activities::new();
        push(&mut m, json!({"id": "a", "title": "A", "sticky": true}));
        push(&mut m, json!({"id": "b", "title": "B", "sticky": true}));
        push(
            &mut m,
            json!({"id": "a", "title": "A2", "sticky": true, "duration": 500}),
        );
        assert_eq!(m.items.len(), 2);
        assert_eq!(m.items.last().unwrap().title, "A2");
        assert_eq!(m.take_effects().last(), Some(&Effect::Peek(500)));
    }

    #[test]
    fn bad_json_is_refused_and_progress_is_clamped() {
        let mut m = Activities::new();
        for bad in ["nope", "[1]", "3", "null", ""] {
            assert_eq!(
                m.command(&Ctx::default(), "push", &[bad.into()], 0)
                    .unwrap(),
                "invalid json",
                "{bad}"
            );
        }
        push(&mut m, json!({"id": "a", "progress": 250, "sticky": true}));
        assert_eq!(m.items[0].progress, 100.0);
        push(&mut m, json!({"id": "b", "progress": -3, "sticky": true}));
        assert_eq!(m.items[1].progress, -1.0);
        push(&mut m, json!({"title": "no id", "sticky": true}));
        assert_eq!(m.items[2].id, "activity-1000");
    }

    #[test]
    fn clear_removes_one_or_all_and_drops_the_peek_it_was_showing() {
        let mut m = Activities::new();
        push(&mut m, json!({"id": "a", "sticky": true}));
        push(&mut m, json!({"id": "b", "sticky": true}));
        m.take_effects();
        assert_eq!(
            m.command(&Ctx::default(), "clear", &["a".into()], 0)
                .unwrap(),
            "ok"
        );
        assert_eq!(m.items.len(), 1);
        assert!(m.take_effects().is_empty(), "a was not the last");
        m.command(&Ctx::default(), "clear", &["".into()], 0);
        assert!(m.items.is_empty());
        assert_eq!(m.take_effects(), vec![Effect::DismissPeek]);
        assert!(m.command(&Ctx::default(), "timer", &[], 0).is_none());
    }

    #[test]
    fn the_views_for_each_slot() {
        let mut m = Activities::new();
        assert!(m.view(Slot::Compact, 0).is_none() && m.view(Slot::Peek, 0).is_none());
        push(
            &mut m,
            json!({"id": "a", "title": "T", "subtitle": "S", "progress": 50, "sticky": true}),
        );
        for slot in [
            Slot::Compact,
            Slot::VerticalCompact,
            Slot::Peek,
            Slot::Expanded,
        ] {
            assert!(m.view(slot, 0).is_some(), "{slot:?}");
        }
        assert!(m.view(Slot::VerticalPeek, 0).is_none());
        // The compact row has no subtitle; the peek and the expanded view do.
        assert!(!texts(&m.view(Slot::Compact, 0).unwrap()).contains(&"S".to_string()));
        assert!(texts(&m.view(Slot::Peek, 0).unwrap()).contains(&"S".to_string()));
        assert!(texts(&m.view(Slot::Expanded, 0).unwrap()).contains(&"S".to_string()));
    }
}
