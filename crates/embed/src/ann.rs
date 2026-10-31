//! Approximate nearest-neighbour search for large projects: an HNSW graph (usearch) per project,
//! embedding model and kind, kept next to the project's database.
//!
//! - Below `EXACT_BELOW` vectors, search stays exact (milliseconds, always correct).
//! - Index keys are `embeddings.rowid`. A vector that changes is replaced, so it gets a new rowid;
//!   the index catches up by adding rows above its high-water mark (no full rebuild on writes).
//! - Rows deleted from the table are filtered out at query time and pruned from the graph; the
//!   graph is rebuilt when stale entries pass a quarter of it.
//! - The graph is saved to disk after large catch-ups and reloaded on the next start.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use engram_core::meta::{get_meta, set_meta};
use engram_core::rusqlite::params;
use engram_core::{Connection, Error, Result};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::store::{VectorHit, decode, exact_top_k};

/// Below this many vectors an exact scan is fast enough and is used instead.
pub const EXACT_BELOW: usize = 5_000;
/// Save the graph after this many additions since the last save.
const SAVE_EVERY: usize = 5_000;

struct Ann {
    index: Index,
    /// Highest `embeddings.rowid` already in the graph.
    high_water: i64,
    unsaved: usize,
    stale: usize,
    file: PathBuf,
}

type Key = (String, String, String);
static GRAPHS: Mutex<Option<HashMap<Key, Arc<Mutex<Ann>>>>> = Mutex::new(None);

fn err(e: impl std::fmt::Display) -> Error {
    Error::msg(format!("vector index: {e}"))
}

/// 32 links per node and a wide search: projects cluster tightly (a document's chunks sit close
/// together), and with fewer links the search can stall in a neighbouring cluster at a million vectors.
fn options(dims: usize) -> IndexOptions {
    IndexOptions {
        dimensions: dims,
        metric: MetricKind::Cos,
        // Half precision halves memory; the ranking change is far below what search notices.
        quantization: ScalarKind::F16,
        connectivity: 32,
        expansion_add: 200,
        expansion_search: 256,
        multi: false,
    }
}

fn meta_key(model: &str, kind: &str) -> String {
    format!("ann.high_water:{model}:{kind}")
}

impl Ann {
    fn open(conn: &Connection, file: PathBuf, model: &str, kind: &str, dims: usize) -> Result<Ann> {
        let index = Index::new(&options(dims)).map_err(err)?;
        let saved: Option<i64> = get_meta(conn, &meta_key(model, kind))?.and_then(|v| v.parse().ok());
        let high_water = match saved {
            Some(hw) if file.exists() && index.load(&file.to_string_lossy()).is_ok() => hw,
            _ => 0,
        };
        let mut ann = Ann { index, high_water, unsaved: 0, stale: 0, file };
        if high_water > 0 {
            // Entries for rows deleted while we were not running.
            let live: i64 = conn.query_row(
                "SELECT COUNT(*) FROM embeddings WHERE model = ? AND kind = ? AND rowid <= ?",
                params![model, kind, high_water],
                |r| r.get(0),
            )?;
            ann.stale = ann.index.size().saturating_sub(live as usize);
            if ann.stale * 4 > ann.index.size() {
                ann = Ann { index: Index::new(&options(dims)).map_err(err)?, high_water: 0, unsaved: 0, stale: 0, file: ann.file };
            }
        }
        Ok(ann)
    }

    /// Add every row newer than the high-water mark.
    fn catch_up(&mut self, conn: &Connection, model: &str, kind: &str) -> Result<()> {
        // `+model`/`+kind` keep SQLite on the rowid range (new rows only) instead of walking the
        // (model, kind) index over every vector on each search.
        let mut stmt = conn.prepare("SELECT rowid, vector FROM embeddings WHERE +model = ? AND +kind = ? AND rowid > ? ORDER BY rowid")?;
        let rows: Vec<(i64, Vec<u8>)> = stmt.query_map(params![model, kind, self.high_water], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
        if rows.is_empty() {
            return Ok(());
        }
        // Graph insertion is the expensive part; spread it over the cores.
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        self.index.reserve_capacity_and_threads(self.index.size() + rows.len(), threads).map_err(err)?;
        let index = &self.index;
        let failed = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            for part in rows.chunks(rows.len().div_ceil(threads)) {
                let failed = &failed;
                s.spawn(move || {
                    for (rowid, bytes) in part {
                        if index.add(*rowid as u64, &decode(bytes)).is_err() {
                            failed.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            }
        });
        if failed.into_inner() {
            return Err(err("adding vectors failed"));
        }
        self.high_water = rows.last().map_or(self.high_water, |r| r.0);
        self.unsaved += rows.len();
        if self.unsaved >= SAVE_EVERY {
            self.save(conn, model, kind)?;
        }
        Ok(())
    }

    fn save(&mut self, conn: &Connection, model: &str, kind: &str) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        self.index.save(&self.file.to_string_lossy()).map_err(err)?;
        set_meta(conn, &meta_key(model, kind), &self.high_water.to_string())?;
        self.unsaved = 0;
        Ok(())
    }

    fn search(&mut self, conn: &Connection, model: &str, query: &[f32], k: usize) -> Result<Vec<VectorHit>> {
        let mut want = k + 8;
        loop {
            let found = self.index.search(query, want).map_err(err)?;
            let keys: Vec<i64> = found.keys.iter().map(|k| *k as i64).collect();
            // Look the hits up by rowid; `+model` stops SQLite walking every vector of the model.
            let mut stmt = conn.prepare("SELECT rowid, owner_id FROM embeddings WHERE +model = ? AND rowid IN (SELECT value FROM json_each(?))")?;
            let owners: HashMap<i64, String> = stmt
                .query_map(params![model, serde_json::to_string(&keys)?], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<engram_core::rusqlite::Result<_>>()?;
            let mut hits = vec![];
            for (key, distance) in found.keys.iter().zip(&found.distances) {
                match owners.get(&(*key as i64)) {
                    Some(owner) => hits.push(VectorHit { id: owner.clone(), score: 1.0 - distance }),
                    None => {
                        // Deleted or replaced since it was added.
                        let _ = self.index.remove(*key);
                        self.stale += 1;
                    }
                }
            }
            if hits.len() >= k || found.keys.len() < want || want >= k * 8 {
                hits.truncate(k);
                return Ok(hits);
            }
            want *= 2;
        }
    }
}

/// Top-k by cosine: exact for small sets, HNSW for large on-disk projects.
pub fn top_k(conn: &Connection, model: &str, kind: &str, query: &[f32], k: usize) -> Result<Vec<VectorHit>> {
    top_k_with(conn, model, kind, query, k, EXACT_BELOW)
}

/// `top_k` with an explicit threshold (tests and the scale benchmark use small ones).
pub fn top_k_with(conn: &Connection, model: &str, kind: &str, query: &[f32], k: usize, exact_below: usize) -> Result<Vec<VectorHit>> {
    let Some(db) = conn.path().filter(|p| !p.is_empty()).map(PathBuf::from) else {
        return exact_top_k(conn, model, kind, query, k);
    };
    let key: Key = (db.to_string_lossy().into(), model.into(), kind.into());
    let existing = GRAPHS.lock().unwrap().get_or_insert_with(HashMap::new).get(&key).cloned();
    let graph = match existing {
        Some(g) => g,
        None => {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings WHERE model = ? AND kind = ?", params![model, kind], |r| r.get(0))?;
            if (count as usize) < exact_below {
                return exact_top_k(conn, model, kind, query, k);
            }
            let safe: String = model.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
            let stem = db.file_stem().map_or("engram".into(), |s| s.to_string_lossy().to_string());
            let file = db.with_file_name(format!("{stem}.vectors-{safe}-{kind}.usearch"));
            let ann = Arc::new(Mutex::new(Ann::open(conn, file, model, kind, query.len())?));
            GRAPHS.lock().unwrap().get_or_insert_with(HashMap::new).entry(key).or_insert(ann).clone()
        }
    };
    let mut ann = graph.lock().unwrap_or_else(|e| e.into_inner());
    ann.catch_up(conn, model, kind)?;
    if ann.stale * 4 > ann.index.size().max(1) {
        // Too many dead entries: start over from the table.
        let file = ann.file.clone();
        *ann = Ann { index: Index::new(&options(query.len())).map_err(err)?, high_water: 0, unsaved: 0, stale: 0, file };
        ann.catch_up(conn, model, kind)?;
        ann.save(conn, model, kind)?;
    }
    ann.search(conn, model, query, k)
}

/// Write every loaded graph to disk (on shutdown). Errors are ignored: the next start catches up.
pub fn save_all(conn_for: impl Fn(&str) -> Option<Connection>) {
    let graphs: Vec<(Key, Arc<Mutex<Ann>>)> = GRAPHS.lock().unwrap().get_or_insert_with(HashMap::new).iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for ((db, model, kind), graph) in graphs {
        let mut ann = graph.lock().unwrap_or_else(|e| e.into_inner());
        if ann.unsaved > 0 {
            if let Some(conn) = conn_for(&db) {
                let _ = ann.save(&conn, &model, &kind);
            }
        }
    }
}
