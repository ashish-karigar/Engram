//! The zoomable map: parts → folders → files → classes → functions, opened where you look.
//!
//! Given the set of expanded nodes, return what is visible: the top level (parts, endpoints,
//! external services, memories), plus the children of every expanded node. Every fine-grained edge
//! in the model (function calls, imports, HTTP calls and routes, service use, memories) is moved up
//! to the nearest visible ancestor of each end and counted, so a collapsed folder shows "34 calls
//! into core/" and an expanded one shows exactly which functions. Each node carries who wrote most
//! of it (from provenance), for colouring the map by author.

use std::collections::{BTreeMap, HashMap, HashSet};

use engram_core::rusqlite::params;
use engram_core::{Connection, Result};
use serde_json::{Value, json};

struct Item {
    kind: String,
    label: String,
    parent: Option<String>,
    path: Option<String>,
    /// The part (area entity id) it belongs to, for the flow layout.
    area: Option<String>,
}

fn dir_id(path: &str) -> String {
    format!("dir:{path}")
}

/// Folders inside a part, with single-child chains merged (`src/app/core` is one step, not three).
fn folder_parent(area_path: &str, file: &str, keep: &HashSet<String>) -> Option<String> {
    let mut dir = file.rsplit_once('/').map(|(d, _)| d.to_string())?;
    loop {
        if dir == area_path || dir.is_empty() {
            return None;
        }
        if keep.contains(&dir) {
            return Some(dir);
        }
        dir = dir.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    }
}

fn who_of(sha: &str, agent: Option<String>, author: Option<String>) -> String {
    match (agent, sha) {
        (Some(a), _) => engram_understand::provenance::agent_label(&a),
        (None, "uncommitted") => "uncommitted".into(),
        (None, _) => author.unwrap_or_else(|| "unknown".into()),
    }
}

pub fn zoom_graph(conn: &Connection, expanded: &[String]) -> Result<Value> {
    let expanded: HashSet<&str> = expanded.iter().map(String::as_str).collect();
    let mut items: HashMap<String, Item> = HashMap::new();

    // Parts.
    let mut stmt = conn.prepare("SELECT x.id, a.name, a.path FROM areas a JOIN entities x ON x.kind = 'area' AND x.path = a.path")?;
    let areas: Vec<(String, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    for (id, name, path) in &areas {
        items.insert(id.clone(), Item { kind: "area".into(), label: name.clone(), parent: None, path: Some(path.clone()), area: Some(id.clone()) });
    }

    // Files, grouped under folders inside their part.
    let mut stmt = conn.prepare(
        "SELECT f.id, f.qualified_name, x.id, x.path FROM entities f
           JOIN edges e ON e.src = f.id AND e.type = 'in_area' JOIN entities x ON x.id = e.dst
          WHERE f.kind = 'file' AND f.status = 'active'",
    )?;
    let files: Vec<(String, String, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    // A folder is kept as a step when it holds files directly or branches into several folders.
    let mut child_dirs: HashMap<String, HashSet<String>> = HashMap::new();
    let mut holds_files: HashSet<String> = HashSet::new();
    for (_, path, _, area_path) in &files {
        let mut dir = path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        holds_files.insert(dir.clone());
        while !dir.is_empty() && dir != *area_path {
            let up = dir.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            child_dirs.entry(up.clone()).or_default().insert(dir.clone());
            dir = up;
        }
    }
    let keep: HashSet<String> = holds_files.iter().cloned().chain(child_dirs.iter().filter(|(_, c)| c.len() > 1).map(|(d, _)| d.clone())).collect();
    for (id, path, area_id, area_path) in &files {
        let parent = folder_parent(area_path, path, &keep);
        // Register the folder chain for this file.
        let mut child = parent.clone();
        while let Some(dir) = child {
            if items.contains_key(&dir_id(&dir)) {
                break;
            }
            // The folder's own parent: its nearest kept ancestor (a folder path reads like a file path here).
            let up = folder_parent(area_path, &dir, &keep);
            let label = match &up {
                Some(u) => dir.strip_prefix(&format!("{u}/")).unwrap_or(&dir).to_string(),
                None => dir.strip_prefix(&format!("{area_path}/")).unwrap_or(&dir).to_string(),
            };
            items.insert(dir_id(&dir), Item { kind: "dir".into(), label, parent: Some(up.clone().map_or(area_id.clone(), |u| dir_id(&u))), path: Some(dir.clone()), area: Some(area_id.clone()) });
            child = up;
        }
        let label = path.rsplit('/').next().unwrap_or(path).to_string();
        items.insert(id.clone(), Item { kind: "file".into(), label, parent: Some(parent.map_or(area_id.clone(), |d| dir_id(&d))), path: Some(path.clone()), area: Some(area_id.clone()) });
    }

    // Classes and functions, under their file or class.
    let file_ids: HashMap<String, String> = files.iter().map(|(id, path, _, _)| (path.clone(), id.clone())).collect();
    let mut stmt = conn.prepare("SELECT entity_id, file, name, kind, parent FROM code_symbols")?;
    let symbols: Vec<(String, String, String, String, Option<String>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let symbol_ids: HashMap<(String, String), String> = symbols.iter().map(|(id, f, n, _, _)| ((f.clone(), n.clone()), id.clone())).collect();
    for (id, file, name, kind, parent) in &symbols {
        let Some(file_id) = file_ids.get(file) else { continue };
        let owner = parent.as_ref().and_then(|p| symbol_ids.get(&(file.clone(), p.clone()))).cloned().unwrap_or_else(|| file_id.clone());
        let label = name.rsplit(['.', ':']).next().unwrap_or(name).to_string();
        let area = items.get(file_id).and_then(|f| f.area.clone());
        items.insert(id.clone(), Item { kind: kind.clone(), label, parent: Some(owner), path: Some(file.clone()), area });
    }

    // APIs: each part that serves endpoints gets one API body between it and its callers; inside
    // it, endpoints are grouped by resource (`/jobs`, `/access`); a group of one is skipped.
    // Endpoints nothing here serves share one body. Calls lift to whatever level is open.
    let mut hub_serves: HashMap<String, String> = HashMap::new();
    let mut hub_size: HashMap<String, usize> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT x.id, x.qualified_name, (SELECT e.src FROM edges e WHERE e.dst = x.id AND e.type = 'serves' LIMIT 1)
           FROM entities x WHERE x.kind = 'endpoint' ORDER BY x.qualified_name",
    )?;
    let endpoints: Vec<(String, String, Option<String>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let placed: Vec<(String, String, String, String)> = endpoints
        .into_iter()
        .map(|(id, name, handler)| {
            let area = handler.and_then(|h| items.get(&h).and_then(|i| i.area.clone()));
            let hub = area.as_ref().map_or("api:unserved".to_string(), |a| format!("api:{a}"));
            let path = name.split_once(' ').map_or(name.as_str(), |(_, p)| p);
            let resource = path.split('/').find(|s| !s.is_empty()).map_or("/".to_string(), |s| format!("/{s}"));
            if let Some(a) = area {
                hub_serves.insert(hub.clone(), a);
            }
            (id, name, hub, resource)
        })
        .collect();
    let mut group_size: HashMap<(String, String), usize> = HashMap::new();
    for (_, _, hub, resource) in &placed {
        *group_size.entry((hub.clone(), resource.clone())).or_default() += 1;
        *hub_size.entry(hub.clone()).or_default() += 1;
    }
    for hub in hub_size.keys() {
        let label = match hub_serves.get(hub).and_then(|a| items.get(a)) {
            Some(area) => format!("{} API", area.label),
            None => "Called, not served here".to_string(),
        };
        items.insert(hub.clone(), Item { kind: "api".into(), label, parent: None, path: None, area: None });
    }
    for (id, name, hub, resource) in placed {
        let parent = if group_size[&(hub.clone(), resource.clone())] > 1 {
            let group = format!("apigroup:{hub}:{resource}");
            items.entry(group.clone()).or_insert_with(|| Item { kind: "api_group".into(), label: resource.clone(), parent: Some(hub.clone()), path: None, area: None });
            group
        } else {
            hub.clone()
        };
        items.insert(id, Item { kind: "endpoint".into(), label: name, parent: Some(parent), path: None, area: None });
    }

    // Children counts, and what is visible: the roots plus children of expanded nodes.
    let mut children: HashMap<String, usize> = HashMap::new();
    for item in items.values() {
        if let Some(p) = &item.parent {
            *children.entry(p.clone()).or_default() += 1;
        }
    }
    let visible = |id: &str, items: &HashMap<String, Item>| -> bool {
        let mut cur = items.get(id).and_then(|i| i.parent.clone());
        while let Some(p) = cur {
            if !expanded.contains(p.as_str()) {
                return false;
            }
            cur = items.get(&p).and_then(|i| i.parent.clone());
        }
        true
    };
    let shown: HashSet<String> = items.keys().filter(|id| visible(id, &items)).cloned().collect();
    // The nearest visible ancestor (or itself) of a code entity.
    let lift = |id: &str| -> Option<String> {
        let mut cur = Some(id.to_string());
        while let Some(c) = cur {
            if shown.contains(&c) {
                return Some(c);
            }
            cur = items.get(&c).and_then(|i| i.parent.clone());
        }
        None
    };

    let mut nodes: BTreeMap<String, Value> = BTreeMap::new();
    for id in &shown {
        let item = &items[id];
        // API groups and endpoints know their API body, so calls into them count toward it.
        let mut root = id.clone();
        while let Some(p) = items.get(&root).and_then(|i| i.parent.clone()) {
            root = p;
        }
        let hub = (item.kind == "api_group" || item.kind == "endpoint").then_some(root);
        nodes.insert(id.clone(), json!({
            "id": id, "label": item.label, "kind": item.kind, "path": item.path, "area": item.area, "hub": hub,
            "children": children.get(id).copied().unwrap_or(0), "expanded": expanded.contains(id.as_str()),
        }));
    }
    for (hub, area) in &hub_serves {
        if let Some(n) = nodes.get_mut(hub) {
            n["serves"] = json!(area);
            n["endpoints"] = json!(hub_size[hub]);
        }
    }
    if let Some(n) = nodes.get_mut("api:unserved") {
        n["endpoints"] = json!(hub_size["api:unserved"]);
    }
    // External services and memories are always visible.
    let mut stmt = conn.prepare("SELECT id, kind, qualified_name FROM entities WHERE kind = 'service'")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
        let (id, kind, name) = row?;
        nodes.insert(id.clone(), json!({"id": id, "label": name, "kind": kind}));
    }

    // Every fine-grained edge, lifted to visible ends and counted.
    let mut counts: BTreeMap<(String, String, String), i64> = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT src, dst, type FROM edges WHERE type IN ('invokes','imports','calls','serves','uses') AND valid_to IS NULL")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
        let (src, dst, kind) = row?;
        let ends = |e: &str| if nodes.contains_key(e) && !items.contains_key(e) { Some(e.to_string()) } else { lift(e) };
        if let (Some(s), Some(d)) = (ends(&src), ends(&dst)) {
            if s != d {
                *counts.entry((s, d, kind)).or_default() += 1;
            }
        }
    }
    // Containment, so an opened node keeps its children around it.
    for id in &shown {
        if let Some(p) = items[id].parent.as_ref().filter(|p| shown.contains(*p)) {
            counts.insert((p.clone(), id.clone(), "contains".into()), 1);
        }
    }
    // Memories, on the most specific visible thing they are about.
    let mut stmt = conn.prepare(
        "SELECT m.id, m.type, m.title, m.status, t.dst FROM memories m JOIN edges t ON t.src = m.id AND t.type = 'touches'
          WHERE m.status = 'current' AND m.type != 'episode'",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))? {
        let (id, kind, title, status, about) = row?;
        if let Some(target) = lift(&about) {
            nodes.entry(id.clone()).or_insert_with(|| json!({"id": id, "label": title, "kind": kind, "status": status}));
            counts.insert((id, target, "about".into()), 1);
        }
    }

    // Who wrote most of each visible code node: lines of its files (or its own lines for symbols).
    let mut stmt = conn.prepare(
        "SELECT h.entity_id, h.commit_sha, h.lines, c.agent, c.author FROM code_history h LEFT JOIN commits c ON c.sha = h.commit_sha",
    )?;
    let mut authorship: HashMap<String, HashMap<String, i64>> = HashMap::new();
    for row in stmt.query_map(params![], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?)))? {
        let (entity, sha, lines, agent, author) = row?;
        let Some(item) = items.get(&entity) else { continue };
        // Files count toward their folders and part; symbols only toward themselves (no double count).
        let mut owners = vec![entity.clone()];
        if item.kind == "file" {
            let mut cur = item.parent.clone();
            while let Some(p) = cur {
                owners.push(p.clone());
                cur = items.get(&p).and_then(|i| i.parent.clone());
            }
        }
        let who = who_of(&sha, agent, author);
        for o in owners {
            *authorship.entry(o).or_default().entry(who.clone()).or_default() += lines;
        }
    }
    for (id, node) in nodes.iter_mut() {
        if let Some(by) = authorship.get(id) {
            let total: i64 = by.values().sum();
            if let Some((who, lines)) = by.iter().max_by_key(|(_, l)| **l) {
                node["owner"] = json!({"who": who, "share": *lines as f64 / total.max(1) as f64, "lines": total});
            }
        }
    }

    let edges: Vec<Value> = counts
        .into_iter()
        .filter(|((s, d, _), _)| nodes.contains_key(s) && nodes.contains_key(d))
        .map(|((s, d, t), n)| json!({"id": format!("{s}>{d}:{t}"), "source": s, "target": d, "type": t, "count": n}))
        .collect();
    Ok(json!({"nodes": nodes.into_values().collect::<Vec<_>>(), "edges": edges}))
}
