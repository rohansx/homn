//! The belief store (US5/US6) — queryable home for extracted [`Belief`](homn_types::Belief)
//! records, with revision history. Adding a belief on a topic already held **supersedes** the
//! prior current belief (sets its `superseded_at`), so `beliefs(topic)` returns the current
//! position plus how the thinking changed.
//!
//! v1 ships both an in-process [`MemoryBeliefStore`] and a [`SqliteBeliefStore`] (the
//! cross-process bridge, like commitments) so the daemon (write) and MCP server (read) share
//! one file.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;
use std::sync::Mutex;

use homn_types::Belief;

/// A queryable belief store. Cheap to clone via `Arc`.
pub trait BeliefStore: Send + Sync {
    /// Add a belief, auto-superseding the prior current belief on the same topic. Returns the
    /// id of the belief that was superseded (if any) so a receipt can reference it.
    fn add(&self, b: Belief) -> anyhow::Result<Option<homn_types::BeliefId>>;

    /// Query beliefs whose topic contains `topic` (case-insensitive substring; empty = all).
    /// Returns current (non-superseded) beliefs first, then superseded ones, each group
    /// newest-first. `current_only=true` drops the history.
    fn query(&self, topic: &str, current_only: bool) -> anyhow::Result<Vec<Belief>>;
}

/// In-process, in-memory belief store. Backed by a `Mutex<Vec>`.
#[derive(Debug, Default)]
pub struct MemoryBeliefStore {
    inner: Mutex<Vec<Belief>>,
}

impl MemoryBeliefStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl BeliefStore for MemoryBeliefStore {
    fn add(&self, b: Belief) -> anyhow::Result<Option<homn_types::BeliefId>> {
        let mut guard = self.inner.lock().expect("belief store lock poisoned");
        // Supersede the prior current belief on the same topic.
        let superseded = guard
            .iter_mut()
            .find(|x| x.topic.eq_ignore_ascii_case(&b.topic) && x.superseded_at.is_none())
            .map(|prior| {
                prior.superseded_at = Some(b.valid_from - chrono::Duration::milliseconds(1));
                prior.id
            });
        guard.push(b);
        Ok(superseded)
    }

    fn query(&self, topic: &str, current_only: bool) -> anyhow::Result<Vec<Belief>> {
        let guard = self.inner.lock().expect("belief store lock poisoned");
        let needle = topic.to_ascii_lowercase();
        let mut matches: Vec<Belief> = guard
            .iter()
            .filter(|b| needle.is_empty() || b.topic.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
        // Current first (newest valid_from first), then history (newest first).
        matches.sort_by(|a, b| {
            b.valid_from
                .cmp(&a.valid_from)
                .then_with(|| a.superseded_at.is_some().cmp(&b.superseded_at.is_some()))
        });
        if current_only {
            matches.retain(|b| b.superseded_at.is_none());
        }
        Ok(matches)
    }
}

/// SQLite-backed belief store — the cross-process bridge (daemon writes, MCP reads). WAL mode.
/// Auto-supersedes on add via `UPDATE … WHERE topic=? AND superseded_ms IS NULL`.
pub struct SqliteBeliefStore {
    path: std::path::PathBuf,
}

impl SqliteBeliefStore {
    /// Open (creating if absent) the beliefs sqlite at `path`.
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = rusqlite::Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;\
             PRAGMA synchronous = NORMAL;\
             CREATE TABLE IF NOT EXISTS beliefs (\
                 id TEXT PRIMARY KEY,\
                 data TEXT NOT NULL,\
                 topic TEXT NOT NULL,\
                 valid_from_ms INTEGER NOT NULL,\
                 superseded_ms INTEGER,\
                 created_ms INTEGER NOT NULL\
             );\
             CREATE INDEX IF NOT EXISTS idx_beliefs_topic ON beliefs(topic);\
             CREATE INDEX IF NOT EXISTS idx_beliefs_current ON beliefs(superseded_ms) WHERE superseded_ms IS NULL;",
        )?;
        Ok(Self { path })
    }

    fn conn(&self) -> anyhow::Result<rusqlite::Connection> {
        Ok(rusqlite::Connection::open(&self.path)?)
    }
}

impl BeliefStore for SqliteBeliefStore {
    fn add(&self, b: Belief) -> anyhow::Result<Option<homn_types::BeliefId>> {
        let conn = self.conn()?;
        let data = serde_json::to_string(&b)?;
        // Auto-supersede the prior current belief on the same topic.
        let superseded_at_ms =
            (b.valid_from - chrono::Duration::milliseconds(1)).timestamp_millis();
        let superseded: Option<String> = conn
            .query_row(
                "UPDATE beliefs SET superseded_ms = ?1 \
                 WHERE topic = ?2 AND superseded_ms IS NULL \
                 RETURNING id",
                rusqlite::params![superseded_at_ms, b.topic],
                |r| r.get::<_, String>(0),
            )
            .ok();
        conn.execute(
            "INSERT OR REPLACE INTO beliefs (id, data, topic, valid_from_ms, superseded_ms, created_ms) \
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
            rusqlite::params![
                b.id.to_string(),
                data,
                b.topic,
                b.valid_from.timestamp_millis(),
                b.created_at.timestamp_millis(),
            ],
        )?;
        Ok(superseded
            .map(|s| homn_types::BeliefId(ulid::Ulid::from_string(&s).unwrap_or_default())))
    }

    fn query(&self, topic: &str, current_only: bool) -> anyhow::Result<Vec<Belief>> {
        let conn = self.conn()?;
        let needle = format!("%{}%", topic.to_ascii_lowercase());
        let mut sql = String::from(
            "SELECT data FROM beliefs WHERE 1=1 AND (LOWER(topic) LIKE ?1 OR ?2 = '')",
        );
        if current_only {
            sql.push_str(" AND superseded_ms IS NULL");
        }
        sql.push_str(" ORDER BY superseded_ms IS NULL DESC, valid_from_ms DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![needle, topic.to_ascii_lowercase()], |r| {
            let data: String = r.get(0)?;
            Ok(data)
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(serde_json::from_str::<Belief>(&r?)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use homn_types::{BeliefId, ExtractionSource};

    fn mk(topic: &str, position: &str, at: DateTime<Utc>) -> Belief {
        Belief {
            id: BeliefId::new(),
            topic: topic.into(),
            position: position.into(),
            confidence: 1.0,
            valid_from: at,
            superseded_at: None,
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn adding_supersedes_the_prior_current_on_the_same_topic() {
        let s = MemoryBeliefStore::new();
        let t = Utc::now();
        let sup = s
            .add(mk("agidb recall", "agidb recall is shaky", t))
            .unwrap();
        assert!(sup.is_none(), "first belief: nothing to supersede");
        let sup = s
            .add(mk(
                "agidb recall",
                "agidb is good enough after the gate",
                t + chrono::Duration::days(2),
            ))
            .unwrap();
        assert!(sup.is_some(), "the second superseded the first");

        let current = s.query("agidb recall", true).unwrap();
        assert_eq!(current.len(), 1);
        assert!(current[0].position.contains("good enough"));
        assert!(current[0].is_current());

        let all = s.query("agidb recall", false).unwrap();
        assert_eq!(all.len(), 2);
        assert!(
            all.iter().any(|b| b.superseded_at.is_some()),
            "history present"
        );
    }

    #[test]
    fn sqlite_reopen_persists_and_supersedes() {
        let db = std::env::temp_dir().join(format!(
            "homnd-beliefs-sqlite-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let t = Utc::now();
        {
            let s = SqliteBeliefStore::open(&db).unwrap();
            s.add(mk("the brain", "use agidb", t)).unwrap();
            s.add(mk(
                "the brain",
                "actually ctxgraph",
                t + chrono::Duration::days(1),
            ))
            .unwrap();
        }
        let s = SqliteBeliefStore::open(&db).unwrap();
        let current = s.query("the brain", true).unwrap();
        assert_eq!(current.len(), 1);
        assert!(current[0].position.contains("ctxgraph"));
        let all = s.query("", false).unwrap();
        assert_eq!(all.len(), 2, "current + history");
        let _ = std::fs::remove_file(&db);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
        }
    }
}
