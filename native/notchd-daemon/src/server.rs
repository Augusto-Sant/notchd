//! The socket server: thread per connection (std only, v1), each connection
//! the single writer of its own stream.

use crate::instance::Instance;
use crate::router::{err, route};
use notchd_proto::request::ErrReply;
use notchd_proto::ui::{ConfigError, ConfigUpdate, Welcome};
use notchd_proto::version::{MAX_LINE_BYTES, PROTO};
use notchd_proto::{Num, ToDaemon, ToUi, Token, to_line};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A bound listener. Binding refuses a socket another daemon answers on and
/// replaces a stale file.
pub struct Server {
    listener: UnixListener,
    instance: Arc<Instance>,
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
            instance: Arc::new(instance),
        })
    }

    /// Accepts forever.
    pub fn serve(self) {
        for stream in self.listener.incoming() {
            let Ok(stream) = stream else { continue };
            let instance = Arc::clone(&self.instance);
            thread::spawn(move || {
                let _ = connection(stream, &instance);
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

fn connection(stream: UnixStream, instance: &Instance) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let line = match read_line(&mut reader)? {
            None => return Ok(()),
            Some(Err(())) => {
                return send(
                    &mut out,
                    &err(None, "args", format!("line over {MAX_LINE_BYTES} bytes")),
                );
            }
            Some(Ok(line)) => line,
        };
        if line.is_empty() {
            continue;
        }
        let msg = match notchd_proto::parse::<ToDaemon>(&line) {
            Ok(m) => m,
            Err(e) => {
                send(
                    &mut out,
                    &err(salvage_id(&line), "args", format!("bad message: {e}")),
                )?;
                continue;
            }
        };
        if let ToDaemon::Hello(h) = &msg {
            if h.proto != PROTO {
                let refusal = ToUi::Err(ErrReply {
                    id: None,
                    code: "proto".into(),
                    message: Some(format!("the daemon speaks proto {PROTO}, not {}", h.proto)),
                    want: Some(PROTO),
                });
                return send(&mut out, &refusal);
            }
            send(&mut out, &welcome(instance))?;
            send(&mut out, &config_message(&instance.config))?;
            continue;
        }
        for reply in route(&msg) {
            send(&mut out, &reply)?;
        }
    }
}

fn welcome(instance: &Instance) -> ToUi {
    ToUi::Welcome(Welcome {
        proto: PROTO,
        daemon: VERSION.into(),
        instance: instance.name.clone(),
        config: instance.config.display().to_string(),
        tokens: Token::ALL
            .iter()
            .filter_map(|t| serde_json::to_value(t).ok()?.as_str().map(String::from))
            .collect(),
    })
}

/// The `config` message for the file, or `config-error`. A missing file is an
/// empty config (a missing key means its default).
pub fn config_message(path: &Path) -> ToUi {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return ToUi::ConfigError(ConfigError {
                message: format!("{}: {e}", path.display()),
            });
        }
    };
    match notchd_core::config::parse(&text) {
        Ok(map) => {
            let cfg = Value::Object(map);
            let r = notchd_core::config::resolve(&cfg);
            let resolved = serde_json::json!({
                "hoverCloseDelay": Num(r.hover_close_delay),
                "hoverOpenDelay": Num(r.hover_open_delay),
            });
            ToUi::Config(ConfigUpdate {
                rev: 1,
                cfg,
                resolved,
            })
        }
        Err(message) => ToUi::ConfigError(ConfigError {
            message: format!("{}: {message}", path.display()),
        }),
    }
}
