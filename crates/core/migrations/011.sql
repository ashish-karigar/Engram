-- Who stated a memory: 'user' when the person told an agent (or wrote it in the app), 'agent' when
-- an agent decided it. NULL for older memories and model extractions, where it is unknown.
ALTER TABLE memories ADD COLUMN stated_by TEXT CHECK (stated_by IN ('user','agent'));
