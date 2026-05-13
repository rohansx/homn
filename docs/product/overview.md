# Product overview

> A local-first daemon for staying in control of an autonomous dev environment. One Rust process, three stacked layers.

## TL;DR

`homn` is the peer process that sits next to your coding agents and handles three jobs they don't handle themselves:

1. **Policy** — evaluates rules and decides what the agent runs without you (analog: `polkitd`).
2. **Face** — encodes the state of your dev environment as an expressive ASCII character in a small always-on-top window (analog: a usable Clippy).
3. **Brain** — uses `ctxgraph` to remember sessions, commits, commands, and open loops, and surfaces what matters when it matters (analog: a personal RAG that actually lives in the loop).

Each layer ships on its own. Each layer makes the next more interesting. Stacked, they form one coherent surface for *"I want to be a human in the loop, even if the loop is mostly autonomous."*

## The problem

Coding agents make dev environments increasingly autonomous. You give a task, the agent works for ten minutes, sometimes it blocks asking for permission. In practice three things happen:

- **You babysit the terminal** waiting for the next permission prompt, defeating the autonomy.
- **You `--dangerously-skip-permissions`** and pray, which is fine until it isn't.
- **You run multiple sessions in parallel** and lose track of which one needs you, which one errored, which one finished.

Existing tooling treats each in isolation. Dozens of "send me a notification when Claude needs permission" wrappers exist. Anthropic just shipped `claude agents` for the parallel-session dashboard problem. Desktop pet projects exist as gimmicks with no real utility. Nobody has built the thing that handles all three coherently.

## Thesis

As agents do more work without you, humans need three control levers:

| Lever  | Question it answers                                | Layer |
|--------|----------------------------------------------------|-------|
| Policy | *What is allowed to happen without me?*            | 1     |
| Signal | *What is happening that I should glance at?*       | 2     |
| Memory | *What has happened that I have already forgotten?* | 3     |

`homn` is one process that answers all three.

## Who it's for

- **Primary**: solo developers running Claude Code (or Codex / Gemini CLI / opencode — anything with hooks) on Linux or macOS, often with 2–5 parallel sessions, who care about local-first tools and want a policy layer they can read and edit.
- **Secondary**: small teams that want a shared, version-controlled permission policy across machines without sending tool decisions through a cloud service.
- **Not for**: developers happy with `--dangerously-skip-permissions`. Windows users in v1 (later).

## What homn is not

- Not a competitor to Claude Code's `agents` view — that's the official multi-session dashboard.
- Not a remote-control / mobile-first tool (claude-remote-approver, claude-push already cover that).
- Not a productivity gamification thing (no XP, no streaks, no character to feed).
- Not an LLM-judgment layer like Anthropic's "auto mode" — `homn` is deterministic rules + learning, by design.
- Not a security product — it's a developer ergonomics product that has safety as a side effect.

## The three layers

### Layer 1 — Policy engine

> Polkit for coding agents. See [architecture/policy-engine.md](../architecture/policy-engine.md).

`homn` registers as a `PreToolUse` / `PermissionRequest` hook. Every tool call hits the daemon, which evaluates a rule file (Rhai DSL) and returns `allow` / `deny` / `ask`. Decisions land in a SQLite audit log with the rule that fired.

Differentiators vs existing notification tools:

| Existing notification tools             | homn layer 1                              |
|-----------------------------------------|-------------------------------------------|
| Stateless: one notification per prompt  | Stateful daemon with evaluated rules      |
| Binary allow/deny in the moment         | Learns patterns, *suggests* rules         |
| No audit trail                          | Every decision logged with reason         |
| Dumb pipe to mobile                     | Actual policy evaluation                  |
| Per-session config                      | Machine-wide, project-aware, syncable     |
| No API to query                         | MCP server: introspectable by the agent   |

Crucial detail (and a deviation from the pasted overview): **`PermissionRequest` hook deny is currently broken upstream** ([anthropics/claude-code#19298](https://github.com/anthropics/claude-code/issues/19298) — the hook fires but the interactive prompt still appears). v1 ships a **PTY-tap fallback** (`homn run claude ...`) for the deny path while we wait for the upstream fix. See [ADR-0003](../architecture/adr/0003-pty-fallback.md).

### Layer 2 — The face

> Clippy, but useful, and not annoying. See [architecture/face.md](../architecture/face.md).

A small (200×120) always-on-top window with an expressive ASCII character. It encodes the state of your dev environment in real time — sessions working, sessions waiting, builds passing, builds failing, PRs landing.

Important shift from the pasted overview: **the face defaults OFF in v1**. Audit log + TUI prompt is the daily-use surface. Face is opt-in for users who want the demo-tier experience. This eliminates the #1 product risk (face fatigue) and lets the face be a marketing surface without being a retention liability.

State vocabulary (placeholder art):

| State | Trigger |
|-------|---------|
| `◕ ◡ ◕` | idle, mild head-bob — all sessions calm |
| `◔_◔`  | tracking — one or more sessions working |
| `◉ ◉`  | eyes wide — a session needs permission |
| `x_x`  | error — last error visible on hover |
| `◕‿◕`  | quick smile — task completed / CI passed |
| `⊙_⊙`  | alert — high-stakes action waiting (push to main, deploy) |
| `¬_¬`  | mild raise — 25+ min on same file (stuck?) |
| `z_z`  | dozing — no activity for an hour |

### Layer 3 — The brain

> A personal RAG system that lives in a daemon and actually knows what you're working on. See [architecture/brain.md](../architecture/brain.md).

`ctxgraph` (your existing local-first context graph engine) wires into `homn` as a memory subsystem. The daemon ingests events (git, shell, transcripts, optionally calendar/mail/browser); `ctxgraph` stores them as a bi-temporal knowledge graph; the face surfaces what matters.

Three concrete user-facing wins:

- **Session resumption.** Opening `claude` in a project triggers *"last time here you were drafting the X email but didn't send it — open recap?"*
- **Open-loop surfacing.** Drafts that never sent, PRs you opened but didn't merge, TODOs from yesterday's conversation. At most one nudge per hour, only when idle.
- **Context-aware policy rules.** `ask if cmd matches "git push * main" && !ctxgraph.has_open_pr_passing_ci(...)`. Policy gets sharper because it has memory.

Privacy posture: everything local. Transcript ingestion is **opt-in by default** with a regex redaction layer for common secret patterns (see [ADR-0005](../architecture/adr/0005-ctxgraph-separate.md)).

## Differentiator: MCP introspection

`homn` exposes itself as an MCP server. The agent can ask the daemon:

- `query_policy(tool, args)` → *what would happen if I tried this?*
- `explain_decision(decision_id)` → *why was this decided?*
- `suggest_rule(pattern)` → *what rule would let me do this whole class?*

This is genuinely novel and undersold in the original brm overview. The agent can reason about its own constraints. **Worth its own launch arc** — bigger upside than the face demo.

## Naming

`homn` reads as *homunculus*. Daemon binary is `homn`; subcommands match (`homn daemon`, `homn rule`, `homn log`, `homn face`). See [ADR-0001](../architecture/adr/0001-naming.md).

## Go-to-market

- **Positioning**: *"the local-first control plane for your coding agents."*
- **Launch sequence**: Phase 1 → HN, Phase 2 → Twitter / Product Hunt (face demo video), Phase 3 → blog post + ctxgraph case study.
- **Business model**: open core. Core OSS; paid team rule-file sync. Modeled on Supabase / Atuin.

## Shipping timeline (realistic)

Adjusted from the pasted overview's 11-week estimate to reflect platform/window-management reality:

| Window | Ship |
|--------|------|
| Weeks 1–4  | Phase 1 v1 — daemon, Rhai rules, TUI prompt, audit log, MCP server, PTY-tap fallback. HN launch. |
| Weeks 5–10 | Phase 2 v0 — Tauri window, ASCII face, 8 core animations, opt-in. Demo video, Twitter launch. |
| Weeks 11–20| Phase 3 v0 — ctxgraph ingestion (git + transcripts), session resumption, searchable memory. |
| Weeks 21+  | Phase 3 expansion — open loops, calendar/mail/browser opt-ins, context-aware rules. |

Each phase ships on its own. If you stop at week 4, you've shipped a useful tool. Stop at week 10, you've shipped a viral tool. Ship all the way, you've shipped a coherent product.

## Open questions

Tracked in [risks/known-unknowns.md](../risks/known-unknowns.md). Top three:

1. PermissionRequest deny bug — does Anthropic fix it, do we PTY-fallback forever, or do we wrap the SDK?
2. ctxgraph schema migration — can layer 3's entity model land without breaking existing ctxgraph users?
3. Face fatigue — does the default-OFF policy hold up, or do power users still configure away?
