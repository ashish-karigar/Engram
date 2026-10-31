-- The system map's raw material, refreshed per file at index time. The map itself (area, endpoint
-- and service entities and their edges) is rebuilt from these tables after each index pass.
CREATE TABLE areas (
  path     TEXT PRIMARY KEY,
  name     TEXT NOT NULL,
  kind     TEXT NOT NULL,
  manifest TEXT NOT NULL
);
CREATE TABLE net_routes (
  file   TEXT NOT NULL,
  symbol TEXT,
  method TEXT NOT NULL,
  path   TEXT NOT NULL,
  prefix INTEGER NOT NULL DEFAULT 0,
  line   INTEGER NOT NULL
);
CREATE INDEX net_routes_file ON net_routes(file);
CREATE TABLE net_calls (
  file   TEXT NOT NULL,
  symbol TEXT,
  method TEXT NOT NULL,
  path   TEXT NOT NULL,
  host   TEXT,
  line   INTEGER NOT NULL
);
CREATE INDEX net_calls_file ON net_calls(file);
-- External services a file uses through an SDK (`import ollama`, `@auth0/auth0-angular`).
CREATE TABLE net_services (
  file    TEXT NOT NULL,
  service TEXT NOT NULL,
  via     TEXT NOT NULL
);
CREATE INDEX net_services_file ON net_services(file);
