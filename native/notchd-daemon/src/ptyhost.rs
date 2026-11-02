//! The terminals of the daemon: the programs the terminal modules run (the
//! `pty` provider) and what a view attaches to (docs/architecture.md, "Free
//! surface"). A session is the program, the last of its output (so a view that
//! attaches later draws what the earlier one drew), the title it set, and the
//! views attached now, each fed the output as it comes through a queue of its
//! own. A view never holds the program: it goes, the session stays.
//!
//! Locking: a session's state is behind its own mutex and the reader thread
//! only takes that one, so a flood of output never waits for the hub's lock.
//! What a module needs to hear (a new title, the end of the program) goes out
//! as a [`Note`] on a channel the hub drains. The queue to a slow view is
//! bounded, so the program is slowed to the view's pace, as a real terminal
//! does, and memory does not grow.

use notchd_providers::pty::{Pty, PtyEvent, PtySpec};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// How much of the output a session keeps for views that attach later.
pub const RING_BYTES: usize = 1 << 20;
/// Messages queued to a view before the program is made to wait.
const QUEUE: usize = 64;
/// The size a program starts at when no view said otherwise.
pub const DEFAULT_SIZE: (u16, u16) = (80, 24);

/// What a module hears: `provider` is `pty` or `usage`, `data` names the
/// module by `id`.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub provider: &'static str,
    pub data: Value,
}

/// What a view is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Data(Vec<u8>),
    Title(String),
    /// The program ended, or the session was replaced: the view is done.
    Close,
}

/// The last output, bounded.
struct Ring {
    bytes: VecDeque<u8>,
    /// Older output was dropped.
    wrapped: bool,
}

impl Ring {
    fn push(&mut self, data: &[u8]) {
        self.bytes.extend(data);
        if self.bytes.len() > RING_BYTES {
            let excess = self.bytes.len() - RING_BYTES;
            self.bytes.drain(..excess);
            self.wrapped = true;
        }
    }

    /// What to feed a view that attaches now. Once output was dropped the
    /// start is some way into it: it begins after the first line end, with
    /// the attributes reset, so it does not begin inside an escape sequence.
    fn replay(&mut self) -> Vec<u8> {
        let all = self.bytes.make_contiguous();
        if !self.wrapped {
            return all.to_vec();
        }
        let from = all
            .iter()
            .position(|b| *b == b'\n')
            .map_or(all.len(), |i| i + 1);
        let mut out = b"\x1b[0m".to_vec();
        out.extend_from_slice(&all[from..]);
        out
    }
}

/// Reads the titles a program sets (OSC 0 and 2) out of its output, across
/// the chunks it arrives in.
#[derive(Default)]
pub struct Osc {
    state: OscState,
    buf: Vec<u8>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum OscState {
    #[default]
    Ground,
    Esc,
    Osc,
    OscEsc,
}

/// A title is cut here: no terminal shows more.
const MAX_OSC: usize = 4096;

impl Osc {
    /// The titles set in `data`, in order.
    pub fn feed(&mut self, data: &[u8]) -> Vec<String> {
        let mut titles = Vec::new();
        for &b in data {
            match self.state {
                OscState::Ground => {
                    if b == 0x1b {
                        self.state = OscState::Esc;
                    }
                }
                OscState::Esc => match b {
                    b']' => self.begin(),
                    0x1b => {}
                    _ => self.state = OscState::Ground,
                },
                OscState::Osc => match b {
                    0x07 => self.end(&mut titles),
                    0x1b => self.state = OscState::OscEsc,
                    _ => {
                        self.buf.push(b);
                        if self.buf.len() > MAX_OSC {
                            self.state = OscState::Ground;
                        }
                    }
                },
                OscState::OscEsc => match b {
                    b'\\' => self.end(&mut titles),
                    b']' => self.begin(),
                    0x1b => self.state = OscState::Esc,
                    _ => self.state = OscState::Ground,
                },
            }
        }
        titles
    }

    fn begin(&mut self) {
        self.buf.clear();
        self.state = OscState::Osc;
    }

    fn end(&mut self, titles: &mut Vec<String>) {
        self.state = OscState::Ground;
        let text = String::from_utf8_lossy(&self.buf).into_owned();
        if let Some((code, title)) = text.split_once(';')
            && (code == "0" || code == "2")
        {
            titles.push(title.to_string());
        }
    }
}

struct Sub {
    id: u64,
    tx: SyncSender<Out>,
}

struct Inner {
    ring: Ring,
    osc: Osc,
    title: String,
    subs: Vec<Sub>,
    next_sub: u64,
    /// `Some(code)` once the program ended.
    exit: Option<Option<i32>>,
}

/// One program in a terminal and its views.
pub struct Session {
    pub id: String,
    pub run: u64,
    inner: Mutex<Inner>,
    pty: OnceLock<Pty>,
}

/// What a view gets on attaching.
pub struct Attached {
    /// The output so far.
    pub replay: Vec<u8>,
    /// The title so far.
    pub title: String,
    /// The program already ended: the view is closed after the replay.
    pub exited: bool,
    pub sub: u64,
    pub rx: Receiver<Out>,
}

impl Session {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Adds a view: it gets the output so far, then what follows, none
    /// missed and none twice.
    pub fn attach(&self) -> Attached {
        let (tx, rx) = sync_channel(QUEUE);
        let mut g = self.lock();
        let sub = g.next_sub;
        g.next_sub += 1;
        g.subs.push(Sub { id: sub, tx });
        Attached {
            replay: g.ring.replay(),
            title: g.title.clone(),
            exited: g.exit.is_some(),
            sub,
            rx,
        }
    }

    /// A view went away.
    pub fn detach(&self, sub: u64) {
        self.lock().subs.retain(|s| s.id != sub);
    }

    /// How many views are attached.
    pub fn views(&self) -> usize {
        self.lock().subs.len()
    }

    pub fn title(&self) -> String {
        self.lock().title.clone()
    }

    pub fn exited(&self) -> bool {
        self.lock().exit.is_some()
    }

    /// Bytes from a view to the program.
    pub fn input(&self, bytes: &[u8]) {
        if let Some(pty) = self.pty.get() {
            let _ = pty.write(bytes);
        }
    }

    /// A view's size: the program's terminal becomes that size.
    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some(pty) = self.pty.get() {
            let _ = pty.resize(cols, rows);
        }
    }

    /// `(cols, rows)` of the program's terminal.
    pub fn size(&self) -> (u16, u16) {
        self.pty.get().map_or(DEFAULT_SIZE, Pty::size)
    }

    pub fn pid(&self) -> Option<u32> {
        self.pty.get().and_then(Pty::pid)
    }

    /// Stops the program and closes every view (a new session replaces this
    /// one, or the daemon is going).
    pub fn shutdown(&self) {
        if let Some(pty) = self.pty.get() {
            pty.stop();
        }
        let subs: Vec<SyncSender<Out>> = {
            let mut g = self.lock();
            g.subs.drain(..).map(|s| s.tx).collect()
        };
        for tx in subs {
            let _ = tx.try_send(Out::Close);
        }
    }

    fn output(&self, note: &Sender<Note>, data: &[u8]) {
        let (changed, subs) = {
            let mut g = self.lock();
            let titles = g.osc.feed(data);
            g.ring.push(data);
            // Each change counts, not only the last of a chunk: an agent
            // that starts and finishes within one read still went through
            // "working".
            let mut changed = Vec::new();
            for t in titles {
                if t != g.title {
                    g.title = t.clone();
                    changed.push(t);
                }
            }
            (
                changed,
                g.subs.iter().map(|s| s.tx.clone()).collect::<Vec<_>>(),
            )
        };
        for tx in &subs {
            let _ = tx.send(Out::Data(data.to_vec()));
        }
        for t in changed {
            for tx in &subs {
                let _ = tx.send(Out::Title(t.clone()));
            }
            let _ = note.send(Note {
                provider: "pty",
                data: json!({"id": self.id, "event": "title", "run": self.run, "title": t}),
            });
        }
    }

    fn ended(&self, note: &Sender<Note>, code: Option<i32>) {
        let subs: Vec<SyncSender<Out>> = {
            let mut g = self.lock();
            g.exit = Some(code);
            g.subs.iter().map(|s| s.tx.clone()).collect()
        };
        for tx in subs {
            let _ = tx.send(Out::Close);
        }
        let _ = note.send(Note {
            provider: "pty",
            data: json!({"id": self.id, "event": "exit", "run": self.run, "code": code}),
        });
    }
}

/// The sessions of the daemon, one per terminal module.
pub struct PtyHost {
    sessions: std::collections::BTreeMap<String, Arc<Session>>,
    notes: Option<Sender<Note>>,
    /// The size a program starts at: the last view's.
    pub hint: (u16, u16),
}

impl Default for PtyHost {
    fn default() -> PtyHost {
        PtyHost {
            sessions: std::collections::BTreeMap::new(),
            notes: None,
            hint: DEFAULT_SIZE,
        }
    }
}

impl PtyHost {
    pub fn set_notes(&mut self, notes: Sender<Note>) {
        self.notes = Some(notes);
    }

    pub fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.get(id).cloned()
    }

    /// Starts `spec` as the session of `id`, replacing the one it had. A
    /// program that cannot run is an `exit` note with the reason.
    pub fn start(&mut self, id: &str, run: u64, mut spec: PtySpec) {
        self.stop(id);
        let Some(notes) = self.notes.clone() else {
            return;
        };
        spec.cols = self.hint.0;
        spec.rows = self.hint.1;
        let session = Arc::new(Session {
            id: id.to_string(),
            run,
            inner: Mutex::new(Inner {
                ring: Ring {
                    bytes: VecDeque::new(),
                    wrapped: false,
                },
                osc: Osc::default(),
                title: String::new(),
                subs: Vec::new(),
                next_sub: 1,
                exit: None,
            }),
            pty: OnceLock::new(),
        });
        let sink_session = Arc::clone(&session);
        let sink_notes = Mutex::new(notes.clone());
        let sink: notchd_providers::pty::Sink = Arc::new(move |ev| {
            let notes = sink_notes.lock().unwrap_or_else(|p| p.into_inner()).clone();
            match ev {
                PtyEvent::Data(d) => sink_session.output(&notes, &d),
                PtyEvent::Exit(code) => sink_session.ended(&notes, code),
            }
        });
        match Pty::spawn(&spec, sink) {
            Ok(pty) => {
                let _ = session.pty.set(pty);
                self.sessions.insert(id.to_string(), session);
            }
            Err(e) => {
                // Shown as what the program printed, then as ended.
                session.output(&notes, format!("{e}\r\n").as_bytes());
                session.ended(&notes, None);
                self.sessions.insert(id.to_string(), session);
            }
        }
    }

    /// Stops the program of `id` and forgets its session.
    pub fn stop(&mut self, id: &str) {
        if let Some(s) = self.sessions.remove(id) {
            s.shutdown();
        }
    }

    /// Writes bytes as typed to the program of `id`.
    pub fn write(&self, id: &str, bytes: &[u8]) {
        if let Some(s) = self.sessions.get(id) {
            s.input(bytes);
        }
    }

    /// Stops every program (the daemon is going).
    pub fn stop_all(&mut self) {
        for (_, s) in std::mem::take(&mut self.sessions) {
            s.shutdown();
        }
    }

    /// The ids that have a session.
    pub fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    #[test]
    fn titles_are_read_across_chunks_with_either_ending_and_only_0_and_2() {
        let mut o = Osc::default();
        assert_eq!(o.feed(b"hi \x1b]0;a ti"), Vec::<String>::new());
        assert_eq!(o.feed(b"tle\x07 more"), vec!["a title"]);
        assert_eq!(o.feed(b"\x1b]2;two\x1b\\"), vec!["two"]);
        assert_eq!(
            o.feed(b"\x1b]1;icon\x07\x1b]7;file:///x\x07"),
            Vec::<String>::new()
        );
        assert_eq!(
            o.feed("\x1b]0;✳ Claude Code\x07".as_bytes()),
            vec!["✳ Claude Code"]
        );
        // Two in one chunk; a bare escape; a broken sequence does not stick.
        assert_eq!(o.feed(b"\x1b]0;a\x07\x1b[31m\x1b]0;b\x07"), vec!["a", "b"]);
        assert_eq!(o.feed(b"\x1b\x1b]0;c\x07"), vec!["c"]);
        assert_eq!(o.feed(b"\x1b]0;x\x1bQ ok \x1b]0;d\x07"), vec!["d"]);
    }

    #[test]
    fn an_endless_title_is_dropped_not_kept() {
        let mut o = Osc::default();
        let mut long = b"\x1b]0;".to_vec();
        long.extend(std::iter::repeat_n(b'x', MAX_OSC + 10));
        long.push(0x07);
        assert!(o.feed(&long).is_empty());
        assert_eq!(o.feed(b"\x1b]0;ok\x07"), vec!["ok"]);
    }

    #[test]
    fn the_replay_is_all_of_it_until_it_wraps_and_then_starts_at_a_line() {
        let mut r = Ring {
            bytes: VecDeque::new(),
            wrapped: false,
        };
        r.push(b"abc\ndef");
        assert_eq!(r.replay(), b"abc\ndef");
        let big = vec![b'x'; RING_BYTES - 3];
        r.push(&big);
        assert!(r.wrapped);
        assert_eq!(r.bytes.len(), RING_BYTES);
        let replay = r.replay();
        assert!(replay.starts_with(b"\x1b[0m"));
        // The first line was cut off: nothing of it is left but what follows.
        assert!(!replay.windows(3).any(|w| w == b"abc"));
        assert!(replay.len() < RING_BYTES);
    }

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

    fn drain(rx: &Receiver<Out>, until: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let limit = Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while !until(&all) && Instant::now() < limit {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Out::Data(d)) => all.extend(d),
                Ok(Out::Close) => break,
                _ => {}
            }
        }
        all
    }

    #[test]
    fn a_view_that_attaches_late_gets_what_was_drawn_then_what_follows() {
        let (tx, notes) = channel();
        let mut h = PtyHost::default();
        h.set_notes(tx);
        h.start(
            "t",
            1,
            sh("printf 'one\\n'; read x; printf 'two:%s\\n' \"$x\"; read y"),
        );
        let s = h.session("t").unwrap();
        // The title and the first line come as notes and output.
        let limit = Instant::now() + Duration::from_secs(10);
        while !s
            .lock()
            .ring
            .bytes
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .windows(3)
            .any(|w| w == b"one")
            && Instant::now() < limit
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let a = s.attach();
        assert!(String::from_utf8_lossy(&a.replay).contains("one"));
        assert_eq!(s.views(), 1);
        s.input(b"hello\n");
        let live = drain(&a.rx, |d| String::from_utf8_lossy(d).contains("two:hello"));
        assert!(
            String::from_utf8_lossy(&live).contains("two:hello"),
            "{live:?}"
        );
        s.detach(a.sub);
        assert_eq!(s.views(), 0);
        s.input(b"bye\n");
        let _ = notes;
        h.stop_all();
    }

    #[test]
    fn a_title_is_a_note_once_and_the_end_is_a_note_with_the_code() {
        let (tx, notes) = channel();
        let mut h = PtyHost::default();
        h.set_notes(tx);
        h.start(
            "t",
            7,
            sh("printf '\\033]0;same\\007\\033]0;same\\007\\033]0;next\\007'; exit 3"),
        );
        let limit = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while Instant::now() < limit {
            if let Ok(n) = notes.recv_timeout(Duration::from_millis(100)) {
                let done = n.data["event"] == "exit";
                seen.push(n.data);
                if done {
                    break;
                }
            }
        }
        let titles: Vec<&str> = seen
            .iter()
            .filter(|d| d["event"] == "title")
            .map(|d| d["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["same", "next"]);
        let last = seen.last().unwrap();
        assert_eq!(
            (
                last["event"].as_str(),
                last["code"].as_i64(),
                last["run"].as_u64()
            ),
            (Some("exit"), Some(3), Some(7))
        );
        // A view attaching after the end is told so, and gets the title.
        let s = h.session("t").unwrap();
        let a = s.attach();
        assert!(a.exited && a.title == "next");
        h.stop_all();
    }

    #[test]
    fn a_program_that_cannot_run_shows_why_and_is_ended() {
        let (tx, notes) = channel();
        let mut h = PtyHost::default();
        h.set_notes(tx);
        h.start(
            "t",
            1,
            PtySpec {
                program: "/nonexistent/notchd".into(),
                ..sh("")
            },
        );
        let n = notes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(n.data["event"], "exit");
        let a = h.session("t").unwrap().attach();
        assert!(a.exited && String::from_utf8_lossy(&a.replay).contains("cannot run"));
    }

    #[test]
    fn replacing_a_session_closes_its_views_and_stops_its_program() {
        let (tx, _notes) = channel();
        let mut h = PtyHost::default();
        h.set_notes(tx);
        h.start("t", 1, sh("sleep 300"));
        let old = h.session("t").unwrap();
        let a = old.attach();
        let pid = old.pid().unwrap() as i32;
        h.start("t", 2, sh("sleep 300"));
        let limit = Instant::now() + Duration::from_secs(10);
        let mut closed = false;
        while Instant::now() < limit && !closed {
            closed = matches!(
                a.rx.recv_timeout(Duration::from_millis(100)),
                Ok(Out::Close)
            );
        }
        assert!(closed, "the old view was not closed");
        let alive = |p: i32| unsafe { libc::kill(p, 0) == 0 };
        while alive(pid) && Instant::now() < limit {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "the old program is still there");
        assert_eq!(h.session("t").unwrap().run, 2);
        h.stop_all();
    }
}
