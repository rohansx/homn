//! The read-path memory the MCP `recall` / `timeline` tools query (v2: US2 / T031).
//!
//! The brain plugs in here as the [`Brain`] trait. The MCP handlers are generic over it: any
//! store that can answer cue recall + a time-bounded timeline can serve the tools. The
//! no-feature default is [`MemoryBrain`] (in-process, for dev/tests); the agidb-backed brain
//! lives in `homnd` behind `brain-agidb` and is wired in when the daemon constructs the server.
//!
//! **Read path — zero network egress (Invariant 2 / FR-019 / SC-006).** Nothing in this module
//! or in the `recall`/`timeline` handlers makes a network call. The brain's methods are the only
//! thing the handlers touch, and [`Brain`] is a pure local query interface. The
//! `tests/read_path_no_egress.rs` integration test asserts every hit carries provenance and
//! that the crate has no HTTP-client dependency.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use homn_types::Observation;
use serde::Serialize;

/// One ranked recall hit, with full provenance (FR-018).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RecallHit {
    /// The recalled text (post-redaction).
    pub text: String,
    /// Capture source label (e.g. `"screen_ocr"`, `"dictation"`).
    pub source: String,
    /// App / window / account, if known.
    pub app: Option<String>,
    /// Valid time — when the activity happened.
    pub captured_at: DateTime<Utc>,
    /// Brain-side confidence (0.0–1.0); 0.0 when the store doesn't score.
    pub confidence: f32,
    /// The observation's id (Ulid string).
    pub observation_id: String,
}

/// One timeline entry, chronological, with provenance.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct TimelineEntry {
    /// When the activity happened (valid time).
    pub at: DateTime<Utc>,
    /// The entry text (post-redaction).
    pub text: String,
    /// Session / episode id, if the sessionizer assigned one.
    pub session: Option<String>,
    /// Capture source label.
    pub source: String,
    /// The observation's id (Ulid string).
    pub observation_id: String,
}

/// The read-path memory. Implementations MUST be local-only (no network in the read path).
#[async_trait]
pub trait Brain: Send + Sync {
    /// Return up to `k` ranked hits for `cue`, best first. `as_of` is the bi-temporal
    /// "recall as the world was known then" bound; `None` means now.
    async fn recall(
        &self,
        cue: &str,
        as_of: Option<DateTime<Utc>>,
        k: usize,
    ) -> anyhow::Result<Vec<RecallHit>>;

    /// Chronological entries for `subject` (entity or topic) within `[from, to]`.
    async fn timeline(
        &self,
        subject: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<TimelineEntry>>;
}

/// In-process brain backed by a vector of observations — the no-feature default used by tests
/// and dev. Recall is naive substring ranking; timeline is a subject-substring + time-window
/// filter. Real recall quality comes from the agidb brain (`homnd::AgidbBrain`).
#[derive(Debug, Default)]
pub struct MemoryBrain {
    obs: tokio::sync::RwLock<Vec<Observation>>,
}

impl MemoryBrain {
    /// Create an empty in-process brain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push an observation into the in-process store (dev/test helper).
    pub async fn push(&self, obs: Observation) {
        self.obs.write().await.push(obs);
    }
}

#[async_trait]
impl Brain for MemoryBrain {
    async fn recall(
        &self,
        cue: &str,
        as_of: Option<DateTime<Utc>>,
        k: usize,
    ) -> anyhow::Result<Vec<RecallHit>> {
        let obs = self.obs.read().await;
        let cue = cue.to_lowercase();
        let mut scored: Vec<(f32, &Observation)> = obs
            .iter()
            .filter(|o| as_of.is_none_or(|t| o.captured_at <= t))
            .filter_map(|o| {
                let text_lc = o.text.to_lowercase();
                let score = if cue.is_empty() {
                    0.0
                } else if text_lc.contains(&cue) {
                    1.0
                } else {
                    // Soft match: fraction of cue words present.
                    let words = cue.split_whitespace().filter(|w| !w.is_empty());
                    let total = words.clone().count().max(1) as f32;
                    let present = words.filter(|w| text_lc.contains(*w)).count() as f32;
                    present / total
                };
                if score > 0.0 {
                    Some((score, o))
                } else {
                    None
                }
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored
            .into_iter()
            .take(k)
            .map(|(confidence, o)| to_recall_hit(o, confidence))
            .collect())
    }

    async fn timeline(
        &self,
        subject: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<TimelineEntry>> {
        let obs = self.obs.read().await;
        let subject = subject.to_lowercase();
        let mut entries: Vec<TimelineEntry> = obs
            .iter()
            .filter(|o| o.captured_at >= from && o.captured_at <= to)
            .filter(|o| subject.is_empty() || o.text.to_lowercase().contains(&subject))
            .map(to_timeline_entry)
            .collect();
        entries.sort_by_key(|e| e.at);
        Ok(entries)
    }
}

fn to_recall_hit(o: &Observation, confidence: f32) -> RecallHit {
    RecallHit {
        text: o.text.clone(),
        source: source_label(o.source),
        app: o.app.clone(),
        captured_at: o.captured_at,
        confidence,
        observation_id: o.id.to_string(),
    }
}

fn to_timeline_entry(o: &Observation) -> TimelineEntry {
    TimelineEntry {
        at: o.captured_at,
        text: o.text.clone(),
        session: o.session.clone().map(|s| s.0),
        source: source_label(o.source),
        observation_id: o.id.to_string(),
    }
}

fn source_label(s: homn_types::SourceKind) -> String {
    // Mirror the serde snake_case name without pulling serde at runtime.
    match s {
        homn_types::SourceKind::ScreenOcr => "screen_ocr",
        homn_types::SourceKind::A11yTree => "a11y_tree",
        homn_types::SourceKind::AmbientAudio => "ambient_audio",
        homn_types::SourceKind::Dictation => "dictation",
        homn_types::SourceKind::Email => "email",
        homn_types::SourceKind::Slack => "slack",
        homn_types::SourceKind::GitHub => "github",
    }
    .to_owned()
}

/// A `Brain` that records every call — for the no-egress / provenance tests.
#[derive(Default)]
pub struct RecordingBrain {
    /// Every `recall` invocation: `(cue, as_of, k)`.
    pub recall_calls: std::sync::Mutex<RecallCalls>,
    /// Every `timeline` invocation: `(subject, from, to)`.
    pub timeline_calls: std::sync::Mutex<TimelineCalls>,
}

/// Recorded `recall` calls.
pub type RecallCalls = Vec<(String, Option<DateTime<Utc>>, usize)>;
/// Recorded `timeline` calls.
pub type TimelineCalls = Vec<(String, DateTime<Utc>, DateTime<Utc>)>;

#[async_trait]
impl Brain for RecordingBrain {
    async fn recall(
        &self,
        cue: &str,
        as_of: Option<DateTime<Utc>>,
        k: usize,
    ) -> anyhow::Result<Vec<RecallHit>> {
        self.recall_calls
            .lock()
            .unwrap()
            .push((cue.to_owned(), as_of, k));
        Ok(Vec::new())
    }
    async fn timeline(
        &self,
        subject: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<TimelineEntry>> {
        self.timeline_calls
            .lock()
            .unwrap()
            .push((subject.to_owned(), from, to));
        Ok(Vec::new())
    }
}

/// Build a shared brain handle (the shape `McpState` stores).
#[allow(dead_code)]
pub fn shared(brain: impl Brain + 'static) -> Arc<dyn Brain> {
    Arc::new(brain)
}

// ============================================================================
// agidb-backed brain (feature-gated) — the real recall/timeline store
// ============================================================================

/// A [`Brain`] backed by a live [`agidb::Agidb`] handle. Construct with [`AgidbBrain::new`];
/// cheap to clone via `Arc`. recall uses `Agidb::recall(Query)` (with `as_of` → `Query::as_of`);
/// timeline uses `Agidb::timeline`. Read-path only, no network (Invariant 2).
#[cfg(feature = "brain-agidb")]
pub struct AgidbBrain {
    brain: std::sync::Arc<agidb::Agidb>,
}

#[cfg(feature = "brain-agidb")]
impl AgidbBrain {
    /// Wrap an existing agidb handle.
    pub fn new(brain: std::sync::Arc<agidb::Agidb>) -> Self {
        Self { brain }
    }
}

#[cfg(feature = "brain-agidb")]
#[async_trait::async_trait]
impl Brain for AgidbBrain {
    async fn recall(
        &self,
        cue: &str,
        as_of: Option<DateTime<Utc>>,
        k: usize,
    ) -> anyhow::Result<Vec<RecallHit>> {
        let mut q = agidb::Query::cue(cue).with_k(k);
        if let Some(t) = as_of {
            q = q.with_as_of(t);
        }
        let recall = self.brain.recall(q).await.map_err(|e| anyhow::anyhow!(e))?;
        Ok(recall
            .matches
            .into_iter()
            .take(k)
            .map(|m| RecallHit {
                text: m.text,
                source: m.provenance.source,
                app: None, // agidb doesn't surface an app field on RecallMatch
                captured_at: m.valid_time.start,
                confidence: m.confidence,
                observation_id: format!("ep-{}", m.episode_id),
            })
            .collect())
    }

    async fn timeline(
        &self,
        subject: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<TimelineEntry>> {
        // subject "" → None (agidb::timeline treats None as "no subject filter").
        let subj = if subject.trim().is_empty() {
            None
        } else {
            Some(subject)
        };
        let episodes = self
            .brain
            .timeline(subj, from, to, 200)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(episodes
            .into_iter()
            .map(|ep| TimelineEntry {
                at: ep.valid_time.start,
                text: ep.text,
                session: ep.provenance.session_id,
                source: ep.provenance.source,
                observation_id: format!("ep-{}", ep.id),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use homn_types::{Observation, Provenance, SourceKind};
    use ulid::Ulid;

    fn obs(text: &str, app: Option<&str>, at: DateTime<Utc>) -> Observation {
        Observation {
            id: Ulid::new(),
            source: SourceKind::ScreenOcr,
            app: app.map(str::to_owned),
            captured_at: at,
            ingested_at: at,
            text: text.to_owned(),
            redactions: vec![],
            session: None,
            speaker: None,
            content_hash: 0,
            provenance: Provenance {
                source_id: "test".to_owned(),
                upstream_ref: "t".to_owned(),
            },
        }
    }

    #[tokio::test]
    async fn recall_ranks_exact_phrase_above_word_overlap() {
        let brain = MemoryBrain::new();
        brain
            .push(obs("the quick brown fox", None, Utc::now()))
            .await;
        brain.push(obs("a quick lunch", None, Utc::now())).await;
        let hits = brain.recall("quick brown fox", None, 3).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].text, "the quick brown fox",
            "exact phrase ranks first"
        );
        assert!(hits[0].confidence > hits[1].confidence);
    }

    #[tokio::test]
    async fn recall_carries_provenance_on_every_hit() {
        let brain = MemoryBrain::new();
        brain
            .push(obs(
                "Sarah promised the quote by Friday",
                Some("Slack"),
                Utc::now(),
            ))
            .await;
        let hits = brain.recall("Sarah promised", None, 3).await.unwrap();
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert!(
            h.observation_id.starts_with("01"),
            "ulid string: {}",
            h.observation_id
        );
        assert_eq!(h.source, "screen_ocr");
        assert_eq!(h.app.as_deref(), Some("Slack"));
    }

    #[tokio::test]
    async fn timeline_filters_by_subject_and_window() {
        let brain = MemoryBrain::new();
        let t0 = "2026-07-10T09:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let t1 = "2026-07-10T11:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let t2 = "2026-07-10T15:00:00Z".parse::<DateTime<Utc>>().unwrap();
        brain.push(obs("pricing thread with Chris", None, t0)).await;
        brain.push(obs("lunch notes", None, t1)).await;
        brain.push(obs("pricing follow-up", None, t2)).await;
        let from = "2026-07-10T00:00:00Z".parse().unwrap();
        let to = "2026-07-10T12:00:00Z".parse().unwrap();
        let entries = brain.timeline("pricing", from, to).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "pricing thread with Chris");
        assert_eq!(entries[0].at, t0);
    }

    #[tokio::test]
    async fn as_of_bounds_recall_to_the_past() {
        let brain = MemoryBrain::new();
        let past = "2026-07-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let now = Utc::now();
        brain.push(obs("old note", None, past)).await;
        brain.push(obs("fresh note", None, now)).await;
        let as_of = "2026-07-10T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let hits = brain.recall("note", Some(as_of), 5).await.unwrap();
        assert_eq!(hits.len(), 1, "only the past observation is visible as_of");
        assert_eq!(hits[0].text, "old note");
    }
}

/// agidb-backed brain tests — only compiled with the `brain-agidb` feature.
#[cfg(all(test, feature = "brain-agidb"))]
mod agidb_tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(day: &str) -> DateTime<Utc> {
        // "2026-07-13" → that day at noon UTC.
        chrono::Utc
            .with_ymd_and_hms(
                day[0..4].parse().unwrap(),
                day[5..7].parse().unwrap(),
                day[8..10].parse().unwrap(),
                12,
                0,
                0,
            )
            .unwrap()
    }

    fn brain_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "homn-mcp-agidbbrain-{}-{label}-{}.agidb",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    async fn make_brain(dir: &std::path::Path) -> std::sync::Arc<agidb::Agidb> {
        let b = agidb::Agidb::open_with(
            agidb::AgidbConfig::new(dir).with_extractor(agidb::ExtractorSetup::Null),
        )
        .await
        .unwrap();
        // Three episodes with explicit valid_times (Mon/Tue/Fri).
        for (text, day) in [
            ("Sarah promised the quote by Friday", "2026-07-13"),
            ("Priya owes the API spec by Tuesday", "2026-07-14"),
            ("shipped the pricing quote to Marco", "2026-07-17"),
        ] {
            let ctx = agidb::ObserveContext {
                observation_time: ts(day),
                provenance: agidb::Provenance {
                    source: "test".to_owned(),
                    ..Default::default()
                },
            };
            b.observe_with_context(text, ctx).await.unwrap();
        }
        b.flush().await.unwrap();
        std::sync::Arc::new(b)
    }

    #[tokio::test]
    async fn agidb_brain_recall_returns_provenance_hits() {
        let dir = brain_dir("recall");
        let agidb_brain = make_brain(&dir).await;
        let brain = AgidbBrain::new(agidb_brain);
        let hits = brain.recall("pricing quote Marco", None, 3).await.unwrap();
        assert!(!hits.is_empty(), "recall must surface the Marco episode");
        assert!(
            hits.iter()
                .any(|h| h.text.contains("pricing quote to Marco")),
            "hit text carries the episode: {hits:?}"
        );
        let h = hits
            .iter()
            .find(|h| h.text.contains("pricing quote"))
            .unwrap();
        assert_eq!(h.source, "test", "provenance source carried through");
        assert!(
            h.observation_id.starts_with("ep-"),
            "observation_id is an ep ref"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn agidb_brain_timeline_is_chronological_in_window() {
        let dir = brain_dir("timeline");
        let agidb_brain = make_brain(&dir).await;
        let brain = AgidbBrain::new(agidb_brain);
        let from = ts("2026-07-13");
        let to = ts("2026-07-17");
        // Empty subject → no subject filter → all episodes in the window.
        let entries = brain.timeline("", from, to).await.unwrap();
        assert_eq!(
            entries.len(),
            3,
            "all three episodes are in the Mon–Fri window"
        );
        // Chronological by valid_time.start.
        assert_eq!(entries[0].text, "Sarah promised the quote by Friday");
        assert_eq!(entries[1].text, "Priya owes the API spec by Tuesday");
        assert_eq!(entries[2].text, "shipped the pricing quote to Marco");
        // Window excludes nothing here; narrow it to Tue only.
        let tue = brain
            .timeline("", ts("2026-07-14"), ts("2026-07-14"))
            .await
            .unwrap();
        assert_eq!(
            tue.len(),
            1,
            "the Tue-only window returns just the Tue episode"
        );
        assert_eq!(tue[0].text, "Priya owes the API spec by Tuesday");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
