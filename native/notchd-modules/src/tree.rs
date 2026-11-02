//! Small builders for the view trees the modules make: JSON in the protocol's
//! shape, parsed into a [`Node`] (so a typo is caught by the first test that
//! builds the tree, and every tree is validated by the same code the daemon
//! applies to a user module's).

use notchd_proto::{Node, Validate};
use serde_json::{Value, json};

/// A node from its JSON; the trees here are written by hand, so one that does
/// not parse is a bug.
pub fn node(v: Value) -> Node {
    let n: Node = serde_json::from_value(v.clone())
        .unwrap_or_else(|e| panic!("a module built a bad tree ({e}): {v}"));
    if let Err(e) = n.validate() {
        panic!("a module built an invalid tree ({e}): {v}");
    }
    n
}

pub fn row(gap: f64, children: Vec<Value>) -> Value {
    json!({"t": "row", "gap": gap, "children": children})
}

pub fn column(gap: f64, children: Vec<Value>) -> Value {
    json!({"t": "column", "gap": gap, "children": children})
}

/// A text; `tone` and `size` are the protocol's names.
pub fn text(s: &str, tone: &str, size: &str) -> Value {
    json!({"t": "text", "text": s, "tone": tone, "size": size})
}

/// A named icon with its Nerd Font code point: what the official modules send
/// (`name`, and the `glyph` a UI without that name draws).
pub type Ic = (&'static str, u32);

fn checked_name(name: &str) -> &str {
    assert!(
        notchd_proto::icons::NAMES.contains(&name),
        "icon name {name} is not in notchd_proto::icons::NAMES"
    );
    name
}

/// An `icon` node; `size` is design pixels (a number) or a text size (a
/// string); an empty `tone` leaves it to the container (a meter's disc).
pub fn icon(ic: Ic, tone: &str, size: Value) -> Value {
    let v = json!({"t": "icon", "glyph": glyph(ic.1), "name": checked_name(ic.0), "size": size});
    if tone.is_empty() {
        v
    } else {
        with(v, "tone", json!(tone))
    }
}

/// A `button` with an icon.
pub fn icon_button(ic: Ic, action: &str) -> Value {
    json!({"t": "button", "glyph": glyph(ic.1), "name": checked_name(ic.0), "action": action})
}

/// Sets `key` on an object node, to refine what a builder made.
pub fn with(mut v: Value, key: &str, value: Value) -> Value {
    if let Some(m) = v.as_object_mut() {
        m.insert(key.into(), value);
    }
    v
}

/// A Nerd Font code point as the string an `icon` takes.
pub fn glyph(code: u32) -> String {
    char::from_u32(code).map(String::from).unwrap_or_default()
}

/// A module's settings as its JSON file declares them (`native/notchd-modules/modules/<id>.json`):
/// the one place a default lives. The config's `<id>.<key>` wins.
pub struct Settings {
    id: &'static str,
    defaults: Value,
    cfg: Value,
}

impl Settings {
    /// `file` is the JSON file's text (`include_str!`).
    pub fn new(id: &'static str, file: &str) -> Settings {
        let info: Value =
            serde_json::from_str(file).unwrap_or_else(|e| panic!("{id}.json does not parse: {e}"));
        Settings {
            id,
            defaults: info,
            cfg: Value::Null,
        }
    }

    pub fn set_config(&mut self, cfg: &Value) {
        self.cfg = cfg.clone();
    }

    /// The config's value, else the field's default.
    pub fn get(&self, key: &str) -> Value {
        if let Some(v) = self
            .cfg
            .get(self.id)
            .and_then(|m| m.get(key))
            .filter(|v| !v.is_null())
        {
            return v.clone();
        }
        self.defaults["settings"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| f["key"] == key)
            .map(|f| f["default"].clone())
            .unwrap_or(Value::Null)
    }

    /// `"<id>": { "enabled": false }` and the like are the host's business.
    pub fn standalone(&self) -> bool {
        self.defaults["standalone"] == json!(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_and_a_button_carry_the_name_and_the_glyph_to_fall_back_on() {
        let i = icon(("media-playback-start", 0xF040A), "accent", json!("body"));
        assert_eq!(i["name"], "media-playback-start");
        assert_eq!(i["glyph"], "\u{F040A}");
        assert_eq!(i["size"], "body");
        node(i);
        let b = icon_button(("window-close", 0xF0156), "close");
        assert_eq!(b["name"], "window-close");
        assert_eq!(b["glyph"], "\u{F0156}");
        node(b);
        assert!(icon(("cpu", 0xF0EE0), "", json!(28)).get("tone").is_none());
    }

    #[test]
    #[should_panic(expected = "not in notchd_proto::icons::NAMES")]
    fn a_module_cannot_send_a_name_the_table_lacks() {
        icon(("made-up", 0xF040A), "accent", json!("body"));
    }
}
