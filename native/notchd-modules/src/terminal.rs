//! A terminal of your own (an entry of the config's `terminals`): its
//! command in a terminal in the notch (`term::TermModule`). Each is a module
//! of its own, with the id of its entry; its title and command are the
//! entry's, its other settings are under its id (`directory`, `idleMinutes`,
//! `width`, `height`, `font`). The closed notch shows the title the program
//! gives the terminal, else the module's; without a folder it starts in the
//! home. The command is a shell script: "cd ~/app && npm run dev" runs both.
//! Over IPC: `terminal <id> ...`.

use crate::term::{Kind, TermModule};
use notchd_core::agent::AgentTitle;
use notchd_core::config;
use notchd_core::terminal::plain_title;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Mutex;

/// A program of your own is never "working": its title as it is.
fn title_of(raw: &str) -> AgentTitle {
    AgentTitle {
        working: false,
        title: plain_title(Some(raw)).title,
    }
}

/// The entries of the config that make a terminal (`Config.terminals`), and
/// why the others do not.
pub fn entries(cfg: &Value) -> config::Terminals {
    config::terminals(cfg, config::TERMINAL_MODULE_IDS)
}

/// The ids of the terminals of the config.
pub fn ids(cfg: &Value) -> Vec<String> {
    entries(cfg).list.into_iter().map(|t| t.id).collect()
}

/// An id as the `&'static str` a module names itself by: one allocation per
/// distinct id for the life of the daemon (the few terminals a config ever
/// names), reused after.
pub fn intern(id: &str) -> &'static str {
    static IDS: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut set = IDS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(s) = set.get(id) {
        return s;
    }
    let s: &'static str = Box::leak(id.to_string().into_boxed_str());
    set.insert(s);
    s
}

pub fn new(id: &str) -> TermModule {
    TermModule::new(Kind {
        id: intern(id),
        app: id.to_string(),
        parse_title: title_of,
        default_command: String::new(),
        logo: None,
        own_dir: false,
        script: true,
        ipc: "terminal",
        usage: None,
        custom: true,
    })
}
