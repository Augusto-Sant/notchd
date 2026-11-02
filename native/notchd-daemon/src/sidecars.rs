//! The sidecars of the daemon's modules (the voice's bridges): one process
//! per module at a time, started when the module asks, fed its input, and
//! stopped with the module or the daemon. What it prints and how it ends go
//! to the hub as [`Note`]s (`sidecar`) naming the module and the run, so the
//! reader threads never take the hub's lock and the lines of a run that was
//! replaced are told apart.

use crate::ptyhost::Note;
use notchd_providers::sidecar::{Event, Sidecar};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Sidecars {
    procs: BTreeMap<String, Sidecar>,
    notes: Option<Sender<Note>>,
}

impl Sidecars {
    pub fn set_notes(&mut self, notes: Sender<Note>) {
        self.notes = Some(notes);
    }

    /// Starts `argv` for module `id` as run `run`; the one running is killed
    /// first. A program that cannot start is reported as the run's end, with
    /// why.
    pub fn start(&mut self, id: &str, run: u64, argv: &[String]) {
        self.stop(id);
        let Some(notes) = self.notes.clone() else {
            return;
        };
        let name = id.to_string();
        let tx = notes.clone();
        let result = Sidecar::spawn(argv, &[], move |e| {
            let data = match e {
                Event::Line(line) => json!({"id": name, "run": run, "event": "line", "line": line}),
                Event::Exit(code) => json!({"id": name, "run": run, "event": "exit", "code": code}),
            };
            let _ = tx.send(Note {
                provider: "sidecar",
                data,
            });
        });
        match result {
            Ok(s) => {
                self.procs.insert(id.to_string(), s);
            }
            Err(e) => {
                let why = format!(
                    "Could not start {}: {e}",
                    argv.first().map_or("the bridge", String::as_str)
                );
                let _ = notes.send(Note {
                    provider: "sidecar",
                    data: json!({"id": id, "run": run, "event": "exit", "error": why}),
                });
            }
        }
    }

    /// A line to its input.
    pub fn write(&mut self, id: &str, line: &str) {
        if let Some(s) = self.procs.get_mut(id) {
            s.write(format!("{line}\n").as_bytes());
        }
    }

    /// SIGTERM.
    pub fn terminate(&mut self, id: &str) {
        if let Some(s) = self.procs.get(id) {
            s.terminate();
        }
    }

    /// Ends it now, with what it started (its module is gone).
    pub fn stop(&mut self, id: &str) {
        if let Some(mut s) = self.procs.remove(id) {
            s.close_input();
            s.kill();
        }
    }

    /// The ids that have a process.
    pub fn ids(&self) -> Vec<String> {
        self.procs.keys().cloned().collect()
    }

    /// Ends every one (the daemon is going): asked to, then killed with its
    /// group when it takes more than a couple of seconds.
    pub fn stop_all(&mut self) {
        let all = std::mem::take(&mut self.procs);
        for s in all.values() {
            s.terminate();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && all.values().any(|s| !s.exited()) {
            std::thread::sleep(Duration::from_millis(20));
        }
        for mut s in all.into_values() {
            s.close_input();
            s.kill();
        }
    }
}
