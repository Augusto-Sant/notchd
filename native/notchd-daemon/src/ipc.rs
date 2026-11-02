//! `notchd ipc <method> [args...]`: the CLI client. It sends one `ipc` request
//! without a handshake and returns the reply.

use notchd_proto::request::Ipc;
use notchd_proto::{ToDaemon, ToUi, to_line};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// `Ok` is the method's reply string; `Err` is the text for stderr.
pub fn call(socket: &Path, method: &str, args: &[String]) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|e| format!("cannot reach the daemon at {}: {e}", socket.display()))?;
    let timeout = Some(Duration::from_secs(5));
    stream
        .set_read_timeout(timeout)
        .and_then(|_| stream.set_write_timeout(timeout))
        .map_err(|e| e.to_string())?;
    let req = ToDaemon::Ipc(Ipc {
        id: 1,
        method: method.into(),
        args: args.to_vec(),
    });
    writeln!(stream, "{}", to_line(&req)).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("no reply: {e}"))?;
    match notchd_proto::parse::<ToUi>(line.trim_end()) {
        Ok(ToUi::Ok(ok)) => Ok(ok.result.unwrap_or_default()),
        Ok(ToUi::Err(e)) => Err(format!("{}: {}", e.code, e.message.unwrap_or_default())),
        Ok(_) | Err(_) => Err(format!("unexpected reply: {}", line.trim_end())),
    }
}
