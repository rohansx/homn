//! `homnd` — the ambient-memory ingestion daemon (Phase 1).
//!
//! Built on the tested `homn-daemon` chassis (Tokio runtime, unix sockets, event bus, supervisor).
//! The pipeline is: [`Source`](homn_sources::Source) → gate ([`homn_gate`]) → dedupe → sessionize →
//! store, with crash-safe watermarks and bounded-channel backpressure. See
//! [`specs/002-ambient-memory/contracts/gate-pipeline.md`] and plan Phase 1.
//!
//! Modules:
//! - [`store`] — the `Store` trait (`MemoryStore` default; `AgidbStore` behind `brain-agidb`)
//! - [`session`] — the v1 sessionizer (app-focus / meeting episodes)
//! - [`dedupe`] — near-duplicate collapse over the post-redaction content hash
//! - [`pipeline`] — [`Pipeline`] + [`pipeline::drain`], the per-source run loop
//!
//! The v1 binary surface (`homn capture start/stop`, `homn status`, …) lives in `homn-bin`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod dedupe;
pub mod pipeline;
pub mod session;
pub mod store;

pub use dedupe::Dedupe;
pub use pipeline::{drain, Pipeline, PipelineStats, Processed, TickResult};
pub use session::Sessionizer;
pub use store::{MemoryStore, Store};

#[cfg(feature = "brain-agidb")]
pub use store::AgidbStore;

/// Pipeline stages, in order, for reference. The gate stage is where Invariant 1 is enforced
/// (see [`homn_gate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// A source produced a pre-gate `RawCapture`.
    Fetched,
    /// The gate ran policy + redaction (fail-closed).
    Gated,
    /// Content-hash + shingle dedupe.
    Deduped,
    /// Session boundary assigned.
    Sessionized,
    /// Persisted to the store; watermark may now advance.
    Stored,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_order_is_gate_before_store() {
        let order = [
            Stage::Fetched,
            Stage::Gated,
            Stage::Deduped,
            Stage::Sessionized,
            Stage::Stored,
        ];
        let gate = order.iter().position(|s| *s == Stage::Gated).unwrap();
        let store = order.iter().position(|s| *s == Stage::Stored).unwrap();
        assert!(gate < store, "gate must precede store (Invariant 1)");
    }
}
