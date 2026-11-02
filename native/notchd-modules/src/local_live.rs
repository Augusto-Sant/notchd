//! The `local-live` module (Voice): a spoken conversation that joins an
//! agent's own text session, with the speech recognition (Voxtype) and the
//! voice (a LocalAI model) on this machine (`core/live/local-live.py`, run by
//! the daemon). See [`crate::voice`].

use crate::voice::{Kind, Voice};

pub fn new() -> Voice {
    Voice::new(Kind {
        id: "local-live",
        local: true,
        info: include_str!("../modules/local-live.json"),
    })
}
