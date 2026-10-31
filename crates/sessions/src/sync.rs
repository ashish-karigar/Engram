use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use engram_capture::scrub::scrub;
use engram_core::db::{new_id, now};
use engram_core::events::{NewEvent, append_event, ensure_actor};
use engram_core::meta::{get_meta, set_meta};
use engram_core::rusqlite::OptionalExtension;
use engram_core::{Connection, Result};
use engram_embed::Embedder;
use engram_rag::document::{delete_document, get_by_external_id};
use serde_json::json;

use crate::source::{AGENTS, Agent, Sources};
use crate::transcript::rebuild_transcript;
use crate::turn::Turn;
use crate::{claude, codex, muse};

/// Turn text kept in the event log. Long pastes and outputs are truncated, not dropped.
const MAX_TEXT: usize = 4000;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub turns: usize,
    pub compactions: usize,
    pub sessions: Vec<(String, String)>,
}

fn session_row(conn: &Connection, actor_id: &str, external: &str, started: &str) -> Result<String> {
    let found: Option<String> = conn.query_row("SELECT id FROM sessions WHERE external_ref = ?", [external], |r| r.get(0)).optional()?;
    if let Some(id) = found {
        return Ok(id);
    }
    let id = new_id("ses");
    let started = if started.is_empty() { now() } else { started.to_string() };
    conn.execute(
        "INSERT INTO sessions (id, actor_id, external_ref, started_at) VALUES (?, ?, ?, ?)",
        engram_core::rusqlite::params![id, actor_id, external, started],
    )?;
    Ok(id)
}

/// Read only what was appended since the last sync, and only whole lines.
fn read_new(path: &Path, offset: u64) -> std::io::Result<(u64, String)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = if len < offset { 0 } else { offset };
    file.seek(SeekFrom::Start(start))?;
    let mut buf = String::new();
    file.take(len - start).read_to_string(&mut buf)?;
    let complete = buf.rfind('\n').map_or(0, |i| i + 1);
    buf.truncate(complete);
    Ok((start, buf))
}

fn parse(agent: Agent, line: &str, session_id: &mut String) -> Vec<Turn> {
    match agent {
        Agent::Claude => claude::parse_line(line).into_iter().collect(),
        Agent::Codex => {
            if let Some(id) = codex::session_id_of(line) {
                *session_id = id;
            }
            codex::parse_line(line, session_id).into_iter().collect()
        }
        Agent::Muse => muse::records(line).iter().filter_map(muse::parse_record).collect(),
    }
}

fn record(conn: &Connection, agent: Agent, actor_id: &str, turn: &Turn, op_id: String) -> Result<String> {
    let external = format!("{}:{}", agent.id(), turn.session_id);
    let session = session_row(conn, actor_id, &external, &turn.ts)?;
    let text: String = scrub(&turn.text).chars().take(MAX_TEXT).collect();
    let kind = if turn.compaction { "agent.compacted" } else { "agent.turn" };
    append_event(
        conn,
        NewEvent {
            actor_id: Some(actor_id.into()),
            session_id: Some(session),
            op_id: Some(op_id),
            ts: (!turn.ts.is_empty()).then(|| turn.ts.clone()),
            ..NewEvent::new(kind, json!({"agent": agent.actor_name(), "role": turn.role, "text": text, "tools": turn.tools, "session": turn.session_id}))
        },
    )?;
    Ok(external)
}

/// Reviewer sessions synced before they were recognised: the event log is append-only, so record
/// that they are hidden (the timeline drops them) and remove their transcript documents. Once.
fn hide_codex_reviews(conn: &Connection, root: &Path, sources: &Sources) -> Result<()> {
    for file in sources.codex_reviews_for(root) {
        let key = file.to_string_lossy();
        let offset_key = format!("sessions.offset:{key}");
        if get_meta(conn, &offset_key)?.is_none_or(|v| v == "hidden") {
            continue;
        }
        if let Some(session) = get_meta(conn, &format!("sessions.sid:{key}"))? {
            if let Some(doc) = get_by_external_id(conn, &format!("session:codex:{session}"))? {
                delete_document(conn, &doc.id)?;
            }
            append_event(conn, NewEvent::new("agent.session_hidden", json!({"agent": "codex", "session": session, "reason": "Codex approval reviewer"})))?;
        }
        set_meta(conn, &offset_key, "hidden")?;
    }
    Ok(())
}

/// Pull new turns from every agent's logs for sessions run inside `root`, then refresh the
/// transcript document of each session that changed. Safe to call repeatedly: offsets are
/// remembered per file and every turn has a stable op id.
pub fn sync_sessions(conn: &Connection, embedder: &dyn Embedder, root: &Path, sources: &Sources) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    hide_codex_reviews(conn, root, sources)?;
    let mut changed: BTreeSet<(String, String)> = BTreeSet::new();
    for agent in AGENTS {
        let actor_id = ensure_actor(conn, "agent", agent.actor_name())?.id;
        for file in sources.files_for(agent, root) {
            let key = file.to_string_lossy();
            let offset: u64 = get_meta(conn, &format!("sessions.offset:{key}"))?.and_then(|v| v.parse().ok()).unwrap_or(0);
            let Ok((start, text)) = read_new(&file, offset) else { continue };
            if text.is_empty() {
                continue;
            }
            let fallback = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let mut session_id = get_meta(conn, &format!("sessions.sid:{key}"))?.unwrap_or(fallback);
            let mut at = start;
            for line in text.split_inclusive('\n') {
                for (i, turn) in parse(agent, line, &mut session_id).into_iter().enumerate() {
                    let op_id = format!("turn:{}:{}:{at}:{i}", agent.id(), key);
                    let external = record(conn, agent, &actor_id, &turn, op_id)?;
                    changed.insert((agent.id().to_string(), external));
                    if turn.compaction { report.compactions += 1 } else { report.turns += 1 }
                }
                at += line.len() as u64;
            }
            set_meta(conn, &format!("sessions.offset:{key}"), &at.to_string())?;
            set_meta(conn, &format!("sessions.sid:{key}"), &session_id)?;
        }
    }
    for (agent, external) in changed {
        rebuild_transcript(conn, embedder, &external)?;
        report.sessions.push((agent, external));
    }
    Ok(report)
}
