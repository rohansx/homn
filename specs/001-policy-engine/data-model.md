# Data Model — Policy Engine (Phase 1)

> Concrete schemas for `audit.db`, `learning.db`, and the in-memory types passed across crate boundaries. The long-form rationale is in [`docs/technical/audit-log.md`](../../docs/technical/audit-log.md).

## SQLite — `$XDG_DATA_HOME/homn/audit.db`

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA temp_store = MEMORY;

CREATE TABLE IF NOT EXISTS decisions (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,                                          -- unix epoch millis
  session_id    TEXT    NOT NULL,
  cwd           TEXT    NOT NULL,
  tool_name     TEXT    NOT NULL,
  tool_input    TEXT    NOT NULL,                                          -- JSON, capped 4 KiB
  decision      TEXT    NOT NULL CHECK (decision IN ('allow', 'deny', 'ask')),
  human_answer  TEXT    CHECK (human_answer IN ('allow', 'deny', 'always_allow', 'always_deny')),
  rule_source   TEXT,                                                      -- e.g. "policies/default.rhai:14"
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

CREATE TABLE IF NOT EXISTS schema_version (
  version INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);
INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, strftime('%s', 'now') * 1000);
```

### Migrations

Migrations are forward-only and live in `homn-audit/migrations/`. Schema version is bumped per migration. On startup the daemon applies all unmet migrations inside a transaction; a failed migration aborts startup with a clear error pointing at the migration file.

### Truncation rules for `tool_input`

The full payload is capped at 4 KiB. Truncation per tool:

- **Bash**: preserve `command`; truncate `env` to first 256 chars; preserve `cwd`.
- **Read / Edit / Write**: preserve `path`; truncate `content` to first 1 KiB + `[truncated, full size: N]` marker.
- **WebFetch**: preserve `url`; truncate body fields.
- **MCP tools**: preserve `mcp__server__tool` name; truncate input args > 2 KiB.

## SQLite — `$XDG_DATA_HOME/homn/learning.db`

```sql
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS pattern_observations (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  pattern_hash    TEXT    NOT NULL,                                       -- xxh3 of (tool_name + normalized input)
  pattern_repr    TEXT    NOT NULL,                                       -- human-readable form for surfacing
  tool_name       TEXT    NOT NULL,
  cwd_prefix      TEXT    NOT NULL,                                       -- common cwd prefix among observations
  human_answer    TEXT    NOT NULL CHECK (human_answer IN ('allow', 'deny')),
  decision_id     INTEGER NOT NULL,                                       -- foreign key into audit.db (loose)
  ts              INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_obs_hash ON pattern_observations(pattern_hash, ts DESC);

CREATE TABLE IF NOT EXISTS suggestions (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  pattern_hash    TEXT    NOT NULL UNIQUE,
  pattern_repr    TEXT    NOT NULL,
  proposed_rule   TEXT    NOT NULL,                                       -- e.g. "allow if tool == \"Bash\" && cmd.matches(\"git push origin feat/*\")"
  proposed_file   TEXT    NOT NULL,                                       -- which policy file to append to
  observation_count INTEGER NOT NULL,
  state           TEXT    NOT NULL CHECK (state IN ('open', 'accepted', 'rejected', 'snoozed')),
  state_changed_at INTEGER NOT NULL,
  expires_at      INTEGER                                                  -- when 'snoozed' or 'rejected' should expire
);

CREATE INDEX IF NOT EXISTS idx_suggestions_state ON suggestions(state, expires_at);

CREATE TABLE IF NOT EXISTS schema_version (
  version INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);
INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, strftime('%s', 'now') * 1000);
```

### Pattern hashing

```text
pattern_hash = xxh3_64(
  tool_name + "|" +
  normalize_input(tool_name, tool_input)
)
```

Normalization rules per tool:

- **Bash**: strip leading whitespace, collapse multiple spaces, replace numeric literals with `<N>`, replace `~/...` with `<HOME>/...`, replace paths inside `cwd` with `<CWD>/...`.
- **Read / Edit / Write**: replace `cwd` prefix with `<CWD>/`.
- **WebFetch**: extract domain + path template; strip query strings.

Two calls with the same normalized form share a pattern hash and accumulate observations.

## Rust types (in `homn-types`)

```rust
// homn-types/src/decision.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: i64,
    pub ts_millis: i64,
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub decision: Decision,
    pub human_answer: Option<HumanAnswer>,
    pub rule_source: Option<RuleSourceLocation>,
    pub rule_text: Option<String>,
    pub ctxgraph_hit: Option<CtxgraphHit>,  // None in Phase 1
    pub latency_ms: u32,
    pub surface: Option<Surface>,
    pub source: DecisionSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Decision { Allow, Deny, Ask }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HumanAnswer { Allow, Deny, AlwaysAllow, AlwaysDeny }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Surface { Tui, Face, Ntfy, Mcp, HookDirect }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DecisionSource { Hook, PtyWrapper, Mcp }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSourceLocation {
    pub file: PathBuf,                       // relative to policies dir
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionId(pub String);            // newtype over the ULID Claude provides
```

## Bus event types

```rust
// homn-types/src/bus.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum BusEvent {
    DecisionMade { decision_id: i64, tool: String, decision: Decision, rule: Option<RuleSourceLocation> },
    AskOpened    { decision_id: i64, payload: HookPayload, context: Option<DecisionContext> },
    AskClosed    { decision_id: i64, answer: HumanAnswer, latency_ms: u32, surface: Surface },
    LearningSuggestion { id: i64, pattern_repr: String, proposed_rule: String, observation_count: u32 },
    SessionStarted { session_id: SessionId, cwd: PathBuf },
    SessionEnded   { session_id: SessionId },
    HighStakesPending { decision_id: i64, kind: HighStakesKind },
    // Phase 3+:
    SessionResumeOffer { session_id: SessionId, context_summary: String },
    OpenLoopNudge      { id: i64, repr: String },
    BuildPassed { repo: PathBuf },
    BuildFailed { repo: PathBuf, error_count: u32 },
    CommitLanded { repo: PathBuf, sha: String },
}
```

## On-disk policy state

```text
$XDG_CONFIG_HOME/homn/
├── homn.toml                              # daemon config
├── policies/
│   ├── default.rhai                       # shipped baseline; user-editable
│   └── <repo-slug>.rhai                   # project overlays
└── ignored/                                # rejected learning suggestions, 30-day TTL
    └── <pattern_hash>.snooze.json
```

`homn.toml` schema:

```toml
[daemon]
socket_path = "${XDG_RUNTIME_DIR}/homn.sock"
events_socket_path = "${XDG_RUNTIME_DIR}/homn-events.sock"
shutdown_grace_ms = 2000

[audit]
db_path = "${XDG_DATA_HOME}/homn/audit.db"
retention_days = 30
compaction_hour = 3        # local time

[learning]
db_path = "${XDG_DATA_HOME}/homn/learning.db"
threshold = 5              # consecutive same-answer asks to trigger suggestion
snooze_days = 30           # how long rejected suggestions stay quiet

[policy]
policies_dir = "${XDG_CONFIG_HOME}/homn/policies"
per_rule_budget_ms = 50
per_call_budget_ms = 200
max_operations = 100_000

[hook]
timeout_ms = 28_000
fallback_decision = "ask"

[pty_wrapper]
enabled = true
prompt_regex = '''Do you want to proceed\? \(y/n\):'''
deny_race_window_ms = 200

[surfaces]
default = "tui"            # "tui" | "face" | "auto"
face_enabled = false
ntfy_topic = ""            # empty = disabled
ntfy_after_idle_minutes = 5

[mcp]
stdio_enabled = true
http_enabled = false
http_bind = "127.0.0.1:9874"
```

Loaded on daemon start; reloaded on `homn config reload` (and via SIGHUP for ops folks).
