# homn

> the homunculus for your coding agents — a local-first daemon for staying in control of an autonomous dev environment

`homn` is one Rust process that gives you three stacked layers of agency over your AI coding agents (Claude Code, Codex, Gemini CLI, opencode — anything with a hooks API):

1. **policy** — decides what the agent is allowed to do without you
2. **face** — an expressive ASCII character that tells you what's happening without you having to ask
3. **brain** — a context graph (`ctxgraph`) that remembers what you've done so the daemon can tell you what you've forgotten

each layer is independently useful. each one makes the next one more interesting. they ship in that order.

## status

**pre-alpha — design phase.** see [`docs/`](./docs) for the full spec.

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
