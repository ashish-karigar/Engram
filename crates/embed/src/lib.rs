//! Text embeddings and vector search, stored in each project's SQLite file.

pub mod embedder;
pub mod fuse;
pub mod local;
pub mod store;

pub use embedder::{Embedder, HashEmbedder, embedder_from_env};
pub use local::{LocalEmbedder, LocalModel};
