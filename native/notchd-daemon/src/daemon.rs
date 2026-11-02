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
    // The user modules: folders of `modules/` next to the config. Without a
    // home for their logs there is nowhere to put them: none run.
    match crate::server::log_dir(&|k| std::env::var(k).ok()) {
        Some(logs) => server.start_modules(&crate::server::modules_dir(&instance.config), &logs),
        None => eprintln!("notchd: no XDG_STATE_HOME or HOME: user modules are not started"),
    }
    server.start_providers();
    server.serve();
    Ok(())
}
