//! /proc and df (resources provider): `/proc/stat` and `/proc/meminfo` on every
//! reading, `df -P -B1 <path>` once a minute (it is a process, and a disk
//! changes slowly). The numbers come out of `notchd-core::resources`.

use crate::resources::{ResourcesProvider, Sample};
use notchd_core::resources::{self, CpuTimes};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How often the disk is read.
const DISK_EVERY: Duration = Duration::from_secs(60);

pub struct Procfs {
    last_cpu: Option<CpuTimes>,
    disk_read: Option<(Instant, String)>,
}

impl Procfs {
    pub fn new() -> Procfs {
        Procfs {
            last_cpu: None,
            disk_read: None,
        }
    }

    fn read_disk(&mut self, path: &str) -> Option<resources::Usage> {
        let due = self
            .disk_read
            .as_ref()
            .is_none_or(|(at, p)| p != path || at.elapsed() >= DISK_EVERY);
        if !due {
            return None;
        }
        self.disk_read = Some((Instant::now(), path.to_string()));
        let out = Command::new("df")
            .args(["-P", "-B1", path])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        resources::disk(&String::from_utf8_lossy(&out.stdout))
    }
}

impl Default for Procfs {
    fn default() -> Procfs {
        Procfs::new()
    }
}

impl ResourcesProvider for Procfs {
    fn sample(&mut self, disk_path: &str) -> Sample {
        let mut s = Sample::default();
        if let Ok(stat) = std::fs::read_to_string("/proc/stat") {
            let times = resources::cpu_times(&stat);
            s.cpu = resources::cpu_usage(self.last_cpu, times);
            if times.is_some() {
                self.last_cpu = times;
            }
            s.cores = resources::cpu_count(&stat);
        }
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            s.memory = resources::memory(&meminfo);
        }
        s.disk = self.read_disk(disk_path);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_of_this_machine_is_a_set_of_sane_fractions() {
        let mut p = Procfs::new();
        let first = p.sample("/");
        assert!(
            first.cpu.is_none(),
            "the first reading has nothing to compare with"
        );
        assert!(first.cores > 0);
        let m = first.memory.expect("meminfo");
        assert!(m.total > 0.0 && (0.0..=1.0).contains(&m.fraction));
        let d = first.disk.expect("df of /");
        assert!(d.total > 0.0 && (0.0..=1.0).contains(&d.fraction));
        std::thread::sleep(Duration::from_millis(30));
        let second = p.sample("/");
        assert!(second.cpu.is_some_and(|c| (0.0..=1.0).contains(&c)));
        assert!(
            second.disk.is_none(),
            "the disk is read once a minute, not every time"
        );
    }
}
