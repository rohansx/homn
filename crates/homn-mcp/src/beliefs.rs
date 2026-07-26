//! The `beliefs` MCP tool backing (US5/US6) — queryable extracted beliefs, so `beliefs(topic)`
//! returns the current position plus its revision history. The trait is the seam; the daemon
//! populates a [`homnd::beliefs::BeliefStore`] and the MCP server is constructed with an
//! `Arc<dyn Beliefs>` over it. Read-path only, no network.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use homn_types::Belief;

/// Anything that can answer `beliefs(topic)`.
#[async_trait]
pub trait Beliefs: Send + Sync {
    /// Return beliefs whose topic matches `topic` (substring; empty = all). `current_only`
    /// drops the revision history. Current (non-superseded) beliefs come first.
    async fn beliefs(&self, topic: &str, current_only: bool) -> anyhow::Result<Vec<Belief>>;
}

/// In-process `Beliefs` over a fixed `Vec` — for tests.
#[derive(Debug, Default)]
pub struct MemoryBeliefs {
    inner: std::sync::Mutex<Vec<Belief>>,
}

impl MemoryBeliefs {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }
    /// Push a belief (test helper).
    pub fn push(&self, b: Belief) {
        self.inner.lock().expect("lock poisoned").push(b);
    }
}

#[async_trait]
impl Beliefs for MemoryBeliefs {
    async fn beliefs(&self, topic: &str, current_only: bool) -> anyhow::Result<Vec<Belief>> {
        let needle = topic.to_ascii_lowercase();
        let guard = self.inner.lock().expect("lock poisoned");
        let mut matches: Vec<Belief> = guard
            .iter()
            .filter(|b| needle.is_empty() || b.topic.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use homn_types::{BeliefId, ExtractionSource};

    fn mk(topic: &str, position: &str, current: bool) -> Belief {
        Belief {
            id: BeliefId::new(),
            topic: topic.into(),
            position: position.into(),
            confidence: 1.0,
            valid_from: chrono::Utc::now(),
            superseded_at: if current {
                None
            } else {
                Some(chrono::Utc::now())
            },
            source_obs: "obs".into(),
            extracted_by: ExtractionSource::Regex,
            disclosure_receipt: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn current_only_drops_history() {
        let store = MemoryBeliefs::new();
        store.push(mk("the brain", "use agidb", false));
        store.push(mk("the brain", "actually ctxgraph", true));
        let current = store.beliefs("brain", true).await.unwrap();
        assert_eq!(current.len(), 1);
        assert!(current[0].position.contains("ctxgraph"));
        let all = store.beliefs("brain", false).await.unwrap();
        assert_eq!(all.len(), 2);
    }
}
