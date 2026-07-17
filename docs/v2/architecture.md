# homn v2 — architecture & tech spec

v0.2 · 2026-07-17 · companion to [`product-overview.md`](./product-overview.md) and [`tech-plan.md`](./tech-plan.md)

---

## 1. The five layers

```
┌─────────────────────────────────────────────────────────────┐
│  L5  BODY        clicky-style cursor buddy · voice (v2)     │
│                  Claude via MCP (v1)                        │
├─────────────────────────────────────────────────────────────┤
│  L4  REFLEXES    primd — predictive retrieval, <100ms       │
│                  live-context injection for meetings (v2)   │
├─────────────────────────────────────────────────────────────┤
│  L3  BRAIN       agidb — observe/recall/beliefs/goals       │
│                  bi-temporal supersession · consolidation   │
│                  unlearn · self-model · MCP serve           │
├─────────────────────────────────────────────────────────────┤
│  L2  GATE        cloakpipe redaction (in-process) — PII/    │
│                  secrets stripped BEFORE anything persists  │
│                  homn-policy (Rhai) decides what may pass   │
│                  homn-audit receipts every decision         │
├─────────────────────────────────────────────────────────────┤
│  L1  SENSES      screenpipe (screen OCR + a11y tree + app   │
│                  events) · convox-voice ASR (working today) │
│                  account connectors (gmail/slack/gh) — 3.5  │
└─────────────────────────────────────────────────────────────┘
```

L2 is where homn v1 lives on: the gate is CloakPipe's redaction **plus** the existing Rhai policy engine **plus** the existing audit crate. One gate, three inherited muscles.

## 2. Data flow (v1)

```
 screen events ─┐
 ambient audio ─┤→ screenpipe sqlite ─┐
 dictation ─────┘   (convox-voice     │
                     is push-based,   │
                     its own Source)  ▼
                              homnd (ingestion daemon, Rust)
                                      │  1. poll/tail new rows (crash-safe watermark)
                                      │  2. dedupe + chunk into Observations
                                      │  3. gate: policy check → cloakpipe redaction
                                      │  4. agidb.observe(text, meta)
                                      ▼
                                  agidb store
                       (redb + mmap sigs + GLiNER extraction
                        + consolidation pass)
                                      │
                                      ▼
                              agidb serve (MCP stdio/HTTP)
                                      │
                                      ▼
                          Claude Desktop / claude.ai connector
```

## 3. Component decisions

### Capture: reuse Screenpipe, don't rebuild

`screenpipe record` gives event-driven screen OCR, accessibility-tree text, audio, and app metadata into local sqlite with an API surface. Rebuilding this is 6 months of platform pain for zero differentiation. `homnd` *tails* Screenpipe's store. The working convox-voice dictation pipe is a second, higher-fidelity audio input (push-based, clean text) — wired in as its own source type. A `Source` trait keeps Screenpipe swappable (it's a YC company; their roadmap is not ours; their MIT core is forkable).

> **Status 2026-07-17:** Screenpipe is *not yet installed* on the dogfood machine — first task of Phase 0. convox-voice is running (systemd user service).

### Brain: agidb, conditional on the Phase 0 dogfood gate

agidb has the right primitives (observe, episodic→semantic consolidation, beliefs + confidence + provenance, bi-temporal supersession, unlearn, MCP serve) and its core suite passes (15/15, verified 2026-07-17). The open risk is HDC recall quality on noisy real-world volume. **The agidb/ctxgraph split is resolved by Phase 0 data, not preference:**

- ≥70% recall@3 on the eval → agidb as-is.
- 40–70% → merge ctxgraph's benchmarked graph/retrieval into agidb as a parallel tier (HDC signatures become *an* index, not the whole bet). One brain. Timeboxed.
- <40% → ctxgraph becomes the store; agidb's belief/goal/unlearn types ported on top.

Either way **agidb supersedes ctxgraph in homn's roadmap** — the old Phase-3 "wire in ctxgraph" plan is retired.

### Gate: CloakPipe in-process + Rhai policy (not Cedar)

Redaction is a library call in `homnd`'s pipeline (regex bank for secrets/keys/cards/aadhaar/PAN, NER-based PII for third parties), not a network proxy hop. Every redaction event goes to a hash-chained evidence ledger (reuse CloakPipe's ledger code) — *"here is a cryptographic log of what was stripped and why"* is a demo moment.

**Policy language decision: Rhai, not Cedar.** The source plan suggested Cedar for CloakPipe consistency. Rejected for v1: homn already ships a tested Rhai engine (hot-reload, wall-clock budgets, rule trace, 142 green tests), and one product must not carry two policy languages. Per-app ingest policies (`never ingest: password managers, banking tabs, incognito`) are Rhai rules evaluated by the same engine that governed Claude Code in v1. Revisit only if policy files are ever shared with CloakPipe proper.

### Interface v1: none

`agidb serve` over MCP is the product. A tray icon + `homn pause` is the entire GUI budget. The Tauri face crate stays parked until Phase 5.

## 4. The local/cloud split

The five invariants (§7) draw the line precisely. What they require to be local is **cheap**; what they permit in the cloud is **the expensive part**. No heavy local models are needed — the design constraint on this hardware (RTX 4050, 6 GB VRAM, faster-whisper resident) is honored by architecture, not by squeezing.

| Job | Where | Model / mechanism | Status |
|---|---|---|---|
| ASR (dictation) | **local** | convox-voice → faster-whisper (CUDA) | ✅ running today |
| ASR (ambient) | **local** | Screenpipe's whisper | Phase 0 install |
| Entity/relation extraction | **local** | GLiNER ONNX (CPU, already inside agidb) | ✅ shipped in agidb |
| Embeddings (tier E) | **local** | potion-base-8M (already inside agidb) | ✅ shipped in agidb |
| Recall / read path | **local** | deterministic HDC + lexical math, zero network | ✅ shipped in agidb |
| Redaction | **local** | CloakPipe regex + NER, in-process | Phase 2 |
| Commitment/belief extraction (write time) | **cloud first** | Claude Haiku via user's key, post-redaction, receipted; qwen2.5:3b evaluated later as a local swap-in | Phase 3 |
| Answer synthesis | **cloud** | Claude itself (it's the MCP client — this costs us nothing) | Phase 3 |
| Triage (v2 proactive loop) | **local** | 3b-class (qwen2.5:3b). *Not* 12B — a 7.4 GB Q4 does not fit 6 GB VRAM beside whisper | Phase 4 |
| Suggestion composition (v2) | **cloud** | Claude via API, only past the triage threshold | Phase 4 |

Cloud-at-write-time is compatible with agidb's own core invariant ("LLMs may participate at write time — never at read") and with invariant 4 below. The marketing claim is scoped honestly: *your memory never leaves; reasoning uses your own key, through the gate, with receipts.*

## 5. Core data model

```rust
// homnd's normalized unit before it hits the gate
struct Observation {
    id: Ulid,
    source: SourceKind,        // ScreenOcr | A11yTree | AmbientAudio | Dictation | Email | Slack ...
    app: Option<String>,       // "Slack", "Chrome:gmail.com", "Zoom"
    captured_at: DateTime,     // valid-time start
    text: String,              // post-redaction
    redactions: Vec<RedactionEvent>,   // type, span-hash, policy id (no plaintext)
    session: Option<SessionId>,        // meeting/work-session grouping
    speaker: Option<SpeakerTag>,       // for audio: me | other | unknown
    content_hash: Blake3,      // dedupe key
}
```

Session boundaries (meeting start/stop, app-focus blocks) are first-class — they let consolidation mint episode-level memories ("the July 14 call with X") instead of confetti.

## 6. Query surface (MCP tools, v1)

- `recall(cue, as_of?)` — cue-based recall with confidence + provenance
- `timeline(entity | topic, from, to)` — what happened, ordered
- `commitments(status?, due_before?)` — extracted promises, mine and theirs
- `beliefs(topic)` — current position + revision history
- `whodis(name)` — relationship dossier: every interaction, last thread, open loops
- `today()` / `standup()` — "what did I actually do"
- `forget(entity | timerange | pattern)` — the unlearn primitive, with audit receipt

The first five map 1:1 to minimi's marketing questions — except these are structured queries against a temporal store, not similarity search.

## 7. Non-negotiable invariants

1. Nothing unredacted touches disk. The gate precedes the store, always.
2. No network calls in the read path. Recall is local math.
3. Every memory has provenance; every deletion has a receipt.
4. Cloud models see only what a policy explicitly allows past the gate, per-call.
5. One command to pause everything; one command to destroy everything.

These extend homn v1's constitution (local-first, audit-everything, conservative defaults) rather than replacing it.

## 8. Repo layout

```
homn/
  crates/
    homnd/           # ingestion daemon (Phase 1) — built on homn-daemon's chassis
    homn-sources/    # Source trait + screenpipe/dictation impls (Phase 1)
    homn-gate/       # cloakpipe redaction + homn-policy rules + homn-audit receipts (Phase 2)
    homn-policy/     # kept from v1 — the gate's and (v2) the clone's policy brain
    homn-audit/      # kept from v1 — receipts, ledgers, unlearn audit
    homn-mcp/        # v1 toolset over agidb (Phase 3) — rmcp patterns from v1
    homn-live/       # meeting loop: primd + triage (Phase 4)
    homn-hook/       # kept — Claude Code stays one governed actuator
    homn-face/       # parked — Phase 5 body
  cli/               # homn status|pause|exclude|forget|eval
  eval/              # Phase 0 harness + 30-question set (becomes CI regression)
  install/           # curl|sh installer (reused from v1)
```

agidb, ctxgraph, cloakpipe, primd stay separate repos, consumed as crates — **homn is the composition, which is the thesis.**

## 9. Reference hardware (dogfood machine)

CachyOS / Arch · KDE Plasma 6 Wayland · Ryzen 7 7435HS (8c/16t) · RTX 4050 Laptop 6 GB · 23 GB RAM. Anything that must run resident (whisper + triage + desktop) is budgeted against the 6 GB card; that is why the triage ceiling is 3b-class and the 12B GGUFs in the local Ollama library are out of scope for this product.
