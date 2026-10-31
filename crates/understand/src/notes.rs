use std::collections::HashSet;

use engram_core::events::{NewEvent, append_event};
use engram_core::memory::NewMemory;
use engram_core::{Connection, Result};
use serde::{Deserialize, Serialize};

use crate::link::link_files;
use crate::reconcile::{SaveResult, save_memory};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoteInput {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub title: String,
    pub claim: String,
    pub rationale: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    pub confidence: Option<f64>,
    /// For temporary facts ("deploy freeze until Friday"): ISO 8601 instant after which it expires.
    #[serde(rename = "expiresAt", alias = "expires_at")]
    pub expires_at: Option<String>,
}

pub struct NoteContext {
    pub actor_id: String,
    pub session_id: Option<String>,
    pub commit_sha: Option<String>,
}

/// An agent's own note is first-hand evidence: log it as an event, then promote it to a memory
/// that cites that event. No model call needed.
pub fn record_note(conn: &Connection, note: &NoteInput, ctx: &NoteContext) -> Result<(SaveResult, i64)> {
    let event = append_event(
        conn,
        NewEvent {
            actor_id: Some(ctx.actor_id.clone()),
            session_id: ctx.session_id.clone(),
            commit_sha: ctx.commit_sha.clone(),
            ..NewEvent::new("agent.note", serde_json::to_value(note)?)
        },
    )?;
    let known: HashSet<String> = note.files.iter().cloned().collect();
    let memory = NewMemory {
        rationale: note.rationale.clone(),
        confidence: Some(note.confidence.unwrap_or(0.8)),
        salience: Some(0.7),
        commit_sha: ctx.commit_sha.clone(),
        authored_by: Some(ctx.actor_id.clone()),
        session_id: ctx.session_id.clone(),
        evidence: vec![event.seq],
        touches: link_files(conn, &note.files, &known)?,
        expires_at: note.expires_at.clone(),
        ..NewMemory::new(note.kind.as_deref().unwrap_or("fact"), &note.title, &note.claim)
    };
    Ok((save_memory(conn, memory)?, event.seq))
}
