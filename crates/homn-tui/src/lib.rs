//! TUI surface for `homn` permission prompts.
//!
//! The default surface in v1 (face is opt-in, Phase 2). Renders the *ask* prompt directly in the
//! calling terminal using `ratatui` + `crossterm`. Hotkeys: `a` allow, `d` deny, `A`/`D` always
//! variants, `s` show generated rule, `q` defer to Claude's own prompt.
//!
//! See [`docs/architecture/policy-engine.md`](../../../docs/architecture/policy-engine.md)
//! §"TUI prompt".
//!
//! Implementation lands in T031.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
