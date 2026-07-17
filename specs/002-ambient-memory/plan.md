# Implementation Plan: homn v2 — Ambient Memory (local human, v1)

**Branch**: `claude/v2-docs-implementation-m8eixq` | **Date**: 2026-07-17 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/002-ambient-memory/spec.md`

## Summary

Turn homn from "policy engine for coding agents" into a local-first ambient memory + clone daemon, absorbing the shipped v1 policy/audit/daemon stack as the governance layer. This plan covers the **shippable v1 (Phases 0–3)**: a validation-week eval gate that decides the brain architecture from real recall data, an ingestion spine (`homnd`) that tails Screenpipe + convox-voice through a swappable `Source` abstraction, an in-process privacy gate (redaction + Rhai ingest policy + hash-chained receipts) that precedes the store, and an MCP surface of seven query tools over the temporal memory store (agidb). The technical approach is *composition over construction*: reuse the tested `homn-daemon` chassis, `homn-policy` engine, `homn-audit` ledger, and `homn-mcp` transport; consume `agidb`, `cloakpipe`, and (conditionally) `ctxgraph` as external crates. Everything is bound by the five invariants and the constitution.

## Technical Context

**Language/Version**: Rust stable, MSRV 1.83 (workspace-pinned).

**Primary Dependencies**:
- *Reused in-repo*: `homn-daemon` (Tokio runtime, unix sockets, event bus, supervisor), `homn-policy` (Rhai engine, hot-reload via `notify`, wall-clock budgets, rule trace), `homn-audit` (SQLite/WAL single-writer), `homn-mcp` (`rmcp` server + rate limiter), `homn-types`, `homn-bin` (clap subcommand dispatch), installer.
- *New in-repo crates*: `homnd`, `homn-sources`, `homn-gate`, `homn-eval`; extensions to `homn-mcp` and `homn-bin`.
- *External crates consumed*: `agidb` (temporal brain: observe/recall/beliefs/goals/unlearn, GLiNER extraction, potion embeddings, MCP-serve primitives), `cloakpipe` (redaction regex bank + NER + hash-chained evidence ledger), `ctxgraph` (retrieval-tier fallback — pulled in **only** if the Phase 0 gate selects the 40–70% or <40% branch).
- *Ecosystem*: `tokio`, `rusqlite`+`tokio-rusqlite`, `rhai`, `rmcp`, `serde`/`serde_json`, `clap`, `ulid`, `blake3` (content hashing / hash chain), `xxhash-rust` (fast dedupe pre-filter), `chrono`, `notify`, `arc-swap`, `thiserror`/`anyhow`, `tracing`.

**Storage**: SQLite via `rusqlite`/`tokio-rusqlite` for the audit/redaction ledger and ingestion watermarks (constitution: no separate DB service). The memory store is `agidb`'s own embedded store (redb + mmap signatures), consumed as a library — not a new database service. Screenpipe's sqlite is read-only upstream (tailed, never written).

**Testing**: `cargo test --workspace`; `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`. Test-first (Constitution VI) is mandatory for `homn-gate`, ledger code in `homn-audit`, and the `homnd` pipeline (watermark recovery, dedupe, sessionizer). Eval harness is a hand-scored + regression harness, not a unit test.

**Target Platform**: Linux (dogfood: CachyOS/Arch, KDE Plasma 6 Wayland, Ryzen 7 7435HS, RTX 4050 6 GB, 23 GB RAM). macOS is a deferred, launch-signal-gated decision. convox-voice is Linux-only.

**Project Type**: Single Rust workspace, one binary (`homn`), subcommand-driven (Constitution VII). Long-running daemon + CLI + MCP server surfaces.

**Performance Goals**:
- Ingest average CPU < 5% over a working day (SC-001).
- Read path: recall answered as local math, zero network calls (Invariant 2, SC-006).
- Install-to-first-answer < 5 min on a clean machine (SC-005).
- Dedupe collapses repeated screen text to a small fraction of raw frames (SC-009).

**Constraints**:
- Nothing unredacted touches disk; gate precedes store; fail closed (Invariant 1, FR-011/012).
- Cloud only at write time, post-gate, per-policy, user's key, receipted (Invariant 4).
- Resident-model budget ≤ 6 GB VRAM; **no heavy local model required for v1** (read path is deterministic; extraction is cloud-first).
- Single binary; new behavior = subcommand.

**Scale/Scope**: One user (dogfood), 7+ days continuous capture. Thousands of raw capture frames/day collapsing (post-dedupe) to a much smaller observation count. Four new crates + two extended crates + an eval harness. v1 ends at Phase 3 (ship).

**Open items resolved by Phase 0 data, not this plan**: the brain architecture branch (agidb as-is vs. ctxgraph retrieval merge vs. ctxgraph as store) is decided by measured recall@3 — see [research.md](./research.md) R1.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design (result at bottom).*

| Principle (see [constitution.md](../../.specify/memory/constitution.md)) | Check | Notes |
|---|---|---|
| I. Local-First, Always | ✓ pass | Capture, gate, store, recall all on-device. The **only** network use is write-time cloud extraction/synthesis — opt-in, user's own key, post-gate, per-policy, receipted (Invariant 4). Read path has zero egress (Invariant 2). This is the honest scoping of "local," and it is a *loud opt-in*, satisfying I + V. Documented in research.md R4. |
| II. Deterministic Rules Before LLM Judgment | ✓ pass | Ingest/pass-gate decisions are deterministic Rhai rules (same engine as v1). No LLM in the decision path. Cloud LLM participates only in write-time *enrichment* (commitment/belief extraction), never in a policy or read decision. |
| III. Audit Everything (NON-NEGOTIABLE) | ✓ pass | Every ingest decision, every redaction, every cloud disclosure, every deletion → a receipt in the `homn-audit` ledger (FR-015/021/024). Hash-chained, tamper-evident, no plaintext. |
| IV. The Agent Can Introspect Its Constraints | ✓ pass | The MCP surface exposes memory *and* (inherited) policy introspection; the agent can see what it may recall and why something was redacted/withheld, without privilege escalation. |
| V. Conservative Defaults, Loud Opt-Ins | ✓ pass | Sensitive capture surfaces OFF by default (FR-026). Cloud enrichment OFF until a key + policy opt-in. `homn pause`/destroy always available (FR-025). |
| VI. Test-First for the Policy Engine | ✓ pass | TDD mandatory for `homn-gate` (redaction + ingest policy), audit/ledger code, and the `homnd` pipeline (watermark recovery, dedupe, sessionizer). Tests written → fail → implemented. CLI plumbing / MCP glue tested loosely. |
| VII. One Binary, Subcommand-Driven | ✓ pass | All new behavior ships as `homn` subcommands (`homn capture/ingest`, `homn exclude`, `homn pause`, `homn status`, `homn forget`, `homn destroy`, `homn eval`, `homn connect`). Lib crates re-export through `homn-bin`. |

**Technical-standards checks**: Tokio only ✓ · SQLite via rusqlite/tokio-rusqlite (no separate DB service; agidb is an embedded library, not a service) ✓ · Unix-socket JSON-line IPC for local control (no D-Bus/HTTP for local IPC; MCP-over-HTTP is a remote *product* surface, opt-in) ✓ · Rhai with wall-clock budgets ✓ · `homn-types`/`homn-policy` change-frozen below 1.0 → new memory types added additively / in new crates, no breaking edits ✓.

**Verdict**: PASS. No violations to justify; Complexity Tracking table omitted.

## Project Structure

### Documentation (this feature)

```text
specs/002-ambient-memory/
├── plan.md              # This file
├── research.md          # Phase 0 output — decisions R1..R10
├── data-model.md        # Phase 1 output — Observation/Session/Redaction/Commitment/Belief/receipts
├── quickstart.md        # Phase 1 output — run + eval locally
├── contracts/
│   ├── source-trait.md      # the Source abstraction (tail + poll-cursor shapes)
│   ├── gate-pipeline.md     # ingest → policy → redaction → store contract, fail-closed
│   ├── mcp-tools.md         # the seven query tools: params, result shapes, provenance
│   └── cli-commands.md      # homn subcommand surface + --json outputs
├── checklists/
│   └── requirements.md  # spec quality checklist (done)
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
homn/
├── Cargo.toml                       # workspace — add new members
├── crates/
│   ├── homn-types/                  # KEPT — add memory types additively (Observation, SourceKind, SessionId, RedactionRef…)
│   ├── homn-daemon/                 # KEPT — chassis reused by homnd (runtime, sockets, event bus, supervisor)
│   ├── homn-policy/                 # KEPT — ingest-policy evaluation (Rhai, hot-reload, budgets, trace)
│   ├── homn-audit/                  # KEPT + extend — redaction ledger, disclosure + deletion receipts (hash chain)
│   ├── homn-mcp/                    # KEPT + extend — the seven memory tools over agidb (rmcp)
│   ├── homn-bin/                    # KEPT + extend — new subcommands
│   ├── homn-hook/                   # KEPT as-is — Claude Code stays one governed actuator (untouched in v1 memory work)
│   ├── homn-learning/               # PARKED — patterns inform v2 triage later
│   ├── homn-tui/                    # PARKED
│   ├── homnd/                       # NEW — ingestion daemon: pipeline, watermarks, backpressure, sessionizer
│   ├── homn-sources/                # NEW — Source trait + ScreenpipeTail + DictationPipe (+ poll-cursor scaffold for Phase 3.5)
│   ├── homn-gate/                   # NEW — cloakpipe redaction stage + homn-policy ingest rules + homn-audit receipts
│   └── homn-eval/                   # NEW — Phase 0 harness: ingest replay, 30-Q scoring, recall@k, ops metrics
├── eval/                            # NEW — question set (authored per-run), fixtures, scoring config → CI regression
├── policies/                        # KEPT + add ingest.rhai (per-app/domain deny + gate-pass rules)
├── install/                         # KEPT — installer extended to install screenpipe if absent, print MCP link
├── tests/                           # workspace integration tests (gate fail-closed, forget-receipt, no-egress read path)
└── docs/v2/                         # source-of-truth docs (already written)
```

**Structure Decision**: Single Rust workspace (Constitution VII), extending the existing one. New capability lands as **new crates** (`homnd`, `homn-sources`, `homn-gate`, `homn-eval`) so the change-frozen `homn-types`/`homn-policy` are touched only additively, and the tested v1 crates are reused rather than rewritten. External brains/redaction stay as separate consumed crates — "homn is the composition." The binary remains single; every surface is a `homn` subcommand re-exported through `homn-bin`.

## Phase sequencing (maps spec user stories → tech-plan phases)

| Tech-plan phase | Spec story | New/changed code | Gate |
|---|---|---|---|
| **Phase 0** — validation week | US1 | `homn-eval` + throwaway `ingest` path + `eval/` question set | recall@3 → brain branch (blocks Phase 2b) |
| **Phase 1** — ingestion spine | US2 (foundation) | `homn-sources`, `homnd` | watermark/dedupe/sessionizer tests green |
| **Phase 2** — the gate | US3 | `homn-gate`, `homn-audit` ledger ext, `policies/ingest.rhai` | fail-closed + tamper-evident ledger tests green |
| **Phase 2b** — brain merge (conditional) | US1 branch | `ctxgraph` retrieval tier fused into recall | eval set as regression; timeboxed 3 wks |
| **Phase 3** — MCP v1 (ship) | US2, US4, US5, US6, US7 | `homn-mcp` seven tools, write-time extraction, `homn-bin` subcommands, installer, README rewrite | seven queries answered w/ receipts; forget receipt; <5 min install |

Phase 3.5 (account connectors) and Phases 4–5 are **out of scope**; the only obligation carried here is the `Source`-trait forward-compat (FR-005a) verified by the poll-cursor contract in `contracts/source-trait.md`.

## Post-Design Constitution Re-Check

Re-evaluated after Phase 1 (research + data-model + contracts). Still **PASS**:
- The gate-pipeline contract makes Invariant 1 structural (the gate's output type *is* the storable Observation; no other constructor) — reinforces III/VI.
- The MCP-tools contract keeps tools 1–6 network-free (Invariant 2 / Constitution I); synthesis is Claude's, outside the server.
- Receipts (Decision/Disclosure/Deletion) in the data model cover every decision, disclosure, and deletion (Constitution III, Invariants 3–4).
- New types are additive to `homn-types` or in new crates; `homn-policy` reused unchanged (change-freeze respected).
- Source-trait contract's poll-cursor shape satisfies FR-005a without expanding v1 scope.

No new violations introduced by the design. Complexity Tracking remains empty.

## Complexity Tracking

No constitution violations — section intentionally empty.
