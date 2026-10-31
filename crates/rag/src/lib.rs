//! Documents as context ("SuperRAG"): extract, chunk, embed, index, and search them.

pub mod chunk;
pub mod document;
pub mod extract;
pub mod ingest;
pub mod project;
pub mod search;

pub use document::Document;
pub use ingest::{DocumentInput, IngestAction, ingest};
pub use search::{ChunkHit, search_chunks};
