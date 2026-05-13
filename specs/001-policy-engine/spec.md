# Feature Specification: Policy Engine (Phase 1)

**Feature Branch**: `001-policy-engine`

**Created**: 2026-05-13

**Status**: Draft

**Input**: User description: "Phase 1 of homn — the policy engine. A daemon + hook integration + audit log + TUI prompt + PTY wrapper + MCP server. The MVP that solves 'I want to stop babysitting permission prompts.'"

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Rule-based allow/deny on a real Claude Code session (Priority: P1) 🎯 MVP

A solo developer installs `homn`, configures the Claude Code hook, and runs `claude` in a project. A destructive Bash call (`rm -rf` outside `/tmp`) is denied by a rule in `default.rhai` and logged to the audit DB. A benign read (`Read` inside `$HOME`) is allowed silently.

**Why this priority**: This is the entire reason `homn` exists. Without it, every other story is decoration. P1 is "the daemon does its job for at least one real session."

**Independent Test**: With `homn daemon` running and the Claude Code hook installed, run `claude` and ask it to `rm -rf ~/dummy-dir`. The agent's call is denied; `homn log --denied` shows the row with the rule that fired.

**Acceptance Scenarios**:

1. **Given** `homn daemon` is running and `~/.claude/settings.json` has the `PermissionRequest` hook for `homn hook permission-request`, **When** Claude Code attempts `Bash` with `cmd="rm -rf ~/scratch"`, **Then** the daemon evaluates `default.rhai`, the matching `deny` rule fires, the hook returns `{behavior: "deny"}`, and a row lands in `audit.db` with `rule_source` populated.
2. **Given** the daemon is running, **When** Claude Code attempts `Read` with `path="$HOME/foo.txt"`, **Then** the `allow` rule fires, returns immediately, and the audit row records `decision="allow"` with `latency_ms < 50`.
3. **Given** the daemon is running, **When** Claude Code attempts a tool/path with no matching rule, **Then** the default `ask if true;` rule fires; the TUI prompt appears in the calling terminal; the user types `a` (allow); the audit row records `decision="ask"`, `human_answer="allow"`, `surface="tui"`.

### User Story 2 — Audit log inspection (Priority: P2)

The same developer queries `homn log` after a day's work and gets a chronological list of decisions with rule citations.

**Why this priority**: The audit log is the killer retention feature. Without `homn log`, the daemon's decisions are invisible to the user; the "I don't remember approving that" pain isn't fixed. P2 because U1 produces the *data*; U2 makes it *readable*.

**Independent Test**: After running Claude Code with `homn daemon` for one hour with mixed tool calls, run `homn log --since 1h` and confirm the output matches what was decided. `homn log --denied` returns only denies. `homn log --json` returns valid JSON consumable by `jq`.

**Acceptance Scenarios**:

1. **Given** at least 10 decisions in `audit.db` in the last hour, **When** the user runs `homn log --since 1h`, **Then** the CLI prints one line per decision in reverse-chronological order with timestamp, decision, tool, input preview, and rule source.
2. **Given** at least one denial in the last hour, **When** the user runs `homn log --denied --since 1h`, **Then** only `decision="deny"` rows are shown.
3. **Given** the `--json` flag, **When** the user runs `homn log --since 1h --json`, **Then** stdout is newline-delimited JSON, one object per row, parseable by `jq`.

### User Story 3 — PTY-tap wrapper guarantees deny (Priority: P3)

Same developer runs `homn run claude ...` instead of `claude ...`. When the daemon decides `deny` for a permission request, the wrapper writes `n\n` to Claude's stdin within 200 ms — *even if Anthropic's `PermissionRequest` hook deny bug is still in effect*.

**Why this priority**: Hard-guarantees the deny semantic regardless of upstream bug #19298. Lower priority than U1+U2 because U1 already provides deny-via-hook (which works for users not blocked by #19298, and degrades to audit-only otherwise). U3 closes the gap.

**Independent Test**: With `homn run claude` and a rule that denies a specific Bash command, ask Claude to run that command. Confirm Claude receives `n\n` synthesized and does not proceed with the tool call. Confirm `homn log` shows `source="pty-wrapper"` and `decision="deny"`.

**Acceptance Scenarios**:

1. **Given** a rule that denies `cmd.matches("git push * main")`, **When** the user runs `homn run claude` and Claude attempts the matching call, **Then** the PTY wrapper writes `n\n` to Claude's stdin within `deny_race_window_ms`, the audit row records `source="pty-wrapper"`.
2. **Given** the daemon returns `allow` for a request, **When** the wrapper sees the prompt, **Then** it writes `y\n` to stdin and Claude proceeds.
3. **Given** the daemon doesn't respond within `deny_race_window_ms`, **When** the prompt is still pending, **Then** the wrapper does *not* synthesize a keystroke; the user's interactive prompt remains, they decide normally.

### User Story 4 — Learning suggestions promote rules (Priority: P4)

After the user has resolved an `ask` decision the same way 5 times in a row for the same pattern, `homn` surfaces a "promote to rule?" prompt. The user accepts; the appropriate policy file gains an annotated rule.

**Why this priority**: The differentiator vs every existing notification tool. P4 because U1–U3 ship the surface; U4 adds intelligence. Crucially, learning is *suggestion-only* — never auto-promotes.

**Independent Test**: Resolve `ask` for `Bash: "cargo test"` 5 times in a row with `allow`. Then run `homn learning list` — the suggestion appears. Run `homn learning accept <id>` — the rule is appended to `default.rhai` with a generated comment.

**Acceptance Scenarios**:

1. **Given** 5 consecutive `ask → allow` decisions for the same pattern hash, **When** the user runs `homn learning list`, **Then** a suggestion appears with the proposed rule text.
2. **Given** an open suggestion, **When** the user runs `homn learning accept <id>`, **Then** the rule is appended to `default.rhai` (or the project file) with a comment recording the source decisions, and the suggestion is removed.
3. **Given** the user rejects a suggestion via `homn learning reject <id>`, **When** the same pattern triggers again, **Then** the suggestion is not re-shown for 30 days.

### User Story 5 — Agent introspects its own constraints via MCP (Priority: P5)

The user adds `homn` to their `~/.claude.json` MCP servers. During a session, Claude calls `query_policy("Bash", {command: "git push --force origin main"})` and gets back `{decision: "deny", rule_source: "default.rhai:10"}`. Claude proposes an alternative path instead of attempting the call.

**Why this priority**: The most novel piece of `homn`. P5 because it's a differentiator, not a daily-driver — most users won't notice it, but the launch story relies on it being demonstrable.

**Independent Test**: Add `homn mcp stdio` to Claude's MCP config. Ask Claude to run a tool that you know is denied. Confirm Claude pre-queries via `query_policy` and references the deny in its response, rather than attempting and failing.

**Acceptance Scenarios**:

1. **Given** `homn` is configured as an MCP server in `~/.claude.json`, **When** Claude calls `query_policy(tool, input)`, **Then** the daemon returns the decision it *would* make without logging or affecting state.
2. **Given** a prior denied decision exists in `audit.db`, **When** Claude calls `explain_decision(id)`, **Then** the daemon returns the rule that fired with source location and a snapshot of the rule text.
3. **Given** the user asks Claude to perform a class of action that should be allowed, **When** Claude calls `suggest_rule(verb="allow", example_tool, example_input)`, **Then** the daemon returns a draft Rhai rule. The rule is **not** written to disk by the MCP call.

### Edge Cases

- **Daemon is down when a hook fires**: hook subcommand connects, fails after one retry with 250 ms backoff, exits 0 with empty response — Claude falls through to its default. No silent allow.
- **Policy file has a syntax error**: daemon retains the last-good version, logs the error, surfaces a `LearningSuggestion`-style nudge to the user, never crashes.
- **Rhai rule hangs**: per-rule 50 ms wall-clock budget enforced via `Engine::set_max_operations`. Exceed → log + treat as non-match. Per-call 200 ms total budget → exceed → return `ask`.
- **SQLite locked during high-rate decisions**: writer task batches with 10 ms window; reads via WAL never block.
- **PTY-wrapper prompt regex stops matching after a Claude Code version change**: wrapper logs the miss, falls back to letting the user decide via the visible terminal prompt. Snapshot tests catch this in CI against pinned Claude Code versions.
- **Audit DB grows large**: daily compaction job runs at 03:30 local; default 30-day retention; configurable to `0` (keep forever).
- **MCP client floods `query_policy`**: rate-limited per session (100 req/min); excess returns rate-limit error to the agent.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST evaluate Rhai policy files in **deny → ask → allow** order; first matching rule wins.
- **FR-002**: System MUST persist every decision to `audit.db` with the rule source, rule text snapshot, decision, latency, and surface that answered.
- **FR-003**: System MUST hot-reload policy files via `inotify` (Linux) / `kqueue` (macOS) on change; reload MUST be atomic (old rules remain active until new file parses cleanly).
- **FR-004**: System MUST enforce wall-clock budgets per rule (50 ms) and per call (200 ms total); exceeding the per-rule budget treats that rule as non-match; exceeding the per-call budget returns `ask`.
- **FR-005**: System MUST expose a Unix-socket JSON-line RPC API and an MCP server (stdio + Streamable HTTP) over the same data plane.
- **FR-006**: System MUST support a PTY-tap wrapper invoked as `homn run claude ...` that races daemon decisions against the user's terminal prompt and synthesizes y/n keystrokes when the daemon decides within `deny_race_window_ms` (default 200 ms).
- **FR-007**: Users MUST be able to query the audit log via `homn log` with `--since`, `--denied`, `--allowed`, `--asked`, `--session`, `--tool`, `--grep`, `--json` filters.
- **FR-008**: System MUST surface learning suggestions when an `ask` pattern has 5 consecutive matching human answers; suggestions are **never** auto-promoted.
- **FR-009**: System MUST allow project-scoped policy overrides; a session's `cwd` determines whether a `<repo-slug>.rhai` overlay applies.
- **FR-010**: System MUST support optional remote-approval mirroring via ntfy when the user is idle ≥N minutes (configurable; default disabled).

### Key Entities

- **Decision** — A single policy evaluation result: id, ts, session_id, cwd, tool_name, tool_input (capped 4 KiB), decision (allow/deny/ask), human_answer (nullable), rule_source, rule_text, ctxgraph_hit (nullable, Phase 3+), latency_ms, surface, source.
- **Policy file** — A `.rhai` file at `$XDG_CONFIG_HOME/homn/policies/`. Hot-reloaded. The `default.rhai` is always loaded; project files overlay based on `cwd`.
- **Rule** — One line in a policy file: `<verb> if <expression>;`. Verb ∈ {allow, deny, ask}.
- **Learning suggestion** — A pattern of N consecutive same-answer asks, surfaced as a candidate rule for user acceptance.
- **Surface** — A registered consumer of `AskOpened` events: TUI (default in v1), face (opt-in, ships in Phase 2), ntfy (opt-in).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The daemon runs continuously on the author's machine for ≥7 days without an unplanned restart, with ≥1000 decisions in the audit DB.
- **SC-002**: P95 latency from hook fire to hook return (the critical path) is ≤500 ms for deterministic decisions, ≤2.5 s for `ask` decisions answered via TUI.
- **SC-003**: Within 30 days of Phase 1 launch (HN post), at least 3 external users have written and shared custom rule files.
- **SC-004**: The author reads `homn log --denied --since 7d` at least once per week within 30 days of running the daemon themselves (signal that the audit log is the killer feature for retention).
- **SC-005**: Within 30 days post-launch, ≥100 GitHub stars on the project.
- **SC-006**: At least one external user reports a meaningful "I would have approved a bad thing without `homn`" event in a public post (Reddit / X / blog).

## Assumptions

- Target users run Claude Code (or another agent with comparable hooks) on Linux or macOS. Windows is explicitly out of v1 scope.
- The `PermissionRequest` hook contract is the one Claude Code documents as of 2026-Q2; we pin a tested version range in `homn install`.
- Anthropic's bug #19298 (PermissionRequest deny ignored) may or may not be fixed during Phase 1. We assume it is *not* and ship the PTY wrapper anyway.
- Users own `~/.claude/settings.json`; `homn install` writes a snippet but doesn't auto-merge without `--apply`.
- The user's filesystem permits `$XDG_RUNTIME_DIR` Unix sockets (true on all mainstream Linux distros and macOS).
- ctxgraph integration is out of scope for Phase 1 — covered by Phase 3.
