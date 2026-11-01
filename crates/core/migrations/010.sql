-- Links between memories that a model judged, with its reason. `src` is the memory being judged,
-- `dst` a neighbour it was compared with. Applied links also exist as edges; pending ones wait
-- for review; declined ones are kept so the same pair is never asked about again.
CREATE TABLE link_judgments (
  id         TEXT PRIMARY KEY,
  src        TEXT NOT NULL REFERENCES memories(id),
  dst        TEXT NOT NULL REFERENCES memories(id),
  relation   TEXT NOT NULL CHECK (relation IN ('same','replaces','extends','depends_on','contradicts')),
  reason     TEXT NOT NULL,
  confidence REAL NOT NULL,
  judged_by  TEXT NOT NULL,
  status     TEXT NOT NULL CHECK (status IN ('applied','pending','declined')),
  created_at TEXT NOT NULL,
  decided_at TEXT,
  UNIQUE (src, dst)
);
CREATE INDEX link_judgments_status ON link_judgments(status);

-- Memories a model has compared with their neighbours, so each one is judged once.
CREATE TABLE memories_judged (
  memory_id TEXT PRIMARY KEY REFERENCES memories(id),
  judged_by TEXT NOT NULL,
  judged_at TEXT NOT NULL,
  links     INTEGER NOT NULL
);
