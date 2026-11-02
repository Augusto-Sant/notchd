//! The `usage` provider: an agent's plan limits, from the endpoint its CLI
//! calls, with the login the CLI keeps (docs/architecture.md, Providers).
//! The rules (the request, what a status means, the limits in an answer) are
//! `notchd-core::usage`; this does the I/O: reads the login file, asks.
//! The login is only read, never refreshed or written.
//!
//! The HTTP call is a trait ([`Http`]) so tests (and anything offline) put a
//! fake in its place and no real login or network is ever touched. The real
//! one runs `curl`, which every OS notchd supports ships, so the daemon has
//! no TLS stack of its own; the token goes to it on its standard input, never
//! on a command line other processes can read.

use notchd_core::usage;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long to wait for an answer: nothing back by then is "offline".
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// What came back. `status` 0: no answer (offline, DNS, timeout).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub status: i64,
    pub body: String,
    /// The `Retry-After` header, `""` when absent.
    pub retry_after: String,
}

pub trait Http: Send + Sync {
    fn get(&self, url: &str, headers: &BTreeMap<String, String>, timeout: Duration) -> Response;
}

/// `curl`, with the request on its standard input.
pub struct Curl;

impl Http for Curl {
    fn get(&self, url: &str, headers: &BTreeMap<String, String>, timeout: Duration) -> Response {
        let mut config = format!("url = \"{}\"\n", quote(url));
        for (k, v) in headers {
            config.push_str(&format!("header = \"{}\"\n", quote(&format!("{k}: {v}"))));
        }
        let child = Command::new("curl")
            .args(["--silent", "--show-error", "--include", "--max-time"])
            .arg(timeout.as_secs().max(1).to_string())
            .args(["--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            return Response::default();
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(config.as_bytes());
        }
        match child.wait_with_output() {
            Ok(out) if out.status.success() => parse_curl(&String::from_utf8_lossy(&out.stdout)),
            _ => Response::default(),
        }
    }
}

/// A value in a curl config file's double quotes.
fn quote(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

/// `curl --include`'s output: the header blocks (one per interim or
/// redirect response), then the body. The last block's status and
/// `Retry-After` count.
pub fn parse_curl(out: &str) -> Response {
    let mut rest = out;
    let mut head = "";
    while rest.starts_with("HTTP/") {
        let (h, tail) = rest
            .split_once("\r\n\r\n")
            .or_else(|| rest.split_once("\n\n"))
            .unwrap_or((rest, ""));
        head = h;
        rest = tail;
    }
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let retry_after = head
        .lines()
        .skip(1)
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("retry-after")
                .then(|| v.trim().to_string())
        })
        .unwrap_or_default();
    Response {
        status,
        body: rest.to_string(),
        retry_after,
    }
}

/// The login file of `provider`: `$CLAUDE_CONFIG_DIR` (else `~/.claude`) and
/// `.credentials.json`, `$CODEX_HOME` (else `~/.codex`) and `auth.json`.
pub fn credentials_path(provider: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let nonempty = |k: &str| env(k).filter(|v| !v.is_empty());
    let home = nonempty("HOME").unwrap_or_default();
    let (var, dir, file) = match provider {
        "claude" => ("CLAUDE_CONFIG_DIR", ".claude", usage::CLAUDE_CREDENTIALS),
        "codex" => ("CODEX_HOME", ".codex", usage::CODEX_CREDENTIALS),
        _ => return None,
    };
    let base = match nonempty(var) {
        Some(d) => d.trim_end_matches('/').to_string(),
        None => format!("{}/{dir}", home.trim_end_matches('/')),
    };
    Some(PathBuf::from(format!("{base}/{file}")))
}

/// When a file was last changed (ms since the epoch), 0 when it is not there.
fn mtime_ms(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// Asks for `provider`'s usage. The answer is the JSON the usage module
/// takes: `{status, body, retry_after, login_mtime}`, `{error, login_mtime}`
/// when there is no login to ask with, or `{unchanged: true, login_mtime}`
/// when `unless_login_is` says to ask only if the login file changed since
/// then (it has not).
pub fn fetch(
    provider: &str,
    unless_login_is: Option<u64>,
    now_ms: u64,
    env: &dyn Fn(&str) -> Option<String>,
    http: &dyn Http,
) -> Value {
    let Some(path) = credentials_path(provider, env) else {
        return json!({"error": format!("no usage for {provider}"), "login_mtime": 0});
    };
    let mtime = mtime_ms(&path);
    if unless_login_is == Some(mtime) {
        return json!({"unchanged": true, "login_mtime": mtime});
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    match usage::request(provider, &text, now_ms as f64) {
        Err(e) => json!({"error": e.error, "login_mtime": mtime}),
        Ok(req) => {
            let r = http.get(&req.url, &req.headers, TIMEOUT);
            json!({"status": r.status, "body": r.body, "retry_after": r.retry_after,
                   "login_mtime": mtime})
        }
    }
}

/// The time, for callers that have only a [`SystemTime`].
pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        seen: Mutex<Vec<(String, BTreeMap<String, String>)>>,
        answer: Response,
    }

    impl Http for Fake {
        fn get(&self, url: &str, headers: &BTreeMap<String, String>, _: Duration) -> Response {
            self.seen
                .lock()
                .unwrap()
                .push((url.into(), headers.clone()));
            self.answer.clone()
        }
    }

    fn fake(status: i64, body: &str) -> Fake {
        Fake {
            seen: Mutex::new(Vec::new()),
            answer: Response {
                status,
                body: body.into(),
                retry_after: String::new(),
            },
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("notchd-usage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_login_file_is_where_the_agents_cli_keeps_it() {
        let env = |k: &str| match k {
            "HOME" => Some("/h".to_string()),
            "CLAUDE_CONFIG_DIR" => Some("/c/".to_string()),
            _ => None,
        };
        assert_eq!(
            credentials_path("claude", &env),
            Some("/c/.credentials.json".into())
        );
        assert_eq!(
            credentials_path("codex", &env),
            Some("/h/.codex/auth.json".into())
        );
        assert_eq!(credentials_path("nope", &env), None);
        let none = |_: &str| None;
        assert_eq!(
            credentials_path("claude", &none),
            Some("/.claude/.credentials.json".into())
        );
    }

    #[test]
    fn it_asks_with_the_token_in_a_header_and_hands_the_answer_back() {
        let dir = scratch("ask");
        std::fs::write(
            dir.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"secret-token","expiresAt":9999999999999}}"#,
        )
        .unwrap();
        let d = dir.display().to_string();
        let env = move |k: &str| (k == "CLAUDE_CONFIG_DIR").then(|| d.clone());
        let http = fake(200, "{\"five_hour\":{}}");
        let r = fetch("claude", None, 1000, &env, &http);
        assert_eq!(r["status"], 200);
        assert_eq!(r["body"], "{\"five_hour\":{}}");
        assert!(r["login_mtime"].as_u64().unwrap() > 0);
        let seen = http.seen.lock().unwrap();
        assert_eq!(seen[0].0, usage::CLAUDE_URL);
        assert_eq!(seen[0].1["Authorization"], "Bearer secret-token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_login_or_with_a_lapsed_one_nothing_is_sent() {
        let dir = scratch("lapsed");
        let d = dir.display().to_string();
        let env = move |k: &str| (k == "CLAUDE_CONFIG_DIR").then(|| d.clone());
        let http = fake(200, "{}");
        let r = fetch("claude", None, 1000, &env, &http);
        assert_eq!(
            (r["error"].as_str(), r["login_mtime"].as_u64()),
            (Some("not signed in"), Some(0))
        );
        std::fs::write(
            dir.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"t","expiresAt":500}}"#,
        )
        .unwrap();
        let r = fetch("claude", None, 1000, &env, &http);
        assert_eq!(r["error"], "login expired");
        // Asked only if the file changed since: it has not.
        let mtime = r["login_mtime"].as_u64().unwrap();
        let r = fetch("claude", Some(mtime), 1000, &env, &http);
        assert_eq!(r["unchanged"], true);
        assert!(http.seen.lock().unwrap().is_empty(), "no request was made");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn curls_output_is_read_into_a_status_the_retry_after_and_a_body() {
        let r = parse_curl("HTTP/2 429 \r\nRetry-After: 900\r\ncontent-type: x\r\n\r\n{\"e\":1}");
        assert_eq!(
            (r.status, r.retry_after.as_str(), r.body.as_str()),
            (429, "900", "{\"e\":1}")
        );
        let r = parse_curl(
            "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nX: y\r\n\r\nbody\r\n\r\nmore",
        );
        assert_eq!((r.status, r.body.as_str()), (200, "body\r\n\r\nmore"));
        assert_eq!(parse_curl("nonsense").status, 0);
        assert_eq!(quote("a\"b\\c\nd"), "a\\\"b\\\\c d");
    }

    /// The real client against a server of its own on the loopback (skipped
    /// without `curl`): what curl prints is what `parse_curl` reads, and the
    /// header reaches the server.
    #[test]
    fn the_real_client_talks_to_a_local_server() {
        if Command::new("curl").arg("--version").output().is_err() {
            eprintln!("NOTE: curl is not installed; the real client was not tested");
            return;
        }
        use std::io::{BufRead, BufReader, Read};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                head.push_str(&line);
            }
            let body = "{\"ok\":true}";
            write!(
                s,
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 42\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            let _ = s.read(&mut [0u8; 1]);
            head
        });
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer abc".to_string());
        let r = Curl.get(
            &format!("http://127.0.0.1:{port}/x"),
            &headers,
            Duration::from_secs(5),
        );
        let head = server.join().unwrap();
        assert_eq!(
            (r.status, r.retry_after.as_str(), r.body.as_str()),
            (429, "42", "{\"ok\":true}")
        );
        assert!(head.contains("Authorization: Bearer abc"), "{head}");
        // Nothing listens: no answer.
        let r = Curl.get(
            &format!("http://127.0.0.1:{port}/x"),
            &headers,
            Duration::from_secs(2),
        );
        assert_eq!(r.status, 0);
    }
}
