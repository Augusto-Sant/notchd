//! notchd's wire protocol (docs/architecture.md): serde types for every message
//! between the daemon and a UI, between the daemon and a user module, the
//! primitive tree, canvas commands and the provider payloads.
//!
//! Conventions, all checked by the fixtures in `tests/protocol/`:
//!
//! - One JSON object per line; `"t"` names the message. Unknown fields are
//!   ignored on input (minor versions add optional fields), never emitted.
//! - Struct fields serialize in declaration order, which is the canonical key
//!   order; free-form values (`serde_json::Value`) serialize with sorted keys.
//! - Numbers that are whole print as integers (`8`, never `8.0`): see [`Num`].
//! - Optional fields are left out when absent; the few the doc shows as `null`
//!   (a `view`'s `tree`, a `state`'s `peek`) are always written.

pub mod canvas;
pub mod common;
pub mod module;
pub mod num;
pub mod provider;
pub mod request;
pub mod settings;
pub mod state;
pub mod surface;
pub mod tree;
pub mod ui;
pub mod version;

pub use canvas::{Cmd, Color};
pub use common::{Align, Edge, Mode, Slot, TextSize, Token, Tone};
pub use module::{FromModule, Manifest, ToModule};
pub use num::Num;
pub use request::{EditOp, EventValue};
pub use tree::Node;
pub use ui::{ToDaemon, ToUi};
pub use version::PROTO;

/// A rule beyond what serde checks (ranges, limits, a node's allowed children).
/// A message that fails it is as invalid as one that does not parse.
pub trait Validate {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Parses one line and validates it.
pub fn parse<T>(line: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned + Validate,
{
    let value: T = serde_json::from_str(line).map_err(|e| e.to_string())?;
    value.validate()?;
    Ok(value)
}

/// Serializes one message as a line (no trailing newline).
pub fn to_line<T: serde::Serialize>(message: &T) -> String {
    serde_json::to_string(message).expect("protocol types always serialize")
}
