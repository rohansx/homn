//! The sessionizer — pipeline stage 4 (T029).
//!
//! Assigns or extends an ambient [`SessionId`] so consolidation can later mint episode-level
//! memories ("the July 14 call with Chris") instead of confetti. v1 uses a single, simple
//! boundary: consecutive observations from the **same `(source, app)`** within a `GAP` of wall
//! time belong to one session; crossing the gap, or a change in source/app, starts a new one.
//!
//! This is deliberately heuristic and stateful-but-cheap — the brain may re-segment later. The
//! Phase 3.5 connector variants (`MailThread`, `ChannelDay`) reuse the same `SessionId` plumbing
//! with a different boundary computed by the connector itself.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::time::Duration;

use chrono::{DateTime, Utc};
use homn_types::{Observation, SessionId, SessionKind, SourceKind};

/// Default focus-gap: two observations more than this apart (same app+source) start a new
/// session. Tuned for screen capture cadence (Screenpipe ~1 frame/sec), so a 5-minute quiet
/// gap is a natural episode boundary.
const DEFAULT_GAP: Duration = Duration::from_secs(5 * 60);

/// The open session the sessionizer is currently extending, if any. Extracted to a named struct
/// so the [`Sessionizer`] field reads cleanly (and clippy's `type_complexity` stays happy).
#[derive(Debug, Clone)]
struct OpenSession {
    /// The source whose stream this session belongs to.
    source: SourceKind,
    /// The app/window the session is scoped to.
    app: Option<String>,
    /// The session kind (AppFocus / Meeting).
    kind: SessionKind,
    /// The session's id, returned for every observation in it.
    id: SessionId,
    /// When the session was last extended (drives the gap boundary).
    last: DateTime<Utc>,
}

/// The v1 sessionizer: stateful, in-process. One per daemon (thread-safe via the call sites).
#[derive(Debug, Clone)]
pub struct Sessionizer {
    gap: Duration,
    /// Current open session, if any.
    current: Option<OpenSession>,
}

impl Sessionizer {
    /// Build a sessionizer with the default 5-minute gap.
    pub fn new() -> Self {
        Self {
            gap: DEFAULT_GAP,
            current: None,
        }
    }

    /// Build with a custom gap (tests / tuning).
    pub fn with_gap(gap: Duration) -> Self {
        Self { gap, current: None }
    }

    /// Assign a session to `obs` (mutating the capture in place is the caller's job; this returns
    /// the id and kind). The observation's `captured_at` drives the boundary; `ingested_at` is
    /// ignored (transaction time is not episode time).
    pub fn assign(&mut self, obs: &Observation) -> (SessionId, SessionKind) {
        let key = (obs.source, obs.app.clone());
        let kind = session_kind_for(obs.source);
        let now = obs.captured_at;

        let start_new = match &self.current {
            None => true,
            Some(open) => {
                open.source != key.0
                    || open.app != key.1
                    || open.kind != kind
                    || (now - open.last).num_seconds() > self.gap.as_secs() as i64
            }
        };

        if start_new {
            let id = SessionId::new(ulid::Ulid::new().to_string());
            self.current = Some(OpenSession {
                source: key.0,
                app: key.1,
                kind,
                id: id.clone(),
                last: now,
            });
            (id, kind)
        } else {
            // Extend the current session: clone the id out of the borrow before reassigning.
            let id = self.current.as_ref().unwrap().id.clone();
            self.current.as_mut().unwrap().last = now;
            (id, kind)
        }
    }
}

impl Default for Sessionizer {
    fn default() -> Self {
        Self::new()
    }
}

/// The session kind a source maps to. Audio-during-a-meeting windows become `Meeting` in v2;
/// everything else is `AppFocus`. (Refined by a meeting detector in Phase 4.)
fn session_kind_for(s: SourceKind) -> SessionKind {
    match s {
        SourceKind::AmbientAudio | SourceKind::Dictation => SessionKind::Meeting,
        _ => SessionKind::AppFocus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use homn_types::{Provenance, SpeakerTag};

    fn obs(source: SourceKind, app: Option<&str>, at: DateTime<Utc>, text: &str) -> Observation {
        Observation {
            id: ulid::Ulid::new(),
            source,
            app: app.map(|s| s.to_owned()),
            captured_at: at,
            ingested_at: at,
            text: text.to_owned(),
            redactions: vec![],
            session: None,
            speaker: if source.is_audio() {
                Some(SpeakerTag::Me)
            } else {
                None
            },
            content_hash: 0,
            provenance: Provenance {
                source_id: "x".to_owned(),
                upstream_ref: "r".to_owned(),
            },
        }
    }

    #[test]
    fn same_app_within_gap_stays_one_session() {
        let mut s = Sessionizer::with_gap(Duration::from_secs(60));
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 9, 0, 0).unwrap();
        let t1 = t0 + chrono::Duration::seconds(30);
        let (a, _) = s.assign(&obs(SourceKind::ScreenOcr, Some("Code"), t0, "x"));
        let (b, _) = s.assign(&obs(SourceKind::ScreenOcr, Some("Code"), t1, "y"));
        assert_eq!(a, b, "within-gap, same app = same session");
    }

    #[test]
    fn gap_exceeded_starts_new_session() {
        let mut s = Sessionizer::with_gap(Duration::from_secs(60));
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 9, 0, 0).unwrap();
        let t1 = t0 + chrono::Duration::seconds(120); // > 60s gap
        let (a, _) = s.assign(&obs(SourceKind::ScreenOcr, Some("Code"), t0, "x"));
        let (b, _) = s.assign(&obs(SourceKind::ScreenOcr, Some("Code"), t1, "y"));
        assert_ne!(a, b, "gap exceeded = new session");
    }

    #[test]
    fn app_change_starts_new_session_even_within_gap() {
        let mut s = Sessionizer::with_gap(Duration::from_secs(600));
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 9, 0, 0).unwrap();
        let t1 = t0 + chrono::Duration::seconds(10);
        let (a, _) = s.assign(&obs(SourceKind::ScreenOcr, Some("Code"), t0, "x"));
        let (b, _) = s.assign(&obs(SourceKind::ScreenOcr, Some("Slack"), t1, "y"));
        assert_ne!(a, b);
    }

    #[test]
    fn audio_sources_get_meeting_kind() {
        let mut s = Sessionizer::new();
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 9, 0, 0).unwrap();
        let (_, kind) = s.assign(&obs(SourceKind::Dictation, None, t0, "x"));
        assert_eq!(kind, SessionKind::Meeting);
        let (_, kind) = s.assign(&obs(SourceKind::ScreenOcr, Some("Code"), t0, "x"));
        assert_eq!(kind, SessionKind::AppFocus);
    }
}
