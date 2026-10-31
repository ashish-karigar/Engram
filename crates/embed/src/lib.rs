//! Text embeddings and vector search, stored in each project's SQLite file.

pub mod ann;
pub mod embedder;
pub mod fuse;
pub mod local;
pub mod rerank;
pub mod store;

pub use embedder::{Embedder, HashEmbedder, embedder_from_env};
pub use local::{LocalEmbedder, LocalModel};
pub use rerank::{LocalReranker, RerankModel, Reranker};
