use std::collections::{HashMap, HashSet};

use engram_core::db::{new_id, now, tx};
use engram_core::events::{NewEvent, append_event};
use engram_core::graph::{EdgeOpts, add_edge};
use engram_core::rusqlite::params;
use engram_core::{Connection, Result};
use engram_embed::Embedder;
use engram_embed::store::{delete_owners, embed_and_store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::chunk::{Chunk, chunk};
use crate::document::{Document, get_by_external_id, get_document, set_status};
use crate::extract::extract;

pub struct DocumentInput {
    /// Stable identity. Re-ingesting the same id updates the document instead of duplicating it.
    pub external_id: Option<String>,
    pub title: String,
    /// `project` (from the watched folder), `upload`, `agent`, ...
    pub source: String,
    /// Path relative to the project root, when the document is a project file.
    pub path: Option<String>,
    /// Supplies the extension used to pick an extractor, e.g. `notes.md`.
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestAction {
    Created,
    Updated,
    Unchanged,
    Failed,
}

impl IngestAction {
    pub fn as_str(self) -> &'static str {
        match self {
            IngestAction::Created => "created",
            IngestAction::Updated => "updated",
            IngestAction::Unchanged => "unchanged",
            IngestAction::Failed => "failed",
        }
    }
}

pub struct IngestOutcome {
    pub document: Document,
    pub action: IngestAction,
}

/// The pipeline: queued → extracting → chunking → embedding → indexing → done.
/// Unchanged content (same hash) is skipped, so re-ingesting a whole folder is cheap.
pub fn ingest(conn: &Connection, embedder: &dyn Embedder, input: DocumentInput) -> Result<IngestOutcome> {
    let hash = hex::encode(Sha256::digest(&input.bytes));
    let external_id = input.external_id.clone().unwrap_or_else(|| format!("doc:{}", &hash[..16]));
    let existing = get_by_external_id(conn, &external_id)?;
    if let Some(doc) = existing.as_ref().filter(|d| d.status == "done" && content_hash(conn, &d.id).as_deref() == Some(hash.as_str())) {
        return Ok(IngestOutcome { document: doc.clone(), action: IngestAction::Unchanged });
    }

    let (id, action) = match &existing {
        Some(doc) => (doc.id.clone(), IngestAction::Updated),
        None => (new_id("doc"), IngestAction::Created),
    };
    upsert_row(conn, &id, &external_id, &input, &hash, existing.is_some())?;

    set_status(conn, &id, "extracting", None)?;
    let extracted = match extract(&input.file_name, &input.bytes) {
        Ok(e) => e,
        Err(err) => {
            set_status(conn, &id, "failed", Some(&err.to_string()))?;
            let document = get_document(conn, &id)?.expect("document row exists");
            return Ok(IngestOutcome { document, action: IngestAction::Failed });
        }
    };

    set_status(conn, &id, "chunking", None)?;
    let chunks = chunk(&extracted.text, &extracted.kind);
    let (stored, fresh) = replace_chunks(conn, &id, &input.title, &chunks)?;

    set_status(conn, &id, "embedding", None)?;
    embed_and_store(conn, embedder, "chunk", &fresh)?;

    set_status(conn, &id, "indexing", None)?;
    link_mentions(conn, &id, input.path.as_deref(), &extracted.text)?;

    conn.execute(
        "UPDATE documents SET status = 'done', mime = ?, char_count = ?, chunk_count = ?, updated_at = ? WHERE id = ?",
        params![extracted.mime, extracted.text.chars().count() as i64, stored.len() as i64, now(), id],
    )?;
    append_event(
        conn,
        NewEvent::new("document.ingested", json!({"documentId": id, "title": input.title, "action": action.as_str(), "chunks": stored.len()})),
    )?;
    let document = get_document(conn, &id)?.expect("document row exists");
    Ok(IngestOutcome { document, action })
}

fn content_hash(conn: &Connection, id: &str) -> Option<String> {
    conn.query_row("SELECT content_hash FROM documents WHERE id = ?", [id], |r| r.get(0)).ok()
}

fn upsert_row(conn: &Connection, id: &str, external_id: &str, input: &DocumentInput, hash: &str, exists: bool) -> Result<()> {
    let stamp = now();
    if exists {
        conn.execute(
            "UPDATE documents SET title = ?, path = ?, content_hash = ?, status = 'queued', error = NULL,
               metadata = ?, version = version + 1, updated_at = ? WHERE id = ?",
            params![input.title, input.path, hash, input.metadata.to_string(), stamp, id],
        )?;
    } else {
        conn.execute(
            "INSERT INTO documents (id, external_id, title, source, path, mime, content_hash, status, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'application/octet-stream', ?, 'queued', ?, ?, ?)",
            params![id, external_id, input.title, input.source, input.path, hash, input.metadata.to_string(), stamp, stamp],
        )?;
    }
    Ok(())
}

/// Replace a document's chunks atomically. Each chunk gets a contextual header
/// ("title > section") that is indexed and embedded with it.
/// Bring the document's chunks in line with `chunks`. A chunk whose context and text are unchanged
/// keeps its id and vector (only its position is updated), so an appended transcript or a small
/// edit re-embeds only what changed. Returns all chunk ids and the `(id, text)` pairs to embed.
fn replace_chunks(conn: &Connection, doc_id: &str, title: &str, chunks: &[Chunk]) -> Result<(Vec<String>, Vec<(String, String)>)> {
    tx(conn, |conn| {
        // (context, content) -> [(id, ordinal, start_line, end_line)], last-popped = earliest.
        type Placed = (String, i64, Option<i64>, Option<i64>);
        let mut existing: HashMap<(String, String), Vec<Placed>> = HashMap::new();
        let mut stmt = conn.prepare("SELECT id, context, content, ordinal, start_line, end_line FROM chunks WHERE document_id = ? ORDER BY ordinal DESC")?;
        let rows = stmt.query_map([doc_id], |r| Ok(((r.get::<_, String>(1)?, r.get::<_, String>(2)?), (r.get(0)?, r.get(3)?, r.get(4)?, r.get(5)?))))?;
        for row in rows {
            let (key, placed) = row?;
            existing.entry(key).or_default().push(placed);
        }

        let (mut stored, mut fresh) = (Vec::with_capacity(chunks.len()), vec![]);
        for (i, c) in chunks.iter().enumerate() {
            let context = match &c.heading {
                Some(h) => format!("{title} > {h}"),
                None => title.to_string(),
            };
            let kept = existing.get_mut(&(context.clone(), c.content.clone())).and_then(Vec::pop);
            let place = (i as i64, Some(c.start_line as i64), Some(c.end_line as i64));
            let id = match kept {
                Some((id, ordinal, start, end)) => {
                    if (ordinal, start, end) != place {
                        conn.execute("UPDATE chunks SET ordinal = ?, start_line = ?, end_line = ? WHERE id = ?", params![place.0, place.1, place.2, id])?;
                    }
                    id
                }
                None => {
                    let id = new_id("chk");
                    conn.execute(
                        "INSERT INTO chunks (id, document_id, ordinal, heading, context, content, start_line, end_line) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                        params![id, doc_id, i as i64, c.heading, context, c.content, c.start_line as i64, c.end_line as i64],
                    )?;
                    fresh.push((id.clone(), format!("{context}\n{}", c.content)));
                    id
                }
            };
            stored.push(id);
        }
        let gone: Vec<String> = existing.into_values().flatten().map(|(id, ..)| id).collect();
        delete_owners(conn, &gone)?;
        conn.execute("DELETE FROM chunks WHERE id IN (SELECT value FROM json_each(?))", [serde_json::to_string(&gone)?])?;
        Ok((stored, fresh))
    })
}

/// Every path-like token in `text` and each of its trailing sub-paths, so `/repo/src/app.py:12`
/// yields `src/app.py` and `app.py`. One pass over the text instead of one search per file.
fn path_tokens(text: &str) -> HashSet<&str> {
    let mut out = HashSet::new();
    for token in text.split(|c: char| !(c.is_alphanumeric() || "/._-".contains(c))) {
        let token = token.trim_matches(|c: char| c == '.' || c == '/');
        if token.is_empty() {
            continue;
        }
        out.insert(token);
        for (i, _) in token.match_indices('/') {
            out.insert(&token[i + 1..]);
        }
    }
    out
}

/// Connect the document to code it talks about: `describes` its own file, `mentions` any other
/// project file named in the text (full path, or a distinctive file name like `lidar.py`).
fn link_mentions(conn: &Connection, doc_id: &str, own_path: Option<&str>, text: &str) -> Result<()> {
    conn.execute("DELETE FROM edges WHERE src = ? AND type IN ('mentions','describes')", [doc_id])?;
    let mut stmt = conn.prepare("SELECT id, qualified_name FROM entities WHERE kind = 'file' AND status = 'active'")?;
    let files: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let basenames: Vec<&str> = files.iter().map(|(_, p)| p.rsplit('/').next().unwrap_or(p)).collect();
    let mut base_count: HashMap<&str, usize> = HashMap::new();
    for b in &basenames {
        *base_count.entry(b).or_default() += 1;
    }
    let tokens = path_tokens(text);
    let mut linked = HashSet::new();
    for ((entity_id, path), base) in files.iter().zip(&basenames) {
        let kind = if Some(path.as_str()) == own_path {
            "describes"
        } else if tokens.contains(path.as_str()) || (base_count[base] == 1 && tokens.contains(base)) {
            "mentions"
        } else {
            continue;
        };
        if linked.insert(entity_id.clone()) {
            add_edge(conn, doc_id, entity_id, kind, EdgeOpts { weight: Some(0.7), ..Default::default() })?;
        }
    }
    Ok(())
}
