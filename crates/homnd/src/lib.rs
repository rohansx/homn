//! `homnd` — the ambient-memory ingestion daemon (Phase 1).
//!
//! Built on the tested `homn-daemon` chassis (Tokio runtime, unix sockets, event bus, supervisor).
//! The pipeline is: [`Source`](homn_sources::Source) → gate ([`homn_gate`]) → dedupe → sessionize →
//! store, with crash-safe watermarks and bounded-channel backpressure. See
//! [`specs/002-ambient-memory/contracts/gate-pipeline.md`] and plan Phase 1.
//!
//! This crate is scaffolded now; the pipeline, dedupe, sessionizer, and store trait are implemented
//! in Phase 1 (tasks T022–T030), tests-first per Constitution VI. Driven as a subcommand through
//! `homn-bin` (Constitution VII: one binary).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Phase-1 pipeline stages, in order, for reference by the implementation tasks.
///
/// Kept as a typed enum so the daemon loop and its tests share one vocabulary for where an item is
/// in the pipeline. The gate stage is where Invariant 1 is enforced (see [`homn_gate`]).
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
        // Structural reminder of the load-bearing ordering: the gate precedes the store, always.
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
