//! The `osd` module: volume, brightness, mic and keyboard light, as a peek.
//! Standalone: a notch of its own. The Omarchy host owns the `osd` IPC target
//! (every `omarchy osd` call arrives as `omarchy-shell osd show <json>`) and
//! forwards it here as the IPC method `osd` with the arguments `show <json>`,
//! `close`, `state` or `ping`; the answers are the target's own.
//!
//! Glyphs and payload rules mirror Omarchy's own OSD
//! (/usr/share/omarchy/shell/plugins/osd/OsdModel.js, MIT). Ported from
//! `hosts/omarchy/modules/osd.qml`.

use crate::module::{CoreModule, Ctx, Effect, Facts};
use crate::tree::{column, glyph, node, row, text, with};
use notchd_proto::{Node, Slot};
use serde_json::{Value, json};

/// JS `parseInt(s, 10)`: the leading integer, or `None` for NaN.
fn parse_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (sign, digits) = match s.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let n = digits.bytes().take_while(u8::is_ascii_digit).count();
    digits[..n].parse::<i64>().ok().map(|v| sign * v)
}

/// The glyph for an icon name (`iconFor`); a name that is not one is shown
/// as it is, no name picks one by the percentage.
fn icon_for(name: &str, percent: i64) -> String {
    let n = name.to_lowercase();
    let g = |c: u32| glyph(c);
    match n.as_str() {
        "volume-muted" | "volume-mute" | "muted" | "mute" => g(0xEEE8),
        "volume-low" => g(0xF026),
        "volume-medium" => g(0xF027),
        "volume-high" | "volume" => g(0xF028),
        "microphone-muted" | "microphone-off" | "mic-muted" | "mic-off" => g(0xF036D),
        "microphone" | "mic" => g(0xF036C),
        "keyboard" => g(0xF030C),
        "brightness" | "display" => g(0xF0379),
        "touchpad" => g(0xF07F8),
        "touch" | "touchscreen" => g(0xF0741),
        "reboot" | "restart" => g(0xF0709),
        "shutdown" | "power" | "poweroff" => g(0xF0425),
        "logout" | "sign-out" | "leave" => g(0xF0343),
        _ if n.starts_with("media") || n.starts_with("player") => g(if n.ends_with("play") {
            0xF040A
        } else if n.ends_with("pause") {
            0xF03E4
        } else if n.ends_with("next") {
            0xF04AD
        } else if n.ends_with("previous") {
            0xF04AE
        } else {
            0xF075A
        }),
        _ if !n.is_empty() => name.to_string(),
        _ if percent <= 0 => g(0xEEE8),
        _ if percent <= 33 => g(0xF026),
        _ if percent <= 66 => g(0xF027),
        _ => g(0xF028),
    }
}

#[derive(Default)]
pub struct Osd {
    icon: String,
    message: String,
    value: i64,
    max: i64,
    has_progress: bool,
    effects: Vec<Effect>,
}

impl Osd {
    pub fn new() -> Osd {
        Osd {
            max: 100,
            ..Osd::default()
        }
    }

    fn fraction(&self) -> f64 {
        if self.max > 0 {
            self.value as f64 / self.max as f64
        } else {
            0.0
        }
    }

    /// `show(payloadJson)`: the payload is Omarchy's (`icon`, `message`,
    /// `value`, `max`, `duration`, `progressText`); a bad one shows as empty.
    fn show(&mut self, payload: &str) {
        let p: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
        let get = |k: &str| notchd_core::js::string_or_empty(p.get(k).unwrap_or(&Value::Null));
        let raw_message = get("message");
        let raw_value = match p.get("value") {
            None | Some(Value::Null) => String::new(),
            Some(v) => notchd_core::js::string(v),
        };
        let or_default = |k: &str, d: &str| {
            let s = get(k);
            if s.is_empty() { d.to_string() } else { s }
        };
        let max = parse_int(&or_default("max", "100"))
            .filter(|m| *m != 0)
            .unwrap_or(100)
            .max(1);
        let parsed = parse_int(&raw_value);
        let has_progress = !raw_value.is_empty() && parsed.is_some() && raw_message.is_empty();
        let value = if has_progress {
            parsed.unwrap_or(0).clamp(0, max)
        } else {
            0
        };
        let percent = if has_progress {
            (value as f64 * 100.0 / max as f64).round() as i64
        } else {
            -1
        };
        let duration = parse_int(&or_default("duration", "1200")).unwrap_or(1200);

        self.max = max;
        self.value = value;
        self.has_progress = has_progress;
        self.message = if !raw_message.is_empty() {
            raw_message
        } else if has_progress {
            let t = get("progressText");
            if t.is_empty() {
                format!("{percent}%")
            } else {
                t
            }
        } else {
            String::new()
        };
        self.icon = icon_for(&get("icon"), percent);
        self.effects
            .push(Effect::Peek(duration.clamp(800, i64::from(u32::MAX)) as u32));
    }

    fn bar_peek(&self) -> Value {
        // Fixed width with a bar so it doesn't shift as the number changes.
        let message = if self.has_progress {
            with(
                with(
                    text(&self.message, "foreground", "body"),
                    "width",
                    json!(38),
                ),
                "align",
                json!("end"),
            )
        } else {
            text(&self.message, "foreground", "body")
        };
        let mut parts = vec![with(
            with(
                text(&self.icon, "foreground", "heading"),
                "width",
                json!(22),
            ),
            "align",
            json!("center"),
        )];
        if self.has_progress {
            parts.push(
                json!({"t": "progress", "value": self.fraction().clamp(0.0, 1.0),
                "tone": "accent", "thickness": 6, "width": 160}),
            );
        }
        parts.push(message);
        row(12.0, parts)
    }

    /// Side edge: the bar stands up, the value above it and the icon below.
    fn vertical_peek(&self) -> Value {
        let mut parts = Vec::new();
        if self.has_progress {
            parts.push(with(
                with(
                    text(&self.message, "foreground", "bodySmall"),
                    "width",
                    json!(38),
                ),
                "align",
                json!("center"),
            ));
            parts.push(json!({"t": "progress", "orientation": "v", "value": self.fraction().clamp(0.0, 1.0),
                "tone": "accent", "thickness": 6, "height": 140}));
        }
        parts.push(text(&self.icon, "foreground", "heading"));
        // A message without a bar (mic muted, keyboard light...): beneath the
        // icon, as is.
        if !self.has_progress && !self.message.is_empty() {
            parts.push(text(&self.message, "foreground", "bodySmall"));
        }
        with(column(10.0, parts), "align", json!("center"))
    }
}

impl CoreModule for Osd {
    fn id(&self) -> &'static str {
        "osd"
    }

    fn standalone(&self) -> bool {
        true
    }

    fn facts(&self, _now: u64) -> Facts {
        Facts {
            priority: 100,
            urgent_peek: true,
            ..Facts::default()
        }
    }

    fn view(&self, slot: Slot, _now: u64) -> Option<Node> {
        Some(node(match slot {
            Slot::Peek => self.bar_peek(),
            Slot::VerticalPeek => self.vertical_peek(),
            _ => return None,
        }))
    }

    fn command(&mut self, ctx: &Ctx, method: &str, args: &[String], _now: u64) -> Option<String> {
        if method != "osd" {
            return None;
        }
        Some(match args.first().map(String::as_str) {
            Some("show") => {
                self.show(args.get(1).map_or("", String::as_str));
                "ok".into()
            }
            Some("close") => {
                self.effects.push(Effect::DismissPeek);
                "ok".into()
            }
            Some("state") => if ctx.peeking { "open" } else { "closed" }.into(),
            Some("ping") => "ok".into(),
            other => format!("unknown osd method: {}", other.unwrap_or("")),
        })
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(m: &mut Osd, v: Value) {
        assert_eq!(
            m.command(&Ctx::default(), "osd", &["show".into(), v.to_string()], 0)
                .as_deref(),
            Some("ok")
        );
    }

    fn texts(n: &Node) -> Vec<String> {
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
        let mut out = Vec::new();
        walk(&serde_json::to_value(n).unwrap(), &mut out);
        out
    }

    #[test]
    fn a_value_is_a_bar_with_its_percentage_and_a_peek_of_at_least_800_ms() {
        let mut m = Osd::new();
        show(&mut m, json!({"icon": "volume", "value": "40"}));
        assert_eq!(m.take_effects(), vec![Effect::Peek(1200)]);
        assert_eq!(texts(&m.view(Slot::Peek, 0).unwrap()), ["\u{f028}", "40%"]);
        let bar = serde_json::to_string(&m.view(Slot::Peek, 0).unwrap()).unwrap();
        assert!(
            bar.contains("\"value\":0.4") && bar.contains("\"width\":160"),
            "{bar}"
        );
        show(
            &mut m,
            json!({"icon": "brightness", "value": 70, "max": 140, "duration": "100"}),
        );
        assert_eq!(m.take_effects(), vec![Effect::Peek(800)]);
        assert_eq!(texts(&m.view(Slot::Peek, 0).unwrap()), ["\u{f0379}", "50%"]);
    }

    #[test]
    fn a_message_has_no_bar_and_progress_text_replaces_the_percentage() {
        let mut m = Osd::new();
        show(
            &mut m,
            json!({"icon": "mic-off", "message": "Mic muted", "value": 30}),
        );
        assert!(!m.has_progress);
        assert_eq!(
            texts(&m.view(Slot::Peek, 0).unwrap()),
            ["\u{f036d}", "Mic muted"]
        );
        assert!(
            !serde_json::to_string(&m.view(Slot::Peek, 0).unwrap())
                .unwrap()
                .contains("progress")
        );
        show(
            &mut m,
            json!({"icon": "keyboard", "value": "2", "max": "3", "progressText": "Level 2"}),
        );
        assert_eq!(
            texts(&m.view(Slot::Peek, 0).unwrap()),
            ["\u{f030c}", "Level 2"]
        );
    }

    #[test]
    fn values_are_read_as_javascript_read_them() {
        let mut m = Osd::new();
        show(&mut m, json!({"value": "250%"}));
        assert_eq!(m.value, 100, "clamped to the max");
        show(&mut m, json!({"value": "-5"}));
        assert_eq!(m.value, 0);
        show(&mut m, json!({"value": "abc"}));
        assert!(!m.has_progress);
        show(&mut m, json!({"value": 40, "max": 0}));
        assert_eq!(m.max, 100, "a max of 0 is the default");
        m.command(
            &Ctx::default(),
            "osd",
            &["show".into(), "not json".into()],
            0,
        );
        assert_eq!(m.message, "");
        // No icon name picks one by the percentage.
        for (v, want) in [(0, 0xEEE8), (20, 0xF026), (50, 0xF027), (90, 0xF028)] {
            show(&mut m, json!({"value": v}));
            assert_eq!(m.icon, glyph(want), "{v}");
        }
    }

    #[test]
    fn icon_names_map_to_glyphs_and_an_unknown_one_is_shown_as_it_is() {
        assert_eq!(icon_for("MUTE", 0), glyph(0xEEE8));
        assert_eq!(icon_for("media-playback-pause", 0), glyph(0xF03E4));
        assert_eq!(icon_for("player", 0), glyph(0xF075A));
        assert_eq!(icon_for("shutdown", 0), glyph(0xF0425));
        assert_eq!(icon_for("󰈈", 0), "󰈈");
    }

    #[test]
    fn the_side_edge_stands_the_bar_up_and_puts_a_message_under_the_icon() {
        let mut m = Osd::new();
        show(&mut m, json!({"icon": "volume", "value": "40"}));
        let v = serde_json::to_string(&m.view(Slot::VerticalPeek, 0).unwrap()).unwrap();
        assert!(
            v.contains("\"orientation\":\"v\"") && v.contains("\"height\":140"),
            "{v}"
        );
        show(&mut m, json!({"icon": "mic-off", "message": "Mic muted"}));
        assert_eq!(
            texts(&m.view(Slot::VerticalPeek, 0).unwrap()),
            ["\u{f036d}", "Mic muted"]
        );
    }

    #[test]
    fn close_state_ping_and_other_methods() {
        let mut m = Osd::new();
        let ask = |m: &mut Osd, ctx: &Ctx, a: &[&str]| {
            m.command(
                ctx,
                "osd",
                &a.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                0,
            )
        };
        assert_eq!(
            ask(&mut m, &Ctx::default(), &["ping"]).as_deref(),
            Some("ok")
        );
        assert_eq!(
            ask(&mut m, &Ctx::default(), &["state"]).as_deref(),
            Some("closed")
        );
        assert_eq!(
            ask(
                &mut m,
                &Ctx {
                    peeking: true,
                    ..Ctx::default()
                },
                &["state"]
            )
            .as_deref(),
            Some("open")
        );
        assert_eq!(
            ask(&mut m, &Ctx::default(), &["close"]).as_deref(),
            Some("ok")
        );
        assert_eq!(m.take_effects(), vec![Effect::DismissPeek]);
        assert!(
            ask(&mut m, &Ctx::default(), &["nope"])
                .unwrap()
                .starts_with("unknown osd method")
        );
        assert!(m.command(&Ctx::default(), "timer", &[], 0).is_none());
        assert!(m.standalone() && m.facts(0).urgent_peek && m.facts(0).priority == 100);
    }
}
