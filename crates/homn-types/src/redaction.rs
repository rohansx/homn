//! Redaction references carried on an [`Observation`](crate::Observation).
//!
//! The full redaction *event* (with its hash-chain links) lives in the `homn-audit` ledger; what
//! rides on the observation is a plaintext-free [`RedactionRef`]. See
//! [`specs/002-ambient-memory/data-model.md`]. Invariant 1: nothing unredacted touches disk, and no
//! redaction record ever stores the original bytes.

use serde::{Deserialize, Serialize};

/// The category of a redacted span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionKind {
    /// An API key or similar high-entropy credential.
    ApiKey,
    /// An auth token / bearer token.
    Token,
    /// A payment card number.
    Card,
    /// An Indian Aadhaar number.
    Aadhaar,
    /// An Indian PAN.
    Pan,
    /// Third-party personally identifiable information (name/email/phone of someone else).
    PersonPii,
    /// An email address.
    EmailAddr,
    /// A phone number.
    Phone,
    /// A catch-all for a detector-specific category not enumerated above.
    Other,
}

/// A plaintext-free reference to a redacted span in an observation's text.
///
/// `span` is a placeholder locator (offset+len into the *redacted* text, or a placeholder-token id),
/// never the original bytes. `ledger_seq` points at the hash-chained [`RedactionKind`] event row in
/// the audit ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionRef {
    /// What kind of sensitive content was removed.
    pub kind: RedactionKind,
    /// Locator of the placeholder in the redacted text (offset, length). Not the original content.
    pub span: SpanRef,
    /// Identifier of the policy / detector that caused the redaction.
    pub policy_id: String,
    /// Position of the corresponding event in the audit hash chain.
    pub ledger_seq: u64,
}

/// A locator for a placeholder in redacted text — an offset and length, never the original bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanRef {
    /// Byte offset of the placeholder in the redacted text.
    pub offset: u32,
    /// Byte length of the placeholder.
    pub len: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_ref_carries_no_plaintext() {
        let r = RedactionRef {
            kind: RedactionKind::ApiKey,
            span: SpanRef {
                offset: 10,
                len: 12,
            },
            policy_id: "rule.api_key".to_owned(),
            ledger_seq: 7,
        };
        let json = serde_json::to_string(&r).unwrap();
        // The serialized form is purely structural: kind, span, policy id, ledger seq. There is no
        // field that could carry the original sensitive bytes.
        assert!(json.contains("api_key"));
        assert!(json.contains("\"offset\":10"));
        assert!(json.contains("\"ledger_seq\":7"));
    }
}
