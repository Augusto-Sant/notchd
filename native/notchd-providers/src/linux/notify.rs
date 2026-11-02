//! notify-send (notifications provider), as any app would send a
//! notification. Fire and forget: the process is reaped on a thread of its own.

use crate::notifications::Notifier;
use std::process::{Command, Stdio};

pub struct NotifySend;

impl Notifier for NotifySend {
    fn notify(&self, title: &str, body: &str) {
        let child = Command::new("notify-send")
            .args(["-a", "notchd", "-i", "alarm-symbolic", "--", title, body])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
