# ADR-0006 — Expose `homn` as an MCP server

**Status**: Accepted

## Context

`homn` is integrated into Claude Code primarily as a hook target and (optionally) a PTY wrapper. Both paths are *gates* — the daemon answers "may this happen?" The pasted overview proposed an additional path: expose `homn` itself as an MCP server, letting the agent query the daemon from within its own reasoning loop.

This is undersold in the original overview. Worth its own ADR because it's the most novel piece of the design.

## Decision

`homn` exposes an MCP server (via the [`rmcp`](https://crates.io/crates/rmcp) reference implementation) over both stdio and Streamable HTTP transports. Tools surfaced:

| Tool                          | Returns                                                          |
|-------------------------------|------------------------------------------------------------------|
| `query_policy(tool, input)`   | Dry-run: what decision would `homn` make for this call? (no log) |
| `explain_decision(id)`        | The rule that fired, the rule's source location, ctxgraph context if any |
| `suggest_rule(pattern)`       | A Rhai rule that would let the agent do this whole class         |
| `recent_decisions(filters)`   | Tail of the audit log, filterable by tool / cwd / decision       |
| `ctxgraph_query(q)`           | Proxied to ctxgraph's MCP server — single tool surface for agents|

### Rationale

- **Agents that can introspect their constraints make better decisions.** When Claude knows *why* a previous attempt was denied, it can propose a different approach (the right hashicorp-style "ask forgiveness, not permission" failure mode is to ask for forgiveness *intelligently*).
- This is **a genuine novelty.** No existing tool exposes the policy engine to the agent as a queryable peer. Notification wrappers gate; policy hooks gate; `homn` gates *and* lets the agent learn.
- It composes cleanly with other MCP servers: ctxgraph, supabase, github, whatever. `homn` is just another peer in the agent's tool set.
- It gives security researchers a real surface to study agent behavior under policy constraints — a launch-worthy artifact.

### Rejected alternatives

| Alternative                              | Reason rejected                                                |
|------------------------------------------|----------------------------------------------------------------|
| Expose only the unix socket; no MCP      | Agents can't introspect their own constraints — biggest missed opportunity |
| Expose via REST + OpenAPI                | MCP is the agent-native protocol; REST is for human/CI consumers |
| Hide policy from the agent for "safety"  | The agent can see its own audit log; pretending otherwise is just opaque |

## Consequences

- The MCP server is the **primary launch story for Phase 1.** "Polkit for coding agents — and the agent can ask the daemon what's allowed." This is the differentiator the original brm overview underplayed.
- Both stdio and Streamable HTTP transports ship in v1. Stdio for Claude Code's MCP config; HTTP for cross-machine setups and `claude agents` multi-host scenarios.
- The MCP surface is the integration point for future tools (other coding agents, IDE plugins, CI). If we keep the tool API stable across `homn` versions, integrations are durable.
- `suggest_rule` is partially LLM-shaped (pattern-matching with templating) but **does not call an external LLM**. It uses string analysis + the user's existing rule patterns to suggest new ones. Keeps the deterministic-rules-first commitment from [ADR-0002](0002-rust-rhai.md).
