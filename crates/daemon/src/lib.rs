//! Runs Engram: many projects, one local server, one set of connectors.

pub mod connectors;
pub mod daemon;
pub mod hook;
pub mod paths;
pub mod project;
pub mod registry;
pub mod secrets;
pub mod settings;

pub use daemon::{DEFAULT_PORT, Daemon, DaemonOptions, RunningDaemon, configured_port, start_daemon};
