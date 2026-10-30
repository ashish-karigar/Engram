use std::collections::HashMap;

use chrono::Utc;
use engram_core::decay::strength;
use engram_core::graph::{ExpandOpts, expand};
use engram_core::memory::{Memory, SearchOpts, get_memories, search_memories};
use engram_core::{Connection, Result};

pub struct RecallItem {
    pub memory: Memory,
    pub score: f64,
    /// Human-readable explanation, stored in the request log.
    pub reason: String,
}

const FTS_SEEDS: i64 = 8;
const GRAPH_WEIGHT: f64 = 0.6;

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
        let rows = stmt.query_map([format!("%{w}%")], |r| r.get::<_, String>(0))?;
        for id in rows {
            ids.push(id?);
        }
    }
    Ok(ids)
}

/// Hybrid recall, no model call:
/// 1. keyword hits (BM25) and name-matched entities are the entry points ("major nodes");
/// 2. expand outward along live edges, scores decaying per hop;
/// 3. rank = relevance × freshness, where freshness comes from decay and usage.
pub fn recall(conn: &Connection, query: &str, opts: &RecallOpts) -> Result<Vec<RecallItem>> {
    let search = SearchOpts { limit: Some(FTS_SEEDS), as_of: opts.as_of.clone(), ..Default::default() };
    let hits = search_memories(conn, query, &search)?;
    let top = hits.first().map(|h| h.score).filter(|s| *s != 0.0).unwrap_or(1.0);
    let mut relevance: HashMap<String, (f64, String)> =
        hits.iter().map(|h| (h.memory.id.clone(), (h.score / top, "keyword match".to_string()))).collect();

    let mut seeds: Vec<String> = hits.iter().map(|h| h.memory.id.clone()).collect();
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
