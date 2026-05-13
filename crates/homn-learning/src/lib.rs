//! Learning subsystem: detects consistent ask-resolution patterns and proposes rule promotions.
//!
//! Crucially **suggestion-only** — `homn` never silently modifies policy. After N consistent
//! same-answer asks for a pattern, a `LearningSuggestion` BusEvent fires; the user accepts or
//! rejects via `homn learning {accept,reject,snooze} <id>`.
//!
//! See [`docs/architecture/policy-engine.md`](../../../docs/architecture/policy-engine.md)
//! §"Learning".
//!
//! Implementation lands across T060–T068.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
