//! SQLite-backed audit log for `homn` decisions.
//!
//! Owns the schema, migrations, and the synchronous-per-call writer API. A single Tokio task
//! writer with batching lands in a future task; for v0 the in-process `write_decision` call is
//! the writer and is fast enough for the expected decision rate (≤10k/day in practice).
//!
//! See [`docs/technical/audit-log.md`](../../../docs/technical/audit-log.md) and
//! [`specs/001-policy-engine/data-model.md`](../../../specs/001-policy-engine/data-model.md).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

use homn_types::{
    Decision, DecisionContext, DecisionRecord, DecisionSource, HumanAnswer, RuleSourceLocation,
    SessionId, Surface,
};
use tokio_rusqlite::Connection;

pub mod ledger;
pub mod watermarks;

pub use ledger::{LedgerEntry, LedgerVerification};

/// Current schema version shipped with this crate. Bump and add a new migration when changing
/// the schema.
pub const SCHEMA_VERSION: i64 = 3;

const MIGRATION_0001: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_0002: &str = include_str!("../migrations/0002_ledger.sql");
const MIGRATION_0003: &str = include_str!("../migrations/0003_watermarks.sql");

/// Ordered list of `(version, sql)` migrations applied at startup. Append-only.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, MIGRATION_0001),
    (2, MIGRATION_0002),
    (3, MIGRATION_0003),
];

/// A decision about to be written — `DecisionRecord` minus the auto-generated `id`.
#[derive(Debug, Clone)]
pub struct NewDecision {
    /// Unix epoch milliseconds.
    pub ts_millis: i64,
    /// The Claude Code session that triggered the decision.
    pub session_id: SessionId,
    /// Working directory of the calling session, as a string.
    pub cwd: String,
    /// Tool name.
    pub tool_name: String,
    /// Tool input as JSON. Caller is responsible for truncating to 4 KiB.
    pub tool_input: serde_json::Value,
    /// Decision outcome.
    pub decision: Decision,
    /// Human's answer, if the decision was `Ask`.
    pub human_answer: Option<HumanAnswer>,
    /// Rule that fired, if any.
    pub rule_source: Option<RuleSourceLocation>,
    /// Snapshot of the rule's source text.
    pub rule_text: Option<String>,
    /// Ctxgraph context (Phase 3+).
    pub ctxgraph_hit: Option<DecisionContext>,
    /// End-to-end latency.
    pub latency_ms: u32,
    /// Surface that answered, if a human did.
    pub surface: Option<Surface>,
    /// Where the request came from.
    pub source: DecisionSource,
}

/// Audit database handle.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open the audit database at the given path, applying any pending migrations.
    pub async fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(path.as_ref()).await?;
        set_pragmas(&conn, /*persistent*/ true).await?;
        run_migrations(&conn).await?;
        Ok(Self { conn })
    }

    /// Open an in-memory audit database — useful for tests.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().await?;
        set_pragmas(&conn, /*persistent*/ false).await?;
        run_migrations(&conn).await?;
        Ok(Self { conn })
    }

    /// Borrow the underlying connection — primarily for tests and (later) the writer task.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Return the schema version recorded in the database.
    pub async fn current_version(&self) -> anyhow::Result<i64> {
        let v: i64 = self
            .conn
            .call(|c| {
                let row: i64 = c.query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                    [],
                    |r| r.get(0),
                )?;
                Ok(row)
            })
            .await?;
        Ok(v)
    }

    /// Return the names of all user tables in the database (for inspection / tests).
    pub async fn table_names(&self) -> anyhow::Result<Vec<String>> {
        let names: Vec<String> = self
            .conn
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(names)
            })
            .await?;
        Ok(names)
    }

    /// Insert a new decision, returning its assigned row id.
    pub async fn write_decision(&self, rec: NewDecision) -> anyhow::Result<i64> {
        let rec_for_insert = SerializedDecision::from_record(&rec);
        let id: i64 = self
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO decisions
                       (ts, session_id, cwd, tool_name, tool_input, decision, human_answer,
                        rule_source, rule_text, ctxgraph_hit, latency_ms, surface, source)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        rec_for_insert.ts,
                        rec_for_insert.session_id,
                        rec_for_insert.cwd,
                        rec_for_insert.tool_name,
                        rec_for_insert.tool_input,
                        rec_for_insert.decision,
                        rec_for_insert.human_answer,
                        rec_for_insert.rule_source,
                        rec_for_insert.rule_text,
                        rec_for_insert.ctxgraph_hit,
                        rec_for_insert.latency_ms,
                        rec_for_insert.surface,
                        rec_for_insert.source,
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await?;
        Ok(id)
    }

    /// Update an existing audit row with the human's answer + which surface answered.
    /// Called by the daemon when an `Ask` decision is resolved (T032/T033).
    pub async fn update_human_answer(
        &self,
        decision_id: i64,
        answer: Option<HumanAnswer>,
        surface: Surface,
    ) -> anyhow::Result<()> {
        let answer_str = answer.map(human_answer_as_str).map(str::to_owned);
        let surface_str = surface_as_str(surface).to_owned();
        let rows_changed: usize = self
            .conn
            .call(move |c| {
                let n = c.execute(
                    "UPDATE decisions SET human_answer = ?, surface = ? WHERE id = ?",
                    rusqlite::params![answer_str, surface_str, decision_id],
                )?;
                Ok(n)
            })
            .await?;
        if rows_changed == 0 {
            anyhow::bail!("no decision with id {decision_id}");
        }
        Ok(())
    }

    /// Return the most recent `limit` decisions, newest first.
    pub async fn tail(&self, limit: u32) -> anyhow::Result<Vec<DecisionRecord>> {
        self.query(LogQuery {
            limit,
            ..Default::default()
        })
        .await
    }

    /// Run a filtered audit-log query. All filters AND together; unset filters match all rows.
    pub async fn query(&self, q: LogQuery) -> anyhow::Result<Vec<DecisionRecord>> {
        let mut sql = String::from(
            "SELECT id, ts, session_id, cwd, tool_name, tool_input, decision,
                    human_answer, rule_source, rule_text, ctxgraph_hit, latency_ms,
                    surface, source FROM decisions",
        );
        let mut where_clauses: Vec<&'static str> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql + Send>> = Vec::new();

        // FTS5-backed --grep first (intersect via subquery for efficiency).
        if let Some(grep) = q.grep.clone() {
            where_clauses
                .push("id IN (SELECT rowid FROM decisions_fts WHERE decisions_fts MATCH ?)");
            params.push(Box::new(grep));
        }
        if let Some(since) = q.since_millis {
            where_clauses.push("ts >= ?");
            params.push(Box::new(since));
        }
        if let Some(until) = q.until_millis {
            where_clauses.push("ts <= ?");
            params.push(Box::new(until));
        }
        if let Some(decision) = q.decision {
            where_clauses.push("decision = ?");
            params.push(Box::new(decision_as_str(decision).to_string()));
        }
        if q.asked {
            where_clauses.push("decision = 'ask'");
        }
        if let Some(sid) = q.session_id.clone() {
            where_clauses.push("session_id = ?");
            params.push(Box::new(sid));
        }
        if let Some(tool) = q.tool_name.clone() {
            where_clauses.push("tool_name = ?");
            params.push(Box::new(tool));
        }

        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }
        sql.push_str(if q.ascending {
            " ORDER BY ts ASC, id ASC"
        } else {
            " ORDER BY ts DESC, id DESC"
        });
        sql.push_str(" LIMIT ?");
        params.push(Box::new(q.limit.max(1) as i64));

        let rows: Vec<SerializedRow> = self
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(&sql)?;
                let param_refs: Vec<&dyn rusqlite::ToSql> = params
                    .iter()
                    .map(|p| p.as_ref() as &dyn rusqlite::ToSql)
                    .collect();
                let rows = stmt
                    .query_map(param_refs.as_slice(), |r| {
                        Ok(SerializedRow {
                            id: r.get(0)?,
                            ts: r.get(1)?,
                            session_id: r.get(2)?,
                            cwd: r.get(3)?,
                            tool_name: r.get(4)?,
                            tool_input: r.get(5)?,
                            decision: r.get(6)?,
                            human_answer: r.get(7)?,
                            rule_source: r.get(8)?,
                            rule_text: r.get(9)?,
                            ctxgraph_hit: r.get(10)?,
                            latency_ms: r.get(11)?,
                            surface: r.get(12)?,
                            source: r.get(13)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await?;
        rows.into_iter().map(SerializedRow::into_record).collect()
    }
}

/// Open a synchronous (non-Tokio) connection and return `true` if any `deny` decision was
/// recorded in the last `within_secs` seconds. Used by the PTY-wrapper (T055) which can't
/// participate in the daemon's Tokio runtime.
pub fn has_recent_deny_sync(audit_path: &Path, within_secs: u64) -> anyhow::Result<bool> {
    let conn = rusqlite::Connection::open(audit_path)?;
    let now_ms: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cutoff = now_ms - (within_secs as i64) * 1000;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM decisions WHERE decision = 'deny' AND ts >= ?",
        [cutoff],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

/// Filter set for [`Db::query`]. All set fields AND together; unset fields match all rows.
#[derive(Debug, Default, Clone)]
pub struct LogQuery {
    /// Lower bound on `ts` (unix epoch millis), inclusive.
    pub since_millis: Option<i64>,
    /// Upper bound on `ts`, inclusive.
    pub until_millis: Option<i64>,
    /// Filter to a specific decision verb.
    pub decision: Option<Decision>,
    /// Convenience: filter to decisions that went through the human surface (decision='ask').
    pub asked: bool,
    /// Filter to a specific session id.
    pub session_id: Option<String>,
    /// Filter to a specific tool name.
    pub tool_name: Option<String>,
    /// FTS5 query string against (tool_input, tool_name, cwd).
    pub grep: Option<String>,
    /// Maximum number of rows. Always positive.
    pub limit: u32,
    /// Order: `false` = newest first (default), `true` = oldest first.
    pub ascending: bool,
}

impl LogQuery {
    /// Builder helper.
    pub fn new() -> Self {
        Self {
            limit: 100,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Wire-format helpers: converting between Rust types and SQLite columns.

struct SerializedDecision {
    ts: i64,
    session_id: String,
    cwd: String,
    tool_name: String,
    tool_input: String, // JSON
    decision: &'static str,
    human_answer: Option<&'static str>,
    rule_source: Option<String>, // "file:line"
    rule_text: Option<String>,
    ctxgraph_hit: Option<String>, // JSON
    latency_ms: i64,
    surface: Option<&'static str>,
    source: &'static str,
}

impl SerializedDecision {
    fn from_record(rec: &NewDecision) -> Self {
        Self {
            ts: rec.ts_millis,
            session_id: rec.session_id.0.clone(),
            cwd: rec.cwd.clone(),
            tool_name: rec.tool_name.clone(),
            tool_input: rec.tool_input.to_string(),
            decision: decision_as_str(rec.decision),
            human_answer: rec.human_answer.map(human_answer_as_str),
            rule_source: rec
                .rule_source
                .as_ref()
                .map(|loc| format!("{}:{}", loc.file.display(), loc.line)),
            rule_text: rec.rule_text.clone(),
            ctxgraph_hit: rec
                .ctxgraph_hit
                .as_ref()
                .map(|c| serde_json::to_string(c).unwrap_or_default()),
            latency_ms: rec.latency_ms as i64,
            surface: rec.surface.map(surface_as_str),
            source: decision_source_as_str(rec.source),
        }
    }
}

struct SerializedRow {
    id: i64,
    ts: i64,
    session_id: String,
    cwd: String,
    tool_name: String,
    tool_input: String,
    decision: String,
    human_answer: Option<String>,
    rule_source: Option<String>,
    rule_text: Option<String>,
    ctxgraph_hit: Option<String>,
    latency_ms: i64,
    surface: Option<String>,
    source: String,
}

impl SerializedRow {
    fn into_record(self) -> anyhow::Result<DecisionRecord> {
        let decision = parse_decision(&self.decision)?;
        let human_answer = self
            .human_answer
            .as_deref()
            .map(parse_human_answer)
            .transpose()?;
        let surface = self.surface.as_deref().map(parse_surface).transpose()?;
        let source = parse_decision_source(&self.source)?;
        let tool_input: serde_json::Value = serde_json::from_str(&self.tool_input)
            .unwrap_or(serde_json::Value::String(self.tool_input.clone()));
        let rule_source = self
            .rule_source
            .as_deref()
            .map(parse_rule_source)
            .transpose()?;
        let ctxgraph_hit = self
            .ctxgraph_hit
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());
        Ok(DecisionRecord {
            id: self.id,
            ts_millis: self.ts,
            session_id: SessionId::new(self.session_id),
            cwd: std::path::PathBuf::from(self.cwd),
            tool_name: self.tool_name,
            tool_input,
            decision,
            human_answer,
            rule_source,
            rule_text: self.rule_text,
            ctxgraph_hit,
            latency_ms: self.latency_ms as u32,
            surface,
            source,
        })
    }
}

fn decision_as_str(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Deny => "deny",
        Decision::Ask => "ask",
    }
}

fn parse_decision(s: &str) -> anyhow::Result<Decision> {
    match s {
        "allow" => Ok(Decision::Allow),
        "deny" => Ok(Decision::Deny),
        "ask" => Ok(Decision::Ask),
        other => Err(anyhow::anyhow!("unknown decision: {other}")),
    }
}

fn human_answer_as_str(a: HumanAnswer) -> &'static str {
    match a {
        HumanAnswer::Allow => "allow",
        HumanAnswer::Deny => "deny",
        HumanAnswer::AlwaysAllow => "always_allow",
        HumanAnswer::AlwaysDeny => "always_deny",
    }
}

fn parse_human_answer(s: &str) -> anyhow::Result<HumanAnswer> {
    match s {
        "allow" => Ok(HumanAnswer::Allow),
        "deny" => Ok(HumanAnswer::Deny),
        "always_allow" => Ok(HumanAnswer::AlwaysAllow),
        "always_deny" => Ok(HumanAnswer::AlwaysDeny),
        other => Err(anyhow::anyhow!("unknown human_answer: {other}")),
    }
}

fn surface_as_str(s: Surface) -> &'static str {
    match s {
        Surface::Tui => "tui",
        Surface::Face => "face",
        Surface::Ntfy => "ntfy",
        Surface::Mcp => "mcp",
        Surface::HookDirect => "hook-direct",
    }
}

fn parse_surface(s: &str) -> anyhow::Result<Surface> {
    match s {
        "tui" => Ok(Surface::Tui),
        "face" => Ok(Surface::Face),
        "ntfy" => Ok(Surface::Ntfy),
        "mcp" => Ok(Surface::Mcp),
        "hook-direct" => Ok(Surface::HookDirect),
        other => Err(anyhow::anyhow!("unknown surface: {other}")),
    }
}

fn decision_source_as_str(s: DecisionSource) -> &'static str {
    match s {
        DecisionSource::Hook => "hook",
        DecisionSource::PtyWrapper => "pty-wrapper",
        DecisionSource::Mcp => "mcp",
    }
}

fn parse_decision_source(s: &str) -> anyhow::Result<DecisionSource> {
    match s {
        "hook" => Ok(DecisionSource::Hook),
        "pty-wrapper" => Ok(DecisionSource::PtyWrapper),
        "mcp" => Ok(DecisionSource::Mcp),
        other => Err(anyhow::anyhow!("unknown source: {other}")),
    }
}

fn parse_rule_source(s: &str) -> anyhow::Result<RuleSourceLocation> {
    let (file, line) = s
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("malformed rule_source: {s}"))?;
    Ok(RuleSourceLocation {
        file: std::path::PathBuf::from(file),
        line: line.parse()?,
    })
}

// ---------------------------------------------------------------------------

async fn set_pragmas(conn: &Connection, persistent: bool) -> anyhow::Result<()> {
    conn.call(move |c| {
        c.pragma_update(None, "foreign_keys", "ON")?;
        c.pragma_update(None, "temp_store", "MEMORY")?;
        if persistent {
            c.pragma_update(None, "journal_mode", "WAL")?;
            c.pragma_update(None, "synchronous", "NORMAL")?;
        }
        Ok(())
    })
    .await?;
    Ok(())
}

async fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    conn.call(|c| {
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version    INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
             );",
        )?;
        Ok(())
    })
    .await?;

    let current: i64 = conn
        .call(|c| {
            let v: i64 = c.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |r| r.get(0),
            )?;
            Ok(v)
        })
        .await?;

    for (version, sql) in MIGRATIONS.iter().copied() {
        if version > current {
            tracing::info!(target: "homn_audit::migrate", version, "applying migration");
            let sql_owned = sql.to_owned();
            conn.call(move |c| {
                let tx = c.transaction()?;
                tx.execute_batch(&sql_owned)?;
                tx.execute(
                    "INSERT INTO schema_version (version, applied_at)
                     VALUES (?, strftime('%s', 'now') * 1000)",
                    [version],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn sample_new_decision() -> NewDecision {
        NewDecision {
            ts_millis: 1_715_000_000_000,
            session_id: SessionId::new("01HXY"),
            cwd: "/home/rsx/dev/x".into(),
            tool_name: "Bash".into(),
            tool_input: json!({"command": "git push origin main"}),
            decision: Decision::Deny,
            human_answer: None,
            rule_source: Some(RuleSourceLocation {
                file: PathBuf::from("default.rhai"),
                line: 14,
            }),
            rule_text: Some("deny if cmd.matches(\"git push * main\")".into()),
            ctxgraph_hit: None,
            latency_ms: 47,
            surface: Some(Surface::HookDirect),
            source: DecisionSource::Hook,
        }
    }

    #[tokio::test]
    async fn open_in_memory_applies_migrations() {
        let db = Db::in_memory().await.unwrap();
        assert_eq!(db.current_version().await.unwrap(), SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn migrations_create_expected_tables() {
        let db = Db::in_memory().await.unwrap();
        let tables = db.table_names().await.unwrap();
        for expected in ["decisions", "decisions_fts", "schema_version"] {
            assert!(
                tables.iter().any(|t| t == expected),
                "expected table `{expected}` to exist; got {tables:?}"
            );
        }
    }

    #[tokio::test]
    async fn decisions_check_constraints_reject_bad_values() {
        let db = Db::in_memory().await.unwrap();
        let err = db
            .conn
            .call(|c| {
                c.execute(
                    "INSERT INTO decisions (ts, session_id, cwd, tool_name, tool_input, decision, latency_ms, source)
                     VALUES (1, 's', '/', 'Bash', '{}', 'bogus', 1, 'hook')",
                    [],
                )?;
                Ok(())
            })
            .await;
        assert!(
            err.is_err(),
            "expected CHECK constraint to reject bad decision value"
        );
    }

    #[tokio::test]
    async fn fts_index_picks_up_inserts() {
        let db = Db::in_memory().await.unwrap();
        db.conn
            .call(|c| {
                c.execute(
                    "INSERT INTO decisions
                       (ts, session_id, cwd, tool_name, tool_input, decision, latency_ms, source)
                     VALUES (1, 's', '/home/x', 'Bash', '{\"command\":\"npm install foo\"}', 'allow', 1, 'hook')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let hits: i64 = db
            .conn
            .call(|c| {
                let v: i64 = c.query_row(
                    "SELECT COUNT(*) FROM decisions_fts WHERE decisions_fts MATCH 'foo'",
                    [],
                    |r| r.get(0),
                )?;
                Ok(v)
            })
            .await
            .unwrap();
        assert_eq!(hits, 1);
    }

    #[tokio::test]
    async fn double_open_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        {
            let db = Db::open(&path).await.unwrap();
            assert_eq!(db.current_version().await.unwrap(), SCHEMA_VERSION);
        }
        {
            let db = Db::open(&path).await.unwrap();
            assert_eq!(db.current_version().await.unwrap(), SCHEMA_VERSION);
        }
    }

    #[tokio::test]
    async fn write_decision_returns_increasing_ids() {
        let db = Db::in_memory().await.unwrap();
        let id1 = db.write_decision(sample_new_decision()).await.unwrap();
        let mut second = sample_new_decision();
        second.ts_millis += 1;
        let id2 = db.write_decision(second).await.unwrap();
        assert!(id2 > id1);
    }

    #[tokio::test]
    async fn tail_returns_records_newest_first_with_round_trip_fidelity() {
        let db = Db::in_memory().await.unwrap();
        let id1 = db.write_decision(sample_new_decision()).await.unwrap();
        let mut newer = sample_new_decision();
        newer.ts_millis += 1_000;
        newer.tool_name = "Read".into();
        newer.decision = Decision::Allow;
        newer.rule_source = Some(RuleSourceLocation {
            file: PathBuf::from("default.rhai"),
            line: 3,
        });
        newer.rule_text = Some("allow if tool == \"Read\"".into());
        newer.surface = None;
        let id2 = db.write_decision(newer.clone()).await.unwrap();

        let rows = db.tail(10).await.unwrap();
        assert_eq!(rows.len(), 2);
        // Newest first:
        assert_eq!(rows[0].id, id2);
        assert_eq!(rows[1].id, id1);

        // Round-trip fidelity on the newer record:
        let got = &rows[0];
        assert_eq!(got.tool_name, "Read");
        assert_eq!(got.decision, Decision::Allow);
        assert_eq!(got.rule_source.as_ref().unwrap().line, 3);
        assert_eq!(
            got.rule_source.as_ref().unwrap().file,
            PathBuf::from("default.rhai")
        );
    }

    #[tokio::test]
    async fn tail_with_limit_caps_results() {
        let db = Db::in_memory().await.unwrap();
        for i in 0..5 {
            let mut d = sample_new_decision();
            d.ts_millis += i;
            db.write_decision(d).await.unwrap();
        }
        let rows = db.tail(3).await.unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn query_filters_by_decision() {
        let db = Db::in_memory().await.unwrap();
        let mut allow = sample_new_decision();
        allow.decision = Decision::Allow;
        allow.ts_millis += 1;
        db.write_decision(sample_new_decision()).await.unwrap(); // deny
        db.write_decision(allow).await.unwrap();

        let denies = db
            .query(LogQuery {
                decision: Some(Decision::Deny),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(denies.len(), 1);
        assert_eq!(denies[0].decision, Decision::Deny);

        let allows = db
            .query(LogQuery {
                decision: Some(Decision::Allow),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(allows.len(), 1);
        assert_eq!(allows[0].decision, Decision::Allow);
    }

    #[tokio::test]
    async fn query_filters_by_since() {
        let db = Db::in_memory().await.unwrap();
        for i in 0..5_i64 {
            let mut d = sample_new_decision();
            d.ts_millis = 1_000 + i;
            db.write_decision(d).await.unwrap();
        }
        let recent = db
            .query(LogQuery {
                since_millis: Some(1_003),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap();
        // 1_003 and 1_004 match.
        assert_eq!(recent.len(), 2);
    }

    #[tokio::test]
    async fn query_filters_by_tool_and_session() {
        let db = Db::in_memory().await.unwrap();
        let mut a = sample_new_decision();
        a.tool_name = "Bash".into();
        a.session_id = SessionId::new("S1");
        let mut b = sample_new_decision();
        b.tool_name = "Read".into();
        b.session_id = SessionId::new("S2");
        b.ts_millis += 1;
        db.write_decision(a).await.unwrap();
        db.write_decision(b).await.unwrap();

        let bash = db
            .query(LogQuery {
                tool_name: Some("Bash".into()),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(bash.len(), 1);
        assert_eq!(bash[0].tool_name, "Bash");

        let s1 = db
            .query(LogQuery {
                session_id: Some("S1".into()),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].session_id.0, "S1");
    }

    #[tokio::test]
    async fn query_grep_uses_fts5() {
        let db = Db::in_memory().await.unwrap();
        let mut a = sample_new_decision();
        a.tool_input = json!({"command": "npm install some-package"});
        let mut b = sample_new_decision();
        b.tool_input = json!({"command": "git status"});
        b.ts_millis += 1;
        db.write_decision(a).await.unwrap();
        db.write_decision(b).await.unwrap();

        let hits = db
            .query(LogQuery {
                grep: Some("install".into()),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].tool_input.to_string().contains("npm install"),
            "expected npm row; got {:?}",
            hits[0].tool_input
        );
    }

    #[tokio::test]
    async fn has_recent_deny_sync_returns_true_after_recent_deny() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        {
            let db = Db::open(&path).await.unwrap();
            db.write_decision(sample_new_decision()).await.unwrap();
        }
        // sample_new_decision has decision=Deny and ts in the recent past relative to "now",
        // but its ts is hard-coded. So we use a generous window.
        let result = has_recent_deny_sync(&path, 60 * 60 * 24 * 365 * 100).unwrap();
        assert!(result, "expected to find the recent deny row");
    }

    #[tokio::test]
    async fn has_recent_deny_sync_returns_false_when_no_recent_deny() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        let db = Db::open(&path).await.unwrap();
        let mut allow = sample_new_decision();
        allow.decision = Decision::Allow;
        db.write_decision(allow).await.unwrap();
        drop(db);
        // Only an allow was recorded, even with a wide window.
        let result = has_recent_deny_sync(&path, 60 * 60 * 24 * 365 * 100).unwrap();
        assert!(!result, "expected no deny");
    }

    #[tokio::test]
    async fn has_recent_deny_sync_respects_time_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        {
            let db = Db::open(&path).await.unwrap();
            // sample_new_decision().ts_millis is 1_715_000_000_000 — way in the past from
            // the actual "now" of the test run. So a 60-second window should NOT include it.
            db.write_decision(sample_new_decision()).await.unwrap();
        }
        let result = has_recent_deny_sync(&path, 60).unwrap();
        assert!(!result, "old deny should be outside a 60s window");
    }

    #[tokio::test]
    async fn update_human_answer_sets_fields() {
        let db = Db::in_memory().await.unwrap();
        let mut d = sample_new_decision();
        d.decision = Decision::Ask;
        d.surface = None;
        let id = db.write_decision(d).await.unwrap();

        db.update_human_answer(id, Some(HumanAnswer::Allow), Surface::Tui)
            .await
            .unwrap();

        let rows = db.tail(1).await.unwrap();
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].human_answer, Some(HumanAnswer::Allow));
        assert_eq!(rows[0].surface, Some(Surface::Tui));
    }

    #[tokio::test]
    async fn update_human_answer_errors_on_missing_id() {
        let db = Db::in_memory().await.unwrap();
        let err = db
            .update_human_answer(9999, Some(HumanAnswer::Deny), Surface::Tui)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no decision with id"));
    }

    #[tokio::test]
    async fn query_ascending_returns_oldest_first() {
        let db = Db::in_memory().await.unwrap();
        for i in 0..3_i64 {
            let mut d = sample_new_decision();
            d.ts_millis = 1_000 + i;
            db.write_decision(d).await.unwrap();
        }
        let rows = db
            .query(LogQuery {
                ascending: true,
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap();
        let ts: Vec<i64> = rows.iter().map(|r| r.ts_millis).collect();
        assert_eq!(ts, vec![1_000, 1_001, 1_002]);
    }
}
