# Research: homn v2 — Ambient Memory (v1)

Phase 0 of the plan. Resolves the load-bearing unknowns and records the decisions. Format per decision: **Decision / Rationale / Alternatives rejected**. Source: `docs/v2/*` (verified audit 2026-07-17) + constitution.

---

## R1 — Brain: agidb, with a data-forced fallback

**Decision**: Adopt `agidb` as the temporal memory store, **conditional on the Phase 0 recall@3 gate**. The gate, not preference, picks the branch:

| recall@3 on the 30-Q real-week set | Consequence |
|---|---|
| ≥ 70% | agidb as-is; Phase 2b skipped |
| 40–70% | Phase 2b mandatory: fuse ctxgraph's benchmarked graph/retrieval tier into agidb as a parallel index (HDC signatures become *an* index, not the whole bet). One brain. Timeboxed 3 wks. |
| < 40% | ctxgraph becomes the primary store; agidb's belief/goal/unlearn types ported on top |

**Rationale**: agidb already has the right primitives (observe, episodic→semantic consolidation, beliefs+confidence+provenance, bi-temporal supersession, unlearn, MCP serve) and its core suite passes (15/15, verified 2026-07-17). The one real risk — HDC recall quality on noisy real-world OCR volume — is unknowable without data, so we buy the answer with a validation week before building on top. Either branch ends with **one brain**; the old "wire in ctxgraph as Phase 3" plan is retired.

**Alternatives rejected**: (a) Commit to agidb blindly — risks building the whole product on an unvalidated recall substrate. (b) Commit to ctxgraph — discards agidb's belief/goal/unlearn types that map 1:1 to the differentiators. (c) Build a new store — months of work, zero differentiation.

## R2 — Capture: consume Screenpipe, don't rebuild

**Decision**: `homnd` **tails** Screenpipe's local sqlite (event-driven screen OCR, a11y-tree text, audio, app metadata). A `Source` trait keeps it swappable. convox-voice dictation is a second, higher-fidelity push-based source.

**Rationale**: Rebuilding cross-platform capture is ~6 months of platform pain for zero differentiation. Screenpipe's MIT core is forkable if their YC roadmap diverges; the trait abstraction contains that risk. **Status: Screenpipe is not yet installed on the dogfood machine — installing it is Phase 0 task #1.**

**Alternatives rejected**: Own capture stack (cost, no differentiation); screen-scraping without a11y tree (worse text quality).

## R3 — Gate: CloakPipe in-process + Rhai policy (not Cedar)

**Decision**: Redaction is an **in-process library call** in the `homnd` pipeline (regex bank for secrets/keys/cards/Aadhaar/PAN + NER-based PII for third parties), not a network proxy hop. Per-app ingest policies are **Rhai rules** on the existing `homn-policy` engine. Redaction events go to a hash-chained evidence ledger (reuse CloakPipe's ledger code, persisted via `homn-audit`).

**Rationale**: homn already ships a tested Rhai engine (hot-reload, wall-clock budgets, rule trace, 142 green tests). One product must not carry two policy languages. In-process redaction is faster and keeps plaintext from ever crossing a socket. The ledger is a demo moment: "here is a cryptographic log of what was stripped and why."

**Alternatives rejected**: **Cedar** (the source plan's suggestion for CloakPipe consistency) — rejected for v1: adds a second policy language for no v1 benefit; revisit only if policy files are ever shared with CloakPipe proper. Network redaction proxy — plaintext crosses a boundary, violates Invariant 1's spirit and adds latency.

## R4 — Local/cloud split: local is cheap, cloud is the expensive part

**Decision**: Everything the invariants require to be local **is** cheap and stays local (ASR, GLiNER extraction, potion embeddings, recall math, redaction). Cloud is used **only at write time**, post-gate, per-policy, with the user's own key and a per-call disclosure receipt, for: commitment/belief extraction (Claude Haiku; qwen2.5:3b evaluated as a local swap-in later) and answer synthesis (Claude itself, as the MCP client — costs the product nothing).

**Rationale**: Honors Invariant 2 (no network in read path) and Invariant 4 (cloud sees only policy-allowed, post-gate content). Honors the 6 GB VRAM ceiling by *architecture*, not by squeezing a frontier model onto a laptop. The marketing claim is scoped honestly: *your memory never leaves; reasoning uses your own key, through the gate, with receipts.* This is compatible with agidb's own invariant ("LLMs may participate at write time — never at read").

**Alternatives rejected**: All-local extraction in v1 (3b-class quality on commitments unproven — make it a measured swap-in, not a launch blocker). All-cloud (violates the local story and read-path invariant). Always-on cloud inference (unit economics die at ~$0.25/action; also violates Invariant 2).

## R5 — Interface v1: MCP only, no GUI

**Decision**: `agidb serve` over MCP (extended with the seven tools) **is** the product. A tray icon + `homn pause`/`homn status` is the entire GUI budget. The Tauri `homn-face` crate stays parked until Phase 5.

**Rationale**: Distribution is "paste a connector link into Claude" — Claude is the body, no UI to build, two-minute setup (mirrors minimi's onboarding). Matches Constitution V (conservative defaults) and keeps v1 scope tight.

**Alternatives rejected**: Ship a desktop app in v1 (scope creep, delays the wedge); CLI-only recall (loses the "Claude is the body" distribution advantage).

## R6 — Dedupe strategy: content-hash + shingle overlap

**Decision**: Two-stage dedupe before storage — exact content-hash (blake3/xxh3) to drop identical frames, then shingle-overlap similarity to collapse near-duplicate OCR frames within an app-focus block.

**Rationale**: Screen text repeats constantly; the plan states dedupe is "where 80% of the noise dies." Exact-hash is a cheap pre-filter; shingle overlap catches the scroll/cursor-jitter near-dupes that exact hashing misses. Directly serves SC-009 and keeps disk growth (a named risk) bounded.

**Alternatives rejected**: Embedding-similarity dedupe (too expensive per-frame at ingest volume); exact-hash only (misses near-dupes, store fills with confetti).

## R7 — Crash-safe ingestion: watermark + fail-closed gate

**Decision**: `homnd` persists a per-source crash-safe watermark (last-consumed upstream row id / cursor) in SQLite; on restart it resumes from the watermark with dedupe as the idempotency backstop. The gate **fails closed**: if redaction or policy evaluation errors for an item, that item is not persisted.

**Rationale**: Invariant 1 (nothing unredacted touches disk) forbids fail-open. Watermark + dedupe gives at-least-once upstream consumption with exactly-once effective storage. Backpressure (bounded channels) prevents capture from outrunning ingest and exhausting memory.

**Alternatives rejected**: In-memory-only cursor (loses position on crash → gaps or re-ingest); fail-open on redaction error (violates Invariant 1).

## R8 — Sessionization: app-focus + meeting heuristics

**Decision**: A sessionizer assigns `SessionId` from app-focus blocks and meeting-app heuristics (Zoom/Meet window / mic-active). Session boundaries are first-class so consolidation mints episode-level memories ("the July 14 call with X") instead of confetti. The boundary mechanism is **source-agnostic** so a Phase 3.5 connector can map a mail thread / channel-day to a session with the same machinery.

**Rationale**: Episode-level memories are what make `timeline`, `whodis`, and `today`/`standup` coherent rather than fragment dumps. Source-agnostic boundaries are the cheap forward-compat for connectors.

**Alternatives rejected**: Fixed time-window sessions (splits a single meeting arbitrarily); no sessions (consolidation degrades to per-fragment).

## R9 — `Source` trait must accommodate poll-cursor sources (Phase 3.5 forward-compat)

**Decision**: The `Source` abstraction is defined around an opaque, serializable **cursor** and a "fetch items since cursor" step, so both a sqlite-tail source (cursor = last row id) and a poll-based OAuth source (cursor = Gmail history id / Slack `oldest` ts / GitHub events cursor) implement the *same* trait. v1 ships the trait + `ScreenpipeTail` + `DictationPipe`; the poll-cursor shape is exercised by a test/scaffold but connectors themselves are Phase 3.5.

**Rationale**: FR-005a. Email/Slack are the highest-signal-per-byte sources (commitments in explicit text, not OCR soup) and strengthen `commitments()`/`whodis()` before Phase 4 depends on them. Baking the cursor shape in now avoids a breaking trait change later. `Observation.source` already reserves `Email | Slack | GitHub` variants.

**Alternatives rejected**: Sqlite-tail-only trait (forces a breaking change to add connectors); building connectors now (out of v1 scope, no launch signal yet).

## R10 — Write-time extraction quality: start narrow, expand from eval failures

**Decision**: Commitment/belief extraction starts narrow — explicit promise phrases + calendar/email signals — and expands driven by Phase 0 eval failures. Cloud (Haiku) vs. local (qwen2.5:3b) is an A/B on the eval set; local swaps in only if quality holds. Extraction is strictly write-time.

**Rationale**: Named risk ("commitment/belief extraction quality"). Narrow-and-expand keeps precision high early; the eval set turns "quality" into a measurable, regressable number. Keeping extraction write-time preserves agidb's read invariant and Invariant 2.

**Alternatives rejected**: Broad open-ended extraction from day one (low precision, poisons `commitments()`); read-time extraction (violates invariants, adds latency + network to reads).

---

## Consolidated unknowns → all resolved

| Unknown (from Technical Context) | Resolved by |
|---|---|
| Which brain / store | R1 (data-forced by Phase 0) |
| How capture arrives | R2 |
| Redaction placement + policy language | R3 |
| Where cloud is allowed | R4 |
| v1 interface | R5 |
| Dedupe approach | R6 |
| Crash safety + fail mode | R7 |
| Session boundaries | R8 |
| Source trait shape (connector forward-compat) | R9 |
| Extraction quality strategy | R10 |

No `NEEDS CLARIFICATION` markers remain.

---

## R1 outcome — Phase 0 gate run, 2026-07-18 (PRELIMINARY; verdict DEFERRED)

**Run**: `homn eval ingest ~/.screenpipe/db.sqlite` → 194 rows, 44 chunks; `homn eval run
eval/questions/2026-07-18.toml` → **recall@1 70.0%, recall@3 76.7%** (factual 90%, temporal 70%,
commitment 70%). Full results + reproduce steps: [`eval/results/2026-07-18.md`](../../eval/results/2026-07-18.md).

**Mechanical verdict**: recall@3 ≥ 70% → the threshold table says `agidb_as_is` (skip Phase 2b).

**Actual verdict: DEFERRED.** This number is a **pipeline validation on real capture, not a
brain-architecture decision.** The sample is too thin and too non-representative:

1. **~45 minutes of capture** (13:39–14:24 UTC, 2026-07-18). The plan calls for 5–7 working days.
2. **No real work activity on screen** — all 12 OCR frames are an identical static Konsole "File"
   menu. The user was not producing varied screen content during this window.
3. **The audio is background media, not the user's speech** — 168 mic transcriptions of a YouTube
   video playing through speakers. Zero mentions of the actual work.
4. The questions are grounded in the captured content, so they're answerable by substring/keyword
   recall — which agidb handles trivially. 76.7% on distinctive-phrase retrieval says little about
   recall over a real week of mixed, noisy, multi-app activity.

A real decision requires a real capture week. The number crossing 70% here is because the task is
easy, not because the brain is proven.

### Blockers found + fixes applied this run

1. **Ambient mic audio is background-media noise.** The mic transcribes whatever plays through the
   speakers, not intentional speech. **Fix applied**: the screenpipe systemd unit now runs with
   `--disable-audio`; new capture adds no audio noise. The historical 168 noise rows remain in the
   DB (fixed, non-growing) and can be purged with `DELETE FROM audio_transcriptions` for a clean
   baseline. **Product finding**: `ambient_audio` is not a viable speech sense without VAD/filtering
   or speaker separation; **push-to-talk dictation (convox-voice) is the speech sense** — exactly
   the L1 design (convox-voice = dictation; ambient_audio = a separate, harder, later sense).
2. **convox-voice did not persist transcriptions.** Only 4 old log lines carried `text="..."`; the
   current faster_whisper backend emitted none, so the DictationPipe / `eval ingest` had no speech
   feed. **Fix applied**: convox-voice now appends each final transcription to
   `~/.local/share/convox-voice/dictation.jsonl` (projx/convox-voice `e073554`), and `homn eval
   ingest` reads it as the `dictation` source alongside screenpipe OCR.
3. **The capture week has not actually started.** ~45 min of a static menu is not a working week.

### Re-run procedure (the condition for a valid R1 decision)

1. Work at the machine for 5–7 days with screenpipe (screen OCR, audio OFF) + convox-voice
   (push-to-talk dictation) running. Dictate intentionally — commands, summaries, commitments.
2. Optionally purge the historical audio noise: `sqlite3 ~/.screenpipe/db.sqlite "DELETE FROM
   audio_transcriptions;"` (audio capture is disabled, so this is safe).
3. Author a fresh `eval/questions/<date>.toml` from the real week — 10 factual, 10 temporal, 10
   commitment, grounded in what actually happened (real apps, real dictation, real people).
4. `homn eval ingest ~/.screenpipe/db.sqlite --brain eval/results/<date>.brain.agidb` then
   `homn eval run eval/questions/<date>.toml --brain eval/results/<date>.brain.agidb`.
5. Record the recall@3 here; the threshold table then decides the branch for real.

**Until that run, the brain branch remains CONDITIONAL** (as R1 always specified). No Phase 2b
work starts; no ctxgraph swap starts. The gate machinery is built, proven on real capture, and
ready — the only missing input is a real week of data.
