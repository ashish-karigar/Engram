//! Serve memory to agents (MCP and HTTP) and to the desktop UI, across many projects.

pub mod api;
pub mod brief;
pub mod budget;
pub mod format;
pub mod http;
pub mod mcp;
pub mod ops;
pub mod profile;
pub mod project;
pub mod recall;
pub mod tokens;

pub use http::{RunningServer, start_server};
pub use project::{ConnectOutcome, ProjectInfo, ProjectManager, ProjectRef};
