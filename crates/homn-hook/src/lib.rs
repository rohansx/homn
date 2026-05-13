//! Claude Code hook integration + PTY-tap wrapper.
//!
//! Two integration paths:
//!
//! 1. **Hook**: `homn hook <event>` subcommand reads the hook payload on stdin, calls the daemon,
//!    writes the hook return on stdout. Primary integration.
//! 2. **PTY-tap wrapper**: `homn run claude ...` spawns Claude under a PTY, races the daemon
//!    decision against the user's terminal prompt, synthesizes `y\n` / `n\n` keystrokes when the
//!    daemon decides within the race window. Fallback for the deny path while Anthropic bug
//!    [#19298](https://github.com/anthropics/claude-code/issues/19298) is open.
//!
//! See [`specs/001-policy-engine/contracts/hook-protocol.md`](../../../specs/001-policy-engine/contracts/hook-protocol.md)
//! for the wire format and [`docs/architecture/adr/0003-pty-fallback.md`](../../../docs/architecture/adr/0003-pty-fallback.md)
//! for the rationale.
//!
//! Implementation lands across T029 (hook subcommand) and T053–T057 (PTY wrapper).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
