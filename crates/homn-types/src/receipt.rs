//! Receipt types — the audit records that make the invariants verifiable.
//!
//! Three kinds, all destined for the hash-chained `homn-audit` ledger, all **plaintext-free**:
//! a [`DecisionReceipt`] for every ingest/policy decision (Constitution III), a
//! [`DisclosureReceipt`] for every cloud call (Invariant 4), and a [`DeletionReceipt`] for every
//! `forget` (Invariant 3). See [`specs/002-ambient-memory/data-model.md`].
//!
//! This crate defines the *shapes*; the `homn-audit` crate owns persistence and the hash chain.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Opaque identifier for a receipt row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReceiptId(pub String);

/// The outcome of an ingest/policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestOutcome {
    /// Item captured and stored (possibly after redaction).
    Allow,
    /// Item denied by policy — never stored.
    Deny,
    /// Item stored with one or more redactions applied.
    Redact,
    /// The gate errored; item dropped (fail-closed).
    Error,
}

/// The scope of a `forget` operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ForgetScope {
    /// Forget everything about a named entity.
    Entity(String),
    /// Forget everything captured within a time range.
    TimeRange {
        /// Inclusive start.
        from: DateTime<Utc>,
        /// Inclusive end.
        to: DateTime<Utc>,
    },
    /// Forget everything matching a pattern.
    Pattern(String),
}

/// Record of one ingest/policy decision (allow / deny / redact / error).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionReceipt {
    /// The outcome.
    pub outcome: IngestOutcome,
    /// The policy rule that fired, if any.
    pub policy_id: Option<String>,
    /// The observation this decision concerns, if one was produced.
    pub observation_ref: Option<String>,
    /// When the decision was made.
    pub at: DateTime<Utc>,
}

/// Record of one cloud disclosure — what left the gate, under which policy, when.
///
/// Carries only a digest of the disclosed (already-redacted) payload, never the payload itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisclosureReceipt {
    /// The policy that authorized the disclosure (an `AllowCloud` rule).
    pub policy_id: String,
    /// The model the payload was disclosed to (e.g. "claude-haiku").
    pub model: String,
    /// A digest of the redacted payload that was sent (not the payload).
    pub payload_digest: String,
    /// When the disclosure happened.
    pub at: DateTime<Utc>,
}

/// Record of one `forget` — proves what was removed without re-exposing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionReceipt {
    /// What was targeted.
    pub scope: ForgetScope,
    /// How many memories matched and were removed.
    pub match_count: u64,
    /// When the deletion happened.
    pub at: DateTime<Utc>,
}

/// Any of the three receipt kinds, as stored in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Receipt {
    /// An ingest/policy decision.
    Decision(DecisionReceipt),
    /// A cloud disclosure.
    Disclosure(DisclosureReceipt),
    /// A deletion.
    Deletion(DeletionReceipt),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_receipt_carries_scope_not_content() {
        let r = Receipt::Deletion(DeletionReceipt {
            scope: ForgetScope::Entity("Test Person".to_owned()),
            match_count: 4,
            at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap(),
        });
        let json = serde_json::to_string(&r).unwrap();
        let back: Receipt = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
        // It records scope + count, sufficient to prove what was removed, without the memories.
        assert!(json.contains("\"match_count\":4"));
        assert!(json.contains("deletion"));
    }

    #[test]
    fn forget_scope_variants_round_trip() {
        let scopes = [
            ForgetScope::Entity("x".to_owned()),
            ForgetScope::Pattern("*.secret".to_owned()),
            ForgetScope::TimeRange {
                from: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
                to: DateTime::<Utc>::from_timestamp(10, 0).unwrap(),
            },
        ];
        for s in scopes {
            let json = serde_json::to_string(&s).unwrap();
            let back: ForgetScope = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }
}
