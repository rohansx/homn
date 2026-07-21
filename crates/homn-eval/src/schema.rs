//! The Phase 0 question-set schema and its 10/10/10 validation.
//!
//! Mirrors `eval/questions/TEMPLATE.toml`. A set that isn't exactly 10 factual + 10 temporal +
//! 10 commitment questions is rejected — the gate is only meaningful over the balanced set.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The three question categories, drawn from the real captured week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// "who sent the screenshot about the bug" — a fact recall.
    Factual,
    /// "what did I decide about X on Tuesday" — time-anchored recall.
    Temporal,
    /// "what did I promise Chris" / "how did my position change" — commitment/belief recall.
    Commitment,
}

impl QuestionKind {
    /// All three kinds, in a stable order.
    pub const ALL: [QuestionKind; 3] = [
        QuestionKind::Factual,
        QuestionKind::Temporal,
        QuestionKind::Commitment,
    ];

    /// The required count of each kind in a valid set.
    pub const REQUIRED_PER_KIND: usize = 10;
}

/// One evaluation question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Unique id within the set (e.g. "f01", "t01", "c01").
    pub id: String,
    /// Which category this question belongs to.
    pub kind: QuestionKind,
    /// The natural-language cue posed to recall.
    pub question: String,
    /// Ground-truth anchor the scorer looks for in the top-k hits (an observation ref or a
    /// distinctive phrase). May be empty in the TEMPLATE; a real run requires it filled.
    #[serde(default)]
    pub expected_ref: String,
    /// Optional hand-scoring guidance when auto-match is ambiguous.
    #[serde(default)]
    pub notes: String,
    /// Optional temporal window `[from, to]` (ISO-8601 strings). When set, a brain that
    /// supports temporal retrieval (agidb `Query::time_window`) filters candidates to episodes
    /// whose valid_time overlaps the window — the fix for "when did X happen" cues that share
    /// no tokens with the answer. Brains without temporal support ignore it.
    #[serde(default)]
    pub time_window: Option<(String, String)>,
}

/// Metadata about a captured week and the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionSetMeta {
    /// The capture window, e.g. "2026-07-10/2026-07-16".
    pub captured_week: String,
    /// Who authored the set.
    #[serde(default)]
    pub authored_by: String,
    /// Default k for recall@k.
    #[serde(default = "default_k")]
    pub k: usize,
}

fn default_k() -> usize {
    3
}

/// A full question set (`[meta]` + `[[question]]` in TOML).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionSet {
    /// Set metadata.
    pub meta: QuestionSetMeta,
    /// The questions. TOML `[[question]]` blocks map here.
    #[serde(default, rename = "question")]
    pub questions: Vec<Question>,
}

/// Errors from loading or validating a question set.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EvalError {
    /// The TOML failed to parse.
    #[error("failed to parse question set: {0}")]
    Parse(String),
    /// The set has the wrong number of a given kind.
    #[error("question set must have exactly {expected} {kind:?} questions, found {found}")]
    KindCount {
        /// Which kind was miscounted.
        kind: QuestionKind,
        /// The required count.
        expected: usize,
        /// The actual count.
        found: usize,
    },
    /// Two questions share an id.
    #[error("duplicate question id: {0}")]
    DuplicateId(String),
    /// A question is missing its ground-truth anchor (only enforced for real runs).
    #[error("question {0} has an empty expected_ref")]
    MissingExpectedRef(String),
}

impl QuestionSet {
    /// Parse a set from TOML text (does not validate counts — call [`Self::validate`]).
    pub fn from_toml_str(s: &str) -> Result<Self, EvalError> {
        toml::from_str(s).map_err(|e| EvalError::Parse(e.to_string()))
    }

    /// Count questions per kind.
    pub fn counts(&self) -> BTreeMap<QuestionKind, usize> {
        let mut m = BTreeMap::new();
        for k in QuestionKind::ALL {
            m.insert(k, 0);
        }
        for q in &self.questions {
            *m.entry(q.kind).or_insert(0) += 1;
        }
        m
    }

    /// Validate the 10/10/10 balance and unique ids.
    ///
    /// `require_expected_refs` is `false` for the TEMPLATE (empty anchors allowed) and `true` for a
    /// real scoring run.
    pub fn validate(&self, require_expected_refs: bool) -> Result<(), EvalError> {
        // Unique ids.
        let mut seen = std::collections::HashSet::new();
        for q in &self.questions {
            if !seen.insert(q.id.as_str()) {
                return Err(EvalError::DuplicateId(q.id.clone()));
            }
        }
        // 10/10/10 balance.
        for (kind, found) in self.counts() {
            if found != QuestionKind::REQUIRED_PER_KIND {
                return Err(EvalError::KindCount {
                    kind,
                    expected: QuestionKind::REQUIRED_PER_KIND,
                    found,
                });
            }
        }
        // Ground-truth anchors, for real runs.
        if require_expected_refs {
            for q in &self.questions {
                if q.expected_ref.trim().is_empty() {
                    return Err(EvalError::MissingExpectedRef(q.id.clone()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(counts: (usize, usize, usize)) -> QuestionSet {
        let mut questions = Vec::new();
        let mk = |kind: QuestionKind, n: usize, prefix: &str| {
            (0..n)
                .map(|i| Question {
                    id: format!("{prefix}{i:02}"),
                    kind,
                    question: format!("q{i}"),
                    expected_ref: format!("ref{i}"),
                    notes: String::new(),
                    time_window: None,
                })
                .collect::<Vec<_>>()
        };
        questions.extend(mk(QuestionKind::Factual, counts.0, "f"));
        questions.extend(mk(QuestionKind::Temporal, counts.1, "t"));
        questions.extend(mk(QuestionKind::Commitment, counts.2, "c"));
        QuestionSet {
            meta: QuestionSetMeta {
                captured_week: "w".to_owned(),
                authored_by: String::new(),
                k: 3,
            },
            questions,
        }
    }

    #[test]
    fn balanced_set_validates() {
        assert!(set_with((10, 10, 10)).validate(true).is_ok());
    }

    #[test]
    fn unbalanced_set_is_rejected() {
        let err = set_with((10, 9, 10)).validate(true).unwrap_err();
        assert_eq!(
            err,
            EvalError::KindCount {
                kind: QuestionKind::Temporal,
                expected: 10,
                found: 9
            }
        );
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let mut s = set_with((10, 10, 10));
        s.questions[1].id = s.questions[0].id.clone();
        assert!(matches!(s.validate(true), Err(EvalError::DuplicateId(_))));
    }

    #[test]
    fn empty_expected_ref_allowed_for_template_but_not_runs() {
        let mut s = set_with((10, 10, 10));
        s.questions[0].expected_ref = String::new();
        assert!(
            s.validate(false).is_ok(),
            "template mode tolerates empty anchors"
        );
        assert!(matches!(
            s.validate(true),
            Err(EvalError::MissingExpectedRef(_))
        ));
    }

    #[test]
    fn parses_toml_meta_and_questions() {
        let toml = r#"
[meta]
captured_week = "2026-07-10/2026-07-16"
k = 3

[[question]]
id = "f01"
kind = "factual"
question = "who sent the screenshot"
expected_ref = "obs-123"
"#;
        let set = QuestionSet::from_toml_str(toml).unwrap();
        assert_eq!(set.meta.captured_week, "2026-07-10/2026-07-16");
        assert_eq!(set.questions.len(), 1);
        assert_eq!(set.questions[0].kind, QuestionKind::Factual);
    }
}
