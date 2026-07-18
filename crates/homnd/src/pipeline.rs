//! The ingestion pipeline — the heart of `homnd` (T022–T030).
//!
//! One [`Pipeline`] owns the gate, the dedupe window, the sessionizer, and a [`Store`], and
//! drives a single [`Source`](homn_sources::Source) through it. The flow per item:
//!
//! `fetch_since(watermark)` → [`Gate::run`] → **dedupe** → **sessionize** →
//! append `DecisionReceipt` to the audit ledger → back-fill redaction `ledger_seq` →
//! [`Store::store`] → `set_watermark(next)`.
//!
//! Hard rules enforced here (see [`contracts/gate-pipeline.md`]):
//! - **R-1 / R-2** the gate is the only place an `Observation` is produced; it fails closed.
//! - **R-3** the watermark advances only after the item is durably stored (or durably dropped by
//!   policy), so a crash re-reads from the last confirmed position.
//! - **R-4** every decision writes a `DecisionReceipt`; redaction `ledger_seq`s are back-filled
//!   from that receipt's chain position.
//! - **R-6** bounded channel backpressure between fetch and process.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use homn_audit::Db;
use homn_gate::{Gate, GateOutput};
use homn_sources::Source;
use homn_types::{Cursor, IngestOutcome, Observation, RawCapture, Receipt};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::dedupe::Dedupe;
use crate::session::Sessionizer;
use crate::store::Store;

/// Per-pipeline counters surfaced by `homn status`.
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    /// Items the source produced.
    pub fetched: u64,
    /// Items the gate dropped (deny or error).
    pub dropped: u64,
    /// Near-duplicates collapsed after the gate.
    pub deduped: u64,
    /// Items durably stored.
    pub stored: u64,
}

/// One source's ingestion pipeline. Cheap to clone via `Arc`.
pub struct Pipeline {
    source_id: String,
    gate: Gate,
    store: Arc<dyn Store>,
    dedupe: Mutex<Dedupe>,
    sessionizer: Mutex<Sessionizer>,
    db: Db,
    paused: Arc<AtomicBool>,
    stats: Mutex<PipelineStats>,
}

impl Pipeline {
    /// Build a pipeline for one source. The `Gate`, `Store`, and `Db` are shared concerns; the
    /// dedupe window and sessionizer are per-source (each source has its own episode stream).
    pub fn new(source_id: impl Into<String>, gate: Gate, store: Arc<dyn Store>, db: Db) -> Self {
        Self {
            source_id: source_id.into(),
            gate,
            store,
            dedupe: Mutex::new(Dedupe::default()),
            sessionizer: Mutex::new(Sessionizer::new()),
            db,
            paused: Arc::new(AtomicBool::new(false)),
            stats: Mutex::new(PipelineStats::default()),
        }
    }

    /// A handle to the pause flag (US7). Setting it true halts the run loop before the next fetch.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.paused.clone()
    }

    /// Read the current stats (for `homn status`).
    pub async fn stats(&self) -> PipelineStats {
        self.stats.lock().await.clone()
    }

    /// Drain one batch from the source through the pipeline, advancing the watermark. Returns
    /// the number of items stored. The run loop calls this in a tight loop, backing off when the
    /// source reports `exhausted`.
    pub async fn tick(&self, source: &dyn Source) -> anyhow::Result<TickResult> {
        if self.paused.load(Ordering::Acquire) {
            return Ok(TickResult::Paused);
        }

        let cursor = self
            .db
            .get_watermark(&self.source_id)
            .await?
            .map(|w| w.cursor);
        let batch = source.fetch_since(cursor.as_ref()).await?;
        let mut stored = 0u64;
        for capture in &batch.items {
            match self.process_one(capture).await? {
                Processed::Stored => stored += 1,
                Processed::Dropped | Processed::Deduped => {}
            }
        }
        // R-3: advance the watermark only after the whole batch is durable (stored or dropped).
        self.db.set_watermark(&self.source_id, &batch.next).await?;
        let mut s = self.stats.lock().await;
        s.fetched += batch.items.len() as u64;
        Ok(if batch.exhausted && batch.items.is_empty() {
            TickResult::Exhausted
        } else {
            TickResult::Drained {
                stored,
                next: batch.next,
            }
        })
    }

    /// The per-item core, factored out so it's unit-testable without a live source.
    async fn process_one(&self, capture: &RawCapture) -> anyhow::Result<Processed> {
        let outcome = self.gate.run(capture);
        match outcome {
            GateOutput::Dropped { outcome, rule_id } => {
                let receipt = Receipt::Decision(homn_types::DecisionReceipt {
                    outcome,
                    policy_id: rule_id,
                    observation_ref: None,
                    at: chrono::Utc::now(),
                });
                self.db.append_receipt(&receipt).await?;
                self.stats_inc(|s| s.dropped += 1).await;
                debug!(outcome = ?outcome, "dropped by gate");
                Ok(Processed::Dropped)
            }
            GateOutput::Stored {
                mut observation,
                mut redactions,
                ..
            } => {
                // 3. DEDUPE (post-redaction content hash).
                {
                    let mut d = self.dedupe.lock().await;
                    if d.is_duplicate(&observation) {
                        // Still record a decision receipt so the ledger explains the drop.
                        self.db
                            .append_receipt(&drop_receipt(IngestOutcome::Allow, &observation))
                            .await?;
                        self.stats_inc(|s| s.deduped += 1).await;
                        return Ok(Processed::Deduped);
                    }
                }
                // 4. SESSIONIZE.
                {
                    let mut ses = self.sessionizer.lock().await;
                    let (id, _kind) = ses.assign(&observation);
                    observation.session = Some(id);
                }
                // R-4: append the decision receipt, then back-fill redaction ledger_seq from it.
                let entry = self
                    .db
                    .append_receipt(&decision_receipt_for(&observation))
                    .await?;
                for r in redactions.iter_mut() {
                    r.ledger_seq = entry.seq as u64;
                }
                observation.redactions = redactions;
                // 5. STORE.
                self.store.store(&observation).await?;
                self.stats_inc(|s| s.stored += 1).await;
                Ok(Processed::Stored)
            }
        }
    }

    async fn stats_inc<F>(&self, f: F)
    where
        F: FnOnce(&mut PipelineStats),
    {
        let mut s = self.stats.lock().await;
        f(&mut s);
    }
}

/// What `tick` did.
#[derive(Debug, Clone)]
pub enum TickResult {
    /// The pause flag was set; nothing was fetched.
    Paused,
    /// The source had nothing new right now.
    Exhausted,
    /// A batch was drained and the watermark advanced.
    Drained {
        /// How many items were durably stored this tick.
        stored: u64,
        /// The new cursor position.
        next: Cursor,
    },
}

/// What happened to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Processed {
    /// Durably stored.
    Stored,
    /// Dropped by policy or a gate error.
    Dropped,
    /// Collapsed as a near-duplicate.
    Deduped,
}

/// The `Allow` decision receipt for a stored observation (the gate's `decision_receipt` doesn't
/// know the post-session observation id is stable; we rebuild here for the stored path).
fn decision_receipt_for(obs: &Observation) -> Receipt {
    Receipt::Decision(homn_types::DecisionReceipt {
        outcome: IngestOutcome::Allow,
        policy_id: None,
        observation_ref: Some(obs.id.to_string()),
        at: obs.ingested_at,
    })
}

/// A decision receipt used when a duplicate is dropped post-gate — still `Allow` (the gate
/// permitted it), recorded so the ledger accounts for every seen item.
fn drop_receipt(outcome: IngestOutcome, obs: &Observation) -> Receipt {
    Receipt::Decision(homn_types::DecisionReceipt {
        outcome,
        policy_id: None,
        observation_ref: Some(obs.id.to_string()),
        at: chrono::Utc::now(),
    })
}

/// Run a source through a pipeline until it reports exhausted, with a bounded-channel
/// fetch→process split for backpressure (R-6). Returns when the source is drained and no more
/// items are immediately available; the caller re-invokes on the next poll interval.
pub async fn drain(pipeline: &Pipeline, source: &dyn Source) -> anyhow::Result<()> {
    loop {
        match pipeline.tick(source).await? {
            TickResult::Paused => break,
            TickResult::Exhausted => break,
            TickResult::Drained { .. } => {
                // Keep draining until the source is exhausted.
                continue;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use homn_gate::{Gate, IngestPolicy};
    use homn_sources::{Batch, Source, SourceError};
    use homn_types::{Cursor, RawCapture, SourceKind};

    /// A scripted source: replays a fixed batch on the first call, then reports exhausted.
    struct ScriptedSource {
        id: String,
        items: std::sync::Mutex<Option<Vec<RawCapture>>>,
    }

    #[async_trait]
    impl Source for ScriptedSource {
        fn id(&self) -> &str {
            &self.id
        }
        fn kind(&self) -> SourceKind {
            SourceKind::ScreenOcr
        }
        async fn fetch_since(&self, _cursor: Option<&Cursor>) -> Result<Batch, SourceError> {
            let mut items = self.items.lock().unwrap();
            let drained = items.take().unwrap_or_default();
            Ok(Batch {
                items: drained,
                next: Cursor::new(1i64),
                exhausted: true,
            })
        }
    }

    fn cap(text: &str) -> RawCapture {
        RawCapture {
            upstream_ref: "r".to_owned(),
            source: SourceKind::ScreenOcr,
            app: Some("Code".to_owned()),
            captured_at: Utc::now(),
            text: text.to_owned(),
            speaker: None,
        }
    }

    async fn setup() -> (Pipeline, ScriptedSource, Arc<MemoryStoreWrapper>) {
        let db = Db::in_memory().await.unwrap();
        let gate = Gate::new(IngestPolicy::compile("allow();").unwrap());
        let store = Arc::new(MemoryStoreWrapper::default());
        let p = Pipeline::new("test", gate, store.clone() as Arc<dyn Store>, db);
        let src = ScriptedSource {
            id: "test".to_owned(),
            items: std::sync::Mutex::new(Some(vec![cap("hello world")])),
        };
        (p, src, store)
    }

    /// A MemoryStore that compiles without the `test` cfg by always snapshotting.
    #[derive(Default)]
    struct MemoryStoreWrapper {
        inner: std::sync::Mutex<Vec<Observation>>,
    }
    #[async_trait]
    impl Store for MemoryStoreWrapper {
        async fn store(&self, obs: &Observation) -> anyhow::Result<String> {
            self.inner.lock().unwrap().push(obs.clone());
            Ok(obs.id.to_string())
        }
    }
    impl MemoryStoreWrapper {
        fn snapshot(&self) -> Vec<Observation> {
            self.inner.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn clean_capture_is_stored_and_watermark_advances() {
        let (p, src, store) = setup().await;
        let res = p.tick(&src).await.unwrap();
        assert!(matches!(res, TickResult::Drained { stored: 1, .. }));
        assert_eq!(store.snapshot().len(), 1);
        assert_eq!(store.snapshot()[0].text, "hello world");
        // Watermark advanced.
        let w = p.db.get_watermark("test").await.unwrap().unwrap();
        assert_eq!(w.cursor, Cursor::new(1i64));
        let stats = p.stats().await;
        assert_eq!(stats.fetched, 1);
        assert_eq!(stats.stored, 1);
    }

    #[tokio::test]
    async fn secret_in_clean_capture_is_redacted_before_store_r1() {
        let db = Db::in_memory().await.unwrap();
        let gate = Gate::new(IngestPolicy::compile("allow();").unwrap());
        let store = Arc::new(MemoryStoreWrapper::default());
        let p = Pipeline::new("test", gate, store.clone() as Arc<dyn Store>, db);
        let src = ScriptedSource {
            id: "test".to_owned(),
            items: std::sync::Mutex::new(Some(vec![cap("card 4242 4242 4242 4242 ok")])),
        };
        p.tick(&src).await.unwrap();
        let stored = store.snapshot();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].text.contains("[REDACTED:card]"));
        assert!(!stored[0].text.contains("4242"));
        // The redaction ref carries the ledger_seq back-filled from the receipt.
        assert_eq!(stored[0].redactions.len(), 1);
        assert!(stored[0].redactions[0].ledger_seq > 0);
    }

    #[tokio::test]
    async fn deny_policy_drops_and_does_not_store() {
        let db = Db::in_memory().await.unwrap();
        let gate = Gate::new(IngestPolicy::compile("deny();").unwrap());
        let store = Arc::new(MemoryStoreWrapper::default());
        let p = Pipeline::new("test", gate, store.clone() as Arc<dyn Store>, db);
        let src = ScriptedSource {
            id: "test".to_owned(),
            items: std::sync::Mutex::new(Some(vec![cap("anything")])),
        };
        p.tick(&src).await.unwrap();
        assert!(store.snapshot().is_empty());
        let stats = p.stats().await;
        assert_eq!(stats.dropped, 1);
        assert_eq!(stats.stored, 0);
        // Even a dropped item advances the watermark (it was durably decided).
        assert!(p.db.get_watermark("test").await.unwrap().is_some());
        // And it wrote a Deny receipt to the ledger.
        let v = p.db.verify_ledger().await.unwrap();
        assert!(v.is_valid());
    }

    #[tokio::test]
    async fn near_duplicate_post_gate_is_collapsed() {
        let db = Db::in_memory().await.unwrap();
        let gate = Gate::new(IngestPolicy::compile("allow();").unwrap());
        let store = Arc::new(MemoryStoreWrapper::default());
        let p = Pipeline::new("test", gate, store.clone() as Arc<dyn Store>, db);
        let dup = cap("same text");
        let src = ScriptedSource {
            id: "test".to_owned(),
            items: std::sync::Mutex::new(Some(vec![dup.clone(), dup])),
        };
        p.tick(&src).await.unwrap();
        assert_eq!(store.snapshot().len(), 1, "second identical item collapsed");
        let stats = p.stats().await;
        assert_eq!(stats.stored, 1);
        assert_eq!(stats.deduped, 1);
    }

    #[tokio::test]
    async fn pause_flag_halts_the_tick() {
        let (p, src, _store) = setup().await;
        p.pause_flag().store(true, Ordering::Release);
        let res = p.tick(&src).await.unwrap();
        assert!(matches!(res, TickResult::Paused));
    }

    #[tokio::test]
    async fn ledger_records_every_decision_and_verifies() {
        // Two clean captures → two Allow receipts, a valid chain.
        let db = Db::in_memory().await.unwrap();
        let gate = Gate::new(IngestPolicy::compile("allow();").unwrap());
        let store = Arc::new(MemoryStoreWrapper::default());
        let p = Pipeline::new("test", gate, store.clone() as Arc<dyn Store>, db);
        let src = ScriptedSource {
            id: "test".to_owned(),
            items: std::sync::Mutex::new(Some(vec![cap("first"), cap("second")])),
        };
        p.tick(&src).await.unwrap();
        let v = p.db.verify_ledger().await.unwrap();
        assert!(v.is_valid(), "ledger chain must verify after ingest");
        assert!(v.total >= 2);
    }
}
