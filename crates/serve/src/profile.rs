use engram_core::memory::Memory;
use engram_core::rusqlite::params;
use engram_core::{Connection, Result};

/// A standing picture of the project, attached to every brief without a search:
/// static = conventions and decisions that hold until replaced; dynamic = what is happening now.
pub struct Profile {
    pub static_facts: Vec<Memory>,
    pub dynamic: Vec<Memory>,
}

const RECENT_DAYS: i64 = 7;

fn ids(conn: &Connection, sql: &str, params: impl engram_core::rusqlite::Params) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |r| r.get(0))?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}

/// The architecture facts the system keeps itself (scope `system:`) are left out: `brief` shows
/// them as the system map, and here they would crowd out what people decided.
pub fn build_profile(conn: &Connection, limit: i64) -> Result<Profile> {
    let static_ids = ids(
        conn,
        "SELECT id FROM memories WHERE status = 'current' AND COALESCE(scope, '') NOT LIKE 'system:%' AND is_inference = 0
           AND type IN ('preference','decision','fact')
         ORDER BY is_hub DESC, reinforcement DESC, (type = 'preference') DESC, salience DESC, observed_at DESC LIMIT ?1",
        params![limit],
    )?;
    let dynamic_ids = ids(
        conn,
        "SELECT id FROM memories WHERE status = 'current' AND COALESCE(scope, '') NOT LIKE 'system:%'
           AND observed_at >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2)
         ORDER BY observed_at DESC LIMIT ?1",
        params![limit * 2, format!("-{RECENT_DAYS} days")],
    )?;
    let load = |ids: &[String]| -> Result<Vec<Memory>> {
        let mut all = engram_core::memory::get_memories(conn, ids)?;
        all.sort_by_key(|m| ids.iter().position(|i| *i == m.id));
        Ok(all)
    };
    let static_facts = load(&static_ids)?;
    let dynamic = load(&dynamic_ids)?.into_iter().filter(|m| !static_ids.contains(&m.id)).take(limit as usize).collect();
    Ok(Profile { static_facts, dynamic })
}
