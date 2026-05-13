# Technical — Audit log

> The killer feature for retention. Every decision logged with reason. Queryable forever.

## Storage

SQLite at `$XDG_DATA_HOME/homn/audit.db`. Single-file, WAL mode, rusqlite + tokio's `tokio-rusqlite` for async access.

### Why SQLite

- Single file, no daemon, no separate ops.
- FTS5 available for free-text search across `tool_input`.
- Rotation is just "compact + vacuum"; backup is `cp audit.db backup.db` while WAL is checkpointed.
- Used by ctxgraph (same dependency, no new schema-knowledge).

## Schema

```sql
CREATE TABLE decisions (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,               -- unix epoch millis
  session_id    TEXT NOT NULL,
  cwd           TEXT NOT NULL,
  tool_name     TEXT NOT NULL,
  tool_input    TEXT NOT NULL,                  -- json blob, capped 4 KiB
  decision      TEXT NOT NULL CHECK (decision IN ('allow', 'deny', 'ask')),
  human_answer  TEXT,                           -- nullable; set if decision='ask'
  rule_source   TEXT,                           -- nullable; file:line if a rule fired
  rule_text     TEXT,                           -- snapshot for retro-readability
  ctxgraph_hit  TEXT,                           -- json blob, the context that informed the decision
  latency_ms    INTEGER NOT NULL,
  surface       TEXT,                           -- tui|face|ntfy|mcp|hook-direct
  source        TEXT NOT NULL                   -- hook|pty-wrapper|mcp
);

CREATE INDEX idx_decisions_ts        ON decisions(ts);
CREATE INDEX idx_decisions_session   ON decisions(session_id);
CREATE INDEX idx_decisions_tool      ON decisions(tool_name);
CREATE INDEX idx_decisions_decision  ON decisions(decision);

-- FTS5 virtual table over tool_input for free-text queries
CREATE VIRTUAL TABLE decisions_fts USING fts5(
  tool_input, tool_name, cwd,
  content='decisions', content_rowid='id'
);

CREATE TRIGGER decisions_ai AFTER INSERT ON decisions BEGIN
  INSERT INTO decisions_fts(rowid, tool_input, tool_name, cwd)
  VALUES (new.id, new.tool_input, new.tool_name, new.cwd);
END;
```

### `tool_input` capping

`tool_input` JSON is capped at 4 KiB. Truncation:

- For Bash: `command` field is preserved verbatim; `env` and others truncated.
- For Read/Edit/Write: `path` preserved verbatim; `content` truncated with a `[truncated]` marker.
- For WebFetch: `url` preserved; `body` truncated.

The full original payload **is not** preserved — keeping audit small matters for query speed and disk usage. Decisions are about *what was asked*, not *what was sent in full*.

## Retention

- **30 days by default**, configurable in `homn.toml`.
- Daily compaction job runs at 03:30 local time:
  1. Delete rows where `ts < now - retention_days * 86400`.
  2. `VACUUM` if deleted > 10% of rows.
  3. WAL checkpoint.

Retention can be `0` (keep forever) for users who treat audit as historical record. SSD-friendly: even a year of heavy use is on the order of 100 MB.

## Queries

The CLI surface is `homn log`:

```
# tail recent decisions
$ homn log
[14:23:01] allow Bash("git status")               session: cloakpipe-refactor
           rule: policies/default.rhai:24

# only denies in the last hour
$ homn log --denied --since 1h
[14:31:45] deny  WebFetch("https://internal.utkrushta.io/api")
           rule: policies/default.rhai:14 — "deny if url.contains(internal.)"

# anything I had to actually look at
$ homn log --asked
[13:02:11] ask → allow  Bash("git push origin main")     latency: 4.2s
           context: [[wiki/concepts/release-process]]
           surface: face

# free-text search
$ homn log --grep "supabase"

# show what happened in one specific session
$ homn log --session 01HXY...
```

JSON output for scripting: `homn log --json`.

## Through the face

The face's hover panel exposes the same queries graphically — a search box, filter chips, click-to-expand on a decision.

## Through MCP

See [mcp-server.md](mcp-server.md). The `recent_decisions` tool lets the agent introspect what just happened. The `explain_decision` tool lets it understand why.

## Privacy

- The audit DB is the user's local file, mode 0600.
- `tool_input` is **not redacted** in audit (it's the user's own history) — but it *is* redacted before flowing into ctxgraph (see [architecture/brain.md](../architecture/brain.md)).
- Backup / sync: nothing automatic. Users who want cloud backup are explicitly opting in via something like `borgmatic` or `restic` — not our problem.

## Performance targets

- Write latency: <5ms p99 (Tokio + WAL).
- Query latency: <50ms p99 for filtered queries on 90 days of data (~100k rows).
- DB size: ~1 KB per decision; ~30 MB / month at heavy use; well under 1 GB/year.

If query latency degrades, the daily compaction job kicks in. Beyond that, we add bloom filter indexes in v2.
