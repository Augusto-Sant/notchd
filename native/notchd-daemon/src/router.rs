//! The router: one parsed `ToDaemon` message applied to the store, the replies
//! out. Pure over a [`Store`] and a time, so it is tested without a socket.
//! Handshake, broadcasting and the config file live in `hub` and `server`.

use crate::testhooks;
use notchd_core::navigate;
use notchd_core::state::ModuleInfo;
use notchd_core::store::{EditOp, ModuleFacts, Store};
use notchd_proto::request::{EditOp as WireOp, ErrReply, OkReply};
use notchd_proto::ui::DragEv;
use notchd_proto::version::IPC_METHODS;
use notchd_proto::{ToDaemon, ToUi};
use serde_json::Value;

pub fn err(id: Option<u64>, code: &str, message: impl Into<String>) -> ToUi {
    ToUi::Err(ErrReply {
        id,
        code: code.into(),
        message: Some(message.into()),
        want: None,
    })
}

pub fn ok(id: u64, result: Option<String>) -> ToUi {
    ToUi::Ok(OkReply { id, result })
}

const SET_USAGE: &str = "usage: set <key> <json value>";

/// Applies a message and returns the reply to a request, or nothing for a
/// fire-and-forget message. `hello` is handled by the connection, not here.
/// `now` is the daemon's clock in milliseconds.
pub fn route(store: &mut Store, msg: &ToDaemon, now: u64) -> Vec<ToUi> {
    match msg {
        ToDaemon::Screens(s) => store.set_screens(s.clone()),
        ToDaemon::Pointer(p) => store.pointer(&p.notch, &p.screen, p.ev, now),
        ToDaemon::Key(k) => store.key(&k.key, &k.mods, now),
        ToDaemon::Tab(t) => store.tab(&t.notch, &t.screen, &t.module, now),
        ToDaemon::Settings(s) => store.settings(s.open, s.page.clone()),
        ToDaemon::Catalog(c) => store.set_ui_catalog(c.modules.clone(), c.default_modules.clone()),
        ToDaemon::Measure(m) => {
            store.measure(&m.notch, &m.screen, m.rest.0, m.compact.0, m.baseline.0)
        }
        ToDaemon::Drag(d) => match (d.ev, &d.position) {
            (DragEv::Start, _) => store.drag_start(&d.notch, &d.screen, now),
            (DragEv::Cancel, _) => store.drag_cancel(&d.notch, &d.screen, now),
            (DragEv::Drop, Some(p)) => store.drop_notch(
                &d.notch,
                &d.screen,
                navigate::Position {
                    edge: match p.edge {
                        notchd_proto::common::Edge::Top => navigate::Edge::Top,
                        notchd_proto::common::Edge::Bottom => navigate::Edge::Bottom,
                        notchd_proto::common::Edge::Left => navigate::Edge::Left,
                        notchd_proto::common::Edge::Right => navigate::Edge::Right,
                    },
                    offset: p.offset.0,
                    screen: p.screen.clone(),
                },
                now,
            ),
            // A drop without a position fails validation before it gets here.
            (DragEv::Drop, None) => {}
        },
        ToDaemon::Pin(p) => store.pin(&p.notch, &p.screen, p.on, now),
        ToDaemon::Focus(f) => store.focus(&f.notch, &f.screen, f.active, now),
        ToDaemon::Facts(f) => store.set_ui_facts(
            f.modules
                .iter()
                .map(|m| ModuleFacts {
                    info: ModuleInfo {
                        id: m.id.clone(),
                        module_id: Some(m.id.clone()),
                        priority: f64::from(m.priority),
                        active: m.active,
                        compact: m.compact,
                        icon: m.icon,
                        peek: m.peek,
                        attention: m.attention,
                        urgent_peek: m.urgent_peek,
                    },
                    expanded: m.expanded,
                    peek_on_focused_screen: m.peek_on_focused_screen,
                    standalone: m.standalone,
                })
                .collect(),
        ),
        ToDaemon::Peek(p) => store.request_peek(&p.module, p.ms.map(|n| n.0), now),
        ToDaemon::PeekDismiss(p) => store.dismiss_peek(p.module.as_deref().unwrap_or("")),
        ToDaemon::ModuleNotch(m) => store.module_op(&m.module, m.op, now),
        ToDaemon::Set(r) => {
            // A `null` on the wire is JS's `undefined`: the key goes.
            let value = (!r.value.is_null()).then_some(&r.value);
            store.set_value(&r.key, value);
            return vec![ok(r.id, None)];
        }
        ToDaemon::Edit(r) => {
            let (notch, op) = match &r.op {
                WireOp::MoveModule {
                    notch,
                    module,
                    delta,
                } => (notch, EditOp::MoveModule(module.clone(), i64::from(*delta))),
                WireOp::ToggleModule { notch, module } => {
                    (notch, EditOp::ToggleModule(module.clone()))
                }
                WireOp::ToggleWorkspace { notch, workspace } => {
                    (notch, EditOp::ToggleWorkspace(workspace.clone()))
                }
            };
            return vec![match store.edit(notch, &op) {
                Ok(()) => ok(r.id, None),
                Err(e) => err(Some(r.id), "args", e),
            }];
        }
        ToDaemon::Ipc(r) => {
            return vec![match ipc(store, &r.method, &r.args, now) {
                Ok(result) => ok(r.id, Some(result)),
                Err((code, message)) => err(Some(r.id), code, message),
            }];
        }
        // A module's `event` goes to the module (the hub); the reserve ack is
        // not needed (the shrink delay runs in the core); surfaces wait.
        ToDaemon::Hello(_)
        | ToDaemon::Event(_)
        | ToDaemon::ReserveApplied(_)
        | ToDaemon::Surface(_)
        | ToDaemon::Attach(_) => {}
    }
    Vec::new()
}

type IpcResult = Result<String, (&'static str, String)>;

/// The `ipc` methods that only need config and state: the same strings
/// `NotchService.qml`'s `IpcHandler` replies with.
fn ipc(store: &mut Store, method: &str, args: &[String], now: u64) -> IpcResult {
    let arg = |i: usize| {
        args.get(i)
            .map(String::as_str)
            .ok_or(("args", format!("{method} needs {} argument(s)", i + 1)))
    };
    match method {
        "open" => store.open(now),
        "close" => store.close(now),
        "toggle" => store.toggle(now),
        "openNotch" | "closeNotch" | "toggleNotch" => {
            let id = arg(0)?;
            let command = method.trim_end_matches("Notch");
            return Ok(if store.notch_command(id, command, now) {
                "ok".into()
            } else {
                format!("no notch {id}")
            });
        }
        "settings" => {
            let open = !store.settings_open;
            store.settings(open, None);
        }
        // The connection layer acts on it (server.rs): stop and exit once
        // the reply is flushed. The store has nothing to do.
        "shutdown" => return Ok("ok".into()),
        "navigate" => return Ok(store.navigate(now)),
        "recenter" => store.recenter(),
        "set" => return Ok(ipc_set(store, args)),
        "state" => return Ok(store.ipc_state()),
        m if m.starts_with("test-") && testhooks::enabled() => {
            return testhooks::apply(store, m, args)
                .unwrap_or_else(|| Err(("method", format!("unknown method: {m}"))));
        }
        m if IPC_METHODS.contains(&m) => {
            return Err(("method", format!("{m} is not implemented yet")));
        }
        m => return Err(("method", format!("unknown method: {m}"))),
    }
    Ok(String::new())
}

/// `set <key> <json>`: an empty value resets the key to its default.
fn ipc_set(store: &mut Store, args: &[String]) -> String {
    let Some(key) = args.first().filter(|k| !k.is_empty()) else {
        return SET_USAGE.into();
    };
    let text = args.get(1).map_or("", String::as_str);
    if text.is_empty() {
        store.set_value(key, None);
        return "ok".into();
    }
    match serde_json::from_str::<Value>(text) {
        Ok(v) => {
            store.set_value(key, Some(&v));
            "ok".into()
        }
        Err(_) => format!("not JSON: {text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notchd_proto::request::{Ipc, Set};
    use serde_json::json;

    fn ipc_msg(method: &str, args: &[&str]) -> ToDaemon {
        ToDaemon::Ipc(Ipc {
            id: 7,
            method: method.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
        })
    }

    fn result(replies: Vec<ToUi>) -> Result<String, (String, String)> {
        match &replies[..] {
            [ToUi::Ok(o)] => Ok(o.result.clone().unwrap_or_default()),
            [ToUi::Err(e)] => Err((e.code.clone(), e.message.clone().unwrap_or_default())),
            other => panic!("{other:?}"),
        }
    }

    fn store() -> Store {
        let mut s = Store::new();
        s.set_config(json!({"hoverOpenDelay": 100, "theme": {"accent": "red"}}));
        s
    }

    #[test]
    fn unknown_method_is_a_method_error_with_the_id() {
        let mut s = store();
        let ToUi::Err(e) = &route(&mut s, &ipc_msg("nope", &[]), 0)[0] else {
            panic!()
        };
        assert_eq!((e.id, e.code.as_str()), (Some(7), "method"));
        assert!(e.message.as_deref().unwrap().contains("unknown method"));
    }

    #[test]
    fn shutdown_is_acknowledged_for_the_connection_to_act_on() {
        let mut s = store();
        assert_eq!(
            result(route(&mut s, &ipc_msg("shutdown", &[]), 0)).unwrap(),
            "ok"
        );
    }

    #[test]
    fn a_module_method_is_not_yet_implemented() {
        let mut s = store();
        for m in ["push", "timer", "agent", "claude", "live", "component"] {
            let (code, msg) = result(route(&mut s, &ipc_msg(m, &["x"]), 0)).unwrap_err();
            assert_eq!(code, "method");
            assert!(msg.contains("not implemented"), "{m}: {msg}");
        }
    }

    #[test]
    fn set_through_ipc_follows_the_qml_replies() {
        let mut s = store();
        let r = |s: &mut Store, a: &[&str]| result(route(s, &ipc_msg("set", a), 0)).unwrap();
        assert_eq!(r(&mut s, &["hoverOpenDelay", "250"]), "ok");
        assert_eq!(s.config()["hoverOpenDelay"], 250);
        assert_eq!(r(&mut s, &["hoverOpenDelay", ""]), "ok", "empty resets");
        assert!(s.config().get("hoverOpenDelay").is_none());
        assert_eq!(r(&mut s, &["theme.accent", ""]), "ok");
        assert!(s.config().get("theme").is_none(), "the emptied parent goes");
        assert_eq!(r(&mut s, &["a.b", "[1,2]"]), "ok");
        assert_eq!(s.config()["a"]["b"], json!([1, 2]));
        assert_eq!(r(&mut s, &["x", "nope"]), "not JSON: nope");
        assert_eq!(r(&mut s, &[]), SET_USAGE);
    }

    #[test]
    fn a_set_request_with_null_removes_and_a_value_sets() {
        let mut s = store();
        let set = |key: &str, value: Value| {
            ToDaemon::Set(Set {
                id: 3,
                key: key.into(),
                value,
            })
        };
        assert!(matches!(
            &route(&mut s, &set("theme.accent", Value::Null), 0)[..],
            [ToUi::Ok(o)] if o.id == 3 && o.result.is_none()
        ));
        assert!(s.config().get("theme").is_none());
        route(&mut s, &set("hoverOpenDelay", json!(200)), 0);
        assert_eq!(s.config()["hoverOpenDelay"], 200);
        assert!(s.take_write().is_some());
    }

    #[test]
    fn notch_commands_need_an_id_and_say_when_there_is_no_such_notch() {
        let mut s = store();
        let (code, _) = result(route(&mut s, &ipc_msg("openNotch", &[]), 0)).unwrap_err();
        assert_eq!(code, "args");
        assert_eq!(
            result(route(&mut s, &ipc_msg("openNotch", &["zzz"]), 0)).unwrap(),
            "no notch zzz"
        );
        assert_eq!(
            result(route(&mut s, &ipc_msg("toggleNotch", &["main"]), 0)).unwrap(),
            "ok"
        );
    }

    #[test]
    fn settings_toggles_and_state_is_json() {
        let mut s = store();
        route(&mut s, &ipc_msg("settings", &[]), 0);
        assert!(s.settings_open);
        route(&mut s, &ipc_msg("settings", &[]), 0);
        assert!(!s.settings_open);
        let json = result(route(&mut s, &ipc_msg("state", &[]), 0)).unwrap();
        assert!(serde_json::from_str::<Value>(&json).unwrap()["notches"].is_array());
    }

    #[test]
    fn input_messages_have_no_reply() {
        let mut s = store();
        let m = notchd_proto::parse::<ToDaemon>(
            r#"{"t":"pointer","notch":"main","screen":"DP-1","ev":"enter"}"#,
        )
        .unwrap();
        assert!(route(&mut s, &m, 0).is_empty());
    }

    #[test]
    fn an_edit_of_a_missing_notch_is_an_args_error() {
        let mut s = store();
        let m = notchd_proto::parse::<ToDaemon>(
            r#"{"t":"edit","id":4,"op":"toggleModule","notch":"zzz","module":"media"}"#,
        )
        .unwrap();
        let (code, _) = result(route(&mut s, &m, 0)).unwrap_err();
        assert_eq!(code, "args");
    }
}
