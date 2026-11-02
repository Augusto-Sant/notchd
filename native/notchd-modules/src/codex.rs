//! The `codex` module: Codex (OpenAI's coding agent) in the notch, in its own
//! terminal UI (`term::TermModule`). The closed notch shows the thread's
//! title, which Codex makes after the first message, and a dot while it
//! works: both read from the title Codex gives the terminal
//! (`agent::codex_title`).

use crate::term::{Kind, TermModule};
use notchd_core::agent::codex_title;

pub fn new() -> TermModule {
    TermModule::new(Kind {
        id: "codex",
        app: "Codex".into(),
        parse_title: codex_title,
        default_command: "codex".into(),
        logo: None,
        own_dir: true,
        script: false,
        ipc: "agent",
        usage: Some("codex"),
        custom: false,
    })
}
