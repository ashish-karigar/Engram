use std::path::Path;

use engram_core::Result;
use engram_core::db::SharedDb;
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
}
