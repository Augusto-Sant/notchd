//! The daemon's parts: the socket server, the router, the CLI and the module host.

pub mod attach;
pub mod corehost;
pub mod daemon;
pub mod ensure;
pub mod hub;
pub mod instance;
pub mod ipc;
pub mod modhost;
pub mod modstate;
pub mod ptyhost;
pub mod router;
pub mod server;
pub mod subs;
pub mod testhooks;
