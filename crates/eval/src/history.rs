//! Impact against history: when a commit changed file F together with other files, did Engram's
//! impact for F point at them? Co-change is a noisy truth (commits mix unrelated edits), so the
//! number to read is recall, and above all recall on companion changes in another part of the system
//! (a frontend file changed with a backend route), compared with an imports-only baseline (what the
//! old graph could see).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::Command;

use engram_core::{Connection, Result};
use engram_serve::system::{Node, impact};

use crate::map::build;

/// Files that import `file`, transitively (the old graph's notion of impact).
fn imports_only(conn: &Connection, file: &str, depth: usize) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT s.qualified_name FROM edges e JOIN entities s ON s.id = e.src JOIN entities d ON d.id = e.dst
         WHERE e.type = 'imports' AND d.kind = 'file' AND d.qualified_name = ?",
    )?;
    let (mut seen, mut queue) = (BTreeSet::new(), VecDeque::from([(file.to_string(), 0)]));
    while let Some((f, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        for importer in stmt.query_map([&f], |r| r.get::<_, String>(0))? {
            let importer = importer?;
            if importer != file && seen.insert(importer.clone()) {
                queue.push_back((importer, d + 1));
            }
        }
    }
    Ok(seen)
}

fn area_of(conn: &Connection, file: &str) -> Option<String> {
    conn.query_row(
        "SELECT x.path FROM entities f JOIN edges e ON e.src = f.id AND e.type = 'in_area' JOIN entities x ON x.id = e.dst
         WHERE f.kind = 'file' AND f.qualified_name = ?",
        [file],
        |r| r.get(0),
    )
    .ok()
}

#[derive(Default)]
struct Tally {
    pairs: usize,
    hit_engram: usize,
    hit_imports: usize,
    cross_pairs: usize,
    cross_engram: usize,
    cross_imports: usize,
    predicted: usize,
    predicted_hits: usize,
    /// A file serving endpoints changed together with a file in another part that calls endpoints.
    contract_pairs: usize,
    contract_engram: usize,
    contract_imports: usize,
}

/// Whether a file (or a function in it) has edges of `kind` going out: serves or calls endpoints.
fn has_edge(conn: &Connection, file: &str, kind: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM edges e JOIN entities s ON s.id = e.src JOIN entities d ON d.id = e.dst
          WHERE e.type = ?1 AND d.kind = 'endpoint' AND (s.qualified_name = ?2 OR s.qualified_name LIKE ?2 || '::%'))",
        [kind, file],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

pub fn run(root: &Path, commits: usize, depth: usize) -> Result<()> {
    let conn = build(root)?;
    let indexed: HashSet<String> = {
        let mut stmt = conn.prepare("SELECT qualified_name FROM entities WHERE kind = 'file' AND status = 'active'")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<engram_core::rusqlite::Result<_>>()?
    };
    let log = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["log", "--no-merges", "--name-only", "--format=@@%h", "-n", &commits.to_string()])
        .output()?;
    let mut changes: Vec<Vec<String>> = vec![];
    for line in String::from_utf8_lossy(&log.stdout).lines() {
        if line.starts_with("@@") {
            changes.push(vec![]);
        } else if !line.is_empty() && indexed.contains(line) {
            if let Some(c) = changes.last_mut() {
                c.push(line.to_string());
            }
        }
    }
    // Very large commits (initial imports, mass renames) say nothing about dependencies.
    let changes: Vec<Vec<String>> = changes.into_iter().filter(|c| c.len() >= 2 && c.len() <= 25).collect();
    let mut t = Tally::default();
    let mut cache: HashMap<String, (BTreeSet<String>, BTreeSet<String>)> = HashMap::new();
    for files in &changes {
        for f in files {
            let (engram, imports) = match cache.get(f) {
                Some(v) => v.clone(),
                None => {
                    let id: String = conn.query_row("SELECT id FROM entities WHERE kind = 'file' AND qualified_name = ?", [f], |r| r.get(0))?;
                    let node = Node { id, kind: "file".into(), name: f.clone(), path: Some(f.clone()) };
                    let predicted: BTreeSet<String> = impact(&conn, &node, depth)?.affected.into_iter().map(|a| a.file).collect();
                    let v = (predicted, imports_only(&conn, f, depth)?);
                    cache.insert(f.clone(), v.clone());
                    v
                }
            };
            t.predicted += engram.len();
            let home = area_of(&conn, f);
            for other in files.iter().filter(|o| *o != f) {
                t.pairs += 1;
                let (e, i) = (engram.contains(other), imports.contains(other));
                t.hit_engram += e as usize;
                t.hit_imports += i as usize;
                t.predicted_hits += e as usize;
                if area_of(&conn, other) != home {
                    t.cross_pairs += 1;
                    t.cross_engram += e as usize;
                    t.cross_imports += i as usize;
                    if has_edge(&conn, f, "serves") && has_edge(&conn, other, "calls") {
                        t.contract_pairs += 1;
                        t.contract_engram += e as usize;
                        t.contract_imports += i as usize;
                    }
                }
            }
        }
    }
    let pct = |a: usize, b: usize| if b == 0 { "n/a".to_string() } else { format!("{:.1}%", 100.0 * a as f64 / b as f64) };
    println!("repo {} · {} commits with 2–25 indexed files · impact depth {depth}", root.display(), changes.len());
    println!("companion files found (recall)      engram {:>7}   imports-only {:>7}   ({} pairs)", pct(t.hit_engram, t.pairs), pct(t.hit_imports, t.pairs), t.pairs);
    println!("  …in another part of the system    engram {:>7}   imports-only {:>7}   ({} pairs)", pct(t.cross_engram, t.cross_pairs), pct(t.cross_imports, t.cross_pairs), t.cross_pairs);
    println!("  …server file → caller in other part engram {:>7}   imports-only {:>7}   ({} pairs)", pct(t.contract_engram, t.contract_pairs), pct(t.contract_imports, t.contract_pairs), t.contract_pairs);
    println!("predicted files that did change     engram {:>7}   ({} predicted)", pct(t.predicted_hits, t.predicted), t.predicted);
    Ok(())
}
