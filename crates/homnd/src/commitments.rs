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
