//! Port of `core/Resources.js` (tests: `tests/resources.test.mjs`).
//!
//! System resources rules: reading /proc and df output, usage fractions and
//! when a resource counts as under pressure. Numbers are `f64`, like the JS.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::js;

/// CPU time counters, in jiffies (`cpuTimes`'s `{ idle, total }`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CpuTimes {
    /// Idle plus iowait.
    pub idle: f64,
    pub total: f64,
}

/// `cpuTimes`: the counters of /proc/stat's aggregate "cpu" line; `None`
/// when the line is missing or malformed.
pub fn cpu_times(text: &str) -> Option<CpuTimes> {
    for line in text.split('\n') {
        let mut parts = line
            .trim_matches(js::is_ws)
            .split(js::is_ws)
            .filter(|p| !p.is_empty());
        // `"".split(/\s+/)` is `[""]`: a blank line never matches "cpu".
        if parts.next() != Some("cpu") {
            continue;
        }
        let n: Vec<f64> = parts.take(8).map(js::str_to_number).collect();
        if n.len() < 4 || n.iter().any(|x| !x.is_finite()) {
            return None;
        }
        let total = n.iter().sum();
        let iowait = n.get(4).copied().filter(|v| *v != 0.0).unwrap_or(0.0);
        return Some(CpuTimes {
            idle: n[3] + iowait,
            total,
        });
    }
    None
}

/// `cpuCount`: how many CPUs /proc/stat lists (its "cpu0", "cpu1"... lines).
pub fn cpu_count(text: &str) -> usize {
    text.split('\n')
        .filter(|l| {
            l.strip_prefix("cpu").is_some_and(|rest| {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                digits > 0 && rest[digits..].chars().next().is_some_and(js::is_ws)
            })
        })
        .count()
}

/// `cpuUsage`: the busy fraction (0 to 1) between two [`cpu_times`]
/// samples; `None` without a previous sample or when no time passed.
pub fn cpu_usage(prev: Option<CpuTimes>, next: Option<CpuTimes>) -> Option<f64> {
    let (prev, next) = (prev?, next?);
    let total = next.total - prev.total;
    if total <= 0.0 {
        return None;
    }
    let busy = total - (next.idle - prev.idle);
    Some((busy / total).clamp(0.0, 1.0))
}

/// Used and total bytes with the used fraction (0 to 1), for memory and disk.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub used: f64,
    pub total: f64,
    pub fraction: f64,
}

/// `memory`: from /proc/meminfo; used is what can't be reclaimed (MemTotal
/// minus MemAvailable), in bytes. `None` when the keys are missing.
pub fn memory(text: &str) -> Option<Usage> {
    let mut kb: HashMap<&str, f64> = HashMap::new();
    for line in text.split('\n') {
        // /^(\w+):\s+(\d+)/
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let value = rest.trim_start_matches(js::is_ws);
        if value.len() == rest.len() {
            continue;
        }
        let digits = value.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            continue;
        }
        if let Ok(n) = value[..digits].parse::<f64>() {
            kb.insert(key, n);
        }
    }
    let total_kb = *kb.get("MemTotal")?;
    let avail_kb = *kb.get("MemAvailable")?;
    if total_kb == 0.0 {
        return None;
    }
    let total = total_kb * 1024.0;
    let used = (total_kb - avail_kb) * 1024.0;
    Some(Usage {
        used,
        total,
        fraction: (used / total).clamp(0.0, 1.0),
    })
}

/// `disk`: a filesystem from `df -P -B1 <path>`: used and total in bytes.
/// Its fraction is used over used plus available, as df's own Capacity: the
/// space reserved for root doesn't count as free. `None` when the output has
/// no data line.
pub fn disk(text: &str) -> Option<Usage> {
    let lines: Vec<&str> = text.trim_matches(js::is_ws).split('\n').collect();
    for line in lines.iter().skip(1).rev() {
        let parts: Vec<&str> = line
            .trim_matches(js::is_ws)
            .split(js::is_ws)
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() < 6 {
            continue;
        }
        let (total, used, avail) = (
            js::str_to_number(parts[1]),
            js::str_to_number(parts[2]),
            js::str_to_number(parts[3]),
        );
        if !total.is_finite() || !used.is_finite() || !avail.is_finite() || used + avail <= 0.0 {
            continue;
        }
        return Some(Usage {
            used,
            total,
            fraction: used / (used + avail),
        });
    }
    None
}

const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

/// `formatBytes`: bytes as a short binary size: "512 MiB", "7.8 GiB",
/// "1.2 TiB".
pub fn format_bytes(bytes: f64) -> String {
    let mut v = if bytes.is_nan() { 0.0 } else { bytes.max(0.0) };
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    let digits = usize::from(u >= 3 && v < 100.0);
    format!("{} {}", js::to_fixed(v, digits), UNITS[u])
}

/// `formatUsage`: "7.8 of 31.2 GiB": both in the total's unit, so they read
/// as one.
pub fn format_usage(used: f64, total: f64) -> String {
    let t = format_bytes(total);
    let unit = t.split(' ').nth(1).unwrap_or("B");
    let idx = UNITS.iter().position(|u| *u == unit).unwrap_or(0);
    let used = if used.is_nan() { 0.0 } else { used.max(0.0) };
    let v = used / 1024f64.powi(idx as i32);
    let digits = usize::from(matches!(unit, "GiB" | "TiB" | "PiB") && v < 100.0);
    format!("{} of {t}", js::to_fixed(v, digits))
}

/// One resource's pressure state (`{ hot, streak }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Pressure {
    pub hot: bool,
    pub streak: u32,
}

/// A pressure rule: `warn` fraction, `hold` samples in a row (default 1),
/// `clear` margin below `warn` (default 0).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PressureRule {
    pub warn: f64,
    pub hold: Option<f64>,
    pub clear: Option<f64>,
}

/// `nextPressure`: the next state from the previous one, the latest
/// fraction and its rule. It turns hot after `hold` samples in a row at or
/// above `warn`, and cools once below `warn - clear`, so a value hovering at
/// the line doesn't flicker. A missing (or non-finite) value keeps the state.
pub fn next_pressure(prev: Option<Pressure>, value: Option<f64>, rule: &PressureRule) -> Pressure {
    let prev = prev.unwrap_or_default();
    let Some(value) = value.filter(|v| v.is_finite()) else {
        return prev;
    };
    let hold = rule
        .hold
        .filter(|h| *h != 0.0 && !h.is_nan())
        .unwrap_or(1.0)
        .max(1.0);
    let clear = rule.clear.filter(|c| !c.is_nan()).unwrap_or(0.0);
    if prev.hot {
        return if value < rule.warn - clear {
            Pressure {
                hot: false,
                streak: 0,
            }
        } else {
            prev
        };
    }
    let streak = if value >= rule.warn {
        prev.streak + 1
    } else {
        0
    };
    Pressure {
        hot: f64::from(streak) >= hold,
        streak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\nintr 12345";
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    // cpuTimes reads the aggregate line, iowait counted as idle
    #[test]
    fn cpu_times_reads_the_aggregate_line_iowait_counted_as_idle() {
        assert_eq!(
            cpu_times(STAT),
            Some(CpuTimes {
                idle: 850.0,
                total: 1000.0
            })
        );
        assert_eq!(cpu_times("intr 1\n"), None);
        assert_eq!(cpu_times(""), None);
    }

    // cpuCount counts the per-CPU lines
    #[test]
    fn cpu_count_counts_the_per_cpu_lines() {
        assert_eq!(cpu_count(STAT), 1);
        assert_eq!(
            cpu_count("cpu  1 2 3 4\ncpu0 1 2 3 4\ncpu1 1 2 3 4\ncpu10 1 2 3 4\n"),
            3
        );
        assert_eq!(cpu_count(""), 0);
    }

    // cpuUsage is the busy share of the time between samples
    #[test]
    fn cpu_usage_is_the_busy_share_of_the_time_between_samples() {
        let t = |idle, total| Some(CpuTimes { idle, total });
        assert_eq!(cpu_usage(t(850.0, 1000.0), t(900.0, 1200.0)), Some(0.75));
        assert_eq!(cpu_usage(None, t(1.0, 2.0)), None, "no previous sample");
        assert_eq!(cpu_usage(t(1.0, 2.0), t(1.0, 2.0)), None, "no time passed");
    }

    // memory counts what cannot be reclaimed, not the page cache
    #[test]
    fn memory_counts_what_cannot_be_reclaimed_not_the_page_cache() {
        let m = memory(
            "MemTotal:       16000000 kB\nMemFree:  1000000 kB\nMemAvailable:   12000000 kB\nCached: 9000000 kB\n",
        )
        .unwrap();
        assert_eq!(m.total, 16_000_000.0 * 1024.0);
        assert_eq!(m.used, 4_000_000.0 * 1024.0);
        assert_eq!(m.fraction, 0.25);
        assert_eq!(memory("MemFree: 1 kB\n"), None);
    }

    // disk uses df capacity: the space reserved for root is not free
    #[test]
    fn disk_uses_df_capacity_the_space_reserved_for_root_is_not_free() {
        let d = disk(
            "Filesystem        1-blocks         Used    Available Capacity Mounted on\n\
             /dev/nvme0n1p2 1000000000000 600000000000 300000000000      67% /",
        )
        .unwrap();
        assert_eq!(d.total, 1_000_000_000_000.0);
        assert_eq!(d.used, 600_000_000_000.0);
        assert!((d.fraction - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(
            disk("Filesystem 1-blocks Used Available Capacity Mounted on"),
            None
        );
        assert_eq!(disk(""), None);
    }

    // sizes read short, used in the total unit
    #[test]
    fn sizes_read_short_used_in_the_total_unit() {
        assert_eq!(format_bytes(512.0 * 1024.0 * 1024.0), "512 MiB");
        assert_eq!(format_bytes(7.8 * GIB), "7.8 GiB");
        assert_eq!(format_bytes(512.0 * GIB), "512 GiB");
        assert_eq!(format_usage(7.8 * GIB, 31.2 * GIB), "7.8 of 31.2 GiB");
        assert_eq!(
            format_usage(900.0 * 1024.0 * 1024.0, 1.5 * GIB * 1024.0),
            "0.0 of 1.5 TiB"
        );
    }

    // Not in the Node tests: toFixed rounds half up on the exact value.
    #[test]
    fn to_fixed_rounds_like_js() {
        assert_eq!(js::to_fixed(2.5, 0), "3");
        assert_eq!(js::to_fixed(0.25, 1), "0.3");
        assert_eq!(js::to_fixed(0.15, 1), "0.1");
        assert_eq!(js::to_fixed(0.04, 1), "0.0");
        assert_eq!(js::to_fixed(99.96, 1), "100.0");
        assert_eq!(js::to_fixed(1.005, 2), "1.00");
    }

    // nextPressure: hot after holding at the line, cools below it with a margin
    #[test]
    fn next_pressure_hot_after_holding_at_the_line_cools_below_it_with_a_margin() {
        let rule = PressureRule {
            warn: 0.9,
            hold: Some(3.0),
            clear: Some(0.05),
        };
        let mut s = Pressure::default();
        s = next_pressure(Some(s), Some(0.95), &rule);
        assert!(!s.hot);
        s = next_pressure(Some(s), Some(0.95), &rule);
        assert!(!s.hot);
        s = next_pressure(Some(s), Some(0.2), &rule);
        assert_eq!(
            s,
            Pressure {
                hot: false,
                streak: 0
            },
            "a spike resets the streak"
        );
        for _ in 0..3 {
            s = next_pressure(Some(s), Some(0.92), &rule);
        }
        assert!(s.hot, "hot after three samples in a row");
        s = next_pressure(Some(s), Some(0.87), &rule);
        assert!(s.hot, "still hot just under the line");
        s = next_pressure(Some(s), Some(0.84), &rule);
        assert!(!s.hot, "cool past the margin");
        assert!(
            next_pressure(
                Some(Pressure {
                    hot: true,
                    streak: 1
                }),
                None,
                &rule
            )
            .hot,
            "no sample keeps the state"
        );
        let warn_only = PressureRule {
            warn: 0.9,
            ..PressureRule::default()
        };
        assert!(
            next_pressure(None, Some(0.95), &warn_only).hot,
            "hold defaults to one sample"
        );
    }
}
