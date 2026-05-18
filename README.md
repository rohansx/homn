# homn

> the homunculus for your coding agents — a local-first daemon for staying in control of an autonomous dev environment

`homn` is one Rust process that gives you three stacked layers of agency over your AI coding agents (Claude Code, Codex, Gemini CLI, opencode — anything with a hooks API):

1. **policy** — decides what the agent is allowed to do without you
2. **face** — an expressive ASCII character that tells you what's happening without you having to ask
3. **brain** — a context graph (`ctxgraph`) that remembers what you've done so the daemon can tell you what you've forgotten

each layer is independently useful. each one makes the next one more interesting. they ship in that order.

## status

**v0 alpha — usable for the deterministic deny path today.** see [`docs/getting-started.md`](./docs/getting-started.md) for a 5-minute walkthrough. The TUI prompt for `ask` decisions, the PTY-tap wrapper, the face, and ctxgraph integration land in later phases.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/rohansx/homn/master/install.sh | sh
homn setup
```

`homn setup` seeds a policy, installs the Claude Code hook (with a backup), and starts
the background daemon. It is idempotent — safe to re-run. Reverse it any time with
`homn uninstall`.

<details>
<summary>Or, step by step / build from source</summary>

```sh
cargo install --git https://github.com/rohansx/homn homn-bin
homn rule edit          # seed + edit your policy
homn install --apply    # install the Claude Code hook
homn daemon             # or install a service unit — see docs/getting-started.md
```
</details>

## optional: wire homn as an MCP server

This is the novel piece that no other Claude-Code policy tool has. Once configured, Claude can call `query_policy` *before* attempting an action — *"would my rules allow rm -rf here?"* — and adjust its plan accordingly. Add to `~/.claude.json`:

```jsonc
{
  "mcpServers": {
    "homn": {
      "command": "homn",
      "args": ["mcp", "stdio"]
    }
  }
}
```

Three tools become available:

- **`query_policy(tool, tool_input, cwd)`** — dry-run evaluation. Returns the decision the engine *would* make, the rule that would fire, and the rule's source location. **Doesn't log to audit, doesn't mutate state.**
- **`explain_decision(decision_id)`** — look up an audit row by id; useful for understanding why a prior call was denied so the agent can propose an alternative.
- **`recent_decisions(limit, decision, tool, grep)`** — tail the audit log; the agent can ask *"what was just denied?"* before re-attempting.

## quick links

- [Product overview](docs/product/overview.md) — what we're building and why
- [Architecture overview](docs/architecture/overview.md) — three-layer design
- [Phase 1 — Policy engine](docs/phases/phase-1-policy.md) — weeks 1–4
- [Risks & open questions](docs/risks/known-unknowns.md) — honest take on what could break this
- [Research: polkit deep dive](docs/research/polkit-deep-dive.md) — the model we're borrowing
- [Research: Claude Code hooks](docs/research/claude-code-hooks.md) — the integration surface

## non-goals

- not a notification toast that disappears in 5s — those are what we're replacing
- not a cloud service — everything local, sync is opt-in
- not a wrapper around Claude Code — `homn` is a *peer process* via the hooks API
- not a multi-tenant SaaS — single-user tool; team features ship as shared rules files
- not a pressure tool — no whips, no "go faster" prompts
- not a replacement for `claude agents` (the official dashboard) — `homn` complements it

## license

Apache-2.0 (core daemon, rules engine, face, ctxgraph integration). Team rule-file sync will be open-core.

## name

`homn` is short for *homunculus* — a small thing that lives at the edge of your terminal and watches what's happening. It's not an acronym; it's a vibe.
