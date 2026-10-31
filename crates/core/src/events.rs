use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::{new_id, now, tx};
use crate::error::Result;

#[derive(Debug, Clone, Serialize)]
pub struct Actor {
    pub id: String,
    pub kind: String,
    pub name: String,
}

/// Find or create an actor. `kind` is one of agent, user, system.
pub fn ensure_actor(conn: &Connection, kind: &str, name: &str) -> Result<Actor> {
    let found = conn
        .query_row("SELECT id FROM actors WHERE kind = ? AND name = ?", [kind, name], |r| r.get::<_, String>(0))
        .optional()?;
    let id = match found {
        Some(id) => id,
        None => {
            let id = new_id("act");
            conn.execute(
                "INSERT INTO actors (id, kind, name, created_at) VALUES (?, ?, ?, ?)",
                params![id, kind, name, now()],
            )?;
            id
        }
    };
    Ok(Actor { id, kind: kind.into(), name: name.into() })
}

pub fn start_session(conn: &Connection, actor_id: &str, external_ref: Option<&str>) -> Result<String> {
    let id = new_id("ses");
    conn.execute(
        "INSERT INTO sessions (id, actor_id, external_ref, started_at) VALUES (?, ?, ?, ?)",
        params![id, actor_id, external_ref, now()],
    )?;
    Ok(id)
}

#[derive(Debug, Default, Clone)]
pub struct NewEvent {
    pub kind: String,
    pub payload: Value,
    pub actor_id: Option<String>,
    pub session_id: Option<String>,
    pub commit_sha: Option<String>,
    pub device_id: Option<String>,
    /// Idempotency key: re-sending the same op id returns the original event.
    pub op_id: Option<String>,
    pub ts: Option<String>,
}

impl NewEvent {
    pub fn new(kind: &str, payload: Value) -> Self {
        NewEvent { kind: kind.into(), payload, ..Default::default() }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    pub seq: i64,
    pub op_id: String,
    pub ts: String,
    pub kind: String,
    pub actor_id: Option<String>,
    pub session_id: Option<String>,
    pub commit_sha: Option<String>,
    pub payload: Value,
    pub hash: String,
}

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn chain_hash(prev: &str, op_id: &str, kind: &str, payload: &str) -> String {
    hex::encode(Sha256::digest(format!("{prev}\n{op_id}\n{kind}\n{payload}")))
}

fn to_event(r: &Row) -> rusqlite::Result<StoredEvent> {
    let payload: String = r.get("payload")?;
    Ok(StoredEvent {
        seq: r.get("seq")?,
        op_id: r.get("op_id")?,
        ts: r.get("ts")?,
        kind: r.get("kind")?,
        actor_id: r.get("actor_id")?,
        session_id: r.get("session_id")?,
        commit_sha: r.get("commit_sha")?,
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        hash: r.get("hash")?,
    })
}

pub fn append_event(conn: &Connection, e: NewEvent) -> Result<StoredEvent> {
    let op_id = e.op_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    tx(conn, |conn| {
        if let Some(existing) = conn.query_row("SELECT * FROM events WHERE op_id = ?", [&op_id], to_event).optional()? {
            return Ok(existing);
        }
        let prev: String = conn
            .query_row("SELECT hash FROM events ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
            .optional()?
            .unwrap_or_else(|| GENESIS.to_string());
        let payload = serde_json::to_string(&e.payload)?;
        let hash = chain_hash(&prev, &op_id, &e.kind, &payload);
        conn.execute(
            "INSERT INTO events (op_id, ts, kind, actor_id, session_id, device_id, commit_sha, payload, prev_hash, hash)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                op_id,
                e.ts.unwrap_or_else(now),
                e.kind,
                e.actor_id,
                e.session_id,
                e.device_id.unwrap_or_else(|| "local".into()),
                e.commit_sha,
                payload,
                prev,
                hash
            ],
        )?;
        let seq = conn.last_insert_rowid();
        Ok(conn.query_row("SELECT * FROM events WHERE seq = ?", [seq], to_event)?)
    })
}

pub fn get_event(conn: &Connection, seq: i64) -> Result<Option<StoredEvent>> {
    Ok(conn.query_row("SELECT * FROM events WHERE seq = ?", [seq], to_event).optional()?)
}

pub fn list_events(conn: &Connection, after_seq: i64, kind: Option<&str>, limit: i64) -> Result<Vec<StoredEvent>> {
    let mut stmt =
        conn.prepare("SELECT * FROM events WHERE seq > ?1 AND (?2 IS NULL OR kind = ?2) ORDER BY seq LIMIT ?3")?;
    let rows = stmt.query_map(params![after_seq, kind, limit], to_event)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The most recent `limit` events, oldest first.
/// Events with `from <= ts < to`, oldest first.
pub fn events_between(conn: &Connection, from: &str, to: &str) -> Result<Vec<StoredEvent>> {
    let mut stmt = conn.prepare("SELECT * FROM events WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, seq")?;
    let rows = stmt.query_map(params![from, to], to_event)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn recent_events(conn: &Connection, limit: i64) -> Result<Vec<StoredEvent>> {
    let max: i64 = conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |r| r.get(0))?;
    list_events(conn, (max - limit).max(0), None, limit)
}

/// Recompute the hash chain. Returns the first broken seq, or None if the log is intact.
pub fn verify_chain(conn: &Connection) -> Result<Option<i64>> {
    let mut stmt = conn.prepare("SELECT seq, op_id, kind, payload, prev_hash, hash FROM events ORDER BY seq")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?))
    })?;
    let mut prev = GENESIS.to_string();
    for row in rows {
        let (seq, op_id, kind, payload, prev_hash, hash) = row?;
        if prev_hash != prev || hash != chain_hash(&prev, &op_id, &kind, &payload) {
            return Ok(Some(seq));
        }
        prev = hash;
    }
    Ok(None)
}
