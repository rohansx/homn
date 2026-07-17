# homn v2 — the local human

**Product overview** · v0.2 · 2026-07-17

> A second brain that lives entirely on your machine. It watches everything you do — screen, calls, dictation, email — passes it through a privacy gate, builds a temporal model of your life, and surfaces it back to you (and to Claude) exactly when you need it. Not a recorder. Not a chatbot. A clone that remembers.

This document supersedes [`docs/product/overview.md`](../product/overview.md) (the Phase-1 "policy engine for coding agents" framing). The permission daemon is not discarded — it is **absorbed**: the policy engine, audit log, daemon chassis, and installer become the governance layer of the larger product. See §6.

---

## 1. One-liner

**homn**: a local-first ambient memory + clone daemon. It ingests your digital life continuously, understands it as entities / commitments / beliefs over time, and exposes that understanding via MCP (v1) and a proactive live-meeting copilot (v2).

## 2. The problem

Every AI assistant starts cold. You re-explain your context every session. Meeting notes die in silos. The tools that do capture your life either:

- **record without understanding** (Screenpipe — a searchable ledger you must query),
- **understand without memory** (HeyClicky — a screenshot-per-question goldfish),
- **remember without temporal structure** (minimi — vectors that can't answer "what did I promise?"),
- or **claim "local" while relaying your data through their backend** → Gemini / Deepgram (minimi again).

## 3. The wedge (v1): "minimi, but honest about locality, with a real brain"

- **Distribution:** MCP server → paste connector link into Claude. No UI to build. Two-minute setup. Claude is the body.
- **Differentiation #1 — your memory never leaves your machine.** Capture, redaction, storage, and recall are all on-device. Recall is deterministic local math — zero network calls in the read path. Cloud models participate only at *write time* (extraction) and only past the redaction gate, using **your own API key**, with a receipt for every disclosure. This is the honest scoping of "local" (see [architecture §4](./architecture.md#4-the-localcloud-split)): categorically stronger than minimi's raw-capture backend relay, without pretending we run a frontier model on a laptop.
- **Differentiation #2 — temporal memory:** the bi-temporal substrate (agidb) answers commitment / belief questions natively: *"what did I promise by Friday"*, *"how has my position on X changed since March"* — first-class queries, not vector-retrieval-and-pray.
- **Differentiation #3 — privacy as architecture:** redaction gate *before* disk (CloakPipe in-process), and non-destructive **unlearn** with an audit receipt ("forget everything about this person"). Nobody else in the category can say either sentence.
- **Differentiation #4 — a policy engine over the assistant itself.** Inherited from homn v1: deny / ask / allow rules with hot-reload, wall-clock budgets, rule tracing, and an audit row for every decision. In v1 this governs what leaves the gate; in v2 it governs what the clone may say, do, and interrupt. No competitor has this layer at all.

## 4. The product (v2): the proactive clone

Meeting detection → live transcript → continuous match against your memory → whispered suggestions: *"you quoted $X to this person 3 weeks ago; you never replied to his follow-up."* Cursor-buddy presence (clicky-style) + local voice pipe. A small local model triages continuously (cheap, private); the frontier model is called only when a suggestion clears the bar — this is also the unit-economics unlock: always-on cloud inference at ~$0.25/action does not survive contact with reality.

## 5. Positioning map

| | Capture | Memory / understanding | Interface | Privacy pipeline |
|---|---|---|---|---|
| Screenpipe | ✅ excellent, OSS | ❌ FTS + sqlite only | ⚠️ search UI | ✅ local (no redaction) |
| HeyClicky | ⚠️ on-demand screenshot | ❌ none | ✅ excellent | ❌ cloud models |
| OpenHuman | ⚠️ account connectors | ⚠️ summary trees | ✅ desktop app | ⚠️ mixed local/cloud |
| minimi | ✅ ambient Mac capture | ⚠️ vectors only | ✅ MCP→Claude | ❌ backend relay to Gemini/Deepgram |
| **homn** | ♻️ reuse Screenpipe + own ASR | ✅ agidb (temporal, beliefs, unlearn) | ✅ MCP v1, clicky-style v2 | ✅ CloakPipe gate + policy engine + receipts |

## 6. What homn was, and what carries over

homn v1 shipped a complete, tested policy engine for coding agents ("polkit for Claude Code"): Rhai rules, SQLite audit, Claude Code hook, PTY-tap enforcement, MCP introspection, learning/suggestion engine, one-command install. 142 tests, all green as of 2026-07-17.

That was the governance layer built before the thing it governs. The absorption map:

| v1 asset | v2 role |
|---|---|
| `homn-daemon` chassis (Tokio, unix sockets, event bus) | `homnd` ingestion spine — direct reuse |
| `homn-policy` (Rhai engine, hot-reload, budgets, trace) | the gate's policy brain + v2 interrupt policy |
| `homn-audit` (SQLite, WAL, single-writer) | redaction ledger + decision receipts + unlearn audit |
| `homn-mcp` (rmcp server, rate limiter) | patterns + transport for the v1 MCP product |
| installer, systemd/launchd units, `homn setup` | direct reuse |
| `homn-face` (Tauri spike) | **parked** → Phase 5 "body" |
| `homn-hook` / PTY tap (the old product) | **kept as a feature**: Claude Code stays one governed actuator |
| `homn-tui`, `homn-learning` | parked; learning patterns inform v2 triage |

## 7. Who it's for

- **v1:** Claude power users who already feel the "stop briefing Claude" pain — devs, founders, consultants. The minimi audience, minus the privacy compromise.
- **v2:** anyone who takes >3 calls/day and loses context between them.
- **Strategic:** homn is the composed demo of the local-first agent-infra thesis. agidb (memory) + CloakPipe (privacy) + primd (retrieval) + the policy engine (permissions) become legible in one visceral experience — even if homn itself stays a dogfood tool, it sells the stack.

## 8. Non-goals

- Not a cloud service. The store and read path never leave the machine; cloud participates per-call, by policy, with receipts.
- Not a general chatbot. Claude is the conversational surface in v1; homn is the memory and the gate.
- Not a Screenpipe competitor. We consume capture; we don't rebuild it (`Source` trait keeps it swappable).
- Not Mac-first. Dogfood target is the author's Linux machine (CachyOS + KDE Wayland); macOS is a launch-audience decision at Phase 3 (see [plan §Phase 3](./tech-plan.md#phase-3--mcp-v1-the-shippable-product)).
- No always-on cloud inference. The read path is local math; the v2 loop is local-triage-first by design.

## 9. What "done" looks like (v1)

- 7 days of the author's life ingested, <5% average CPU, redaction ledger populated.
- Claude, via connector, answers the seven query types ([architecture §6](./architecture.md#6-query-surface-mcp-tools-v1)) about the real week, with receipts.
- `homn forget` works end-to-end and produces an audit receipt.
- Install-to-first-answer under 5 minutes on a clean machine.

*Naming note: shipping as **homn** means the README is rewritten decisively — "homn is the local human: memory, permissions, presence" — rather than leaving two meanings alive. That rewrite is a Phase 3 task.*
