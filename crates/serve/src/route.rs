//! Which project a request on the shared endpoint is about.
//!
//! Agents are connected once, not per project, so each call has to be placed. In order:
//! the folder the agent says it is working in, else the project where that agent was last
//! active (its session logs and notes are attributed to it), else the only project there is.

use std::path::Path;

use engram_core::rusqlite::OptionalExtension;
use engram_core::{Connection, Error, Result};

use crate::project::ProjectRef;

/// Run `f` on a connection to the project's database: its own connection when the project is on
/// disk (so requests never queue behind each other), else the shared one.
pub fn with_conn<T>(project: &ProjectRef, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    match project.db_path.as_deref() {
        Some(path) => f(&engram_core::db::open(path)?),
        None => f(&project.db.lock().unwrap()),
    }
}

/// The deepest watched project whose folder contains `cwd`.
pub fn project_containing<'a>(projects: &'a [ProjectRef], cwd: &Path) -> Option<&'a ProjectRef> {
    projects.iter().filter(|p| cwd.starts_with(&p.info.root)).max_by_key(|p| p.info.root.len())
}

fn last_activity(project: &ProjectRef, actor: &str) -> Option<String> {
    with_conn(project, |conn| {
        Ok(conn
            .query_row(
                "SELECT MAX(e.ts) FROM events e JOIN actors a ON a.id = e.actor_id WHERE a.name = ?",
                [actor],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    })
    .ok()
    .flatten()
}

pub fn pick_project(projects: &[ProjectRef], actor: &str, cwd: Option<&str>) -> Result<ProjectRef> {
    let names = || projects.iter().map(|p| p.info.root.as_str()).collect::<Vec<_>>().join(", ");
    if let Some(cwd) = cwd.filter(|c| !c.trim().is_empty()) {
        // Roots are canonical (e.g. /private/var on macOS), so resolve symlinks in the agent's path too.
        let path = Path::new(cwd).canonicalize().unwrap_or_else(|_| cwd.into());
        return project_containing(projects, &path)
            .cloned()
            .ok_or_else(|| Error::msg(format!("MemoryOS is not watching {cwd}. Watched folders: {}", names())));
    }
    let recent = projects.iter().filter_map(|p| Some((last_activity(p, actor)?, p))).max_by(|a, b| a.0.cmp(&b.0));
    match (recent, projects) {
        (Some((_, p)), _) => Ok(p.clone()),
        (None, [only]) => Ok(only.clone()),
        (None, []) => Err(Error::msg("MemoryOS is not watching any project yet. Add a folder in the MemoryOS app.")),
        (None, _) => Err(Error::msg(format!("Pass `cwd` (the folder you are working in) so MemoryOS can pick the project. Watched folders: {}", names()))),
    }
}
