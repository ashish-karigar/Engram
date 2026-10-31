use engram_core::rusqlite::{OptionalExtension, Row, params};
use engram_core::{Connection, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub external_id: String,
    pub title: String,
    pub source: String,
    pub path: Option<String>,
    pub mime: String,
    pub status: String,
    pub error: Option<String>,
    pub metadata: Value,
    pub char_count: i64,
    pub chunk_count: i64,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn to_document(r: &Row) -> engram_core::rusqlite::Result<Document> {
    let metadata: String = r.get("metadata")?;
    Ok(Document {
        id: r.get("id")?,
        external_id: r.get("external_id")?,
        title: r.get("title")?,
        source: r.get("source")?,
        path: r.get("path")?,
        mime: r.get("mime")?,
        status: r.get("status")?,
        error: r.get("error")?,
        metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
        char_count: r.get("char_count")?,
        chunk_count: r.get("chunk_count")?,
        version: r.get("version")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

pub fn get_document(conn: &Connection, id: &str) -> Result<Option<Document>> {
    Ok(conn.query_row("SELECT * FROM documents WHERE id = ?", [id], to_document).optional()?)
}

pub fn get_by_external_id(conn: &Connection, external_id: &str) -> Result<Option<Document>> {
    Ok(conn.query_row("SELECT * FROM documents WHERE external_id = ?", [external_id], to_document).optional()?)
}

pub fn list_documents(conn: &Connection, source: Option<&str>, limit: i64) -> Result<Vec<Document>> {
    let mut stmt = conn.prepare("SELECT * FROM documents WHERE (?1 IS NULL OR source = ?1) ORDER BY updated_at DESC LIMIT ?2")?;
    let rows = stmt.query_map(params![source, limit], to_document)?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}

pub fn set_status(conn: &Connection, id: &str, status: &str, error: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE documents SET status = ?, error = ?, updated_at = ? WHERE id = ?",
        params![status, error, engram_core::db::now(), id],
    )?;
    Ok(())
}

/// Chunk ids of a document, oldest first.
pub fn chunk_ids(conn: &Connection, document_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM chunks WHERE document_id = ? ORDER BY ordinal")?;
    let rows = stmt.query_map([document_id], |r| r.get(0))?;
    Ok(rows.collect::<engram_core::rusqlite::Result<_>>()?)
}

/// Remove a document with its chunks, vectors and graph edges.
pub fn delete_document(conn: &Connection, id: &str) -> Result<()> {
    engram_core::db::tx(conn, |conn| {
        engram_embed::store::delete_owners(conn, &chunk_ids(conn, id)?)?;
        conn.execute("DELETE FROM edges WHERE src = ?1 OR dst = ?1", [id])?;
        conn.execute("DELETE FROM documents WHERE id = ?", [id])?;
        Ok(())
    })
}
