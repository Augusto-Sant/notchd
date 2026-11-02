//! The socket server: thread per connection (std only, v1). Each connection
//! reads on its own thread and writes on a second one fed by a channel, so it
//! is the single writer of its stream and a slow UI never blocks the hub.

use crate::hub::Hub;
use crate::instance::Instance;
use crate::router::err;
use notchd_proto::request::ErrReply;
use notchd_proto::version::{MAX_LINE_BYTES, PROTO};
use notchd_proto::{ToDaemon, ToUi, to_line};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub use crate::hub::VERSION;

/// A UI that does not read for this long is dropped rather than waited on.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// A bound listener. Binding refuses a socket another daemon answers on and
/// replaces a stale file.
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    hub: Arc<Hub>,
}

impl Server {
    pub fn bind(instance: Instance) -> Result<Server, String> {
        let path = &instance.socket;
        if let Some(dir) = path.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(format!("a daemon already listens on {}", path.display()));
            }
            fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let listener = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Server {
            listener,
            path: path.clone(),
            hub: Hub::new(&instance.name, &instance.config),
        })
    }

    /// The instance's hub: what the module host (and in-process tests) feed.
    pub fn hub(&self) -> Arc<Hub> {
        Arc::clone(&self.hub)
    }

    /// Accepts forever.
    pub fn serve(self) {
        let hub = Arc::clone(&self.hub);
        thread::spawn(move || hub.run_ticker());
        for stream in self.listener.incoming() {
            let Ok(stream) = stream else { continue };
            let hub = Arc::clone(&self.hub);
            let path = self.path.clone();
            thread::spawn(move || {
                if let Ok(true) = connection(stream, &hub) {
                    // `shutdown`: the reply is flushed. Stop the ticker,
                    // free the socket and exit. (Module processes are not
                    // hosted by the daemon yet; the host stops its own.)
                    hub.stop();
                    let _ = fs::remove_file(&path);
                    std::process::exit(0);
                }
            });
        }
    }
}

/// One line, at most [`MAX_LINE_BYTES`] without the newline. `None` at EOF;
/// `Some(Err)` for an over-long line (the connection is closed: the rest of it
/// is not worth reading).
fn read_line(reader: &mut impl BufRead) -> std::io::Result<Option<Result<String, ()>>> {
    let mut buf = Vec::new();
    let n = reader
        .take(MAX_LINE_BYTES as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() > MAX_LINE_BYTES {
        return Ok(Some(Err(())));
    }
    Ok(Some(Ok(String::from_utf8_lossy(&buf).into_owned())))
}

fn send(out: &mut impl Write, msg: &ToUi) -> std::io::Result<()> {
    let mut line = to_line(msg);
    line.push('\n');
    out.write_all(line.as_bytes())
}

/// The `id` of a line that did not parse as a message, when it has one, so
/// the error can answer the request.
fn salvage_id(line: &str) -> Option<u64> {
    serde_json::from_str::<Value>(line)
        .ok()?
        .get("id")?
        .as_u64()
}

/// `Ok(true)`: the peer asked the daemon to shut down.
fn connection(stream: UnixStream, hub: &Arc<Hub>) -> std::io::Result<bool> {
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut out = stream.try_clone()?;
    let (id, rx) = hub.register();
    let writer = thread::spawn(move || {
        for msg in rx {
            if send(&mut out, &msg).is_err() {
                break;
            }
        }
    });
    let result = serve_lines(stream, hub, id);
    // Closing the channel lets the writer flush what is queued (a refusal, an
    // error) and end.
    hub.unregister(id);
    let _ = writer.join();
    result
}

/// `Ok(true)` when the connection asked for `shutdown`.
fn serve_lines(stream: UnixStream, hub: &Hub, id: u64) -> std::io::Result<bool> {
    let mut reader = BufReader::new(stream);
    loop {
        let line = match read_line(&mut reader)? {
            None => return Ok(false),
            Some(Err(())) => {
                hub.send(
                    id,
                    err(None, "args", format!("line over {MAX_LINE_BYTES} bytes")),
                );
                return Ok(false);
            }
            Some(Ok(line)) => line,
        };
        if line.is_empty() {
            continue;
        }
        let msg = match notchd_proto::parse::<ToDaemon>(&line) {
            Ok(m) => m,
            Err(e) => {
                hub.send(
                    id,
                    err(salvage_id(&line), "args", format!("bad message: {e}")),
                );
                continue;
            }
        };
        if let ToDaemon::Hello(h) = &msg {
            if h.proto != PROTO {
                hub.send(
                    id,
                    ToUi::Err(ErrReply {
                        id: None,
                        code: "proto".into(),
                        message: Some(format!("the daemon speaks proto {PROTO}, not {}", h.proto)),
                        want: Some(PROTO),
                    }),
                );
                return Ok(false);
            }
            hub.subscribe(id);
            continue;
        }
        hub.handle(id, &msg);
        if matches!(&msg, ToDaemon::Ipc(r) if r.method == "shutdown") {
            return Ok(true);
        }
    }
}
