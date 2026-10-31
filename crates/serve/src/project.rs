use std::path::Path;

use engram_core::{Error, Result};
use std::sync::Arc;

use engram_core::db::SharedDb;
use engram_embed::Embedder;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub root: String,
}

#[derive(Clone)]
pub struct ProjectRef {
    pub info: ProjectInfo,
    pub db: SharedDb,
    /// On-disk database. When set, each request opens its own connection (SQLite in WAL mode
    /// serves concurrent readers), so one slow request never blocks the others.
    pub db_path: Option<std::path::PathBuf>,
    /// The project's embedding model, shared by search and document ingest.
    pub embedder: Arc<dyn Embedder>,
}

#[derive(Debug, Serialize)]
pub struct ConnectOutcome {
    pub agent: String,
    pub preview: String,
    pub result: Option<String>,
}

/// What the server needs from whoever owns the running projects (the daemon).
pub trait ProjectManager: Send + Sync {
    fn list(&self) -> Vec<ProjectRef>;
    fn get(&self, id: &str) -> Option<ProjectRef>;
    fn add(&self, root: &Path) -> Result<ProjectRef>;
    fn remove(&self, id: &str) -> Result<()>;
    fn connect(&self, id: &str, agent: &str, apply: bool) -> Result<ConnectOutcome>;

    /// App settings as the UI may see them (secrets masked).
    fn settings(&self) -> Result<serde_json::Value> {
        Err(Error::msg("settings are not available"))
    }
    /// Save settings and apply them. Returns the new public view.
    fn update_settings(&self, _incoming: serde_json::Value) -> Result<serde_json::Value> {
        Err(Error::msg("settings are not available"))
    }
    /// Try the configured model (`llm`) or embedder (`embeddings`) with a tiny request.
    fn test_settings(&self, _target: &str) -> Result<serde_json::Value> {
        Err(Error::msg("settings are not available"))
    }
}
