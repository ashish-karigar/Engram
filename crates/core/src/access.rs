use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::Value;

use crate::db::now;
use crate::error::Result;

#[derive(Debug, Default)]
pub struct AccessRecord {
    pub actor_id: Option<String>,
    pub session_id: Option<String>,
    pub tool: String,
    pub request: Value,
    /// What was returned and why, e.g. `[{id, score, reason}]`.
    pub returned: Value,
    pub tokens_out: Option<i64>,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
}

pub fn log_access(conn: &Connection, a: AccessRecord) -> Result<i64> {
    conn.execute(
        "INSERT INTO access_log (ts, actor_id, session_id, tool, request, returned, tokens_out, latency_ms, error)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            now(),
            a.actor_id,
            a.session_id,
            a.tool,
            serde_json::to_string(&a.request)?,
            serde_json::to_string(&a.returned)?,
            a.tokens_out,
            a.latency_ms,
            a.error
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessEntry {
    pub id: i64,
    pub ts: String,
    pub actor: Option<String>,
    pub tool: String,
    pub request: Value,
    pub returned: Value,
    pub tokens_out: Option<i64>,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
}

pub fn recent_access(conn: &Connection, actor_id: Option<&str>, limit: i64) -> Result<Vec<AccessEntry>> {
    let mut stmt = conn.prepare(
        "SELECT l.id, l.ts, l.tool, l.request, l.returned, l.tokens_out, l.latency_ms, l.error, a.name AS actor
         FROM access_log l LEFT JOIN actors a ON a.id = l.actor_id
         WHERE (?1 IS NULL OR l.actor_id = ?1) ORDER BY l.id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![actor_id, limit], |r| {
        let request: String = r.get("request")?;
        let returned: String = r.get("returned")?;
        Ok(AccessEntry {
            id: r.get("id")?,
            ts: r.get("ts")?,
            actor: r.get("actor")?,
            tool: r.get("tool")?,
            request: serde_json::from_str(&request).unwrap_or(Value::Null),
            returned: serde_json::from_str(&returned).unwrap_or(Value::Null),
            tokens_out: r.get("tokens_out")?,
            latency_ms: r.get("latency_ms")?,
            error: r.get("error")?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
