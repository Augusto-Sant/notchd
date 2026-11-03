//! Port of `core/Components.js` (tests: `tests/components.test.mjs`).
//!
//! The rules of the optional components (what the voice modules need beyond
//! the app: LocalAI and its voices, Voxtype, system packages): what a
//! settings row says about one, which buttons it has, which choices wait on an
//! installation, and what a module's error says to do. The registry is
//! `core/components/components.json`; the state comes from the installer
//! (`core/components/install.py status`).
//!
//! The registry and the installer's status are typed ([`Component`],
//! [`ComponentState`]); settings fields and schemas, which belong to the
//! settings rules, stay loose `serde_json::Value`s here.

use std::collections::{BTreeMap, HashMap};

use crate::js;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};

/// The settings page that lists every component (`PAGE`).
pub const PAGE: &str = "components";
/// What a module's error points to (`WHERE`).
pub const WHERE: &str = "Settings › Components";

// ---- JS semantics the files lean on ---------------------------------------

/// A field of the type asked for, else `None`.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Ok(T::deserialize(Value::deserialize(d)?).ok())
}

/// A field of the type asked for, else its default.
fn lenient_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    Ok(T::deserialize(Value::deserialize(d)?).unwrap_or_default())
}

/// A size as JS `Number(x)` reads it (a numeric string too); absent when NaN.
fn de_js_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let n = js::number(&Value::deserialize(d)?);
    Ok((!n.is_nan()).then_some(n))
}

fn de_truthy<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(js::truthy(&Value::deserialize(d)?))
}

/// `x !== false`.
fn de_not_false<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Value::deserialize(d)? != Value::Bool(false))
}

fn de_number<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(js::number_or_zero(&Value::deserialize(d)?))
}

fn default_true() -> bool {
    true
}

// ---- the registry ---------------------------------------------------------

/// The registry as the repo has it (`core/components/components.json`): the
/// daemon is built from the same tree, so it needs no path to read it.
pub const REGISTRY: &str = include_str!("../../../core/components/components.json");

/// How a component is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Files downloaded into notchd's folder.
    Files,
    /// A LocalAI backend and model.
    Localai,
    /// System packages, installed by the system.
    System,
}

/// A file a `files` component downloads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentFile {
    /// Path under the component's folder.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    /// Whether it is made executable.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub exec: Option<bool>,
    /// Where it comes from.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub url: Option<String>,
}

/// The LocalAI model a `localai` component installs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    /// The gallery entry (`localai@...`).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub gallery: Option<String>,
    /// The name LocalAI knows it by.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    /// What the installation overrides (backend, options), loose.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub overrides: Option<Value>,
}

/// One entry of `core/components/components.json`. `id`, `title` and `kind`
/// are required (what [`parse_catalog`] filters on); the rest is read when it
/// is of its type. Keys the rules don't know are kept in `other`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    /// The component's id.
    pub id: String,
    /// What a row is called.
    pub title: String,
    /// How it is installed.
    pub kind: Kind,
    /// The settings group it is listed under.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub group: Option<String>,
    /// Installed from the AUR.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub aur: Option<bool>,
    /// Download size in bytes.
    #[serde(
        default,
        deserialize_with = "de_js_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub size: Option<f64>,
    /// What of `size` another component of the same backend already holds.
    #[serde(
        default,
        deserialize_with = "de_js_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub shared_size: Option<f64>,
    /// What it is.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    /// Ids of the modules that need it.
    #[serde(default, deserialize_with = "lenient_default")]
    pub needed_by: Vec<String>,
    /// System packages (`system`).
    #[serde(
        default,
        deserialize_with = "lenient_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub packages: Vec<String>,
    /// Programs the packages provide (`system`).
    #[serde(
        default,
        deserialize_with = "lenient_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub bins: Vec<String>,
    /// The folder under notchd's own (`files`).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub dir: Option<String>,
    /// Whether removing deletes the whole folder (`files`).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub remove_dir: Option<bool>,
    /// The files to download (`files`).
    #[serde(
        default,
        deserialize_with = "lenient_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub files: Vec<ComponentFile>,
    /// The language of a voice.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub language: Option<String>,
    /// The engine of a voice, `piper` or `pocket`.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub engine: Option<String>,
    /// Pocket TTS's name for the language.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub hf_language: Option<String>,
    /// The component it needs installed first.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub requires: Option<String>,
    /// The LocalAI backend (`localai@cpu-piper`).
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend: Option<String>,
    /// The folder the backend lands in; components sharing it share its size.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub backend_dir: Option<String>,
    /// The model it installs.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub model: Option<ModelSpec>,
    /// What it leaves on disk, for removal.
    #[serde(
        default,
        deserialize_with = "lenient_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub paths: Vec<String>,
    /// The text the voice speaks to warm up.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub warmup: Option<String>,
    /// Any other key of the entry.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

impl Component {
    /// The entry's value for a key, as `c[k]` is in JS (`None` for undefined).
    pub fn get(&self, key: &str) -> Option<Value> {
        serde_json::to_value(self).ok()?.get(key).cloned()
    }
}

/// The registry's file as a catalog (`parseCatalog`): the entries that have a
/// non-empty id, a title and a known kind, in order. Anything else (a bad
/// file) is an empty catalog.
pub fn parse_catalog(text: &str) -> Vec<Component> {
    let Ok(Value::Object(mut data)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(Value::Array(list)) = data.shift_remove("components") else {
        return Vec::new();
    };
    list.into_iter()
        .filter_map(|v| Component::deserialize(v).ok())
        .filter(|c| !c.id.is_empty())
        .collect()
}

/// What the installer says of one component (an entry of `status`). Fields of
/// the wrong type are read as JS would use them: flags by truthiness, numbers
/// as `Number(x) || 0`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ComponentState {
    /// It is there.
    #[serde(default, deserialize_with = "de_truthy")]
    pub installed: bool,
    /// An installation is going on.
    #[serde(default, deserialize_with = "de_truthy")]
    pub active: bool,
    /// Whether it may be removed from the settings (`!== false`).
    #[serde(default = "default_true", deserialize_with = "de_not_false")]
    pub removable: bool,
    /// `queued`, `downloading`, `installing`, `failed`...
    #[serde(default, deserialize_with = "lenient")]
    pub phase: Option<String>,
    /// 0..100 when known (a number; anything else is unknown).
    #[serde(default, deserialize_with = "lenient")]
    pub percent: Option<f64>,
    /// Bytes downloaded.
    #[serde(default, deserialize_with = "de_number")]
    pub done: f64,
    /// Bytes to download.
    #[serde(default, deserialize_with = "de_number")]
    pub total: f64,
    /// Bytes it uses on disk.
    #[serde(default, deserialize_with = "de_number")]
    pub disk: f64,
    /// What the installer is doing.
    #[serde(default, deserialize_with = "lenient")]
    pub detail: Option<String>,
    /// Why it failed.
    #[serde(default, deserialize_with = "lenient")]
    pub error: Option<String>,
}

impl Default for ComponentState {
    /// What a status entry that is present but empty means.
    fn default() -> Self {
        ComponentState {
            installed: false,
            active: false,
            removable: true,
            phase: None,
            percent: None,
            done: 0.0,
            total: 0.0,
            disk: 0.0,
            detail: None,
            error: None,
        }
    }
}

/// The installer's `status` output as `{ id: state }` (`parseStatus`); empty
/// for anything that is not an object. A falsy entry (`null`...) is as no
/// entry, a js::truthy one that is not an object is an empty state.
pub fn parse_status(text: &str) -> BTreeMap<String, ComponentState> {
    let Ok(Value::Object(data)) = serde_json::from_str::<Value>(text) else {
        return BTreeMap::new();
    };
    data.into_iter()
        .filter(|(_, v)| js::truthy(v))
        .map(|(id, v)| {
            let st = match v {
                Value::Object(_) => ComponentState::deserialize(v).unwrap_or_default(),
                _ => ComponentState::default(),
            };
            (id, st)
        })
        .collect()
}

/// A component by id (`byId`).
pub fn by_id<'a>(catalog: &'a [Component], id: &str) -> Option<&'a Component> {
    catalog.iter().find(|c| c.id == id)
}

/// The installer's command line (`installerCommand`): `script` is the file
/// (or a test's fake), a `.py` runs under `python3`.
pub fn installer_command(script: &str, args: &[String]) -> Vec<String> {
    let mut run = if script.ends_with(".py") {
        vec!["python3".to_owned(), script.to_owned()]
    } else {
        vec![script.to_owned()]
    };
    run.extend_from_slice(args);
    run
}

/// A size for people (`formatBytes`): 950 B, 63 MB, 1.5 GB (decimal, as
/// downloads are). Zero for anything that isn't a positive number.
pub fn format_bytes(n: f64) -> String {
    if !n.is_finite() || n <= 0.0 {
        return "0 B".to_owned();
    }
    if n < 1000.0 {
        return format!("{} B", n.round());
    }
    let units = ["kB", "MB", "GB", "TB"];
    let mut v = n;
    let mut i = 0;
    v /= 1000.0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    let text = if v >= 100.0 || i == 0 {
        format!("{}", v.round())
    } else {
        let t = format!("{:.1}", (v * 10.0).round() / 10.0);
        t.strip_suffix(".0")
            .map_or_else(|| t.clone(), str::to_owned)
    };
    format!("{text} {}", units[i])
}

/// What installing a component takes (`sizeToInstall`): its size, less its
/// `shared_size` when another installed component of the same backend
/// (`shared`) already holds that.
pub fn size_to_install(component: &Component, shared: bool) -> f64 {
    let n = component.size.unwrap_or(0.0);
    if shared {
        (n - component.shared_size.unwrap_or(0.0)).max(0.0)
    } else {
        n
    }
}

/// Whether an installed component holds the backend `component` runs on too
/// (`sharesBackend`): another one of the catalog with its `backend_dir`.
pub fn shares_backend(component: &Component, catalog: &[Component], installed: &[String]) -> bool {
    let Some(dir) = &component.backend_dir else {
        return false;
    };
    catalog.iter().any(|o| {
        o.id != component.id && o.backend_dir.as_ref() == Some(dir) && installed.contains(&o.id)
    })
}

// ---- a row ----------------------------------------------------------------

/// What a row shows of a component's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewState {
    /// Not read yet.
    Unknown,
    /// Not installed.
    Missing,
    /// Being installed.
    Working,
    /// There.
    Installed,
    /// The last installation failed.
    Failed,
}

/// A button of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionId {
    /// Install it.
    Install,
    /// Stop installing it.
    Cancel,
    /// Remove it.
    Remove,
}

/// A button: what it does and what it says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Action {
    /// What it does.
    pub id: ActionId,
    /// What it says.
    pub label: String,
}

fn action(id: ActionId, label: &str) -> Action {
    Action {
        id,
        label: label.to_owned(),
    }
}

/// What a row shows of one component (the object `view` returns).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    /// Where the component stands.
    pub state: ViewState,
    /// 0..100 while working and known, else -1 (a bar that only moves).
    pub percent: f64,
    /// The state in words.
    pub text: String,
    /// "about 488 MB" (what it takes to install) or what it uses.
    pub size: String,
    /// The buttons.
    pub actions: Vec<Action>,
}

/// What a row shows of one component (`view`). `st` is the installer's state
/// of it, `None` until read; `shared` is [`shares_backend`].
pub fn view(component: &Component, st: Option<&ComponentState>, shared: bool) -> View {
    let size = format!("about {}", format_bytes(size_to_install(component, shared)));
    let mut out = View {
        state: ViewState::Unknown,
        percent: -1.0,
        text: "Checking…".to_owned(),
        size: size.clone(),
        actions: vec![],
    };
    let Some(st) = st else {
        return out;
    };
    let system = component.kind == Kind::System;
    let removable = st.removable && !system;
    if st.active {
        out.state = ViewState::Working;
        out.percent = st.percent.unwrap_or(-1.0);
        let pct = if out.percent >= 0.0 {
            format!(" {} %", js::num_str(out.percent))
        } else {
            String::new()
        };
        out.text = match st.phase.as_deref() {
            Some("downloading") => {
                if pct.is_empty() {
                    "Downloading…".to_owned()
                } else {
                    let bytes = if st.total > 0.0 {
                        format!(" ({} of {})", format_bytes(st.done), format_bytes(st.total))
                    } else {
                        String::new()
                    };
                    format!("Downloading{pct}{bytes}")
                }
            }
            Some("queued") => "Starting…".to_owned(),
            _ => match st.detail.as_deref().filter(|d| !d.is_empty()) {
                Some(d) => format!("Installing{pct}: {d}"),
                None => format!("Installing{pct}…"),
            },
        };
        out.actions = vec![action(ActionId::Cancel, "Cancel")];
        return out;
    }
    if st.installed {
        out.state = ViewState::Installed;
        out.text = if system {
            "Installed by the system".to_owned()
        } else if st.disk > 0.0 {
            format!("Installed ({})", format_bytes(st.disk))
        } else {
            "Installed".to_owned()
        };
        out.size = if system { size } else { format_bytes(st.disk) };
        out.actions = if removable {
            vec![action(ActionId::Remove, "Remove")]
        } else {
            vec![]
        };
        return out;
    }
    if st.phase.as_deref() == Some("failed") {
        out.state = ViewState::Failed;
        let why = st
            .error
            .as_deref()
            .filter(|e| !e.is_empty())
            .unwrap_or("it stopped");
        out.text = format!("Failed: {why}");
        out.actions = vec![action(ActionId::Install, "Try again")];
        if removable && st.disk > 0.0 {
            out.actions.push(action(ActionId::Remove, "Clean up"));
        }
        return out;
    }
    out.state = ViewState::Missing;
    out.text = "Not installed".to_owned();
    out.actions = vec![action(ActionId::Install, "Install")];
    out
}

/// The ids of the installed components, sorted (`installedIds`): a key that
/// changes only when the set does, so what depends on it is not rebuilt at
/// every progress tick.
pub fn installed_ids(states: &BTreeMap<String, ComponentState>) -> Vec<String> {
    states
        .iter()
        .filter(|(_, s)| s.installed)
        .map(|(id, _)| id.clone())
        .collect()
}

/// Whether any component is being installed (`anyActive`; the window polls
/// while so).
pub fn any_active(states: &BTreeMap<String, ComponentState>) -> bool {
    states.values().any(|s| s.active)
}

/// "Voice and Codex Live" of the module ids that need a component, by their
/// `titles` (`usedBy`; `{ id: title }`, an id without one is its own).
pub fn used_by(component: &Component, titles: &HashMap<String, String>) -> String {
    let names: Vec<&str> = component
        .needed_by
        .iter()
        .map(|id| {
            titles
                .get(id)
                .filter(|t| !t.is_empty())
                .map_or(id.as_str(), String::as_str)
        })
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The help under a row's title: what it is, what it takes, who needs it
/// (`helpText`).
pub fn help_text(component: &Component, titles: &HashMap<String, String>) -> String {
    let by = used_by(component, titles);
    let description = component.description.as_deref().unwrap_or("");
    if by.is_empty() {
        description.to_owned()
    } else {
        format!("{description} For {by}.")
    }
}

// ---- settings fields ------------------------------------------------------

/// A settings field for a component, on `page` under `section` (`field`).
pub fn field(page: &str, section: &str, id: &str) -> Value {
    json!({ "page": page, "section": section, "type": "component", "component": id })
}

/// "They use 1.6 GB in notchd's folders." with something installed, else ""
/// (`diskText`). System packages don't count.
pub fn disk_text(catalog: &[Component], states: &BTreeMap<String, ComponentState>) -> String {
    let total: f64 = catalog
        .iter()
        .filter(|c| c.kind != Kind::System)
        .filter_map(|c| states.get(&c.id))
        .map(|st| st.disk)
        .sum();
    if total > 0.0 {
        format!("They use {} in notchd's folders.", format_bytes(total))
    } else {
        String::new()
    }
}

/// The fields of the overview page (`overviewFields`): a note with what is
/// installed, then every component under its group.
pub fn overview_fields(
    catalog: &[Component],
    states: &BTreeMap<String, ComponentState>,
) -> Vec<Value> {
    let mut out = vec![json!({
        "page": PAGE,
        "type": "note",
        "label": "Optional components",
        "help": format!(
            "Nothing here ships with notchd: install what a module needs, and remove it when you stop using it. {}",
            disk_text(catalog, states)
        ),
    })];
    out.extend(catalog.iter().map(|c| {
        field(
            PAGE,
            c.group
                .as_deref()
                .filter(|g| !g.is_empty())
                .unwrap_or("Other"),
            &c.id,
        )
    }));
    out
}

/// A `where` as the rules build it: a value of `None` is JS `undefined`.
type Where = Vec<(String, Option<Value>)>;

fn matches_where(entry: &Value, where_: &Where) -> bool {
    where_.iter().all(|(k, want)| {
        let have = entry.get(k);
        match want {
            // JS `===` (indexOf too): 2000000 and 2000000.0 are one number.
            Some(Value::Array(list)) => have.is_some_and(|h| list.iter().any(|v| js::eq(v, h))),
            _ => js::eq_opt(have, want.as_ref()),
        }
    })
}

fn matching_where<'a>(catalog: &'a [Component], where_: &Where) -> Vec<&'a Component> {
    catalog
        .iter()
        .filter(|c| serde_json::to_value(c).is_ok_and(|entry| matches_where(&entry, where_)))
        .collect()
}

/// The registry's components that match `where_` (`matching`): every key of it
/// equals the component's (or is one of the values, when it is a list).
pub fn matching<'a>(catalog: &'a [Component], where_: &Map<String, Value>) -> Vec<&'a Component> {
    let w: Where = where_
        .iter()
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect();
    matching_where(catalog, &w)
}

/// A field of type `components` (`{ match, by: { key, field } }`) becomes a
/// row per component that matches (`expand`), each shown while the field
/// `by.key` has the value of the component's `by.field` (the voices of the
/// chosen language), or always without `by`.
pub fn expand(f: &Value, catalog: &[Component]) -> Vec<Value> {
    let empty = Map::new();
    let where_ = f.get("match").and_then(Value::as_object).unwrap_or(&empty);
    matching(catalog, where_)
        .into_iter()
        .map(|c| {
            let mut row = Map::new();
            row.insert("type".into(), json!("component"));
            row.insert("component".into(), json!(c.id));
            for key in ["section", "page"] {
                if let Some(v) = f.get(key).filter(|v| js::truthy(v)) {
                    row.insert(key.into(), v.clone());
                }
            }
            if let Some(by) = f.get("by").filter(|v| js::truthy(v)) {
                let mut when = Map::new();
                when.insert("key".into(), by.get("key").cloned().unwrap_or(Value::Null));
                if let Some(is) = by
                    .get("field")
                    .and_then(Value::as_str)
                    .and_then(|k| c.get(k))
                {
                    when.insert("is".into(), is);
                }
                row.insert("when".into(), Value::Object(when));
            }
            Value::Object(row)
        })
        .collect()
}

/// A field's effective value, by key (`valueOf` of `needed` and `gate`); `None`
/// is undefined.
pub type ValueOf<'a> = &'a dyn Fn(&str) -> Option<Value>;

/// The ids an option's `needs` name for one step of the gating (`needed`):
/// with `strings`, those that are ids (`None` when there is none); else those
/// that are registry matches, whose values may be `"@key"` (that field's
/// value, `value_of`), as the ids of what matches (none: an empty list),
/// `None` when there is no match. A `needs` entry that is neither a string nor
/// an object is skipped.
pub fn needed(
    option: &Value,
    catalog: &[Component],
    value_of: Option<ValueOf>,
    strings: bool,
) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut any = false;
    let needs = option
        .get("needs")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for n in needs {
        match n {
            Value::String(id) => {
                if strings {
                    out.push(id.clone());
                    any = true;
                }
            }
            Value::Object(map) if !strings => {
                any = true;
                let where_: Where = map
                    .iter()
                    .map(|(k, v)| {
                        let want = match (v, value_of) {
                            (Value::String(s), Some(f)) if s.starts_with('@') => f(&s[1..]),
                            _ => Some(v.clone()),
                        };
                        (k.clone(), want)
                    })
                    .collect();
                out.extend(
                    matching_where(catalog, &where_)
                        .into_iter()
                        .map(|c| c.id.clone()),
                );
            }
            _ => {}
        }
    }
    any.then_some(out)
}

/// A choice with the options that wait on a component marked `disabled`, and
/// its help saying which; the field itself when nothing waits (`lock`).
fn lock(
    f: &Value,
    catalog: &[Component],
    installed: &[String],
    ids_of: &dyn Fn(&Value) -> Option<Vec<String>>,
) -> Value {
    let Some(options) = f.get("options").and_then(Value::as_array) else {
        return f.clone();
    };
    let mut notes: Vec<String> = Vec::new();
    let mut changed = false;
    let mut out_options = Vec::with_capacity(options.len());
    for o in options {
        let Some(ids) = ids_of(o) else {
            out_options.push(o.clone());
            continue;
        };
        let missing: Vec<&String> = ids.iter().filter(|id| !installed.contains(id)).collect();
        if !ids.is_empty() && missing.is_empty() {
            out_options.push(o.clone());
            continue;
        }
        for id in missing {
            if let Some(c) = by_id(catalog, id).filter(|c| !notes.contains(&c.title)) {
                notes.push(c.title.clone());
            }
        }
        changed = true;
        let mut disabled = o.clone();
        if let Value::Object(m) = &mut disabled {
            m.insert("disabled".into(), Value::Bool(true));
        }
        out_options.push(disabled);
    }
    if !changed {
        return f.clone();
    }
    let mut out = f.clone();
    out["options"] = Value::Array(out_options);
    if !notes.is_empty() {
        let note = format!(
            "Needs {}: install {} in {WHERE}.",
            notes.join(" and "),
            if notes.len() > 1 { "them" } else { "it" }
        );
        out["help"] = match f
            .get("help")
            .and_then(Value::as_str)
            .filter(|h| !h.is_empty())
        {
            Some(h) => json!(format!("{h} {note}")),
            None => json!(note),
        };
    }
    out
}

/// The schema with its component fields filled in (`decorate`: title as
/// `label`, `help`; a `components` field is a row per component it matches)
/// and its choices that wait on a component marked: an option with
/// `needs: [ids]` is `disabled` while one of them isn't installed, and its
/// field's help says which. Nothing is gated until the state has been read
/// (`installed` is `None`), so a choice isn't locked while it loads. A `needs`
/// that is a match (`{ engine, language: "@voiceLanguage" }`) waits on a
/// field's value: [`gate`]. The schema given is left alone.
pub fn decorate(
    schema: &[Value],
    catalog: &[Component],
    installed: Option<&[String]>,
    titles: &HashMap<String, String>,
) -> Vec<Value> {
    let mut fields = Vec::new();
    for f in schema {
        if f.get("type").and_then(Value::as_str) == Some("components") {
            fields.extend(expand(f, catalog));
        } else {
            fields.push(f.clone());
        }
    }
    fields
        .into_iter()
        .map(|f| {
            if f.get("type").and_then(Value::as_str) == Some("component") {
                let Some(c) = f
                    .get("component")
                    .and_then(Value::as_str)
                    .and_then(|id| by_id(catalog, id))
                else {
                    return f;
                };
                let mut out = f.clone();
                let has = |k: &str| f.get(k).is_some_and(js::truthy);
                if !has("label") {
                    out["label"] = json!(c.title);
                }
                if !has("help") {
                    out["help"] = json!(help_text(c, titles));
                }
                return out;
            }
            match installed {
                Some(installed) if f.get("options").is_some_and(js::truthy) => {
                    lock(&f, catalog, installed, &|o| needed(o, catalog, None, true))
                }
                _ => f,
            }
        })
        .collect()
}

/// The second step of the gating (`gate`), for what depends on a field's
/// value: the options whose `needs` match the registry by `"@key"` are
/// disabled while the matching component (the voice of the chosen language)
/// isn't installed. `value_of(key)` is a field's effective value. Run on a
/// schema [`decorate`] made; it leaves the options without such a `needs`
/// alone, and everything while `installed` is `None`.
pub fn gate(
    schema: &[Value],
    catalog: &[Component],
    installed: Option<&[String]>,
    value_of: ValueOf,
) -> Vec<Value> {
    let Some(installed) = installed else {
        return schema.to_vec();
    };
    schema
        .iter()
        .map(|f| {
            if !f.get("options").is_some_and(js::truthy) {
                return f.clone();
            }
            lock(f, catalog, installed, &|o| {
                needed(o, catalog, Some(value_of), false)
            })
        })
        .collect()
}

/// What a module's error adds when components are missing (`missingHint`):
/// "Install X and Y in Settings › Components." `ids` are the bridge's; titles
/// come from the catalog, an unknown id stays as it is.
pub fn missing_hint(ids: &[String], catalog: &[Component]) -> String {
    let names: Vec<&str> = ids
        .iter()
        .map(|id| by_id(catalog, id).map_or(id.as_str(), |c| c.title.as_str()))
        .collect();
    let list = match names.as_slice() {
        [] => return String::new(),
        [one] => (*one).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    };
    format!("Install {list} in {WHERE}.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn catalog() -> Vec<Component> {
        parse_catalog(include_str!("../../../core/components/components.json"))
    }

    fn find(catalog: &[Component], id: &str) -> Component {
        by_id(catalog, id)
            .unwrap_or_else(|| panic!("no component {id}"))
            .clone()
    }

    fn st(v: Value) -> ComponentState {
        ComponentState::deserialize(v).expect("a state")
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    fn titles(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn action_ids(v: &View) -> Vec<ActionId> {
        v.actions.iter().map(|a| a.id).collect()
    }

    // parseCatalog: the shipped registry, and nothing malformed
    #[test]
    fn parse_catalog_the_shipped_registry_and_nothing_malformed() {
        let catalog = catalog();
        for id in [
            "localai",
            "voxtype",
            "voice-pocket-pt",
            "voice-piper-pt-br",
            "codex-live-audio",
        ] {
            assert!(by_id(&catalog, id).is_some(), "{id}");
        }
        // LocalAI and Voxtype replaced them
        assert!(!catalog.iter().any(|c| ["piper"].contains(&c.id.as_str())
            || c.id.contains("whisper")
            || c.id.contains("chatterbox")));
        assert!(parse_catalog("not json").is_empty());
        let ids: Vec<String> = parse_catalog(
            r#"{"components": [{"id": "a", "title": "A", "kind": "files"}, {"id": "b"}, {"id": "c", "title": "C", "kind": "magic"}, null]}"#,
        )
        .into_iter()
        .map(|c| c.id)
        .collect();
        assert_eq!(ids, ["a"]);
    }

    // parseStatus: an object, else nothing
    #[test]
    fn parse_status_an_object_else_nothing() {
        let status = parse_status(r#"{"a": {"installed": true}}"#);
        assert_eq!(status.len(), 1);
        assert!(status["a"].installed);
        assert!(parse_status("[1]").is_empty());
        assert!(parse_status("").is_empty());
    }

    // installerCommand: a .py runs under python3, a fake as it is
    #[test]
    fn installer_command_a_py_runs_under_python3_a_fake_as_it_is() {
        assert_eq!(
            installer_command("/x/install.py", &strs(&["install", "a"])),
            strs(&["python3", "/x/install.py", "install", "a"])
        );
        assert_eq!(
            installer_command("/t/fake", &strs(&["status"])),
            strs(&["/t/fake", "status"])
        );
    }

    // formatBytes: sizes for people
    #[test]
    fn format_bytes_sizes_for_people() {
        assert_eq!(format_bytes(0.0), "0 B");
        assert_eq!(format_bytes(950.0), "950 B");
        assert_eq!(format_bytes(63_000_000.0), "63 MB");
        assert_eq!(format_bytes(488_000_000.0), "488 MB");
        assert_eq!(format_bytes(1_533_000_000.0), "1.5 GB");
        assert_eq!(format_bytes(6_000_000_000.0), "6 GB");
        assert_eq!(format_bytes(148_000_000.0), "148 MB");
        assert_eq!(format_bytes(-4.0), "0 B");
        // (added) kilobytes are whole, NaN is nothing
        assert_eq!(format_bytes(1_500.0), "2 kB");
        assert_eq!(format_bytes(f64::NAN), "0 B");
    }

    // view: not read yet
    #[test]
    fn view_not_read_yet() {
        let catalog = catalog();
        let v = view(&find(&catalog, "localai"), None, false);
        assert_eq!(v.state, ViewState::Unknown);
        assert!(v.actions.is_empty());
    }

    // view: not installed offers Install and the size
    #[test]
    fn view_not_installed_offers_install_and_the_size() {
        let catalog = catalog();
        let v = view(
            &find(&catalog, "localai"),
            Some(&st(
                json!({"installed": false, "active": false, "phase": ""}),
            )),
            false,
        );
        assert_eq!(v.state, ViewState::Missing);
        assert_eq!(v.text, "Not installed");
        assert_eq!(v.size, "about 197 MB");
        assert_eq!(action_ids(&v), [ActionId::Install]);
    }

    // view: a download shows percent and bytes, and can be cancelled
    #[test]
    fn view_a_download_shows_percent_and_bytes_and_can_be_cancelled() {
        let catalog = catalog();
        let state = st(
            json!({"active": true, "phase": "downloading", "percent": 42, "done": 83_000_000, "total": 196_718_754}),
        );
        let v = view(&find(&catalog, "localai"), Some(&state), false);
        assert_eq!(v.state, ViewState::Working);
        assert_eq!(v.percent, 42.0);
        assert_eq!(v.text, "Downloading 42 % (83 MB of 197 MB)");
        assert_eq!(action_ids(&v), [ActionId::Cancel]);
    }

    // view: work without a percent has a bar that only moves
    #[test]
    fn view_work_without_a_percent_has_a_bar_that_only_moves() {
        let catalog = catalog();
        let piper = find(&catalog, "voice-piper-pt-br");
        let small = find(&catalog, "localai");
        let v = view(
            &piper,
            Some(&st(
                json!({"active": true, "phase": "installing", "percent": null, "detail": "Installing piper-tts"}),
            )),
            false,
        );
        assert_eq!(v.percent, -1.0);
        assert_eq!(v.text, "Installing: Installing piper-tts");
        assert_eq!(
            view(
                &small,
                Some(&st(json!({"active": true, "phase": "queued"}))),
                false
            )
            .text,
            "Starting…"
        );
        assert_eq!(
            view(
                &small,
                Some(&st(json!({"active": true, "phase": "downloading"}))),
                false
            )
            .text,
            "Downloading…"
        );
        assert_eq!(
            view(
                &piper,
                Some(&st(
                    json!({"active": true, "phase": "installing", "percent": 30})
                )),
                false
            )
            .text,
            "Installing 30 %…"
        );
    }

    // view: installed says what it uses and offers Remove
    #[test]
    fn view_installed_says_what_it_uses_and_offers_remove() {
        let catalog = catalog();
        let v = view(
            &find(&catalog, "localai"),
            Some(&st(
                json!({"installed": true, "disk": 196_718_754, "removable": true}),
            )),
            false,
        );
        assert_eq!(v.state, ViewState::Installed);
        assert_eq!(v.text, "Installed (197 MB)");
        assert_eq!(action_ids(&v), [ActionId::Remove]);
    }

    // view: a system package is installed by the system, never removed here
    #[test]
    fn view_a_system_package_is_installed_by_the_system_never_removed_here() {
        let catalog = catalog();
        let pulse = find(&catalog, "libpulse");
        let v = view(
            &pulse,
            Some(&st(
                json!({"installed": true, "removable": false, "disk": 0}),
            )),
            false,
        );
        assert_eq!(v.text, "Installed by the system");
        assert!(v.actions.is_empty());
        let missing = view(
            &pulse,
            Some(&st(json!({"installed": false, "phase": ""}))),
            false,
        );
        assert_eq!(action_ids(&missing), [ActionId::Install]);
    }

    // view: a failure says why, offers a retry, and cleaning up when something is left
    #[test]
    fn view_a_failure_says_why_offers_a_retry_and_cleaning_up_when_something_is_left() {
        let catalog = catalog();
        let small = find(&catalog, "localai");
        let pulse = find(&catalog, "libpulse");
        let base = json!({"installed": false, "active": false, "phase": "failed", "error": "The server answered 404 Not Found.", "disk": 0, "removable": true});
        let v = view(&small, Some(&st(base.clone())), false);
        assert_eq!(v.state, ViewState::Failed);
        assert_eq!(v.text, "Failed: The server answered 404 Not Found.");
        assert_eq!(action_ids(&v), [ActionId::Install]);
        let mut left = base.clone();
        left["disk"] = json!(5000);
        assert_eq!(
            action_ids(&view(&small, Some(&st(left.clone())), false)),
            [ActionId::Install, ActionId::Remove]
        );
        let mut system = left;
        system["disk"] = json!(0);
        system["removable"] = json!(false);
        assert_eq!(
            action_ids(&view(&pulse, Some(&st(system)), false)),
            [ActionId::Install]
        );
        // (added) no error given: it stopped
        let mut silent = base;
        silent["error"] = json!("");
        assert_eq!(
            view(&small, Some(&st(silent)), false).text,
            "Failed: it stopped"
        );
    }

    // installedIds and anyActive
    #[test]
    fn installed_ids_and_any_active() {
        let states = parse_status(
            r#"{"b": {"installed": true}, "a": {"installed": true}, "c": {"installed": false, "active": true}}"#,
        );
        assert_eq!(installed_ids(&states), ["a", "b"]);
        assert!(any_active(&states));
        assert!(!any_active(&parse_status(r#"{"a": {"installed": true}}"#)));
        assert!(!any_active(&BTreeMap::new()));
        assert!(installed_ids(&BTreeMap::new()).is_empty());
    }

    // usedBy and helpText name the modules
    #[test]
    fn used_by_and_help_text_name_the_modules() {
        let catalog = catalog();
        let small = find(&catalog, "localai");
        let pulse = find(&catalog, "libpulse");
        assert_eq!(
            used_by(&small, &titles(&[("local-live", "Voice")])),
            "Voice"
        );
        assert_eq!(
            used_by(
                &pulse,
                &titles(&[("local-live", "Voice"), ("codex-live", "Codex Live")])
            ),
            "Voice and Codex Live"
        );
        assert_eq!(used_by(&pulse, &titles(&[])), "local-live and codex-live");
        let help = help_text(&small, &titles(&[("local-live", "Voice")]));
        assert!(
            help.starts_with("The engine the voices run on") && help.ends_with(" For Voice."),
            "{help}"
        );
    }

    fn s(v: &Value, k: &str) -> Option<String> {
        v.get(k).and_then(Value::as_str).map(str::to_owned)
    }

    // overviewFields: a note, then every component under its group
    #[test]
    fn overview_fields_a_note_then_every_component_under_its_group() {
        let catalog = catalog();
        let states = parse_status(
            r#"{"localai": {"installed": true, "disk": 197000000}, "libpulse": {"installed": true, "disk": 0}}"#,
        );
        let fields = overview_fields(&catalog, &states);
        assert_eq!(s(&fields[0], "type").as_deref(), Some("note"));
        assert!(s(&fields[0], "help").unwrap().contains("They use 197 MB"));
        assert_eq!(fields.len(), catalog.len() + 1);
        assert!(
            fields[1..]
                .iter()
                .all(|f| s(f, "type").as_deref() == Some("component")
                    && s(f, "page").as_deref() == Some("components"))
        );
        // the tools, then a group per language, then Codex Live's
        let mut groups: Vec<String> = Vec::new();
        for f in &fields[1..] {
            let g = s(f, "section").unwrap();
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
        assert_eq!(groups[0], "Voice");
        assert_eq!(groups.last().unwrap(), "Codex Live");
        let languages: HashSet<_> = catalog
            .iter()
            .filter(|c| c.engine.is_some())
            .map(|c| c.language.clone())
            .collect();
        assert_eq!(groups.len(), 2 + languages.len());
        assert!(
            groups[1..groups.len() - 1]
                .iter()
                .all(|g| g.starts_with("Voices: "))
        );
        assert!(
            !s(&overview_fields(&catalog, &BTreeMap::new())[0], "help")
                .unwrap()
                .contains("They use")
        );
    }

    // decorate: component fields get their title and help
    #[test]
    fn decorate_component_fields_get_their_title_and_help() {
        let catalog = catalog();
        let out = decorate(
            &[field("p", "S", "localai")],
            &catalog,
            Some(&[]),
            &titles(&[("local-live", "Voice")]),
        );
        assert_eq!(s(&out[0], "label").as_deref(), Some("LocalAI"));
        assert!(s(&out[0], "help").unwrap().ends_with("For Voice."));
        // an unknown component stays as it is
        assert_eq!(
            decorate(
                &[field("p", "S", "nope")],
                &catalog,
                Some(&[]),
                &titles(&[])
            )[0]
            .get("label"),
            None
        );
    }

    fn tts() -> Value {
        json!({"key": "voiceModel", "type": "choice", "help": "Pick.", "options": [
            {"value": "", "label": "First"},
            {"value": "pocket-pt", "label": "Pocket TTS", "needs": ["voice-pocket-pt"]}]})
    }

    fn by_language() -> Value {
        json!({"key": "voiceModel", "type": "choice", "help": "Pick.", "options": [
            {"value": "", "label": "First"},
            {"value": "piper", "label": "Piper", "needs": [{"engine": "piper", "language": "@lang"}]},
            {"value": "pocket", "label": "Pocket TTS", "needs": [{"engine": "pocket", "language": "@lang"}]}]})
    }

    fn disabled(f: &Value) -> Vec<bool> {
        f["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o.get("disabled") == Some(&json!(true)))
            .collect()
    }

    // decorate: a choice that needs a component is disabled until it is installed
    #[test]
    fn decorate_a_choice_that_needs_a_component_is_disabled_until_it_is_installed() {
        let catalog = catalog();
        let locked =
            decorate(&[tts()], &catalog, Some(&strs(&["localai"])), &titles(&[])).remove(0);
        assert_eq!(locked["options"][0].get("disabled"), None);
        assert_eq!(locked["options"][1]["disabled"], json!(true));
        assert_eq!(
            s(&locked, "help").as_deref(),
            Some(
                "Pick. Needs Voice: Pocket TTS, Portuguese (Brazil): install it in Settings › Components."
            )
        );
        let open = decorate(
            &[tts()],
            &catalog,
            Some(&strs(&["localai", "voice-pocket-pt"])),
            &titles(&[]),
        )
        .remove(0);
        assert_eq!(open["options"][1].get("disabled"), None);
        assert_eq!(s(&open, "help").as_deref(), Some("Pick."));
        // the state not read yet: nothing is locked
        assert_eq!(
            decorate(&[tts()], &catalog, None, &titles(&[]))[0]["options"][1].get("disabled"),
            None
        );
    }

    // decorate: leaves the schema it was given alone
    #[test]
    fn decorate_leaves_the_schema_it_was_given_alone() {
        let catalog = catalog();
        let schema = [tts()];
        let before = serde_json::to_string(&schema).unwrap();
        decorate(&schema, &catalog, Some(&[]), &titles(&[]));
        assert_eq!(serde_json::to_string(&schema).unwrap(), before);
    }

    // missingHint points to the settings, not to make
    #[test]
    fn missing_hint_points_to_the_settings_not_to_make() {
        let catalog = catalog();
        assert_eq!(
            missing_hint(&strs(&["localai"]), &catalog),
            "Install LocalAI in Settings › Components."
        );
        assert_eq!(
            missing_hint(&strs(&["voxtype", "libpulse", "localai"]), &catalog),
            "Install Voxtype, Audio tools and LocalAI in Settings › Components."
        );
        assert_eq!(
            missing_hint(&strs(&["mystery"]), &catalog),
            "Install mystery in Settings › Components."
        );
        assert_eq!(missing_hint(&[], &catalog), "");
    }

    // the registry names modules that exist, and no two components share a file
    #[test]
    fn the_registry_names_modules_that_exist_and_no_two_components_share_a_file() {
        let catalog = catalog();
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../notchd-modules/modules");
        let modules: Vec<String> = std::fs::read_dir(dir)
            .expect("native/notchd-modules/modules")
            .filter_map(|e| {
                e.ok()?
                    .file_name()
                    .to_str()?
                    .strip_suffix(".json")
                    .map(str::to_owned)
            })
            .collect();
        for c in &catalog {
            for m in &c.needed_by {
                assert!(modules.contains(m), "{} needed by {m}", c.id);
            }
        }
        let files: Vec<String> = catalog
            .iter()
            .flat_map(|c| {
                c.files.iter().map(move |f| {
                    format!(
                        "{}/{}",
                        c.dir.as_deref().unwrap_or("undefined"),
                        f.name.as_deref().unwrap_or("undefined")
                    )
                })
            })
            .collect();
        assert_eq!(files.len(), files.iter().collect::<HashSet<_>>().len());
    }

    fn voices(catalog: &[Component]) -> Vec<Component> {
        catalog
            .iter()
            .filter(|c| c.engine.is_some())
            .cloned()
            .collect()
    }

    fn languages(voices: &[Component]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for v in voices {
            let l = v.language.clone().unwrap();
            if !out.contains(&l) {
                out.push(l);
            }
        }
        out
    }

    // the voices: every language has a Piper and a Pocket TTS voice, consistent with its gallery entry and backend
    #[test]
    fn the_voices_every_language_has_piper_and_pocket_consistent_with_gallery_and_backend() {
        let catalog = catalog();
        let voices = voices(&catalog);
        let languages = languages(&voices);
        assert!(
            languages.len() >= 7
                && languages.contains(&"pt".to_owned())
                && languages.contains(&"en".to_owned())
        );
        let mut ids = HashSet::new();
        for code in &languages {
            let engines: Vec<_> = voices
                .iter()
                .filter(|c| c.language.as_ref() == Some(code))
                .map(|c| c.engine.clone().unwrap())
                .collect();
            assert_eq!(engines, ["piper", "pocket"], "{code}");
        }
        for c in &voices {
            assert_eq!(c.kind, Kind::Localai);
            assert!(ids.insert(c.id.clone()), "duplicate {}", c.id);
            assert_eq!(c.requires.as_deref(), Some("localai"));
            // the gallery entry, its backend and the folder the backend lands in agree with the engine
            let backend = if c.engine.as_deref() == Some("piper") {
                "cpu-piper"
            } else {
                "cpu-pocket-tts"
            };
            let model = c.model.as_ref().unwrap();
            assert_eq!(
                c.backend.as_deref(),
                Some(format!("localai@{backend}").as_str())
            );
            assert_eq!(c.backend_dir.as_deref(), Some(backend));
            assert_eq!(model.overrides.as_ref().unwrap()["backend"], json!(backend));
            assert!(model.gallery.as_deref().unwrap().starts_with("localai@"));
            assert!(c.paths.contains(&format!("backends/{backend}")), "{}", c.id);
            assert!(
                c.size.unwrap() > c.shared_size.unwrap() && c.shared_size.unwrap() > 0.0,
                "{}",
                c.id
            );
            assert!(
                c.warmup.as_deref().is_some_and(|w| !w.is_empty()),
                "{}",
                c.id
            );
            assert!(
                c.title.contains(if c.engine.as_deref() == Some("piper") {
                    "Piper"
                } else {
                    "Pocket TTS"
                }),
                "{}",
                c.title
            );
            let first = voices.iter().find(|v| v.language == c.language).unwrap();
            assert_eq!(c.group, first.group);
        }
        // a model name is the one thing LocalAI knows it by: no two voices share one
        let names: Vec<_> = voices
            .iter()
            .map(|c| c.model.as_ref().unwrap().name.clone().unwrap())
            .collect();
        assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
        // Piper: the gallery's voice is the model, named by its language; its files are its own
        for c in voices
            .iter()
            .filter(|v| v.engine.as_deref() == Some("piper"))
        {
            let model = c.model.as_ref().unwrap();
            assert_eq!(
                model.gallery.clone().unwrap(),
                format!("localai@{}", model.name.clone().unwrap())
            );
            let own: Vec<_> = c
                .paths
                .iter()
                .filter(|p| p.starts_with("models/"))
                .collect();
            assert!(own.len() >= 2, "{}", c.id);
            for p in own {
                assert!(
                    !voices.iter().any(|o| o.id != c.id && o.paths.contains(p)),
                    "{p}"
                );
            }
        }
        // Pocket TTS: one model of the shared gallery entry per language, the language and alba in its options, weights in the shared cache
        for c in voices
            .iter()
            .filter(|v| v.engine.as_deref() == Some("pocket"))
        {
            let model = c.model.as_ref().unwrap();
            let hf = c.hf_language.clone().unwrap();
            assert_eq!(model.gallery.as_deref(), Some("localai@pocket-tts"));
            assert_eq!(
                model.name.clone().unwrap(),
                format!("pocket-{}", c.language.clone().unwrap())
            );
            let options = model.overrides.as_ref().unwrap()["options"]
                .as_array()
                .unwrap()
                .clone();
            assert!(
                options.contains(&json!(format!("language:{hf}"))),
                "{}",
                c.id
            );
            assert!(options.contains(&json!("default_voice:alba")), "{}", c.id);
            assert!(hf.ends_with("_24l"));
            assert!(c.paths.contains(&"hf".to_owned()));
            assert!(languages.contains(&c.language.clone().unwrap()));
        }
        let hf: HashSet<_> = voices
            .iter()
            .filter(|v| v.engine.as_deref() == Some("pocket"))
            .map(|c| c.hf_language.clone())
            .collect();
        assert_eq!(hf.len(), languages.len());
    }

    // the engine's backend is shared: the others of its language engine are what the size and the first install count
    #[test]
    fn the_engines_backend_is_shared_the_others_count_for_the_size_and_the_first_install() {
        let catalog = catalog();
        let voices = voices(&catalog);
        let view_size = |c: &Component, shared: bool| {
            view(
                c,
                Some(&st(
                    json!({"installed": false, "active": false, "phase": ""}),
                )),
                shared,
            )
            .size
        };
        for engine in ["piper", "pocket"] {
            let list: Vec<_> = voices
                .iter()
                .filter(|c| c.engine.as_deref() == Some(engine))
                .collect();
            for c in &list {
                let others: Vec<_> = list
                    .iter()
                    .filter(|o| o.id != c.id)
                    .map(|o| o.id.clone())
                    .collect();
                assert!(!shares_backend(c, &catalog, &[]));
                assert!(!shares_backend(c, &catalog, std::slice::from_ref(&c.id))); // itself doesn't count
                assert!(!shares_backend(c, &catalog, &strs(&["localai", "voxtype"])));
                for o in &others {
                    assert!(
                        shares_backend(c, &catalog, std::slice::from_ref(o)),
                        "{} with {o}",
                        c.id
                    );
                }
                // the other engine's voices share nothing with it
                for o in voices
                    .iter()
                    .filter(|v| v.engine.as_deref() != Some(engine))
                {
                    assert!(!shares_backend(c, &catalog, std::slice::from_ref(&o.id)));
                }
                assert_eq!(
                    size_to_install(c, true),
                    c.size.unwrap() - c.shared_size.unwrap()
                );
                assert_eq!(size_to_install(c, false), c.size.unwrap());
                assert!(view_size(c, true).starts_with("about "));
                assert_ne!(view_size(c, true), view_size(c, false));
            }
        }
        // no backend of its own
        let all: Vec<String> = voices.iter().map(|v| v.id.clone()).collect();
        assert!(!shares_backend(&find(&catalog, "localai"), &catalog, &all));
    }

    // a "components" field is a row per voice that matches, shown with the language the field says
    #[test]
    fn a_components_field_is_a_row_per_voice_that_matches_shown_with_the_language_the_field_says() {
        let catalog = catalog();
        let voices = voices(&catalog);
        let languages = languages(&voices);
        let field = json!({"type": "components", "section": "TTS", "page": "p", "match": {"engine": ["piper", "pocket"]}, "by": {"key": "lang", "field": "language"}});
        let rows = decorate(&[field], &catalog, Some(&[]), &titles(&[]));
        let got: Vec<_> = rows.iter().map(|r| s(r, "component").unwrap()).collect();
        let want: Vec<_> = voices.iter().map(|c| c.id.clone()).collect();
        assert_eq!(got, want);
        for r in &rows {
            let c = find(&catalog, &s(r, "component").unwrap());
            assert_eq!(
                [s(r, "type"), s(r, "section"), s(r, "page"), s(r, "label")],
                [
                    Some("component".into()),
                    Some("TTS".into()),
                    Some("p".into()),
                    Some(c.title.clone())
                ]
            );
            assert_eq!(r["when"], json!({"key": "lang", "is": c.language}));
        }
        // narrower matches, and no `by`: always shown
        let piper = decorate(
            &[json!({"type": "components", "match": {"engine": "piper"}})],
            &catalog,
            Some(&[]),
            &titles(&[]),
        );
        assert_eq!(piper.len(), languages.len());
        assert!(piper.iter().all(|r| r.get("when").is_none()));
        assert!(
            decorate(
                &[json!({"type": "components", "match": {"engine": "none"}})],
                &catalog,
                Some(&[]),
                &titles(&[])
            )
            .is_empty()
        );
    }

    // gate: a choice that needs the voice of a chosen language waits on that language's component alone
    #[test]
    fn gate_a_choice_that_needs_the_voice_of_a_chosen_language_waits_on_that_languages_component_alone()
     {
        let catalog = catalog();
        let voices = voices(&catalog);
        let languages = languages(&voices);
        let lang = |code: &str| {
            let code = code.to_owned();
            move |key: &str| (key == "lang").then(|| json!(code))
        };
        let schema = by_language();
        for code in &languages {
            let id = |engine: &str| {
                voices
                    .iter()
                    .find(|c| {
                        c.language.as_ref() == Some(code) && c.engine.as_deref() == Some(engine)
                    })
                    .unwrap()
                    .id
                    .clone()
            };
            let (pocket, piper) = (id("pocket"), id("piper"));
            let gated = |installed: &[String]| {
                gate(
                    std::slice::from_ref(&schema),
                    &catalog,
                    Some(installed),
                    &lang(code),
                )
                .remove(0)
            };
            // nothing installed: both engines wait, "first installed" never does
            assert_eq!(disabled(&gated(&[])), [false, true, true], "{code}");
            // the language's own voice opens its engine, no other language's does
            assert_eq!(
                disabled(&gated(std::slice::from_ref(&piper))),
                [false, false, true],
                "{code}"
            );
            assert_eq!(
                disabled(&gated(std::slice::from_ref(&pocket))),
                [false, true, false],
                "{code}"
            );
            for other in languages.iter().filter(|l| *l != code) {
                let installed: Vec<String> = voices
                    .iter()
                    .filter(|c| c.language.as_ref() == Some(other))
                    .map(|c| c.id.clone())
                    .collect();
                assert_eq!(
                    disabled(&gated(&installed)),
                    [false, true, true],
                    "{code} with {other}"
                );
            }
            // the help names what is missing, in that language
            let help = s(&gated(&[]), "help").unwrap();
            assert!(
                help.contains(&format!(
                    "Needs {} and {}",
                    find(&catalog, &piper).title,
                    find(&catalog, &pocket).title
                )),
                "{help}"
            );
            assert!(
                !s(&gated(&[piper, pocket]), "help")
                    .unwrap()
                    .contains("Needs")
            );
        }
        // an unknown language matches nothing: both wait, nothing to install is named
        let unknown = gate(
            std::slice::from_ref(&schema),
            &catalog,
            Some(&strs(&["localai"])),
            &lang("xx"),
        )
        .remove(0);
        assert_eq!(disabled(&unknown), [false, true, true]);
        assert!(!s(&unknown, "help").unwrap().contains("Needs"));
        // the state not read yet: nothing is locked; decorate leaves these needs to gate
        assert_eq!(
            disabled(&gate(std::slice::from_ref(&schema), &catalog, None, &lang("pt"))[0]),
            [false, false, false]
        );
        assert_eq!(
            disabled(&decorate(&[schema], &catalog, Some(&[]), &titles(&[]))[0]),
            [false, false, false]
        );
        // and gate leaves the plain ids to decorate, and fields without options alone
        let plain = [tts(), json!({"key": "x"})];
        assert_eq!(gate(&plain, &catalog, Some(&[]), &lang("pt")), plain);
    }
}
