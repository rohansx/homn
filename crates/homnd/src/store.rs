//! The `Store` abstraction — the last stage of the ingestion pipeline.
//!
//! The brain plugs in here. `MemoryStore` is the default, no-feature test impl; `AgidbStore`
//! is the real brain, gated behind `brain-agidb` so CI (which never compiles agidb) still
//! builds the whole spine. See [`specs/002-ambient-memory/contracts/gate-pipeline.md`] stage 5.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use async_trait::async_trait;
use homn_types::Observation;

/// The durable memory a gated observation lands in.
#[async_trait]
pub trait Store: Send + Sync {
    /// Persist a post-gate observation. Called only after the gate and after the audit receipt.
    ///
    /// Returns the store's own id for the observation (for receipts / UI); the default impls
    /// return the observation's ULID as a string.
    async fn store(&self, obs: &Observation) -> anyhow::Result<String>;

    /// Forget memories matching `scope`, returning a [`DeletionReceipt`] (proof of what was
    /// removed, without re-exposing content — Invariant 3 / FR-023). The default is a no-op
    /// returning a zero-match receipt (so the no-feature `MemoryStore` stays honest); the
    /// agidb-backed store overrides it with real unlearn.
    async fn forget(
        &self,
        scope: &homn_types::ForgetScope,
    ) -> anyhow::Result<homn_types::DeletionReceipt> {
        Ok(homn_types::DeletionReceipt {
            scope: scope.clone(),
            match_count: 0,
            at: chrono::Utc::now(),
        })
    }
}

/// In-process, in-memory store — the no-feature default used by tests and the Phase-0 harness.
#[derive(Debug, Default)]
pub struct MemoryStore {
    #[cfg(test)]
    inner: std::sync::Mutex<Vec<Observation>>,
}

#[async_trait]
impl Store for MemoryStore {
    async fn store(&self, obs: &Observation) -> anyhow::Result<String> {
        #[cfg(test)]
        {
            self.inner
                .lock()
                .expect("store lock poisoned")
                .push(obs.clone());
        }
        // Without the test feature the store is a pure pass-through; the real brain is agidb.
        let _ = obs;
        Ok(obs.id.to_string())
    }
}

#[cfg(test)]
impl MemoryStore {
    /// Snapshot of every stored observation, in insertion order.
    pub fn snapshot(&self) -> Vec<Observation> {
        self.inner.lock().expect("store lock poisoned").clone()
    }
}

/// The agidb-backed store. Gated behind `brain-agidb` so the workspace builds without agidb.
#[cfg(feature = "brain-agidb")]
pub struct AgidbStore {
    brain: std::sync::Arc<agidb::Agidb>,
}

#[cfg(feature = "brain-agidb")]
impl AgidbStore {
    /// Wrap an existing agidb handle.
    pub fn new(brain: std::sync::Arc<agidb::Agidb>) -> Self {
        Self { brain }
    }
}

#[cfg(feature = "brain-agidb")]
#[async_trait]
impl Store for AgidbStore {
    async fn store(&self, obs: &Observation) -> anyhow::Result<String> {
        // agidb.observe takes the (post-redaction) text + a source provenance tag. The store
        // owns the redaction ledger back-fill via the observation's redactions, which it ignores
        // here (agidb's own schema will hold them when the integration is complete).
        self.brain
            .observe_with(&obs.text, &obs.provenance.source_id)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(obs.id.to_string())
    }

    /// Forget via agidb's `unlearn(Episode(id))` — episode-level for all three scopes. Entity
    /// and Pattern resolve the target episodes with `recall_cue`; TimeRange lists episodes in
    /// the window with `list_episodes_in_range`. Each match is unlearned individually; the
    /// receipt records the total removed (Invariant 3 — proves scope without re-exposing content).
    ///
    /// v1 note: this is episode-level forget, not a concept-cascade (beliefs/atoms referencing a
    /// concept aren't removed). True concept-level forget needs `Agidb::concept_id_for` +
    /// `UnlearnTarget::Concept` — a follow-up.
    async fn forget(
        &self,
        scope: &homn_types::ForgetScope,
    ) -> anyhow::Result<homn_types::DeletionReceipt> {
        use agidb::UnlearnTarget;
        use homn_types::ForgetScope;

        // Collect the episode ids to forget, by scope.
        let ids: Vec<agidb::EpisodeId> = match scope {
            ForgetScope::Entity(name) => episode_ids_for_cue(&self.brain, name).await?,
            ForgetScope::Pattern(p) => episode_ids_for_cue(&self.brain, p).await?,
            ForgetScope::TimeRange { from, to } => {
                let eps = self
                    .brain
                    .timeline(None, *from, *to, 500)
                    .await
                    .map_err(|e| anyhow::anyhow!(e))?;
                eps.into_iter().map(|e| e.id).collect()
            }
        };

        // Unlearn each. agidb's unlearn is per-target; sum the removed episodes.
        let mut removed: u64 = 0;
        for id in ids {
            let report = self
                .brain
                .unlearn(UnlearnTarget::Episode(id), "homn forget")
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            removed += report.episodes_removed as u64;
        }

        Ok(homn_types::DeletionReceipt {
            scope: scope.clone(),
            match_count: removed,
            at: chrono::Utc::now(),
        })
    }
}

/// Resolve a cue (entity name or pattern) to the episode ids that recall surfaces for it.
#[cfg(feature = "brain-agidb")]
async fn episode_ids_for_cue(
    brain: &agidb::Agidb,
    cue: &str,
) -> anyhow::Result<Vec<agidb::EpisodeId>> {
    let recall = brain
        .recall_cue(cue.to_owned())
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(recall.matches.into_iter().map(|m| m.episode_id).collect())
}
