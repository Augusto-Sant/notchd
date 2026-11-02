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

/// The built-in modules' JSON files (`modules/<id>.json`): each one's title,
/// settings fields, defaults and kind. The one source: the modules read their
/// defaults from it and [`catalog`] hands it to the settings schema, so no UI
/// has to ship or send it. `osd` has none: it is a host's (`catalog`).
const INFO: &[(&str, &str)] = &[
    ("activities", include_str!("../modules/activities.json")),
    ("claude", include_str!("../modules/claude.json")),
    ("codex", include_str!("../modules/codex.json")),
    ("codex-live", include_str!("../modules/codex-live.json")),
    ("local-live", include_str!("../modules/local-live.json")),
    ("media", include_str!("../modules/media.json")),
    ("resources", include_str!("../modules/resources.json")),
    ("timer", include_str!("../modules/timer.json")),
];

/// The built-in modules as `catalog` entries (`{ moduleId, title, settings,
/// standalone, agent, live, terminal }`), from their JSON files, in id order.
pub fn catalog() -> Vec<serde_json::Value> {
    use notchd_core::settings::{module_title, parse_module_info};
    INFO.iter()
        .map(|(id, text)| {
            let info = parse_module_info(text);
            let title = module_title(id, Some(&serde_json::json!({ "title": info.title })));
            serde_json::json!({
                "moduleId": id,
                "title": title,
                "settings": info.settings,
                "standalone": info.standalone,
                "agent": info.agent,
                "live": info.live,
                "terminal": info.terminal,
            })
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_has_every_built_in_with_a_json_file_and_not_the_hosts() {
        let ids: Vec<String> = catalog()
            .iter()
            .map(|e| e["moduleId"].as_str().unwrap().to_string())
            .collect();
        let mut expected: Vec<&str> = IDS.iter().copied().filter(|i| *i != "osd").collect();
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    #[test]
    fn an_entry_carries_its_title_kind_and_inherited_fields() {
        let all = catalog();
        let get = |id: &str| all.iter().find(|e| e["moduleId"] == id).unwrap().clone();
        assert_eq!(get("media")["title"], "Now playing");
        assert_eq!(get("media")["agent"], false);
        let claude = get("claude");
        assert_eq!(claude["agent"], true);
        // The fields every agent has come with its file's own.
        assert!(
            claude["settings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["key"] == "command")
        );
        assert_eq!(get("local-live")["live"], true);
    }
}
