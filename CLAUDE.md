<!-- SPECKIT START -->
# homn — guidance for AI coding agents working in this repo

**Active spec**: [`specs/002-ambient-memory/`](./specs/002-ambient-memory/) — homn v2, the local human (ambient memory v1, Phases 0–3). Supersedes 001 as the active work; [`specs/001-policy-engine/`](./specs/001-policy-engine/) remains the shipped record of the v1 policy engine (absorbed as the governance layer).

Read in this order when picking up work:

1. [`.specify/memory/constitution.md`](./.specify/memory/constitution.md) — non-negotiable principles
2. [`docs/v2/`](./docs/v2/) — the pivot: product-overview, architecture, tech-plan (source of truth)
3. [`specs/002-ambient-memory/spec.md`](./specs/002-ambient-memory/spec.md) — what we're building (user stories US1–US7, the five invariants)
4. [`specs/002-ambient-memory/plan.md`](./specs/002-ambient-memory/plan.md) — how (with constitution gate checks + phase sequencing)
5. [`specs/002-ambient-memory/research.md`](./specs/002-ambient-memory/research.md) · [`data-model.md`](./specs/002-ambient-memory/data-model.md) · [`contracts/`](./specs/002-ambient-memory/contracts/) — decisions, entities, interface contracts
6. [`specs/002-ambient-memory/tasks.md`](./specs/002-ambient-memory/tasks.md) — the task list, grouped by user story
7. [`specs/002-ambient-memory/quickstart.md`](./specs/002-ambient-memory/quickstart.md) — how to run + eval locally

Long-form architecture and rationale live in [`docs/`](./docs/) and [`docs/architecture/adr/`](./docs/architecture/adr/). Don't duplicate; reference.

## Technologies

- **Language**: Rust stable (1.88+).
- **Async runtime**: Tokio (only).
- **Storage**: SQLite via `rusqlite` + `tokio-rusqlite`.
- **Policy DSL**: Rhai with hard wall-clock budgets.
- **IPC**: Unix sockets, JSON-line RPC.
- **TUI**: `ratatui` + `crossterm`.
- **PTY**: `portable-pty`.
- **MCP**: `rmcp`.
- **CLI**: `clap` (derive).

## Project structure

```
homn/
├── Cargo.toml                       # workspace
├── crates/
│   ├── homn-bin/                    # binary, subcommand dispatch
│   ├── homn-daemon/                 # long-running process
│   ├── homn-policy/                 # Rhai engine + rules eval
│   ├── homn-audit/                  # SQLite layer
│   ├── homn-hook/                   # claude code hook + PTY tap
│   ├── homn-tui/                    # TUI prompt renderer
│   ├── homn-mcp/                    # MCP server
│   ├── homn-learning/               # pattern detector + suggestion engine
│   └── homn-types/                  # shared types
├── tests/                           # integration tests
├── specs/                           # spec-kit features
└── docs/                            # architecture, ADRs, phases, research
```

## Shell commands

```sh
cargo run -p homn-bin -- daemon --foreground     # boot the daemon
cargo run -p homn-bin -- log --since 1h          # tail audit
cargo run -p homn-bin -- rule edit               # edit policies/default.rhai
cargo run -p homn-bin -- run claude              # PTY-wrapped Claude
cargo test --workspace                            # all tests
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
```

## Constitution gates

Every PR must pass the gates in [plan.md §Constitution Check](./specs/001-policy-engine/plan.md#constitution-check). Specifically:

- Tests first for `homn-policy`, `homn-audit`, `homn-hook` (Constitution VI).
- Audit log records every decision with rule source (Constitution III).
- Local-first; no network in core (Constitution I).
- Conservative defaults (Constitution V).
<!-- SPECKIT END -->
