//! Reading the system map: a compact overview for agents (`map`), and the impact of changing a
//! file, function, endpoint or service (`impact`): everything that depends on it, across language
//! and process boundaries, grouped by area, each with the chain that explains why.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use engram_core::rusqlite::{OptionalExtension, params};
use engram_core::{Connection, Error, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub path: Option<String>,
}

fn node(conn: &Connection, id: &str) -> Result<Option<Node>> {
    Ok(conn
        .query_row("SELECT id, kind, qualified_name, path FROM entities WHERE id = ?", [id], |r| {
            Ok(Node { id: r.get(0)?, kind: r.get(1)?, name: r.get(2)?, path: r.get(3)? })
        })
        .optional()?)
}

fn file_of(n: &Node) -> String {
    n.path.clone().unwrap_or_else(|| n.name.split("::").next().unwrap_or(&n.name).to_string())
}

/// The area (name, kind) a file belongs to.
fn area_of_file(conn: &Connection, file: &str) -> Result<Option<(String, String)>> {
    Ok(conn
        .query_row(
            "SELECT a.name, a.kind FROM entities f JOIN edges e ON e.src = f.id AND e.type = 'in_area'
             JOIN entities x ON x.id = e.dst JOIN areas a ON a.path = x.path
             WHERE f.kind = 'file' AND f.qualified_name = ? LIMIT 1",
            [file],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// Find what the caller means: an endpoint (`PATCH /jobs/{id}`), a file path (or a unique file
/// name), a function (`file::name` or a unique name), or a service.
pub fn resolve_target(conn: &Connection, target: &str) -> Result<Node> {
    let target = target.trim();
    let one = |sql: &str, arg: &str| -> Result<Vec<String>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([arg], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
    };
    let mut candidates: Vec<String> = vec![];
    if let Some((method, path)) = target.split_once(' ').filter(|(m, _)| m.chars().all(|c| c.is_ascii_uppercase())) {
        let wanted = format!("{} {}", method, engram_code::net::normalize(path));
        candidates = one("SELECT id FROM entities WHERE kind = 'endpoint' AND qualified_name = ?", &wanted)?;
    }
    if candidates.is_empty() && target.starts_with('/') {
        let path = engram_code::net::normalize(target);
        candidates = one("SELECT id FROM entities WHERE kind = 'endpoint' AND substr(qualified_name, instr(qualified_name, ' ') + 1) = ?", &path)?;
    }
    for sql in [
        "SELECT id FROM entities WHERE status = 'active' AND qualified_name = ?",
        "SELECT id FROM entities WHERE kind = 'file' AND status = 'active' AND (qualified_name LIKE '%/' || ?)",
        "SELECT id FROM entities WHERE status = 'active' AND kind NOT IN ('file','module','area') AND qualified_name LIKE '%::' || ?",
        "SELECT id FROM entities WHERE kind = 'service' AND lower(qualified_name) = lower(?)",
    ] {
        if !candidates.is_empty() {
            break;
        }
        candidates = one(sql, target)?;
    }
    match candidates.as_slice() {
        [id] => node(conn, id)?.ok_or_else(|| Error::msg("target vanished")),
        [] => Err(Error::msg(format!("Nothing in the system map matches `{target}`. Give a file path, `file::function`, an endpoint like `GET /jobs/{{id}}`, or a service name; for a concept like `auth`, use `feature`."))),
        many => {
            let names: Vec<String> = many.iter().take(8).filter_map(|id| node(conn, id).ok().flatten()).map(|n| n.name).collect();
            Err(Error::msg(format!("`{target}` matches several things; be more specific: {}", names.join(", "))))
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Affected {
    pub file: String,
    pub area: Option<String>,
    pub depth: usize,
    /// How the change reaches this file, e.g. "calls PATCH /jobs/{} (served by jobs.py::update_job_status)".
    pub why: String,
}

/// A part of the system that relies on a service without calling it from code, e.g. "frontend
/// is hosted on Cloudflare (per docs)".
#[derive(Debug, Serialize)]
pub struct PartUse {
    pub part: String,
    /// "frontend (Angular app) is hosted on Cloudflare"
    pub relation: String,
    /// code, config, compose or docs: the strongest evidence for it.
    pub how: String,
}

/// A doc or config line that names a service, to update if the service changes.
#[derive(Debug, Serialize)]
pub struct Mention {
    pub file: String,
    pub line: Option<i64>,
    pub why: String,
}

#[derive(Debug, Serialize)]
pub struct Impact {
    pub target: Node,
    /// Who wrote the target and when (provenance), if known.
    pub written_by: Option<String>,
    pub affected: Vec<Affected>,
    pub endpoints: Vec<String>,
    pub decisions: Vec<(String, String, String)>,
    /// For a service: the parts that rely on it (hosting, data, auth, ...), from the architecture.
    pub parts: Vec<PartUse>,
    /// For a service: docs and config that name it.
    #[serde(rename = "namedIn")]
    pub named_in: Vec<Mention>,
}

/// What a service means to the rest of the system beyond code that calls it: the parts that rely
/// on it, the docs and config that name it, and memories that mention it.
fn service_reach(conn: &Connection, service: &str, skip: &BTreeMap<String, Affected>) -> Result<(Vec<PartUse>, Vec<Mention>, Vec<(String, String, String)>)> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(a.name, l.area), a.kind, l.role, l.how FROM system_links l LEFT JOIN areas a ON a.path = l.area
          WHERE lower(l.service) = lower(?) ORDER BY l.area, l.role",
    )?;
    let parts = stmt
        .query_map([service], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?
        .collect::<engram_core::rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(name, kind, role, how)| {
            let part = kind.map_or(name.clone(), |k| format!("{name} ({k})"));
            let relation = format!("{part} {} {service}", engram_understand::architecture::phrase(&role));
            PartUse { part: name, relation, how }
        })
        .collect();

    let mut stmt = conn.prepare(
        "SELECT file, via FROM net_services WHERE lower(service) = lower(?) AND (via LIKE 'docs:%' OR via LIKE 'config:%') ORDER BY file, via",
    )?;
    let named_in = stmt
        .query_map([service], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<engram_core::rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(file, _)| !skip.contains_key(file))
        .map(|(file, via)| {
            // via: "docs:50:mentions \"Cloudflare\"" or "config:12:image postgres"
            let mut bits = via.splitn(3, ':').skip(1);
            let line = bits.next().and_then(|l| l.parse().ok());
            Mention { file, line, why: bits.next().unwrap_or("").to_string() }
        })
        .collect();

    let mut stmt = conn.prepare(
        "SELECT type, title, claim FROM memories WHERE status = 'current' AND COALESCE(scope, '') NOT LIKE 'system:%'
           AND instr(lower(title || ' ' || claim), lower(?)) > 0 ORDER BY salience DESC LIMIT 6",
    )?;
    let memories = stmt.query_map([service], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    Ok((parts, named_in, memories))
}

/// Everything that depends on `target`, up to `depth` steps away, closest first.
pub fn impact(conn: &Connection, target: &Node, depth: usize) -> Result<Impact> {
    // Reverse dependencies of one node: (dependent node id, explanation), at function level where
    // the code allows: callers of a function, callers of the endpoints it serves, users of a service.
    let edges_into = |id: &str, types: &str| -> Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT e.src, x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type IN ({types}) AND e.valid_to IS NULL"
        ))?;
        let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
    };
    let served_by = |id: &str| -> Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare("SELECT e.dst, d.qualified_name FROM edges e JOIN entities d ON d.id = e.dst WHERE e.src = ? AND e.type = 'serves'")?;
        let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
    };
    let functions_in = |n: &Node| -> Result<Vec<(String, String)>> {
        // A file: its functions; a class: its methods.
        let (file, prefix) = match n.kind.as_str() {
            "file" => (n.name.clone(), String::new()),
            _ => (file_of(n), format!("{}.", n.name.split("::").nth(1).unwrap_or(""))),
        };
        let mut stmt = conn.prepare("SELECT entity_id, name FROM code_symbols WHERE file = ? AND kind IN ('function','method')")?;
        let rows: Vec<(String, String)> = stmt.query_map([&file], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
        Ok(rows.into_iter().filter(|(_, name)| prefix.is_empty() || name.starts_with(&prefix) || name.starts_with(&prefix.replace('.', "::"))).collect())
    };
    let of_function = |id: &str, name: &str, out: &mut Vec<(String, String)>| -> Result<()> {
        for (caller, _) in edges_into(id, "'invokes'")? {
            out.push((caller, format!("calls {name}")));
        }
        for (endpoint, endpoint_name) in served_by(id)? {
            out.push((endpoint, format!("{endpoint_name} is served by {name}")));
        }
        Ok(())
    };
    let dependents = |n: &Node| -> Result<Vec<(String, String)>> {
        let mut out = vec![];
        match n.kind.as_str() {
            "endpoint" => out.extend(edges_into(&n.id, "'calls'")?.into_iter().map(|(id, _)| (id, format!("calls {}", n.name)))),
            "service" => out.extend(edges_into(&n.id, "'calls','uses'")?.into_iter().map(|(id, _)| (id, format!("uses {}", n.name)))),
            "function" | "method" => of_function(&n.id, &n.name, &mut out)?,
            "file" | "class" | "struct" | "trait" | "interface" | "enum" => {
                let mut callers_found: HashSet<String> = HashSet::new();
                for (id, name) in functions_in(n)? {
                    let before = out.len();
                    of_function(&id, &format!("{}::{name}", file_of(n)), &mut out)?;
                    for (dep, _) in &out[before..] {
                        if let Some(dep_node) = node(conn, dep)? {
                            callers_found.insert(file_of(&dep_node));
                        }
                    }
                }
                // Importers that use something other than its functions (types, constants, the
                // module itself) still depend on it.
                if n.kind == "file" {
                    for (importer, name) in edges_into(&n.id, "'imports'")? {
                        if !callers_found.contains(&name) {
                            out.push((importer, format!("imports {}", n.name)));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(out)
    };

    let mut seen: HashSet<String> = HashSet::from([target.id.clone()]);
    let mut queue: VecDeque<(Node, usize)> = VecDeque::from([(target.clone(), 0)]);
    let mut files: BTreeMap<String, Affected> = BTreeMap::new();
    let mut endpoints: Vec<String> = vec![];
    if target.kind == "endpoint" {
        endpoints.push(target.name.clone());
    }
    while let Some((n, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        for (id, why) in dependents(&n)? {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(next) = node(conn, &id)? else { continue };
            if next.kind == "endpoint" {
                endpoints.push(next.name.clone());
            } else {
                let file = file_of(&next);
                let target_file = file_of(target);
                if file != target_file && !files.contains_key(&file) {
                    let area = area_of_file(conn, &file)?.map(|(name, kind)| format!("{name} ({kind})"));
                    let why = if next.kind == "file" { why.clone() } else { format!("{} → {}", next.name, why) };
                    files.insert(file.clone(), Affected { file, area, depth: d + 1, why });
                }
            }
            // Endpoints and services pass the change on without using up a step.
            let cost = if next.kind == "endpoint" || next.kind == "service" { 0 } else { 1 };
            queue.push_back((next, d + cost));
        }
    }

    // Decisions and other memories recorded on the target or any affected file.
    let mut all_files: Vec<String> = files.keys().cloned().collect();
    all_files.push(file_of(target));
    let mut stmt = conn.prepare(
        "SELECT DISTINCT m.type, m.title, m.claim FROM memories m JOIN edges e ON e.src = m.id AND e.type = 'touches'
         JOIN entities x ON x.id = e.dst
         WHERE m.status = 'current' AND x.qualified_name IN (SELECT value FROM json_each(?)) LIMIT 12",
    )?;
    let mut decisions: Vec<(String, String, String)> = stmt
        .query_map(params![serde_json::to_string(&all_files)?], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<engram_core::rusqlite::Result<_>>()?;
    // A service is rarely called from code alone (hosting, DNS, auth): also what the architecture and docs say.
    let (parts, named_in) = if target.kind == "service" {
        let (parts, named_in, memories) = service_reach(conn, &target.name, &files)?;
        for m in memories {
            if !decisions.contains(&m) {
                decisions.push(m);
            }
        }
        (parts, named_in)
    } else {
        (vec![], vec![])
    };

    let mut affected: Vec<Affected> = files.into_values().collect();
    affected.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.area.cmp(&b.area)).then(a.file.cmp(&b.file)));
    let written_by = engram_understand::provenance::provenance_line(&engram_understand::provenance::provenance(conn, &target.id)?);
    Ok(Impact { target: target.clone(), written_by, affected, endpoints, decisions, parts, named_in })
}

/// Agent-facing text for an impact result.
pub fn impact_text(i: &Impact, limit: usize) -> String {
    let mut lines = vec![];
    if let Some(who) = &i.written_by {
        lines.push(format!("{}: {who}", i.target.name));
    }
    if !i.parts.is_empty() {
        lines.push(format!("Parts that rely on {}:", i.target.name));
        lines.extend(i.parts.iter().map(|p| format!("  • {} ({})", p.relation, p.how)));
    }
    if i.affected.is_empty() && !i.parts.is_empty() {
        lines.push(format!("No code calls {} directly.", i.target.name));
    } else {
        lines.push(format!("Changing {} ({}) affects {} file(s) through code:", i.target.name, i.target.kind, i.affected.len()));
    }
    let mut by_area: BTreeMap<String, Vec<&Affected>> = BTreeMap::new();
    for a in i.affected.iter().take(limit) {
        by_area.entry(a.area.clone().unwrap_or_else(|| "(no area)".into())).or_default().push(a);
    }
    for (area, items) in by_area {
        lines.push(format!("{area}:"));
        lines.extend(items.iter().map(|a| format!("  • {} — {}", a.file, a.why)));
    }
    if i.affected.len() > limit {
        lines.push(format!("  … and {} more (raise `limit` or narrow the target)", i.affected.len() - limit));
    }
    if !i.endpoints.is_empty() {
        lines.push(format!("Endpoints involved: {}", i.endpoints.join(", ")));
    }
    if !i.named_in.is_empty() {
        lines.push("Docs and config that name it (update them if it changes):".into());
        lines.extend(i.named_in.iter().take(limit).map(|m| {
            let at = m.line.map(|l| format!("{}:{l}", m.file)).unwrap_or_else(|| m.file.clone());
            format!("  • {at} — {}", m.why)
        }));
    }
    if !i.decisions.is_empty() {
        let heading = if i.target.kind == "service" { "What memory says about it" } else { "Recorded decisions on these files" };
        lines.push(format!("{heading} (check before changing):"));
        lines.extend(i.decisions.iter().map(|(kind, title, claim)| format!("  • [{kind}] {title}: {claim}")));
    }
    lines.join("\n")
}

#[derive(Debug, Serialize)]
pub struct Endpoint {
    pub name: String,
    pub handlers: Vec<String>,
    pub callers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SystemMap {
    /// (name, kind, path, files)
    pub areas: Vec<(String, String, String, i64)>,
    pub endpoints: Vec<Endpoint>,
    /// (service, files using it)
    pub services: Vec<(String, Vec<String>)>,
    /// "frontend → backend" -> endpoint names
    pub links: BTreeMap<String, Vec<String>>,
    /// Area path -> what it relies on: "hosted on Oracle (per docs) · data PostgreSQL (same host) · auth Auth0".
    pub architecture: BTreeMap<String, String>,
    /// Memories the code contradicts: (id, title, reason).
    pub flags: Vec<(String, String, String)>,
    /// Architecture changes in the last 30 days, newest first.
    pub changes: Vec<String>,
    /// Unresolved contradictions the model found between memories, or with the code: (id, title, reason).
    pub conflicts: Vec<(String, String, String)>,
}

pub fn system_map(conn: &Connection) -> Result<SystemMap> {
    let mut stmt = conn.prepare(
        "SELECT a.name, a.kind, a.path, (SELECT COUNT(*) FROM edges e JOIN entities x ON x.id = e.dst WHERE e.type = 'in_area' AND x.kind = 'area' AND x.path = a.path)
         FROM areas a ORDER BY a.path",
    )?;
    let areas = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<engram_core::rusqlite::Result<Vec<_>>>()?;
    let list = |sql: &str, id: &str| -> Result<Vec<String>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
    };
    let mut stmt = conn.prepare("SELECT id, qualified_name FROM entities WHERE kind = 'endpoint' ORDER BY substr(qualified_name, instr(qualified_name, ' ') + 1), qualified_name")?;
    let ids: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut endpoints = vec![];
    let mut links: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut area_cache: HashMap<String, Option<String>> = HashMap::new();
    let mut area = |file: &str| -> Result<Option<String>> {
        if let Some(a) = area_cache.get(file) {
            return Ok(a.clone());
        }
        let a = area_of_file(conn, file)?.map(|(n, _)| n);
        area_cache.insert(file.to_string(), a.clone());
        Ok(a)
    };
    for (id, name) in ids {
        let handlers = list("SELECT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type = 'serves'", &id)?;
        let callers = list("SELECT DISTINCT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type = 'calls'", &id)?;
        for h in &handlers {
            for c in &callers {
                let (from, to) = (area(c.split("::").next().unwrap_or(c))?, area(h.split("::").next().unwrap_or(h))?);
                if let (Some(from), Some(to)) = (from, to) {
                    if from != to {
                        let entry = links.entry(format!("{from} → {to}")).or_default();
                        if !entry.contains(&name) {
                            entry.push(name.clone());
                        }
                    }
                }
            }
        }
        endpoints.push(Endpoint { name, handlers, callers });
    }
    let mut stmt = conn.prepare("SELECT id, qualified_name FROM entities WHERE kind = 'service' ORDER BY qualified_name")?;
    let svc: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let mut services = vec![];
    for (id, name) in svc {
        let mut users: Vec<String> = list("SELECT DISTINCT x.qualified_name FROM edges e JOIN entities x ON x.id = e.src WHERE e.dst = ? AND e.type IN ('calls','uses')", &id)?
            .into_iter()
            .map(|u| u.split("::").next().unwrap_or(&u).to_string())
            .collect();
        users.sort();
        users.dedup();
        services.push((name, users));
    }
    let architecture = engram_understand::architecture::summary(conn)?;
    let flags = engram_understand::architecture::flags(conn)?;
    let changes = engram_understand::architecture::changes(conn, 30)?;
    let conflicts = engram_understand::judge::conflicts(conn)?;
    Ok(SystemMap { areas, endpoints, services, links, architecture, flags, changes, conflicts })
}

/// Recent architecture changes, memories the code contradicts and unresolved conflicts, for `map` and `brief`.
pub fn architecture_notes(m: &SystemMap) -> Vec<String> {
    let mut lines = vec![];
    if !m.changes.is_empty() {
        lines.push("Architecture changes (last 30 days):".into());
        lines.extend(m.changes.iter().take(5).map(|c| format!("  • {c}")));
    }
    if !m.flags.is_empty() {
        lines.push("Memories the code now contradicts (check before relying on them):".into());
        lines.extend(m.flags.iter().map(|(id, title, reason)| format!("  • {title} ({id}) {reason}")));
    }
    if !m.conflicts.is_empty() {
        lines.push("Unresolved conflicts between memories (ask the user which holds):".into());
        lines.extend(m.conflicts.iter().take(5).map(|(id, title, reason)| format!("  • {title} ({id}) {reason}")));
    }
    lines
}

fn is_test(path: &str) -> bool {
    path.contains("/tests/") || path.starts_with("tests/") || path.contains(".spec.") || path.rsplit('/').next().is_some_and(|f| f.starts_with("test_"))
}

/// Agent-facing text: the parts, how they connect, every endpoint with its handler and its
/// non-test callers, and external services.
pub fn map_text(m: &SystemMap, area_filter: Option<&str>) -> String {
    let mut lines = vec!["Parts:".to_string()];
    lines.extend(m.areas.iter().map(|(name, kind, path, files)| {
        let relies = m.architecture.get(path).map_or(String::new(), |a| format!(": {a}"));
        format!("  • {name} — {kind} ({}, {files} files){relies}", if path.is_empty() { "." } else { path })
    }));
    if !m.links.is_empty() {
        lines.push("How they connect:".into());
        lines.extend(m.links.iter().map(|(pair, eps)| format!("  • {pair} over HTTP: {} endpoint(s)", eps.len())));
    }
    lines.extend(architecture_notes(m));
    let short = |q: &str| -> String {
        let (file, sym) = q.split_once("::").map_or((q, None), |(f, s)| (f, Some(s)));
        let file = file.rsplit('/').next().unwrap_or(file);
        sym.map_or(file.to_string(), |s| format!("{file}::{s}"))
    };
    let wanted = |e: &Endpoint| area_filter.is_none_or(|a| e.handlers.iter().chain(&e.callers).any(|x| x.contains(a)));
    let served: Vec<&Endpoint> = m.endpoints.iter().filter(|e| !e.handlers.is_empty() && wanted(e)).collect();
    if !served.is_empty() {
        lines.push("Endpoints (handler ← callers outside tests):".into());
        for e in served {
            let callers: Vec<String> = e.callers.iter().filter(|c| !is_test(c)).map(|c| short(c)).collect();
            let tests = e.callers.iter().filter(|c| is_test(c)).count();
            let callers = match (callers.is_empty(), tests) {
                (true, 0) => "no caller in this repo".to_string(),
                (true, t) => format!("{t} test(s) only"),
                (false, 0) => callers.join(", "),
                (false, t) => format!("{} (+{t} tests)", callers.join(", ")),
            };
            lines.push(format!("  • {} — {} ← {callers}", e.name, e.handlers.iter().map(|h| short(h)).collect::<Vec<_>>().join(", ")));
        }
    }
    let unserved: Vec<&Endpoint> = m.endpoints.iter().filter(|e| e.handlers.is_empty() && wanted(e)).collect();
    if !unserved.is_empty() {
        lines.push(format!("Called but not served here: {}", unserved.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if !m.services.is_empty() {
        lines.push("External services:".into());
        lines.extend(m.services.iter().map(|(s, users)| format!("  • {s} — used by {}", users.iter().map(|u| short(u)).collect::<Vec<_>>().join(", "))));
    }
    lines.join("\n")
}
