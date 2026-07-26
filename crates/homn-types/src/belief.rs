//! A `Belief` — a position on a topic, with a revision history (US5 / US6). Write-time only
//! (FR-020), like [`crate::Commitment`]. Extracted from captured text by a backend (regex v1,
//! local Ollama / cloud later) and stored so `beliefs(topic)` returns the current position
//! plus how it changed. See [`specs/002-ambient-memory/data-model.md`] §Belief.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Identifier for a belief record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BeliefId(pub Ulid);

impl BeliefId {
    /// Mint a new id.
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl std::fmt::Display for BeliefId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for BeliefId {
    fn default() -> Self {
        Self::new()
    }
}

/// A single position on a topic, taken at a point in time. Later positions on the same topic
/// `supersede` earlier ones (revision history); `superseded_at` is set when a newer position
/// replaces this one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Belief {
    /// Stable id.
    pub id: BeliefId,
    /// The topic this belief is about (e.g. "the brain", "the pricing").
    pub topic: String,
    /// The position taken (e.g. "agidb recall is shaky — maybe merge ctxgraph").
    pub position: String,
    /// Confidence in `[0.0, 1.0]`; the regex extractor records `1.0` (an explicit assertion).
    pub confidence: f32,
    /// When this position became the agent's belief (the observation's captured_at).
    pub valid_from: DateTime<Utc>,
    /// Set when a newer position on the same topic supersedes this one (the new position's
    /// `valid_from` minus 1ms, so the intervals are contiguous). `None` = the current belief.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<DateTime<Utc>>,
    /// The observation this was extracted from (provenance).
    pub source_obs: String,
    /// What extracted it (regex / local / cloud).
    pub extracted_by: crate::ExtractionSource,
    /// Present iff `extracted_by == Cloud` (Invariant 4 — proves what was disclosed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosure_receipt: Option<String>,
    /// When the belief was first recorded.
    pub created_at: DateTime<Utc>,
}

impl Belief {
    /// Is this the current (non-superseded) belief on its topic?
    pub fn is_current(&self) -> bool {
        self.superseded_at.is_none()
    }
}
