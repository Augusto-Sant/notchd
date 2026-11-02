//! `notchd daemon`: bind the instance's socket and serve.

use crate::instance::Instance;
use crate::server::Server;

pub fn run(instance: Instance) -> Result<(), String> {
    let server = Server::bind(instance)?;
    server.serve();
    Ok(())
}
