//! The `pty` provider: a program in a pseudo terminal, which the daemon owns
//! so that it outlives the UI (docs/architecture.md, Providers and "Free
//! surface"). The terminal modules (`claude`, `codex`, your own terminals)
//! start one; a view only ever attaches to it.
//!
//! `portable-pty` does the platform part (`openpty` and a controlling
//! terminal on Unix, ConPTY on Windows) behind one API, so this file is the
//! same on every OS. Output reaches a sink on the reader thread; the program
//! is reaped on a thread of its own, so none is left behind as a zombie, and
//! dropping a [`Pty`] stops the program (its process group: a shell script
//! and what it started).

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

/// What to run, where and at what size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtySpec {
    pub program: String,
    pub args: Vec<String>,
    /// `None`: the daemon's own folder.
    pub cwd: Option<String>,
    /// Set on top of the daemon's environment.
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// What a running program does, as the sink hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PtyEvent {
    /// Bytes it wrote to the terminal.
    Data(Vec<u8>),
    /// The program ended (its exit code when it has one), after the output it
    /// had written was delivered.
    Exit(Option<i32>),
}

/// Called on the reader thread, in order. It must not block for long: a slow
/// sink is what slows the program (a terminal does the same).
pub type Sink = Arc<dyn Fn(PtyEvent) + Send + Sync>;

/// How long output may still arrive after the program ended: a descendant
/// that kept the terminal open would otherwise hold the end back for good.
const DRAIN: Duration = Duration::from_millis(500);
/// How long a stopped program has to end before it is killed.
const GRACE: Duration = Duration::from_secs(2);

pub struct Pty {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
    size: Mutex<(u16, u16)>,
    exited: Arc<AtomicBool>,
}

impl Pty {
    /// Starts the program. The sink hears its output and its end.
    pub fn spawn(spec: &PtySpec, sink: Sink) -> Result<Pty, String> {
        let size = PtySize {
            rows: spec.rows.max(1),
            cols: spec.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| format!("cannot open a terminal: {e}"))?;
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        if let Some(dir) = &spec.cwd {
            cmd.cwd(dir);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("cannot run {}: {e}", spec.program))?;
        // The program has the slave; ours must close, or reading never ends.
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("cannot read the terminal: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("cannot write the terminal: {e}"))?;

        let eof = Arc::new((Mutex::new(false), Condvar::new()));
        let exited = Arc::new(AtomicBool::new(false));
        let reader_eof = Arc::clone(&eof);
        let reader_sink = Arc::clone(&sink);
        thread::Builder::new()
            .name("notchd-pty-read".into())
            .spawn(move || {
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => reader_sink(PtyEvent::Data(buf[..n].to_vec())),
                    }
                }
                let (lock, cv) = &*reader_eof;
                *lock.lock().unwrap_or_else(|p| p.into_inner()) = true;
                cv.notify_all();
            })
            .map_err(|e| e.to_string())?;
        let waiter_exited = Arc::clone(&exited);
        thread::Builder::new()
            .name("notchd-pty-wait".into())
            .spawn(move || {
                let code = child.wait().ok().map(|s| s.exit_code() as i32);
                waiter_exited.store(true, Ordering::SeqCst);
                let (lock, cv) = &*eof;
                let guard = lock.lock().unwrap_or_else(|p| p.into_inner());
                let _ = cv.wait_timeout_while(guard, DRAIN, |done| !*done);
                sink(PtyEvent::Exit(code));
            })
            .map_err(|e| e.to_string())?;
        Ok(Pty {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            pid,
            size: Mutex::new((size.cols, size.rows)),
            exited,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The program ended (its output may still be on its way to the sink).
    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// Bytes to the program, as typed.
    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let mut w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        w.write_all(bytes)
            .and_then(|()| w.flush())
            .map_err(|e| e.to_string())
    }

    /// Tells the program its terminal's size changed.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let (cols, rows) = (cols.max(1), rows.max(1));
        self.master
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        *self.size.lock().unwrap_or_else(|p| p.into_inner()) = (cols, rows);
        Ok(())
    }

    /// `(cols, rows)` as last set.
    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Stops the program: it is asked to hang up (as closing a terminal
    /// does), and killed if it is still there after a grace period. Its
    /// whole process group is meant, so a script's children go too.
    pub fn stop(&self) {
        if self.exited() {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // The session leader's pid is its group's.
            unsafe { libc::kill(-(pid as i32), libc::SIGHUP) };
        }
        let exited = Arc::clone(&self.exited);
        let pid = self.pid;
        let mut killer = self
            .killer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone_killer();
        thread::spawn(move || {
            let until = std::time::Instant::now() + GRACE;
            while std::time::Instant::now() < until {
                if exited.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            }
            let _ = pid;
            let _ = killer.kill();
        });
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    fn sh(script: &str) -> PtySpec {
        PtySpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: None,
            env: vec![("TERM".into(), "xterm-256color".into())],
            cols: 80,
            rows: 24,
        }
    }

    fn start(spec: &PtySpec) -> (Pty, Receiver<PtyEvent>) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let sink: Sink = Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        (Pty::spawn(spec, sink).expect("spawn"), rx)
    }

    /// Reads events until `done` says the text is complete, or 10 s pass.
    fn until(rx: &Receiver<PtyEvent>, mut text: String, done: impl Fn(&str) -> bool) -> String {
        let limit = Instant::now() + Duration::from_secs(10);
        while !done(&text) && Instant::now() < limit {
            if let Ok(PtyEvent::Data(d)) = rx.recv_timeout(Duration::from_millis(100)) {
                text.push_str(&String::from_utf8_lossy(&d));
            }
        }
        text
    }

    fn exit_of(rx: &Receiver<PtyEvent>) -> Option<Option<i32>> {
        let limit = Instant::now() + Duration::from_secs(10);
        while Instant::now() < limit {
            if let Ok(PtyEvent::Exit(c)) = rx.recv_timeout(Duration::from_millis(100)) {
                return Some(c);
            }
        }
        None
    }

    #[test]
    fn a_program_writes_reads_resizes_and_ends() {
        let (pty, rx) = start(&sh(
            "echo ready; stty size; read line; echo got:$line; stty size; exit 7",
        ));
        let text = until(&rx, String::new(), |t| t.contains("24 80"));
        assert!(text.contains("ready") && text.contains("24 80"), "{text:?}");
        pty.write(b"hello\n").unwrap();
        // A change of size arrives as the program's own terminal size.
        pty.resize(100, 30).unwrap();
        assert_eq!(pty.size(), (100, 30));
        let text = until(&rx, text, |t| t.contains("30 100"));
        assert!(text.contains("got:hello"), "{text:?}");
        assert!(text.contains("30 100"), "{text:?}");
        assert_eq!(exit_of(&rx), Some(Some(7)));
        assert!(pty.exited());
    }

    #[test]
    fn the_title_a_program_sets_is_in_its_output() {
        let (_pty, rx) = start(&sh("printf '\\033]0;a title\\007'; echo done"));
        let text = until(&rx, String::new(), |t| t.contains("done"));
        assert!(text.contains("\u{1b}]0;a title\u{7}"), "{text:?}");
        exit_of(&rx);
    }

    #[test]
    fn a_program_that_cannot_run_says_so() {
        let (tx, _rx) = channel::<PtyEvent>();
        let tx = Mutex::new(tx);
        let sink: Sink = Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        let spec = PtySpec {
            program: "/nonexistent/notchd-test".into(),
            ..sh("")
        };
        assert!(Pty::spawn(&spec, sink).is_err());
    }

    #[test]
    fn dropping_it_stops_the_program_and_what_it_started() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("notchd-pty-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pids = dir.join("pids");
        let script = format!("sleep 300 & echo $$ $! > {}; wait", pids.display());
        let (pty, rx) = start(&sh(&script));
        let limit = Instant::now() + Duration::from_secs(10);
        let pids: Vec<i32> = loop {
            // The shell creates/truncates the file before `echo` writes its
            // two PIDs, so existence alone races the write.
            let text = std::fs::read_to_string(&pids).unwrap_or_default();
            let found: Vec<i32> = text
                .split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect();
            if found.len() == 2 {
                break found;
            }
            assert!(Instant::now() < limit, "the shell did not write its PIDs");
            thread::sleep(Duration::from_millis(20));
        };
        drop(pty);
        assert!(exit_of(&rx).is_some(), "the program did not end");
        let limit = Instant::now() + Duration::from_secs(10);
        let alive = |p: i32| unsafe { libc::kill(p, 0) == 0 };
        while pids.iter().any(|p| alive(*p)) && Instant::now() < limit {
            thread::sleep(Duration::from_millis(20));
        }
        // Reaped: no zombie either (kill(pid, 0) succeeds for one).
        assert!(pids.iter().all(|p| !alive(*p)), "left behind: {pids:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flood_of_output_arrives_whole() {
        let (_pty, rx) = start(&sh(
            "head -c 4000000 /dev/zero | tr '\\0' x; echo; echo end",
        ));
        let limit = Instant::now() + Duration::from_secs(20);
        let mut n = 0usize;
        let mut tail = Vec::new();
        while Instant::now() < limit {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(PtyEvent::Data(d)) => {
                    n += d.iter().filter(|b| **b == b'x').count();
                    tail.extend_from_slice(&d);
                    if tail.len() > 16 {
                        tail.drain(..tail.len() - 16);
                    }
                }
                Ok(PtyEvent::Exit(_)) => break,
                Err(_) => {}
            }
        }
        assert_eq!(n, 4_000_000);
        assert!(tail.ends_with(b"end\r\n"), "{tail:?}");
    }
}
