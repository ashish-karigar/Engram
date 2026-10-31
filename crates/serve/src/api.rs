use std::collections::BTreeMap;

use engram_core::access::recent_access;
use engram_core::events::recent_events;
use engram_core::graph::get_entity;
use engram_core::memory::{get_memory, history};
use engram_core::rusqlite::{Result as SqlResult, params};
use engram_core::{Connection, Result};
use serde_json::{Value, json};

/// The architecture graph: parts (areas), the endpoints between them, external services, and the
/// recorded memories attached to the part whose files they are about. Tens of nodes, not thousands:
/// what a person or an agent needs to see how the system hangs together.
pub fn architecture_data(conn: &Connection) -> Result<Value> {
    let mut nodes: BTreeMap<String, Value> = BTreeMap::new();
    let mut edges: BTreeMap<(String, String, String), ()> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT x.id, a.name, a.kind, a.path, (SELECT COUNT(*) FROM edges e WHERE e.dst = x.id AND e.type = 'in_area')
         FROM areas a JOIN entities x ON x.kind = 'area' AND x.path = a.path",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))? {
        let (id, name, kind, path, files) = row?;
        nodes.insert(id.clone(), json!({"id": id, "label": name, "kind": "area", "status": kind, "path": path, "files": files}));
    }
    // The area of the file behind an entity (a file, or a function `file::name`).
    let area_sql = "SELECT e.dst FROM entities f JOIN edges e ON e.src = f.id AND e.type = 'in_area'
                    WHERE f.kind = 'file' AND f.qualified_name = ? LIMIT 1";
    let area_of = |qualified: &str| -> Result<Option<String>> {
        let file = qualified.split("::").next().unwrap_or(qualified);
        Ok(conn.query_row(area_sql, [file], |r| r.get::<_, String>(0)).optional_ok()?)
    };
    let mut stmt = conn.prepare(
        "SELECT g.type, s.qualified_name, d.id, d.kind, d.qualified_name FROM edges g
         JOIN entities s ON s.id = g.src JOIN entities d ON d.id = g.dst
         WHERE g.type IN ('serves','calls','uses') AND g.valid_to IS NULL",
    )?;
    let rows: Vec<(String, String, String, String, String)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<SqlResult<_>>()?;
    for (kind, from, target_id, target_kind, target_name) in rows {
        let Some(area) = area_of(&from)? else { continue };
        nodes.entry(target_id.clone()).or_insert_with(|| json!({"id": target_id, "label": target_name, "kind": target_kind}));
        let edge = match kind.as_str() {
            "serves" => (target_id.clone(), area, "served by".to_string()),
            "uses" => (area, target_id.clone(), "uses".to_string()),
            _ => (area, target_id.clone(), "calls".to_string()),
        };
        edges.insert(edge, ());
    }
    // Memories, attached to the part their files belong to.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT m.id, m.type, m.title, m.status, f.qualified_name FROM memories m
         JOIN edges t ON t.src = m.id AND t.type = 'touches' JOIN entities f ON f.id = t.dst
         WHERE m.status = 'current' AND m.type != 'episode'",
    )?;
    let memories: Vec<(String, String, String, String, String)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<SqlResult<_>>()?;
    for (id, kind, title, status, file) in memories {
        let Some(area) = area_of(&file)? else { continue };
        nodes.entry(id.clone()).or_insert_with(|| json!({"id": id, "label": title, "kind": kind, "status": status}));
        edges.insert((id, area, "about".to_string()), ());
    }
    let edges: Vec<Value> = edges
        .into_keys()
        .filter(|(s, d, _)| nodes.contains_key(s) && nodes.contains_key(d))
        .map(|(s, d, t)| json!({"id": format!("{s}>{d}:{t}"), "source": s, "target": d, "type": t}))
        .collect();
    Ok(json!({"nodes": nodes.into_values().collect::<Vec<_>>(), "edges": edges}))
}

trait OptionalOk<T> {
    fn optional_ok(self) -> SqlResult<Option<T>>;
}
impl<T> OptionalOk<T> for SqlResult<T> {
    fn optional_ok(self) -> SqlResult<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(engram_core::rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Read-only JSON views for the desktop UI.
pub fn graph_data(conn: &Connection, include_archived: bool, limit: i64) -> Result<Value> {
    // Ordered by id, so the same graph always serializes the same way and clients can tell real changes.
    let mut nodes: BTreeMap<String, Value> = BTreeMap::new();
    let memories: Vec<(String, String, String, String, i64)> = conn
        .prepare(
            "SELECT id, type, title, status, is_hub FROM memories
             WHERE (?1 = 1 OR status IN ('current','superseded')) ORDER BY observed_at DESC LIMIT ?2",
        )?
        .query_map(params![include_archived as i64, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<SqlResult<_>>()?;
    for (id, kind, title, status, hub) in &memories {
        nodes.insert(id.clone(), json!({"id": id, "label": title, "kind": kind, "status": status, "isHub": *hub == 1}));
    }

    let ids = serde_json::to_string(&nodes.keys().collect::<Vec<_>>())?;
    let entities: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT DISTINCT e.id, e.kind, e.qualified_name FROM entities e JOIN edges g ON (g.src = e.id OR g.dst = e.id)
             WHERE g.src IN (SELECT value FROM json_each(?1)) OR g.dst IN (SELECT value FROM json_each(?1))
             UNION SELECT id, kind, qualified_name FROM entities WHERE kind IN ('module','file') AND status = 'active'",
        )?
        .query_map([&ids], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<SqlResult<_>>()?;
    for (id, kind, name) in entities {
        nodes.insert(id.clone(), json!({"id": id, "label": name, "kind": kind}));
    }

    let documents: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT id, title, source FROM documents
             WHERE status = 'done' AND mime NOT LIKE 'text/x-%'
               AND NOT (COALESCE(path, '') GLOB '*.json' OR COALESCE(path, '') GLOB '*.toml' OR COALESCE(path, '') GLOB '*.yaml'
                        OR COALESCE(path, '') GLOB '*.yml' OR COALESCE(path, '') GLOB '*.ini' OR COALESCE(path, '') GLOB '*.cfg'
                        OR COALESCE(path, '') GLOB '*.xml' OR COALESCE(path, '') GLOB '*.csv')
             ORDER BY updated_at DESC LIMIT 150",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<SqlResult<_>>()?;
    for (id, title, source) in documents {
        nodes.insert(id.clone(), json!({"id": id, "label": title, "kind": "document", "status": source}));
    }

    let all = serde_json::to_string(&nodes.keys().collect::<Vec<_>>())?;
    let edges: Vec<Value> = conn
        .prepare(
            "SELECT id, src, dst, type FROM edges
             WHERE valid_to IS NULL AND src IN (SELECT value FROM json_each(?1)) AND dst IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([&all], |r| Ok(json!({"id": r.get::<_, String>(0)?, "source": r.get::<_, String>(1)?, "target": r.get::<_, String>(2)?, "type": r.get::<_, String>(3)?})))?
        .collect::<SqlResult<_>>()?;
    Ok(json!({"nodes": nodes.into_values().collect::<Vec<_>>(), "edges": edges}))
}

pub fn node_detail(conn: &Connection, id: &str) -> Result<Value> {
    if id.starts_with("chk_") {
        let doc_id: Option<String> = conn.query_row("SELECT document_id FROM chunks WHERE id = ?", [id], |r| r.get(0)).ok();
        let mut detail = match doc_id {
            Some(doc_id) => node_detail(conn, &doc_id)?,
            None => Value::Null,
        };
        if let Some(obj) = detail.as_object_mut() {
            obj.insert("focus".into(), json!(id));
        }
        return Ok(detail);
    }
    if let Some(doc) = engram_rag::document::get_document(conn, id)? {
        let chunks: Vec<Value> = conn
            .prepare("SELECT id, heading, start_line, end_line, substr(content, 1, 400) FROM chunks WHERE document_id = ? ORDER BY ordinal")?
            .query_map([id], |r| Ok(json!({"id": r.get::<_, String>(0)?, "heading": r.get::<_, Option<String>>(1)?, "startLine": r.get::<_, Option<i64>>(2)?, "endLine": r.get::<_, Option<i64>>(3)?, "preview": r.get::<_, String>(4)?})))?
            .collect::<SqlResult<_>>()?;
        return Ok(json!({"type": "document", "document": doc, "chunks": chunks}));
    }
    if let Some(memory) = get_memory(conn, id)? {
        let evidence: Vec<Value> = conn
            .prepare("SELECT e.seq, e.ts, e.kind, e.payload FROM memory_evidence me JOIN events e ON e.seq = me.event_seq WHERE me.memory_id = ?")?
            .query_map([id], |r| {
                let payload: String = r.get(3)?;
                Ok(json!({"seq": r.get::<_, i64>(0)?, "ts": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "payload": serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null)}))
            })?
            .collect::<SqlResult<_>>()?;
        return Ok(json!({"type": "memory", "memory": memory, "history": history(conn, id)?, "evidence": evidence}));
    }
    Ok(match get_entity(conn, id)? {
        Some(entity) => {
            let signature: Option<String> = conn.query_row("SELECT signature FROM code_symbols WHERE entity_id = ?", [id], |r| r.get(0)).ok();
            let provenance = engram_understand::provenance::provenance(conn, id)?;
            json!({"type": "entity", "entity": entity, "signature": signature, "provenance": provenance})
        }
        None => Value::Null,
    })
}

pub fn stats(conn: &Connection) -> Result<Value> {
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0));
    Ok(json!({
        "events": count("SELECT COUNT(*) FROM events")?,
        "memories": count("SELECT COUNT(*) FROM memories WHERE status = 'current'")?,
        "archived": count("SELECT COUNT(*) FROM memories WHERE status = 'archived'")?,
        "entities": count("SELECT COUNT(*) FROM entities WHERE status = 'active'")?,
        "requests": count("SELECT COUNT(*) FROM access_log")?,
        "documents": count("SELECT COUNT(*) FROM documents WHERE status = 'done'")?,
        "chunks": count("SELECT COUNT(*) FROM chunks")?,
        "inferred": count("SELECT COUNT(*) FROM memories WHERE is_inference = 1 AND reviewed_at IS NULL AND status = 'current'")?,
    }))
}

pub fn timeline(conn: &Connection, limit: i64) -> Result<Value> {
    Ok(serde_json::to_value(recent_events(conn, limit)?)?)
}

pub fn request_log(conn: &Connection, limit: i64) -> Result<Value> {
    Ok(serde_json::to_value(recent_access(conn, None, limit)?)?)
}

pub fn documents(conn: &Connection, limit: i64) -> Result<Value> {
    Ok(serde_json::to_value(engram_rag::document::list_documents(conn, None, limit)?)?)
}

pub fn review_queue(conn: &Connection) -> Result<Value> {
    Ok(serde_json::to_value(engram_core::memory::list_inferred(conn, 50)?)?)
}
