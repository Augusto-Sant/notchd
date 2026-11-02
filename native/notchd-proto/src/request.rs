//! Requests (UI, CLI, scripts) and their replies.

use crate::num::Num;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `set`: `Config.withValue` semantics. A `null` value removes the key and
/// empties its parents. Replies `ok` or `err`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Set {
    pub id: u64,
    /// Dotted for nested objects (`theme.accent`).
    pub key: String,
    pub value: Value,
}

/// `edit`: the list edits of `Settings.js` as intent ops; the core computes and
/// writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    pub id: u64,
    #[serde(flatten)]
    pub op: EditOp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum EditOp {
    MoveModule {
        notch: String,
        module: String,
        /// Places to move, negative towards the start.
        delta: i32,
    },
    ToggleModule {
        notch: String,
        module: String,
    },
    ToggleWorkspace {
        notch: String,
        workspace: String,
    },
}

/// `ipc`: a method of docs/ipc.md with its string arguments; the reply is the
/// same string the method returns today.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ipc {
    pub id: u64,
    pub method: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OkReply {
    pub id: u64,
    /// The method's reply string; absent for a `set` or `edit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// `code` is one of [`crate::version::ERR_CODES`]. A refused handshake has no
/// `id` and carries `want`, the major the daemon speaks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub want: Option<u32>,
}

/// The `value` of an `event`: null, a fraction or number, a string, a boolean.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventValue {
    Null(()),
    Bool(bool),
    Number(Num),
    Text(String),
}
