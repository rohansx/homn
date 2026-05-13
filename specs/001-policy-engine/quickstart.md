# Quickstart — homn Phase 1

> 5-minute onboarding for someone joining the project (you, future-you, or an external contributor).

## What you need installed

- Rust stable (1.83+). `rustup default stable`.
- `cargo`, `git`.
- `sqlite3` (for ad-hoc audit DB inspection).
- A working Claude Code install (≥ 2.0).

## Clone + build

```sh
git clone https://github.com/<owner>/homn.git
cd homn
cargo build --workspace
```

(The repo doesn't exist yet — Phase 1 Week 1 task is to scaffold the workspace.)

## Run the daemon (development mode)

```sh
cargo run -p homn-bin -- daemon --foreground
```

In another terminal, exchange a ping:

```sh
echo '{"id":"01H","method":"ping","params":{}}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/homn.sock
# expect: {"id":"01H","result":{"pong":true}}
```

## Wire up Claude Code

```sh
# Print the snippet to merge into your ~/.claude/settings.json
cargo run -p homn-bin -- install

# Or auto-apply (idempotent; backs up the existing settings.json):
cargo run -p homn-bin -- install --apply
```

Then in a new terminal run `claude` normally — the hook routes permission requests through the daemon.

## Inspect the audit log

```sh
cargo run -p homn-bin -- log --since 1h
cargo run -p homn-bin -- log --denied --since 24h
cargo run -p homn-bin -- log --since 1h --json | jq .
```

## Edit policy

```sh
cargo run -p homn-bin -- rule edit
# opens $EDITOR on ~/.config/homn/policies/default.rhai
# changes are hot-reloaded; no daemon restart needed
```

## Run with PTY wrapper (guaranteed deny)

```sh
cargo run -p homn-bin -- run claude
# spawns claude with the wrapper; same UX, deny path is enforced even with #19298
```

## Run tests

```sh
cargo test --workspace                      # all unit + integration
cargo test -p homn-policy                   # just the rules engine
cargo test --test e2e_hook                  # integration test against fake Claude Code
```

## Lint + format

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

CI runs both on every PR; both must pass.

## Project layout (after Week 1)

```
homn/
├── Cargo.toml                       # workspace
├── crates/
│   ├── homn-bin/                    # binary, subcommand dispatch (clap derive)
│   ├── homn-daemon/                 # the long-running process
│   ├── homn-policy/                 # Rhai engine + rules eval
│   ├── homn-audit/                  # SQLite layer
│   ├── homn-hook/                   # claude code hook + PTY tap
│   ├── homn-tui/                    # TUI prompt renderer
│   ├── homn-mcp/                    # MCP server
│   ├── homn-learning/               # pattern detector + suggestion engine
│   └── homn-types/                  # shared types (Decision, BusEvent, etc.)
├── tests/
│   ├── integration/
│   └── fixtures/                    # canned hook payloads, fake Claude binary
├── specs/                           # spec-kit features (this folder)
└── docs/                            # long-form architecture, ADRs, phases, research
```

## Where to look when stuck

| Symptom                                                            | Look here                                                                                          |
|--------------------------------------------------------------------|----------------------------------------------------------------------------------------------------|
| Hook isn't being called                                             | `~/.claude/settings.json` — verify `homn hook permission-request` is in the right matcher           |
| Daemon socket doesn't exist                                         | `systemctl --user status homn` (Linux) / `launchctl list | grep homn` (macOS); check `~/.config/homn/homn.toml` |
| Rule evaluates but doesn't fire                                     | `cargo run -p homn-bin -- rule trace <tool> <input>` shows why each rule did/didn't match           |
| Audit DB grows large                                                | Verify daily compaction job is running; check `audit.retention_days` in `homn.toml`                  |
| `homn run claude` doesn't synthesize keystrokes                     | Bug — check `pty_wrapper.prompt_regex` matches Claude's current prompt format; add snapshot test    |
| MCP client doesn't see the server                                   | `~/.claude.json` — verify `mcpServers.homn = { command: "homn", args: ["mcp", "stdio"] }`           |

## Docs index

- [docs/product/overview.md](../../docs/product/overview.md) — what we're building and why
- [docs/architecture/overview.md](../../docs/architecture/overview.md) — three-layer system
- [docs/architecture/adr/](../../docs/architecture/adr/) — load-bearing decisions
- [docs/risks/known-unknowns.md](../../docs/risks/known-unknowns.md) — what could break this
- [.specify/memory/constitution.md](../../.specify/memory/constitution.md) — non-negotiable principles
