//! Suite D: scale and speed. Builds synthetic projects of N chunks (Zipf-distributed words for
//! keyword search, clustered unit vectors for vector search), then measures the real search path:
//! nearest-neighbour recall against exact search, vector and full-search latency, build time,
//! disk and memory. Synthetic because embedding a million real texts on a laptop takes half a day.

use std::path::Path;
use std::time::Instant;

use engram_core::db::open;
use engram_core::events::ensure_actor;
use engram_core::rusqlite::params;
use engram_core::{Connection, Result};
use engram_embed::Embedder;
use engram_embed::ann::top_k;
use engram_embed::store::{exact_top_k, put};
use engram_serve::ops::{Ctx, rank};
use serde_json::json;

use crate::report::percentile;

const DIM: usize = 384;
const MODEL: &str = "synthetic-384";
const CLUSTERS: usize = 1000;
const VOCABULARY: usize = 30_000;
const CHUNKS_PER_DOC: usize = 100;

/// Deterministic generator (no `rand` dependency): 64-bit LCG.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn unit(&mut self) -> f32 {
        (self.next() as f64 / (1u64 << 53) as f64) as f32
    }
}

fn normalize(v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.into_iter().map(|x| x / n).collect()
}

/// Pronounceable pseudo-words, e.g. "takomi", so keyword search has a real vocabulary.
fn word(i: usize) -> String {
    const SYL: [&str; 20] = ["ka", "to", "mi", "re", "su", "na", "lo", "pe", "di", "ru", "ba", "ne", "zo", "fi", "ga", "vu", "sha", "qi", "wo", "le"];
    let (mut i, mut w) = (i, String::new());
    loop {
        w.push_str(SYL[i % SYL.len()]);
        i /= SYL.len();
        if i == 0 {
            return w;
        }
    }
}

/// Skewed word choice: a few words are very common, most are rare (like real text).
fn zipf_word(rng: &mut Rng) -> String {
    word((VOCABULARY as f32 * rng.unit().powf(3.0)) as usize)
}

fn random_unit(rng: &mut Rng) -> Vec<f32> {
    normalize((0..DIM).map(|_| rng.unit() - 0.5).collect())
}

/// Structure like real embeddings: a chunk is near its document, documents are near their topic,
/// and unrelated text is far. (Uniform random vectors have no near neighbours at all, which makes
/// every approximate index look bad and says nothing about real projects.)
fn mix(parts: &[(&[f32], f32)]) -> Vec<f32> {
    normalize((0..DIM).map(|i| parts.iter().map(|(v, w)| v[i] * w).sum()).collect())
}

fn topic(t: usize) -> Vec<f32> {
    random_unit(&mut Rng(1_000_003 + t as u64))
}

fn doc_direction(d: usize) -> Vec<f32> {
    random_unit(&mut Rng(9_000_011 + d as u64))
}

fn chunk_vector(rng: &mut Rng, d: usize) -> Vec<f32> {
    let noise = random_unit(rng);
    mix(&[(&topic(d % CLUSTERS), 1.0), (&doc_direction(d), 0.8), (&noise, 0.6)])
}

fn build(conn: &Connection, n: usize) -> Result<()> {
    let have: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
    if have as usize == n {
        return Ok(());
    }
    let mut rng = Rng(7);
    let stamp = "2026-10-31T00:00:00.000Z";
    conn.execute_batch("BEGIN")?;
    for d in 0..n.div_ceil(CHUNKS_PER_DOC) {
        let doc = format!("doc_{d}");
        conn.execute(
            "INSERT INTO documents (id, external_id, title, source, mime, content_hash, status, created_at, updated_at)
             VALUES (?1, ?1, ?2, 'synthetic', 'text/markdown', ?1, 'done', ?3, ?3)",
            params![doc, format!("{} notes", word(d)), stamp],
        )?;
        for c in 0..CHUNKS_PER_DOC.min(n - d * CHUNKS_PER_DOC) {
            let id = format!("chk_{d}_{c}");
            let text: Vec<String> = (0..60).map(|_| zipf_word(&mut rng)).collect();
            conn.execute(
                "INSERT INTO chunks (id, document_id, ordinal, heading, context, content) VALUES (?, ?, ?, NULL, ?, ?)",
                params![id, doc, c as i64, format!("{} notes", word(d)), text.join(" ")],
            )?;
            put(conn, &id, "chunk", MODEL, &chunk_vector(&mut rng, d))?;
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(())
}

/// A query is about some existing document, phrased differently (its own noise), like a real
/// question about a project.
struct QueryEmbedder {
    docs: usize,
}
impl Embedder for QueryEmbedder {
    fn model_id(&self) -> String {
        MODEL.into()
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let seed = t.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211));
                let mut rng = Rng(seed);
                let d = (rng.next() as usize) % self.docs;
                chunk_vector(&mut rng, d)
            })
            .collect())
    }
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
    out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()).map_or(0.0, |kb| kb / 1024.0)
}

pub fn run(dir: &Path, sizes: &[usize], queries: usize) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    println!("chunks | build s | index build s | recall@10 | similarity vs exact | vector p50 / p95 ms | exact p50 ms | full search p50 / p95 ms | disk MB | RSS MB");
    println!("--- | --- | --- | --- | --- | --- | --- | --- | --- | ---");
    let mut results = vec![];
    for &n in sizes {
        let path = dir.join(format!("scale-{n}.sqlite"));
        let conn = open(&path)?;
        let t = Instant::now();
        build(&conn, n)?;
        let build_s = t.elapsed().as_secs_f64();

        let embedder = QueryEmbedder { docs: n.div_ceil(CHUNKS_PER_DOC) };
        let texts: Vec<String> = (0..queries).map(|i| format!("{} {} {}", word(i * 7 % 900), word(i * 13 % 3000), word(i * 31 % 20_000))).collect();
        let vectors = embedder.embed(&texts)?;

        let t = Instant::now();
        top_k(&conn, MODEL, "chunk", &vectors[0], 10)?;
        let first_s = t.elapsed().as_secs_f64();

        let (mut ann_ms, mut exact_ms, mut overlap, mut quality) = (vec![], vec![], 0usize, vec![]);
        let recall_queries = queries.min(20);
        for (i, v) in vectors.iter().enumerate() {
            let t = Instant::now();
            let approx = top_k(&conn, MODEL, "chunk", v, 10)?;
            ann_ms.push(t.elapsed().as_secs_f64() * 1000.0);
            if i < recall_queries {
                let t = Instant::now();
                let exact = exact_top_k(&conn, MODEL, "chunk", v, 10)?;
                exact_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                overlap += approx.iter().filter(|h| exact.iter().any(|e| e.id == h.id)).count();
                // How good the returned neighbours are, relative to the true best ones (1.0 = as good).
                let mean = |hits: &[engram_embed::store::VectorHit]| hits.iter().map(|h| h.score as f64).sum::<f64>() / hits.len().max(1) as f64;
                quality.push(mean(&approx) / mean(&exact).max(1e-9));
            }
        }
        let recall = overlap as f64 / (recall_queries * 10) as f64;
        let quality = quality.iter().sum::<f64>() / quality.len().max(1) as f64;

        let actor = ensure_actor(&conn, "user", "engram-eval")?;
        let ctx = Ctx { conn: &conn, actor: &actor, embedder: &embedder };
        let mut full_ms = vec![];
        for q in &texts {
            let t = Instant::now();
            rank(&ctx, q, "hybrid", 12)?;
            full_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let disk: u64 = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("scale-{n}.")))
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        let row = json!({
            "chunks": n, "build_s": build_s, "first_query_s": first_s, "ann_recall_at_10": recall, "similarity_vs_exact": quality,
            "vector_p50_ms": percentile(&ann_ms, 0.5), "vector_p95_ms": percentile(&ann_ms, 0.95),
            "exact_p50_ms": percentile(&exact_ms, 0.5), "full_p50_ms": percentile(&full_ms, 0.5), "full_p95_ms": percentile(&full_ms, 0.95),
            "disk_mb": disk as f64 / 1e6, "rss_mb": rss_mb(),
        });
        println!(
            "{n} | {build_s:.0} | {first_s:.1} | {recall:.3} | {quality:.4} | {:.1} / {:.1} | {:.0} | {:.1} / {:.1} | {:.0} | {:.0}",
            percentile(&ann_ms, 0.5), percentile(&ann_ms, 0.95), percentile(&exact_ms, 0.5),
            percentile(&full_ms, 0.5), percentile(&full_ms, 0.95), disk as f64 / 1e6, rss_mb()
        );
        results.push(row);
    }
    std::fs::write(dir.join("scale-results.json"), serde_json::to_string_pretty(&results)?)?;
    Ok(())
}
