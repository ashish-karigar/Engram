-- Provenance (docs/DATA_MODEL.md, layer 3): commits, who they are attributed to, and which
-- commits wrote the current lines of each file and symbol.
CREATE TABLE commits (
  sha        TEXT PRIMARY KEY,
  ts         TEXT NOT NULL,
  author     TEXT NOT NULL,
  subject    TEXT NOT NULL,
  files      TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(files)),
  -- The agent and session the commit is attributed to, and how sure: 'edited' (the agent edited
  -- these files just before) or 'session' (an agent session was active here just before).
  agent      TEXT,
  session    TEXT,
  evidence   TEXT
);
CREATE INDEX commits_ts ON commits(ts);
-- How many of an entity's current lines each commit wrote (from blame; 'uncommitted' for work in progress).
CREATE TABLE code_history (
  entity_id  TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  lines      INTEGER NOT NULL,
  PRIMARY KEY (entity_id, commit_sha)
);
