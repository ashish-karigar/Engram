-- Symbol details beside each symbol entity (see docs/DATA_MODEL.md, layer 1).
CREATE TABLE code_symbols (
  entity_id  TEXT PRIMARY KEY REFERENCES entities(id),
  file       TEXT NOT NULL,
  name       TEXT NOT NULL,
  kind       TEXT NOT NULL,
  start_line INTEGER NOT NULL,
  end_line   INTEGER NOT NULL,
  -- The class or impl a method belongs to (its qualified name), NULL at top level.
  parent     TEXT,
  signature  TEXT NOT NULL DEFAULT ''
);
CREATE INDEX code_symbols_file ON code_symbols(file);
CREATE INDEX code_symbols_name ON code_symbols(name);
-- Raw call sites per file; resolved into `invokes` edges after each index pass (layer 2).
CREATE TABLE code_calls (
  file     TEXT NOT NULL,
  caller   TEXT,
  callee   TEXT NOT NULL,
  receiver TEXT,
  line     INTEGER NOT NULL
);
CREATE INDEX code_calls_file ON code_calls(file);
