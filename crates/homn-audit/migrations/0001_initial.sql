-- homn-audit migration 0001: initial schema
--
-- See specs/001-policy-engine/data-model.md for the rationale behind each column.
--
-- NOTE: Connection-level PRAGMAs (journal_mode, synchronous, foreign_keys, temp_store) are set
-- by `Db::open` *before* migrations run, because PRAGMA changes can't run inside a transaction.

CREATE TABLE IF NOT EXISTS decisions (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,                                          -- unix epoch millis
  session_id    TEXT    NOT NULL,
  cwd           TEXT    NOT NULL,
  tool_name     TEXT    NOT NULL,
  tool_input    TEXT    NOT NULL,                                          -- JSON, capped 4 KiB
  decision      TEXT    NOT NULL CHECK (decision IN ('allow', 'deny', 'ask')),
  human_answer  TEXT    CHECK (human_answer IN ('allow', 'deny', 'always_allow', 'always_deny')),
  rule_source   TEXT,                                                      -- e.g. "default.rhai:14"
  rule_text     TEXT,                                                      -- snapshot for retro-readability
  ctxgraph_hit  TEXT,                                                      -- JSON; Phase 3+; NULL in Phase 1
  latency_ms    INTEGER NOT NULL,
  surface       TEXT    CHECK (surface IN ('tui', 'face', 'ntfy', 'mcp', 'hook-direct')),
  source        TEXT    NOT NULL CHECK (source IN ('hook', 'pty-wrapper', 'mcp'))
);

CREATE INDEX IF NOT EXISTS idx_decisions_ts        ON decisions(ts);
CREATE INDEX IF NOT EXISTS idx_decisions_session   ON decisions(session_id);
CREATE INDEX IF NOT EXISTS idx_decisions_tool      ON decisions(tool_name);
CREATE INDEX IF NOT EXISTS idx_decisions_decision  ON decisions(decision);

CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(
  tool_input, tool_name, cwd,
  content='decisions',
  content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS decisions_ai AFTER INSERT ON decisions BEGIN
  INSERT INTO decisions_fts(rowid, tool_input, tool_name, cwd)
  VALUES (new.id, new.tool_input, new.tool_name, new.cwd);
END;
