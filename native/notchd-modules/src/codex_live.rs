//! The `codex-live` module: a spoken conversation with Codex over its
//! realtime voice (`core/live/codex-live.py`, run by the daemon). See
//! [`crate::voice`].

use crate::voice::{Kind, Voice};

pub fn new() -> Voice {
    Voice::new(Kind {
        id: "codex-live",
        local: false,
        info: include_str!("../../../core/modules/codex-live.json"),
    })
}
