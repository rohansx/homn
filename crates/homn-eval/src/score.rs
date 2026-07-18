//! Recall@k scoring and the brain-architecture gate.
//!
//! Scoring is decoupled from the store: it runs against any [`Recaller`], so the harness is
//! buildable and testable now (with a mock) and wires to agidb later (task T013/T030). A question
//! counts as hit@k if its `expected_ref` matches one of the top-k hits — by reference equality or
//! by substring appearance in the hit text (the hand-scoring fallback the README describes).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::{QuestionKind, QuestionSet};

/// A single recalled hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The observation reference / id of this hit.
    pub reference: String,
    /// The hit's text (used for the substring fallback match).
    pub text: String,
}

/// Anything that can answer a cue with ranked hits. Implemented over agidb in the daemon;
/// implemented by a mock in tests.
pub trait Recaller {
    /// Return up to `k` ranked hits for `cue`, best first.
    fn recall(&self, cue: &str, k: usize) -> Vec<Hit>;
}

/// Whether `expected_ref` is satisfied by any of the top-`k` hits.
fn is_hit(expected_ref: &str, hits: &[Hit], k: usize) -> bool {
    let needle = expected_ref.trim();
    if needle.is_empty() {
        return false;
    }
    hits.iter()
        .take(k)
        .any(|h| h.reference == needle || h.text.contains(needle))
}

/// Operational metrics gathered alongside recall (populated during a real run; see `ops.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct OpsMetrics {
    /// Observations stored per day over the capture window.
    pub observations_per_day: f64,
    /// Disk growth attributable to the store, in bytes.
    pub disk_growth_bytes: u64,
    /// Average ingest CPU percentage.
    pub ingest_cpu_pct: f64,
    /// GLiNER extraction precision on a sampled set of extractions (0.0–1.0).
    pub extraction_precision: f64,
}

/// The result of scoring a set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    /// k used for recall@k.
    pub k: usize,
    /// Total questions scored.
    pub total: usize,
    /// Overall recall@1 (0.0–1.0).
    pub recall_at_1: f64,
    /// Overall recall@k (0.0–1.0).
    pub recall_at_k: f64,
    /// Recall@k per kind.
    pub per_kind_recall_at_k: BTreeMap<QuestionKind, f64>,
    /// Optional operational metrics from a real run.
    pub ops: Option<OpsMetrics>,
}

/// Score a validated question set against a recaller.
pub fn score(set: &QuestionSet, recaller: &dyn Recaller, k: usize) -> RunResult {
    let mut hits_at_1 = 0usize;
    let mut hits_at_k = 0usize;
    let mut per_kind_total: BTreeMap<QuestionKind, usize> = BTreeMap::new();
    let mut per_kind_hit_k: BTreeMap<QuestionKind, usize> = BTreeMap::new();

    for q in &set.questions {
        let hits = recaller.recall(&q.question, k);
        *per_kind_total.entry(q.kind).or_default() += 1;
        if is_hit(&q.expected_ref, &hits, 1) {
            hits_at_1 += 1;
        }
        if is_hit(&q.expected_ref, &hits, k) {
            hits_at_k += 1;
            *per_kind_hit_k.entry(q.kind).or_default() += 1;
        }
    }

    let total = set.questions.len();
    let ratio = |num: usize, den: usize| {
        if den == 0 {
            0.0
        } else {
            num as f64 / den as f64
        }
    };

    let per_kind_recall_at_k = QuestionKind::ALL
        .iter()
        .map(|kind| {
            let den = per_kind_total.get(kind).copied().unwrap_or(0);
            let num = per_kind_hit_k.get(kind).copied().unwrap_or(0);
            (*kind, ratio(num, den))
        })
        .collect();

    RunResult {
        k,
        total,
        recall_at_1: ratio(hits_at_1, total),
        recall_at_k: ratio(hits_at_k, total),
        per_kind_recall_at_k,
        ops: None,
    }
}

/// The brain-architecture branch chosen from recall@3 (research R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainBranch {
    /// recall@3 ≥ 70%: adopt agidb as-is; skip Phase 2b.
    AgidbAsIs,
    /// 40% ≤ recall@3 < 70%: Phase 2b retrieval merge is mandatory before Phase 3.
    RetrievalMerge,
    /// recall@3 < 40%: ctxgraph becomes the store; port agidb's belief/goal/unlearn types on top.
    StoreSwap,
}

impl BrainBranch {
    /// A one-line human description of the branch's consequence.
    pub fn consequence(self) -> &'static str {
        match self {
            BrainBranch::AgidbAsIs => "proceed with agidb as-is (skip Phase 2b)",
            BrainBranch::RetrievalMerge => {
                "Phase 2b mandatory: fuse ctxgraph retrieval into agidb before Phase 3"
            }
            BrainBranch::StoreSwap => {
                "ctxgraph becomes the store; port agidb belief/goal/unlearn types on top"
            }
        }
    }
}

/// Decide the brain branch from a recall@3 fraction (0.0–1.0).
pub fn gate_verdict(recall_at_3: f64) -> BrainBranch {
    if recall_at_3 >= 0.70 {
        BrainBranch::AgidbAsIs
    } else if recall_at_3 >= 0.40 {
        BrainBranch::RetrievalMerge
    } else {
        BrainBranch::StoreSwap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Question, QuestionSetMeta};

    /// A mock recaller: maps a cue substring to a canned hit list.
    struct MockRecaller {
        // For question text containing `.0`, return hits `.1`.
        rules: Vec<(&'static str, Vec<Hit>)>,
    }

    impl Recaller for MockRecaller {
        fn recall(&self, cue: &str, k: usize) -> Vec<Hit> {
            for (needle, hits) in &self.rules {
                if cue.contains(needle) {
                    return hits.iter().take(k).cloned().collect();
                }
            }
            vec![]
        }
    }

    fn q(id: &str, kind: QuestionKind, question: &str, expected: &str) -> Question {
        Question {
            id: id.to_owned(),
            kind,
            question: question.to_owned(),
            expected_ref: expected.to_owned(),
            notes: String::new(),
        }
    }

    fn hit(reference: &str, text: &str) -> Hit {
        Hit {
            reference: reference.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn recall_at_1_vs_at_k_differ_by_rank() {
        // The correct answer is the 2nd hit: miss@1, hit@3.
        let set = QuestionSet {
            meta: QuestionSetMeta {
                captured_week: "w".to_owned(),
                authored_by: String::new(),
                k: 3,
            },
            questions: vec![q(
                "f01",
                QuestionKind::Factual,
                "who sent the screenshot",
                "obs-9",
            )],
        };
        let recaller = MockRecaller {
            rules: vec![(
                "screenshot",
                vec![
                    hit("obs-1", "nope"),
                    hit("obs-9", "the screenshot"),
                    hit("obs-3", "nope"),
                ],
            )],
        };
        let r = score(&set, &recaller, 3);
        assert_eq!(r.recall_at_1, 0.0);
        assert_eq!(r.recall_at_k, 1.0);
    }

    #[test]
    fn substring_fallback_matches_on_hit_text() {
        let set = QuestionSet {
            meta: QuestionSetMeta {
                captured_week: "w".to_owned(),
                authored_by: String::new(),
                k: 3,
            },
            questions: vec![q(
                "c01",
                QuestionKind::Commitment,
                "what did I promise Chris",
                "quote by Friday",
            )],
        };
        let recaller = MockRecaller {
            rules: vec![(
                "promise",
                vec![hit("obs-42", "I'll send the quote by Friday")],
            )],
        };
        let r = score(&set, &recaller, 3);
        assert_eq!(r.recall_at_k, 1.0);
        assert_eq!(r.per_kind_recall_at_k[&QuestionKind::Commitment], 1.0);
    }

    #[test]
    fn gate_thresholds() {
        assert_eq!(gate_verdict(0.72), BrainBranch::AgidbAsIs);
        assert_eq!(gate_verdict(0.70), BrainBranch::AgidbAsIs);
        assert_eq!(gate_verdict(0.55), BrainBranch::RetrievalMerge);
        assert_eq!(gate_verdict(0.40), BrainBranch::RetrievalMerge);
        assert_eq!(gate_verdict(0.39), BrainBranch::StoreSwap);
        assert_eq!(gate_verdict(0.0), BrainBranch::StoreSwap);
    }
}
