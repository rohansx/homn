//! The privacy gate — Phase 2 (US3).
//!
//! The single place where Invariant 1 (nothing unredacted touches disk) is enforced: every captured
//! item passes POLICY → REDACT before it can become a storable [`Observation`](homn_types::Observation),
//! and the gate **fails closed** on any error. See
//! [`specs/002-ambient-memory/contracts/gate-pipeline.md`].
//!
//! Modules:
//! - [`redaction`] — the redaction bank (regex detectors + placeholders, always-on secrets scan)
//! - [`policy`] — the Rhai ingest policy (`allow` / `deny` / `redact(kinds)` / `allow_cloud`),
//!   hot-reloadable, fail-closed
//! - [`pipeline`] — [`Gate::run`], the only constructor of a stored `Observation`

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod pipeline;
pub mod policy;
pub mod redaction;

pub use pipeline::{decision_receipt, Gate, GateOutput};
pub use policy::{
    spawn_reloader as spawn_policy_reloader, IngestContext, IngestPolicy, IngestPolicyHandle,
    IngestPolicyReloader, PolicyDecision,
};
pub use redaction::{Redacted, RedactionBank, RedactionSpan};

/// The action an ingest policy resolves to for a captured item.
///
/// Defined here (not just in Rhai) so the pipeline can pattern-match the outcome and the audit
/// receipt can name it. `AllowCloud` is the only action that permits a later write-time cloud
/// disclosure, and only for the named redaction kinds (Invariant 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestAction {
    /// Never capture this item.
    Deny,
    /// Capture, but redact the given kinds first.
    Redact(Vec<homn_types::RedactionKind>),
    /// Capture as-is (still subject to the always-on secrets scan).
    Allow,
    /// Capture and permit write-time cloud enrichment on the redacted text.
    AllowCloud,
}

impl IngestAction {
    /// Whether this action permits the item to be persisted at all.
    pub fn persists(&self) -> bool {
        !matches!(self, IngestAction::Deny)
    }

    /// Whether this action permits a later cloud disclosure of the (redacted) item.
    pub fn permits_cloud(&self) -> bool {
        matches!(self, IngestAction::AllowCloud)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_does_not_persist_and_never_permits_cloud() {
        assert!(!IngestAction::Deny.persists());
        assert!(!IngestAction::Deny.permits_cloud());
        assert!(IngestAction::Allow.persists());
        assert!(!IngestAction::Allow.permits_cloud());
        assert!(IngestAction::AllowCloud.permits_cloud());
    }
}