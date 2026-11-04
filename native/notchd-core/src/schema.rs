//! The `schema` entity: the settings pages and fields the UI renders,
//! resolved. Ported from `NotchService.qml` (`corePages`, `settingsPages`,
//! `coreSettingsSchema`, `settingsSchema`) over [`crate::settings`] (module
//! pages and fields, `shownFields`, `visiblePages`).
//!
//! The Components page (`components::overview_fields`) and the rows and the
//! locked choices of the module pages (`decorate`, `gate`) are in it, from the
//! registry and what the installer last said (`Inputs::components`). Not in
//! it: the host's own fields (`hostSettingsSchema`). A `choice`'s edge-card
//! `preview` is a QML file, so the UI keys its previews on the field instead.

use crate::components;
use crate::config::{self, NotchSpec};
use crate::js;
use crate::settings;
use notchd_proto::Num;
use notchd_proto::settings::{Display, Field, FieldOption, FieldType, Page, When};
use notchd_proto::ui::Schema;
use serde_json::{Value, json};

/// What the schema is built from.
pub struct Inputs<'a> {
    pub config: &'a Value,
    pub notches: &'a [NotchSpec],
    /// The notch the window edits (`settingsNotch`), by index.
    pub settings_notch: usize,
    /// The screens' names.
    pub screens: &'a [String],
    pub supports_workspaces: bool,
    /// Workspace ids reported by the compositor, for settings UIs that do
    /// not have direct access to its workspace manager.
    pub known_workspaces: &'a [String],
    /// `moduleCatalog`: `{ moduleId, title, settings, standalone }` per module.
    pub catalog: &'a [Value],
    pub default_modules: &'a [&'a str],
    /// The optional components: without them there is no Components page.
    pub components: Option<Components<'a>>,
}

/// The registry and what the installer said of it.
pub struct Components<'a> {
    pub catalog: &'a [components::Component],
    /// `None` until the installer has been asked: nothing is locked meanwhile.
    pub states: Option<&'a std::collections::BTreeMap<String, components::ComponentState>>,
}

/// `corePages`: the sidebar, with the Nerd Font code point of each icon.
const CORE_PAGES: [(&str, &str, u32); 5] = [
    ("general", "General", 0xF0493),
    ("notch", "Notches", 0xF0379),
    ("modules", "Modules", 0xF0A66),
    ("components", "Components", 0xF03D6),
    ("appearance", "Appearance", 0xF0E0C),
];

fn pick<'a>(cfg: &'a Value, key: &str) -> Option<&'a Value> {
    config::value_at(cfg, key)
}

/// `coreSettingsSchema`, as the loose objects `Settings.js` works on.
fn core_fields(i: &Inputs) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let n = i
        .notches
        .get(i.settings_notch.min(i.notches.len().saturating_sub(1)));
    let prefix = n.map_or("", |n| n.prefix.as_str());
    let key = |k: &str| format!("{prefix}{k}");
    let inherited = |k: &str, fallback: Value| -> Value {
        if prefix.is_empty() {
            return fallback;
        }
        pick(i.config, k).cloned().unwrap_or(fallback)
    };
    let several = i.notches.len() > 1;
    let own = settings::notch_page_id(n.map_or(0, |n| n.index) as u64);
    let default_of = |k: &str| config::DEFAULTS.get(k).cloned().unwrap_or(Value::Null);
    let standalone: Vec<&Value> = i
        .catalog
        .iter()
        .filter(|m| js::truthy_opt(m.get("standalone")))
        .collect();

    out.push(json!({"page": "notch", "key": "notches", "type": "notches", "label": "Notches",
        "help": if several {
            "Each has its own modules and place; the gear opens its settings."
        } else {
            "The gear opens the notch's settings. Add a notch to show other modules somewhere else, each with its own place."
        }}));
    if !prefix.is_empty()
        && let Some(n) = n
    {
        out.push(json!({"page": own, "key": key("id"), "type": "string", "label": "Name",
            "default": config::default_notch_id(n.index),
            "help": "What IPC calls it: openNotch, closeNotch and toggleNotch take it, so renaming it breaks a shortcut that names it. An agent opens by its own id instead: agent <id> toggle."}));
    }
    let modules_help = format!(
        "{}{}",
        if several {
            "What this notch shows. A module may show in several notches. Their order breaks ties in the expanded view."
        } else {
            "Switched-on modules load. Their order breaks ties in the expanded view."
        },
        if standalone.is_empty() {
            String::new()
        } else {
            let titles: Vec<String> = standalone
                .iter()
                .map(|m| settings::module_title(&js::string(&m["moduleId"]), Some(m)))
                .collect();
            format!(
                " Standalone modules, such as {}, have a place of their own: see the Modules page.",
                titles.join(", ")
            )
        }
    );
    out.push(
        json!({"page": own, "section": "Modules", "key": key("modules"), "type": "modules",
        "label": "Loaded modules", "default": inherited("modules", json!(i.default_modules)),
        "help": modules_help}),
    );
    out.push(json!({"page": own, "section": "Layout", "key": key("reserveSpace"), "type": "boolean",
        "label": "Keep windows clear", "default": inherited("reserveSpace", default_of("reserveSpace")),
        "help": "Windows keep clear of the closed notch instead of going under it, along its whole edge: a sliver at rest, more while a module is active. Peeks and the open notch still cover them."}));
    out.push(json!({"page": own, "section": "Layout", "key": key("tabs"), "type": "boolean",
        "label": "Tabs", "default": inherited("tabs", default_of("tabs")),
        "help": "Open, the notch shows one module at a time, with a tab for each, instead of all of them one above the other; closed, every active module side by side. Opening the notch from a module (a shortcut, IPC) picks its tab. Only a notch with several modules has tabs."}));
    out.push(json!({"page": own, "section": "Layout", "key": key("idleIcons"), "type": "boolean",
        "label": "Icons for idle modules", "default": inherited("idleIcons", default_of("idleIcons")),
        "help": "A module with nothing to show closed, like a timer that isn't running, shows its icon in the closed notch. Off, it stays out until it has something."}));
    let edges: Vec<Value> = ["top", "bottom", "left", "right"]
        .iter()
        .map(|e| json!({"value": e, "label": capitalize(e)}))
        .collect();
    out.push(json!({"page": own, "section": "Position", "key": key("position.edge"), "type": "choice",
        "display": "cards", "label": "Edge", "default": inherited("position.edge", json!("top")),
        "help": "The screen edge the notch hangs from. On a side edge, an active module without a vertical view shows a dot.",
        "options": edges}));
    out.push(json!({"page": own, "section": "Position", "key": key("position.offset"), "type": "number",
        "display": "percent", "label": "Along the edge", "default": inherited("position.offset", json!(0.5)),
        "min": 0, "max": 1, "step": 0.01,
        "help": "Where on the edge: 0 % is its left (or top) end, 50 % the middle."}));
    if i.screens.len() > 1 {
        let mut options = vec![
            json!({"value": "", "label": "Follow focus"}),
            json!({"value": "all", "label": "Every screen"}),
        ];
        options.extend(i.screens.iter().map(|s| json!({"value": s, "label": s})));
        out.push(json!({"page": own, "section": "Position", "key": key("position.screen"), "type": "choice",
            "label": "Screen", "default": inherited("position.screen", json!("")),
            "help": "Where the notch stays. Following focus, it moves to the screen you work on. On a screen of its own, it stays there, peeks included; on a screen without a notch that shows it, a peek appears at the top. On every screen, each screen has its own copy, and a peek shows on the focused one.",
            "options": options}));
    }
    out.push(json!({"page": own, "section": "Position", "key": key("draggable"), "type": "boolean",
        "label": "Drag to move", "default": inherited("draggable", default_of("draggable")),
        "help": "Hold the notch for a moment, then drag it to any edge of the screen; it snaps to the nearest one, and to the middle when close to it. A quick click only pins it."}));
    if i.supports_workspaces {
        let choices: Vec<Value> = i
            .known_workspaces
            .iter()
            .map(|w| json!({"value": w, "label": w}))
            .collect();
        out.push(json!({"page": own, "section": "Layout", "key": key("workspaces"), "type": "workspaces",
            "label": "Workspaces", "default": inherited("workspaces", Value::Null),
            "options": choices,
            "help": "Where the closed notch shows; none picked means every workspace. Peeks and the open notch show on any."}));
    }
    out.push(
        json!({"page": "general", "section": "Hover", "key": "hoverOpenDelay", "type": "integer",
        "label": "Open delay",
        "help": if several {
            "How long the pointer rests on a notch before it expands. For every notch."
        } else {
            "How long the pointer rests on the notch before it expands."
        },
        "default": default_of("hoverOpenDelay"), "min": 0, "max": 1000, "step": 10, "unit": "ms"}),
    );
    out.push(
        json!({"page": "general", "section": "Hover", "key": "hoverCloseDelay", "type": "integer",
        "label": "Close delay",
        "help": if several {
            "How long it stays open after the pointer leaves. For every notch."
        } else {
            "How long it stays open after the pointer leaves."
        },
        "default": default_of("hoverCloseDelay"), "min": 0, "max": 2000, "step": 10, "unit": "ms"}),
    );
    out.push(json!({"page": "modules", "type": "moduleCatalog", "part": "notch", "label": "Notch modules",
        "help": "Shown by notches: which ones a notch shows is set on its page, under Notches. The gear opens a module's own settings."}));
    if i.catalog
        .iter()
        .any(|m| settings::module_kind(m) == "agent")
    {
        out.push(json!({"page": "modules", "type": "moduleCatalog", "part": "agent", "label": "Agents",
            "help": "Coding agents, each in its own terminal. Notches show them like any module; a notch of their own suits them best. The gear opens an agent's folder, command and size. A shortcut opens one with agent <id> toggle."}));
    }
    out.push(json!({"page": "modules", "type": "moduleCatalog", "part": "terminal", "label": "Terminals",
        "help": "Commands of your own, each in a terminal: btop, a dev server, a shell. Add terminal makes one running your shell; its gear changes its command, folder and size. A notch shows it once the notch's page lists it. A shortcut opens one with terminal <id> toggle."}));
    if !standalone.is_empty() {
        out.push(json!({"page": "modules", "type": "moduleCatalog", "part": "standalone", "label": "Standalone modules",
            "help": "Each has a place of its own, over any notch that hangs there, and no notch shows it. Its page switches it on and places it."}));
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn num(v: Option<&Value>) -> Option<Num> {
    v.and_then(Value::as_f64).map(Num)
}

fn text(v: Option<&Value>) -> Option<String> {
    v.filter(|v| js::truthy(v)).map(js::string)
}

/// A loose field as the wire's [`Field`]; `None` for a type the protocol does
/// not have. `value` is the config's value, else the default.
fn to_field(f: &Value, shown: bool, value_of: &dyn Fn(&str) -> Option<Value>) -> Option<Field> {
    let ty: FieldType = serde_json::from_value(f.get("type")?.clone()).ok()?;
    let key = text(f.get("key"));
    let default = f.get("default").cloned().unwrap_or(Value::Null);
    let value = key
        .as_deref()
        .and_then(value_of)
        .unwrap_or_else(|| default.clone());
    let options = match f.get("options") {
        Some(Value::Array(list)) => Some(
            list.iter()
                .map(|o| FieldOption {
                    value: o.get("value").cloned().unwrap_or(Value::Null),
                    label: o.get("label").map(js::string).unwrap_or_default(),
                    preview: text(o.get("preview")),
                    disabled: o.get("disabled").and_then(Value::as_bool).filter(|d| *d),
                })
                .collect(),
        ),
        _ => None,
    };
    let display = match f.get("display").and_then(Value::as_str) {
        Some("cards") => Some(Display::Cards),
        Some("chips") => Some(Display::Chips),
        Some("percent") => Some(Display::Percent),
        _ => None,
    };
    let when = f.get("when").filter(|w| js::truthy(w)).map(|w| When {
        key: w.get("key").map(js::string).unwrap_or_default(),
        is: w.get("is").cloned().unwrap_or(Value::Null),
    });
    Some(Field {
        page: text(f.get("page")).unwrap_or_else(|| "general".into()),
        section: text(f.get("section")).unwrap_or_default(),
        key,
        r#type: ty,
        label: f.get("label").map(js::string).unwrap_or_default(),
        help: text(f.get("help")),
        options,
        display,
        min: num(f.get("min")),
        max: num(f.get("max")),
        step: num(f.get("step")),
        unit: text(f.get("unit")),
        default,
        value,
        visible: shown,
        locked: None,
        when,
        component: text(f.get("component")),
        row: None,
        part: text(f.get("part")),
    })
}

/// The whole schema (rev 0; the store numbers it as it sends it).
pub fn build(i: &Inputs) -> Schema {
    let mut loose = core_fields(i);
    loose.extend(settings::module_fields(i.catalog, ""));
    let empty = std::collections::BTreeMap::new();
    let mut shared_of: Box<dyn Fn(&components::Component) -> bool> = Box::new(|_| false);
    if let Some(c) = &i.components {
        let states = c.states.unwrap_or(&empty);
        loose.extend(components::overview_fields(c.catalog, states));
        let titles: std::collections::HashMap<String, String> = i
            .catalog
            .iter()
            .filter_map(|m| {
                let id = m.get("moduleId")?.as_str()?;
                Some((id.to_string(), settings::module_title(id, Some(m))))
            })
            .collect();
        let installed: Option<Vec<String>> = c.states.map(components::installed_ids);
        loose = components::decorate(&loose, c.catalog, installed.as_deref(), &titles);
        // The choices that wait on the component of a field's value (the
        // voice of the chosen language), by the config's value or the default.
        let base = loose.clone();
        let value_of = |k: &str| pick(i.config, k).cloned();
        let effective = |k: &str| settings::effective_value(&base, k, value_of);
        loose = components::gate(&loose, c.catalog, installed.as_deref(), &effective);
        let list = installed.clone().unwrap_or_default();
        shared_of = Box::new(move |comp| components::shares_backend(comp, c.catalog, &list));
    }

    let mut pages: Vec<Value> = CORE_PAGES
        .iter()
        .map(|(id, label, icon)| {
            json!({"id": id, "label": label,
                "icon": char::from_u32(*icon).map(String::from).unwrap_or_default()})
        })
        .collect();
    pages.extend(
        i.notches
            .iter()
            .map(|n| json!({"id": settings::notch_page_id(n.index as u64), "label": n.id})),
    );
    pages.extend(settings::module_pages(i.catalog));
    let pages = settings::visible_pages(&pages, &loose);

    let value_of = |k: &str| pick(i.config, k).cloned();
    let shown = settings::shown_fields(&loose, value_of);
    let mut next = 0;
    let fields: Vec<Field> = loose
        .iter()
        .filter_map(|f| {
            let visible = shown.get(next) == Some(f);
            if visible {
                next += 1;
            }
            let mut field = to_field(f, visible, &value_of)?;
            if field.r#type == FieldType::Component
                && let Some(c) = &i.components
                && let Some(comp) = field
                    .component
                    .as_deref()
                    .and_then(|id| components::by_id(c.catalog, id))
            {
                let state = c.states.and_then(|s| s.get(&comp.id));
                field.row =
                    serde_json::to_value(components::view(comp, state, shared_of(comp))).ok();
            }
            Some(field)
        })
        .collect();

    Schema {
        rev: 0,
        modules: Some(i.catalog.to_vec()),
        pages: pages
            .iter()
            .map(|p| Page {
                id: p["id"].as_str().unwrap_or("").to_string(),
                label: p["label"].as_str().unwrap_or("").to_string(),
                icon: p
                    .get("icon")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                sub: settings::is_sub_page(p),
            })
            .collect(),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_for(cfg: Value, catalog: &[Value], screens: &[&str]) -> Schema {
        let notches = config::notches(&cfg, &["media", "activities"]);
        let screens: Vec<String> = screens.iter().map(|s| (*s).into()).collect();
        build(&Inputs {
            config: &cfg,
            notches: &notches,
            settings_notch: 0,
            screens: &screens,
            supports_workspaces: false,
            known_workspaces: &[],
            catalog,
            default_modules: &["media", "activities"],
            components: None,
        })
    }

    fn field<'a>(s: &'a Schema, key: &str) -> &'a Field {
        s.fields
            .iter()
            .find(|f| f.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no field {key}"))
    }

    #[test]
    fn workspace_field_carries_the_compositors_current_choices() {
        let cfg = json!({});
        let notches = config::notches(&cfg, &["media"]);
        let known = vec!["1".to_string(), "2".to_string(), "11".to_string()];
        let schema = build(&Inputs {
            config: &cfg,
            notches: &notches,
            settings_notch: 0,
            screens: &[],
            supports_workspaces: true,
            known_workspaces: &known,
            catalog: &[],
            default_modules: &["media"],
            components: None,
        });
        let choices = field(&schema, "workspaces").options.as_ref().unwrap();
        assert_eq!(
            choices.iter().map(|c| c.value.clone()).collect::<Vec<_>>(),
            vec![json!("1"), json!("2"), json!("11")]
        );
    }

    #[test]
    fn the_core_pages_with_fields_and_a_page_per_notch() {
        let s = build_for(json!({}), &[], &["DP-1"]);
        let ids: Vec<_> = s.pages.iter().map(|p| (p.id.as_str(), p.sub)).collect();
        assert_eq!(
            ids,
            [
                ("general", false),
                ("notch", false),
                ("modules", false),
                ("notch/0", true)
            ],
            "Components and Appearance have no field: hidden"
        );
        assert_eq!(s.pages[0].icon, "\u{F0493}");
        assert_eq!(s.pages[3].label, "main");
    }

    #[test]
    fn a_field_has_its_default_and_the_configs_value() {
        let s = build_for(json!({"hoverOpenDelay": 250}), &[], &["DP-1"]);
        let f = field(&s, "hoverOpenDelay");
        assert_eq!(f.default, json!(120));
        assert_eq!(f.value, json!(250));
        assert!(f.visible);
        assert_eq!(
            (f.min, f.max, f.step),
            (Some(Num(0.0)), Some(Num(1000.0)), Some(Num(10.0)))
        );
        assert_eq!(field(&s, "hoverCloseDelay").value, json!(300));
        assert_eq!(field(&s, "modules").default, json!(["media", "activities"]));
    }

    #[test]
    fn the_screen_choice_waits_for_a_second_screen() {
        let one = build_for(json!({}), &[], &["DP-1"]);
        assert!(
            !one.fields
                .iter()
                .any(|f| f.key.as_deref() == Some("position.screen"))
        );
        let two = build_for(json!({}), &[], &["DP-1", "DP-2"]);
        let f = field(&two, "position.screen");
        let values: Vec<_> = f
            .options
            .as_ref()
            .unwrap()
            .iter()
            .map(|o| o.value.clone())
            .collect();
        assert_eq!(
            values,
            [json!(""), json!("all"), json!("DP-1"), json!("DP-2")]
        );
    }

    #[test]
    fn a_list_notch_keeps_its_keys_under_its_prefix_and_inherits_defaults() {
        let cfg = json!({"tabs": false, "notches": [{"id": "a"}, {"id": "b"}]});
        let s = build_for(cfg, &[], &["DP-1"]);
        let f = field(&s, "notches.0.tabs");
        assert_eq!(f.default, json!(false), "the top-level key it inherits");
        assert!(
            s.fields
                .iter()
                .any(|f| f.key.as_deref() == Some("notches.0.id"))
        );
        // Only the notch the window edits has fields, so only its page shows
        // (`coreSettingsSchema` builds the picked notch's keys).
        let pages: Vec<_> = s
            .pages
            .iter()
            .filter(|p| p.id.starts_with("notch/"))
            .collect();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].id, "notch/0");
    }

    #[test]
    fn a_user_modules_settings_sit_on_its_page_under_its_id() {
        let def = json!({"moduleId": "weather", "title": "Weather", "settings": [
            {"key": "city", "type": "string", "label": "City", "default": "Berlin"},
            {"key": "units", "type": "choice", "label": "Units", "default": "c",
             "options": [{"value": "c", "label": "C"}, {"value": "f", "label": "F"}]},
            {"key": "fine", "type": "integer", "label": "Fine", "default": 1,
             "when": {"key": "units", "is": "f"}}
        ]});
        let s = build_for(json!({"weather": {"city": "Paris"}}), &[def], &["DP-1"]);
        let page = s.pages.iter().find(|p| p.id == "modules/weather").unwrap();
        assert_eq!((page.label.as_str(), page.sub), ("Weather", true));
        let f = field(&s, "weather.city");
        assert_eq!(
            (f.page.as_str(), f.value.clone()),
            ("modules/weather", json!("Paris"))
        );
        assert!(!field(&s, "weather.fine").visible, "its when does not hold");
        assert_eq!(
            field(&s, "weather.fine").when.as_ref().unwrap().key,
            "weather.units"
        );
    }

    #[test]
    fn a_standalone_module_has_a_switch_and_a_place_and_the_catalog_lists_it() {
        let def = json!({"moduleId": "osd", "title": "OSD", "standalone": true, "settings": []});
        let s = build_for(json!({}), &[def], &["DP-1"]);
        assert!(field(&s, "osd.enabled").visible);
        assert!(
            s.fields
                .iter()
                .any(|f| f.part.as_deref() == Some("standalone"))
        );
        assert!(field(&s, "modules").help.as_ref().unwrap().contains("OSD"));
    }
}
