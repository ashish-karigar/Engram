//! Read the coding agents' own session logs, so Engram knows what was discussed and why,
//! not only what changed. One parser per agent; everything downstream is agent-agnostic.

pub mod claude;
pub mod codex;
pub mod muse;
pub mod source;
pub mod sync;
pub mod transcript;
pub mod turn;

pub use source::{Agent, Sources};
pub use sync::{SyncReport, sync_sessions};
pub use turn::{Role, ToolUse, Turn};
