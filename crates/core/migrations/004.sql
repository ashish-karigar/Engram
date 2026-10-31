-- The activity timeline reads events by time range.
CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
