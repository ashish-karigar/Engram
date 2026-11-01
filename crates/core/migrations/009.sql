-- The system's architecture: which part (area) relies on which service, and for what.
-- Rebuilt from the code with the system map; `evidence` holds "file:line why" strings.
CREATE TABLE system_links (
  area     TEXT NOT NULL,
  service  TEXT NOT NULL,
  -- host, data, auth, secrets, llm, mail, proxy or uses
  role     TEXT NOT NULL,
  -- strongest evidence: code, config, compose (runs alongside the part) or docs
  how      TEXT NOT NULL,
  evidence TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(evidence)),
  PRIMARY KEY (area, service, role)
);
-- Memories the code now contradicts ("backend is hosted on Oracle" after a move to AWS).
CREATE TABLE memory_flags (
  memory_id  TEXT PRIMARY KEY REFERENCES memories(id),
  reason     TEXT NOT NULL,
  flagged_at TEXT NOT NULL
);
