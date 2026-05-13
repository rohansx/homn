# Research — Policy Engine (Phase 1)

> The long-form research lives in [`docs/research/`](../../docs/research/). This file is the spec-kit-shaped index plus deltas specific to Phase 1.

## Already documented

- **Polkit pattern (the model we're borrowing)**: [`docs/research/polkit-deep-dive.md`](../../docs/research/polkit-deep-dive.md). Key takeaway: decision authority is separate from enforcement point is separate from auth agent. We map the same split onto `homn daemon` / hook (+ PTY wrapper) / TUI prompt (+ optional face).
- **Claude Code hook contract**: [`docs/research/claude-code-hooks.md`](../../docs/research/claude-code-hooks.md). Key takeaway: `PermissionRequest` hook is authoritative for `allow`. Bug [#19298](https://github.com/anthropics/claude-code/issues/19298) means `deny` is not enforced — we ship the PTY wrapper as a workaround.
- **Prior art landscape**: [`docs/research/prior-art.md`](../../docs/research/prior-art.md). Every existing tool handles exactly one slice (notification, allowlist, pet, dashboard). The novelty of `homn` is integrating policy + signal + memory as one daemon.

## Deltas to resolve before Phase 0 closes

These are *open* research items. Each gets a yes/no answer before the corresponding implementation task starts.

### R-001 — `portable-pty` macOS PTY size propagation

**Question**: Does `portable-pty` reliably propagate `SIGWINCH` and `winsize` updates to the child process on macOS, so that `claude`'s TUI redraws cleanly when the user resizes their terminal under `homn run claude`?

**Why it matters**: User-facing UX regression if Claude's TUI is stuck at the initial terminal size.

**Approach**: Spike a 30-line Rust program that wraps `bash` (then `claude`) and confirms `SIGWINCH` round-trips. ~1 hour of work.

**Owner**: implementer of `homn-hook/src/pty.rs`.

### R-002 — `rmcp` Streamable HTTP transport completeness as of v1

**Question**: Does `rmcp ≥ 0.3` ship a working Streamable HTTP transport, or are we shipping stdio-only in v1?

**Why it matters**: HTTP transport is the future of MCP; stdio is the present. If HTTP is wobbly in `rmcp`, ship stdio-only and document an upgrade path.

**Approach**: Read `rmcp` 0.3 release notes + open a smoke test server. ~2 hours.

**Owner**: implementer of `homn-mcp/`.

### R-003 — Claude Code's hook payload schema versioning

**Question**: Does Claude Code emit a schema version in the hook payload, or do we have to detect by content shape?

**Why it matters**: If the schema changes between Claude Code minor versions, we want to detect and gracefully degrade or refuse to evaluate. Versioning makes that mechanical.

**Approach**: Inspect Claude Code 2.x hook payloads across a few versions. Open an issue against `anthropics/claude-code` asking for explicit versioning if missing. ~1 day to inspect; potentially upstream PR.

**Owner**: implementer of `homn-hook/src/lib.rs`.

### R-004 — Rhai engine performance budget on real rules

**Question**: With ~50 rules in `default.rhai` (the example I've been sketching), what's the realistic p99 evaluation latency on commodity hardware (the author's CachyOS laptop, a M-series Mac, a cheap Linux VM)?

**Why it matters**: We've committed to ≤200 ms per call across all rules. If a realistic ruleset doesn't fit, we either raise budgets or pre-compile rules into a faster form.

**Approach**: Microbenchmark a synthetic ruleset using `criterion`. Tune `set_max_operations` against measured ops-per-rule. ~2 hours.

**Owner**: implementer of `homn-policy/`.

### R-005 — Anthropic on #19298

**Question**: Does Anthropic have public commitment / timeline on fixing PermissionRequest deny?

**Why it matters**: If the fix is imminent, we de-emphasize PTY wrapper in marketing. If not, it leads the launch story.

**Approach**: Read GitHub issue thread; reach out to Anthropic devrel via Twitter / Discord. Passive monitoring through Phase 1.

**Owner**: author.

## Decisions captured from research

These are *closed* — captured as ADRs:

- [ADR-0001 — naming](../../docs/architecture/adr/0001-naming.md)
- [ADR-0002 — Rust + Rhai](../../docs/architecture/adr/0002-rust-rhai.md)
- [ADR-0003 — PTY-tap fallback](../../docs/architecture/adr/0003-pty-fallback.md)
- [ADR-0004 — Tauri over egui for the face](../../docs/architecture/adr/0004-tauri-vs-egui.md) (Phase 2)
- [ADR-0005 — ctxgraph separate repo](../../docs/architecture/adr/0005-ctxgraph-separate.md) (Phase 3)
- [ADR-0006 — MCP server exposure](../../docs/architecture/adr/0006-mcp-server.md)
