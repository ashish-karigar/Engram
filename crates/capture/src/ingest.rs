use std::path::Path;

use engram_core::events::{NewEvent, StoredEvent, append_event};
use engram_core::meta::{get_meta, set_meta};
use engram_core::{Connection, Result};
use serde_json::json;

use crate::git::list_commits;
use crate::scrub::scrub_value;
use crate::snapshot::Snapshot;
use crate::watcher::FileChange;

const LAST_COMMIT_KEY: &str = "capture.last_commit";

pub fn record_file_changes(
    conn: &Connection,
    actor_id: &str,
    changes: &[FileChange],
    commit_sha: Option<String>,
) -> Result<StoredEvent> {
    let files: Vec<_> = changes.iter().map(|c| json!({"path": c.path, "change": c.change.as_str()})).collect();
    append_event(
        conn,
        NewEvent {
            actor_id: Some(actor_id.into()),
            commit_sha,
            ..NewEvent::new("file.change", json!({ "files": files }))
        },
    )
}

pub fn record_snapshot(conn: &Connection, actor_id: &str, snap: &Snapshot) -> Result<StoredEvent> {
    append_event(
        conn,
        NewEvent {
            actor_id: Some(actor_id.into()),
            commit_sha: snap.base.clone(),
            op_id: Some(format!("snapshot:{}", snap.sha)),
            ..NewEvent::new("snapshot.created", json!({"ref": snap.reference, "sha": snap.sha, "base": snap.base}))
        },
    )
}

/// Ingest commits not seen yet. Idempotent: every commit has a stable op id.
pub fn sync_commits(conn: &Connection, root: &Path, actor_id: &str, limit: usize) -> Result<Vec<StoredEvent>> {
    let since = get_meta(conn, LAST_COMMIT_KEY)?;
    let commits = list_commits(root, since.as_deref(), limit);
    let mut events = Vec::with_capacity(commits.len());
    for c in &commits {
        let payload = scrub_value(json!({"sha": c.sha, "author": c.author, "subject": c.subject, "files": c.files}));
        events.push(append_event(
            conn,
            NewEvent {
                actor_id: Some(actor_id.into()),
                commit_sha: Some(c.sha.clone()),
                ts: Some(c.date.clone()),
                op_id: Some(format!("commit:{}", c.sha)),
                ..NewEvent::new("git.commit", payload)
            },
        )?);
    }
    if let Some(last) = commits.last() {
        set_meta(conn, LAST_COMMIT_KEY, &last.sha)?;
    }
    Ok(events)
}
