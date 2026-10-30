use std::collections::HashMap;

use engram_core::access::recent_access;
use engram_core::events::recent_events;
use engram_core::graph::get_entity;
use engram_core::memory::{get_memory, history};
use engram_core::rusqlite::{Result as SqlResult, params};
use engram_core::{Connection, Result};
use serde_json::{Value, json};

/// Read-only JSON views for the desktop UI.
pub fn graph_data(conn: &Connection, include_archived: bool, limit: i64) -> Result<Value> {
    let mut nodes: HashMap<String, Value> = HashMap::new();
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
        Some(entity) => json!({"type": "entity", "entity": entity}),
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
    }))
}

pub fn timeline(conn: &Connection, limit: i64) -> Result<Value> {
    Ok(serde_json::to_value(recent_events(conn, limit)?)?)
}

pub fn request_log(conn: &Connection, limit: i64) -> Result<Value> {
    Ok(serde_json::to_value(recent_access(conn, None, limit)?)?)
}
