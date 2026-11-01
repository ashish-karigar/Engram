use std::time::Instant;

use engram_core::access::{AccessRecord, log_access};
use engram_core::events::Actor;
use engram_core::graph::{ExpandOpts, expand, get_entity};
use engram_core::memory::{get_memories, get_memory, history, record_access};
use engram_core::{Connection, Error, Result};
use engram_embed::{Embedder, Reranker};
use engram_rag::document::get_document;
use engram_rag::{ChunkHit, DocumentInput, ingest, search_chunks};
use engram_understand::notes::{NoteContext, NoteInput, record_note};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::brief::orientation;
use crate::budget::{Packable, estimate_tokens, pack_by_density};
use crate::format::{format_chunk, format_memory};
use crate::profile::build_profile;
use crate::recall::{RecallItem, RecallOpts, recall};
use crate::relevance::{blend, cut, vocabulary};

/// What every operation needs: the project's database, who is asking, and the embedding model.
pub struct Ctx<'a> {
    pub conn: &'a Connection,
    pub actor: &'a Actor,
    pub embedder: &'a dyn Embedder,
    /// Optional second pass over the top results; see `Rerank`.
    pub rerank: Option<Rerank<'a>>,
}

/// Re-score the best `top` results with a cross-encoder, then blend:
/// `weight × reranker + (1 − weight) × first-pass score`. Results below `top` keep only their
/// first-pass share, so they stay behind the re-scored ones.
#[derive(Clone, Copy)]
pub struct Rerank<'a> {
    pub model: &'a dyn Reranker,
    pub weight: f64,
    pub top: usize,
}

pub struct OpResult {
    pub text: String,
    pub data: Value,
}

const DEFAULT_BUDGET: usize = 1200;
/// Document chunks considered per search before ranking and the cutoff.
const CHUNK_CANDIDATES: usize = 12;
/// Snippet length shown per chunk; `format_chunk` trims to the same length.
const SNIPPET_CHARS: usize = 280;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReq {
    pub query: String,
    pub budget: Option<usize>,
    pub as_of: Option<String>,
    /// `hybrid` (default): memories and documents; `memories`; or `documents`.
    pub mode: Option<String>,
}

#[derive(Default, Deserialize)]
pub struct BriefReq {
    pub task: Option<String>,
    pub budget: Option<usize>,
}

#[derive(Default, Deserialize)]
pub struct MapReq {
    /// Only endpoints touching this area or path (e.g. "frontend").
    pub area: Option<String>,
}

#[derive(Deserialize)]
pub struct ImpactReq {
    /// A file path, `file::function`, an endpoint like `PATCH /jobs/{id}`, or a service name.
    pub target: String,
    pub depth: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct FeatureReq {
    /// A concept: `auth`, `email sync`, `payments`.
    pub query: String,
    /// Files listed per part (default 10).
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct ExpandReq {
    pub id: String,
    pub depth: Option<i64>,
}

#[derive(Deserialize)]
pub struct AddDocumentReq {
    pub title: String,
    pub content: String,
    /// Stable id: adding again with the same id updates the document.
    pub id: Option<String>,
}

/// Run an operation, time it, and write the request-log entry with what came back and why.
fn logged(ctx: &Ctx, tool: &str, request: Value, run: impl FnOnce() -> Result<(OpResult, Vec<Value>)>) -> Result<OpResult> {
    let started = Instant::now();
    let record = |returned: Vec<Value>, tokens: Option<i64>, error: Option<String>| AccessRecord {
        actor_id: Some(ctx.actor.id.clone()),
        tool: tool.into(),
        request: request.clone(),
        returned: Value::Array(returned),
        tokens_out: tokens,
        latency_ms: Some(started.elapsed().as_millis() as i64),
        error,
        ..Default::default()
    };
    match run() {
        Ok((result, returned)) => {
            let ids: Vec<String> =
                returned.iter().filter_map(|r| r["id"].as_str()).filter(|id| id.starts_with("mem_")).map(String::from).collect();
            record_access(ctx.conn, &ids)?;
            log_access(ctx.conn, record(returned, Some(estimate_tokens(&result.text) as i64), None))?;
            Ok(result)
        }
        Err(err) => {
            log_access(ctx.conn, record(vec![], None, Some(err.to_string())))?;
            Err(err)
        }
    }
}

/// Something that can go into a context budget.
enum Item {
    Memory(RecallItem),
    Chunk(ChunkHit),
}

impl Item {
    fn line(&self) -> String {
        match self {
            Item::Memory(m) => format_memory(&m.memory, Some(&m.reason)),
            Item::Chunk(c) => format_chunk(c),
        }
    }
    fn returned(&self) -> Value {
        match self {
            Item::Memory(m) => json!({"id": m.memory.id, "score": round(m.score), "reason": m.reason}),
            Item::Chunk(c) => json!({"id": c.chunk_id, "score": round(c.score), "reason": c.reason, "document": c.title}),
        }
    }
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Take results best first while they fit the budget (a result that does not fit is skipped).
fn fit(items: Vec<(Item, f64)>, budget: usize) -> Vec<Item> {
    let mut used = 0;
    items
        .into_iter()
        .filter(|(item, _)| {
            let cost = estimate_tokens(&item.line());
            let fits = used + cost <= budget;
            if fits {
                used += cost;
            }
            fits
        })
        .map(|(item, _)| item)
        .collect()
}

/// Choose what fits the budget by value per token, then present it by value, best first.
fn pack(items: Vec<(Item, f64)>, budget: usize) -> Vec<Item> {
    let packable = items.into_iter().map(|(item, value)| Packable { cost: estimate_tokens(&item.line()), value, item: (item, value) }).collect();
    let mut picked = pack_by_density(packable, budget).0;
    picked.sort_by(|a, b| b.1.total_cmp(&a.1));
    picked.into_iter().map(|(item, _)| item).collect()
}

/// Prose explains why, code shows what, data files (fixtures, eval outputs, lockfiles) rarely
/// answer a question: prefer them in that order when scores are close.
fn source_prior(hit: &ChunkHit) -> f64 {
    const DATA: &[&str] = &[".json", ".jsonl", ".csv", ".tsv", ".yaml", ".yml", ".toml", ".xml", ".lock"];
    let path = hit.path.as_deref().unwrap_or(&hit.title).to_lowercase();
    if DATA.iter().any(|ext| path.ends_with(ext)) {
        0.5
    } else if hit.mime.starts_with("text/x-") {
        0.7
    } else {
        1.0
    }
}

/// One list, best first: memories (`[decision]`, `[fact]`, ...) and document chunks (`[doc]`) mixed.
fn render(items: &[Item]) -> (String, Vec<Value>) {
    (items.iter().map(Item::line).collect::<Vec<_>>().join("\n"), items.iter().map(Item::returned).collect())
}

/// Memories and document chunks on one scale (see `relevance`): each ranker's own score, blended
/// with how much of the query's distinctive vocabulary the item contains; then the optional rerank.
fn gather(ctx: &Ctx, query: &str, mode: &str, as_of: Option<String>, chunk_candidates: usize) -> Result<Vec<(Item, f64)>> {
    let items = first_pass(ctx, query, mode, as_of, chunk_candidates)?;
    match ctx.rerank {
        Some(r) => rerank(r, query, items),
        None => Ok(items),
    }
}

/// What the reranker reads for an item: what it says, not its ids and dates.
fn rerank_text(item: &Item) -> String {
    match item {
        Item::Memory(m) => format!("{}: {}. {}", m.memory.title, m.memory.claim, m.memory.rationale.as_deref().unwrap_or("")),
        Item::Chunk(c) => format!("{}\n{}", c.heading.as_deref().unwrap_or(&c.title), c.content),
    }
}

fn rerank(r: Rerank, query: &str, mut items: Vec<(Item, f64)>) -> Result<Vec<(Item, f64)>> {
    items.sort_by(|a, b| b.1.total_cmp(&a.1));
    let n = r.top.min(items.len());
    let texts: Vec<String> = items[..n].iter().map(|(item, _)| rerank_text(item)).collect();
    let scores = r.model.score(query, &texts)?;
    for (i, (_, score)) in items.iter_mut().enumerate() {
        let cross = scores.get(i).copied().unwrap_or(0.0) as f64;
        *score = r.weight * cross + (1.0 - r.weight) * *score;
    }
    Ok(items)
}

/// Memories and document chunks on one scale, before any reranking.
fn first_pass(ctx: &Ctx, query: &str, mode: &str, as_of: Option<String>, chunk_candidates: usize) -> Result<Vec<(Item, f64)>> {
    let vocab = vocabulary(ctx.conn, query)?;
    let mut items: Vec<(Item, f64)> = vec![];
    if mode != "documents" {
        let opts = RecallOpts { as_of: as_of.clone(), ..Default::default() };
        items.extend(recall(ctx.conn, Some(ctx.embedder), query, &opts)?.into_iter().map(|m| {
            let text = format!("{} {} {}", m.memory.title, m.memory.claim, m.memory.rationale.as_deref().unwrap_or(""));
            let v = blend(vocab.coverage(&text), m.score);
            (Item::Memory(m), v)
        }));
    }
    if mode != "memories" {
        let hits = search_chunks(ctx.conn, Some(ctx.embedder), query, chunk_candidates)?;
        let top = hits.first().map(|h| h.score).unwrap_or(1.0);
        let in_window = dated_documents(ctx.conn, query, as_of.as_deref(), &hits)?;
        items.extend(hits.into_iter().map(|mut h| {
            let prior = source_prior(&h);
            let text = format!("{} {} {}", h.title, h.heading.as_deref().unwrap_or(""), h.content);
            let when = if in_window.contains(&h.document_id) { WHEN_BONUS } else { 0.0 };
            let v = blend(vocab.coverage(&text), prior * h.score / top) + when;
            // The answer is often deep in a chunk: show the part that matches (`expand` has it all).
            h.content = vocab.best_window(&h.content, SNIPPET_CHARS).to_string();
            (Item::Chunk(h), v)
        }));
    }
    Ok(items)
}

/// Added to a chunk whose session falls in the time the question names ("10 days ago").
const WHEN_BONUS: f64 = 0.6;

/// Documents among `hits` dated inside the time window the query names, if it names one.
/// Dated documents are session transcripts (`metadata.started`); "today" is `as_of` or now.
fn dated_documents(conn: &engram_core::Connection, query: &str, as_of: Option<&str>, hits: &[engram_rag::ChunkHit]) -> Result<std::collections::HashSet<String>> {
    let today = as_of
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.date_naive())
        .unwrap_or_else(|| chrono::Utc::now().date_naive());
    let Some(window) = crate::when::window(query, today) else { return Ok(Default::default()) };
    let ids: Vec<&str> = hits.iter().map(|h| h.document_id.as_str()).collect();
    let mut stmt = conn.prepare("SELECT id, json_extract(metadata, '$.started') FROM documents WHERE id IN (SELECT value FROM json_each(?))")?;
    let rows = stmt.query_map([serde_json::to_string(&ids)?], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
    let mut out = std::collections::HashSet::new();
    for row in rows {
        let (id, started) = row?;
        let day = started.as_deref().and_then(|s| s.get(..10)).and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        if day.is_some_and(|d| window.contains(d)) {
            out.insert(id);
        }
    }
    Ok(out)
}

/// One entry of the full ranking behind `search`, before the cutoff and token budget.
pub struct Ranked {
    /// A memory (`mem_…`) or chunk (`chk_…`) id.
    pub id: String,
    /// The chunk's document; None for memories.
    pub document_id: Option<String>,
    pub score: f64,
}

/// The ranking `search` draws from, best first, without the cutoff: for evaluation (recall@K needs
/// a longer list than an agent is shown) and for callers that page through results.
pub fn rank(ctx: &Ctx, query: &str, mode: &str, chunk_candidates: usize) -> Result<Vec<Ranked>> {
    rank_as_of(ctx, query, mode, chunk_candidates, None)
}

/// `rank` as if asked at `as_of` (RFC 3339): what was current then, and "10 days ago" counted from it.
pub fn rank_as_of(ctx: &Ctx, query: &str, mode: &str, chunk_candidates: usize, as_of: Option<String>) -> Result<Vec<Ranked>> {
    let mut items = gather(ctx, query, mode, as_of, chunk_candidates)?;
    items.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(items
        .into_iter()
        .map(|(item, score)| match item {
            Item::Memory(m) => Ranked { id: m.memory.id, document_id: None, score },
            Item::Chunk(c) => Ranked { id: c.chunk_id, document_id: Some(c.document_id), score },
        })
        .collect())
}

pub fn op_search(ctx: &Ctx, req: SearchReq) -> Result<OpResult> {
    let mode = req.mode.clone().unwrap_or_else(|| "hybrid".into());
    let request = json!({"query": req.query, "budget": req.budget, "asOf": req.as_of, "mode": mode});
    logged(ctx, "search", request, || {
        let picked = fit(cut(gather(ctx, &req.query, &mode, req.as_of.clone(), CHUNK_CANDIDATES)?), req.budget.unwrap_or(DEFAULT_BUDGET));
        let (text, returned) = render(&picked);
        let text = if text.is_empty() { "Nothing relevant in memory or documents.".to_string() } else { text };
        Ok((OpResult { text, data: Value::Array(returned.clone()) }, returned))
    })
}

pub fn op_brief(ctx: &Ctx, req: BriefReq) -> Result<OpResult> {
    logged(ctx, "brief", json!({"task": req.task, "budget": req.budget}), || {
        let o = orientation(ctx.conn, 12, 6)?;
        let profile = build_profile(ctx.conn, 6)?;
        let mut items = match &req.task {
            Some(task) => gather(ctx, task, "hybrid", None, CHUNK_CANDIDATES)?.into_iter().map(|(i, v)| (i, v + 1.0)).collect(),
            None => vec![],
        };
        let seen: Vec<String> = items.iter().filter_map(|(i, _)| match i { Item::Memory(m) => Some(m.memory.id.clone()), _ => None }).collect();
        let standing = profile.static_facts.iter().chain(&profile.dynamic).filter(|m| !seen.contains(&m.id));
        items.extend(standing.enumerate().map(|(i, m)| {
            (Item::Memory(RecallItem { memory: m.clone(), score: 0.6 / (1.0 + i as f64 * 0.2), reason: "profile".into() }), 0.6 / (1.0 + i as f64 * 0.2))
        }));
        let picked = pack(items, req.budget.unwrap_or(DEFAULT_BUDGET));
        let (body, returned) = render(&picked);

        let mut lines: Vec<String> = vec![];
        let system = crate::system::system_map(ctx.conn)?;
        if !system.areas.is_empty() {
            // The parts and how they connect; the full endpoint list is one `map` call away.
            lines.push("System map:".into());
            lines.extend(system.areas.iter().map(|(name, kind, path, files)| {
                let relies = system.architecture.get(path).map_or(String::new(), |a| format!(" · {a}"));
                format!("- {name}: {kind}, {files} files{relies}")
            }));
            lines.extend(system.links.iter().map(|(pair, eps)| format!("- {pair} over HTTP ({} endpoints)", eps.len())));
            lines.extend(crate::system::architecture_notes(&system));
            if !system.services.is_empty() {
                lines.push(format!("- external services: {}", system.services.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(", ")));
            }
            lines.push("Use `feature` to see how a concept works (`auth`, `email`) across the parts, `map` for every endpoint, and `impact` before changing shared code.".into());
            lines.push(String::new());
        } else if !o.modules.is_empty() {
            lines.push("Project map:".into());
            lines.extend(o.modules.iter().map(|m| format!("- {} ({} files)", m.name, m.files)));
            lines.push(String::new());
        }
        if !body.is_empty() {
            lines.push(format!("Key memories:\n{body}"));
            lines.push(String::new());
        }
        if !o.activity.is_empty() {
            lines.push("Recent agent activity:".into());
            lines.extend(o.activity.iter().map(|a| format!("- {}: {} at {}", a.actor, a.kind, a.ts)));
        }
        let text = lines.join("\n").trim().to_string();
        let text = if text.is_empty() { "Memory is empty so far.".to_string() } else { text };
        Ok((OpResult { text, data: json!({"modules": o.modules, "activity": o.activity, "picked": returned}) }, returned))
    })
}

pub fn op_map(ctx: &Ctx, req: MapReq) -> Result<OpResult> {
    logged(ctx, "map", json!({"area": req.area}), || {
        let m = crate::system::system_map(ctx.conn)?;
        let text = if m.areas.is_empty() && m.endpoints.is_empty() { "No system map yet: the project has no manifests or HTTP endpoints indexed.".to_string() } else { crate::system::map_text(&m, req.area.as_deref()) };
        Ok((OpResult { text, data: serde_json::to_value(&m)? }, vec![]))
    })
}

pub fn op_impact(ctx: &Ctx, req: ImpactReq) -> Result<OpResult> {
    let request = json!({"target": req.target, "depth": req.depth});
    logged(ctx, "impact", request, || {
        let target = crate::system::resolve_target(ctx.conn, &req.target)?;
        let i = crate::system::impact(ctx.conn, &target, req.depth.unwrap_or(3).clamp(1, 6))?;
        let returned = i.affected.iter().map(|a| json!({"id": a.file, "reason": a.why})).collect();
        Ok((OpResult { text: crate::system::impact_text(&i, req.limit.unwrap_or(40)), data: serde_json::to_value(&i)? }, returned))
    })
}

/// A concept across the code: the files on each side, endpoints, services, config and tests,
/// plus the few memories about it.
pub fn op_feature(ctx: &Ctx, req: FeatureReq) -> Result<OpResult> {
    logged(ctx, "feature", json!({"query": req.query, "limit": req.limit}), || {
        let f = crate::feature::feature(ctx.conn, &req.query)?;
        let mut text = crate::feature::feature_text(&f, req.limit.unwrap_or(10));
        // Only memories that are about the concept, not ones that merely rank near it.
        let memories: Vec<Item> = cut(gather(ctx, &req.query, "memories", None, 0)?)
            .into_iter()
            .map(|(i, _)| i)
            .filter(|i| match i {
                Item::Memory(m) => crate::feature::mentions(&req.query, &format!("{} {}", m.memory.title, m.memory.claim)),
                Item::Chunk(_) => false,
            })
            .take(3)
            .collect();
        let (body, returned) = render(&memories);
        if !body.is_empty() {
            text.push_str(&format!("\nMemories:\n{body}"));
        }
        text.push_str("\nUse `impact` on a file before changing it, and `expand` on a memory for its story.");
        let mut returned = returned;
        returned.extend(f.files.iter().map(|h| json!({"id": h.file, "reason": h.why.join("; ")})));
        Ok((OpResult { text, data: serde_json::to_value(&f)? }, returned))
    })
}

pub fn op_profile(ctx: &Ctx) -> Result<OpResult> {
    logged(ctx, "profile", json!({}), || {
        let p = build_profile(ctx.conn, 8)?;
        let section = |name: &str, ms: &[engram_core::memory::Memory]| {
            if ms.is_empty() { String::new() } else { format!("{name}:\n{}", ms.iter().map(|m| format_memory(m, None)).collect::<Vec<_>>().join("\n")) }
        };
        let text = [section("Standing (holds until replaced)", &p.static_facts), section("Current focus (last 7 days)", &p.dynamic)]
            .into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
        let returned: Vec<Value> = p.static_facts.iter().chain(&p.dynamic).map(|m| json!({"id": m.id, "reason": "profile"})).collect();
        let data = json!({"static": p.static_facts, "dynamic": p.dynamic});
        Ok((OpResult { text: if text.is_empty() { "No profile yet.".into() } else { text }, data }, returned))
    })
}

fn expand_document(ctx: &Ctx, id: &str) -> Result<Vec<String>> {
    if id.starts_with("chk_") {
        let row: (String, String, Option<String>, String, Option<i64>, Option<i64>) = ctx.conn.query_row(
            "SELECT d.title, c.context, d.path, c.content, c.start_line, c.end_line FROM chunks c JOIN documents d ON d.id = c.document_id WHERE c.id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        ).map_err(|_| Error::msg(format!("unknown node: {id}")))?;
        let place = row.2.map(|p| format!(" ({p}:{}-{})", row.4.unwrap_or(0), row.5.unwrap_or(0))).unwrap_or_default();
        return Ok(vec![format!("{}{place}", row.1), row.3]);
    }
    let doc = get_document(ctx.conn, id)?.ok_or_else(|| Error::msg(format!("unknown node: {id}")))?;
    let mut lines = vec![format!("Document {} ({}, {} chunks, v{})", doc.title, doc.mime, doc.chunk_count, doc.version)];
    let mut stmt = ctx.conn.prepare("SELECT id, COALESCE(heading, '(start)') FROM chunks WHERE document_id = ? ORDER BY ordinal")?;
    let sections: Vec<String> = stmt.query_map([id], |r| Ok(format!("- {} ({})", r.get::<_, String>(1)?, r.get::<_, String>(0)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    lines.push("Sections:".into());
    lines.extend(sections);
    Ok(lines)
}

/// Open one node: a memory's full story, a document or chunk, or a code entity's connections.
pub fn op_expand(ctx: &Ctx, req: ExpandReq) -> Result<OpResult> {
    logged(ctx, "expand", json!({"id": req.id, "depth": req.depth}), || {
        let mut lines: Vec<String> = vec![];
        if req.id.starts_with("doc_") || req.id.starts_with("chk_") {
            lines = expand_document(ctx, &req.id)?;
        } else if let Some(me) = get_memory(ctx.conn, &req.id)? {
            lines.push(format_memory(&me, None));
            let versions = history(ctx.conn, &req.id)?;
            if versions.len() > 1 {
                lines.push("History:".into());
                lines.extend(versions.iter().map(|v| format!("  v{} [{}] {}", v.version, v.status, v.claim)));
            }
            let evidence: Vec<String> = ctx.conn
                .prepare("SELECT event_seq FROM memory_evidence WHERE memory_id = ?")?
                .query_map([&req.id], |r| Ok(format!("#{}", r.get::<_, i64>(0)?)))?
                .collect::<engram_core::rusqlite::Result<_>>()?;
            if !evidence.is_empty() {
                lines.push(format!("Evidence events: {}", evidence.join(", ")));
            }
        } else {
            // Code: by id, or by name (`jobs-api.ts::JobsApi.updateStatus`, a path, an endpoint).
            let entity = match get_entity(ctx.conn, &req.id)? {
                Some(e) => e,
                None => {
                    let n = crate::system::resolve_target(ctx.conn, &req.id)?;
                    get_entity(ctx.conn, &n.id)?.ok_or_else(|| Error::msg(format!("unknown node: {}", req.id)))?
                }
            };
            lines.push(format!("{} {}", entity.kind, entity.qualified_name));
            let signature: Option<(String, i64, i64)> = ctx.conn
                .query_row("SELECT signature, start_line, end_line FROM code_symbols WHERE entity_id = ?", [&entity.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .ok();
            if let Some((sig, from, to)) = signature {
                lines.push(format!("{sig}   (lines {from}-{to})"));
            }
            let p = engram_understand::provenance::provenance(ctx.conn, &entity.id)?;
            if let Some(line) = engram_understand::provenance::provenance_line(&p) {
                lines.push(format!("Provenance: {line}"));
            }
            for s in p.sessions.iter().take(5) {
                lines.push(format!("  session: {} {}{}", s.agent, s.session, s.document_id.as_deref().map_or(String::new(), |d| format!(" (transcript {d})"))));
            }
        }

        let reach_from = if req.id.starts_with("chk_") {
            ctx.conn.query_row("SELECT document_id FROM chunks WHERE id = ?", [&req.id], |r| r.get(0)).unwrap_or_else(|_| req.id.clone())
        } else {
            req.id.clone()
        };
        let opts = ExpandOpts { max_depth: req.depth.unwrap_or(1), limit: 40, ..Default::default() };
        let reached: Vec<_> = expand(ctx.conn, std::slice::from_ref(&reach_from), &opts)?.into_iter().filter(|n| n.id != reach_from).collect();
        let memories = get_memories(ctx.conn, &reached.iter().filter(|n| n.id.starts_with("mem_")).map(|n| n.id.clone()).collect::<Vec<_>>())?;
        let mut related = vec![];
        for n in &reached {
            if let Some(e) = get_entity(ctx.conn, &n.id)? {
                related.push(format!("- {} {} ({})", e.kind, e.qualified_name, e.id));
            } else if let Some(d) = get_document(ctx.conn, &n.id)? {
                related.push(format!("- document {} ({})", d.title, d.id));
            }
        }
        if !related.is_empty() {
            lines.push("Connected code and documents:".into());
            lines.extend(related);
        }
        if !memories.is_empty() {
            lines.push("Connected memories:".into());
            lines.extend(memories.iter().map(|m| format_memory(m, None)));
        }
        let ret = memories.iter().map(|m| json!({"id": m.id, "reason": "connected"})).collect();
        Ok((OpResult { text: lines.join("\n"), data: json!({"reached": reached}) }, ret))
    })
}

pub fn op_note(ctx: &Ctx, mut note: NoteInput) -> Result<OpResult> {
    // Written in the app means written by the person.
    if ctx.actor.name == "engram-ui" {
        note.stated_by = Some("user".into());
    }
    logged(ctx, "note", serde_json::to_value(&note)?, || {
        let nctx = NoteContext { actor_id: ctx.actor.id.clone(), session_id: None, commit_sha: None };
        let (saved, event_seq) = record_note(ctx.conn, &note, &nctx)?;
        let extends = saved.extends.as_deref().map(|p| format!(", extends {p}")).unwrap_or_default();
        let text = format!("Saved ({}{extends}): {}", saved.action.as_str(), saved.memory.id);
        let data = json!({"action": saved.action.as_str(), "memory": saved.memory, "eventSeq": event_seq, "extends": saved.extends});
        Ok((OpResult { text, data }, vec![json!({"id": saved.memory.id, "reason": saved.action.as_str()})]))
    })
}

/// Store a document an agent wrote or found (spec, design note, research). It is chunked,
/// embedded and searchable immediately.
pub fn op_add_document(ctx: &Ctx, req: AddDocumentReq) -> Result<OpResult> {
    let request = json!({"title": req.title, "id": req.id, "chars": req.content.chars().count()});
    logged(ctx, "add_document", request, || {
        let name = if req.title.contains('.') { req.title.clone() } else { format!("{}.md", req.title) };
        let input = DocumentInput {
            external_id: req.id.clone().map(|id| format!("agent:{id}")),
            title: req.title.clone(),
            source: "agent".into(),
            path: None,
            file_name: name,
            bytes: req.content.clone().into_bytes(),
            metadata: json!({"addedBy": ctx.actor.name}),
        };
        let out = ingest(ctx.conn, ctx.embedder, input)?;
        let text = format!("Document {} ({}): {} chunks, status {}", out.document.id, out.action.as_str(), out.document.chunk_count, out.document.status);
        let returned = vec![json!({"id": out.document.id, "reason": out.action.as_str()})];
        Ok((OpResult { text, data: serde_json::to_value(&out.document)? }, returned))
    })
}
