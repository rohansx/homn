//! The `commitments` MCP tool backing (US5 / T052) — queryable extracted commitments, so
//! `commitments(status?, due_before?)` answers without a recall pass over raw text.
//!
//! The trait is the seam: the daemon populates a [`homnd::commitments::CommitmentStore`] and the
//! MCP server is constructed with an `Arc<dyn Commitments>` over it (the bridge impl lives in
//! `homn-bin`, which depends on both crates, so `homn-mcp` stays decoupled from the ingestion
//! stack). Read-path only, no network.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use homn_types::{Commitment, CommitmentStatus};

/// Anything that can answer `commitments(status?, due_before?)`.
#[async_trait]
pub trait Commitments: Send + Sync {
    /// Return commitments, optionally filtered by status and/or due-before. `None` status = any;
    /// `due_before` excludes commitments due after that instant (undated ones are never excluded).
    async fn commitments(
        &self,
        status: Option<CommitmentStatus>,
        due_before: Option<DateTime<Utc>>,
    ) -> anyhow::Result<Vec<Commitment>>;
}

/// In-process `Commitments` over a fixed `Vec` — for tests.
#[derive(Debug, Default)]
pub struct MemoryCommitments {
    inner: std::sync::Mutex<Vec<Commitment>>,
}

impl MemoryCommitments {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }
    /// Push a commitment (test helper).
    pub fn push(&self, c: Commitment) {
        self.inner.lock().expect("lock poisoned").push(c);
    }
}

#[async_trait]
impl Commitments for MemoryCommitments {
    async fn commitments(
        &self,
        status: Option<CommitmentStatus>,
        due_before: Option<DateTime<Utc>>,
    ) -> anyhow::Result<Vec<Commitment>> {
        let now = Utc::now();
        let guard = self.inner.lock().expect("lock poisoned");
        Ok(guard
            .iter()
            .filter(|c| match status {
                None => true,
                Some(CommitmentStatus::Overdue) => c.is_overdue(now),
                Some(want) => {
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

/// Build a shared `Commitments` handle (the shape `McpState` stores).
#[allow(dead_code)]
pub fn shared(c: impl Commitments + 'static) -> std::sync::Arc<dyn Commitments> {
    std::sync::Arc::new(c)
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

    #[tokio::test]
    async fn filters_by_status_and_overdue() {
        let store = MemoryCommitments::new();
        let past = Utc::now() - chrono::Duration::days(3);
        store.push(mk("open", None, CommitmentStatus::Open));
        store.push(mk("overdue", Some(past), CommitmentStatus::Open));
        store.push(mk("done", None, CommitmentStatus::Fulfilled));

        let open = store
            .commitments(Some(CommitmentStatus::Open), None)
            .await
            .unwrap();
        assert_eq!(
            open.len(),
            1,
            "only the still-open one (overdue reads as Overdue)"
        );
        let overdue = store
            .commitments(Some(CommitmentStatus::Overdue), None)
            .await
            .unwrap();
        assert_eq!(overdue.len(), 1);
        let all = store.commitments(None, None).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn due_before_keeps_undated_excludes_future() {
        let store = MemoryCommitments::new();
        store.push(mk("undated", None, CommitmentStatus::Open));
        store.push(mk(
            "future",
            Some(Utc::now() + chrono::Duration::days(30)),
            CommitmentStatus::Open,
        ));
        let cutoff = Utc::now() + chrono::Duration::days(2);
        let due = store.commitments(None, Some(cutoff)).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].text, "undated");
    }
}
