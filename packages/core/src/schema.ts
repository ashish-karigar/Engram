// Ordered, append-only list of migrations. Never edit a shipped entry; add a new one.
export const MIGRATIONS: readonly string[] = [
  `
  CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
  );

  CREATE TABLE actors (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('agent','user','system')),
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (kind, name)
  );

  CREATE TABLE sessions (
    id           TEXT PRIMARY KEY,
    actor_id     TEXT NOT NULL REFERENCES actors(id),
    external_ref TEXT,
    started_at   TEXT NOT NULL,
    ended_at     TEXT
  );

  -- Raw, immutable record of everything observed. Memory is derived from this and can be rebuilt.
  CREATE TABLE events (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    op_id       TEXT NOT NULL UNIQUE,
    ts          TEXT NOT NULL,
    kind        TEXT NOT NULL,
    actor_id    TEXT REFERENCES actors(id),
    session_id  TEXT REFERENCES sessions(id),
    device_id   TEXT NOT NULL,
    commit_sha  TEXT,
    payload     TEXT NOT NULL CHECK (json_valid(payload)),
    prev_hash   TEXT NOT NULL,
    hash        TEXT NOT NULL
  );
  CREATE INDEX events_kind_ts ON events(kind, ts);
  CREATE INDEX events_session ON events(session_id);
  CREATE TRIGGER events_no_update BEFORE UPDATE ON events
    BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
  CREATE TRIGGER events_no_delete BEFORE DELETE ON events
    BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;

  -- Things memories are about: files, symbols, modules, configs, data flows, dependencies.
  CREATE TABLE entities (
    id               TEXT PRIMARY KEY,
    kind             TEXT NOT NULL,
    qualified_name   TEXT NOT NULL,
    path             TEXT,
    status           TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','deleted')),
    first_seen_commit TEXT,
    last_seen_commit  TEXT,
    renamed_from     TEXT REFERENCES entities(id),
    created_at       TEXT NOT NULL,
    UNIQUE (kind, qualified_name)
  );
  CREATE INDEX entities_path ON entities(path);

  -- The unit that gets recalled. Times are bitemporal: observed_at (when we learned it),
  -- valid_from/valid_to (when it was true), event_date (when the described thing happened).
  CREATE TABLE memories (
    id               TEXT PRIMARY KEY,
    type             TEXT NOT NULL CHECK (type IN
                       ('decision','fact','preference','episode','rejected_option','dead_end','task')),
    title            TEXT NOT NULL,
    claim            TEXT NOT NULL,
    rationale        TEXT,
    scope            TEXT,
    summary          TEXT,
    status           TEXT NOT NULL DEFAULT 'current'
                       CHECK (status IN ('current','superseded','archived','expired')),
    version          INTEGER NOT NULL DEFAULT 1,
    is_hub           INTEGER NOT NULL DEFAULT 0,
    observed_at      TEXT NOT NULL,
    valid_from       TEXT NOT NULL,
    valid_to         TEXT,
    event_date       TEXT,
    commit_sha       TEXT,
    confidence       REAL NOT NULL DEFAULT 0.5,
    salience         REAL NOT NULL DEFAULT 0.5,
    access_count     INTEGER NOT NULL DEFAULT 0,
    last_accessed_at TEXT,
    authored_by      TEXT REFERENCES actors(id),
    session_id       TEXT REFERENCES sessions(id),
    updated_at       TEXT NOT NULL
  );
  CREATE INDEX memories_status_type ON memories(status, type);
  CREATE INDEX memories_hub ON memories(is_hub) WHERE is_hub = 1;

  CREATE TABLE memory_evidence (
    memory_id TEXT NOT NULL REFERENCES memories(id),
    event_seq INTEGER NOT NULL REFERENCES events(seq),
    PRIMARY KEY (memory_id, event_seq)
  );

  -- Graph edges between any two node ids (memories or entities).
  CREATE TABLE edges (
    id         TEXT PRIMARY KEY,
    src        TEXT NOT NULL,
    dst        TEXT NOT NULL,
    type       TEXT NOT NULL,
    weight     REAL NOT NULL DEFAULT 1.0,
    valid_from TEXT NOT NULL,
    valid_to   TEXT,
    created_by TEXT REFERENCES actors(id),
    UNIQUE (src, dst, type)
  );
  CREATE INDEX edges_src ON edges(src, type);
  CREATE INDEX edges_dst ON edges(dst, type);

  -- Hybrid search: keyword now, vectors via memory_embeddings.
  CREATE VIRTUAL TABLE memories_fts USING fts5(
    title, claim, rationale, summary,
    content='memories', content_rowid='rowid'
  );
  CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, title, claim, rationale, summary)
    VALUES (new.rowid, new.title, new.claim, new.rationale, new.summary);
  END;
  CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, claim, rationale, summary)
    VALUES ('delete', old.rowid, old.title, old.claim, old.rationale, old.summary);
  END;
  CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, title, claim, rationale, summary)
    VALUES ('delete', old.rowid, old.title, old.claim, old.rationale, old.summary);
    INSERT INTO memories_fts(rowid, title, claim, rationale, summary)
    VALUES (new.rowid, new.title, new.claim, new.rationale, new.summary);
  END;

  CREATE TABLE memory_embeddings (
    memory_id TEXT NOT NULL REFERENCES memories(id),
    model     TEXT NOT NULL,
    dim       INTEGER NOT NULL,
    vector    BLOB NOT NULL,
    PRIMARY KEY (memory_id, model)
  );

  -- Pointers into git. Code itself lives in git; uncommitted work is saved on refs/engram/*.
  CREATE TABLE snapshots (
    id         TEXT PRIMARY KEY,
    ts         TEXT NOT NULL,
    ref_name   TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    base_sha   TEXT,
    actor_id   TEXT REFERENCES actors(id),
    note       TEXT
  );

  -- Observability: every agent request, what came back, and why.
  CREATE TABLE access_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          TEXT NOT NULL,
    actor_id    TEXT REFERENCES actors(id),
    session_id  TEXT REFERENCES sessions(id),
    tool        TEXT NOT NULL,
    request     TEXT NOT NULL CHECK (json_valid(request)),
    returned    TEXT NOT NULL CHECK (json_valid(returned)),
    tokens_out  INTEGER,
    latency_ms  INTEGER,
    error       TEXT
  );
  CREATE INDEX access_log_ts ON access_log(ts);
  CREATE INDEX access_log_actor ON access_log(actor_id, ts);
  `,
  `
  -- Per-agent API tokens. Only the sha256 is stored; the raw token is shown once.
  CREATE TABLE tokens (
    id         TEXT PRIMARY KEY,
    actor_id   TEXT NOT NULL REFERENCES actors(id),
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    revoked_at TEXT
  );
  `,
];
