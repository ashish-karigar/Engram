use std::collections::HashSet;

use engram_core::events::{StoredEvent, ensure_actor, list_events};
use engram_core::memory::NewMemory;
use engram_core::meta::{get_meta, set_meta};
use engram_core::{Connection, Result};
use serde::Serialize;

use crate::digest::{DiffProvider, build_digest};
use crate::extract::extract_candidates;
use crate::link::link_files;
use crate::llm::LlmClient;
use crate::reconcile::{SaveAction, save_memory};
use crate::tasks::{Task, group_tasks};
use crate::verify::verify_candidate;

const CURSOR_KEY: &str = "understand.cursor";
const SKIP_KINDS: &[&str] = &["snapshot.created", "agent.note", "document.ingested", "agent.compacted"];

pub struct PipelineOptions<'a> {
    pub llm: &'a dyn LlmClient,
    pub get_diff: Option<DiffProvider<'a>>,
    pub batch_limit: i64,
    pub idle_gap_ms: i64,
}

impl<'a> PipelineOptions<'a> {
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        PipelineOptions { llm, get_diff: None, batch_limit: 500, idle_gap_ms: 10 * 60_000 }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct PipelineReport {
    pub tasks: usize,
    pub created: usize,
    pub superseded: usize,
    pub reinforced: usize,
    pub rejected: Vec<(String, String)>,
    pub error: Option<String>,
}

/// A task is worth a model call only if it carries intent: a commit, several file changes, or a
/// conversation where the user asked and the agent answered in words.
fn is_significant(task: &Task) -> bool {
    let count = |kind: &str| task.events.iter().filter(|e| e.kind == kind).count();
    let said = |role: &str| task.events.iter().any(|e| e.kind == "agent.turn" && e.payload["role"] == role && e.payload["text"].as_str().is_some_and(|t| !t.is_empty()));
    count("git.commit") > 0 || count("file.change") >= 3 || (said("user") && said("assistant"))
}

fn known_paths(events: &[StoredEvent]) -> HashSet<String> {
    let mut paths = HashSet::new();
    for e in events {
        for f in e.payload["files"].as_array().into_iter().flatten() {
            if let Some(path) = f.as_str().or_else(|| f["path"].as_str()) {
                paths.insert(path.to_string());
            }
        }
    }
    paths
}

/// Process events after the cursor. On a model error the cursor stays put, so the task is retried.
pub fn run_pipeline(conn: &Connection, opts: &PipelineOptions) -> Result<PipelineReport> {
    let mut report = PipelineReport::default();
    let cursor: i64 = get_meta(conn, CURSOR_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(0);
    let events: Vec<_> = list_events(conn, cursor, None, opts.batch_limit)?
        .into_iter()
        .filter(|e| !SKIP_KINDS.contains(&e.kind.as_str()))
        .collect();
    let author = ensure_actor(conn, "system", "engram-extractor")?.id;

    for task in group_tasks(&events, opts.idle_gap_ms) {
        let last = task.events.last().map_or(cursor, |e| e.seq);
        if is_significant(&task) {
            if let Err(err) = process_task(conn, &task, opts, &author, &mut report) {
                report.error = Some(err.to_string());
                return Ok(report);
            }
            report.tasks += 1;
        }
        set_meta(conn, CURSOR_KEY, &last.to_string())?;
    }
    Ok(report)
}

fn process_task(conn: &Connection, task: &Task, opts: &PipelineOptions, author: &str, report: &mut PipelineReport) -> Result<()> {
    let digest = build_digest(&task.events, opts.get_diff);
    let paths = known_paths(&task.events);
    let commit_sha = task.events.iter().rev().find_map(|e| e.commit_sha.clone());
    for c in extract_candidates(opts.llm, &digest.text)? {
        let grounding = match verify_candidate(&c, &digest.by_seq) {
            Ok(g) => g,
            Err(reason) => {
                report.rejected.push((c.title, reason.to_string()));
                continue;
            }
        };
        let confidence = if grounding.exact { c.confidence } else { c.confidence * 0.8 };
        let memory = NewMemory {
            rationale: c.rationale.clone(),
            confidence: Some(confidence),
            salience: Some(0.4 + 0.4 * confidence),
            observed_at: Some(task.ended_at.clone()),
            commit_sha: commit_sha.clone(),
            authored_by: Some(author.to_string()),
            evidence: grounding.evidence.clone(),
            touches: link_files(conn, &c.files, &paths)?,
            ..NewMemory::new(&c.kind, &c.title, &c.claim)
        };
        match save_memory(conn, memory)?.action {
            SaveAction::Created => report.created += 1,
            SaveAction::Superseded => report.superseded += 1,
            SaveAction::Reinforced => report.reinforced += 1,
        }
    }
    Ok(())
}
