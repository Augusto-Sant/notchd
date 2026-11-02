//! Provider traits (docs/architecture.md, Providers); one backend per OS behind cfg(target_os).

pub mod audio;
pub mod components;
pub mod media;
pub mod notifications;
pub mod osd;
pub mod pty;
pub mod resources;
pub mod usage;

#[cfg(target_os = "linux")]
pub mod linux;
