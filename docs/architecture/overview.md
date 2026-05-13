# Architecture — Overview

> One Rust binary. One long-running daemon. Three logical layers. Multiple surfaces. Read this once; then dive into the per-layer docs.

## The shape of the system

```
┌────────────────────────────────────────────────────────────────────────────┐
│                              homn daemon (rust)                            │
│                                                                            │
│  ┌──────────────┐    ┌──────────────────┐    ┌──────────────────────────┐  │
│  │ policy core  │◄───┤ event bus        ├───►│ ctxgraph (layer 3)       │  │
│  │ (layer 1)    │    │ (broadcast::Sender) │  │ knowledge graph (sqlite) │  │
│  │              │    │                  │    │ entity resolver + FTS5   │  │
│  └──────┬───────┘    └─────────┬────────┘    └──────────────────────────┘  │
│         │                      │                                           │
│         │                      │                                           │
│         ▼                      ▼                                           │
│  ┌──────────────┐    ┌──────────────────┐                                  │
│  │ rules eval   │    │ event sinks:     │                                  │
│  │ (rhai)       │    │ - git watch      │                                  │
│  │ + learning   │    │ - shell hist     │                                  │
│  │ + audit log  │    │ - cc hooks       │                                  │
│  │ (sqlite)     │    │ - cal/gmail      │                                  │
│  └──────┬───────┘    │ - browser ext    │                                  │
│         │            └──────────────────┘                                  │
│         │                                                                  │
│         │     ┌───────────────────────────┐                                │
│         └───► │ MCP server (rmcp)         │   ← exposes query_policy,      │
│               │   stdio + streamable HTTP │     explain_decision,          │
│               └───────────────────────────┘     suggest_rule, ctxgraph     │
└─────────┬───────────────────────────────────────────────────────┬──────────┘
          │                                                       │
          │ unix socket  ($XDG_RUNTIME_DIR/homn.sock)              │ event stream
          │ JSON-line RPC                                          │ (sse / long-poll)
          ▼                                                       ▼
   ┌────────────────────────┐                          ┌──────────────────────┐
   │ claude code            │                          │ face (layer 2)       │
   │ (hook calls)           │                          │ tauri window         │
   │                        │                          │ ascii character      │
   │ + homn run claude      │                          │ hover for context    │
   │   (PTY-tap fallback)   │                          │                      │
   │                        │                          │ OR: TUI prompt       │
   │ + TUI prompt           │                          │ (default in v1)      │
   └────────────────────────┘                          └──────────────────────┘
```

## Why one daemon, three layers

The daemon is the **only** persistent process. Everything else is a client:

- The **face** subscribes to the event bus — you can run `homn` headless or with the face on.
- **`ctxgraph`** is a queryable subsystem that both policy and face consume.
- Claude Code, the face, the CLI (`homn rule`, `homn log`), the optional browser extension — all talk to the daemon over a Unix socket.

This is the polkit pattern, adapted ([research/polkit-deep-dive.md](../research/polkit-deep-dive.md)):

| Polkit                                  | homn                                                      |
|-----------------------------------------|-----------------------------------------------------------|
| `polkitd` (decision authority)          | `homn daemon`                                             |
| `pkexec` / NetworkManager (enforcement) | Claude Code hook + PTY-tap fallback                       |
| polkit-gnome-agent / hyprpolkitagent    | `homn face` OR TUI prompt                                 |

## Why Rust

- Long-lived daemon: Tokio + Unix socket gives us thousands of req/s with negligible memory.
- Sub-millisecond rule evaluation: Rhai is embedded, no JIT cold start.
- Single static binary install: matters for a tool people install with `cargo install homn` or `brew install homn`.
- Native MCP server: `rmcp` crate is the reference implementation.
- Plays well with `ctxgraph`'s existing Rust codebase: no FFI boundary.

See [ADR-0002](adr/0002-rust-rhai.md) for the alternatives we rejected (Go, Python, Node).

## The boundaries that matter

For the system to be testable and replaceable, each layer must have a **clear API surface** to the layers above and below it.

### Layer 1 → Layer 2 (policy → face)

One-way event stream. Layer 1 emits structured events; layer 2 subscribes:

```rust
enum BusEvent {
    DecisionMade { id: DecisionId, tool: String, decision: Decision, rule: Option<RuleId> },
    AskOpened    { id: DecisionId, payload: HookPayload, context: Option<CtxgraphHit> },
    AskClosed    { id: DecisionId, answer: HumanAnswer, latency_ms: u32 },
    LearningSuggestion { rule_source: String, pattern: String, count: u32 },
    HighStakesPending { id: DecisionId, kind: HighStakesKind },
}
```

The face never *modifies* daemon state — it can only display events and forward user input back as a decision answer on the request-response socket.

### Layer 1 ↔ Layer 3 (policy ↔ brain)

Two-way, but narrow. Policy can *query* ctxgraph from inside a Rhai rule:

```rhai
allow if tool == "Read" && ctxgraph.recently_edited(path, hours: 24);
```

Policy can also *write* decision events to ctxgraph (a decision is an event worth remembering). The wire format is [docs/technical/ipc-protocol.md](../technical/ipc-protocol.md).

### Layer 2 → Layer 3 (face → brain)

Face *reads* from ctxgraph for hover panels and search:

```
hover the face → ctxgraph search(query) → results in right pane
```

No writes from face → brain. The brain is a derived store; only ingestors write to it.

## Surfaces (where decisions appear)

A single decision can manifest on any of these, depending on what's available:

| Surface       | When it's used                                            | Module          |
|---------------|-----------------------------------------------------------|-----------------|
| TUI prompt    | v1 default; SSH sessions; face muted; face not installed  | `homn::tui`     |
| Tauri face    | Opt-in; user has GUI session; face running                | `homn::face`    |
| ntfy push     | User AFK (idle ≥N min); user configured ntfy topic        | `homn::ntfy`    |
| MCP query     | Agent introspects (`query_policy`, `explain_decision`)    | `homn::mcp`     |
| `homn log`    | Post-hoc human review                                     | `homn::cli`     |

The daemon's decision pipeline is **surface-agnostic**: it produces an event, registered surfaces compete to render it, the first one to get a human answer wins, the rest get a cancel event.

## Storage layout

```
$XDG_CONFIG_HOME/homn/
├── homn.toml              # daemon config (paths, ntfy topic, etc.)
├── policies/
│   ├── default.rhai       # baseline rules
│   └── <repo-name>.rhai   # project overrides (matched by cwd)
└── ignored/               # rules learning has suggested but the user rejected

$XDG_DATA_HOME/homn/
├── audit.db               # SQLite: every decision logged
├── learning.db            # SQLite: pattern frequency for rule suggestions
└── face/                  # face state, position, mute settings

$XDG_RUNTIME_DIR/
└── homn.sock              # primary IPC socket
└── homn-events.sock       # event broadcast socket (subscribers only)
```

Ctxgraph storage lives at its own canonical location (`$XDG_DATA_HOME/ctxgraph/`) — `homn` is a consumer, not the owner.

## Cargo workspace layout (proposed)

```
homn/
├── Cargo.toml                # workspace root
├── crates/
│   ├── homn-bin/             # the binary, subcommand dispatch
│   ├── homn-daemon/          # long-running process, event bus, MCP
│   ├── homn-policy/          # Rhai integration + rule evaluation
│   ├── homn-audit/           # SQLite schema + queries
│   ├── homn-hook/            # Claude Code hook protocol + PTY tap
│   ├── homn-tui/             # TUI prompt renderer (ratatui)
│   ├── homn-face/            # Tauri command bindings (separate src-tauri/ for UI)
│   ├── homn-mcp/             # MCP server (rmcp)
│   ├── homn-ctxgraph/        # client adapter for ctxgraph
│   └── homn-types/           # shared types (BusEvent, Decision, etc.)
├── src-tauri/                # face UI (webview + svelte/react)
└── docs/
```

Bin re-exports lib crates so install is `cargo install homn` and you get one binary with subcommands.

## Per-layer documents

- [policy-engine.md](policy-engine.md) — Layer 1: Rhai rules, evaluation order, audit, learning
- [face.md](face.md) — Layer 2: Tauri window, state vocabulary, event subscription
- [brain.md](brain.md) — Layer 3: ctxgraph integration, ingestors, schema extensions
- [data-flow.md](data-flow.md) — End-to-end sequence diagrams
- [adr/](adr/) — Decision records (one per major architectural commitment)
