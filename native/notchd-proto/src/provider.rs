//! Provider payloads: the facts, one shape across OSes (docs/architecture.md,
//! Providers). They are what a `provider` message's `data` holds and what the
//! official modules consume. `screens` is the [`crate::ui::Screens`] message.

use crate::num::Num;
use serde::{Deserialize, Serialize};

/// `media`: every player and its state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaData {
    pub players: Vec<Player>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    pub id: String,
    pub name: String,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// A path or URL, never inline bytes.
    pub art_url: String,
    /// Seconds.
    pub length: Num,
    /// Seconds.
    pub position: Num,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub can_seek: bool,
}

/// Commands the `media` provider takes (from a module's `event`, not on the wire).
pub const MEDIA_COMMANDS: &[&str] = &["play", "pause", "next", "previous", "seek"];

/// `resources`: CPU as a fraction, memory and disk in bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourcesData {
    pub cpu: Num,
    pub mem: Usage,
    pub disk: DiskUsage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub used: u64,
    pub total: u64,
    pub path: String,
}

/// `usage`: an agent's plan limits (`Usage.js`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageData {
    pub limits: Vec<Limit>,
    pub stale: bool,
    /// What went wrong, `""` when nothing did.
    pub problem: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    /// Stays the same from one answer to the next.
    pub id: String,
    pub label: String,
    /// 0..100.
    pub percent: Num,
    /// Epoch milliseconds, 0 unknown.
    pub resets_at: u64,
    /// 0 unknown.
    pub window_secs: u64,
}

/// `osd`: what the Omarchy host forwards from `omarchy-shell osd show <json>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OsdData {
    pub icon: String,
    pub message: String,
    pub value: Num,
    pub max: Num,
}

/// `notifications`: `notify(title, body)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotifyData {
    pub title: String,
    pub body: String,
}

impl crate::Validate for MediaData {}
impl crate::Validate for ResourcesData {}
impl crate::Validate for UsageData {}
impl crate::Validate for OsdData {}
impl crate::Validate for NotifyData {}
