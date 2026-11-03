//! The `resources` module: CPU, memory and the disk holding `/`. Expanded: a
//! bar each, with how much is used. Closed: all three percentages, or, with
//! the `compact` setting at "short", nothing until one runs short. One runs
//! short at 90 % or more (the CPU for about 10 s in a row, so a burst does
//! not count): it turns red, the module peeks once, and it stays red until
//! it drops back. Ported from `core/modules/resources.qml`; the readings are
//! the `resources` provider's (`notchd-providers`), the rules
//! `notchd-core::resources`.

use crate::module::{CoreModule, Effect, Facts};
use crate::tree::{Ic, Settings, column, icon, node, row, text, with};
use notchd_core::resources::{Pressure, PressureRule, Usage, format_usage, next_pressure};
use notchd_proto::{Node, Slot};
use notchd_providers::resources::Sample;
use serde_json::{Value, json};

/// The filesystem shown as Disk: the one holding this path.
pub const DISK_PATH: &str = "/";

const RULES: [(Res, PressureRule); 3] = [
    (
        Res::Cpu,
        PressureRule {
            warn: 0.9,
            hold: Some(5.0),
            clear: Some(0.1),
        },
    ),
    (
        Res::Memory,
        PressureRule {
            warn: 0.9,
            hold: Some(1.0),
            clear: Some(0.05),
        },
    ),
    (
        Res::Disk,
        PressureRule {
            warn: 0.9,
            hold: Some(1.0),
            clear: Some(0.02),
        },
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Res {
    Cpu,
    Memory,
    Disk,
}

impl Res {
    fn ic(self) -> Ic {
        match self {
            Res::Cpu => ("cpu", 0xF0EE0),
            Res::Memory => ("memory", 0xF035B),
            Res::Disk => ("drive-harddisk", 0xF02CA),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Res::Cpu => "CPU",
            Res::Memory => "Memory",
            Res::Disk => "Disk",
        }
    }
}

pub struct Resources {
    settings: Settings,
    cpu: Option<f64>,
    cores: usize,
    memory: Option<Usage>,
    disk: Option<Usage>,
    pressure: [Option<Pressure>; 3],
    /// The one that last ran short, for the peek.
    peek_of: Option<Res>,
    effects: Vec<Effect>,
}

impl Resources {
    pub fn new() -> Resources {
        Resources {
            settings: Settings::new("resources", include_str!("../modules/resources.json")),
            cpu: None,
            cores: 0,
            memory: None,
            disk: None,
            pressure: [None; 3],
            peek_of: None,
            effects: Vec::new(),
        }
    }

    fn always_compact(&self) -> bool {
        self.settings.get("compact") != json!("short")
    }

    fn slot(r: Res) -> usize {
        RULES.iter().position(|(x, _)| *x == r).unwrap_or(0)
    }

    fn is_short(&self, r: Res) -> bool {
        self.pressure[Self::slot(r)].is_some_and(|p| p.hot)
    }

    fn fraction(&self, r: Res) -> Option<f64> {
        match r {
            Res::Cpu => self.cpu,
            Res::Memory => self.memory.map(|u| u.fraction),
            Res::Disk => self.disk.map(|u| u.fraction),
        }
    }

    fn detail(&self, r: Res) -> String {
        match r {
            Res::Cpu if self.cores > 0 => {
                format!(
                    "{} {}",
                    self.cores,
                    if self.cores == 1 { "core" } else { "cores" }
                )
            }
            Res::Cpu => String::new(),
            Res::Memory => self
                .memory
                .map_or(String::new(), |u| format_usage(u.used, u.total)),
            Res::Disk => self.disk.map_or(String::new(), |u| {
                format!("{} · {DISK_PATH}", format_usage(u.used, u.total))
            }),
        }
    }

    /// A new value for a resource: updates its pressure, and peeks when it
    /// turns short.
    fn record(&mut self, r: Res, value: Option<f64>) {
        let i = Self::slot(r);
        let prev = self.pressure[i];
        let next = next_pressure(prev, value, &RULES[i].1);
        self.pressure[i] = Some(next);
        if next.hot && !prev.is_some_and(|p| p.hot) {
            self.peek_of = Some(r);
            self.effects.push(Effect::Peek(4000));
        }
    }

    fn rows(&self) -> [Res; 3] {
        [Res::Cpu, Res::Memory, Res::Disk]
    }

    /// What the closed notch shows.
    fn compact_rows(&self) -> Vec<Res> {
        self.rows()
            .into_iter()
            .filter(|r| self.always_compact() || self.is_short(*r))
            .collect()
    }

    fn percent(f: Option<f64>) -> String {
        f.map_or("–".into(), |f| format!("{}%", (f * 100.0).round()))
    }

    /// Red while short, else `normal`.
    fn tone(&self, r: Res, normal: &'static str) -> &'static str {
        if self.is_short(r) { "danger" } else { normal }
    }

    /// Icon, name and detail, the percentage, and a bar; red while short.
    fn resource_row(&self, r: Res) -> Value {
        let tone = self.tone(r, "accent");
        let head = row(
            10.0,
            vec![
                with(icon(r.ic(), tone, json!("body")), "width", json!(18)),
                text(r.label(), "foreground", "body"),
                with(
                    text(&self.detail(r), "muted", "caption"),
                    "fill",
                    json!(true),
                ),
                text(
                    &Self::percent(self.fraction(r)),
                    self.tone(r, "foreground"),
                    "bodySmall",
                ),
            ],
        );
        column(
            5.0,
            vec![
                head,
                json!({"t": "progress", "value": self.fraction(r).unwrap_or(0.0), "tone": tone, "thickness": 4}),
            ],
        )
    }
}

impl Default for Resources {
    fn default() -> Resources {
        Resources::new()
    }
}

impl CoreModule for Resources {
    fn id(&self) -> &'static str {
        "resources"
    }

    fn facts(&self, _now: u64) -> Facts {
        Facts {
            // Below media and activities: it only takes the closed notch when
            // nothing else wants it. What runs short still peeks.
            priority: 20,
            active: true,
            ..Facts::default()
        }
    }

    fn view(&self, slot: Slot, _now: u64) -> Option<Node> {
        let shown = self.compact_rows();
        Some(node(match slot {
            Slot::Compact => {
                if shown.is_empty() {
                    return None;
                }
                row(
                    12.0,
                    shown
                        .iter()
                        .map(|r| {
                            row(
                                6.0,
                                vec![
                                    icon(r.ic(), self.tone(*r, "accent"), json!("body")),
                                    text(
                                        &Self::percent(self.fraction(*r)),
                                        "foreground",
                                        "bodySmall",
                                    ),
                                ],
                            )
                        })
                        .collect(),
                )
            }
            // Side edge: each one's icon over its number.
            Slot::VerticalCompact => {
                if shown.is_empty() {
                    return None;
                }
                with(
                    column(
                        8.0,
                        shown
                            .iter()
                            .map(|r| {
                                with(
                                    column(
                                        2.0,
                                        vec![
                                            icon(r.ic(), self.tone(*r, "accent"), json!("body")),
                                            text(
                                                &self.fraction(*r).map_or("–".into(), |f| {
                                                    format!("{}", (f * 100.0).round())
                                                }),
                                                "foreground",
                                                "caption",
                                            ),
                                        ],
                                    ),
                                    "align",
                                    json!("center"),
                                )
                            })
                            .collect(),
                    ),
                    "align",
                    json!("center"),
                )
            }
            Slot::Peek => with(
                self.resource_row(self.peek_of.unwrap_or(Res::Cpu)),
                "width",
                json!(300),
            ),
            Slot::VerticalPeek => return None,
            Slot::Expanded => column(
                12.0,
                self.rows().iter().map(|r| self.resource_row(*r)).collect(),
            ),
        }))
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn config(&mut self, cfg: &Value, _now: u64) {
        self.settings.set_config(cfg);
    }

    fn feed(&mut self, provider: &str, data: &Value, _now: u64) {
        if provider != "resources" {
            return;
        }
        let Ok(s) = serde_json::from_value::<Sample>(data.clone()) else {
            return;
        };
        if s.cores > 0 {
            self.cores = s.cores;
        }
        if let Some(c) = s.cpu {
            self.cpu = Some(c);
            self.record(Res::Cpu, Some(c));
        }
        if let Some(m) = s.memory {
            self.memory = Some(m);
            self.record(Res::Memory, Some(m.fraction));
        }
        if let Some(d) = s.disk {
            self.disk = Some(d);
            self.record(Res::Disk, Some(d.fraction));
        }
    }

    /// `test-resources <cpu> <memory> <disk> [cores memory-GiB disk-GiB]`:
    /// fractions (0 to 1) in place of the system's readings, which stop (the
    /// offscreen renders show demo values).
    fn demo(&mut self, args: &[String], now: u64) -> Option<String> {
        let f = |i: usize, default: f64| match args.get(i) {
            Some(a) => a.parse::<f64>().ok(),
            None => Some(default),
        };
        let (cpu, mem, disk) = (f(0, f64::NAN)?, f(1, f64::NAN)?, f(2, f64::NAN)?);
        if [cpu, mem, disk].iter().any(|v| v.is_nan()) {
            return None;
        }
        let (cores, mem_gib, disk_gib) = (f(3, 8.0)?, f(4, 31.2)?, f(5, 465.0)?);
        let gib = 1024.0 * 1024.0 * 1024.0;
        let usage = |fraction: f64, total: f64| Usage {
            used: fraction * total,
            total,
            fraction,
        };
        let sample = Sample {
            cpu: Some(cpu),
            cores: cores as usize,
            memory: Some(usage(mem, mem_gib * gib)),
            disk: Some(usage(disk, disk_gib * gib)),
        };
        self.feed("resources", &serde_json::to_value(sample).ok()?, now);
        Some("ok".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    fn sample(cpu: Option<f64>, mem: f64, disk: Option<f64>) -> Value {
        let usage = |f: f64, total: f64| Usage {
            used: f * total,
            total,
            fraction: f,
        };
        serde_json::to_value(Sample {
            cpu,
            cores: 4,
            memory: Some(usage(mem, 16.0 * GIB)),
            disk: disk.map(|d| usage(d, 500.0 * GIB)),
        })
        .unwrap()
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

    fn with_short_setting(m: &mut Resources) {
        m.config(&json!({"resources": {"compact": "short"}}), 0);
    }

    #[test]
    fn closed_it_shows_all_three_by_default_and_none_until_one_runs_short_when_asked() {
        let mut m = Resources::new();
        assert!(m.facts(0).active);
        assert!(m.view(Slot::Compact, 0).is_none() || m.always_compact());
        m.feed("resources", &sample(Some(0.31), 0.5, Some(0.4)), 0);
        assert_eq!(
            texts(&m.view(Slot::Compact, 0).unwrap()),
            ["31%", "50%", "40%"]
        );
        with_short_setting(&mut m);
        assert!(m.view(Slot::Compact, 0).is_none() && m.view(Slot::VerticalCompact, 0).is_none());
        m.feed("resources", &sample(Some(0.31), 0.95, Some(0.4)), 0);
        assert_eq!(texts(&m.view(Slot::Compact, 0).unwrap()), ["95%"]);
        assert!(m.view(Slot::Expanded, 0).is_some());
    }

    #[test]
    fn a_resource_that_runs_short_peeks_once_and_cools_down_with_a_margin() {
        let mut m = Resources::new();
        m.feed("resources", &sample(None, 0.95, Some(0.4)), 0);
        assert_eq!(m.take_effects(), vec![Effect::Peek(4000)]);
        assert!(m.is_short(Res::Memory) && m.peek_of == Some(Res::Memory));
        m.feed("resources", &sample(None, 0.97, None), 0);
        assert!(m.take_effects().is_empty(), "still short: no second peek");
        m.feed("resources", &sample(None, 0.88, None), 0);
        assert!(m.is_short(Res::Memory), "inside the 5 % margin");
        m.feed("resources", &sample(None, 0.80, None), 0);
        assert!(!m.is_short(Res::Memory));
    }

    #[test]
    fn the_cpu_must_stay_busy_for_five_readings() {
        let mut m = Resources::new();
        for _ in 0..4 {
            m.feed("resources", &sample(Some(0.99), 0.1, None), 0);
        }
        assert!(!m.is_short(Res::Cpu) && m.take_effects().is_empty());
        m.feed("resources", &sample(Some(0.99), 0.1, None), 0);
        assert!(m.is_short(Res::Cpu));
        assert_eq!(m.take_effects(), vec![Effect::Peek(4000)]);
    }

    #[test]
    fn expanded_has_a_row_per_resource_with_its_detail() {
        let mut m = Resources::new();
        m.feed("resources", &sample(Some(0.5), 0.25, Some(0.5)), 0);
        let t = texts(&m.view(Slot::Expanded, 0).unwrap());
        for want in [
            "CPU",
            "4 cores",
            "50%",
            "Memory",
            "4.0 of 16.0 GiB",
            "25%",
            "Disk",
            "250 of 500 GiB · /",
        ] {
            assert!(t.contains(&want.to_string()), "{want} in {t:?}");
        }
        // Nothing read yet: dashes and no detail.
        let t = texts(&Resources::new().view(Slot::Expanded, 0).unwrap());
        assert_eq!(t.iter().filter(|s| *s == "–").count(), 3);
    }

    #[test]
    fn the_vertical_view_has_numbers_without_a_percent_sign() {
        let mut m = Resources::new();
        m.feed("resources", &sample(Some(0.5), 0.25, Some(0.5)), 0);
        let t = texts(&m.view(Slot::VerticalCompact, 0).unwrap());
        assert_eq!(t, ["50", "25", "50"]);
    }

    #[test]
    fn a_short_resource_is_red_in_every_view() {
        let mut m = Resources::new();
        m.feed("resources", &sample(None, 0.95, Some(0.4)), 0);
        let peek = serde_json::to_string(&m.view(Slot::Peek, 0).unwrap()).unwrap();
        assert!(
            peek.contains("\"danger\"") && peek.contains("Memory"),
            "{peek}"
        );
        let compact = serde_json::to_string(&m.view(Slot::Compact, 0).unwrap()).unwrap();
        assert!(compact.contains("\"danger\""));
    }

    #[test]
    fn demo_values_stand_in_for_the_system() {
        let mut m = Resources::new();
        let args: Vec<String> = ["0.35", "0.5", "0.4"].map(String::from).to_vec();
        assert_eq!(m.demo(&args, 0).as_deref(), Some("ok"));
        assert_eq!(m.cpu, Some(0.35));
        assert!(m.demo(&["x".into()], 0).is_none());
    }
}
