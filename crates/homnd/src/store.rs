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
}
