//! Session identifiers.
//!
//! Newtype over the ULID string that Claude Code provides in its hook payloads. Kept as `String`
//! (not parsed into `Ulid`) because Claude is the source of truth and we don't want to be strict
//! about format — if Claude ever changes the format, our serialization survives.

use serde::{Deserialize, Serialize};

/// Stable identifier for a single Claude Code session.
///
/// Format is Claude's choice (currently a ULID). We treat it as opaque.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl SessionId {
    /// Create a new `SessionId` from a string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Borrow the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// The kind of ambient session (v2 ambient-memory grouping).
///
/// A [`Session`] groups observations into an episode so consolidation can mint episode-level
/// memories ("the July 14 call with X") instead of confetti. The boundary mechanism is
/// source-agnostic: the connector variants let a Phase 3.5 mail thread or channel-day be one
/// session using the same machinery as an app-focus block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// A block of continuous focus on a single app/window.
    AppFocus,
    /// A meeting (Zoom/Meet window active, or mic active).
    Meeting,
    /// Reserved — Phase 3.5: a mail thread treated as one session.
    MailThread,
    /// Reserved — Phase 3.5: a channel's activity for a day treated as one session.
    ChannelDay,
}

/// An ambient session: a first-class grouping of observations with boundaries.
///
/// Reuses [`SessionId`] as its opaque identifier. `ended_at == None` means the session is still
/// open. Note this is distinct from a Claude Code hook session even though both are keyed by
/// [`SessionId`]; the `kind` disambiguates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Opaque session identifier.
    pub id: SessionId,
    /// What kind of session this is.
    pub kind: SessionKind,
    /// When the session opened.
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// When the session closed; `None` while still active.
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Human-readable label once an entity is known (e.g. "call with X").
    pub label: Option<String>,
    /// The app / channel this session is anchored to, if any.
    pub app_or_channel: Option<String>,
}

impl Session {
    /// Whether the session is still open (no end boundary detected yet).
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_round_trips_as_bare_string() {
        let sid = SessionId::new("01HXY0123456789ABCDEFGHJKM");
        let json = serde_json::to_string(&sid).unwrap();
        assert_eq!(json, "\"01HXY0123456789ABCDEFGHJKM\"");
        let parsed: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, sid);
    }

    #[test]
    fn session_id_display_returns_inner() {
        let sid = SessionId::from("hello");
        assert_eq!(format!("{sid}"), "hello");
    }
}
