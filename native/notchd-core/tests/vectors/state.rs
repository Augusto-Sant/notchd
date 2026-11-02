//! `core/State.js` against `notchd_core::state`. Modules are named by `id`:
//! the functions that return modules return their ids.
use super::*;
use notchd_core::state as s;

fn modules(v: &Value) -> Vec<Option<s::ModuleInfo>> {
    from_json(v)
}

fn present(v: &Value) -> Vec<s::ModuleInfo> {
    modules(v).into_iter().flatten().collect()
}

fn ids<'a>(l: impl IntoIterator<Item = &'a s::ModuleInfo>) -> Value {
    json!(l.into_iter().map(|m| m.id.clone()).collect::<Vec<_>>())
}

fn id_or_null(m: Option<&s::ModuleInfo>) -> Value {
    m.map_or(Value::Null, |m| json!(m.id))
}

pub fn call(f: &str, a: &[Value]) -> Option<Value> {
    let ctx = arg(a, 0);
    Some(match f {
        "activeModules" => ids(&s::active_modules(&modules(ctx))),
        "compactModule" => {
            let active = present(ctx);
            id_or_null(s::compact_module(&active))
        }
        "compactModules" => {
            let active = if ctx.is_null() { vec![] } else { present(ctx) };
            ids(s::compact_modules(
                &active,
                arg(a, 1).as_bool().unwrap_or(false),
                arg(a, 2).as_bool().unwrap_or(false),
            ))
        }
        "attentionPeek" => id_or_null(s::attention_peek(&if ctx.is_null() {
            vec![]
        } else {
            modules(ctx)
        })),
        "canPeek" => {
            let module: Option<s::ModuleInfo> = from_json(ctx);
            json!(s::can_peek(module.as_ref(), &from_json(arg(a, 1))))
        }
        "mode" => to_json(&s::mode(&from_json(ctx))),
        "needsWindow" => json!(s::needs_window(&from_json(ctx))),
        "shownModules" => {
            let mods = if ctx.is_null() { vec![] } else { present(ctx) };
            let r = s::shown_modules(&mods, arg(a, 1).as_bool().unwrap_or(false), &text(a, 2));
            json!({ "modules": ids(r.modules.iter().copied()), "tabs": ids(r.tabs.iter().copied()) })
        }
        "wantsKeyboard" => json!(s::wants_keyboard(&from_json(ctx))),
        "keyboardFocus" => to_json(&s::keyboard_focus(&from_json(ctx))),
        "keyboardChange" => to_json(&s::keyboard_change(&from_json(ctx))),
        "reservedSpace" => n(s::reserved_space(&from_json(ctx))),
        "reserveStep" => to_json(&s::reserve_step(num(a, 0), num(a, 1))),
        "workspaceAllowed" => {
            let list: Option<Vec<s::WorkspaceKey>> = from_json(ctx);
            let ws: Option<s::Workspace> = from_json(arg(a, 1));
            json!(s::workspace_allowed(list.as_deref(), ws.as_ref()))
        }
        "notchScreen" => json!(s::notch_screen(&from_json(ctx))),
        "reserveScreens" => {
            let screens: Option<Vec<s::ScreenState>> = from_json(ctx);
            json!(s::reserve_screens(
                &screens.unwrap_or_default(),
                &text(a, 1)
            ))
        }
        "peekCopy" => {
            let copies: Option<Vec<String>> = from_json(ctx);
            or_null(s::peek_copy(&copies.unwrap_or_default(), &text(a, 1)).map(|c| json!(c)))
        }
        "needsVisitor" => {
            let peeks: Option<Vec<s::PeekNotch>> = from_json(ctx);
            json!(s::needs_visitor(&peeks.unwrap_or_default(), &text(a, 1)))
        }
        "focusedPeek" => {
            let peeks: Option<Vec<s::PeekNotch>> = from_json(ctx);
            json!(s::focused_peek(&peeks.unwrap_or_default(), &text(a, 1)).map_or(-1, |i| i as i64))
        }
        "reserveZones" => {
            let reserves: Option<Vec<Option<s::Reserve>>> = from_json(ctx);
            to_json(&s::reserve_zones(&reserves.unwrap_or_default()))
        }
        _ => return None,
    })
}
