//! Port of `core/settings/Settings.js` (tests: `tests/settings.test.mjs`).
//!
//! Settings-window rules: pages, sections, module info files and list edits.
//! Pages and fields are loose JSON objects in the settings data, so they stay
//! [`serde_json::Value`]; a missing key is `None` where JS has `undefined`,
//! and `Value::Null` is JS `null`. The shape that is fixed (what
//! `parseModuleInfo` returns) is the typed [`ModuleInfo`].

use crate::js;
use serde::Serialize;
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------------------
// JS semantics the rules rely on

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// `s` with its first letter upper-cased (`charAt(0).toUpperCase() + slice(1)`).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A value's `title` if truthy as a string, else `fallback` (`m.title || id`).
fn title_or(m: &Value, fallback: &str) -> String {
    match m.get("title") {
        Some(t) if js::truthy(t) => js::string(t),
        _ => fallback.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Pages

/// `mergePages`: the core pages with the host's applied; a host page with a
/// core id replaces its keys, any other is appended. The inputs are not changed.
pub fn merge_pages(core: &[Value], host: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = core.to_vec();
    for h in host {
        // Like the JS loop, the last page with a matching id wins.
        match out
            .iter()
            .rposition(|p| js::eq_opt(p.get("id"), h.get("id")))
        {
            None => out.push(h.clone()),
            Some(i) => {
                if let (Some(dst), Some(src)) = (out[i].as_object_mut(), h.as_object()) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }
    out
}

/// `pageOf`: the page a field is on (`general` without one). `None` is a
/// truthy `page` that is not a string, which no page id equals.
fn page_of(field: &Value) -> Option<&str> {
    match field.get("page") {
        Some(Value::String(s)) if !s.is_empty() => Some(s),
        p if js::truthy_opt(p) => None,
        _ => Some("general"),
    }
}

/// `parentPage`: a page id with a "/" is a sub-page of the part before it
/// ("modules/media" of "modules"). JS `undefined` is the empty id.
pub fn parent_page(id: &str) -> &str {
    id.find('/').map_or(id, |at| &id[..at])
}

fn page_id(page: &Value) -> &str {
    str_of(page, "id").unwrap_or("")
}

/// `isSubPage`: the page has no sidebar entry of its own.
pub fn is_sub_page(page: &Value) -> bool {
    parent_page(page_id(page)) != page_id(page)
}

/// `visiblePages`: pages with at least one field, in their declared order.
pub fn visible_pages(pages: &[Value], schema: &[Value]) -> Vec<Value> {
    pages
        .iter()
        .filter(|p| schema.iter().any(|f| page_of(f) == str_of(p, "id")))
        .cloned()
        .collect()
}

/// `navPages`: the pages the sidebar lists, the ones that are not sub-pages.
pub fn nav_pages(pages: &[Value]) -> Vec<Value> {
    pages.iter().filter(|p| !is_sub_page(p)).cloned().collect()
}

/// `findPage`: the page with that id.
pub fn find_page<'a>(pages: &'a [Value], id: &str) -> Option<&'a Value> {
    pages.iter().find(|p| str_of(p, "id") == Some(id))
}

/// `resolvePage`: the page to show: the wanted one if visible, else its
/// parent, else the first sidebar page (or the first page, or "").
pub fn resolve_page(pages: &[Value], wanted: &str) -> String {
    if find_page(pages, wanted).is_some() {
        return wanted.to_string();
    }
    let parent = parent_page(wanted);
    if find_page(pages, parent).is_some() {
        return parent.to_string();
    }
    let nav = nav_pages(pages);
    match (nav.first(), pages.first()) {
        (Some(p), _) | (None, Some(p)) => page_id(p).to_string(),
        (None, None) => String::new(),
    }
}

/// `cyclePage`: the sidebar page `delta` steps from the one `current` sits
/// under, wrapping around; "" without sidebar pages.
pub fn cycle_page(pages: &[Value], current: &str, delta: i64) -> String {
    let nav = nav_pages(pages);
    if nav.is_empty() {
        return String::new();
    }
    let here = parent_page(current);
    // The last match wins, and no match is -1, as in JS.
    let at = nav
        .iter()
        .rposition(|p| page_id(p) == here)
        .map_or(-1, |i| i as i64);
    let n = nav.len() as i64;
    page_id(&nav[(((at + delta) % n + n) % n) as usize]).to_string()
}

// ---------------------------------------------------------------------------
// Fields

/// `effectiveValue`: a field's value as the settings show it: the config's
/// (`value_of(key)`, `None` unset), else the default of the field with that
/// key. `Some(Null)` is a value, as in JS.
pub fn effective_value(
    schema: &[Value],
    key: &str,
    value_of: impl Fn(&str) -> Option<Value>,
) -> Option<Value> {
    if let Some(v) = value_of(key) {
        return Some(v);
    }
    schema
        .iter()
        .find(|f| str_of(f, "key") == Some(key))
        .and_then(|f| f.get("default").cloned())
}

/// `shownFields`: the schema without the fields whose `when: { key, is }`
/// doesn't hold (`is` may be a list of accepted values).
pub fn shown_fields(schema: &[Value], value_of: impl Fn(&str) -> Option<Value>) -> Vec<Value> {
    schema
        .iter()
        .filter(|f| {
            let Some(when) = f.get("when").filter(|w| js::truthy(w)) else {
                return true;
            };
            let v = effective_value(schema, str_of(when, "key").unwrap_or(""), &value_of);
            match when.get("is") {
                Some(Value::Array(list)) => v
                    .as_ref()
                    .is_some_and(|v| list.iter().any(|x| js::eq(x, v))),
                is => js::eq_opt(v.as_ref(), is),
            }
        })
        .cloned()
        .collect()
}

/// One section of a page: its name and its fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Section {
    /// The section name, "" for fields without one.
    pub name: String,
    /// The fields in schema order.
    pub fields: Vec<Value>,
}

/// `sections`: a page's fields grouped by section, in first-seen order.
pub fn sections(schema: &[Value], page: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for f in schema {
        if page_of(f) != Some(page) {
            continue;
        }
        let name = match f.get("section") {
            Some(s) if js::truthy(s) => js::string(s),
            _ => String::new(),
        };
        match out.iter_mut().find(|s| s.name == name) {
            Some(s) => s.fields.push(f.clone()),
            None => out.push(Section {
                name,
                fields: vec![f.clone()],
            }),
        }
    }
    out
}

/// `modulePageId`: a module's own page, under Modules.
pub fn module_page_id(module_id: &str) -> String {
    format!("modules/{module_id}")
}

// ---------------------------------------------------------------------------
// Module info files

/// What `parseModuleInfo` returns: a module's title, settings and kind.
/// Serialised it has the JS shape (flags absent unless set).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ModuleInfo {
    /// The module's title, "" without one.
    pub title: String,
    /// Its settings fields, inherited ones first.
    pub settings: Vec<Value>,
    /// No notch shows it; it has a place of its own.
    #[serde(skip_serializing_if = "is_false")]
    pub standalone: bool,
    /// A coding agent in a terminal.
    #[serde(skip_serializing_if = "is_false")]
    pub agent: bool,
    /// A spoken conversation with an agent.
    #[serde(skip_serializing_if = "is_false")]
    pub live: bool,
    /// One of the config's own `terminals`.
    #[serde(skip_serializing_if = "is_false")]
    pub terminal: bool,
    /// Where an agent's plan usage comes from (`agent.usage`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    /// Why the file was (partly) dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// `parseModuleInfo`: a module's JSON file as its title, fields and kind.
/// Anything malformed is dropped, so a bad file costs the module its page and
/// nothing else; `error` says why.
pub fn parse_module_info(text: &str) -> ModuleInfo {
    let mut info = ModuleInfo::default();
    let data: Value = match serde_json::from_str(text) {
        Ok(d) => d,
        Err(e) => {
            info.error = Some(e.to_string());
            return info;
        }
    };
    if !data.is_object() {
        info.error = Some("not an object".into());
        return info;
    }
    if let Some(t) = str_of(&data, "title") {
        info.title = t.to_string();
    }
    info.standalone = data.get("standalone") == Some(&Value::Bool(true));
    let own: Vec<Value> = match data.get("settings") {
        Some(Value::Array(list)) => list
            .iter()
            .filter(|f| f.is_object() && f.get("type").is_some_and(Value::is_string))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    let mut inherited: Vec<Value> = Vec::new();
    if let Some(agent) = data.get("agent") {
        if agent.is_object() && str_of(agent, "command").is_some_and(|c| !c.trim().is_empty()) {
            info.agent = true;
            if let Some(u) = str_of(agent, "usage").filter(|u| !u.is_empty()) {
                info.usage = Some(u.to_string());
            }
            inherited = agent_settings(agent, &info.title);
        } else {
            info.error = Some("agent: needs a command".into());
        }
    }
    if let Some(live) = data.get("live") {
        let local = live.get("local") == Some(&Value::Bool(true));
        if live.is_object()
            && (local || str_of(live, "command").is_some_and(|c| !c.trim().is_empty()))
        {
            info.live = true;
            inherited = live_settings(live, &info.title);
        } else {
            info.error = Some("live: needs a command".into());
        }
    }
    // A field of its own replaces an inherited one with the same key (and, as
    // in JS, any key-less own field drops the inherited key-less ones).
    let own_keys: Vec<Option<&Value>> = own.iter().map(|f| f.get("key")).collect();
    info.settings = inherited
        .into_iter()
        .filter(|f| !own_keys.iter().any(|k| js::eq_opt(*k, f.get("key"))))
        .chain(own.iter().cloned())
        .collect();
    info
}

/// `agentSettings`: the fields every agent has, from its file's `agent`
/// (its `command` is the default; `help` adds to a field's help by key;
/// `usage` brings the plan usage fields). `title` names it in the texts.
pub fn agent_settings(agent: &Value, title: &str) -> Vec<Value> {
    let name = if title.is_empty() { "the agent" } else { title };
    let command = str_of(agent, "command").unwrap_or("").trim();
    let mut fields = terminal_fields(
        command,
        name,
        "Empty is notchd's own folder, notchd in your Documents folder, made the first time.",
        &format!(
            "Closed and not working this long, {name} stops; the next time the notch opens, a new one starts (/resume brings the session back). 0 keeps it running."
        ),
    );
    if str_of(agent, "usage").is_some_and(|u| !u.is_empty()) {
        fields.push(json!({ "key": "usage", "type": "boolean", "label": "Plan usage", "default": true, "section": "Usage",
            "help": format!("How much of the plan's limits (5 hours, the week) is used, in the open notch, asked every few minutes with the login {name} keeps. notchd only reads that login; {name} renews it when it runs.") }));
        fields.push(json!({ "key": "usageWarn", "type": "integer", "label": "Warn at", "default": 80, "min": 50, "max": 100, "step": 5, "unit": "%", "section": "Usage",
            "help": "The notch peeks once when a limit passes this, and again when it is reached. 100 warns only at the limit." }));
    }
    if let Some(extra) = agent.get("help").filter(|h| h.is_object()) {
        for f in &mut fields {
            let add = str_of(extra, str_of(f, "key").unwrap_or("")).filter(|s| !s.is_empty());
            if let (Some(add), Some(obj)) = (add, f.as_object_mut()) {
                let help = match obj.get("help") {
                    Some(h) if js::truthy(h) => format!("{} {add}", js::string(h)),
                    _ => add.to_string(),
                };
                obj.insert("help".into(), Value::String(help));
            }
        }
    }
    fields
}

/// The voices a Codex realtime (v3) conversation speaks in; "" is Codex's choice.
const LIVE_VOICES: &[&str] = &[
    "juniper", "maple", "spruce", "ember", "vale", "breeze", "arbor", "sol", "cove", "aube",
    "bubbie", "haetsal", "hanul", "himari", "leher", "miko", "neer", "porto", "rindo", "rio",
    "rivage", "selva", "sonna", "tinta", "viola",
];

/// `liveSettings`: the fields of a spoken conversation, from its file's
/// `live` (`live.command` is the default command; `live.local` brings the
/// local one). `title` names it.
pub fn live_settings(live: &Value, title: &str) -> Vec<Value> {
    let name = if title.is_empty() { "the agent" } else { title };
    let next = "Applies to the next conversation.";
    if live.get("local") == Some(&Value::Bool(true)) {
        return local_live_settings(next);
    }
    let command = str_of(live, "command").unwrap_or("").trim();
    let mut voices = vec![json!({ "value": "", "label": "Codex's" })];
    voices.extend(
        LIVE_VOICES
            .iter()
            .map(|v| json!({ "value": v, "label": capitalize(v) })),
    );
    vec![
        json!({ "key": "command", "type": "string", "label": "Command", "default": command,
            "help": format!("The command that runs Codex; notchd starts its server (app-server) with it. {next}") }),
        json!({ "key": "directory", "type": "string", "label": "Folder", "default": "",
            "help": format!("Where {name} works when you ask it something; ~ is your home. Empty is notchd's own folder, notchd in your Documents folder. {next}") }),
        json!({ "key": "voice", "type": "choice", "label": "Voice", "default": "", "display": "chips", "options": voices,
            "help": format!("The voice it speaks in. {next}") }),
        json!({ "key": "access", "type": "choice", "label": "May", "default": "read-only",
            "options": [{ "value": "read-only", "label": "Only read" }, { "value": "workspace-write", "label": "Change files in its folder" }],
            "help": format!("What {name} may do in its folder when you ask it to. It never asks for approval (no one would answer): what it may not do, it doesn't. {next}") }),
        json!({ "type": "component", "component": "codex-live-audio", "section": "Needs" }),
    ]
}

/// The languages of the speech models, named in their own language.
fn speech_languages() -> Vec<Value> {
    [
        ("pt", "Português"),
        ("en", "English"),
        ("es", "Español"),
        ("fr", "Français"),
        ("de", "Deutsch"),
        ("it", "Italiano"),
        ("nl", "Nederlands"),
    ]
    .iter()
    .map(|(v, l)| json!({ "value": v, "label": l }))
    .collect()
}

/// `localLiveSettings`: the fields of a local spoken conversation: which
/// agent's session it joins, speech to text (Voxtype) and text to speech
/// (a LocalAI voice).
fn local_live_settings(next: &str) -> Vec<Value> {
    let part = |id: &str, section: &str| json!({ "type": "component", "component": id, "section": section });
    let stt = "Speech to text (STT)";
    let tts = "Text to speech (TTS)";
    let mut languages = vec![
        json!({ "value": "", "label": "Voxtype's own" }),
        json!({ "value": "auto", "label": "Auto-detect" }),
    ];
    languages.extend(speech_languages());
    vec![
        json!({ "key": "agent", "type": "choice", "section": "Agent", "label": "Agent", "default": "codex",
            "options": [{ "value": "codex", "label": "Codex" }, { "value": "claude", "label": "Claude Code" }],
            "help": format!("Whose text session your words go into, the one you also type in. {next}") }),
        json!({ "key": "command", "type": "string", "section": "Agent", "label": "Command", "default": "",
            "help": format!("The command line that runs the agent; empty is codex or claude. {next}") }),
        json!({ "key": "session", "type": "string", "section": "Agent", "label": "Session", "default": "last",
            "help": format!("last: the agent's most recent session in the folder. new: one of its own. Or a session id. {next}") }),
        json!({ "key": "directory", "type": "string", "section": "Agent", "label": "Folder", "default": "",
            "help": format!("Where the agent works, and whose sessions \"last\" looks in; ~ is your home. Empty is notchd's own folder, notchd in your Documents folder. {next}") }),
        json!({ "key": "access", "type": "choice", "section": "Agent", "label": "May", "default": "read-only",
            "options": [{ "value": "read-only", "label": "Only read" }, { "value": "workspace-write", "label": "Change files in its folder" }],
            "help": format!("What the agent may do in its folder when you ask it to (Claude: plan mode, or accept edits). It never asks for approval (no one would answer). {next}") }),
        part("libpulse", "Audio"),
        json!({ "type": "note", "section": stt, "label": "Listening",
            "help": "Speech to text turns what you say into the words the agent receives. It is Voxtype, on this machine, with the speech model you set up for dictation." }),
        part("voxtype", stt),
        json!({ "key": "language", "type": "choice", "section": stt, "label": "Language you speak", "default": "", "display": "chips",
            "options": languages,
            "help": format!("Empty is Voxtype's own setting. Any other code Voxtype knows can be written in the config file as language. {next}") }),
        json!({ "key": "speechModel", "type": "string", "section": stt, "label": "Speech model", "default": "",
            "help": format!("A Voxtype model, such as small or large-v3-turbo (voxtype info models lists them); empty is the one Voxtype is set up with. {next}") }),
        json!({ "type": "note", "section": tts, "label": "Speaking",
            "help": "Text to speech is the voice that answers you: the agent's words, read aloud by a LocalAI voice on this machine. Piper is light and quick; Pocket TTS is more natural and heavier." }),
        part("localai", tts),
        json!({ "key": "voiceLanguage", "type": "choice", "section": tts, "label": "Language it speaks", "default": "pt", "display": "chips",
            "options": speech_languages(),
            "help": format!("The language of the voice: the voices below are the ones of this language. {next}") }),
        json!({ "type": "components", "section": tts, "match": { "engine": ["piper", "pocket"] }, "by": { "key": "voiceLanguage", "field": "language" } }),
        json!({ "key": "voiceModel", "type": "choice", "section": tts, "label": "Speaks with", "default": "",
            "options": [{ "value": "", "label": "First installed" },
                        { "value": "piper", "label": "Piper", "needs": [{ "engine": "piper", "language": "@voiceLanguage" }] },
                        { "value": "pocket", "label": "Pocket TTS", "needs": [{ "engine": "pocket", "language": "@voiceLanguage" }] }],
            "help": format!("Which voice engine speaks, in the language above: the first installed is Piper, else Pocket TTS. {next}") }),
        json!({ "key": "otherVoiceModel", "type": "string", "section": tts, "label": "Another voice model", "default": "",
            "help": format!("Any text-to-speech model of the LocalAI you use, by its name; it takes the place of the choice above. {next}") }),
        json!({ "key": "voice", "type": "string", "section": tts, "label": "Voice", "default": "",
            "help": format!("A voice of that model, for those that have several: Pocket TTS has alba, azelma, rafael and more. Empty is the model's own. {next}") }),
        json!({ "key": "localaiUrl", "type": "string", "section": tts, "label": "LocalAI address", "default": "",
            "help": format!("Where LocalAI answers. Empty is notchd's own, http://127.0.0.1:18080, which it starts when a conversation needs it and stops after; an address where one is already running is used as it is. {next}") }),
    ]
}

/// `terminalFields`: the fields of a program in a terminal (an agent or one
/// of your own): folder, command (`command` its default), when it stops, size,
/// font. `empty_dir` says where it starts without a folder, `idle` what
/// stopping when idle does.
fn terminal_fields(command: &str, name: &str, empty_dir: &str, idle: &str) -> Vec<Value> {
    let later = "Applies the next time it starts (the restart button).";
    vec![
        json!({ "key": "directory", "type": "string", "label": "Folder", "default": "",
            "help": format!("Where {name} starts; ~ is your home. {empty_dir} {later}") }),
        json!({ "key": "command", "type": "string", "label": "Command", "default": command,
            "help": format!("The command line that runs {name}. {later}") }),
        json!({ "key": "idleMinutes", "type": "integer", "label": "Stop when idle", "default": 0, "min": 0, "max": 240, "step": 15, "unit": "min",
            "help": idle }),
        json!({ "key": "width", "type": "integer", "label": "Width", "default": 760, "min": 320, "max": 1600, "step": 20, "unit": "px" }),
        json!({ "key": "height", "type": "integer", "label": "Terminal height", "default": 420, "min": 120, "max": 1000, "step": 20, "unit": "px" }),
        json!({ "key": "font", "type": "string", "label": "Font", "default": "",
            "help": format!("A monospace font family for the terminal; empty uses the theme's. {later}") }),
    ]
}

/// `terminalInfo`: the module info of a terminal of your own (an entry of the
/// config's `terminals`: `{ id, title, command }`). Its title and command are
/// its entry's, edited there (`absolute` keys; the command is `required`).
pub fn terminal_info(t: &Value) -> ModuleInfo {
    let id = t.get("id").map(js::string).unwrap_or_default();
    let title = str_of(t, "title").unwrap_or("");
    let name = if title.is_empty() { id.as_str() } else { title };
    let entry = format!("terminals.{id}.");
    let later = "Applies the next time it starts (the restart button).";
    let own = [
        json!({ "key": format!("{entry}title"), "absolute": true, "type": "string", "label": "Title", "default": "",
            "help": format!("Its name, in the notch and here; empty is its id, {id}.") }),
        json!({ "key": format!("{entry}command"), "absolute": true, "required": true, "type": "string", "label": "Command",
            "help": format!("The command line it runs, in a shell: \"btop\", \"cd ~/app && npm run dev\". {later}") }),
    ];
    let fields = terminal_fields(
        str_of(t, "command").unwrap_or(""),
        name,
        "Empty is your home.",
        &format!(
            "Closed this long, {name} stops; the next time the notch opens, it starts again. 0 keeps it running."
        ),
    );
    ModuleInfo {
        title: title.to_string(),
        terminal: true,
        settings: own
            .into_iter()
            .chain(
                fields
                    .into_iter()
                    .filter(|f| str_of(f, "key") != Some("command")),
            )
            .collect(),
        ..ModuleInfo::default()
    }
}

/// `moduleInfoPath`: the JSON file of the module at `url` (its .qml): the same
/// path, .json. Quickshell names files under the shell's folder
/// `qs:@/qs/<path>` (resolved against `shell_dir`) and others
/// `qs:///<absolute path>`; "" for anything else. A malformed percent escape
/// (a JS `URIError`) gives "" too.
pub fn module_info_path(url: &str, shell_dir: &str) -> String {
    if !url.ends_with(".qml") {
        return String::new();
    }
    let path = if let Some(p) = url.strip_prefix("file://") {
        p.to_string()
    } else if let (Some(p), false) = (url.strip_prefix("qs:@/qs/"), shell_dir.is_empty()) {
        format!("{}/{p}", shell_dir.strip_suffix('/').unwrap_or(shell_dir))
    } else if let Some(p) = url.strip_prefix("qs://") {
        p.to_string()
    } else {
        return String::new();
    };
    match js::decode_uri_component(&path) {
        Some(p) => p
            .strip_suffix(".qml")
            .map(|s| format!("{s}.json"))
            .unwrap_or(p),
        None => String::new(),
    }
}

/// `settingDefault`: a module setting's default, from its fields; `None` for
/// an unknown key or a field without a default.
pub fn setting_default(fields: &[Value], key: &str) -> Option<Value> {
    fields
        .iter()
        .find(|f| str_of(f, "key") == Some(key))
        .and_then(|f| f.get("default").cloned())
}

// ---------------------------------------------------------------------------
// Module fields and pages

/// What a standalone module's page has before its own fields: a switch and
/// where it shows. `edge_preview` is the url of the edge cards' picture.
fn standalone_fields(id: &str, edge_preview: &str) -> Vec<Value> {
    let page = module_page_id(id);
    let edges: Vec<Value> = ["top", "bottom", "left", "right"]
        .iter()
        .map(|e| json!({ "value": e, "label": capitalize(e), "preview": edge_preview }))
        .collect();
    vec![
        json!({ "page": page, "key": format!("{id}.enabled"), "type": "boolean", "label": "Enabled", "default": true,
            "help": "Off, it does not load at all." }),
        json!({ "page": page, "section": "Position", "key": format!("{id}.position.edge"), "type": "choice", "display": "cards",
            "label": "Edge", "default": "top",
            "help": "Where it shows, on the screen you work on. No notch shows it: it has a place of its own, over any notch that hangs there.",
            "options": edges }),
        json!({ "page": page, "section": "Position", "key": format!("{id}.position.offset"), "type": "number", "display": "percent",
            "label": "Along the edge", "default": 0.5, "min": 0, "max": 1, "step": 0.01,
            "help": "0 % is its left (or top) end, 50 % the middle." }),
    ]
}

/// `moduleRefs`: a field's references to other fields of its module
/// (`when.key`, `by.key`, an option's `needs` value "@key") put under the
/// module's id, as its own key is.
fn module_refs(field: &Value, module_id: &str) -> Value {
    let mut out = field.clone();
    let Some(obj) = out.as_object_mut() else {
        return out;
    };
    for name in ["when", "by"] {
        if let Some(r) = field.get(name).filter(|r| js::truthy_opt(r.get("key"))) {
            let mut r = r.clone();
            let key = js::string(&r["key"]);
            r["key"] = Value::String(format!("{module_id}.{key}"));
            obj.insert(name.into(), r);
        }
    }
    if let Some(Value::Array(options)) = field.get("options") {
        let options: Vec<Value> = options
            .iter()
            .map(|o| module_option_refs(o, module_id))
            .collect();
        obj.insert("options".into(), Value::Array(options));
    }
    out
}

fn module_option_refs(option: &Value, module_id: &str) -> Value {
    let Some(Value::Array(needs)) = option.get("needs") else {
        return option.clone();
    };
    let needs: Vec<Value> = needs
        .iter()
        .map(|n| match n {
            Value::String(_) => n.clone(),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = match v.as_str().and_then(|s| s.strip_prefix('@')) {
                            Some(rest) => Value::String(format!("@{module_id}.{rest}")),
                            None => v.clone(),
                        };
                        (k.clone(), v)
                    })
                    .collect::<Map<_, _>>(),
            ),
            // `Object.keys` of a non-object has nothing: an empty object.
            _ => Value::Object(Map::new()),
        })
        .collect();
    let mut out = option.clone();
    out["needs"] = Value::Array(needs);
    out
}

/// `moduleFields`: the settings fields of every module (`{ moduleId, title,
/// settings, standalone }`, from their JSON files), with their keys under the
/// module's id (an `absolute` one's stays as given), on the module's own page
/// unless they name another (then under its title); a standalone module's
/// switch and position first.
pub fn module_fields(modules: &[Value], edge_preview: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for m in modules {
        if !js::truthy_opt(m.get("moduleId")) {
            continue;
        }
        let id = js::string(&m["moduleId"]);
        if js::truthy_opt(m.get("standalone")) {
            out.extend(standalone_fields(&id, edge_preview));
        }
        let Some(Value::Array(settings)) = m.get("settings") else {
            continue;
        };
        for f in settings {
            let mut field = f.clone();
            let absolute = js::truthy_opt(f.get("absolute"));
            if js::truthy_opt(f.get("key")) && !absolute {
                field["key"] = Value::String(format!("{id}.{}", js::string(&f["key"])));
            }
            if !absolute {
                field = module_refs(&field, &id);
            }
            if !js::truthy_opt(field.get("page")) {
                field["page"] = Value::String(module_page_id(&id));
            } else if !js::truthy_opt(field.get("section")) {
                field["section"] = Value::String(title_or(m, &id));
            }
            out.push(field);
        }
    }
    out
}

/// `modulePages`: a page per module, titled after it (its id without one).
pub fn module_pages(modules: &[Value]) -> Vec<Value> {
    modules
        .iter()
        .filter(|m| js::truthy_opt(m.get("moduleId")))
        .map(|m| {
            let id = js::string(&m["moduleId"]);
            json!({ "id": module_page_id(&id), "label": title_or(m, &id) })
        })
        .collect()
}

/// `moduleTitle`: a module's own title (the loaded module's or its JSON
/// file's), else its id capitalized.
pub fn module_title(id: &str, module: Option<&Value>) -> String {
    match module {
        Some(m) if js::truthy_opt(m.get("title")) => title_or(m, ""),
        _ => capitalize(id),
    }
}

/// `moduleKind`: which list of the Modules page a module is in: "standalone",
/// "agent" (a coding agent, or a conversation with one: `live`), "terminal",
/// else "notch".
pub fn module_kind(m: &Value) -> &'static str {
    if js::truthy_opt(m.get("standalone")) {
        "standalone"
    } else if js::truthy_opt(m.get("agent")) || js::truthy_opt(m.get("live")) {
        "agent"
    } else if js::truthy_opt(m.get("terminal")) {
        "terminal"
    } else {
        "notch"
    }
}

/// `catalogPart`: one of the Modules page's lists (`part`, a [`module_kind`];
/// "notch" by default), in catalog order.
pub fn catalog_part(catalog: &[Value], part: Option<&str>) -> Vec<Value> {
    let part = part.filter(|p| !p.is_empty()).unwrap_or("notch");
    catalog
        .iter()
        .filter(|m| js::truthy(m) && module_kind(m) == part)
        .cloned()
        .collect()
}

/// `indexIn`: where `id` is in a list value, `None` if it is not (or the
/// value is not a list).
pub fn index_in(list: Option<&Value>, id: &str) -> Option<usize> {
    match list {
        Some(Value::Array(a)) => a.iter().position(|x| x.as_str() == Some(id)),
        _ => None,
    }
}

/// `moduleStatus`: where a module shows, for the Modules page: "On" or "Off"
/// for a standalone one (`enabled` false is off), else "Off" when no notch
/// lists it, "On" with one notch, or "In a, b" naming the notches that list it.
pub fn module_status(id: &str, specs: &[Value], standalone: bool, enabled: Option<bool>) -> String {
    if standalone {
        return if enabled == Some(false) { "Off" } else { "On" }.into();
    }
    let places: Vec<String> = specs
        .iter()
        .filter(|s| index_in(s.get("moduleIds"), id).is_some())
        .map(|s| {
            s.get("id")
                .filter(|v| !v.is_null())
                .map(js::string)
                .unwrap_or_default()
        })
        .collect();
    if places.is_empty() {
        "Off".into()
    } else if specs.len() > 1 {
        format!("In {}", places.join(", "))
    } else {
        "On".into()
    }
}

// ---------------------------------------------------------------------------
// Notches

/// `notchPageId`: a notch's own page, under Notches, by its index.
pub fn notch_page_id(index: u64) -> String {
    format!("notch/{index}")
}

/// `notchIndexOfPage`: the notch index a page is for ("notch/2" is 2); -1 for
/// any other page.
pub fn notch_index_of_page(id: &str) -> i64 {
    id.strip_prefix("notch/")
        .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|d| d.parse().ok())
        .unwrap_or(-1)
}

/// `notchSummary`: one line about a notch, for the Notches page: where it is
/// and what it shows. `notch`: `{ edge, moduleIds }` (null gives ""); `titles`:
/// id -> title.
pub fn notch_summary(notch: &Value, titles: &Value) -> String {
    if !js::truthy(notch) {
        return String::new();
    }
    let edge = match notch.get("edge") {
        Some(e) if js::truthy(e) => js::string(e),
        _ => "top".into(),
    };
    let ids = to_list(notch.get("moduleIds").unwrap_or(&Value::Null));
    let shows = if ids.is_empty() {
        "no modules".to_string()
    } else {
        ids.iter()
            .map(|id| match titles.get(id) {
                Some(t) if js::truthy(t) => js::string(t),
                _ => module_title(id, None),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("{} edge · {shows}", capitalize(&edge))
}

// ---------------------------------------------------------------------------
// List edits

/// `toList`: a list value as a plain list of strings. Arrays and array-likes
/// (an object with a numeric `length`, what a Repeater model hands over) are
/// accepted; anything else, strings included, is empty.
pub fn to_list(value: &Value) -> Vec<String> {
    match value {
        Value::Array(a) => a.iter().map(js::string).collect(),
        Value::Object(o) => match o.get("length").and_then(Value::as_f64) {
            Some(n) => (0..n.max(0.0) as usize)
                .map(|i| {
                    o.get(&i.to_string())
                        .map_or_else(|| "undefined".to_string(), js::string)
                })
                .collect(),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// `toggleId`: `ids` with `id` added at the end, or removed if present.
pub fn toggle_id(ids: &[String], id: &str) -> Vec<String> {
    let mut next = ids.to_vec();
    match next.iter().position(|x| x == id) {
        None => next.push(id.to_string()),
        Some(at) => {
            next.remove(at);
        }
    }
    next
}

/// `moveId`: `ids` with `id` moved `delta` places; unchanged if it would
/// leave the list or is not there.
pub fn move_id(ids: &[String], id: &str, delta: i64) -> Vec<String> {
    let mut next = ids.to_vec();
    let Some(at) = next.iter().position(|x| x == id) else {
        return next;
    };
    let to = at as i64 + delta;
    if to < 0 || to >= next.len() as i64 {
        return next;
    }
    let moved = next.remove(at);
    next.insert(to as usize, moved);
    next
}

/// `allIds`: every id to list: enabled ones in their order, then the rest available.
pub fn all_ids(enabled: &[String], available: &[String]) -> Vec<String> {
    let mut out = enabled.to_vec();
    for a in available {
        if !out.contains(a) {
            out.push(a.clone());
        }
    }
    out
}

/// `workspaceValue`: a workspace as the config stores it: a number for a
/// numbered one, else its name.
pub fn workspace_value(w: &Value) -> Value {
    workspace_from_str(&js::string(w))
}

fn workspace_from_str(s: &str) -> Value {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(n) = s.parse::<u64>() {
            return json!(n);
        }
        if let Some(n) = s.parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

/// `sortWorkspaces`: numbers in order, then names (by UTF-16 code units, as JS).
pub fn sort_workspaces(list: &mut [Value]) {
    list.sort_by(|a, b| match (a, b) {
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::Number(_), _) => std::cmp::Ordering::Less,
        (_, Value::Number(_)) => std::cmp::Ordering::Greater,
        _ => js::string(a)
            .encode_utf16()
            .cmp(js::string(b).encode_utf16()),
    });
}

/// `workspaceChoices`: the workspaces to offer: 1 to 10, the ones the
/// compositor knows and the ones already picked (both list values); numbers
/// in order, then names.
pub fn workspace_choices(known: &Value, picked: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut add = |s: String| {
        if s.is_empty() {
            return;
        }
        let v = workspace_from_str(&s);
        if !out.iter().any(|x| js::eq(x, &v)) {
            out.push(v);
        }
    };
    for i in 1..=10 {
        add(i.to_string());
    }
    to_list(known).into_iter().for_each(&mut add);
    to_list(picked).into_iter().for_each(&mut add);
    sort_workspaces(&mut out);
    out
}

/// `toggleWorkspace`: `picked` (a list value) with `w` added or removed, in
/// order; `None` once empty, which removes the key (every workspace).
pub fn toggle_workspace(picked: &Value, w: &Value) -> Option<Vec<Value>> {
    let mut list: Vec<Value> = to_list(picked)
        .iter()
        .map(|s| workspace_from_str(s))
        .collect();
    let v = workspace_value(w);
    match list.iter().position(|x| js::eq(x, &v)) {
        None => list.push(v),
        Some(at) => {
            list.remove(at);
        }
    }
    if list.is_empty() {
        None
    } else {
        sort_workspaces(&mut list);
        Some(list)
    }
}

/// `snapToStep`: a slider value: `v` snapped to `step` from `min` and kept
/// within [min, max], without float noise (0.35, not 0.35000000000000003).
/// A step that is not positive is 1.
pub fn snap_to_step(v: f64, min: f64, max: f64, step: f64) -> f64 {
    let step = if step > 0.0 { step } else { 1.0 };
    // JS Math.round rounds halves up.
    let snapped = min + ((v - min) / step + 0.5).floor() * step;
    // JS prints numbers below 1e-6 in exponent form, which has no decimals.
    let decimals = if step < 1e-6 {
        0
    } else {
        format!("{step}").split('.').nth(1).map_or(0, str::len)
    };
    let clean: f64 = format!("{snapped:.decimals$}").parse().unwrap_or(snapped);
    max.min(clean).max(min)
}

/// `storedValue`: what a control's edit writes: `None` (remove the key) when
/// it equals the field's default, so the config never stores defaults.
pub fn stored_value(field: &Value, value: Option<&Value>) -> Option<Value> {
    let default = if js::truthy(field) {
        field.get("default")
    } else {
        None
    };
    match (default, value) {
        (Some(d), Some(v)) if js::eq(v, d) => None,
        _ => value.cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn core() -> Vec<Value> {
        vec![
            json!({ "id": "general", "label": "General", "icon": 1 }),
            json!({ "id": "modules", "label": "Modules", "icon": 2 }),
        ]
    }

    fn with_sub() -> Vec<Value> {
        vec![
            json!({ "id": "general" }),
            json!({ "id": "modules" }),
            json!({ "id": "modules/media" }),
            json!({ "id": "appearance" }),
        ]
    }

    fn ids(pages: &[Value]) -> Vec<&str> {
        pages.iter().map(|p| str_of(p, "id").unwrap()).collect()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn keys(fields: &[Value]) -> Vec<Option<&str>> {
        fields.iter().map(|f| str_of(f, "key")).collect()
    }

    fn field_with<'a>(info: &'a ModuleInfo, key: &str) -> &'a Value {
        info.settings
            .iter()
            .find(|f| str_of(f, "key") == Some(key))
            .unwrap()
    }

    fn voice_info() -> ModuleInfo {
        parse_module_info(&json!({ "title": "Voice", "live": { "local": true } }).to_string())
    }

    fn repo(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn registry() -> Vec<Value> {
        let text = std::fs::read_to_string(repo("core/components/components.json")).unwrap();
        serde_json::from_str::<Value>(&text).unwrap()["components"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn voice_key() -> &'static str {
        "local-live.voiceLanguage"
    }

    fn voice_fields() -> Vec<Value> {
        module_fields(
            &[
                json!({ "moduleId": "local-live", "title": "Voice", "settings": voice_info().settings }),
            ],
            "",
        )
    }

    // Node: mergePages lets a host relabel a core page and append its own
    #[test]
    fn merge_pages_lets_a_host_relabel_a_core_page_and_append_its_own() {
        let core = core();
        let pages = merge_pages(
            &core,
            &[
                json!({ "id": "modules", "label": "Plugins" }),
                json!({ "id": "extra", "label": "Extra" }),
            ],
        );
        let got: Vec<(Option<&str>, Option<&str>, Option<&Value>)> = pages
            .iter()
            .map(|p| (str_of(p, "id"), str_of(p, "label"), p.get("icon")))
            .collect();
        assert_eq!(
            got,
            vec![
                (Some("general"), Some("General"), Some(&json!(1))),
                (Some("modules"), Some("Plugins"), Some(&json!(2))),
                (Some("extra"), Some("Extra"), None),
            ]
        );
        assert_eq!(core[1]["label"], "Modules", "core pages are not mutated");
    }

    // Node: visiblePages hides pages without fields; fields default to general
    #[test]
    fn visible_pages_hides_pages_without_fields_and_fields_default_to_general() {
        let schema = [
            json!({ "key": "a" }),
            json!({ "key": "b", "page": "extra" }),
        ];
        let pages = merge_pages(&core(), &[json!({ "id": "extra" })]);
        assert_eq!(ids(&visible_pages(&pages, &schema)), ["general", "extra"]);
    }

    // Node: resolvePage falls back to the first visible page
    #[test]
    fn resolve_page_falls_back_to_the_first_visible_page() {
        assert_eq!(resolve_page(&core(), "modules"), "modules");
        assert_eq!(resolve_page(&core(), "gone"), "general");
        assert_eq!(resolve_page(&[], "gone"), "");
    }

    // Node: cyclePage wraps both ways
    #[test]
    fn cycle_page_wraps_both_ways() {
        assert_eq!(cycle_page(&core(), "general", 1), "modules");
        assert_eq!(cycle_page(&core(), "modules", 1), "general");
        assert_eq!(cycle_page(&core(), "general", -1), "modules");
        assert_eq!(cycle_page(&[], "general", 1), "");
    }

    // Node: sections groups a page by section in first-seen order
    #[test]
    fn sections_groups_a_page_by_section_in_first_seen_order() {
        let schema = [
            json!({ "key": "a", "section": "Hover" }),
            json!({ "key": "b", "section": "Layout" }),
            json!({ "key": "c", "section": "Hover" }),
            json!({ "key": "d", "page": "modules" }),
            json!({ "key": "e" }),
        ];
        let out = sections(&schema, "general");
        let got: Vec<(&str, Vec<Option<&str>>)> = out
            .iter()
            .map(|s| (s.name.as_str(), keys(&s.fields)))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Hover", vec![Some("a"), Some("c")]),
                ("Layout", vec![Some("b")]),
                ("", vec![Some("e")]),
            ]
        );
    }

    // Node: toList accepts arrays and array-likes, rejects the rest
    #[test]
    fn to_list_accepts_arrays_and_array_likes_rejects_the_rest() {
        assert_eq!(to_list(&json!(["a", 1])), ["a", "1"]);
        assert_eq!(to_list(&json!({ "length": 1, "0": "x" })), ["x"]);
        assert!(to_list(&json!("abc")).is_empty());
        // JS undefined is a missing value, which callers pass as null.
        assert!(to_list(&Value::Null).is_empty());
    }

    // Node: toggleId and moveId return new lists
    #[test]
    fn toggle_id_and_move_id_return_new_lists() {
        let ids = strings(&["a", "b", "c"]);
        assert_eq!(toggle_id(&ids, "b"), ["a", "c"]);
        assert_eq!(toggle_id(&ids, "d"), ["a", "b", "c", "d"]);
        assert_eq!(move_id(&ids, "c", -1), ["a", "c", "b"]);
        assert_eq!(move_id(&ids, "a", -1), ["a", "b", "c"]);
        assert_eq!(move_id(&ids, "x", 1), ["a", "b", "c"]);
        assert_eq!(ids, ["a", "b", "c"]);
    }

    // Node: allIds lists enabled ids first, then the other available ones
    #[test]
    fn all_ids_lists_enabled_ids_first_then_the_other_available_ones() {
        assert_eq!(
            all_ids(
                &strings(&["media", "ghost"]),
                &strings(&["activities", "media", "osd"])
            ),
            ["media", "ghost", "activities", "osd"]
        );
    }

    // Node: workspaceChoices offers 1-10, known and picked ones, numbers first
    #[test]
    fn workspace_choices_offers_1_10_known_and_picked_ones_numbers_first() {
        assert_eq!(
            workspace_choices(&json!(["2", "code", "12"]), &json!(["notes", 3])),
            vec![
                json!(1),
                json!(2),
                json!(3),
                json!(4),
                json!(5),
                json!(6),
                json!(7),
                json!(8),
                json!(9),
                json!(10),
                json!(12),
                json!("code"),
                json!("notes")
            ]
        );
        assert_eq!(
            workspace_choices(&Value::Null, &Value::Null),
            (1..=10).map(|n| json!(n)).collect::<Vec<_>>()
        );
    }

    // Node: toggleWorkspace keeps numbers as numbers and clears to undefined
    #[test]
    fn toggle_workspace_keeps_numbers_as_numbers_and_clears_to_none() {
        assert_eq!(
            toggle_workspace(&json!([3, 1]), &json!("2")),
            Some(vec![json!(1), json!(2), json!(3)])
        );
        assert_eq!(
            toggle_workspace(&json!([1, 2]), &json!(2)),
            Some(vec![json!(1)])
        );
        assert_eq!(
            toggle_workspace(&Value::Null, &json!("code")),
            Some(vec![json!("code")])
        );
        assert_eq!(toggle_workspace(&json!([1]), &json!(1)), None);
    }

    // Node: snapToStep snaps to the step from min, clamps, and drops float noise
    #[test]
    fn snap_to_step_snaps_to_the_step_from_min_clamps_and_drops_float_noise() {
        assert_eq!(snap_to_step(0.351, 0.0, 1.0, 0.01), 0.35);
        assert_eq!(snap_to_step(0.1 + 0.2, 0.0, 1.0, 0.01), 0.3);
        assert_eq!(snap_to_step(1.4, 0.0, 1.0, 0.01), 1.0);
        assert_eq!(snap_to_step(-3.0, 0.0, 1.0, 0.01), 0.0);
        assert_eq!(snap_to_step(124.0, 0.0, 1000.0, 10.0), 120.0);
        assert_eq!(snap_to_step(7.0, 5.0, 100.0, 10.0), 5.0);
        assert_eq!(snap_to_step(7.0, 0.0, 100.0, 0.0), 7.0);
    }

    // Node: storedValue removes the key when an edit picks the default
    #[test]
    fn stored_value_removes_the_key_when_an_edit_picks_the_default() {
        let field = json!({ "key": "position.edge", "default": "top" });
        assert_eq!(stored_value(&field, Some(&json!("top"))), None);
        assert_eq!(
            stored_value(&field, Some(&json!("left"))),
            Some(json!("left"))
        );
        assert_eq!(
            stored_value(&json!({ "default": 0.5 }), Some(&json!(0.5))),
            None
        );
        assert_eq!(
            stored_value(&json!({ "default": false }), Some(&json!(false))),
            None
        );
        assert_eq!(
            stored_value(&json!({}), Some(&json!("x"))),
            Some(json!("x"))
        );
        assert_eq!(stored_value(&json!({ "default": "top" }), None), None);
    }

    // Node: moduleFields puts a module's fields under its id, on its own page
    #[test]
    fn module_fields_puts_a_modules_fields_under_its_id_on_its_own_page() {
        let fields = module_fields(
            &[
                json!({ "moduleId": "resources", "title": "Resources", "settings": [
                    { "key": "compact", "type": "choice", "default": "always" },
                    { "key": "disk", "type": "string", "page": "extra", "section": "Disk" },
                    { "key": "color", "type": "color", "page": "appearance" },
                    { "type": "note", "label": "Linux only" }
                ] }),
                json!({ "moduleId": "media", "title": "Now playing", "settings": [] }),
                json!({ "moduleId": "bare", "settings": [{ "key": "x", "type": "boolean" }] }),
            ],
            "",
        );
        let got: Vec<_> = fields
            .iter()
            .map(|f| (str_of(f, "key"), str_of(f, "page"), str_of(f, "section")))
            .collect();
        assert_eq!(
            got,
            vec![
                (Some("resources.compact"), Some("modules/resources"), None),
                (Some("resources.disk"), Some("extra"), Some("Disk")),
                (
                    Some("resources.color"),
                    Some("appearance"),
                    Some("Resources")
                ),
                (None, Some("modules/resources"), None),
                (Some("bare.x"), Some("modules/bare"), None),
            ]
        );
        assert_eq!(fields[0]["default"], "always");
        assert!(module_fields(&[], "").is_empty());
    }

    // Node: modulePages gives each loaded module a page under Modules
    #[test]
    fn module_pages_gives_each_loaded_module_a_page_under_modules() {
        assert_eq!(
            module_pages(&[
                json!({ "moduleId": "media", "title": "Now playing" }),
                json!({ "moduleId": "bare" }),
                Value::Null
            ]),
            vec![
                json!({ "id": "modules/media", "label": "Now playing" }),
                json!({ "id": "modules/bare", "label": "bare" })
            ]
        );
    }

    // Node: a module page is a sub-page: off the sidebar, under its parent
    #[test]
    fn a_module_page_is_a_sub_page_off_the_sidebar_under_its_parent() {
        assert_eq!(
            ids(&nav_pages(&with_sub())),
            ["general", "modules", "appearance"]
        );
        assert_eq!(parent_page("modules/media"), "modules");
        assert_eq!(parent_page("general"), "general");
        // JS undefined is the empty id.
        assert_eq!(parent_page(""), "");
    }

    // Node: resolvePage falls back from a gone sub-page to its parent
    #[test]
    fn resolve_page_falls_back_from_a_gone_sub_page_to_its_parent() {
        assert_eq!(resolve_page(&with_sub(), "modules/media"), "modules/media");
        assert_eq!(resolve_page(&with_sub(), "modules/timer"), "modules");
        assert_eq!(resolve_page(&with_sub(), "gone/x"), "general");
        assert_eq!(resolve_page(&[json!({ "id": "a/b" })], "x"), "a/b");
    }

    // Node: cyclePage steps through the sidebar, from a sub-page's parent
    #[test]
    fn cycle_page_steps_through_the_sidebar_from_a_sub_pages_parent() {
        assert_eq!(cycle_page(&with_sub(), "modules", 1), "appearance");
        assert_eq!(cycle_page(&with_sub(), "modules/media", 1), "appearance");
        assert_eq!(cycle_page(&with_sub(), "modules/media", -1), "general");
    }

    // Node: moduleTitle uses the loaded module's title, else its id
    #[test]
    fn module_title_uses_the_loaded_modules_title_else_its_id() {
        assert_eq!(
            module_title("media", Some(&json!({ "title": "Now playing" }))),
            "Now playing"
        );
        assert_eq!(module_title("timer", None), "Timer");
    }

    // Node: moduleStatus says where a module shows, or whether a standalone one is on
    #[test]
    fn module_status_says_where_a_module_shows_or_whether_a_standalone_one_is_on() {
        let one = [json!({ "id": "main", "moduleIds": ["media"] })];
        let two = [
            json!({ "id": "main", "moduleIds": ["media", "osd"] }),
            json!({ "id": "jobs", "moduleIds": ["activities", "media"] }),
        ];
        assert_eq!(module_status("media", &one, false, None), "On");
        assert_eq!(module_status("timer", &one, false, None), "Off");
        assert_eq!(module_status("media", &two, false, None), "In main, jobs");
        assert_eq!(module_status("activities", &two, false, None), "In jobs");
        assert_eq!(
            module_status("osd", &two, true, Some(true)),
            "On",
            "a notch listing it does not count"
        );
        assert_eq!(module_status("osd", &one, true, Some(false)), "Off");
        assert_eq!(module_status("timer", &[], false, None), "Off");
    }

    // Node: catalogPart splits the Modules page into notch modules, agents and standalone ones
    #[test]
    fn catalog_part_splits_the_modules_page_into_notch_modules_agents_and_standalone_ones() {
        let catalog = [
            json!({ "moduleId": "media" }),
            json!({ "moduleId": "osd", "standalone": true }),
            json!({ "moduleId": "claude", "agent": true }),
            json!({ "moduleId": "timer", "standalone": false }),
        ];
        let part = |p: Option<&str>| -> Vec<String> {
            catalog_part(&catalog, p)
                .iter()
                .map(|m| str_of(m, "moduleId").unwrap().to_string())
                .collect()
        };
        assert_eq!(part(Some("notch")), ["media", "timer"]);
        assert_eq!(part(None), ["media", "timer"]);
        assert_eq!(part(Some("standalone")), ["osd"]);
        assert_eq!(part(Some("agent")), ["claude"]);
        assert!(catalog_part(&[], Some("agent")).is_empty());
    }

    // Node: an agent's file brings the fields every agent has, its command the default
    #[test]
    fn an_agents_file_brings_the_fields_every_agent_has_its_command_the_default() {
        let info = parse_module_info(
            &json!({
                "title": "Aider", "agent": { "command": " aider ", "help": { "idleMinutes": "It is light." } },
                "settings": [{ "key": "font", "type": "string", "label": "Its font", "default": "Mono" }, { "key": "extra", "type": "boolean", "default": true }]
            })
            .to_string(),
        );
        assert!(info.agent);
        assert_eq!(
            keys(&info.settings),
            [
                Some("directory"),
                Some("command"),
                Some("idleMinutes"),
                Some("width"),
                Some("height"),
                Some("font"),
                Some("extra")
            ]
        );
        assert_eq!(
            setting_default(&info.settings, "command"),
            Some(json!("aider"))
        );
        // Empty: notchd's own folder.
        assert_eq!(
            setting_default(&info.settings, "directory"),
            Some(json!(""))
        );
        let help = str_of(field_with(&info, "idleMinutes"), "help").unwrap();
        assert!(
            help.contains("Aider stops") && help.ends_with("It is light."),
            "{help}"
        );
        // Its own field wins over the inherited one.
        assert_eq!(field_with(&info, "font")["label"], "Its font");
    }

    // Node: an agent with usage gets its switch and warning, under Usage
    #[test]
    fn an_agent_with_usage_gets_its_switch_and_warning_under_usage() {
        let info = parse_module_info(
            &json!({ "title": "Claude", "agent": { "command": "claude", "usage": "claude" } })
                .to_string(),
        );
        assert_eq!(info.usage.as_deref(), Some("claude"));
        let usage: Vec<_> = info
            .settings
            .iter()
            .filter(|f| str_of(f, "section") == Some("Usage"))
            .map(|f| (str_of(f, "key"), f["default"].clone()))
            .collect();
        assert_eq!(
            usage,
            vec![(Some("usage"), json!(true)), (Some("usageWarn"), json!(80))]
        );
        let none = parse_module_info(
            &json!({ "title": "Aider", "agent": { "command": "aider" } }).to_string(),
        );
        assert_eq!(none.usage, None);
        assert!(
            !none
                .settings
                .iter()
                .any(|f| str_of(f, "key") == Some("usage"))
        );
    }

    // Node: an agent without a command is an error, not an agent
    #[test]
    fn an_agent_without_a_command_is_an_error_not_an_agent() {
        for agent in [
            json!({}),
            json!({ "command": "" }),
            json!({ "command": 3 }),
            json!("claude"),
            json!([]),
        ] {
            let info = parse_module_info(&json!({ "title": "X", "agent": agent }).to_string());
            assert!(!info.agent, "{agent}");
            assert_eq!(info.error.as_deref(), Some("agent: needs a command"));
        }
    }

    // Node: a live module's file brings the conversation's fields
    #[test]
    fn a_live_modules_file_brings_the_conversations_fields() {
        let info = parse_module_info(
            &json!({ "title": "Codex Live", "live": { "command": " codex " } }).to_string(),
        );
        assert!(info.live);
        assert!(!info.agent);
        let with_key: Vec<_> = info
            .settings
            .iter()
            .filter_map(|f| str_of(f, "key"))
            .collect();
        assert_eq!(with_key, ["command", "directory", "voice", "access"]);
        assert_eq!(
            setting_default(&info.settings, "command"),
            Some(json!("codex"))
        );
        assert_eq!(
            setting_default(&info.settings, "access"),
            Some(json!("read-only"))
        );
        // Codex's own voice by default, the others as chips.
        let voice = field_with(&info, "voice");
        assert_eq!(voice["default"], "");
        assert_eq!(voice["display"], "chips");
        assert_eq!(voice["options"][0]["value"], "");
        assert!(
            voice["options"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["value"] == "maple" && o["label"] == "Maple")
        );
        // Listed with the agents.
        assert_eq!(module_kind(&json!({ "live": true })), "agent");
        let listed = catalog_part(
            &[
                json!({ "moduleId": "codex-live", "live": true }),
                json!({ "moduleId": "media" }),
            ],
            Some("agent"),
        );
        assert_eq!(
            listed
                .iter()
                .map(|m| str_of(m, "moduleId").unwrap())
                .collect::<Vec<_>>(),
            ["codex-live"]
        );
    }

    // Node: a local live module needs no command: its fields are the agent's, the models'
    #[test]
    fn a_local_live_module_needs_no_command_its_fields_are_the_agents_the_models() {
        let info = voice_info();
        assert!(info.live);
        assert_eq!(info.error, None);
        let with_key: Vec<_> = info
            .settings
            .iter()
            .filter_map(|f| str_of(f, "key"))
            .collect();
        assert_eq!(
            with_key,
            [
                "agent",
                "command",
                "session",
                "directory",
                "access",
                "language",
                "speechModel",
                "voiceLanguage",
                "voiceModel",
                "otherVoiceModel",
                "voice",
                "localaiUrl"
            ]
        );
        let default = |k: &str| setting_default(&info.settings, k);
        assert_eq!(default("agent"), Some(json!("codex")));
        assert_eq!(default("session"), Some(json!("last")));
        assert_eq!(default("language"), Some(json!(""))); // Voxtype's own
        assert_eq!(default("command"), Some(json!("")));
        assert_eq!(default("voiceModel"), Some(json!("")));
        assert_eq!(default("voiceLanguage"), Some(json!("pt")));
        assert_eq!(default("localaiUrl"), Some(json!("")));
    }

    // Node: a live module without a command is an error
    #[test]
    fn a_live_module_without_a_command_is_an_error() {
        for live in [json!({}), json!({ "command": " " }), json!(true), json!([])] {
            let info = parse_module_info(&json!({ "title": "X", "live": live }).to_string());
            assert!(!info.live, "{live}");
            assert_eq!(info.error.as_deref(), Some("live: needs a command"));
        }
    }

    // Node: parseModuleInfo keeps a title and well-formed fields, drops the rest
    #[test]
    fn parse_module_info_keeps_a_title_and_well_formed_fields_drops_the_rest() {
        let as_json = |text: &str| serde_json::to_value(parse_module_info(text)).unwrap();
        assert_eq!(
            as_json(
                r#"{ "title": "Timer", "settings": [{ "key": "a", "type": "boolean" }, 3, { "key": "b" }] }"#
            ),
            json!({ "title": "Timer", "settings": [{ "key": "a", "type": "boolean" }] })
        );
        assert_eq!(
            as_json(r#"{ "settings": {} }"#),
            json!({ "title": "", "settings": [] })
        );
        assert!(parse_module_info(r#"{ "standalone": true }"#).standalone);
        assert!(!parse_module_info(r#"{ "standalone": "yes" }"#).standalone);
        let bad = parse_module_info("{ nope");
        assert_eq!((bad.title.as_str(), bad.settings.len()), ("", 0));
        assert!(bad.error.is_some());
        assert_eq!(
            parse_module_info("[]").error.as_deref(),
            Some("not an object")
        );
    }

    // Node: settingDefault finds a field's default by key
    #[test]
    fn setting_default_finds_a_fields_default_by_key() {
        let fields = [
            json!({ "key": "focus", "default": 25 }),
            json!({ "key": "notify", "default": false }),
        ];
        assert_eq!(setting_default(&fields, "focus"), Some(json!(25)));
        assert_eq!(setting_default(&fields, "notify"), Some(json!(false)));
        assert_eq!(setting_default(&fields, "gone"), None);
        // JS undefined fields is the empty list.
        assert_eq!(setting_default(&[], "focus"), None);
    }

    // Node: notch pages are sub-pages of Notches, by index
    #[test]
    fn notch_pages_are_sub_pages_of_notches_by_index() {
        assert_eq!(notch_page_id(2), "notch/2");
        assert_eq!(notch_index_of_page("notch/2"), 2);
        assert_eq!(notch_index_of_page("notch"), -1);
        assert_eq!(notch_index_of_page("modules/media"), -1);
        assert_eq!(notch_index_of_page("notch/x"), -1);
        assert_eq!(parent_page(&notch_page_id(0)), "notch");
    }

    // Node: notchSummary says where a notch is and what it shows
    #[test]
    fn notch_summary_says_where_a_notch_is_and_what_it_shows() {
        let titles = json!({ "media": "Now playing" });
        assert_eq!(
            notch_summary(
                &json!({ "edge": "bottom", "moduleIds": ["media", "timer"] }),
                &titles
            ),
            "Bottom edge · Now playing, Timer"
        );
        assert_eq!(
            notch_summary(&json!({ "edge": "top", "moduleIds": [] }), &titles),
            "Top edge · no modules"
        );
        assert_eq!(notch_summary(&Value::Null, &titles), "");
    }

    // Node: every module JSON file is well-formed
    // Every module's JSON file parses, and each field the window can save has
    // a key, a type and a default within its range.
    #[test]
    fn every_module_json_file_is_well_formed() {
        let dirs = ["core/modules", "hosts/omarchy/modules"];
        let list = |ext: &str| -> Vec<String> {
            dirs.iter()
                .flat_map(|dir| {
                    let mut names: Vec<String> = std::fs::read_dir(repo(dir))
                        .unwrap()
                        .map(|e| e.unwrap().file_name().into_string().unwrap())
                        .filter(|f| f.ends_with(ext))
                        .map(|f| format!("{dir}/{f}"))
                        .collect();
                    names.sort();
                    names
                })
                .collect()
        };
        let files = list(".json");
        assert!(files.len() >= 5, "files: {files:?}");
        for file in &files {
            let text = std::fs::read_to_string(repo(file)).unwrap();
            let info = parse_module_info(&text);
            assert_eq!(info.error, None, "{file}");
            assert!(!info.title.is_empty(), "{file}: no title");
            let data: Value = serde_json::from_str(&text).unwrap();
            let unknown: Vec<_> = data
                .as_object()
                .unwrap()
                .keys()
                .filter(|k| {
                    !["title", "settings", "standalone", "agent", "live"].contains(&k.as_str())
                })
                .collect();
            assert!(unknown.is_empty(), "{file}: unknown keys {unknown:?}");
            if info.standalone {
                for f in &info.settings {
                    let head = f
                        .get("key")
                        .map(js::string)
                        .unwrap_or_else(|| "undefined".into());
                    let head = head.split('.').next().unwrap().to_string();
                    assert!(
                        !["enabled", "position"].contains(&head.as_str()),
                        "{file}: {head} is the standalone switch's or position's"
                    );
                }
            }
            let inherited = if info.agent {
                agent_settings(&data["agent"], &info.title).len()
            } else if info.live {
                live_settings(&data["live"], &info.title).len()
            } else {
                0
            };
            let own = data
                .get("settings")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            assert!(
                info.settings.len() >= inherited.max(own),
                "{file}: a field was dropped"
            );
            for f in &info.settings {
                match str_of(f, "type") {
                    Some("note") => continue,
                    Some("component") => {
                        assert!(
                            js::truthy_opt(f.get("component")),
                            "{file}: a component row without a component"
                        );
                        continue;
                    }
                    Some("components") => {
                        assert!(
                            js::truthy_opt(f.get("match")),
                            "{file}: components without a match"
                        );
                        continue;
                    }
                    _ => {}
                }
                assert!(
                    js::truthy_opt(f.get("key")) && f.get("default").is_some(),
                    "{file}: {f}"
                );
                if let Some(min) = f.get("min").and_then(Value::as_f64) {
                    let d = f["default"].as_f64().unwrap();
                    assert!(
                        d >= min && d <= f["max"].as_f64().unwrap(),
                        "{file}: {} out of range",
                        f["key"]
                    );
                }
                if let Some(options) = f.get("options").and_then(Value::as_array) {
                    assert!(
                        options
                            .iter()
                            .any(|o| js::eq_opt(o.get("value"), f.get("default"))),
                        "{file}: {} default not an option",
                        f["key"]
                    );
                }
            }
        }
        for qml in list(".qml") {
            let json = format!("{}.json", qml.strip_suffix(".qml").unwrap());
            assert!(files.contains(&json), "{json} is missing");
        }
    }

    // Node: moduleInfoPath finds the JSON file next to a module, file or qs: url
    #[test]
    fn module_info_path_finds_the_json_file_next_to_a_module_file_or_qs_url() {
        assert_eq!(
            module_info_path("file:///p/core/modules/media.qml", "/c"),
            "/p/core/modules/media.json"
        );
        assert_eq!(
            module_info_path("file:///p/my%20mods/a.qml", ""),
            "/p/my mods/a.json"
        );
        assert_eq!(
            module_info_path("qs:@/qs/core/modules/timer.qml", "/home/u/.config/qs/"),
            "/home/u/.config/qs/core/modules/timer.json"
        );
        assert_eq!(module_info_path("qs:@/qs/core/modules/timer.qml", ""), "");
        // The Omarchy shell names a plugin's core modules this way; their files
        // were not found, so their titles and settings were missing there.
        assert_eq!(
            module_info_path(
                "qs:///home/u/plugins/notchd/core/modules/media.qml",
                "/usr/share/omarchy/shell"
            ),
            "/home/u/plugins/notchd/core/modules/media.json"
        );
        assert_eq!(module_info_path("https://x/y.qml", "/c"), "");
        assert_eq!(module_info_path("", "/c"), "");
    }

    // Node: a standalone module's page starts with its switch and position
    #[test]
    fn a_standalone_modules_page_starts_with_its_switch_and_position() {
        let fields = module_fields(
            &[
                json!({ "moduleId": "osd", "title": "OSD", "standalone": true, "settings": [{ "key": "size", "type": "integer", "default": 1 }] }),
            ],
            "edge.qml",
        );
        let got: Vec<_> = fields
            .iter()
            .map(|f| {
                (
                    str_of(f, "key"),
                    str_of(f, "page"),
                    str_of(f, "section"),
                    f["default"].clone(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (Some("osd.enabled"), Some("modules/osd"), None, json!(true)),
                (
                    Some("osd.position.edge"),
                    Some("modules/osd"),
                    Some("Position"),
                    json!("top")
                ),
                (
                    Some("osd.position.offset"),
                    Some("modules/osd"),
                    Some("Position"),
                    json!(0.5)
                ),
                (Some("osd.size"), Some("modules/osd"), None, json!(1)),
            ]
        );
        assert_eq!(fields[1]["options"][0]["preview"], "edge.qml");
    }

    // Node: a terminal of your own: its entry's title and command, then the fields of a terminal, its own list
    // Its title and command are its entry's, edited in place: a `command`
    // under its id as well would be a second command for the same terminal.
    #[test]
    fn a_terminal_of_your_own_its_entrys_title_and_command_then_the_fields_of_a_terminal() {
        let info = terminal_info(&json!({ "id": "btop", "title": "System", "command": "btop" }));
        assert_eq!(info.title, "System");
        assert!(info.terminal);
        assert_eq!(
            keys(&info.settings),
            [
                Some("terminals.btop.title"),
                Some("terminals.btop.command"),
                Some("directory"),
                Some("idleMinutes"),
                Some("width"),
                Some("height"),
                Some("font")
            ]
        );
        assert!(
            info.settings[0]["absolute"] == true
                && info.settings[1]["absolute"] == true
                && info.settings[1]["required"] == true
        );
        let help = str_of(&info.settings[2], "help").unwrap();
        assert!(help.contains("Empty is your home"), "{help}");
        assert_eq!(module_kind(&json!({ "terminal": true })), "terminal");
        assert_eq!(module_kind(&json!({ "agent": true })), "agent");
        // Without a title, the id names it.
        let logs = terminal_info(&json!({ "id": "logs", "title": "", "command": "x" }));
        assert!(
            str_of(&logs.settings[3], "help")
                .unwrap()
                .contains("logs stops")
        );
        // On its page, the entry's keys stay as they are; the others go under its id.
        let fields = module_fields(
            &[
                json!({ "moduleId": "btop", "title": "System", "terminal": true, "settings": info.settings }),
            ],
            "",
        );
        assert_eq!(
            keys(&fields)[..3],
            [
                Some("terminals.btop.title"),
                Some("terminals.btop.command"),
                Some("btop.directory")
            ]
        );
        assert!(
            fields
                .iter()
                .all(|f| str_of(f, "page") == Some("modules/btop"))
        );
    }

    // Node: the voice modules' pages have a row for every component they need, and each exists
    #[test]
    fn the_voice_modules_pages_have_a_row_for_every_component_they_need_and_each_exists() {
        let components = |info: &ModuleInfo| -> Vec<String> {
            info.settings
                .iter()
                .filter(|f| str_of(f, "type") == Some("component"))
                .map(|f| str_of(f, "component").unwrap().to_string())
                .collect()
        };
        let registry = registry();
        let local = voice_info();
        let codex = parse_module_info(
            &json!({ "title": "Codex Live", "live": { "command": "codex" } }).to_string(),
        );
        assert_eq!(components(&local), ["libpulse", "voxtype", "localai"]);
        assert_eq!(components(&codex), ["codex-live-audio"]);
        for id in components(&local).iter().chain(components(&codex).iter()) {
            assert!(registry.iter().any(|c| str_of(c, "id") == Some(id)), "{id}");
        }
        // a component row keeps its module's page and sections, and takes no config key
        let fields = voice_fields();
        let row = fields
            .iter()
            .find(|f| str_of(f, "component") == Some("voxtype"))
            .unwrap();
        assert_eq!(row["page"], "modules/local-live");
        assert_eq!(row["section"], "Speech to text (STT)");
        assert_eq!(row.get("key"), None);
    }

    // Node: the Voice page keeps speech to text and text to speech apart, each with a note
    #[test]
    fn the_voice_page_keeps_speech_to_text_and_text_to_speech_apart_each_with_a_note() {
        let sections = sections(&voice_fields(), "modules/local-live");
        let names: Vec<_> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Agent",
                "Audio",
                "Speech to text (STT)",
                "Text to speech (TTS)"
            ]
        );
        let named = |name: &str| &sections.iter().find(|s| s.name == name).unwrap().fields;
        for name in ["Speech to text (STT)", "Text to speech (TTS)"] {
            assert_eq!(named(name)[0]["type"], "note", "{name}");
        }
        let keys_of = |name: &str| -> Vec<String> {
            named(name)
                .iter()
                .filter_map(|f| str_of(f, "key"))
                .map(|k| k.replace("local-live.", ""))
                .collect()
        };
        assert_eq!(keys_of("Speech to text (STT)"), ["language", "speechModel"]);
        assert_eq!(
            keys_of("Text to speech (TTS)"),
            [
                "voiceLanguage",
                "voiceModel",
                "otherVoiceModel",
                "voice",
                "localaiUrl"
            ]
        );
        assert!(
            named("Speech to text (STT)")
                .iter()
                .any(|f| str_of(f, "component") == Some("voxtype"))
        );
        assert!(
            named("Text to speech (TTS)")
                .iter()
                .any(|f| str_of(f, "component") == Some("localai"))
        );
    }

    // Node: the languages: one list for what you speak and what speaks, each a choice that keeps its default
    #[test]
    fn the_languages_one_list_for_what_you_speak_and_what_speaks_each_a_choice_that_keeps_its_default()
     {
        let local = voice_info();
        let stt = field_with(&local, "language");
        let tts = field_with(&local, "voiceLanguage");
        let codes = |f: &Value| -> Vec<String> {
            f["options"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["value"].as_str().unwrap().to_string())
                .filter(|v| !v.is_empty() && v != "auto")
                .collect()
        };
        assert_eq!(codes(stt), codes(tts));
        let first_two: Vec<_> = stt["options"]
            .as_array()
            .unwrap()
            .iter()
            .take(2)
            .map(|o| o["value"].clone())
            .collect();
        assert_eq!(first_two, [json!(""), json!("auto")]);
        // every language of the speech models has both engines in the registry, and only those do
        let registry = registry();
        let voices: Vec<&Value> = registry
            .iter()
            .filter(|c| c.get("engine").is_some())
            .collect();
        let mut languages: Vec<String> = voices
            .iter()
            .map(|c| str_of(c, "language").unwrap().to_string())
            .collect();
        languages.sort();
        languages.dedup();
        let mut want = codes(tts);
        want.sort();
        assert_eq!(languages, want);
        for code in codes(tts) {
            let mut engines: Vec<_> = voices
                .iter()
                .filter(|c| str_of(c, "language") == Some(code.as_str()))
                .map(|c| str_of(c, "engine").unwrap())
                .collect();
            engines.sort();
            assert_eq!(engines, ["piper", "pocket"], "{code}");
        }
        // the label is in its own language, and a key the config file already has keeps meaning
        let pt = stt["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["value"] == "pt")
            .unwrap();
        assert_eq!(pt["label"], "Português");
        assert_eq!(stt["key"], "language");
    }

    // Node: a field with `when` shows only while another field has that value; the default counts
    #[test]
    fn a_field_with_when_shows_only_while_another_field_has_that_value_the_default_counts() {
        let schema = [
            json!({ "key": "lang", "type": "choice", "default": "pt" }),
            json!({ "key": "a", "when": { "key": "lang", "is": "pt" } }),
            json!({ "key": "b", "when": { "key": "lang", "is": ["en", "es"] } }),
            json!({ "key": "c" }),
        ];
        let shown = |stored: Value| -> Vec<String> {
            shown_fields(&schema, |k| stored.get(k).cloned())
                .iter()
                .map(|f| str_of(f, "key").unwrap().to_string())
                .collect()
        };
        assert_eq!(shown(json!({})), ["lang", "a", "c"]);
        assert_eq!(shown(json!({ "lang": "es" })), ["lang", "b", "c"]);
        assert_eq!(shown(json!({ "lang": "xx" })), ["lang", "c"]);
        assert_eq!(
            effective_value(&schema, "lang", |_| None),
            Some(json!("pt"))
        );
        assert_eq!(effective_value(&schema, "nope", |_| None), None);
    }

    // Node: moduleFields puts the references to a module's own fields under its id, as it does their keys
    #[test]
    fn module_fields_puts_the_references_to_a_modules_own_fields_under_its_id_as_it_does_their_keys()
     {
        let fields = module_fields(
            &[json!({ "moduleId": "m", "title": "M", "settings": [
                { "key": "lang", "type": "choice", "default": "pt" },
                { "key": "a", "type": "string", "default": "", "when": { "key": "lang", "is": "pt" } },
                { "type": "components", "by": { "key": "lang", "field": "language" }, "match": { "engine": "piper" } },
                { "key": "v", "type": "choice", "default": "", "options": [{ "value": "x", "needs": ["id", { "engine": "piper", "language": "@lang", "plain": "text" }] }] }
            ] })],
            "",
        );
        assert_eq!(fields[1]["when"], json!({ "key": "m.lang", "is": "pt" }));
        assert_eq!(
            fields[2]["by"],
            json!({ "key": "m.lang", "field": "language" })
        );
        assert_eq!(
            fields[3]["options"][0]["needs"],
            json!(["id", { "engine": "piper", "language": "@m.lang", "plain": "text" }])
        );
        // the same through the whole chain: the voice page's own references resolve to keys it has
        let voice = voice_fields();
        let have: Vec<&str> = voice.iter().filter_map(|f| str_of(f, "key")).collect();
        for f in &voice {
            if let Some(when) = f.get("when") {
                assert!(
                    have.contains(&when["key"].as_str().unwrap()),
                    "when {}",
                    when["key"]
                );
            }
            if let Some(by) = f.get("by") {
                assert!(
                    have.contains(&by["key"].as_str().unwrap()),
                    "by {}",
                    by["key"]
                );
            }
            for o in f
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for n in o
                    .get("needs")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    for v in n.as_object().into_iter().flat_map(|m| m.values()) {
                        if let Some(key) = v.as_str().and_then(|s| s.strip_prefix('@')) {
                            assert!(have.contains(&key), "needs {v}");
                        }
                    }
                }
            }
        }
    }

    // Node: the Voice page shows the voices of the chosen language only, and every language's on the components page
    //
    // The registry's `components` rows are made by `Components.decorate` in
    // JS; until that is ported the test expands the voice page's one
    // `components` field itself (a row per match, `when` its language). The
    // components page half of the Node test (`overviewFields`) belongs to the
    // components port.
    #[test]
    fn the_voice_page_shows_the_voices_of_the_chosen_language_only() {
        let registry = registry();
        let engines = |c: &Value| c.get("engine").is_some();
        let base: Vec<Value> = voice_fields()
            .into_iter()
            .flat_map(|f| {
                if str_of(&f, "type") != Some("components") {
                    return vec![f];
                }
                let wanted: Vec<&str> = f["match"]["engine"].as_array().unwrap().iter().map(|e| e.as_str().unwrap()).collect();
                registry
                    .iter()
                    .filter(|c| str_of(c, "engine").is_some_and(|e| wanted.contains(&e)))
                    .map(|c| {
                        json!({ "type": "component", "component": c["id"], "section": f["section"],
                                "when": { "key": f["by"]["key"], "is": c[f["by"]["field"].as_str().unwrap()] } })
                    })
                    .collect()
            })
            .collect();
        let voice_rows = |lang: Option<&str>| -> Vec<String> {
            shown_fields(&base, |k| {
                if k == voice_key() {
                    lang.map(|l| json!(l))
                } else {
                    None
                }
            })
            .iter()
            .filter(|f| str_of(f, "type") == Some("component"))
            .filter(|f| {
                registry
                    .iter()
                    .find(|c| str_of(c, "id") == str_of(f, "component"))
                    .is_some_and(engines)
            })
            .map(|f| str_of(f, "component").unwrap().to_string())
            .collect()
        };
        let mut codes: Vec<&str> = registry
            .iter()
            .filter(|c| engines(c))
            .map(|c| str_of(c, "language").unwrap())
            .collect();
        codes.sort();
        codes.dedup();
        for code in codes {
            let want: Vec<String> = registry
                .iter()
                .filter(|c| engines(c) && str_of(c, "language") == Some(code))
                .map(|c| str_of(c, "id").unwrap().to_string())
                .collect();
            assert_eq!(voice_rows(Some(code)), want, "{code}");
            assert_eq!(want.len(), 2);
        }
        assert_eq!(voice_rows(None), voice_rows(Some("pt"))); // the default language
        assert!(voice_rows(Some("xx")).is_empty());
    }
}
