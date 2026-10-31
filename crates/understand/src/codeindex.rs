use std::path::Path;

use engram_code::net::{self, NetLang};
use engram_code::{Lang, Resolver, parse_file};
use engram_core::graph::{EdgeOpts, add_edge, upsert_entity};
use engram_core::{Connection, Result};

use crate::system::store_file;

/// `templateUrl: './applications.html'` in an Angular component → the template's path.
fn angular_template(path: &str, source: &str) -> Option<String> {
    let at = source.find("templateUrl")?;
    let rest = &source[at..];
    let start = rest.find(['\'', '"', '`'])? + 1;
    let end = start + rest[start..].find(['\'', '"', '`'])?;
    let rel = rest[start..end].trim_start_matches("./");
    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    Some(if dir.is_empty() { rel.to_string() } else { format!("{dir}/{rel}") })
}

/// Methods a template calls from its bindings: `(event)="m(…)"`, `[prop]="m()"`, `{{ m() }}`.
fn template_calls(html: &str) -> Vec<(String, usize)> {
    use std::sync::LazyLock;
    static BINDING: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r#"(?:[\(\[][\w.\-]+[\)\]]\s*=\s*"([^"]*)"|\{\{([^}]*)\}\})"#).unwrap());
    static CALL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?:^|[^\w.])([a-zA-Z_]\w*)\s*\(").unwrap());
    let mut out = vec![];
    for b in BINDING.captures_iter(html) {
        let expr = b.get(1).or(b.get(2)).unwrap();
        let line = html[..expr.start()].matches('\n').count() + 1;
        for c in CALL.captures_iter(expr.as_str()) {
            out.push((c[1].to_string(), line));
        }
    }
    out
}

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
        conn.execute("DELETE FROM code_symbols WHERE file = ?", [path])?;
        conn.execute("DELETE FROM code_calls WHERE file = ?", [path])?;
        for s in parsed.symbols.iter().filter(|s| s.kind != "impl") {
            let symbol = upsert_entity(conn, s.kind, &format!("{path}::{}", s.name), Some(path), commit_sha)?;
            // Methods hang off their class (`JobsApi.updateStatus`, `Store::open`), others off the file.
            let parent = s.name.rsplit_once(['.', ':']).map(|(p, _)| p.trim_end_matches(':').to_string());
            let parent_id = match &parent {
                Some(p) => upsert_entity(conn, "class", &format!("{path}::{p}"), Some(path), commit_sha).ok().map(|e| e.id),
                None => None,
            };
            add_edge(conn, parent_id.as_deref().unwrap_or(&file.id), &symbol.id, "contains", EdgeOpts::default())?;
            conn.execute(
                "INSERT OR REPLACE INTO code_symbols (entity_id, file, name, kind, start_line, end_line, parent, signature) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                engram_core::rusqlite::params![symbol.id, path, s.name, s.kind, s.start_line as i64, s.end_line as i64, parent, s.signature],
            )?;
            result.symbols += 1;
        }
        // Angular: a component's template calls its methods (`(click)="changeStatus(job)"`).
        if let Some(template) = angular_template(path, &source) {
            if let Ok(html) = std::fs::read_to_string(root.join(&template)) {
                upsert_entity(conn, "file", &template, Some(&template), commit_sha)?;
                conn.execute("DELETE FROM code_calls WHERE file = ?", [&template])?;
                for (callee, line) in template_calls(&html) {
                    conn.execute(
                        "INSERT INTO code_calls (file, caller, callee, receiver, line) VALUES (?, NULL, ?, ?, ?)",
                        engram_core::rusqlite::params![template, callee, format!("component:{path}"), line as i64],
                    )?;
                }
            }
        }
        // Each call is recorded with the innermost function around it (None at module level).
        for c in &parsed.calls {
            let caller = parsed
                .symbols
                .iter()
                .filter(|s| matches!(s.kind, "function" | "method") && s.start_line <= c.line && c.line <= s.end_line)
                .min_by_key(|s| s.end_line - s.start_line)
                .map(|s| s.name.clone());
            conn.execute(
                "INSERT INTO code_calls (file, caller, callee, receiver, line) VALUES (?, ?, ?, ?, ?)",
                engram_core::rusqlite::params![path, caller, c.callee, c.receiver, c.line as i64],
            )?;
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
