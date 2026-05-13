# ADR-0003 — PTY-tap fallback for the deny path

**Status**: Accepted

## Context

The pasted product overview assumed `PermissionRequest` hook was authoritative for both `allow` and `deny` returns. Research into the current Claude Code state ([research/claude-code-hooks.md](../../research/claude-code-hooks.md)) found:

- [anthropics/claude-code#19298](https://github.com/anthropics/claude-code/issues/19298): the `PermissionRequest` hook fires correctly and its return is logged, but **`{ behavior: "deny" }` does not prevent the interactive prompt from showing**. Allow works. Deny does not.
- This breaks the polkit-style guarantee that "the first matching deny rule stops the action."
- Anthropic may fix this upstream, but we cannot commit to a ship date that depends on someone else's fix.

We need a deny path that works **today**, without modifying Claude Code, and that degrades gracefully if Anthropic eventually fixes the bug.

## Decision

`homn` ships a **PTY-tap wrapper** invoked as `homn run claude ...`. It:

1. Spawns `claude` as a child process under a pseudo-terminal.
2. Reads `claude`'s stdout through the master fd, plumbs it to the user's terminal unchanged.
3. Regex-matches the permission prompt pattern (`Do you want to proceed? (y/n):`).
4. On match, calls the daemon over the unix socket in parallel.
5. If the daemon returns `deny` within ~200ms, the wrapper writes `n\n` to `claude`'s stdin (a synthesized keystroke).
6. If the daemon returns `allow`, the wrapper writes `y\n`.
7. If the daemon doesn't return in time, the user's interactive prompt remains visible — they decide normally.

The wrapper is **opt-in**. Users who run `claude` directly (no wrapper) still get the hook-based path for `allow` decisions; `deny` decisions degrade to "logged but not enforced" until Anthropic fixes #19298.

### Rejected alternatives

| Alternative                       | Reason rejected                                                   |
|-----------------------------------|-------------------------------------------------------------------|
| Wait for Anthropic to fix #19298  | Can't commit to a ship date that depends on someone else          |
| Wrap the Claude Agent SDK         | Reimplements TUI, slash commands, sessions — owns the whole world |
| Fork `claude` CLI                 | Maintenance burden; ToS implications                              |
| Build as a permission MCP server  | The bug also affects the MCP permission flow                      |

## Consequences

- v1 has two integration paths: hook (`claude`) and wrapper (`homn run claude`). Users opt into the wrapper for stronger deny semantics.
- The prompt-detection regex is brittle to format changes — pin in a config file, add snapshot tests, ship a fast update mechanism.
- The wrapper is a small surface (~500 LOC of Rust around `portable-pty` + a regex), so the maintenance tax is bounded.
- If Anthropic fixes #19298, the wrapper becomes optional (still useful for users who want belt-and-suspenders) but stops being required.
- Documentation must be explicit about which path the user is on: the install README should say *"want deny enforcement now? use `homn run claude`. happy with deny-as-audit-only? use `claude` directly."*
