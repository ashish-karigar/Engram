//! Build a repository's system map in memory and print it: areas, endpoints with their handlers
//! and callers, external services, and calls nothing in the repository serves. Used to inspect the
//! map and to score it against a hand-checked list of endpoints.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::process::Command;

use engram_code::{Lang, NetLang, Resolver};
use engram_core::db::open_memory;
use engram_core::{Connection, Result};
use engram_understand::codeindex::index_code;
use engram_understand::system::rebuild_map;

pub fn build(root: &Path) -> Result<Connection> {
    let out = Command::new("git").arg("-C").arg(root).args(["ls-files", "--cached", "--others", "--exclude-standard"]).output()?;
    let all: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect();
    let known: HashSet<String> = all.iter().filter(|p| Lang::from_path(p).is_some() || NetLang::from_path(p).is_some()).cloned().collect();
    let manifests: Vec<(String, String)> = all
        .iter()
        .filter(|p| p.ends_with("Cargo.toml"))
        .filter_map(|p| Some((p.clone(), std::fs::read_to_string(root.join(p)).ok()?)))
        .collect();
    let conn = open_memory()?;
    let paths: Vec<String> = known.iter().cloned().collect();
    index_code(&conn, root, &paths, &Resolver::new(&known, &manifests), None)?;
    let repo = root.file_name().map_or("project".into(), |n| n.to_string_lossy().to_string());
    let stats = rebuild_map(&conn, &repo, &all, |f| std::fs::read_to_string(root.join(f)).ok())?;
    eprintln!("{}", serde_json::to_string(&stats)?);
    Ok(conn)
}

fn names(conn: &Connection, sql: &str, id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}

pub fn run(root: &Path) -> Result<()> {
    let conn = build(root)?;
    println!("AREAS");
    let mut stmt = conn.prepare("SELECT a.name, a.kind, a.path, (SELECT COUNT(*) FROM edges e JOIN entities x ON x.id = e.dst WHERE e.type = 'in_area' AND x.kind = 'area' AND x.path = a.path) FROM areas a ORDER BY a.path")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))? {
        let (name, kind, path, files) = row?;
        println!("  {name:<20} {kind:<20} {:<30} {files} files", if path.is_empty() { "." } else { &path });
    }
    println!("ENDPOINTS (handler <- callers)");
    let mut stmt = conn.prepare("SELECT id, qualified_name FROM entities WHERE kind = 'endpoint' ORDER BY qualified_name")?;
    let endpoints: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut unserved = vec![];
    for (id, name) in &endpoints {
        let handlers = names(&conn, "SELECT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type = 'serves'", id)?;
        let callers = names(&conn, "SELECT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type = 'calls'", id)?;
        if handlers.is_empty() {
            unserved.push(format!("  {name}  <- {}", callers.join(", ")));
            continue;
        }
        println!("  {name:<42} {}  <- {}", handlers.join(", "), if callers.is_empty() { "(no caller found)".into() } else { callers.join(", ") });
    }
    println!("SERVICES (used by)");
    let mut stmt = conn.prepare("SELECT id, qualified_name FROM entities WHERE kind = 'service' ORDER BY qualified_name")?;
    let services: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    for (id, name) in &services {
        let users = names(&conn, "SELECT DISTINCT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type IN ('calls','uses')", id)?;
        let mut by_file: BTreeMap<String, ()> = BTreeMap::new();
        for u in users {
            by_file.insert(u.split("::").next().unwrap_or(&u).to_string(), ());
        }
        println!("  {name:<22} {}", by_file.keys().cloned().collect::<Vec<_>>().join(", "));
        let docs = names(&conn, "SELECT DISTINCT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type = 'mentions'", id)?;
        if !docs.is_empty() {
            println!("  {:<22} named in {}", "", docs.join(", "));
        }
    }
    println!("SERVICE EVIDENCE (config and docs)");
    let mut stmt = conn.prepare("SELECT service, file, via FROM net_services WHERE via LIKE 'config:%' OR via LIKE 'docs:%' ORDER BY service, file")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
        let (service, file, via) = row?;
        let (how, rest) = via.split_once(':').unwrap_or((&via, ""));
        let (line, why) = rest.split_once(':').unwrap_or(("", rest));
        println!("  {service:<14} {how:<6} {file}:{line}  {why}");
    }
    println!("CALLS NOTHING HERE SERVES");
    for u in unserved {
        println!("{u}");
    }
    Ok(())
}
