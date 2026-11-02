//! The `notifications` provider: `notify(title, body)`, shown by the OS. A
//! module only posts one; it never takes the notification service over.

pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// The backend of this OS; `None` where there is none yet.
pub fn system() -> Option<Box<dyn Notifier>> {
    #[cfg(target_os = "linux")]
    return Some(Box::new(crate::linux::notify::NotifySend));
    #[cfg(not(target_os = "linux"))]
    return None;
}
