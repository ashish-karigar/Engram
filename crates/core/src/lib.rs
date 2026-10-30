//! Storage and graph model for Engram: immutable event log, temporal memories, entities, edges.

pub mod access;
pub mod db;
pub mod decay;
pub mod error;
pub mod events;
pub mod graph;
pub mod memory;
pub mod meta;

pub use error::{Error, Result};
pub use rusqlite::Connection;
