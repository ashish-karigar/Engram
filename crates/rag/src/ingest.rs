use std::collections::HashSet;

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
use crate::document::{Document, chunk_ids, get_by_external_id, get_document, set_status};
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
    let stored = replace_chunks(conn, &id, &input.title, &chunks)?;

    set_status(conn, &id, "embedding", None)?;
    let texts: Vec<(String, String)> = stored.iter().map(|(cid, context, content)| (cid.clone(), format!("{context}\n{content}"))).collect();
    embed_and_store(conn, embedder, "chunk", &texts)?;

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
fn replace_chunks(conn: &Connection, doc_id: &str, title: &str, chunks: &[Chunk]) -> Result<Vec<(String, String, String)>> {
    tx(conn, |conn| {
        delete_owners(conn, &chunk_ids(conn, doc_id)?)?;
        conn.execute("DELETE FROM chunks WHERE document_id = ?", [doc_id])?;
        let mut stored = Vec::with_capacity(chunks.len());
        for (i, c) in chunks.iter().enumerate() {
            let id = new_id("chk");
            let context = match &c.heading {
                Some(h) => format!("{title} > {h}"),
                None => title.to_string(),
            };
            conn.execute(
                "INSERT INTO chunks (id, document_id, ordinal, heading, context, content, start_line, end_line) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![id, doc_id, i as i64, c.heading, context, c.content, c.start_line as i64, c.end_line as i64],
            )?;
            stored.push((id, context, c.content.clone()));
        }
        Ok(stored)
    })
}

/// Connect the document to code it talks about: `describes` its own file, `mentions` any other
/// project file named in the text (full path, or a distinctive file name like `lidar.py`).
fn link_mentions(conn: &Connection, doc_id: &str, own_path: Option<&str>, text: &str) -> Result<()> {
    conn.execute("DELETE FROM edges WHERE src = ? AND type IN ('mentions','describes')", [doc_id])?;
    let mut stmt = conn.prepare("SELECT id, qualified_name FROM entities WHERE kind = 'file' AND status = 'active'")?;
    let files: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<engram_core::rusqlite::Result<_>>()?;
    let basenames: Vec<&str> = files.iter().map(|(_, p)| p.rsplit('/').next().unwrap_or(p)).collect();
    let mut linked = HashSet::new();
    for ((entity_id, path), base) in files.iter().zip(&basenames) {
        let kind = if Some(path.as_str()) == own_path {
            "describes"
        } else if text.contains(path.as_str()) || (basenames.iter().filter(|b| *b == base).count() == 1 && text.contains(base)) {
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
