-- homn-audit migration 0003: source watermarks (daemon resume state)
--
-- The crash-safe resume position for each capture source (specs/002-ambient-memory,
-- contracts/gate-pipeline.md R-3). `cursor` is the opaque JSON serialization of a
-- `homn_types::Cursor`; the daemon never interprets it, only persists and hands it back.
-- Advanced ONLY after an item is durably stored or durably dropped by policy, so a crash
-- re-reads from the last confirmed position and dedupe collapses the replay (R7).
--
-- Lives in the audit DB (one sqlite handle) rather than a separate daemon-state file: it is
-- daemon state, not audit data, but co-locating keeps the migration + connection story single.

CREATE TABLE IF NOT EXISTS watermarks (
  source_id  TEXT PRIMARY KEY,
  cursor     TEXT NOT NULL,   -- canonical JSON of homn_types::Cursor
  updated_at TEXT NOT NULL    -- RFC3339 / chrono Utc
);