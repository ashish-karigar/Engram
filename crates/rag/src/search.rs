use std::collections::HashMap;

use engram_core::memory::{fts_match, fts_terms};
use engram_core::rusqlite::params;
use engram_core::{Connection, Result};
use engram_embed::Embedder;
use engram_embed::fuse::rrf;
use engram_embed::store::top_k;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkHit {
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub path: Option<String>,
    pub mime: String,
    pub heading: Option<String>,
    pub content: String,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub score: f64,
    /// Which retrievers found it: keyword, semantic, or both.
    pub reason: String,
}

const CANDIDATES: usize = 40;

/// In large projects, words in more than this share of chunks are left out of the keyword query:
/// they barely change the ranking, but ranking every chunk that contains them is what makes a
/// search slow at scale. Small projects keep every word (there a key word can be in many chunks).
const COMMON_WORD_SHARE: f64 = 0.05;
const PRUNE_FROM: f64 = 10_000.0;

fn keyword_ranking(conn: &Connection, query: &str) -> Result<Vec<String>> {
    let terms = fts_terms(query);
    let total: f64 = conn.query_row("SELECT COALESCE(MAX(rowid), 0) FROM chunks", [], |r| r.get(0))?;
    let mut rare = vec![];
    for t in &terms {
        let df: f64 = conn.query_row("SELECT COALESCE(SUM(doc), 0) FROM chunks_vocab WHERE term = lower(?)", [t], |r| r.get(0))?;
        if total < PRUNE_FROM || df <= total * COMMON_WORD_SHARE {
            rare.push(t.clone());
        }
    }
    let Some(matcher) = fts_match(if rare.is_empty() { &terms } else { &rare }) else { return Ok(vec![]) };
    let mut stmt = conn.prepare(
        "SELECT c.id FROM chunks_fts JOIN chunks c ON c.rowid = chunks_fts.rowid
         WHERE chunks_fts MATCH ? ORDER BY bm25(chunks_fts, 3.0, 2.0, 1.0) LIMIT ?",
    )?;
    let rows = stmt.query_map(params![matcher, CANDIDATES as i64], |r| r.get(0))?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}

/// Hybrid document search: BM25 over chunk text and headers, plus vector similarity,
/// fused with Reciprocal Rank Fusion. No model call except embedding the query.
pub fn search_chunks(conn: &Connection, embedder: Option<&dyn Embedder>, query: &str, limit: usize) -> Result<Vec<ChunkHit>> {
    let keyword = keyword_ranking(conn, query)?;
    let semantic: Vec<String> = match embedder {
        Some(e) => {
            match e.embed_query(&[query.to_string()]) {
                Ok(mut v) if !v.is_empty() => {
                    top_k(conn, &e.model_id(), "chunk", &v.remove(0), CANDIDATES)?.into_iter().filter(|h| h.score > 0.05).map(|h| h.id).collect()
                }
                _ => vec![], // embedding server slow or down: keyword results still come back
            }
        }
        None => vec![],
    };
    let fused = rrf(&[keyword.clone(), semantic.clone()], 60.0);
    let mut hits = vec![];
    for (id, score) in fused.into_iter().take(limit) {
        let reason = match (keyword.contains(&id), semantic.contains(&id)) {
            (true, true) => "keyword + semantic",
            (true, false) => "keyword",
            _ => "semantic",
        };
        if let Some(hit) = load_hit(conn, &id, score, reason)? {
            hits.push(hit);
        }
    }
    Ok(hits)
}

fn load_hit(conn: &Connection, id: &str, score: f64, reason: &str) -> Result<Option<ChunkHit>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.document_id, d.title, d.path, c.heading, c.content, c.start_line, c.end_line, d.mime
         FROM chunks c JOIN documents d ON d.id = c.document_id WHERE c.id = ?",
    )?;
    let mut rows = stmt.query([id])?;
    let Some(r) = rows.next()? else { return Ok(None) };
    Ok(Some(ChunkHit {
        chunk_id: r.get(0)?,
        document_id: r.get(1)?,
        title: r.get(2)?,
        path: r.get(3)?,
        mime: r.get(8)?,
        heading: r.get(4)?,
        content: r.get(5)?,
        start_line: r.get(6)?,
        end_line: r.get(7)?,
        score,
        reason: reason.to_string(),
    }))
}

/// Chunk counts per document, used by the UI list.
pub fn chunk_counts(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT document_id, COUNT(*) FROM chunks GROUP BY document_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}
