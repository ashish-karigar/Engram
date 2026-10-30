use std::collections::HashSet;

use engram_core::graph::upsert_entity;
use engram_core::{Connection, Result};
use engram_core::rusqlite::OptionalExtension;

/// Resolve file paths mentioned by a memory to file entities. Unknown paths are ignored
/// unless the caller already saw them (`known`).
pub fn link_files(conn: &Connection, files: &[String], known: &HashSet<String>) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut ids = vec![];
    for file in files {
        let path = file.trim_start_matches("./");
        if !seen.insert(path.to_string()) {
            continue;
        }
        let existing: Option<String> = conn
            .query_row("SELECT id FROM entities WHERE kind = 'file' AND qualified_name = ?", [path], |r| r.get(0))
            .optional()?;
        match existing {
            Some(id) => ids.push(id),
            None if known.contains(path) => ids.push(upsert_entity(conn, "file", path, Some(path), None)?.id),
            None => {}
        }
    }
    Ok(ids)
}
