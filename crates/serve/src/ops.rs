use std::time::Instant;

use engram_core::access::{AccessRecord, log_access};
use engram_core::events::Actor;
use engram_core::graph::{ExpandOpts, expand, get_entity};
use engram_core::memory::{get_memories, get_memory, history, record_access};
use engram_core::{Connection, Error, Result};
use engram_understand::notes::{NoteContext, NoteInput, record_note};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::brief::orientation;
use crate::budget::{Packable, estimate_tokens, pack_by_density};
use crate::format::format_memory;
use crate::recall::{RecallItem, RecallOpts, recall};

/// The four operations every agent gets, whether it connects over MCP or plain HTTP.
pub struct OpResult {
    pub text: String,
    pub data: Value,
}

const DEFAULT_BUDGET: usize = 1200;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReq {
    pub query: String,
    pub budget: Option<usize>,
    pub as_of: Option<String>,
}

#[derive(Default, Deserialize)]
pub struct BriefReq {
    pub task: Option<String>,
    pub budget: Option<usize>,
}

#[derive(Deserialize)]
pub struct ExpandReq {
    pub id: String,
    pub depth: Option<i64>,
}

/// What came back and why, for the request log.
fn returned(items: &[RecallItem]) -> Vec<Value> {
    items.iter().map(|i| json!({"id": i.memory.id, "score": (i.score * 1000.0).round() / 1000.0, "reason": i.reason})).collect()
}

/// Run an operation, time it, and write the request-log entry.
fn logged(
    conn: &Connection,
    actor: &Actor,
    tool: &str,
    request: Value,
    run: impl FnOnce() -> Result<(OpResult, Vec<Value>)>,
) -> Result<OpResult> {
    let started = Instant::now();
    let latency = || Some(started.elapsed().as_millis() as i64);
    let record = |returned: Vec<Value>, tokens: Option<i64>, error: Option<String>| AccessRecord {
        actor_id: Some(actor.id.clone()),
        tool: tool.into(),
        request: request.clone(),
        returned: Value::Array(returned),
        tokens_out: tokens,
        latency_ms: latency(),
        error,
        ..Default::default()
    };
    match run() {
        Ok((result, returned)) => {
            let ids: Vec<String> = returned.iter().filter_map(|r| r["id"].as_str()).filter(|id| id.starts_with("mem_")).map(String::from).collect();
            record_access(conn, &ids)?;
            log_access(conn, record(returned, Some(estimate_tokens(&result.text) as i64), None))?;
            Ok(result)
        }
        Err(err) => {
            log_access(conn, record(vec![], None, Some(err.to_string())))?;
            Err(err)
        }
    }
}

fn pack_items(items: Vec<RecallItem>, budget: usize) -> Vec<RecallItem> {
    let packable = items
        .into_iter()
        .map(|item| Packable { value: item.score, cost: estimate_tokens(&format_memory(&item.memory, Some(&item.reason))), item })
        .collect();
    let (mut packed, _) = pack_by_density(packable, budget);
    packed.sort_by(|a, b| b.score.total_cmp(&a.score));
    packed
}

pub fn op_search(conn: &Connection, actor: &Actor, req: SearchReq) -> Result<OpResult> {
    let request = json!({"query": req.query, "budget": req.budget, "asOf": req.as_of});
    logged(conn, actor, "search", request, || {
        let opts = RecallOpts { as_of: req.as_of.clone(), ..Default::default() };
        let picked = pack_items(recall(conn, &req.query, &opts)?, req.budget.unwrap_or(DEFAULT_BUDGET));
        let text = if picked.is_empty() {
            "No matching memories.".to_string()
        } else {
            picked.iter().map(|i| format_memory(&i.memory, Some(&i.reason))).collect::<Vec<_>>().join("\n")
        };
        let data = Value::Array(returned(&picked));
        Ok((OpResult { text, data }, returned(&picked)))
    })
}

pub fn op_brief(conn: &Connection, actor: &Actor, req: BriefReq) -> Result<OpResult> {
    logged(conn, actor, "brief", json!({"task": req.task, "budget": req.budget}), || {
        let o = orientation(conn, 12, 6)?;
        let relevant = match &req.task {
            Some(task) => recall(conn, task, &RecallOpts { limit: Some(10), ..Default::default() })?,
            None => vec![],
        };
        let seen: Vec<String> = relevant.iter().map(|r| r.memory.id.clone()).collect();
        let boosted = relevant.into_iter().map(|r| RecallItem { score: r.score + 1.0, ..r });
        let background = o.memories.iter().filter(|m| !seen.contains(&m.id)).enumerate().map(|(i, m)| RecallItem {
            memory: m.clone(),
            score: 0.5 / (1.0 + i as f64 * 0.2),
            reason: "recent".into(),
        });
        let picked = pack_items(boosted.chain(background).collect(), req.budget.unwrap_or(DEFAULT_BUDGET));

        let mut lines: Vec<String> = vec![];
        if !o.modules.is_empty() {
            lines.push("Project map:".into());
            lines.extend(o.modules.iter().map(|m| format!("- {} ({} files)", m.name, m.files)));
            lines.push(String::new());
        }
        if !picked.is_empty() {
            lines.push("What to know:".into());
            lines.extend(picked.iter().map(|i| format_memory(&i.memory, Some(&i.reason))));
            lines.push(String::new());
        }
        if !o.activity.is_empty() {
            lines.push("Recent agent activity:".into());
            lines.extend(o.activity.iter().map(|a| format!("- {}: {} at {}", a.actor, a.kind, a.ts)));
        }
        let text = lines.join("\n").trim().to_string();
        let text = if text.is_empty() { "Memory is empty so far.".to_string() } else { text };
        let data = json!({"modules": o.modules, "activity": o.activity, "picked": returned(&picked)});
        Ok((OpResult { text, data }, returned(&picked)))
    })
}

/// Open one node: a memory's full story (reasons, history, evidence) or an entity's connections.
pub fn op_expand(conn: &Connection, actor: &Actor, req: ExpandReq) -> Result<OpResult> {
    logged(conn, actor, "expand", json!({"id": req.id, "depth": req.depth}), || {
        let opts = ExpandOpts { max_depth: req.depth.unwrap_or(1), limit: 40, ..Default::default() };
        let reached: Vec<_> = expand(conn, std::slice::from_ref(&req.id), &opts)?.into_iter().filter(|n| n.id != req.id).collect();
        let memory_ids: Vec<String> = reached.iter().filter(|n| n.id.starts_with("mem_")).map(|n| n.id.clone()).collect();
        let memories = get_memories(conn, &memory_ids)?;
        let mut lines: Vec<String> = vec![];

        if let Some(me) = get_memory(conn, &req.id)? {
            lines.push(format_memory(&me, None));
            let versions = history(conn, &req.id)?;
            if versions.len() > 1 {
                lines.push("History:".into());
                lines.extend(versions.iter().map(|v| format!("  v{} [{}] {}", v.version, v.status, v.claim)));
            }
            let evidence: Vec<String> = conn
                .prepare("SELECT event_seq FROM memory_evidence WHERE memory_id = ?")?
                .query_map([&req.id], |r| Ok(format!("#{}", r.get::<_, i64>(0)?)))?
                .collect::<engram_core::rusqlite::Result<_>>()?;
            if !evidence.is_empty() {
                lines.push(format!("Evidence events: {}", evidence.join(", ")));
            }
        } else {
            let entity = get_entity(conn, &req.id)?.ok_or_else(|| Error::msg(format!("unknown node: {}", req.id)))?;
            lines.push(format!("{} {}", entity.kind, entity.qualified_name));
        }
        let mut code = vec![];
        for n in reached.iter().filter(|n| n.id.starts_with("ent_")) {
            if let Some(e) = get_entity(conn, &n.id)? {
                code.push(format!("- {} {} ({})", e.kind, e.qualified_name, e.id));
            }
        }
        if !code.is_empty() {
            lines.push("Connected code:".into());
            lines.extend(code);
        }
        if !memories.is_empty() {
            lines.push("Connected memories:".into());
            lines.extend(memories.iter().map(|m| format_memory(m, None)));
        }
        let ret = memories.iter().map(|m| json!({"id": m.id, "reason": "connected"})).collect();
        Ok((OpResult { text: lines.join("\n"), data: json!({"reached": reached}) }, ret))
    })
}

pub fn op_note(conn: &Connection, actor: &Actor, note: NoteInput) -> Result<OpResult> {
    logged(conn, actor, "note", serde_json::to_value(&note)?, || {
        let ctx = NoteContext { actor_id: actor.id.clone(), session_id: None, commit_sha: None };
        let (saved, event_seq) = record_note(conn, &note, &ctx)?;
        let text = format!("Saved ({}): {}", saved.action.as_str(), saved.memory.id);
        let data = json!({"action": saved.action.as_str(), "memory": saved.memory, "eventSeq": event_seq});
        Ok((OpResult { text, data }, vec![json!({"id": saved.memory.id, "reason": saved.action.as_str()})]))
    })
}
