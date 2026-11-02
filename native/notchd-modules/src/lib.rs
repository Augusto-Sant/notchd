//! Official modules (docs/architecture.md, Modules): one file each, behind the
//! [`module::CoreModule`] trait the daemon's `corehost` drives.

pub mod activities;
pub mod claude;
pub mod codex;
pub mod codex_live;
pub mod local_live;
pub mod media;
pub mod module;
pub mod osd;
pub mod resources;
pub mod terminal;
pub mod timer;
pub mod tree;

pub use module::{CoreModule, Ctx, Effect, Facts};

/// The ids of the modules that live in the daemon. A module moves here (and
/// out of the UI's own list of built-ins) when its logic is ported.
pub const IDS: &[&str] = &["activities", "media", "osd", "resources", "timer"];

/// A fresh instance of a module of [`IDS`].
pub fn create(id: &str) -> Option<Box<dyn CoreModule>> {
    Some(match id {
        "activities" => Box::new(activities::Activities::new()),
        "media" => Box::new(media::Media::new()),
        "osd" => Box::new(osd::Osd::new()),
        "resources" => Box::new(resources::Resources::new()),
        "timer" => Box::new(timer::Timer::new()),
        _ => return None,
    })
}

/// A module's title, for the tab and the settings page.
pub fn title(id: &str) -> &'static str {
    match id {
        "activities" => "Activities",
        "media" => "Now playing",
        "osd" => "OSD",
        "resources" => "Resources",
        "timer" => "Timer",
        _ => "",
    }
}
