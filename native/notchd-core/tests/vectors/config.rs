//! `core/Config.js` against `notchd_core::config`.
use super::*;
use notchd_core::config as c;

fn spec(v: &Value) -> c::NotchSpec {
    c::NotchSpec {
        id: String::new(),
        index: 0,
        prefix: String::new(),
        module_ids: strs(&v["moduleIds"]),
        reserve_space: false,
        draggable: true,
        tabs: true,
        idle_icons: true,
        workspaces: None,
        position: None,
    }
}

fn notch_json(s: &c::NotchSpec) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("id".into(), json!(s.id));
    m.insert("index".into(), json!(s.index));
    m.insert("prefix".into(), json!(s.prefix));
    m.insert("moduleIds".into(), json!(s.module_ids));
    m.insert("reserveSpace".into(), json!(s.reserve_space));
    m.insert("draggable".into(), json!(s.draggable));
    m.insert("tabs".into(), json!(s.tabs));
    m.insert("idleIcons".into(), json!(s.idle_icons));
    m.insert(
        "workspaces".into(),
        s.workspaces.clone().map_or(Value::Null, Value::Array),
    );
    if let Some(p) = &s.position {
        m.insert("position".into(), p.clone());
    }
    Value::Object(m)
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    let cfg = arg(a, 0);
    Some(match f {
        // The engines word a syntax error differently: only the fixed message
        // is compared, any other is "syntax".
        "parse" => match c::parse(arg(a, 0).as_str().unwrap_or("")) {
            Ok(map) => json!({ "ok": true, "config": map }),
            Err(e) if e == "the config must be a JSON object" => json!({ "ok": false, "error": e }),
            Err(_) => json!({ "ok": false, "error": "syntax" }),
        },
        "valueAt" => or_undef(c::value_at(cfg, &text(a, 1)).cloned()),
        "withValue" => c::with_value(cfg, &text(a, 1), opt(a, 2)),
        "workspaceList" => or_null(c::workspace_list(opt(a, 0)).map(Value::Array)),
        "resolve" => {
            let r = c::resolve(cfg);
            json!({ "hoverOpenDelay": n(r.hover_open_delay), "hoverCloseDelay": n(r.hover_close_delay) })
        }
        "hasNotchList" => json!(c::has_notch_list(cfg)),
        "defaultNotchId" => json!(c::default_notch_id(int(a, 0) as usize)),
        "notches" => {
            let defaults = strs(arg(a, 1));
            let defaults: Vec<&str> = defaults.iter().map(String::as_str).collect();
            Value::Array(c::notches(cfg, &defaults).iter().map(notch_json).collect())
        }
        "moduleUnion" => {
            let specs: Vec<_> = list(a, 0).iter().map(spec).collect();
            let extra: Vec<String> = arg(a, 1)
                .as_array()
                .map(|l| {
                    l.iter()
                        .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
            json!(c::module_union(&specs, &extra))
        }
        "moduleLoadList" => {
            let specs: Vec<_> = list(a, 0).iter().map(spec).collect();
            let (standalone, enabled) = (strs(arg(a, 1)), strs(arg(a, 2)));
            let standalone: Vec<&str> = standalone.iter().map(String::as_str).collect();
            let enabled: Vec<&str> = enabled.iter().map(String::as_str).collect();
            json!(c::module_load_list(&specs, &standalone, &enabled))
        }
        "freeNotchId" => {
            let ids = strs(arg(a, 0));
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            json!(c::free_notch_id(&ids))
        }
        "withNotchAdded" => c::with_notch_added(cfg, arg(a, 1)),
        "withoutNotch" => c::without_notch(cfg, int(a, 1)),
        "withWorkspace" => {
            let list = arg(a, 0).as_array();
            let ws = arg(a, 1);
            or_null(c::with_workspace(list.map(Vec::as_slice), Some(ws)).map(Value::Array))
        }
        "terminals" => {
            let taken = strs(arg(a, 1));
            let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
            let t = c::terminals(cfg, &taken);
            let list: Vec<Value> = t
                .list
                .iter()
                .map(|x| json!({ "id": x.id, "title": x.title, "command": x.command }))
                .collect();
            json!({ "list": list, "errors": t.errors })
        }
        "withTerminalAdded" => {
            let taken = strs(arg(a, 1));
            let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
            let t = c::with_terminal_added(cfg, &taken);
            json!({ "config": t.config, "id": t.id })
        }
        "withoutTerminal" => c::without_terminal(cfg, &text(a, 1)),
        "reservedKeys" => json!(c::reserved_keys()),
        _ => return None,
    })
}
