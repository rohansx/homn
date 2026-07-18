//! The privacy gate — Phase 2 (US3).
//!
//! The single place where Invariant 1 (nothing unredacted touches disk) is enforced: every captured
//! item passes POLICY → REDACT before it can become a storable [`Observation`](homn_types::Observation),
//! and the gate **fails closed** on any error. See
//! [`specs/002-ambient-memory/contracts/gate-pipeline.md`].
//!
//! This crate is scaffolded now; the redaction stage (cloakpipe), the Rhai ingest-policy stage
//! (homn-policy), and the hash-chained ledger (homn-audit) are implemented in Phase 2 — tasks
//! T034–T042, tests-first per Constitution VI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

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
