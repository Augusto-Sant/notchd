//! Test hooks: IPC methods only a test needs. Off unless the daemon runs with
//! `NOTCHD_TEST_HOOKS=1` in its environment (`scripts/qml-test.sh` sets it for
//! the QML tests' daemons). Never set it for a real instance.
//!
//! - `test-linger <ms>`: how long a guest's window stays after it closes
//!   (5000 ms), so a test need not wait seconds.

use notchd_core::store::Store;

pub const ENV: &str = "NOTCHD_TEST_HOOKS";

pub fn enabled() -> bool {
    std::env::var(ENV).is_ok_and(|v| v == "1")
}

type Reply = Result<String, (&'static str, String)>;

/// Runs a hook method; `None` when `method` is not one.
pub fn apply(store: &mut Store, method: &str, args: &[String]) -> Option<Reply> {
    if method != "test-linger" {
        return None;
    }
    let ms = args.first().and_then(|a| a.parse::<u64>().ok());
    Some(match ms {
        Some(ms) => {
            store.set_window_linger(ms);
            Ok("ok".into())
        }
        None => Err(("args", "test-linger needs milliseconds".into())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linger_is_set_and_other_methods_are_not_hooks() {
        let mut s = Store::new();
        let args = |a: &str| vec![a.to_string()];
        assert_eq!(
            apply(&mut s, "test-linger", &args("300")),
            Some(Ok("ok".into()))
        );
        assert!(matches!(
            apply(&mut s, "test-linger", &args("x")),
            Some(Err(_))
        ));
        assert!(apply(&mut s, "state", &[]).is_none());
    }
}
