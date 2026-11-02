//! Free surfaces (tier 3): a rectangle the UI hands to something that draws itself.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceKind {
    Terminal,
    Webview,
}

/// Core to UI: a surface came into use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceOpen {
    pub r#ref: String,
    pub kind: SurfaceKind,
    pub cols: u32,
    pub rows: u32,
}

/// `surface-close` (core to UI) and `attach` (UI to core).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceRef {
    pub r#ref: String,
}

/// What happened on a surface. `data` and `title` come from the core; `input`
/// and `resize` from the UI. Bytes are base64.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "lowercase")]
pub enum SurfaceEv {
    Data { b64: String },
    Input { b64: String },
    Resize { cols: u32, rows: u32 },
    Title { text: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMsg {
    pub r#ref: String,
    #[serde(flatten)]
    pub ev: SurfaceEv,
}
