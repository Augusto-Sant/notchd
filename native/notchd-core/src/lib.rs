//! notchd's rules, ported from the plain-JS files under `core/`.
//! Each module is one JS file; its tests are that file's Node tests.

pub mod agent;
pub mod components;
pub mod config;
pub mod countdown;
pub mod live;
pub mod navigate;
pub mod peek;
pub mod position;
pub mod resources;
pub mod schema;
pub mod settings;
pub mod state;
pub mod store;
pub mod terminal;
pub mod timers;
pub mod usage;

pub(crate) mod js;
