use rusqlite::{Connection, OptionalExtension, Row, named_params, params};
use serde::Serialize;

use crate::db::{new_id, now, tx};
use crate::error::{Error, Result};
use crate::graph::{EdgeOpts, add_edge};

pub const TYPES: &[&str] = &["decision", "fact", "preference", "episode", "rejected_option", "dead_end", "task"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub claim: String,
    pub rationale: Option<String>,
    pub scope: Option<String>,
    pub summary: Option<String>,
    /// current, superseded, archived or expired.
    pub status: String,
    pub version: i64,
    pub is_hub: bool,
    pub observed_at: String,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub event_date: Option<String>,
    pub commit_sha: Option<String>,
    pub confidence: f64,
    pub salience: f64,
    pub access_count: i64,
    pub last_accessed_at: Option<String>,
    pub authored_by: Option<String>,
    pub session_id: Option<String>,
}

fn to_memory(r: &Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: r.get("id")?,
        kind: r.get("type")?,
        title: r.get("title")?,
        claim: r.get("claim")?,
        rationale: r.get("rationale")?,
        scope: r.get("scope")?,
        summary: r.get("summary")?,
        status: r.get("status")?,
        version: r.get("version")?,
        is_hub: r.get::<_, i64>("is_hub")? == 1,
        observed_at: r.get("observed_at")?,
        valid_from: r.get("valid_from")?,
        valid_to: r.get("valid_to")?,
        event_date: r.get("event_date")?,
        commit_sha: r.get("commit_sha")?,
        confidence: r.get("confidence")?,
        salience: r.get("salience")?,
        access_count: r.get("access_count")?,
        last_accessed_at: r.get("last_accessed_at")?,
        authored_by: r.get("authored_by")?,
        session_id: r.get("session_id")?,
    })
}

#[derive(Debug, Default, Clone)]
pub struct NewMemory {
    pub kind: String,
    pub title: String,
    pub claim: String,
    pub rationale: Option<String>,
    pub scope: Option<String>,
    pub summary: Option<String>,
    pub is_hub: bool,
    pub observed_at: Option<String>,
    pub valid_from: Option<String>,
    pub event_date: Option<String>,
    pub commit_sha: Option<String>,
    pub confidence: Option<f64>,
    pub salience: Option<f64>,
    pub authored_by: Option<String>,
    pub session_id: Option<String>,
    /// Raw events that support this memory.
    pub evidence: Vec<i64>,
    /// Entities (files, symbols, ...) this memory is about. Creates `touches` edges.
    pub touches: Vec<String>,
}

impl NewMemory {
    pub fn new(kind: &str, title: &str, claim: &str) -> Self {
        NewMemory { kind: kind.into(), title: title.into(), claim: claim.into(), ..Default::default() }
    }
}

pub fn get_memory(conn: &Connection, id: &str) -> Result<Option<Memory>> {
    Ok(conn.query_row("SELECT * FROM memories WHERE id = ?", [id], to_memory).optional()?)
}

pub fn get_memories(conn: &Connection, ids: &[String]) -> Result<Vec<Memory>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE id IN (SELECT value FROM json_each(?))")?;
    let rows = stmt.query_map([serde_json::to_string(ids)?], to_memory)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn create_memory(conn: &Connection, m: NewMemory) -> Result<Memory> {
    let stamp = now();
    create_memory_at(conn, m, &stamp)
}

pub fn create_memory_at(conn: &Connection, m: NewMemory, at: &str) -> Result<Memory> {
    tx(conn, |conn| insert_memory(conn, &m, at, 1))
}

fn insert_memory(conn: &Connection, m: &NewMemory, at: &str, version: i64) -> Result<Memory> {
    if !TYPES.contains(&m.kind.as_str()) {
        return Err(Error::msg(format!("unknown memory type: {}", m.kind)));
    }
    let id = new_id("mem");
    let observed = m.observed_at.as_deref().unwrap_or(at);
    let valid_from = m.valid_from.as_deref().unwrap_or(observed);
    conn.execute(
        "INSERT INTO memories (id, type, title, claim, rationale, scope, summary, version, is_hub,
           observed_at, valid_from, event_date, commit_sha, confidence, salience, authored_by, session_id, updated_at)
         VALUES (:id, :type, :title, :claim, :rationale, :scope, :summary, :version, :is_hub,
           :observed, :valid_from, :event_date, :commit_sha, :confidence, :salience, :authored_by, :session_id, :at)",
        named_params! {
            ":id": id, ":type": m.kind, ":title": m.title, ":claim": m.claim, ":rationale": m.rationale,
            ":scope": m.scope, ":summary": m.summary, ":version": version, ":is_hub": m.is_hub as i64,
            ":observed": observed, ":valid_from": valid_from, ":event_date": m.event_date,
            ":commit_sha": m.commit_sha, ":confidence": m.confidence.unwrap_or(0.5),
            ":salience": m.salience.unwrap_or(0.5), ":authored_by": m.authored_by,
            ":session_id": m.session_id, ":at": at,
        },
    )?;
    for seq in &m.evidence {
        conn.execute("INSERT OR IGNORE INTO memory_evidence (memory_id, event_seq) VALUES (?, ?)", params![id, seq])?;
    }
    for entity in &m.touches {
        let opts = EdgeOpts { valid_from: Some(valid_from.to_string()), created_by: m.authored_by.clone(), weight: None };
        add_edge(conn, &id, entity, "touches", opts)?;
    }
    get_memory(conn, &id)?.ok_or_else(|| Error::msg("memory vanished after insert"))
}

/// Replace a memory with a newer version. The old one stays as history; nothing is deleted.
pub fn supersede(conn: &Connection, old_id: &str, next: NewMemory) -> Result<Memory> {
    let stamp = now();
    supersede_at(conn, old_id, next, &stamp)
}

pub fn supersede_at(conn: &Connection, old_id: &str, next: NewMemory, at: &str) -> Result<Memory> {
    tx(conn, |conn| {
        let old = get_memory(conn, old_id)?.ok_or_else(|| Error::msg(format!("memory not found: {old_id}")))?;
        if old.status != "current" {
            return Err(Error::msg(format!("memory {old_id} is {}, cannot supersede", old.status)));
        }
        let valid_from = next.valid_from.clone().or_else(|| next.observed_at.clone()).unwrap_or_else(|| at.to_string());
        let created = insert_memory(conn, &NewMemory { valid_from: Some(valid_from.clone()), ..next }, at, old.version + 1)?;
        conn.execute(
            "UPDATE memories SET status = 'superseded', valid_to = ?, updated_at = ? WHERE id = ?",
            params![valid_from, at, old_id],
        )?;
        let opts = EdgeOpts { valid_from: Some(valid_from), created_by: created.authored_by.clone(), weight: None };
        add_edge(conn, &created.id, old_id, "supersedes", opts)?;
        Ok(created)
    })
}

pub fn set_status(conn: &Connection, id: &str, status: &str) -> Result<()> {
    set_status_at(conn, id, status, &now())
}

pub fn set_status_at(conn: &Connection, id: &str, status: &str, at: &str) -> Result<()> {
    if status == "current" {
        conn.execute("UPDATE memories SET status = 'current', valid_to = NULL, updated_at = ? WHERE id = ?", [at, id])?;
    } else {
        conn.execute(
            "UPDATE memories SET status = ?, valid_to = COALESCE(valid_to, ?), updated_at = ? WHERE id = ?",
            [status, at, at, id],
        )?;
    }
    Ok(())
}

/// All versions of a memory, oldest first, following supersedes edges in both directions.
pub fn history(conn: &Connection, id: &str) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE chain(id) AS (
           SELECT ?1 UNION
           SELECT CASE WHEN e.src = c.id THEN e.dst ELSE e.src END
           FROM chain c JOIN edges e ON e.type = 'supersedes' AND (e.src = c.id OR e.dst = c.id)
         )
         SELECT m.* FROM memories m JOIN chain USING (id) ORDER BY m.version",
    )?;
    let rows = stmt.query_map([id], to_memory)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "in", "is", "it", "of", "on", "or", "that",
    "the", "this", "to", "was", "we", "what", "when", "where", "which", "who", "why", "with", "not", "do", "does", "did",
];

#[derive(Debug, Default, Clone)]
pub struct SearchOpts {
    pub limit: Option<i64>,
    pub include_history: bool,
    /// Only memories that were valid at this instant.
    pub as_of: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub memory: Memory,
    /// BM25 turned so that higher is better.
    pub score: f64,
}

fn tokenize(query: &str) -> Vec<String> {
    query.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|s| !s.is_empty()).map(String::from).collect()
}

pub fn search_memories(conn: &Connection, query: &str, opts: &SearchOpts) -> Result<Vec<SearchHit>> {
    let words = tokenize(query);
    let content: Vec<&String> = words.iter().filter(|w| !STOP_WORDS.contains(&w.to_lowercase().as_str())).collect();
    let terms: Vec<&String> = if content.is_empty() { words.iter().collect() } else { content };
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let matcher = terms.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" OR ");
    let mut stmt = conn.prepare(
        "SELECT m.*, bm25(memories_fts, 4.0, 2.0, 1.0, 1.0) AS rank
         FROM memories_fts JOIN memories m ON m.rowid = memories_fts.rowid
         WHERE memories_fts MATCH ?1
           AND (?2 = 1 OR m.status = 'current')
           AND (?3 IS NULL OR (m.valid_from <= ?3 AND (m.valid_to IS NULL OR m.valid_to > ?3)))
         ORDER BY rank LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![matcher, opts.include_history as i64, opts.as_of, opts.limit.unwrap_or(10)],
        |r| Ok(SearchHit { memory: to_memory(r)?, score: -r.get::<_, f64>("rank")? }),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn record_access(conn: &Connection, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE memories SET access_count = access_count + 1, last_accessed_at = ?
         WHERE id IN (SELECT value FROM json_each(?))",
        params![now(), serde_json::to_string(ids)?],
    )?;
    Ok(())
}
