//! The [`Observation`] — the normalized, post-gate unit of captured activity.
//!
//! An `Observation` is what reaches the memory store. It is constructed **only** from
//! already-redacted text: the gate returns observations, raw capture does not. See
//! [`specs/002-ambient-memory/data-model.md`] and the five invariants in the spec.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{RedactionRef, SessionId, SourceKind};

/// Who was speaking, for audio-sourced observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerTag {
    /// The user themselves.
    Me,
    /// Someone else.
    Other,
    /// Speaker could not be determined.
    Unknown,
}

/// Where an observation came from, sufficient to trace it back to its capture source and time.
///
/// This is the structural expression of Invariant 3's first clause ("every memory has provenance").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The source instance id (matches the watermark's `source_id`).
    pub source_id: String,
    /// The upstream anchor (row id / message id / event id) this observation was derived from.
    pub upstream_ref: String,
}

/// The normalized unit of captured activity, **after** it has passed the gate.
///
/// `text` is always post-redaction; `redactions` are plaintext-free references into the audit
/// ledger. `content_hash` is the dedupe key. There is deliberately no constructor that takes
/// pre-gate text — the gate is the only producer of `Observation`s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// Sortable, time-ordered id.
    pub id: Ulid,
    /// Which source produced the underlying capture.
    pub source: SourceKind,
    /// App / window / account, if known.
    pub app: Option<String>,
    /// Valid-time start: when the activity happened.
    pub captured_at: DateTime<Utc>,
    /// Transaction-time start: when homn recorded it (bi-temporal).
    pub ingested_at: DateTime<Utc>,
    /// Post-redaction text. Never contains unredacted secrets or third-party PII.
    pub text: String,
    /// Plaintext-free references to what was stripped from `text`.
    pub redactions: Vec<RedactionRef>,
    /// Session / episode grouping, if assigned by the sessionizer.
    pub session: Option<SessionId>,
    /// For audio: who was speaking.
    pub speaker: Option<SpeakerTag>,
    /// Dedupe key over the post-redaction content + source + app.
    pub content_hash: u64,
    /// Trace back to the capture source and upstream item.
    pub provenance: Provenance,
}

impl Observation {
    /// Compute the dedupe content hash for a piece of post-redaction content.
    ///
    /// Uses xxh3 — a fast, non-cryptographic hash appropriate for near-duplicate collapse. (The
    /// tamper-evident hash *chain* in the audit ledger is a separate, cryptographic concern.)
    pub fn compute_content_hash(source: SourceKind, app: Option<&str>, text: &str) -> u64 {
        use xxhash_rust::xxh3::Xxh3;
        let mut h = Xxh3::new();
        // Domain-separate the fields so identical text from different apps hashes differently.
        h.update(&(source as u32).to_le_bytes());
        h.update(app.unwrap_or("").as_bytes());
        h.update(&[0u8]); // separator
        h.update(text.as_bytes());
        h.digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_field_separated() {
        let a = Observation::compute_content_hash(SourceKind::ScreenOcr, Some("Slack"), "hello");
        let b = Observation::compute_content_hash(SourceKind::ScreenOcr, Some("Slack"), "hello");
        assert_eq!(
            a, b,
            "identical inputs must hash identically (dedupe correctness)"
        );

        let diff_app =
            Observation::compute_content_hash(SourceKind::ScreenOcr, Some("Zoom"), "hello");
        assert_ne!(
            a, diff_app,
            "same text from a different app must not collide"
        );

        let diff_src =
            Observation::compute_content_hash(SourceKind::Dictation, Some("Slack"), "hello");
        assert_ne!(
            a, diff_src,
            "same text from a different source must not collide"
        );
    }

    #[test]
    fn observation_round_trips() {
        let obs = Observation {
            id: Ulid::from_parts(1, 2),
            source: SourceKind::Dictation,
            app: Some("Zoom".to_owned()),
            captured_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap(),
            ingested_at: DateTime::<Utc>::from_timestamp(1_700_000_005, 0).unwrap(),
            text: "the quote is [REDACTED]".to_owned(),
            redactions: vec![],
            session: Some(SessionId::new("01HXY0")),
            speaker: Some(SpeakerTag::Me),
            content_hash: 123,
            provenance: Provenance {
                source_id: "dictation".to_owned(),
                upstream_ref: "seq-9".to_owned(),
            },
        };
        let json = serde_json::to_string(&obs).unwrap();
        let back: Observation = serde_json::from_str(&json).unwrap();
        assert_eq!(obs, back);
    }
}
