use engram_core::memory::{Memory, get_memories};
use engram_core::{Connection, Result};
use serde::Serialize;

#[derive(Serialize)]
pub struct ModuleSummary {
    pub name: String,
    pub files: i64,
}

#[derive(Serialize)]
pub struct Activity {
    pub actor: String,
    pub kind: String,
    pub ts: String,
}

pub struct Orientation {
    /// Subsystems ranked by how connected they are: the "major nodes".
    pub modules: Vec<ModuleSummary>,
    /// Recent current memories worth knowing at the start of any task.
    pub memories: Vec<Memory>,
    /// Latest activity per agent, so a newly joined agent sees who is doing what.
    pub activity: Vec<Activity>,
}

pub fn orientation(conn: &Connection, memory_limit: i64, module_limit: i64) -> Result<Orientation> {
    let modules = conn
        .prepare(
            "SELECT m.qualified_name, COUNT(*) AS files
             FROM entities m JOIN edges e ON e.src = m.id AND e.type = 'contains'
             WHERE m.kind = 'module' AND m.status = 'active'
             GROUP BY m.id ORDER BY files DESC LIMIT ?",
        )?
        .query_map([module_limit], |r| Ok(ModuleSummary { name: r.get(0)?, files: r.get(1)? }))?
        .collect::<engram_core::rusqlite::Result<_>>()?;

    let ids: Vec<String> = conn
        .prepare(
            "SELECT id FROM memories WHERE status = 'current' AND type != 'episode'
             ORDER BY (type IN ('decision','preference','dead_end')) DESC, observed_at DESC LIMIT ?",
        )?
        .query_map([memory_limit], |r| r.get(0))?
        .collect::<engram_core::rusqlite::Result<_>>()?;

    let activity = conn
        .prepare(
            "SELECT a.name, e.kind, MAX(e.ts) AS ts FROM events e JOIN actors a ON a.id = e.actor_id
             WHERE a.kind = 'agent' GROUP BY a.id ORDER BY ts DESC LIMIT 5",
        )?
        .query_map([], |r| Ok(Activity { actor: r.get(0)?, kind: r.get(1)?, ts: r.get(2)? }))?
        .collect::<engram_core::rusqlite::Result<_>>()?;

    Ok(Orientation { modules, memories: get_memories(conn, &ids)?, activity })
}
