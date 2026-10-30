use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::db::{new_id, now};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub qualified_name: String,
    pub path: Option<String>,
    pub status: String,
}

fn to_entity(r: &Row) -> rusqlite::Result<Entity> {
    Ok(Entity {
        id: r.get("id")?,
        kind: r.get("kind")?,
        qualified_name: r.get("qualified_name")?,
        path: r.get("path")?,
        status: r.get("status")?,
    })
}

#[derive(Debug, Default, Clone)]
pub struct EdgeOpts {
    pub weight: Option<f64>,
    pub valid_from: Option<String>,
    pub created_by: Option<String>,
}

/// Add a typed edge; an identical edge is returned instead of duplicated.
pub fn add_edge(conn: &Connection, src: &str, dst: &str, kind: &str, opts: EdgeOpts) -> Result<String> {
    let existing: Option<String> = conn
        .query_row("SELECT id FROM edges WHERE src = ? AND dst = ? AND type = ?", [src, dst, kind], |r| r.get(0))
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = new_id("edg");
    conn.execute(
        "INSERT INTO edges (id, src, dst, type, weight, valid_from, created_by) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![id, src, dst, kind, opts.weight.unwrap_or(1.0), opts.valid_from.unwrap_or_else(now), opts.created_by],
    )?;
    Ok(id)
}

pub fn upsert_entity(
    conn: &Connection,
    kind: &str,
    qualified_name: &str,
    path: Option<&str>,
    commit_sha: Option<&str>,
) -> Result<Entity> {
    let found = conn
        .query_row("SELECT * FROM entities WHERE kind = ? AND qualified_name = ?", [kind, qualified_name], to_entity)
        .optional()?;
    if let Some(mut entity) = found {
        conn.execute(
            "UPDATE entities SET status = 'active', last_seen_commit = COALESCE(?, last_seen_commit) WHERE id = ?",
            params![commit_sha, entity.id],
        )?;
        entity.status = "active".into();
        return Ok(entity);
    }
    let id = new_id("ent");
    conn.execute(
        "INSERT INTO entities (id, kind, qualified_name, path, first_seen_commit, last_seen_commit, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![id, kind, qualified_name, path, commit_sha, commit_sha, now()],
    )?;
    get_entity(conn, &id)?.ok_or_else(|| Error::msg("entity vanished after insert"))
}

pub fn get_entity(conn: &Connection, id: &str) -> Result<Option<Entity>> {
    Ok(conn.query_row("SELECT * FROM entities WHERE id = ?", [id], to_entity).optional()?)
}

/// Record that an entity moved. Memories that touched the old id follow it to the new one.
pub fn rename_entity(conn: &Connection, old_id: &str, qualified_name: &str, path: Option<&str>) -> Result<Entity> {
    let old = get_entity(conn, old_id)?.ok_or_else(|| Error::msg(format!("entity not found: {old_id}")))?;
    let next = upsert_entity(conn, &old.kind, qualified_name, path, None)?;
    conn.execute("UPDATE entities SET renamed_from = ? WHERE id = ?", [old_id, &next.id])?;
    conn.execute("UPDATE entities SET status = 'deleted' WHERE id = ?", [old_id])?;
    conn.execute("UPDATE edges SET dst = ? WHERE dst = ? AND type = 'touches'", [&next.id, old_id])?;
    Ok(next)
}

#[derive(Debug, Clone, Serialize)]
pub struct Reached {
    pub id: String,
    pub depth: i64,
    /// Falls with each hop and with edge weight. Higher means closer and more relevant.
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct ExpandOpts {
    pub max_depth: i64,
    pub edge_types: Option<Vec<String>>,
    pub decay: f64,
    pub limit: i64,
}

impl Default for ExpandOpts {
    fn default() -> Self {
        ExpandOpts { max_depth: 2, edge_types: None, decay: 0.7, limit: 50 }
    }
}

/// Walk outward from the seeds along live edges (either direction), up to `max_depth` hops.
/// This is the "find major nodes, then explore their neighbours" step of recall.
pub fn expand(conn: &Connection, seeds: &[String], opts: &ExpandOpts) -> Result<Vec<Reached>> {
    if seeds.is_empty() {
        return Ok(vec![]);
    }
    let types = opts.edge_types.as_ref().map(serde_json::to_string).transpose()?;
    let mut stmt = conn.prepare(
        "WITH RECURSIVE walk(id, depth, score) AS (
           SELECT value, 0, 1.0 FROM json_each(?1)
           UNION
           SELECT CASE WHEN e.src = w.id THEN e.dst ELSE e.src END, w.depth + 1, w.score * e.weight * ?3
           FROM walk w JOIN edges e ON (e.src = w.id OR e.dst = w.id)
           WHERE w.depth < ?2 AND e.valid_to IS NULL
             AND (?4 IS NULL OR e.type IN (SELECT value FROM json_each(?4)))
         )
         SELECT id, MIN(depth) AS depth, MAX(score) AS score FROM walk
         GROUP BY id ORDER BY score DESC, depth LIMIT ?5",
    )?;
    let rows = stmt.query_map(
        params![serde_json::to_string(seeds)?, opts.max_depth, opts.decay, types, opts.limit],
        |r| Ok(Reached { id: r.get(0)?, depth: r.get(1)?, score: r.get(2)? }),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
