//! The router: one parsed `ToDaemon` message in, the replies out. Pure, so it
//! is tested without a socket. Handshake and config sending live in `server`.

use notchd_proto::request::{ErrReply, OkReply};
use notchd_proto::version::IPC_METHODS;
use notchd_proto::{ToDaemon, ToUi};

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

/// The reply to a request, or nothing for a fire-and-forget message
/// (`pointer`, `key`, ...: accepted and, until the core exists, ignored).
/// `hello` is handled by the connection, not here.
pub fn route(msg: &ToDaemon) -> Vec<ToUi> {
    match msg {
        ToDaemon::Ipc(r) => {
            if IPC_METHODS.contains(&r.method.as_str()) {
                vec![err(
                    Some(r.id),
                    "method",
                    format!("{} is not implemented yet", r.method),
                )]
            } else {
                vec![err(
                    Some(r.id),
                    "method",
                    format!("unknown method: {}", r.method),
                )]
            }
        }
        ToDaemon::Set(r) => vec![err(Some(r.id), "config", "set is not implemented yet")],
        ToDaemon::Edit(r) => vec![err(Some(r.id), "args", "edit is not implemented yet")],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notchd_proto::request::Ipc;

    fn ipc(method: &str) -> ToDaemon {
        ToDaemon::Ipc(Ipc {
            id: 7,
            method: method.into(),
            args: vec![],
        })
    }

    #[test]
    fn unknown_method_is_a_method_error_with_the_id() {
        let ToUi::Err(e) = &route(&ipc("nope"))[0] else {
            panic!()
        };
        assert_eq!((e.id, e.code.as_str()), (Some(7), "method"));
        assert!(e.message.as_deref().unwrap().contains("unknown method"));
    }

    #[test]
    fn a_known_method_is_not_yet_implemented() {
        let ToUi::Err(e) = &route(&ipc("state"))[0] else {
            panic!()
        };
        assert!(e.message.as_deref().unwrap().contains("not implemented"));
    }

    #[test]
    fn input_messages_have_no_reply() {
        let m = notchd_proto::parse::<ToDaemon>(
            r#"{"t":"pointer","notch":"main","screen":"DP-1","ev":"enter"}"#,
        )
        .unwrap();
        assert!(route(&m).is_empty());
    }
}
