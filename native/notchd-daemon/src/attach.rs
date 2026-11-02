//! A surface connection (docs/architecture.md, "Free surface"): a view of a
//! terminal that lives in the daemon, on a socket of its own so a flood of
//! output never delays a `view` update on the UI's. The first line is
//! `{"t":"attach","ref":"term:claude","cols":..,"rows":..}`; then the daemon
//! sends the output so far and what follows (`surface` messages, `ev: data`
//! with base64 bytes, `title`), and `surface-close` when the program ends or
//! is replaced; the view sends `input` and `resize`.
//!
//! Both ends are here. [`serve`] is the daemon's. [`Client`] is the view's,
//! and `notchd attach <ref>` ([`run`]) is that client as a program on a
//! terminal: how the Linux UI draws a terminal with a widget that can only
//! run a program (`qmltermwidget` has no way to be fed bytes): the widget
//! runs `notchd attach`, whose standard output is what the program printed
//! and whose standard input goes to it. A UI that draws a terminal itself
//! (macOS, Windows) speaks the same lines with [`Client`]'s counterpart.

use crate::hub::Hub;
use crate::ptyhost::Out;
use crate::router::err;
use notchd_proto::surface::{Attach, SurfaceEv, SurfaceMsg, SurfaceRef, b64_decode, b64_encode};
use notchd_proto::{ToDaemon, ToUi, to_line};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A line of output sent to a view at most this long (before base64).
const CHUNK: usize = 32 * 1024;
/// A view that does not read for this long is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

fn line(msg: &ToUi) -> Vec<u8> {
    let mut l = to_line(msg).into_bytes();
    l.push(b'\n');
    l
}

fn data_lines(rf: &str, bytes: &[u8], out: &mut impl Write) -> std::io::Result<()> {
    for chunk in bytes.chunks(CHUNK) {
        out.write_all(&line(&ToUi::Surface(SurfaceMsg {
            r#ref: rf.to_string(),
            ev: SurfaceEv::Data {
                b64: b64_encode(chunk),
            },
        })))?;
    }
    Ok(())
}

/// The daemon's end: serves one view until it or the program is done.
pub fn serve(hub: &Arc<Hub>, stream: UnixStream, mut reader: impl BufRead, req: &Attach) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let attached = match hub.pty_attach(&req.r#ref, req.cols, req.rows) {
        Ok(a) => a,
        Err(e) => {
            let _ = out.write_all(&line(&err(None, "surface", e)));
            return;
        }
    };
    let session = Arc::clone(&attached.session);
    let sub = attached.att.sub;
    let had_output = !attached.att.replay.is_empty();
    let rf = req.r#ref.clone();
    let writer = {
        let rf = rf.clone();
        let stream = stream.try_clone();
        let att = attached.att;
        thread::spawn(move || {
            let write = || -> std::io::Result<()> {
                data_lines(&rf, &att.replay, &mut out)?;
                if !att.title.is_empty() {
                    out.write_all(&line(&ToUi::Surface(SurfaceMsg {
                        r#ref: rf.clone(),
                        ev: SurfaceEv::Title {
                            text: att.title.clone(),
                        },
                    })))?;
                }
                if att.exited {
                    return Ok(());
                }
                for o in att.rx {
                    match o {
                        Out::Data(d) => data_lines(&rf, &d, &mut out)?,
                        Out::Title(t) => out.write_all(&line(&ToUi::Surface(SurfaceMsg {
                            r#ref: rf.clone(),
                            ev: SurfaceEv::Title { text: t },
                        })))?,
                        Out::Close => break,
                    }
                }
                Ok(())
            };
            let _ = write();
            let _ = out.write_all(&line(&ToUi::SurfaceClose(SurfaceRef { r#ref: rf })));
            // The view's reading end is woken: it sees the end.
            if let Ok(s) = stream {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        })
    };
    // The view's size: the program's terminal becomes it. A reattach at the
    // same size sends no resize signal, so the program is made to draw again
    // (a nudge: one column narrower and back).
    if let (Some(c), Some(r)) = (req.cols, req.rows) {
        let (c, r) = (c.clamp(1, 1000) as u16, r.clamp(1, 1000) as u16);
        if session.size() == (c, r) {
            if had_output && !session.exited() && c > 1 {
                let s = Arc::clone(&session);
                thread::spawn(move || {
                    s.resize(c - 1, r);
                    thread::sleep(Duration::from_millis(60));
                    s.resize(c, r);
                });
            }
        } else {
            session.resize(c, r);
        }
    }
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(ToDaemon::Surface(m)) = notchd_proto::parse::<ToDaemon>(buf.trim_end()) else {
            continue;
        };
        match m.ev {
            SurfaceEv::Input { b64 } => {
                if let Some(bytes) = b64_decode(&b64) {
                    session.input(&bytes);
                }
            }
            SurfaceEv::Resize { cols, rows } => {
                session.resize(cols.clamp(1, 1000) as u16, rows.clamp(1, 1000) as u16);
            }
            _ => {}
        }
    }
    hub.pty_detach(&session, sub);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = writer.join();
}

/// What a view hears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Data(Vec<u8>),
    Title(String),
    /// The program ended (or was replaced): the view is done.
    Close,
}

/// The view's end of a surface connection.
pub struct Client {
    writer: Arc<Mutex<UnixStream>>,
    reader: BufReader<UnixStream>,
    rf: String,
}

impl Client {
    pub fn connect(socket: &Path, rf: &str, cols: u32, rows: u32) -> Result<Client, String> {
        let stream = UnixStream::connect(socket)
            .map_err(|e| format!("cannot reach the daemon at {}: {e}", socket.display()))?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let c = Client {
            writer: Arc::new(Mutex::new(stream)),
            reader,
            rf: rf.to_string(),
        };
        c.send(&ToDaemon::Attach(Attach {
            r#ref: rf.to_string(),
            cols: Some(cols),
            rows: Some(rows),
        }))?;
        Ok(c)
    }

    fn send(&self, msg: &ToDaemon) -> Result<(), String> {
        let mut l = to_line(msg);
        l.push('\n');
        self.writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .write_all(l.as_bytes())
            .map_err(|e| e.to_string())
    }

    /// Bytes typed to the program.
    pub fn input(&self, bytes: &[u8]) -> Result<(), String> {
        self.send(&ToDaemon::Surface(SurfaceMsg {
            r#ref: self.rf.clone(),
            ev: SurfaceEv::Input {
                b64: b64_encode(bytes),
            },
        }))
    }

    /// The view's size changed.
    pub fn resize(&self, cols: u32, rows: u32) -> Result<(), String> {
        self.send(&ToDaemon::Surface(SurfaceMsg {
            r#ref: self.rf.clone(),
            ev: SurfaceEv::Resize { cols, rows },
        }))
    }

    /// A handle that sends input and resizes from another thread.
    pub fn sender(&self) -> Sender {
        Sender {
            writer: Arc::clone(&self.writer),
            rf: self.rf.clone(),
        }
    }

    /// The next event: `Ok(None)` after `timeout` with nothing, an error for
    /// a refusal (`no terminal ...`) or a broken connection.
    pub fn next(&mut self, timeout: Duration) -> Result<Option<Event>, String> {
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        loop {
            let mut buf = String::new();
            match self.reader.read_line(&mut buf) {
                Ok(0) => return Ok(Some(Event::Close)),
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(e.to_string()),
            }
            match notchd_proto::parse::<ToUi>(buf.trim_end()) {
                Ok(ToUi::Surface(SurfaceMsg {
                    ev: SurfaceEv::Data { b64 },
                    ..
                })) => return Ok(Some(Event::Data(b64_decode(&b64).unwrap_or_default()))),
                Ok(ToUi::Surface(SurfaceMsg {
                    ev: SurfaceEv::Title { text },
                    ..
                })) => return Ok(Some(Event::Title(text))),
                Ok(ToUi::SurfaceClose(_)) => return Ok(Some(Event::Close)),
                Ok(ToUi::Err(e)) => {
                    return Err(format!("{}: {}", e.code, e.message.unwrap_or_default()));
                }
                _ => {}
            }
        }
    }
}

/// A [`Client`]'s writing end, for another thread.
#[derive(Clone)]
pub struct Sender {
    writer: Arc<Mutex<UnixStream>>,
    rf: String,
}

impl Sender {
    fn send(&self, ev: SurfaceEv) -> Result<(), String> {
        let mut l = to_line(&ToDaemon::Surface(SurfaceMsg {
            r#ref: self.rf.clone(),
            ev,
        }));
        l.push('\n');
        self.writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .write_all(l.as_bytes())
            .map_err(|e| e.to_string())
    }

    pub fn input(&self, bytes: &[u8]) -> Result<(), String> {
        self.send(SurfaceEv::Input {
            b64: b64_encode(bytes),
        })
    }

    pub fn resize(&self, cols: u32, rows: u32) -> Result<(), String> {
        self.send(SurfaceEv::Resize { cols, rows })
    }
}

/// The terminal's size, from the window size ioctl on `fd`.
fn tty_size(fd: i32) -> Option<(u32, u32)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0;
    (ok && ws.ws_col > 0 && ws.ws_row > 0).then_some((u32::from(ws.ws_col), u32::from(ws.ws_row)))
}

/// Raw mode on standard input, undone when dropped.
struct Raw(Option<libc::termios>);

impl Raw {
    fn enter() -> Raw {
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(0, &mut t) } != 0 {
            return Raw(None);
        }
        let saved = t;
        unsafe {
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(0, libc::TCSANOW, &t);
        }
        Raw(Some(saved))
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        if let Some(t) = &self.0 {
            unsafe { libc::tcsetattr(0, libc::TCSANOW, t) };
        }
    }
}

/// `notchd attach <ref>`: a view of the terminal `ref`, on this terminal.
/// Ends when the program does, or the daemon goes.
pub fn run(socket: &Path, rf: &str) -> Result<(), String> {
    let (cols, rows) = tty_size(0).unwrap_or((80, 24));
    let mut client = Client::connect(socket, rf, cols, rows)?;
    let _raw = Raw::enter();
    let sender = client.sender();
    // What is typed goes to the program. The end of it is the end of the view.
    thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if sender.input(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });
    // The size follows the terminal's (looked at, not signalled: no handler
    // to install, and a tenth of a second is not noticed).
    let sizer = client.sender();
    thread::spawn(move || {
        let mut last = (cols, rows);
        loop {
            thread::sleep(Duration::from_millis(100));
            if let Some(now) = tty_size(0)
                && now != last
            {
                last = now;
                if sizer.resize(now.0, now.1).is_err() {
                    break;
                }
            }
        }
    });
    let mut stdout = std::io::stdout();
    loop {
        match client.next(Duration::from_secs(3600))? {
            Some(Event::Data(d)) => {
                if stdout.write_all(&d).and_then(|()| stdout.flush()).is_err() {
                    break;
                }
            }
            Some(Event::Title(_)) | None => {}
            Some(Event::Close) => break,
        }
    }
    Ok(())
}
