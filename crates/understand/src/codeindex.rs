use std::path::Path;

use engram_code::{Lang, Resolver, parse_file};
use engram_core::graph::{EdgeOpts, add_edge, upsert_entity};
use engram_core::{Connection, Result};

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

/// Turn source files (Python, Rust, TypeScript, JavaScript) into entities and edges:
/// `module -contains-> file -contains-> symbol` and `file -imports-> file`.
/// Re-running is safe: entities are upserted and edges are unique.
pub fn index_code(conn: &Connection, root: &Path, paths: &[String], resolver: &Resolver, commit_sha: Option<&str>) -> Result<IndexResult> {
    let mut result = IndexResult::default();
    for path in paths {
        let Some(lang) = Lang::from_path(path) else { continue };
        let Ok(source) = std::fs::read_to_string(root.join(path)) else { continue };
        let Some(parsed) = parse_file(lang, &source) else { continue };
        let file = upsert_entity(conn, "file", path, Some(path), commit_sha)?;
        let dir = module_of(path);
        let module = upsert_entity(conn, "module", &dir, Some(&dir), commit_sha)?;
        add_edge(conn, &module.id, &file.id, "contains", EdgeOpts::default())?;
        result.files += 1;

        // `impl` blocks are containers, not things anyone asks about; their methods are recorded.
        for s in parsed.symbols.iter().filter(|s| s.kind != "impl") {
            let symbol = upsert_entity(conn, s.kind, &format!("{path}::{}", s.name), Some(path), commit_sha)?;
            add_edge(conn, &file.id, &symbol.id, "contains", EdgeOpts::default())?;
            result.symbols += 1;
        }
        for import in &parsed.imports {
            let Some(target) = resolver.resolve(path, import) else { continue };
            let dst = upsert_entity(conn, "file", &target, Some(&target), commit_sha)?;
            if add_edge(conn, &file.id, &dst.id, "imports", EdgeOpts { weight: Some(0.8), ..Default::default() }).is_ok() {
                result.import_edges += 1;
            }
        }
    }
    Ok(result)
}
