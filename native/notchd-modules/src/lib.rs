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
pub mod plan;
pub mod resources;
pub mod term;
pub mod terminal;
pub mod timer;
pub mod tree;
pub mod voice;

pub use module::{CoreModule, Ctx, Effect, Facts};

/// The ids of the modules that live in the daemon. A module moves here (and
/// out of the UI's own list of built-ins) when its logic is ported.
pub const IDS: &[&str] = &[
    "activities",
    "claude",
    "codex",
    "codex-live",
    "local-live",
    "media",
    "osd",
    "resources",
    "timer",
];

/// A fresh instance of a module of [`IDS`].
pub fn create(id: &str) -> Option<Box<dyn CoreModule>> {
    Some(match id {
        "activities" => Box::new(activities::Activities::new()),
        "claude" => Box::new(claude::new()),
        "codex" => Box::new(codex::new()),
        "codex-live" => Box::new(codex_live::new()),
        "local-live" => Box::new(local_live::new()),
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
        "claude" => "Claude",
        "codex" => "Codex",
        "codex-live" => "Codex Live",
        "local-live" => "Voice",
        "media" => "Now playing",
        "osd" => "OSD",
        "resources" => "Resources",
        "timer" => "Timer",
        _ => "",
    }
}
