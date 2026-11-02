//! Core to module (user modules): the manifest and the line-JSON on the
//! module's stdin and stdout.

use crate::Validate;
use crate::common::Slot;
use crate::request::EventValue;
use crate::settings::FieldDef;
use crate::tree::Node;
use crate::version::PROTO;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// `module.json`. `command` is required; everything else has a default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// argv, run with the module's folder as cwd; `${config}` and `${cache}`
    /// expand to the module's own dirs.
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urgent_peek: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peek_on_focused_screen: Option<bool>,
    /// Documentation only in v1; shown in the settings page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<FieldDef>,
    /// Provider names pushed to the module.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscribe: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standalone: Option<bool>,
}

impl Validate for Manifest {
    fn validate(&self) -> Result<(), String> {
        if self.command.is_empty() || self.command[0].is_empty() {
            return Err("command must be a non-empty argv".into());
        }
        for name in &self.subscribe {
            if !crate::version::SUBSCRIBABLE.contains(&name.as_str()) {
                return Err(unknown_provider(name));
            }
        }
        Ok(())
    }
}

fn unknown_provider(name: &str) -> String {
    format!(
        "no provider {name:?} for modules (available: {})",
        crate::version::SUBSCRIBABLE.join(", ")
    )
}

/// A module's folder name: letters, digits, `-` and `_`.
pub fn valid_module_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Core to module, on the module's stdin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum ToModule {
    #[serde(rename = "init")]
    Init(Init),
    #[serde(rename = "config")]
    Config(ModuleConfig),
    #[serde(rename = "event")]
    Event(ModuleEvent),
    #[serde(rename = "provider")]
    Provider(ProviderData),
    #[serde(rename = "cmd")]
    Cmd(Cmd),
    #[serde(rename = "visible")]
    Visible(Visible),
    #[serde(rename = "stop")]
    Stop {},
    /// The core refused something the module sent (`provider`: unknown name).
    #[serde(rename = "error")]
    Error(CoreError),
}

impl ToModule {
    pub const TAGS: &'static [&'static str] = &[
        "init", "config", "event", "provider", "cmd", "visible", "stop", "error",
    ];
}

impl Validate for ToModule {
    fn validate(&self) -> Result<(), String> {
        match self {
            ToModule::Init(i) if i.proto != PROTO => {
                Err(format!("init.proto {} is not {PROTO}", i.proto))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Init {
    pub proto: u32,
    pub id: String,
    /// The module's own config (its settings' values), keys sorted.
    pub config: Value,
    pub tokens: Vec<String>,
    pub slots: Vec<Slot>,
    /// The module's folder.
    pub dir: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleConfig {
    pub config: Value,
}

/// An `action` the module's tree defined fired.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleEvent {
    pub slot: Slot,
    pub action: String,
    pub value: EventValue,
}

/// A provider's payload, pushed to a subscriber (shapes in [`crate::provider`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderData {
    pub name: String,
    pub data: Value,
}

/// The module's own IPC: `notchd ipc module <id> <method> args...`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cmd {
    pub method: String,
    pub args: Vec<String>,
}

/// A slot of the module is now shown or hidden.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Visible {
    pub slot: Slot,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoreError {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Module to core, on the module's stdout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum FromModule {
    #[serde(rename = "set")]
    Set(ModuleSet),
    #[serde(rename = "view")]
    View(ModuleView),
    #[serde(rename = "peek")]
    Peek(Peek),
    #[serde(rename = "log")]
    Log(Log),
    #[serde(rename = "error")]
    Error(ModuleError),
    #[serde(rename = "notify")]
    Notify(Notify),
    #[serde(rename = "open")]
    Open {},
    #[serde(rename = "subscribe")]
    Subscribe(ProviderName),
    #[serde(rename = "unsubscribe")]
    Unsubscribe(ProviderName),
}

impl FromModule {
    pub const TAGS: &'static [&'static str] = &[
        "set",
        "view",
        "peek",
        "log",
        "error",
        "notify",
        "open",
        "subscribe",
        "unsubscribe",
    ];
}

impl Validate for FromModule {
    fn validate(&self) -> Result<(), String> {
        match self {
            FromModule::View(v) => v.tree.as_ref().map_or(Ok(()), |t| t.validate()),
            FromModule::Subscribe(p) | FromModule::Unsubscribe(p)
                if !crate::version::SUBSCRIBABLE.contains(&p.name.as_str()) =>
            {
                Err(unknown_provider(&p.name))
            }
            _ => Ok(()),
        }
    }
}

/// The changeable `NotchModule` properties.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urgent_peek: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded_width: Option<u32>,
}

/// The same tree as core to UI `view`, without `module` and `rev`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleView {
    pub slot: Slot,
    pub tree: Option<Node>,
}

/// `requestPeek(ms)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Peek {
    pub ms: u32,
    /// The slot to show; default `peek`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<Slot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Log {
    pub level: LogLevel,
    pub text: String,
}

/// Shows an error row for the module; it keeps running.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleError {
    pub message: String,
}

/// Goes through the `notifications` provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notify {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderName {
    pub name: String,
}
