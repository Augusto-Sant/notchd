//! The `sidecar` provider: a program the daemon runs for a module and talks to
//! in lines (the voice modules' Python bridges, `core/live/*.py`). Its
//! standard output is read a line at a time, its standard input written, and
//! both ends close with it: a sidecar whose input ends stops by itself, so one
//! left behind by a daemon that died does not run on.
//!
//! The reader thread owns the child and reaps it, so the exit is always
//! reported, once, after the last line. The process runs in a group of its
//! own, so a forced stop reaches what it started (the microphone, the agent,
//! the speakers' player) too.

use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What a running sidecar tells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A line of its standard output, without the line end.
    Line(String),
    /// It ended (`None`: by a signal). Always the last event.
    Exit(Option<i32>),
}

/// A running sidecar.
pub struct Sidecar {
    pid: u32,
    stdin: Option<ChildStdin>,
    gone: Arc<AtomicBool>,
}

impl Sidecar {
    /// Starts `argv` with `env` added to the daemon's environment. `on` is
    /// called from a thread of its own for every event.
    pub fn spawn(
        argv: &[String],
        env: &[(String, String)],
        on: impl Fn(Event) + Send + 'static,
    ) -> std::io::Result<Sidecar> {
        let Some((program, args)) = argv.split_first() else {
            return Err(std::io::Error::other("no command"));
        };
        let mut cmd = Command::new(program);
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let stdin = child.stdin.take();
        let out = child.stdout.take();
        let gone = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&gone);
        std::thread::Builder::new()
            .name("notchd-sidecar".into())
            .spawn(move || {
                if let Some(out) = out {
                    for line in BufReader::new(out).split(b'\n') {
                        let Ok(bytes) = line else { break };
                        on(Event::Line(String::from_utf8_lossy(&bytes).into_owned()));
                    }
                }
                let code = child.wait().ok().and_then(|s| s.code());
                done.store(true, Ordering::SeqCst);
                on(Event::Exit(code));
            })?;
        Ok(Sidecar { pid, stdin, gone })
    }

    /// Whether it has ended.
    pub fn exited(&self) -> bool {
        self.gone.load(Ordering::SeqCst)
    }

    /// Bytes to its standard input; false when it can take none.
    pub fn write(&mut self, bytes: &[u8]) -> bool {
        match self.stdin.as_mut() {
            Some(w) => w.write_all(bytes).and_then(|()| w.flush()).is_ok(),
            None => false,
        }
    }

    /// Closes its standard input: the end of it stops a sidecar.
    pub fn close_input(&mut self) {
        self.stdin = None;
    }

    /// SIGTERM to the process: it ends what it started and stops.
    pub fn terminate(&self) {
        if !self.exited() {
            signal(self.pid, false, libc::SIGTERM);
        }
    }

    /// SIGKILL to its whole group.
    pub fn kill(&self) {
        if !self.exited() {
            signal(self.pid, true, libc::SIGKILL);
        }
    }
}

fn signal(pid: u32, group: bool, sig: i32) {
    let Ok(pid) = i32::try_from(pid) else { return };
    if pid <= 1 {
        return;
    }
    // SAFETY: `kill` takes plain numbers; the pid is a child of ours that has
    // not been reaped (`exited` was false), or at worst one that just was.
    unsafe {
        libc::kill(if group { -pid } else { pid }, sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
    use std::time::Duration;

    fn run(script: &str) -> (Sidecar, Receiver<Event>) {
        let (tx, rx) = channel();
        let argv = vec!["sh".to_string(), "-c".to_string(), script.to_string()];
        let s = Sidecar::spawn(&argv, &[], move |e| {
            let _ = tx.send(e);
        })
        .expect("spawn");
        (s, rx)
    }

    fn next(rx: &Receiver<Event>) -> Event {
        rx.recv_timeout(Duration::from_secs(10)).expect("an event")
    }

    #[test]
    fn its_lines_come_in_order_and_the_exit_last() {
        let (_s, rx) = run("echo one; echo two; exit 3");
        assert_eq!(next(&rx), Event::Line("one".into()));
        assert_eq!(next(&rx), Event::Line("two".into()));
        assert_eq!(next(&rx), Event::Exit(Some(3)));
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "nothing after the exit"
        );
    }

    #[test]
    fn what_is_written_reaches_it_and_the_end_of_input_stops_it() {
        let (mut s, rx) = run("while read l; do echo \"got $l\"; done");
        assert!(s.write(b"hello\n"));
        assert_eq!(next(&rx), Event::Line("got hello".into()));
        s.close_input();
        assert_eq!(next(&rx), Event::Exit(Some(0)));
        assert!(!s.write(b"late\n"));
    }

    #[test]
    fn terminate_and_kill_end_it_and_what_it_started() {
        // The TERM goes to the process alone, which ends what it started. A
        // plain `sleep 300 & wait` passed only when the TERM came before the
        // shell started the sleep: after, the sleep kept stdout open (CI, #6).
        let (s, rx) = run("trap 'kill $!; exit' TERM; sleep 300 & wait");
        std::thread::sleep(Duration::from_millis(300));
        s.terminate();
        assert!(matches!(next(&rx), Event::Exit(_)));
        // A process that ignores SIGTERM goes with its group's SIGKILL.
        let (s, rx) = run("trap '' TERM; sleep 300 & wait");
        std::thread::sleep(Duration::from_millis(300));
        s.terminate();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(300)),
            Err(RecvTimeoutError::Timeout),
            "it ignored the TERM"
        );
        s.kill();
        assert!(matches!(next(&rx), Event::Exit(None)));
    }

    #[test]
    fn a_missing_program_is_an_error_not_an_exit() {
        let r = Sidecar::spawn(&["/nonexistent/x".to_string()], &[], |_| {});
        assert!(r.is_err());
    }
}
