//! `notchd daemon`: bind the instance's socket and serve.

use crate::instance::Instance;
use crate::server::Server;

pub fn run(instance: Instance) -> Result<(), String> {
    let server = Server::bind(instance.clone())?;
    // The first line of the log `--ensure` points the daemon's stderr at.
    eprintln!(
        "notchd {} (pid {}) listening on {}",
        crate::hub::VERSION,
        std::process::id(),
        instance.socket.display()
    );
    server.serve();
    Ok(())
}
