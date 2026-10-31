use std::collections::HashMap;

use chrono::Utc;
use engram_core::decay::strength;
use engram_core::graph::{ExpandOpts, expand};
use engram_core::memory::{Memory, SearchOpts, get_memories, search_memories};
use engram_core::{Connection, Result};
use engram_embed::Embedder;
use engram_embed::fuse::rrf;
use engram_embed::store::{embed_and_store, memories_missing, top_k};

pub struct RecallItem {
    pub memory: Memory,
    pub score: f64,
    /// Human-readable explanation, stored in the request log.
    pub reason: String,
}

const SEEDS: i64 = 10;
const GRAPH_WEIGHT: f64 = 0.6;
const MIN_COSINE: f32 = 0.15;

#[derive(Default)]
pub struct RecallOpts {
    pub limit: Option<usize>,
    pub depth: Option<i64>,
    pub as_of: Option<String>,
}

/// Entities whose path or name contains a query word act as extra graph entry points.
fn entity_seeds(conn: &Connection, query: &str) -> Result<Vec<String>> {
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.chars().count() >= 3)
        .take(6)
        .map(String::from)
        .collect();
    let mut stmt =
        conn.prepare("SELECT id FROM entities WHERE status = 'active' AND lower(qualified_name) LIKE ? LIMIT 5")?;
    let mut ids = vec![];
    for w in words {
        for id in stmt.query_map([format!("%{w}%")], |r| r.get::<_, String>(0))? {
            ids.push(id?);
        }
    }
    Ok(ids)
}

/// Vector ranking of memories. Memories without a vector are embedded first (cheap, incremental).
fn semantic_ranking(conn: &Connection, embedder: &dyn Embedder, query: &str) -> Result<Vec<String>> {
    let model = embedder.model_id();
    // Background upkeep embeds new memories; at query time only a handful are caught up.
    let missing = memories_missing(conn, &model, 20)?;
    if !missing.is_empty() {
        let _ = embed_and_store(conn, embedder, "memory", &missing);
    }
    let Ok(mut v) = embedder.embed(&[query.to_string()]) else { return Ok(vec![]) };
    if v.is_empty() {
        return Ok(vec![]);
    }
    let q = v.remove(0);
    Ok(top_k(conn, &model, "memory", &q, SEEDS as usize * 2)?.into_iter().filter(|h| h.score >= MIN_COSINE).map(|h| h.id).collect())
}

/// Hybrid recall, no LLM call:
/// 1. keyword (BM25) and vector rankings are fused with RRF; together with name-matched code
///    entities they are the entry points ("major nodes");
/// 2. expand outward along live edges, scores decaying per hop;
/// 3. rank = relevance × freshness, where freshness comes from decay, use and reinforcement.
pub fn recall(conn: &Connection, embedder: Option<&dyn Embedder>, query: &str, opts: &RecallOpts) -> Result<Vec<RecallItem>> {
    let search = SearchOpts { limit: Some(SEEDS), as_of: opts.as_of.clone(), ..Default::default() };
    let keyword: Vec<String> = search_memories(conn, query, &search)?.into_iter().map(|h| h.memory.id).collect();
    let semantic = match embedder {
        Some(e) if opts.as_of.is_none() => semantic_ranking(conn, e, query)?,
        _ => vec![],
    };
    let fused = rrf(&[keyword.clone(), semantic.clone()], 60.0);
    let top = fused.first().map(|f| f.1).unwrap_or(1.0);
    let mut relevance: HashMap<String, (f64, String)> = HashMap::new();
    for (id, score) in fused.iter().take(SEEDS as usize) {
        let reason = match (keyword.contains(id), semantic.contains(id)) {
            (true, true) => "keyword + semantic",
            (true, false) => "keyword",
            _ => "semantic",
        };
        relevance.insert(id.clone(), (score / top, reason.to_string()));
    }

    let mut seeds: Vec<String> = relevance.keys().cloned().collect();
    seeds.extend(entity_seeds(conn, query)?);
    let expand_opts = ExpandOpts { max_depth: opts.depth.unwrap_or(2), limit: 80, ..Default::default() };
    for node in expand(conn, &seeds, &expand_opts)? {
        if !node.id.starts_with("mem_") || relevance.contains_key(&node.id) {
            continue;
        }
        let hops = if node.depth == 1 { "hop" } else { "hops" };
        relevance.insert(node.id, (node.score * GRAPH_WEIGHT, format!("{} {hops} from a match", node.depth)));
    }

    let ids: Vec<String> = relevance.keys().cloned().collect();
    let now = Utc::now();
    let mut items: Vec<RecallItem> = get_memories(conn, &ids)?
        .into_iter()
        .filter(|m| opts.as_of.is_some() || m.status == "current")
        .map(|memory| {
            let (rel, reason) = relevance[&memory.id].clone();
            let fresh = 0.5 + strength(&memory, now).min(1.0) / 2.0;
            RecallItem { memory, score: rel * fresh, reason }
        })
        .collect();
    items.sort_by(|a, b| b.score.total_cmp(&a.score));
    items.truncate(opts.limit.unwrap_or(20));
    Ok(items)
}
