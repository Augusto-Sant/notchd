//! Test hooks: IPC methods that stand in for the module host, so a test can
//! give the daemon the facts and peek requests the built-in modules would.
//! Off unless the daemon runs with `NOTCHD_TEST_HOOKS=1` in its environment
//! (the QML shadow test, `tests/qml/shadow.qml`, sets it for its own daemon).
//! Never set it for a real instance.
//!
//! - `test-module <json>`: a module's facts, replacing its earlier ones.
//!   `{"id":"a","active":true,"priority":0,"compact":true,"peek":true,
//!   "icon":false,"attention":false,"urgentPeek":false,"expanded":true,
//!   "peekOnFocusedScreen":false,"standalone":false}`; absent flags are false.
//! - `test-module-remove <id>`
//! - `test-peek <id> [ms]`: `NotchService.requestPeek` for the module.
//! - `test-dismiss [id]`: `NotchService.dismissPeek` (every peek for no id).

use notchd_core::state::ModuleInfo;
use notchd_core::store::{ModuleFacts, Store};
use serde_json::Value;

pub const ENV: &str = "NOTCHD_TEST_HOOKS";

pub fn enabled() -> bool {
    std::env::var(ENV).is_ok_and(|v| v == "1")
}

type Reply = Result<String, (&'static str, String)>;

/// Runs a hook method; `None` when `method` is not one.
pub fn apply(store: &mut Store, method: &str, args: &[String], now: u64) -> Option<Reply> {
    if !matches!(
        method,
        "test-module" | "test-module-remove" | "test-peek" | "test-dismiss"
    ) {
        return None;
    }
    let arg = |i: usize| args.get(i).map(String::as_str);
    let need = |i: usize| arg(i).ok_or(("args", format!("{method} needs {} argument(s)", i + 1)));
    let run = |store: &mut Store| -> Reply {
        match method {
            "test-module" => {
                let value: Value = serde_json::from_str(need(0)?)
                    .map_err(|e| ("args", format!("not JSON: {e}")))?;
                let flag = |k: &str| value.get(k).and_then(Value::as_bool).unwrap_or(false);
                let mut info: ModuleInfo = serde_json::from_value(value.clone())
                    .map_err(|e| ("args", format!("bad facts: {e}")))?;
                if info.id.is_empty() {
                    return Err(("args", "facts need an id".into()));
                }
                info.module_id.get_or_insert_with(|| info.id.clone());
                store.set_module(ModuleFacts {
                    info,
                    expanded: flag("expanded"),
                    peek_on_focused_screen: flag("peekOnFocusedScreen"),
                    standalone: flag("standalone"),
                });
            }
            "test-module-remove" => store.remove_module(need(0)?),
            "test-peek" => {
                let ms = arg(1).and_then(|s| s.parse::<f64>().ok());
                store.request_peek(need(0)?, ms, now);
            }
            _ => store.dismiss_peek(arg(0).unwrap_or("")),
        }
        Ok("ok".into())
    };
    Some(run(store))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_and_peeks_reach_the_store() {
        let mut s = Store::new();
        let run = |s: &mut Store, m: &str, a: &[&str]| {
            let args: Vec<String> = a.iter().map(|x| x.to_string()).collect();
            apply(s, m, &args, 0).unwrap()
        };
        let facts = r#"{"id":"a","active":true,"priority":5,"peek":true}"#;
        assert_eq!(run(&mut s, "test-module", &[facts]), Ok("ok".into()));
        let m = s.module("a").unwrap();
        assert!(m.info.active && m.info.peek && m.info.priority == 5.0);
        assert_eq!(m.info.module_id.as_deref(), Some("a"));
        assert!(run(&mut s, "test-module", &["nope"]).is_err());
        assert!(run(&mut s, "test-module", &["{}"]).is_err());
        assert_eq!(run(&mut s, "test-module-remove", &["a"]), Ok("ok".into()));
        assert!(s.module("a").is_none());
        assert!(apply(&mut s, "state", &[], 0).is_none());
    }
}
