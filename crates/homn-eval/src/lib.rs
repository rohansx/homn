//! Phase 0 recall-evaluation harness (spec 002, US1).
//!
//! The whole product is gated on one number: does the brain's recall survive real life? This crate
//! is the balanced-question-set schema ([`schema`]), the recall@k scorer decoupled from any store
//! ([`score`]), and the brain-architecture gate ([`score::gate_verdict`]). It is **buildable and
//! testable now** — the store side (agidb) plugs in through the [`score::Recaller`] trait later.
//!
//! See [`eval/README.md`] and [`specs/002-ambient-memory/`] (research R1, tasks T011–T016).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ops;
pub mod schema;
pub mod score;

#[cfg(feature = "brain-agidb")]
pub mod ingest;

pub use schema::{EvalError, Question, QuestionKind, QuestionSet, QuestionSetMeta};
pub use score::{gate_verdict, score, BrainBranch, Hit, OpsMetrics, Recaller, RunResult};

/// Format a [`RunResult`] + chosen [`BrainBranch`] as a human-readable verdict table for
/// `homn eval run`. Pure formatting — no store, no agidb — so it is testable in the no-feature
/// build (CI) and reused by the feature-gated CLI path.
pub fn format_report(result: &RunResult, branch: BrainBranch) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "recall@1 : {:.1}%", result.recall_at_1 * 100.0);
    let _ = writeln!(s, "recall@{}: {:.1}%", result.k, result.recall_at_k * 100.0);
    let _ = writeln!(s, "questions: {}", result.total);
    for (kind, r) in &result.per_kind_recall_at_k {
        let _ = writeln!(s, "  {:?}: {:.1}%", kind, r * 100.0);
    }
    if let Some(ops) = result.ops {
        let _ = writeln!(s, "ops:");
        let _ = writeln!(s, "  obs/day           : {:.1}", ops.observations_per_day);
        let _ = writeln!(s, "  disk growth bytes : {}", ops.disk_growth_bytes);
        let _ = writeln!(s, "  ingest cpu pct    : {:.1}", ops.ingest_cpu_pct);
        let _ = writeln!(s, "  extraction prec   : {:.2}", ops.extraction_precision);
    }
    let _ = writeln!(s, "gate: {}", branch.consequence());
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Question, QuestionKind, QuestionSet, QuestionSetMeta};

    fn run_result(recall_k: f64) -> RunResult {
        RunResult {
            k: 3,
            total: 30,
            recall_at_1: recall_k,
            recall_at_k: recall_k,
            per_kind_recall_at_k: [(QuestionKind::Factual, recall_k)].into_iter().collect(),
            ops: None,
        }
    }

    #[test]
    fn format_report_includes_recall_and_gate_consequence() {
        let r = run_result(0.72);
        let branch = gate_verdict(r.recall_at_k);
        let report = format_report(&r, branch);
        assert!(report.contains("recall@3: 72.0%"), "{report}");
        assert!(report.contains("questions: 30"), "{report}");
        assert!(report.contains("proceed with agidb as-is"), "{report}");
    }

    #[test]
    fn format_report_omits_ops_block_when_none() {
        let r = run_result(0.5);
        let report = format_report(&r, gate_verdict(r.recall_at_k));
        assert!(
            !report.contains("ops:"),
            "no ops block when ops is None: {report}"
        );
    }

    // Suppress an unused-fn warning for the Question import in case the schema shape changes.
    #[allow(dead_code)]
    fn _q(id: &str, kind: QuestionKind, q: &str, exp: &str) -> Question {
        Question {
            id: id.to_owned(),
            kind,
            question: q.to_owned(),
            expected_ref: exp.to_owned(),
            notes: String::new(),
            time_window: None,
        }
    }
    #[allow(dead_code)]
    fn _set() -> QuestionSet {
        QuestionSet {
            meta: QuestionSetMeta {
                captured_week: "w".to_owned(),
                authored_by: String::new(),
                k: 3,
            },
            questions: vec![],
        }
    }
}
