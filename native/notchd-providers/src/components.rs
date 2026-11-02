//! The `components` provider: runs the installer (`core/components/install.py`
//! or a test's fake) for the optional components (docs/settings.md,
//! "Components"). `install` starts its work detached and returns, so an
//! installation outlives the call, the daemon and the shell; the state is in
//! the installer's own files, which `status` reads.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long `status` may take.
pub const STATUS_LIMIT: Duration = Duration::from_secs(30);
/// How long `install`, `cancel` and `remove` may take (a removal deletes
/// gigabytes).
pub const ACTION_LIMIT: Duration = Duration::from_secs(600);

/// What a run of the installer gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// `None`: killed for taking too long, or by a signal.
    pub code: Option<i32>,
    pub stdout: String,
}

/// Runs `argv` to its end (or `limit`) with `env` added, its standard output
/// collected. A child that outlives the limit is killed.
pub fn run(argv: &[String], env: &[(String, String)], limit: Duration) -> Result<Run, String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("no installer".into());
    };
    let mut child = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let deadline = Instant::now() + limit;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    Ok(Run {
        code,
        stdout: reader.join().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), script.into()]
    }

    #[test]
    fn it_collects_the_output_and_the_code() {
        let r = run(&sh("echo '{}'; exit 4"), &[], Duration::from_secs(10)).unwrap();
        assert_eq!(
            r,
            Run {
                code: Some(4),
                stdout: "{}\n".into()
            }
        );
    }

    #[test]
    fn it_gets_the_environment_it_is_given() {
        let r = run(
            &sh("echo $WHO"),
            &[("WHO".into(), "me".into())],
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(r.stdout, "me\n");
    }

    #[test]
    fn one_that_takes_too_long_is_killed() {
        let r = run(&sh("sleep 300"), &[], Duration::from_millis(100)).unwrap();
        assert_eq!(r.code, None);
        assert!(run(&["/nonexistent/x".into()], &[], Duration::from_secs(1)).is_err());
    }
}
