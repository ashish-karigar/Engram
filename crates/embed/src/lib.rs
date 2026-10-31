//! Text embeddings and vector search, stored in each project's SQLite file.

pub mod embedder;
pub mod fuse;
pub mod store;

pub use embedder::{Embedder, HashEmbedder, embedder_from_env};
