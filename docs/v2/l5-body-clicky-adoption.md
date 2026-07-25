# L5 Body — adopting Clicky as homn's visible presence + hands

**Status**: Proposed (design doc, not yet implemented) · 2026-07-23
**Supersedes**: the parked `homn-face` Tauri spike as the Phase 5 body ([`tech-plan.md`](./tech-plan.md) §Phase 5)
**Companion ADR**: [ADR-0008](../architecture/adr/0008-clicky-as-l5-body.md)

## 1. The thesis

homn is, today, a **memory + permission** layer: it captures (screenpipe + convox-voice),
remembers (agidb temporal memory), guards (Rhai policy + redaction + receipts), and is present
only through MCP tools (recall/timeline/commitments/whodis/today/forget). It has **no visible
presence, no voice-out, and no hands** — those were Phase 5 ("the body"), parked until v1
traction.

[Clicky](https://github.com/farzaa/clicky) (MIT) is an **AI companion that lives next to your
cursor**: it sees your screen, hears you (push-to-talk), talks back (TTS), and **points at /
clicks UI elements** (Computer Use). The cross-platform port **[ClickyX](https://github.com/unn-Known1/clickyX)**
is **Rust + Tauri + React, local-first, zero telemetry, MIT** — the same stack as our parked
`homn-face` spike — with a `localhost:32123` HTTP bridge, an `enigo`-based computer-use engine,
an automation scheduler, and **MCP CRUD support already built in**.

**Decision: adopt ClickyX as homn's L5 Body.** Instead of building the cursor-buddy presence +
computer-use from scratch, we fork ClickyX (MIT permits it) and wire it to homn's brain (memory)
and gate (policy). This is the combination nobody else has: **policy-gated, audited hands, grounded
in a memory only homn has.**

## 2. Layer fit (why Clicky is exactly L5)

| Layer | homn today | Clicky | Resolution |
|---|---|---|---|
| L1 Senses | screenpipe (continuous screen OCR + output audio) + convox-voice (push-to-talk dictation, persisted) | on-demand screenshots + push-to-talk (for the query) | **coexist at different cadences** — screenpipe for memory, Clicky for pointing (§5.1) |
| L2 Gate | ✅ Rhai policy + redaction + hash-chained receipts (built) | "Privacy Guard" (skips vault screenshots) | **homn's gate governs Clicky's actions** (§4.2) |
| L3 Brain | ✅ agidb temporal memory (built) | flat per-app journal (20-msg cap) | **Clicky uses homn's memory via MCP**, drops its journal (§4.1) |
| L4 Reflexes | planned (primd) | — | unaffected |
| **L5 Body** | ❌ parked (`homn-face` spike) | ✅ cursor buddy + pointing + `enigo` computer-use + TTS + automation scheduler | **ClickyX becomes the body** (this doc) |

Clicky is not a chatbot; it is a runtime (tray + overlay + bridge + agents + computer-use + skills).
homn is not a body; it is a brain + gate. They are **complementary**, not competitive.

## 3. The moat — policy-gated, audited hands

This is the whole reason to do it. Clicky can act (computer-use) but **you cannot constrain or
audit what it does** — there is no policy engine, no audit trail, no "ask before destructive."
homn **already built** that (the v1 policy engine: Rhai deny/ask/allow, wall-clock budgets,
hash-chained `DecisionReceipt`s, fail-closed). The integration is therefore:

> **Clicky = hands + voice-out. homn = brain + gate.** Every computer-use action Clicky would take
> (click/type/scroll/open) routes through homn's policy engine first → deny / ask / allow, with a
> `DecisionReceipt` written to the audit ledger. Every answer Clicky gives is grounded in homn's
> memory (recall/timeline/commitments). Screenpipe watches but can't act; minimi acts but you
> can't constrain it; **homn is the only ambient assistant whose hands are constitutionally gated
> and audited**, because that was Phase 1.

This is the trust story that makes always-on computer-use sellable rather than creepy — and it
is the exact thesis the v2 pivot doc named as differentiator #4.

## 4. The three wiring points

### 4.1 Memory → answers (grounding)
Clicky's Claude call is augmented with homn's recall context. Before sending screenshot +
transcript to Claude, homn injects "here's what I remember about this" — the top `recall(cue)`
hits for the transcript, the active commitments, the `whodis` for any named person — into the
prompt. ClickyX already speaks MCP, so this is a wiring task against homn's existing MCP tools
(`recall`, `timeline`, `commitments` once built), not new infrastructure. Clicky's flat per-app
journal is retired; homn's agidb store is the single memory.

### 4.2 Hands → gate (the differentiator)
Clicky's computer-use actions (`enigo` click/type/scroll, or Anthropic Computer Use `[POINT]`
→ coordinate) route through homn's policy engine **before execution** — the same gate that
already governs Claude Code tool calls. A new `cua` (computer-use action) policy scope is added
alongside the existing `tool`/`ingest` scopes: `allow_cua(action, app, target_text)` /
`deny` / `ask`. Every action emits a `DecisionReceipt` (the existing audit ledger). Destructive
actions (delete, send, purchase) default to `ask` (the same conservative-defaults principle as
FR-026). This is where old homn (the policy engine) and new homn (the body) literally become one
product.

### 4.3 Voice-out
homn has no TTS; Clicky has ElevenLabs / Edge / system TTS. That becomes homn's voice-out — the
"clone that whispers" needs it. Voice-in stays on **convox-voice** (local, persisted) — Clicky's
push-to-talk is for the conversation; the hotkeys are distinct (see §5.1).

## 5. De-duping the overlap (the real design calls)

### 5.1 Voice-in: keep convox-voice, Clicky is voice-out + conversation
- **convox-voice**: local faster-whisper, hold-to-talk, **persists every utterance to
  `dictation.jsonl`** (homn's speech sense). Keep as the memory feed.
- **Clicky**: push-to-talk for the *conversation* (transcript sent to Claude, not persisted as
  memory — that's convox-voice's job).
- **Hotkey**: distinct keys (e.g. convox-voice = RightAlt-hold; Clicky = Ctrl+Alt+Space) to
  avoid conflict. **Do not unify on Clicky's input** — convox-voice is local-first and persists;
  Clicky's STT (AssemblyAI/Whisper) is for the immediate query.

### 5.2 Screen: screenpipe continuous, Clicky on-demand
- **screenpipe**: change-driven continuous OCR (memory substrate). Keep.
- **Clicky**: on-demand screenshot for the current query / pointing. Keep.
- They coexist — screenpipe feeds memory, Clicky feeds the answer. No de-dupe needed; different
  cadences and purposes.

### 5.3 Retire `homn-face`
The parked `homn-face` Tauri spike (the cursor-buddy scaffold) is **superseded** — ClickyX is a
far more complete, proven body (overlay + pointing + computer-use + TTS + automation) on the
same stack. Merge the face-spike branch for its CI-job scaffolding, then delete the face crate;
the body is ClickyX-forked.

## 6. The fork

- Fork `unn-Known1/clickyX` into the homn org; rebrand the presence as the homunculus (homn's
  ◕ ◡ ◕), not Clicky's blue triangle.
- License: ClickyX is MIT; the fork retains MIT for the body code; the homn *integration* code
  (gate wiring, memory wiring) is homn's license.
- The fork is a **vendored dependency** under `crates/homn-body/` (or a sibling repo consumed as a
  path/git dep), same pattern as agidb/cloakpipe — not a workspace member, so the workspace still
  builds without it until `body-clicky` is enabled.

## 7. Phasing (this is Phase 5, deliberately after v1 traction)

The tech-plan already sequences Phase 5 after v1 ship + launch signal. This doc does **not** pull
it forward — the body is gated on (a) US5/US6 (commitments/beliefs/whodis) shipping, so the body
has something to *say*, and (b) real capture-week recall@3 confirming the brain is good enough to
ground answers. Adopting Clicky is the *mechanism* for Phase 5, not a reason to start it now.

Within Phase 5, the order:
1. Fork + rebrand ClickyX; get it building in the homn workspace behind `body-clicky`.
2. Wire memory-in (§4.1) — Clicky's answers pull from homn's recall/commitments.
3. Wire voice-out (§4.3) — Clicky's TTS as homn's voice.
4. Wire hands→gate (§4.2) — the differentiator; route computer-use through `homn-policy`; add
   the `cua` policy scope; default destructive actions to `ask`.
5. Retire `homn-face`.

## 8. Risks

| Risk | Mitigation |
|---|---|
| ClickyX is a large Rust+Tauri+React codebase; a fork drifts from upstream | Pin a commit; upstream-merge quarterly; keep our integration code isolated in `crates/homn-body/` adapters so rebases touch only the fork, not the wiring |
| "Tutor" UX framing vs homn's "memory/clone" framing | The cursor-buddy presence is the same shape either way; the prompt/UX rebrand is the cheap part — the runtime (presence + voice + pointing + computer-use) is the expensive part we're adopting |
| Computer-use through a policy gate adds latency to every action | The gate is sub-millisecond (Rhai, already benchmarked); `ask` is the only blocking path and only for destructive actions (default-off) |
| ClickyX upstream could go private (the original farzaa/clicky already did this for "new stuff") | We fork ClickyX (not farzaa/clicky); MIT grant is irrevocable; pin a commit |
| Two push-to-talk hotkeys confuse users | Document clearly; convox-voice = "dictate to memory", Clicky = "ask the homunculus" |

## 9. What this is NOT

- Not a replacement for screenpipe or convox-voice — those stay as L1 senses.
- Not a replacement for agidb — that stays as L3 brain.
- Not a reason to start Phase 5 before v1 — it's the *mechanism* for Phase 5, sequenced after
  US5/US6 and the real recall gate.
- Not a drop-in — it's a fork-and-adapt with three real wiring points.

## 10. Open questions (to resolve before Phase 5 starts)

1. **`cua` policy scope shape**: what context does an `enigo`/Computer-Use action carry that the
   Rhai rule matches on? (action kind, app, target element text/role, screen region). Needs a
   contract in `specs/003-cua/contracts/` when we start.
2. **Single binary or two?**: does ClickyX-fork ship as a second binary alongside `homn`, or do
   we merge the Tauri shell into the homn binary? (Likely: two binaries — `homn` the daemon, the
   fork the body — talking over the existing MCP/HTTP seam.)
3. **Wake word**: "hey homn" on convox-voice's stack (the tech-plan already names this) vs
   Clicky's wake-word — unify or keep distinct.
4. **Memory-in prompt budget**: how much recall context to inject before the screenshot+transcript
   overpowers homn's signal — tune against the eval harness.