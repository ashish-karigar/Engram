-- Per-agent API tokens. Only the sha256 is stored; the raw token is shown once.
  CREATE TABLE tokens (
    id         TEXT PRIMARY KEY,
    actor_id   TEXT NOT NULL REFERENCES actors(id),
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    revoked_at TEXT
  );
