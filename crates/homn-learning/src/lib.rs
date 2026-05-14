//! Learning subsystem (US4 / T060-T068): detects consistent ask-resolution patterns and
//! proposes rule promotions.
//!
//! Crucially **suggestion-only** — `homn` never silently modifies policy. After N
//! consecutive same-answer asks for the same pattern, [`record_observation`] returns a
//! [`Suggestion`] that the caller (the daemon, the CLI) can offer to the user via
//! `homn learning list` / `homn learning accept <id>`.
//!
//! Pattern normalisation: see [`normalize`]. We don't try to perfectly reconstruct what
//! the user meant — we extract a coarse glob that's likely to match the next N variants
//! of the same intent (e.g. all `git push origin feat/*`, all reads under `~/dev/`).
//!
//! See [`docs/architecture/policy-engine.md`](../../../docs/architecture/policy-engine.md)
//! §"Learning" for the design rationale.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

use homn_types::HumanAnswer;
use tokio_rusqlite::Connection;

pub mod normalize;

pub use normalize::{normalize_pattern, NormalizedPattern};

/// Current schema version shipped with this crate.
pub const SCHEMA_VERSION: i64 = 1;
/// How many consistent same-answer asks before we surface a suggestion.
pub const DEFAULT_THRESHOLD: u32 = 5;

const MIGRATION_0001: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATIONS: &[(i64, &str)] = &[(1, MIGRATION_0001)];

/// A surfaced suggestion: a rule the user should consider adding to their policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Row id in the suggestions table.
    pub id: i64,
    /// Stable hash of the normalised pattern.
    pub pattern_hash: String,
    /// Human-readable pattern (e.g. `Bash: git push origin feat/*`).
    pub pattern_repr: String,
    /// Tool the pattern applies to.
    pub tool_name: String,
    /// Verb we'd like to add (`"allow"` or `"deny"`).
    pub proposed_verb: String,
    /// The full rule line we'd append, e.g. `allow if tool == "Bash" && cmd.matches("git push origin feat/*");`.
    pub proposed_rule: String,
    /// Policy file we'd append to.
    pub proposed_file: String,
    /// How many observations triggered this.
    pub observation_count: u32,
    /// `"open"`, `"accepted"`, `"rejected"`, or `"snoozed"`.
    pub state: String,
}

/// Database handle for the learning subsystem.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open or create the learning database at `path`. Applies pending migrations.
    pub async fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(path.as_ref()).await?;
        set_pragmas(&conn, true).await?;
        run_migrations(&conn).await?;
        Ok(Self { conn })
    }

    /// In-memory database for tests.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().await?;
        set_pragmas(&conn, false).await?;
        run_migrations(&conn).await?;
        Ok(Self { conn })
    }

    /// Record an ask-resolution observation. If this observation makes the pattern hit the
    /// threshold (and the previous N observations all share this answer), an upserted
    /// `Suggestion` is returned. Otherwise `None`.
    pub async fn record_observation(
        &self,
        decision_id: i64,
        tool: &str,
        tool_input: &serde_json::Value,
        cwd: &str,
        answer: HumanAnswer,
    ) -> anyhow::Result<Option<Suggestion>> {
        // Only allow/deny answers count for learning; the "always_*" forms imply the user
        // already wants a rule, and the daemon should surface that intent immediately.
        // For v0 we keep things simple: only Allow / Deny generate observations. AlwaysAllow
        // / AlwaysDeny are mapped to Allow / Deny respectively.
        let answer_simple = match answer {
            HumanAnswer::Allow | HumanAnswer::AlwaysAllow => "allow",
            HumanAnswer::Deny | HumanAnswer::AlwaysDeny => "deny",
        };

        let pattern = normalize::normalize_pattern(tool, tool_input, cwd);
        let now_millis = unix_millis_now();
        let cwd_prefix = pattern.cwd_prefix.clone();
        let hash = pattern.hash.clone();
        let repr = pattern.repr.clone();
        let tool_name = tool.to_owned();
        let answer_str = answer_simple.to_owned();

        // 1) Insert observation.
        self.conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO pattern_observations
                       (pattern_hash, pattern_repr, tool_name, cwd_prefix, human_answer, decision_id, ts)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![hash, repr, tool_name, cwd_prefix, answer_str, decision_id, now_millis],
                )?;
                Ok(())
            })
            .await?;

        // 2) Check threshold: last N observations for this hash all have the same answer.
        let threshold = DEFAULT_THRESHOLD;
        let hash = pattern.hash.clone();
        let same_count: i64 = self
            .conn
            .call(move |c| {
                let v: i64 = c.query_row(
                    "SELECT COUNT(*) FROM (
                       SELECT human_answer FROM pattern_observations
                       WHERE pattern_hash = ?
                       ORDER BY ts DESC LIMIT ?
                     )
                     WHERE human_answer = (
                       SELECT human_answer FROM pattern_observations
                       WHERE pattern_hash = ?
                       ORDER BY ts DESC LIMIT 1
                     )",
                    rusqlite::params![hash, threshold as i64, hash],
                    |r| r.get(0),
                )?;
                Ok(v)
            })
            .await?;

        if (same_count as u32) < threshold {
            return Ok(None);
        }

        // 3) Threshold hit: upsert a suggestion. If one already exists (open or rejected),
        // we leave it alone — re-firing on every observation past threshold would be noisy.
        let proposed_verb = answer_simple.to_owned();
        let proposed_rule = generate_rule(&pattern, answer_simple);
        let proposed_file = "default.rhai".to_owned();
        let hash = pattern.hash.clone();
        let repr = pattern.repr.clone();
        let tool_name = tool.to_owned();
        let proposed_rule_clone = proposed_rule.clone();
        let proposed_file_clone = proposed_file.clone();
        let count = same_count as u32;

        let inserted_id: Option<i64> = self
            .conn
            .call(move |c| {
                let existing: Option<i64> = c
                    .query_row(
                        "SELECT id FROM suggestions WHERE pattern_hash = ? AND state IN ('open', 'rejected', 'snoozed')",
                        [hash.as_str()],
                        |r| r.get(0),
                    )
                    .ok();
                if existing.is_some() {
                    return Ok(None);
                }
                c.execute(
                    "INSERT INTO suggestions
                       (pattern_hash, pattern_repr, tool_name, proposed_verb, proposed_rule,
                        proposed_file, observation_count, state, state_changed_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?)",
                    rusqlite::params![
                        hash,
                        repr,
                        tool_name,
                        proposed_verb,
                        proposed_rule_clone,
                        proposed_file_clone,
                        count,
                        now_millis,
                    ],
                )?;
                Ok(Some(c.last_insert_rowid()))
            })
            .await?;

        match inserted_id {
            Some(id) => Ok(Some(Suggestion {
                id,
                pattern_hash: pattern.hash,
                pattern_repr: pattern.repr,
                tool_name: tool.to_owned(),
                proposed_verb: answer_simple.to_owned(),
                proposed_rule,
                proposed_file,
                observation_count: same_count as u32,
                state: "open".to_owned(),
            })),
            None => Ok(None),
        }
    }

    /// List open suggestions, oldest first (so the user sees the longest-standing first).
    pub async fn list_open(&self) -> anyhow::Result<Vec<Suggestion>> {
        self.conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT id, pattern_hash, pattern_repr, tool_name, proposed_verb,
                            proposed_rule, proposed_file, observation_count, state
                     FROM suggestions WHERE state = 'open'
                     ORDER BY state_changed_at ASC",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(Suggestion {
                            id: r.get(0)?,
                            pattern_hash: r.get(1)?,
                            pattern_repr: r.get(2)?,
                            tool_name: r.get(3)?,
                            proposed_verb: r.get(4)?,
                            proposed_rule: r.get(5)?,
                            proposed_file: r.get(6)?,
                            observation_count: r.get::<_, i64>(7)? as u32,
                            state: r.get(8)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
            .map_err(Into::into)
    }

    /// Fetch a single suggestion by id (any state).
    pub async fn get(&self, id: i64) -> anyhow::Result<Option<Suggestion>> {
        self.conn
            .call(move |c| {
                let row = c
                    .query_row(
                        "SELECT id, pattern_hash, pattern_repr, tool_name, proposed_verb,
                                proposed_rule, proposed_file, observation_count, state
                         FROM suggestions WHERE id = ?",
                        [id],
                        |r| {
                            Ok(Suggestion {
                                id: r.get(0)?,
                                pattern_hash: r.get(1)?,
                                pattern_repr: r.get(2)?,
                                tool_name: r.get(3)?,
                                proposed_verb: r.get(4)?,
                                proposed_rule: r.get(5)?,
                                proposed_file: r.get(6)?,
                                observation_count: r.get::<_, i64>(7)? as u32,
                                state: r.get(8)?,
                            })
                        },
                    )
                    .ok();
                Ok(row)
            })
            .await
            .map_err(Into::into)
    }

    /// Mark a suggestion accepted. Returns the suggestion's proposed rule + file so the caller
    /// can persist it.
    pub async fn accept(&self, id: i64) -> anyhow::Result<Suggestion> {
        let now = unix_millis_now();
        let sugg = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no suggestion with id {id}"))?;
        self.conn
            .call(move |c| {
                c.execute(
                    "UPDATE suggestions SET state = 'accepted', state_changed_at = ?, expires_at = NULL WHERE id = ?",
                    rusqlite::params![now, id],
                )?;
                Ok(())
            })
            .await?;
        Ok(sugg)
    }

    /// Mark a suggestion rejected for `days` days (default 30 — see DEFAULT_THRESHOLD docs).
    /// After expiry, a fresh stream of consistent answers could surface it again.
    pub async fn reject(&self, id: i64, days: u32) -> anyhow::Result<()> {
        let now = unix_millis_now();
        let expires = now + (days as i64) * 24 * 60 * 60 * 1000;
        let rows: usize = self
            .conn
            .call(move |c| {
                let n = c.execute(
                    "UPDATE suggestions SET state = 'rejected', state_changed_at = ?, expires_at = ? WHERE id = ?",
                    rusqlite::params![now, expires, id],
                )?;
                Ok(n)
            })
            .await?;
        if rows == 0 {
            anyhow::bail!("no suggestion with id {id}");
        }
        Ok(())
    }
}

/// Generate the Rhai rule string for a suggestion.
///
/// Format:
/// - Bash:    `{verb} if tool == "Bash" && cmd.matches("{glob}");`
/// - Read/Edit/Write: `{verb} if tool == "{tool}" && path.matches("{glob}");`
/// - WebFetch: `{verb} if tool == "WebFetch" && url.contains("{host}");`
/// - Other tools: `{verb} if tool == "{tool}";` (coarsest fallback)
fn generate_rule(pattern: &NormalizedPattern, verb: &str) -> String {
    match pattern.tool.as_str() {
        "Bash" => format!(
            "{verb} if tool == \"Bash\" && cmd.matches({lit});",
            lit = rhai_string_literal(&pattern.repr_value)
        ),
        "Read" | "Edit" | "Write" => format!(
            "{verb} if tool == \"{tool}\" && path.matches({lit});",
            tool = pattern.tool,
            lit = rhai_string_literal(&pattern.repr_value)
        ),
        "WebFetch" => format!(
            "{verb} if tool == \"WebFetch\" && url.contains({lit});",
            lit = rhai_string_literal(&pattern.repr_value)
        ),
        other => format!("{verb} if tool == \"{other}\";"),
    }
}

/// Escape a string for embedding inside a Rhai double-quoted literal.
fn rhai_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unix_millis_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

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

/// Append a suggestion's rule to a policy file. Idempotent — if the exact rule line is
/// already in the file, this is a no-op. Adds a comment header documenting the suggestion
/// and the count of observations that triggered it.
pub fn append_rule_to_policy(file_path: &Path, suggestion: &Suggestion) -> anyhow::Result<bool> {
    let existing = std::fs::read_to_string(file_path).unwrap_or_default();
    if existing.contains(&suggestion.proposed_rule) {
        return Ok(false);
    }

    let ts = chrono::Utc::now().format("%Y-%m-%d");
    let block = format!(
        "\n// added by homn learning on {ts} — {count} consistent \"{verb}\" answers for this pattern\n// pattern: {repr}\n{rule}\n",
        ts = ts,
        count = suggestion.observation_count,
        verb = suggestion.proposed_verb,
        repr = suggestion.pattern_repr,
        rule = suggestion.proposed_rule,
    );

    // Append. We write to a temp file and rename for atomicity so a partial write can't
    // corrupt the user's policy.
    let tmp_path = file_path.with_extension("rhai.tmp");
    std::fs::write(&tmp_path, format!("{existing}{block}"))?;
    std::fs::rename(&tmp_path, file_path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn one_observation_doesnt_create_suggestion() {
        let db = Db::in_memory().await.unwrap();
        let out = db
            .record_observation(
                1,
                "Bash",
                &json!({"command": "npm install foo"}),
                "/home/rsx/dev/x",
                HumanAnswer::Allow,
            )
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn five_consistent_observations_create_suggestion() {
        let db = Db::in_memory().await.unwrap();
        let mut last = None;
        for i in 0..5 {
            last = db
                .record_observation(
                    i,
                    "Bash",
                    &json!({"command": format!("npm install pkg-{i}")}),
                    "/home/rsx/dev/x",
                    HumanAnswer::Allow,
                )
                .await
                .unwrap();
        }
        let sugg = last.expect("5th observation should create a suggestion");
        assert_eq!(sugg.tool_name, "Bash");
        assert_eq!(sugg.proposed_verb, "allow");
        assert!(
            sugg.proposed_rule.contains("allow if"),
            "rule should be an allow: {}",
            sugg.proposed_rule
        );
        assert!(
            sugg.proposed_rule.contains("npm install"),
            "rule should contain command prefix: {}",
            sugg.proposed_rule
        );
        assert_eq!(sugg.observation_count, 5);
    }

    #[tokio::test]
    async fn mixed_answers_do_not_create_suggestion() {
        let db = Db::in_memory().await.unwrap();
        for i in 0..4 {
            db.record_observation(
                i,
                "Bash",
                &json!({"command": format!("rm something-{i}")}),
                "/tmp",
                HumanAnswer::Allow,
            )
            .await
            .unwrap();
        }
        // 5th is a Deny — breaks the streak.
        let out = db
            .record_observation(
                100,
                "Bash",
                &json!({"command": "rm last-thing"}),
                "/tmp",
                HumanAnswer::Deny,
            )
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn list_open_returns_suggestions_in_creation_order() {
        let db = Db::in_memory().await.unwrap();
        for i in 0..5 {
            db.record_observation(
                i,
                "Bash",
                &json!({"command": format!("git status -s {i}")}),
                "/home/rsx",
                HumanAnswer::Allow,
            )
            .await
            .unwrap();
        }
        let list = db.list_open().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].state, "open");
    }

    #[tokio::test]
    async fn accept_returns_suggestion_and_marks_state() {
        let db = Db::in_memory().await.unwrap();
        for i in 0..5 {
            db.record_observation(
                i,
                "Bash",
                &json!({"command": format!("git status {i}")}),
                "/home/rsx",
                HumanAnswer::Allow,
            )
            .await
            .unwrap();
        }
        let list = db.list_open().await.unwrap();
        let id = list[0].id;

        let accepted = db.accept(id).await.unwrap();
        assert_eq!(accepted.id, id);

        // After accept, list_open returns nothing.
        assert!(db.list_open().await.unwrap().is_empty());

        // But get(id) still works.
        let got = db.get(id).await.unwrap().unwrap();
        assert_eq!(got.state, "accepted");
    }

    #[tokio::test]
    async fn reject_sets_expires_at() {
        let db = Db::in_memory().await.unwrap();
        for i in 0..5 {
            db.record_observation(
                i,
                "Bash",
                &json!({"command": format!("git diff {i}")}),
                "/home/rsx",
                HumanAnswer::Allow,
            )
            .await
            .unwrap();
        }
        let id = db.list_open().await.unwrap()[0].id;
        db.reject(id, 30).await.unwrap();
        let got = db.get(id).await.unwrap().unwrap();
        assert_eq!(got.state, "rejected");
        assert!(db.list_open().await.unwrap().is_empty());
    }

    #[test]
    fn rhai_string_literal_escapes_quotes_and_backslashes() {
        assert_eq!(rhai_string_literal("hello"), "\"hello\"");
        assert_eq!(rhai_string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(rhai_string_literal("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn append_rule_to_policy_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("default.rhai");
        std::fs::write(&path, "// existing rules\n").unwrap();
        let sugg = Suggestion {
            id: 1,
            pattern_hash: "xx".into(),
            pattern_repr: "Bash: git status *".into(),
            tool_name: "Bash".into(),
            proposed_verb: "allow".into(),
            proposed_rule: r#"allow if tool == "Bash" && cmd.matches("git status *");"#.into(),
            proposed_file: "default.rhai".into(),
            observation_count: 5,
            state: "open".into(),
        };

        let added = append_rule_to_policy(&path, &sugg).unwrap();
        assert!(added, "first append should write");
        let after_first = std::fs::read_to_string(&path).unwrap();
        assert!(after_first.contains(&sugg.proposed_rule));

        let added2 = append_rule_to_policy(&path, &sugg).unwrap();
        assert!(!added2, "second append should be a no-op");
        let after_second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after_first, after_second, "file should be unchanged");
    }
}
