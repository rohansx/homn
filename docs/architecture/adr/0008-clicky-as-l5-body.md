# ADR-0008 — Adopt Clicky (ClickyX) as homn's L5 Body

**Status**: Accepted
**Date**: 2026-07-23
**Supersedes**: the `homn-face` Tauri spike as the Phase 5 body (see [`docs/v2/tech-plan.md`](../../v2/tech-plan.md) §Phase 5)
**Companion**: [docs/v2/l5-body-clicky-adoption.md](../../v2/l5-body-clicky-adoption.md) (full integration spec)

## Context

homn is a memory + permission layer: capture (screenpipe + convox-voice), remember (agidb),
guard (Rhai policy + redaction + receipts), present via MCP. It has **no visible presence,
no voice-out, no hands** — those are Phase 5 ("the body"), parked until v1 traction. The parked
`homn-face` Tauri 2 spike was a minimal cursor-buddy scaffold; building the full body (overlay +
pointing + computer-use + TTS + automation) from scratch is weeks–months of work for zero
differentiation (presence is table stakes, not a moat).

[Clicky](https://github.com/farzaa/clicky) (MIT, by Farza Majeed) is an AI companion that lives
next to your cursor — sees your screen, hears you (push-to-talk), talks back (TTS), points at
and clicks UI elements (Computer Use). The cross-platform port **[ClickyX](https://github.com/unn-Known1/clickyX)**
is **Rust + Tauri + React, local-first, zero telemetry, MIT** — the same stack as our parked face
spike — with a `localhost:32123` HTTP bridge, an `enigo`-based computer-use engine, an automation
scheduler, and **MCP CRUD support already built in**.

Clicky is a *body* (presence + voice-out + hands); homn is a *brain + gate*. They are
complementary. The original farzaa/clicky has moved new development to closed-source, but the
MIT grant on the existing code is irrevocable, and ClickyX is an independent MIT cross-platform
fork we can pin.

## Decision

**Adopt ClickyX as homn's L5 Body** — fork it (MIT permits this) into the homn org, rebrand the
presence as the homunculus, and wire it to homn's brain and gate via three points:

1. **Memory → answers**: Clicky's Claude call is augmented with homn's `recall` / `timeline` /
   `commitments` context (ClickyX already speaks MCP). Clicky's flat per-app journal is retired;
   agidb is the single memory.
2. **Hands → gate** (the differentiator): every computer-use action (click/type/scroll/open)
   routes through homn's `homn-policy` engine *before execution* → deny/ask/allow + a
   `DecisionReceipt` in the audit ledger. A new `cua` policy scope is added; destructive actions
   default to `ask`.
3. **Voice-out**: Clicky's TTS (ElevenLabs/Edge/system) becomes homn's voice-out; voice-in stays
   on **convox-voice** (local, persisted). Hotkeys are distinct.

**Retire the `homn-face` spike** — ClickyX supersedes it (a far more complete, proven body on the
same stack).

## Consequences

**The moat**: homn becomes the only ambient assistant whose hands are **constitutionally gated
and audited** — Screenpipe watches but can't act; minimi/Clicky act but you can't constrain them;
homn's policy engine (Phase 1, built + tested) is the missing piece of every computer-use
assistant. Combined with a memory only homn has (agidb temporal/commitment/belief + unlearn-with-
receipt), this is the trust story that makes always-on computer-use sellable rather than creepy.
This is the v2 pivot doc's differentiator #4 made real.

**Cost**: ClickyX is a large Rust+Tauri+React codebase; integration is a fork-and-adapt, not a
drop-in. Mitigation: pin a commit, isolate the integration code in `crates/homn-body/` adapters so
upstream rebases touch the fork not the wiring, gate it behind a `body-clicky` feature so the
workspace still builds without it.

**Sequencing**: this is **Phase 5, deliberately after v1 traction** — not pulled forward. The body
is gated on US5/US6 (commitments/beliefs/whodis) shipping so the body has something to *say*, and
on the real capture-week recall@3 confirming the brain is good enough to ground answers. Adopting
Clicky is the *mechanism* for Phase 5, not a reason to start it now.

**Overlap resolved**: screenpipe (continuous capture → memory) and Clicky (on-demand screenshot
→ answer) coexist at different cadences; convox-voice stays as voice-in (local, persisted),
Clicky's push-to-talk is for the conversation; the `homn-face` crate is deleted (its CI-job
scaffolding merged first).

**Open questions** (in the companion spec, §10): the `cua` policy scope shape, single-binary vs
two, wake-word unification, and the memory-in prompt budget. These are resolved when Phase 5
starts, not by this ADR.