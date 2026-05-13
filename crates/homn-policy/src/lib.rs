//! Rhai-based policy engine.
//!
//! Evaluates a `.rhai` ruleset in deny → ask → allow order; first match wins. Each rule has a
//! hard wall-clock budget (50 ms default); each call has a total budget (200 ms default).
//!
//! See [`docs/technical/policy-language.md`](../../../docs/technical/policy-language.md) for the DSL,
//! [`docs/architecture/policy-engine.md`](../../../docs/architecture/policy-engine.md) for the architecture,
//! and [`specs/001-policy-engine/spec.md`](../../../specs/001-policy-engine/spec.md) §"User Story 1" for
//! the acceptance criteria.
//!
//! Implementation is TDD-mandatory per Constitution Principle VI. Tasks T020 (tests) and T023 (engine)
//! are next.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
