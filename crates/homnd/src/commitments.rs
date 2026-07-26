//! The commitment store (US5) — queryable home for extracted [`Commitment`](homn_types::Commitment)
//! records, so `commitments(status?, due_before?)` answers without a recall pass over raw text.
//!
//! v1 is an in-process [`MemoryCommitmentStore`] (the daemon owns it; tests use it directly). A
//! SQLite-backed store — shared between the daemon (write) and the MCP server (read), like the
//! audit DB — is the follow-up that makes `commitments()` queryable across processes; the trait
//! is the seam so that swap is invisible to callers.
//!
//! Status is derived lazily at query time: an `Open` commitment past its `due` is reported as
//! `Overdue`, so a stored commitment never needs a background sweep to "go overdue."

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use homn_types::{Commitment, CommitmentStatus};

/// A queryable commitment store. Cheap to clone via `Arc` for the daemon + MCP server to share.
pub trait CommitmentStore: Send + Sync {
    /// Add an extracted commitment.
    fn add(&self, c: Commitment) -> anyhow::Result<()>;

    /// Query, optionally filtering by status and/or due-before. `None` status = any;
    /// `due_before` excludes commitments due after that instant (undated commitments are never
    /// excluded by `due_before`). Overdue-ness is computed at query time against `now`.
    fn query(
        &self,
        status: Option<CommitmentStatus>,
        due_before: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Commitment>>;
}

/// In-process, in-memory store — the v1 default. Backed by a `Mutex<Vec>`.
#[derive(Debug, Default)]
pub struct MemoryCommitmentStore {
    inner: Mutex<Vec<Commitment>>,
}

impl MemoryCommitmentStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CommitmentStore for MemoryCommitmentStore {
    fn add(&self, c: Commitment) -> anyhow::Result<()> {
        self.inner
            .lock()
            .expect("commitment store lock poisoned")
            .push(c);
        Ok(())
    }

    fn query(
        &self,
        status: Option<CommitmentStatus>,
        due_before: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Commitment>> {
        let guard = self.inner.lock().expect("commitment store lock poisoned");
        Ok(guard
            .iter()
            .filter(|c| match status {
                None => true,
                // An Open commitment past due is reported as Overdue at query time.
                Some(CommitmentStatus::Overdue) => c.is_overdue(now),
                Some(want) => {
                    // Skip Open-but-overdue when filtering for Open (it reads as Overdue).
                    if want == CommitmentStatus::Open && c.is_overdue(now) {
                        return false;
                    }
                    c.status == want
                }
            })
            .filter(|c| match due_before {
                None => true,
                Some(cutoff) => c.due.is_none_or(|d| d <= cutoff),
            })
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use homn_types::{CommitmentId, ExtractionSource, Party};

    fn mk(text: &str, due: Option<DateTime<Utc>>, status: CommitmentStatus) -> Commitment {
        Commitment {
            id: CommitmentId::new(),
            text: text.into(),
            owner: Party::Me,
            counterpart: None,
            due,
            status,
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn query_filters_by_status() {
        let s = MemoryCommitmentStore::new();
        s.add(mk("open one", None, CommitmentStatus::Open)).unwrap();
        s.add(mk("done one", None, CommitmentStatus::Fulfilled))
            .unwrap();
        let now = Utc::now();
        assert_eq!(
            s.query(Some(CommitmentStatus::Open), None, now)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            s.query(Some(CommitmentStatus::Fulfilled), None, now)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(s.query(None, None, now).unwrap().len(), 2);
    }

    #[test]
    fn open_past_due_reads_as_overdue_at_query_time() {
        let s = MemoryCommitmentStore::new();
        let past = Utc::now() - chrono::Duration::days(3);
        s.add(mk("overdue promise", Some(past), CommitmentStatus::Open))
            .unwrap();
        let now = Utc::now();
        // It's stored as Open but reports as Overdue.
        assert_eq!(
            s.query(Some(CommitmentStatus::Overdue), None, now)
                .unwrap()
                .len(),
            1
        );
        // And it does NOT show under Open (it reads as overdue, not open).
        assert_eq!(
            s.query(Some(CommitmentStatus::Open), None, now)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn due_before_excludes_later_but_keeps_undated() {
        let s = MemoryCommitmentStore::new();
        let far = Utc::now() + chrono::Duration::days(30);
        let near = Utc::now() - chrono::Duration::days(1);
        s.add(mk("far future", Some(far), CommitmentStatus::Open))
            .unwrap();
        s.add(mk("near past", Some(near), CommitmentStatus::Open))
            .unwrap();
        s.add(mk("undated", None, CommitmentStatus::Open)).unwrap();
        let now = Utc::now();
        let cutoff = Utc::now() + chrono::Duration::days(2);
        let due = s.query(None, Some(cutoff), now).unwrap();
        // far-future excluded; near-past kept (due<=cutoff); undated kept.
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|c| c.text != "far future"));
    }
}

use std::path::Path;

/// SQLite-backed commitment store — the cross-process bridge so the daemon (write) and the MCP
/// server (read) share one file, like the audit DB. WAL mode lets a reader coexist with the
/// writer. The full [`Commitment`] is stored as JSON in `data`; `status`, `due_ms`, and
/// `created_at_ms` are index columns so filtering happens in SQL, not over deserialized rows.
pub struct SqliteCommitmentStore {
    path: std::path::PathBuf,
}

impl SqliteCommitmentStore {
    /// Open (creating if absent) the commitments sqlite at `path`. Sets WAL + normal journal.
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
             CREATE TABLE IF NOT EXISTS commitments (\
                 id TEXT PRIMARY KEY,\
                 data TEXT NOT NULL,\
                 status TEXT NOT NULL,\
                 due_ms INTEGER,\
                 created_at_ms INTEGER NOT NULL\
             );\
             CREATE INDEX IF NOT EXISTS idx_commitments_status ON commitments(status);\
             CREATE INDEX IF NOT EXISTS idx_commitments_due ON commitments(due_ms);",
        )?;
        Ok(Self { path })
    }

    fn conn(&self) -> anyhow::Result<rusqlite::Connection> {
        Ok(rusqlite::Connection::open(&self.path)?)
    }
}

impl CommitmentStore for SqliteCommitmentStore {
    fn add(&self, c: Commitment) -> anyhow::Result<()> {
        let conn = self.conn()?;
        let data = serde_json::to_string(&c)?;
        let status = status_str(c.status);
        let due_ms = c.due.map(|d| d.timestamp_millis());
        let created_ms = c.created_at.timestamp_millis();
        conn.execute(
            "INSERT OR REPLACE INTO commitments (id, data, status, due_ms, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![c.id.to_string(), data, status, due_ms, created_ms],
        )?;
        Ok(())
    }

    fn query(
        &self,
        status: Option<CommitmentStatus>,
        due_before: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Commitment>> {
        let conn = self.conn()?;
        let now_ms = now.timestamp_millis();
        // Build the SQL filter. `overdue` is a DERIVED status (stored as `open` + past due), so
        // the SQL selects `open` rows and splits them by due vs now; `open` excludes the
        // past-due ones (they read as overdue at query time). Fulfilled/Cancelled map directly.
        let mut sql = String::from("SELECT data FROM commitments WHERE 1=1");
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        match status {
            None => {}
            Some(CommitmentStatus::Open) => {
                sql.push_str(" AND status = 'open' AND (due_ms IS NULL OR due_ms >= ?)");
                params.push(Box::new(now_ms));
            }
            Some(CommitmentStatus::Overdue) => {
                sql.push_str(" AND status = 'open' AND due_ms IS NOT NULL AND due_ms < ?");
                params.push(Box::new(now_ms));
            }
            Some(s) => {
                sql.push_str(" AND status = ?");
                params.push(Box::new(status_str(s).to_owned()));
            }
        }
        if let Some(cutoff) = due_before {
            sql.push_str(" AND (due_ms IS NULL OR due_ms <= ?)");
            params.push(Box::new(cutoff.timestamp_millis()));
        }
        sql.push_str(" ORDER BY created_at_ms ASC");
        let p: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(p.as_slice(), |r| {
            let data: String = r.get(0)?;
            Ok(data)
        })?;
        let mut out = Vec::new();
        for r in rows {
            let data = r?;
            out.push(serde_json::from_str::<Commitment>(&data)?);
        }
        Ok(out)
    }
}

fn status_str(s: CommitmentStatus) -> &'static str {
    match s {
        CommitmentStatus::Open => "open",
        CommitmentStatus::Fulfilled => "fulfilled",
        CommitmentStatus::Overdue => "overdue",
        CommitmentStatus::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use homn_types::{CommitmentId, ExtractionSource, Party};

    fn mk(text: &str, due: Option<DateTime<Utc>>, status: CommitmentStatus) -> Commitment {
        Commitment {
            id: CommitmentId::new(),
            text: text.into(),
            owner: Party::Me,
            counterpart: None,
            due,
            status,
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: Utc::now(),
        }
    }

    fn tmp_db() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "homnd-commitments-sqlite-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sqlite_round_trip_filters_status_and_due() {
        let db = tmp_db();
        let store = SqliteCommitmentStore::open(&db).unwrap();
        store.add(mk("open", None, CommitmentStatus::Open)).unwrap();
        store
            .add(mk("done", None, CommitmentStatus::Fulfilled))
            .unwrap();
        let now = Utc::now();
        let all = store.query(None, None, now).unwrap();
        assert_eq!(all.len(), 2);
        let open = store
            .query(Some(CommitmentStatus::Open), None, now)
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].text, "open");
        let _ = std::fs::remove_file(&db);
        // clean WAL/shm sidecars
        for s in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{s}", db.display()));
        }
    }

    #[test]
    fn sqlite_overdue_reads_at_query_time() {
        let db = tmp_db();
        let store = SqliteCommitmentStore::open(&db).unwrap();
        let past = Utc::now() - chrono::Duration::days(3);
        store
            .add(mk("overdue", Some(past), CommitmentStatus::Open))
            .unwrap();
        let now = Utc::now();
        let overdue = store
            .query(Some(CommitmentStatus::Overdue), None, now)
            .unwrap();
        assert_eq!(overdue.len(), 1);
        let open = store
            .query(Some(CommitmentStatus::Open), None, now)
            .unwrap();
        assert_eq!(
            open.len(),
            0,
            "an open-but-past-due reads as Overdue, not Open"
        );
        let _ = std::fs::remove_file(&db);
        for s in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{s}", db.display()));
        }
    }

    #[test]
    fn sqlite_reopen_persists() {
        let db = tmp_db();
        {
            let store = SqliteCommitmentStore::open(&db).unwrap();
            store
                .add(mk("persist me", None, CommitmentStatus::Open))
                .unwrap();
        }
        // Reopen in a new handle (simulating the MCP server reading what the daemon wrote).
        let store = SqliteCommitmentStore::open(&db).unwrap();
        let now = Utc::now();
        let all = store.query(None, None, now).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text, "persist me");
        let _ = std::fs::remove_file(&db);
        for s in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{s}", db.display()));
        }
    }
}
