//! The `Source` abstraction for homn's ingestion spine.
//!
//! One trait covers both **tail** sources (Screenpipe sqlite: cursor = last row id) and **poll**
//! sources (Phase 3.5 Gmail/Slack/GitHub connectors: cursor = history id / `oldest` ts / events
//! cursor). Keeping the cursor opaque and serializable is what lets connectors land later without a
//! breaking trait change (FR-005a). See
//! [`specs/002-ambient-memory/contracts/source-trait.md`].
//!
//! A `Source` emits **pre-gate** [`RawCapture`] items. It never redacts, persists, or decides
//! policy — that is the daemon + gate's job, which keeps Invariant 1 enforced in exactly one place.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use async_trait::async_trait;
use homn_types::{Cursor, RawCapture, SourceKind};

/// Errors a source can raise while fetching.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// The underlying store / provider could not be reached or read.
    #[error("source unavailable: {0}")]
    Unavailable(String),
    /// The cursor could not be interpreted by this source.
    #[error("invalid cursor: {0}")]
    InvalidCursor(String),
    /// Any other source-specific failure.
    #[error("source error: {0}")]
    Other(String),
}

/// A batch of pre-gate items plus the advanced cursor.
#[derive(Debug, Clone)]
pub struct Batch {
    /// Pre-gate items. `text` may contain secrets/PII until the gate runs.
    pub items: Vec<RawCapture>,
    /// The advanced resume position. Persisted by the daemon **only after** these items are
    /// gated + stored (or dropped by policy).
    pub next: Cursor,
    /// `true` when the source has no more items right now and the caller may back off.
    pub exhausted: bool,
}

/// A capture source: a swappable producer of pre-gate observations.
///
/// Implementations must be:
/// - **read-only upstream** — never mutate Screenpipe's sqlite or a provider's data;
/// - **cursor-monotonic** — `next` never precedes the input cursor, so a crash re-read returns a
///   superset that dedupe collapses (at-least-once upstream, exactly-once stored);
/// - **gate-blind** — emit `RawCapture`, never persist or redact;
/// - **backpressure-friendly** — return bounded batches; let the daemon control cadence.
#[async_trait]
pub trait Source: Send + Sync {
    /// Stable id used to key this source's watermark row.
    fn id(&self) -> &str;

    /// The kind of observations this source produces (may vary per item for multi-kind stores;
    /// this is the source's *primary* kind).
    fn kind(&self) -> SourceKind;

    /// Fetch items strictly after `cursor` (`None` = from the beginning / a sane default window).
    async fn fetch_since(&self, cursor: Option<&Cursor>) -> Result<Batch, SourceError>;
}
