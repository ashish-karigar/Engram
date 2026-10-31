//! Resolve call sites (`code_calls`) to the functions they reach: `invokes` edges between
//! functions, the finest level of the dependency layer (docs/DATA_MODEL.md, layer 2).
//!
//! Resolution is by name, narrowed by where the caller can see: its own class, its own file, the
//! files it imports, then a qualified name (`Store::open`, `JobsApi.updateStatus`) anywhere. A unique
//! match is certain (weight 1.0); two or three are kept at half weight; more is too ambiguous to keep.
//! Calls into libraries (`http.get`, `json.dumps`) find nothing in the project and are skipped.

use std::collections::{HashMap, HashSet};

use engram_core::graph::{EdgeOpts, add_edge};
use engram_core::{Connection, Result};
use serde::Serialize;

#[derive(Debug, Default, Serialize)]
pub struct CallStats {
    pub sites: usize,
    pub resolved: usize,
    pub ambiguous: usize,
    pub outside_project: usize,
}

struct Sym {
    id: String,
    file: String,
    /// Within the file: `JobsApi.updateStatus`, `Store::open`, `helper`.
    name: String,
}

/// Code can only call code in its own language family (a template calls its TypeScript component).
fn family(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("") {
        "py" => "python",
        "rs" => "rust",
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "html" => "js",
        "swift" => "swift",
        _ => "other",
    }
}

fn simple(name: &str) -> &str {
    name.rsplit(['.', ':']).next().unwrap_or(name)
}

/// `JobsApi.updateStatus` → `JobsApi`; `Store::open` → `Store`.
fn owner(name: &str) -> Option<&str> {
    name.rsplit_once(['.', ':']).map(|(o, _)| o.trim_end_matches(':'))
}

pub fn link_calls(conn: &Connection) -> Result<CallStats> {
    conn.execute("DELETE FROM edges WHERE type = 'invokes'", [])?;
    let mut stmt = conn.prepare("SELECT entity_id, file, name FROM code_symbols WHERE kind IN ('function','method')")?;
    let symbols: Vec<Sym> = stmt
        .query_map([], |r| Ok(Sym { id: r.get(0)?, file: r.get(1)?, name: r.get(2)? }))?
        .collect::<engram_core::rusqlite::Result<_>>()?;
    let mut by_simple: HashMap<&str, Vec<&Sym>> = HashMap::new();
    for s in &symbols {
        by_simple.entry(simple(&s.name)).or_default().push(s);
    }
    let by_qualified: HashMap<(&str, &str), &Sym> = symbols.iter().map(|s| ((s.file.as_str(), s.name.as_str()), s)).collect();

    let mut stmt = conn.prepare(
        "SELECT s.qualified_name, d.qualified_name FROM edges e JOIN entities s ON s.id = e.src JOIN entities d ON d.id = e.dst
         WHERE e.type = 'imports' AND e.valid_to IS NULL",
    )?;
    let mut imports: HashMap<String, HashSet<String>> = HashMap::new();
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (from, to) = row?;
        imports.entry(from).or_default().insert(to);
    }
    let file_ids: HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT qualified_name, id FROM entities WHERE kind = 'file' AND status = 'active'")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?
    };

    let mut stmt = conn.prepare("SELECT file, caller, callee, receiver FROM code_calls")?;
    let sites: Vec<(String, Option<String>, String, Option<String>)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut stats = CallStats { sites: sites.len(), ..Default::default() };
    let mut linked: HashSet<(String, String)> = HashSet::new();

    for (file, caller, callee, receiver) in &sites {
        let Some(all_named) = by_simple.get(callee.as_str()) else {
            stats.outside_project += 1;
            continue;
        };
        let named: Vec<&Sym> = all_named.iter().copied().filter(|s| family(&s.file) == family(file)).collect();
        if named.is_empty() {
            stats.outside_project += 1;
            continue;
        }
        let named = &named;
        let caller_sym = caller.as_deref().and_then(|c| by_qualified.get(&(file.as_str(), c)));
        let caller_id = caller_sym.map(|s| s.id.clone()).or_else(|| file_ids.get(file).cloned());
        let Some(caller_id) = caller_id else { continue };
        let on_self = receiver.as_deref().is_none_or(|r| ["self", "this", "Self", "cls", "super"].contains(&r));
        let caller_class = caller.as_deref().and_then(owner);
        let receiver_type = receiver.as_deref().map(|r| simple(r.trim_end_matches("()")));

        // `relevance::vocabulary`, `llm.extract()`: the receiver names the module (file) it lives in.
        let module_of = |s: &&Sym| s.file.rsplit('/').next().and_then(|f| f.split('.').next()).map(str::to_string);
        let tiers: [Box<dyn Fn(&&Sym) -> bool>; 7] = [
            // 0. A component's template calling the component (receiver `component:<file>`).
            Box::new(|s: &&Sym| receiver.as_deref().and_then(|r| r.strip_prefix("component:")) == Some(s.file.as_str())),
            // 1. A method of the caller's own class.
            Box::new(|s: &&Sym| on_self && s.file == *file && caller_class.is_some() && owner(&s.name) == caller_class),
            // 2. The same file.
            Box::new(|s: &&Sym| s.file == *file && (on_self || receiver_type.is_some_and(|t| owner(&s.name) == Some(t)) || receiver.is_none())),
            // 3. A file the caller imports.
            Box::new(|s: &&Sym| imports.get(file).is_some_and(|i| i.contains(&s.file))),
            // 4. Qualified by its type anywhere: `Store::open`, `JobsApi.load`.
            Box::new(|s: &&Sym| receiver_type.is_some_and(|t| owner(&s.name) == Some(t))),
            // 5. Qualified by its module: `relevance::vocabulary`, `llm.extract()`.
            Box::new(|s: &&Sym| owner(&s.name).is_none() && receiver_type.is_some_and(|t| module_of(s).as_deref() == Some(t))),
            // 6. A distinctive name defined exactly once in the project (`use` inside a function,
            //    re-exports). Short or generic names (`run`, `new`, `get`) never get here.
            Box::new(|s: &&Sym| named.len() == 1 && callee.len() >= 8 && simple(&s.name) == callee),
        ];
        let Some(found) = tiers.iter().map(|keep| named.iter().filter(|s| keep(s)).collect::<Vec<_>>()).find(|c| !c.is_empty()) else {
            stats.outside_project += 1;
            continue;
        };
        let guessed = !tiers[..6].iter().any(|keep| named.iter().any(|s| keep(s)));
        let weight = match found.len() {
            1 if guessed => 0.5,
            1 => 1.0,
            2 | 3 => 0.5,
            _ => {
                stats.ambiguous += 1;
                continue;
            }
        };
        if found.len() > 1 {
            stats.ambiguous += 1;
        } else {
            stats.resolved += 1;
        }
        for target in found {
            if target.id != caller_id && linked.insert((caller_id.clone(), target.id.clone())) {
                add_edge(conn, &caller_id, &target.id, "invokes", EdgeOpts { weight: Some(weight), ..Default::default() })?;
            }
        }
    }
    Ok(stats)
}
