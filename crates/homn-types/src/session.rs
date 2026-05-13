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
