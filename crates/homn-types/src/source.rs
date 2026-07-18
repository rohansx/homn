//! Capture-source types for the v2 ambient-memory ingestion spine.
//!
//! A [`SourceKind`] tags where an observation came from. [`RawCapture`] is the pre-gate item a
//! `Source` emits (see the `homn-sources` crate); [`Cursor`] and [`Watermark`] are the crash-safe
//! resume machinery. See [`specs/002-ambient-memory/data-model.md`] and
//! [`specs/002-ambient-memory/contracts/source-trait.md`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a captured item originated.
///
/// The set is intentionally open-ended: the `Email`, `Slack`, and `GitHub` variants are **reserved
/// for the Phase 3.5 account connectors** and are not emitted by the v1 sources. Reserving them now
/// keeps the observation schema stable when connectors land (see FR-005a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Screen OCR text (Screenpipe).
    ScreenOcr,
    /// Accessibility-tree text (Screenpipe).
    A11yTree,
    /// Ambient microphone audio, transcribed (Screenpipe whisper).
    AmbientAudio,
    /// Push-based dictation (convox-voice), higher fidelity than ambient audio.
    Dictation,
    /// Reserved — Phase 3.5 Gmail connector. Not emitted in v1.
    Email,
    /// Reserved — Phase 3.5 Slack connector. Not emitted in v1.
    Slack,
    /// Reserved — Phase 3.5 GitHub connector. Not emitted in v1.
    GitHub,
}

impl SourceKind {
    /// Whether this source produces audio (and therefore may carry a speaker tag).
    pub fn is_audio(self) -> bool {
        matches!(self, SourceKind::AmbientAudio | SourceKind::Dictation)
    }

    /// Whether this variant is a Phase 3.5 account connector (reserved, not emitted in v1).
    pub fn is_connector(self) -> bool {
        matches!(
            self,
            SourceKind::Email | SourceKind::Slack | SourceKind::GitHub
        )
    }
}

/// An opaque, serializable resume position for a [`Source`](../../homn_sources/trait.Source.html).
///
/// A sqlite-tail source stores its last row id here; a poll-based connector stores a Gmail history
/// id / Slack `oldest` timestamp / GitHub events cursor. The ingestion daemon never interprets the
/// contents — it only persists and hands them back — which is what lets one `Source` trait cover
/// both tail and poll-cursor shapes without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cursor(pub serde_json::Value);

impl Cursor {
    /// Wrap any JSON-serializable value as a cursor.
    pub fn new(value: impl Into<serde_json::Value>) -> Self {
        Self(value.into())
    }
}

/// The crash-safe resume position for one source, persisted by the daemon.
///
/// Invariant: advanced only *after* an item is durably stored (or durably dropped by policy), so a
/// crash re-reads from here and dedupe collapses the replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watermark {
    /// Stable id of the source this watermark belongs to.
    pub source_id: String,
    /// The last durably-consumed position.
    pub cursor: Cursor,
    /// When the watermark was last advanced.
    pub updated_at: DateTime<Utc>,
}

/// A pre-gate captured item as emitted by a `Source`.
///
/// **Warning:** `text` is pre-redaction and may contain secrets or third-party PII. It becomes an
/// [`Observation`](crate::Observation) only after passing the gate — the gate is the only thing that
/// may turn a `RawCapture` into something persistable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCapture {
    /// Provenance anchor in the upstream store (row id, message id, event id).
    pub upstream_ref: String,
    /// Which source produced this item.
    pub source: SourceKind,
    /// App / window / account the item came from, if known.
    pub app: Option<String>,
    /// Valid-time start: when the captured activity actually happened.
    pub captured_at: DateTime<Utc>,
    /// Pre-redaction text. Never persist this directly.
    pub text: String,
    /// For audio sources: who was speaking, if known.
    pub speaker: Option<crate::SpeakerTag>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_round_trips_snake_case() {
        let json = serde_json::to_string(&SourceKind::ScreenOcr).unwrap();
        assert_eq!(json, "\"screen_ocr\"");
        let back: SourceKind = serde_json::from_str("\"a11y_tree\"").unwrap();
        assert_eq!(back, SourceKind::A11yTree);
    }

    #[test]
    fn connector_variants_are_flagged() {
        assert!(SourceKind::Email.is_connector());
        assert!(SourceKind::Slack.is_connector());
        assert!(SourceKind::GitHub.is_connector());
        assert!(!SourceKind::ScreenOcr.is_connector());
        assert!(SourceKind::Dictation.is_audio());
        assert!(!SourceKind::ScreenOcr.is_audio());
    }

    #[test]
    fn cursor_is_transparent_json() {
        let c = Cursor::new(42);
        assert_eq!(serde_json::to_string(&c).unwrap(), "42");
    }
}
