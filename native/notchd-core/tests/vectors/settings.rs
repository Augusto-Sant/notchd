//! `core/settings/Settings.js` against `notchd_core::settings`.
use super::*;
use notchd_core::settings as s;

/// The engines word a JSON syntax error differently: only the fixed messages
/// are compared, any other is "syntax".
fn info(i: &s::ModuleInfo) -> Value {
    let mut v = to_json(i);
    if let Some(Value::String(e)) = v.get("error").cloned() {
        let fixed = [
            "not an object",
            "agent: needs a command",
            "live: needs a command",
        ];
        if !fixed.contains(&e.as_str()) {
            v["error"] = json!("syntax");
        }
    }
    v
}

fn values_of(a: &[Value], i: usize) -> impl Fn(&str) -> Option<Value> + '_ {
    let v = arg(a, i);
    move |k| v.get(k).cloned()
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    Some(match f {
        "mergePages" => json!(s::merge_pages(&list(a, 0), &list(a, 1))),
        "parentPage" => json!(s::parent_page(&text(a, 0))),
        "isSubPage" => json!(s::is_sub_page(arg(a, 0))),
        "visiblePages" => json!(s::visible_pages(&list(a, 0), &list(a, 1))),
        "navPages" => json!(s::nav_pages(&list(a, 0))),
        "findPage" => or_null(s::find_page(&list(a, 0), &text(a, 1)).cloned()),
        "resolvePage" => json!(s::resolve_page(&list(a, 0), &text(a, 1))),
        "cyclePage" => json!(s::cycle_page(&list(a, 0), &text(a, 1), int(a, 2))),
        "effectiveValue" => or_undef(s::effective_value(
            &list(a, 0),
            &text(a, 1),
            values_of(a, 2),
        )),
        "shownFields" => json!(s::shown_fields(&list(a, 0), values_of(a, 1))),
        "sections" => to_json(&s::sections(&list(a, 0), &text(a, 1))),
        "modulePageId" => json!(s::module_page_id(&text(a, 0))),
        "parseModuleInfo" => info(&s::parse_module_info(&text(a, 0))),
        "agentSettings" => json!(s::agent_settings(arg(a, 0), &text(a, 1))),
        "liveSettings" => json!(s::live_settings(arg(a, 0), &text(a, 1))),
        "terminalInfo" => info(&s::terminal_info(arg(a, 0))),
        "moduleInfoPath" => json!(s::module_info_path(&text(a, 0), &text(a, 1))),
        "settingDefault" => or_undef(s::setting_default(&list(a, 0), &text(a, 1))),
        "moduleFields" => json!(s::module_fields(&list(a, 0), &text(a, 1))),
        "modulePages" => json!(s::module_pages(&list(a, 0))),
        "moduleTitle" => json!(s::module_title(
            &text(a, 0),
            opt(a, 1).filter(|v| !v.is_null())
        )),
        "moduleKind" => json!(s::module_kind(arg(a, 0))),
        "catalogPart" => json!(s::catalog_part(&list(a, 0), arg(a, 1).as_str())),
        "moduleStatus" => json!(s::module_status(
            &text(a, 0),
            &list(a, 1),
            arg(a, 2).as_bool().unwrap_or(false),
            arg(a, 3).as_bool()
        )),
        "indexIn" => json!(s::index_in(opt(a, 0), &text(a, 1)).map_or(-1, |i| i as i64)),
        "notchPageId" => json!(s::notch_page_id(int(a, 0) as u64)),
        "notchIndexOfPage" => json!(s::notch_index_of_page(&text(a, 0))),
        "notchSummary" => json!(s::notch_summary(arg(a, 0), arg(a, 1))),
        "toList" => json!(s::to_list(arg(a, 0))),
        "toggleId" => json!(s::toggle_id(&strs(arg(a, 0)), &text(a, 1))),
        "moveId" => json!(s::move_id(&strs(arg(a, 0)), &text(a, 1), int(a, 2))),
        "allIds" => json!(s::all_ids(&strs(arg(a, 0)), &strs(arg(a, 1)))),
        "workspaceValue" => s::workspace_value(arg(a, 0)),
        "sortWorkspaces" => {
            let mut l = list(a, 0);
            s::sort_workspaces(&mut l);
            json!(l)
        }
        "workspaceChoices" => json!(s::workspace_choices(arg(a, 0), arg(a, 1))),
        "toggleWorkspace" => or_undef(s::toggle_workspace(arg(a, 0), arg(a, 1)).map(Value::Array)),
        "snapToStep" => n(s::snap_to_step(
            num(a, 0),
            num(a, 1),
            num(a, 2),
            step(arg(a, 3)),
        )),
        "storedValue" => or_undef(s::stored_value(arg(a, 0), opt(a, 1))),
        _ => return None,
    })
}

/// JS `Number(step)` on a numeric string (the Rust signature takes a number).
fn step(v: &Value) -> f64 {
    match v {
        Value::String(t) => t.trim().parse().unwrap_or(f64::NAN),
        _ => v.as_f64().unwrap_or(f64::NAN),
    }
}
