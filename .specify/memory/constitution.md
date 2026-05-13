# homn Constitution

> The non-negotiable principles for `homn`. These bind every PR, every ADR, every change. Read [docs/product/overview.md](../../docs/product/overview.md) for product context; read this file for the rules of engagement.

## Core Principles

### I. Local-First, Always

`homn` runs entirely on the user's machine. No telemetry, no cloud calls, no network egress without explicit opt-in. The only paid feature (team rule-file sync) is itself a local-first protocol with end-to-end signatures. *If a feature needs the network to function at all, it does not belong in core.*

### II. Deterministic Rules Before LLM Judgment

Policy decisions are deterministic Rhai rule evaluations. LLM-in-the-loop judgment is a non-goal — it's slow, expensive, and produces decisions you can't reproduce. The `suggest_rule` tool uses pattern analysis, not Claude calls. Any feature that introduces non-determinism into the decision path must justify it in an ADR.

### III. Audit Everything (NON-NEGOTIABLE)

Every policy decision lands in `audit.db` with the rule that fired (or "no rule, asked"), the surface that answered, latency, and full input snapshot. *Opaque approve/deny is the failure mode of every existing tool. We do not ship that mode.* Retention is the user's choice; the *recording* is not.

### IV. The Agent Can Introspect Its Constraints

`homn` exposes an MCP server. The agent can query its own policy, explain past decisions, and propose rules — all without escalating privilege. This is a feature, not a leak. Pretending the agent shouldn't know its constraints makes the system more opaque, not safer.

### V. Conservative Defaults, Loud Opt-Ins

The face is OFF by default. Transcript ingestion is OFF by default. Network surfaces (ntfy, HTTP MCP) are OFF by default. Anything that can degrade privacy, focus, or autonomy ships off; users turn it on with eyes open. *Defaults are policy.*

### VI. Test-First for the Policy Engine

The Rhai engine, audit log, hook contract, and PTY wrapper are TDD-mandatory. Tests written → reviewed → fail → implemented. Other crates (CLI plumbing, face UI) can be tested loosely. The decision pipeline itself is non-negotiable.

### VII. One Binary, Subcommand-Driven

`homn` ships as a single binary. New behaviors are subcommands, not separate binaries. Lib crates inside the workspace re-export through `homn-bin`. *Users install one thing; we ship lockstep.*

## Technical Standards

- **Language**: Rust stable, MSRV-pinned per release.
- **Async runtime**: Tokio. No mixing of async runtimes.
- **Storage**: SQLite via `rusqlite` + `tokio-rusqlite`. No separate database services.
- **IPC**: Unix sockets, JSON-line RPC. No D-Bus. No HTTP for local IPC.
- **DSL**: Rhai with hard wall-clock budgets enforced via `set_max_operations`.
- **Hook contract**: Versioned. `homn install` pins a Claude Code version range.
- **CLI**: `clap` with derived help. Every subcommand has `--json` output for scripting.

## Development Workflow

- Each phase has a spec (`specs/<###-feature>/`), an implementation plan, a tasks file, and ADRs for load-bearing decisions.
- The spec-kit workflow (`/speckit-specify`, `/speckit-plan`, `/speckit-tasks`, `/speckit-implement`) is the authoritative path for new features; freeform docs in `docs/` are the *long-form reference*.
- Pull requests must reference (a) the user story they advance, (b) the ADRs they comply with, and (c) any constitution principle they bend (require justification).
- The `homn-types` and `homn-policy` crates are change-frozen below `1.0.0`. Breaking changes require an ADR and a migration path.

## Governance

This constitution supersedes ad-hoc preferences. Amendments require:

1. An ADR proposing the change with the rationale.
2. A 7-day review window for any maintainer to push back.
3. Bumping the constitution version + dated last-amended marker below.

Disagreements during PR review are resolved by quoting the principle; if the principle is silent or ambiguous, amend it before merging.

**Version**: 0.1.0 | **Ratified**: 2026-05-13 | **Last Amended**: 2026-05-13
