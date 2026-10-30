//! Runs Engram: many projects, one local server, one set of connectors.

pub mod connectors;
pub mod daemon;
pub mod paths;
pub mod project;
pub mod registry;
pub mod secrets;

pub use daemon::{Daemon, DaemonOptions, RunningDaemon, start_daemon};
