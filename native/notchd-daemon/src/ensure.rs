//! `notchd daemon --ensure`: single-instance per socket.

use crate::instance::Instance;
use std::os::unix::net::UnixStream;

/// Whether a daemon answers on the instance's socket.
pub fn running(instance: &Instance) -> bool {
    UnixStream::connect(&instance.socket).is_ok()
}

/// Returns `Ok(())` at once when a daemon runs; otherwise runs one in the
/// foreground (the host spawns `notchd daemon --ensure` as its own child;
/// detaching and the socket-activation units come with the packaging).
pub fn ensure(instance: Instance) -> Result<(), String> {
    if running(&instance) {
        return Ok(());
    }
    crate::daemon::run(instance)
}
