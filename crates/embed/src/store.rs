use std::cmp::Ordering;
use std::collections::BinaryHeap;

use engram_core::rusqlite::params;
use engram_core::{Connection, Result};

use crate::embedder::Embedder;

fn encode(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// Vectors are L2-normalized on the way in, so cosine similarity is a dot product.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn put(conn: &Connection, owner_id: &str, kind: &str, model: &str, vector: &[f32]) -> Result<()> {
    conn.execute(
        "INSERT INTO embeddings (owner_id, kind, model, dim, vector) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(owner_id, model) DO UPDATE SET vector = excluded.vector, dim = excluded.dim",
        params![owner_id, kind, model, vector.len() as i64, encode(vector)],
    )?;
    Ok(())
}

pub fn delete_owners(conn: &Connection, owner_ids: &[String]) -> Result<()> {
    conn.execute(
        "DELETE FROM embeddings WHERE owner_id IN (SELECT value FROM json_each(?))",
        [serde_json::to_string(owner_ids)?],
    )?;
    Ok(())
}

/// Embed in batches and store. Returns how many vectors were written.
pub fn embed_and_store(conn: &Connection, embedder: &dyn Embedder, kind: &str, items: &[(String, String)]) -> Result<usize> {
    let model = embedder.model_id();
    for batch in items.chunks(32) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        for ((id, _), vector) in batch.iter().zip(embedder.embed(&texts)?) {
            put(conn, id, kind, &model, &vector)?;
        }
    }
    Ok(items.len())
}

/// Memories that have no vector for this model yet, as (id, text) pairs.
pub fn memories_missing(conn: &Connection, model: &str, limit: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.title || '. ' || m.claim || COALESCE('. ' || m.rationale, '') FROM memories m
         WHERE m.status = 'current' AND NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.owner_id = m.id AND e.model = ?1)
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![model, limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorHit {
    pub id: String,
    pub score: f32,
}

struct MinByScore(VectorHit);
impl PartialEq for MinByScore {
    fn eq(&self, o: &Self) -> bool {
        self.0.score == o.0.score
    }
}
impl Eq for MinByScore {}
impl PartialOrd for MinByScore {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for MinByScore {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.score.total_cmp(&self.0.score)
    }
}

/// Exact top-k by cosine: one pass over the vectors with a size-k min-heap, O(n·d + n log k).
/// Fast enough for tens of thousands of chunks per project; an ANN index can replace it later.
pub fn top_k(conn: &Connection, model: &str, kind: &str, query: &[f32], k: usize) -> Result<Vec<VectorHit>> {
    let mut stmt = conn.prepare("SELECT owner_id, vector FROM embeddings WHERE model = ? AND kind = ?")?;
    let mut rows = stmt.query(params![model, kind])?;
    let mut heap: BinaryHeap<MinByScore> = BinaryHeap::with_capacity(k + 1);
    while let Some(row) = rows.next()? {
        let bytes: Vec<u8> = row.get(1)?;
        let score = dot(query, &decode(&bytes));
        heap.push(MinByScore(VectorHit { id: row.get(0)?, score }));
        if heap.len() > k {
            heap.pop();
        }
    }
    let mut hits: Vec<VectorHit> = heap.into_iter().map(|m| m.0).collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(hits)
}
