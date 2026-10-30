use std::collections::HashSet;
use std::path::Path;

use engram_core::graph::{EdgeOpts, add_edge, upsert_entity};
use engram_core::{Connection, Result};

use crate::pyindex::{parse_python, resolve_import};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct IndexResult {
    pub files: usize,
    pub symbols: usize,
    pub import_edges: usize,
}

fn module_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => "(root)".to_string(),
    }
}

/// Turn Python files into entities and edges:
/// `module -contains-> file -contains-> symbol` and `file -imports-> file`.
/// Re-running is safe: entities are upserted and edges are unique.
pub fn index_python(
    conn: &Connection,
    root: &Path,
    paths: &[String],
    all_files: &HashSet<String>,
    commit_sha: Option<&str>,
) -> Result<IndexResult> {
    let mut result = IndexResult::default();
    for path in paths.iter().filter(|p| p.ends_with(".py")) {
        let Ok(source) = std::fs::read_to_string(root.join(path)) else { continue };
        let parsed = parse_python(&source)?;
        let file = upsert_entity(conn, "file", path, Some(path), commit_sha)?;
        let dir = module_of(path);
        let module = upsert_entity(conn, "module", &dir, Some(&dir), commit_sha)?;
        add_edge(conn, &module.id, &file.id, "contains", EdgeOpts::default())?;
        result.files += 1;

        for s in &parsed.symbols {
            let symbol = upsert_entity(conn, s.kind, &format!("{path}::{}", s.name), Some(path), commit_sha)?;
            add_edge(conn, &file.id, &symbol.id, "contains", EdgeOpts::default())?;
            result.symbols += 1;
        }
        for imp in &parsed.imports {
            let Some(target) = resolve_import(imp, path, all_files).filter(|t| t != path) else { continue };
            let dst = upsert_entity(conn, "file", &target, Some(&target), commit_sha)?;
            add_edge(conn, &file.id, &dst.id, "imports", EdgeOpts { weight: Some(0.8), ..Default::default() })?;
            result.import_edges += 1;
        }
    }
    Ok(result)
}
