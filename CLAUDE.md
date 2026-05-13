<!-- SPECKIT START -->
# homn — guidance for AI coding agents working in this repo

**Active spec**: [`specs/001-policy-engine/`](./specs/001-policy-engine/) — Phase 1 (the policy engine MVP).

Read in this order when picking up work:

1. [`.specify/memory/constitution.md`](./.specify/memory/constitution.md) — non-negotiable principles
2. [`specs/001-policy-engine/spec.md`](./specs/001-policy-engine/spec.md) — what we're building (user stories P1–P5)
3. [`specs/001-policy-engine/plan.md`](./specs/001-policy-engine/plan.md) — how (with constitution gate checks)
4. [`specs/001-policy-engine/tasks.md`](./specs/001-policy-engine/tasks.md) — the actual task list, grouped by user story
5. [`specs/001-policy-engine/quickstart.md`](./specs/001-policy-engine/quickstart.md) — how to run + test locally

Long-form architecture and rationale live in [`docs/`](./docs/) and [`docs/architecture/adr/`](./docs/architecture/adr/). Don't duplicate; reference.

## Technologies

- **Language**: Rust stable (1.83+).
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
