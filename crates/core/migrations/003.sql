-- Documents: raw input the user or an agent hands Engram (READMEs, specs, PDFs, code files).
CREATE TABLE documents (
  id           TEXT PRIMARY KEY,
  external_id  TEXT NOT NULL UNIQUE,
  title        TEXT NOT NULL,
  source       TEXT NOT NULL,
  path         TEXT,
  mime         TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  status       TEXT NOT NULL CHECK (status IN ('queued','extracting','chunking','embedding','indexing','done','failed')),
  error        TEXT,
  metadata     TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
  char_count   INTEGER NOT NULL DEFAULT 0,
  chunk_count  INTEGER NOT NULL DEFAULT 0,
  version      INTEGER NOT NULL DEFAULT 1,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);
CREATE INDEX documents_path ON documents(path);

-- Chunks: retrieval units with a contextual header (document title and section path).
CREATE TABLE chunks (
  id          TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  ordinal     INTEGER NOT NULL,
  heading     TEXT,
  context     TEXT NOT NULL,
  content     TEXT NOT NULL,
  start_line  INTEGER,
  end_line    INTEGER
);
CREATE INDEX chunks_document ON chunks(document_id, ordinal);

CREATE VIRTUAL TABLE chunks_fts USING fts5(heading, context, content, content='chunks', content_rowid='rowid');
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunks_fts(rowid, heading, context, content) VALUES (new.rowid, new.heading, new.context, new.content);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  INSERT INTO chunks_fts(chunks_fts, rowid, heading, context, content) VALUES ('delete', old.rowid, old.heading, old.context, old.content);
END;

-- Vectors for chunks and memories, keyed by model so providers never mix.
CREATE TABLE embeddings (
  owner_id TEXT NOT NULL,
  kind     TEXT NOT NULL CHECK (kind IN ('chunk','memory')),
  model    TEXT NOT NULL,
  dim      INTEGER NOT NULL,
  vector   BLOB NOT NULL,
  PRIMARY KEY (owner_id, model)
);
CREATE INDEX embeddings_kind ON embeddings(model, kind);

-- Memory model: inferred facts, review, reinforcement, expiry.
ALTER TABLE memories ADD COLUMN is_inference INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN reviewed_at TEXT;
ALTER TABLE memories ADD COLUMN reinforcement INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN expires_at TEXT;
