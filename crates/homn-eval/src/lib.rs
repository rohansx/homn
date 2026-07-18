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

pub mod schema;
pub mod score;

pub use schema::{EvalError, Question, QuestionKind, QuestionSet, QuestionSetMeta};
pub use score::{gate_verdict, score, BrainBranch, Hit, OpsMetrics, Recaller, RunResult};
