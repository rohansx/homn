//! A `Commitment` — an extracted promise (mine or theirs), write-time only (FR-020 / US5).
//!
//! Extracted from captured text by a [`crate::extract`] backend (regex v1, cloud Haiku or local
//! qwen later) and stored queryably so `commitments(status?, due_before?)` answers without a
//! recall pass over raw text. See [`specs/002-ambient-memory/data-model.md`] §Commitment.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Who owns a commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum Party {
    /// A commitment I made.
    Me,
    /// A commitment someone else made (to me or a third party).
    Entity(String),
}

/// Lifecycle of a commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentStatus {
    /// Outstanding; due in the future or undated.
    Open,
    /// Done — the promise was kept (e.g. "sent the quote" follows "I'll send the quote").
    Fulfilled,
    /// Past due and not fulfilled.
    Overdue,
    /// Explicitly cancelled / withdrawn.
    Cancelled,
}

/// Identifier for a commitment (sortable, time-ordered ULID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitmentId(pub Ulid);

impl CommitmentId {
    /// Mint a new id.
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl std::fmt::Display for CommitmentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for CommitmentId {
    fn default() -> Self {
        Self::new()
    }
}

/// What extracted the commitment (Invariant 4 traceability — `Cloud` requires a
/// `disclosure_receipt`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "backend", content = "model")]
pub enum ExtractionSource {
    /// The deterministic regex extractor (v1, no network, no disclosure).
    Regex,
    /// A future local model backend (e.g. qwen2.5:3b via Ollama) — no disclosure.
    Local(String),
    /// A cloud model backend (e.g. claude-haiku) — REQUIRES a `disclosure_receipt`.
    Cloud {
        /// The model that saw the (already-redacted) text.
        model: String,
    },
}

/// An extracted promise. `text` is post-redaction (Invariant 1); `source_obs` is the observation
/// the promise was extracted from (provenance, FR-018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commitment {
    /// Stable id.
    pub id: CommitmentId,
    /// The promise, post-redaction (e.g. "I'll send the pricing quote by Friday").
    pub text: String,
    /// Who made the promise.
    pub owner: Party,
    /// Who the promise is to, if known.
    pub counterpart: Option<Party>,
    /// When the promise is due, if a date/time was extractable.
    pub due: Option<DateTime<Utc>>,
    /// Lifecycle state.
    pub status: CommitmentStatus,
    /// The observation this was extracted from (provenance).
    pub source_obs: String,
    /// What extracted it (regex / local / cloud).
    pub extracted_by: ExtractionSource,
    /// Present iff `extracted_by == Cloud` (Invariant 4 — proves what was disclosed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosure_receipt: Option<String>,
    /// When the commitment was first recorded.
    pub created_at: DateTime<Utc>,
}

impl Commitment {
    /// Is this commitment overdue (past due, not fulfilled/cancelled)?
    pub fn is_overdue(&self, now: DateTime<Utc>) -> bool {
        matches!(self.status, CommitmentStatus::Open) && self.due.is_some_and(|d| d < now)
    }
}
