-- homn-learning migration 0001: initial schema
--
-- See specs/001-policy-engine/data-model.md §learning.db for the rationale.
-- Connection-level PRAGMAs (journal_mode, etc.) are set by Db::open before this runs.

CREATE TABLE IF NOT EXISTS pattern_observations (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  pattern_hash    TEXT    NOT NULL,
  pattern_repr    TEXT    NOT NULL,
  tool_name       TEXT    NOT NULL,
  cwd_prefix      TEXT    NOT NULL,
  human_answer    TEXT    NOT NULL CHECK (human_answer IN ('allow', 'deny')),
  decision_id     INTEGER NOT NULL,                                       -- loose ref into audit.db
  ts              INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_obs_hash ON pattern_observations(pattern_hash, ts DESC);

CREATE TABLE IF NOT EXISTS suggestions (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  pattern_hash      TEXT    NOT NULL UNIQUE,
  pattern_repr      TEXT    NOT NULL,
  tool_name         TEXT    NOT NULL,
  proposed_verb     TEXT    NOT NULL CHECK (proposed_verb IN ('allow', 'deny')),
  proposed_rule     TEXT    NOT NULL,
  proposed_file     TEXT    NOT NULL,
  observation_count INTEGER NOT NULL,
  state             TEXT    NOT NULL CHECK (state IN ('open', 'accepted', 'rejected', 'snoozed')),
  state_changed_at  INTEGER NOT NULL,
  expires_at        INTEGER
);

CREATE INDEX IF NOT EXISTS idx_suggestions_state ON suggestions(state, expires_at);
