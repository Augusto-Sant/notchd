//! The `claude` module: Claude Code in the notch, in its own terminal UI
//! (`term::TermModule`). The closed notch shows the conversation's title,
//! which Claude Code makes after the first message, and a dot while it
//! works: both read from the title Claude Code gives the terminal
//! (`agent::claude_title`). The logo is `core/modules/claude.svg` (see
//! NOTICE).

use crate::term::{Kind, TermModule};
use notchd_core::agent::claude_title;

pub fn new() -> TermModule {
    TermModule::new(Kind {
        id: "claude",
        app: "Claude".into(),
        parse_title: claude_title,
        default_command: "claude".into(),
        logo: Some("claude.svg"),
        own_dir: true,
        script: false,
        ipc: "agent",
        usage: Some("claude"),
        custom: false,
    })
}
