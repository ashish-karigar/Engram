use std::path::Path;

use engram_core::{Connection, Result};
use engram_embed::Embedder;
use serde_json::json;

use crate::document::{delete_document, get_by_external_id};
use crate::extract::is_supported;
use crate::ingest::{DocumentInput, IngestOutcome, ingest};

pub const MAX_FILE_BYTES: u64 = 512 * 1024;

const SKIP_DIRS: &[&str] = &["node_modules", "target", ".git", "dist", "build", "__pycache__", ".venv", "venv", "vendor"];
const SKIP_FILES: &[&str] = &["package-lock.json", "Cargo.lock", "yarn.lock", "pnpm-lock.yaml", "poetry.lock"];

/// Project files worth indexing as documents: docs, configs and source, minus generated noise.
pub fn should_index(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    is_supported(rel)
        && !rel.split('/').any(|p| SKIP_DIRS.contains(&p))
        && !SKIP_FILES.contains(&name)
        && !name.ends_with(".min.js")
}

fn external_id(rel: &str) -> String {
    format!("file:{rel}")
}

/// Ingest (or refresh) one project file. Returns None for files that are skipped.
pub fn ingest_file(conn: &Connection, embedder: &dyn Embedder, root: &Path, rel: &str, max_bytes: u64) -> Result<Option<IngestOutcome>> {
    let full = root.join(rel);
    let Ok(meta) = std::fs::metadata(&full) else {
        remove_file(conn, rel)?;
        return Ok(None);
    };
    if !should_index(rel) || !meta.is_file() || meta.len() > max_bytes {
        return Ok(None);
    }
    let input = DocumentInput {
        external_id: Some(external_id(rel)),
        title: rel.to_string(),
        source: "project".into(),
        path: Some(rel.to_string()),
        file_name: rel.to_string(),
        bytes: std::fs::read(&full)?,
        metadata: json!({"bytes": meta.len()}),
    };
    ingest(conn, embedder, input).map(Some)
}

pub fn remove_file(conn: &Connection, rel: &str) -> Result<()> {
    if let Some(doc) = get_by_external_id(conn, &external_id(rel))? {
        delete_document(conn, &doc.id)?;
    }
    Ok(())
}
