use std::path::Path;

use engram_code::net::{self, NetLang};
use engram_code::{Lang, Resolver, parse_file};
use engram_core::graph::{EdgeOpts, add_edge, upsert_entity};
use engram_core::{Connection, Result};

use crate::system::store_file;

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

/// Connect a folder to its parent folders (`backend/app/routers` ⊂ `backend/app` ⊂ `backend`),
/// so the project's architecture shows as one tree instead of islands.
fn link_folder_tree(conn: &Connection, dir: &str, id: &str, commit_sha: Option<&str>) -> Result<()> {
    let (mut child_dir, mut child_id) = (dir.to_string(), id.to_string());
    while let Some((parent, _)) = child_dir.rsplit_once('/') {
        let parent_entity = upsert_entity(conn, "module", parent, Some(parent), commit_sha)?;
        let fresh = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE src = ? AND dst = ? AND type = 'contains'",
            [&parent_entity.id, &child_id],
            |r| r.get::<_, i64>(0),
        )? == 0;
        add_edge(conn, &parent_entity.id, &child_id, "contains", EdgeOpts::default())?;
        if !fresh {
            break;
        }
        (child_dir, child_id) = (parent.to_string(), parent_entity.id);
    }
    Ok(())
}

/// Turn source files (Python, Rust, TypeScript, JavaScript) into entities and edges:
/// `module -contains-> file -contains-> symbol` and `file -imports-> file`.
/// Re-running is safe: entities are upserted and edges are unique.
pub fn index_code(conn: &Connection, root: &Path, paths: &[String], resolver: &Resolver, commit_sha: Option<&str>) -> Result<IndexResult> {
    let mut result = IndexResult::default();
    for path in paths {
        let net_lang = NetLang::from_path(path);
        let Ok(source) = std::fs::read_to_string(root.join(path)) else { continue };
        let Some(lang) = Lang::from_path(path) else {
            // Languages without a parser here (Swift) still contribute routes and calls.
            if let Some(nl) = net_lang {
                upsert_entity(conn, "file", path, Some(path), commit_sha)?;
                store_file(conn, path, &net::extract(nl, path, &source), &[], &[])?;
            }
            continue;
        };
        let Some(parsed) = parse_file(lang, &source) else { continue };
        if let Some(nl) = net_lang {
            store_file(conn, path, &net::extract(nl, path, &source), &parsed.symbols, &parsed.imports)?;
        }
        let file = upsert_entity(conn, "file", path, Some(path), commit_sha)?;
        let dir = module_of(path);
        let module = upsert_entity(conn, "module", &dir, Some(&dir), commit_sha)?;
        add_edge(conn, &module.id, &file.id, "contains", EdgeOpts::default())?;
        link_folder_tree(conn, &dir, &module.id, commit_sha)?;
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
