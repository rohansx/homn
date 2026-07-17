# homn v2 — tech plan & phases

v0.2 · 2026-07-17 · companion to [`product-overview.md`](./product-overview.md) and [`architecture.md`](./architecture.md)

Total to shippable v1 (Phase 3): **~5–7 weeks**, of which week one is mostly passive data collection. Phases 4–5 (v2) begin only after launch signal.

---

## Asset audit (verified 2026-07-17)

Every dependency was checked on the dogfood machine before this plan was written:

| Asset | Role | Verified state |
|---|---|---|
| convox-voice | L1 dictation ASR | ✅ running now — systemd user service, active development through Jul 14 |
| agidb | L3 brain | ✅ core tests 15/15 pass; MCP server binary builds; active through Jul 8 |
| cloakpipe | L2 redaction | ✅ real Rust crate (MIT), evidence-ledger code exists, live as hosted service |
| primd | L4 reflexes (v2) | ✅ exists, v0.4.1-era, HTTP API + cold tier; not needed until Phase 4 |
| ctxgraph | brain fallback | ✅ exists, benchmarked (CoNLL04 leaderboard), last touched Jun 24 |
| homn v1 | L2 policy + chassis | ✅ 142 workspace tests green; installer, service units, MCP crate all shipped |
| **screenpipe** | L1 screen capture | ❌ **not installed** — first task of Phase 0 |
| Ollama models | local utility | qwen2.5 {1.5b, 3b, 7b} installed; 2× Gemma-12B Q4 installed but unusable for this product (7.4 GB vs 6 GB VRAM) — remove or ignore |

Known host quirk: Homebrew's `pkg-config` shadows the system one and hides `/usr/lib/pkgconfig`; export `PKG_CONFIG_PATH=/usr/lib/pkgconfig:/usr/share/pkgconfig` before any GTK/webkit-linked build (bit us on `homn-face`).

---

## Phase 0 — validation week (do this before everything)

**Goal: answer "does agidb's recall survive real life?" with data, not vibes.**

1. Install Screenpipe; run `screenpipe record` + convox-voice for 5–7 normal working days.
2. Write a throwaway `ingest.rs`: tail Screenpipe sqlite → naive chunking → `agidb observe`. No redaction yet — own data only, nothing leaves the machine.
3. Build a 30-question eval set from *the actual week*: 10 factual ("who sent the screenshot about the bug"), 10 temporal ("what did I decide about X on Tuesday"), 10 commitment/belief ("what did I promise Chris", "how did my pitch framing change").
4. Score recall@1 and recall@3 by hand. Also measure: observations/day, disk growth, ingest CPU, GLiNER precision on OCR junk (sample 100 extractions, count garbage).

**Gate:**

| recall@3 | Consequence |
|---|---|
| ≥70% | proceed with agidb as-is |
| 40–70% | Phase 2b (ctxgraph retrieval merge) becomes mandatory, before Phase 3 |
| <40% | ctxgraph becomes the store; agidb's belief/goal/unlearn types ported on top |

*Est: 1 week (mostly passive), ~2 days of code.*

## Phase 1 — homnd, the ingestion spine

Built on the v1 `homn-daemon` chassis (Tokio runtime, unix sockets, event bus, supervisor patterns — already tested).

- `Source` trait; impls for `ScreenpipeTail` (poll sqlite watermark) and `DictationPipe` (unix socket/stdin from convox-voice).
- Chunking: coalesce OCR frames per app-focus block; sentence-split audio; dedupe via content-hash + shingle overlap. Screen text repeats *constantly* — **dedupe is where 80% of the noise dies.**
- Sessionizer: app-focus + meeting-app heuristics (Zoom/Meet/mic-active) → `SessionId`.
- Backpressure + crash-safe watermarks; `homn pause` / `homn status` CLI.

Tests-first applies (this is pipeline correctness code): watermark recovery, dedupe, sessionizer boundaries.

*Est: 1–1.5 weeks (chassis reuse is the discount vs. the original 1.5-week estimate from scratch).*

## Phase 2 — the gate

- CloakPipe redaction as an in-process stage: secrets regex bank (keys, tokens, cards, aadhaar/PAN), NER-based PII for third parties.
- Per-app ingest policies as **Rhai rules** evaluated by the v1 `homn-policy` engine (`never ingest: password managers, banking tabs, incognito`). One policy language across the product — Cedar rejected for v1 (see [architecture §3](./architecture.md#gate-cloakpipe-in-process--rhai-policy-not-cedar)).
- Deny-list UX: config file + `homn exclude <app|domain>`.
- Hash-chained redaction ledger — reuse CloakPipe evidence-ledger code, persisted via the v1 `homn-audit` crate.

Tests-first applies (Constitution VI: this is policy/audit territory).

*Est: 1 week (mostly reuse).*

## Phase 2b — brain merge (only if Phase 0 says so)

- Port ctxgraph's retrieval tier into agidb as a parallel index; recall fuses HDC-tier + graph-tier scores.
- The Phase 0 eval set becomes the regression suite for the merge.

*Est: 2–3 weeks. The riskiest line in the plan — timebox it hard.*

## Phase 3 — MCP v1 (the shippable product)

- Extend `agidb serve` with the seven-tool query surface ([architecture §6](./architecture.md#6-query-surface-mcp-tools-v1)).
- Commitment/belief extraction happens at **write time** via cloud (Claude Haiku, user's key, post-redaction, receipted) — never in the read path, preserving agidb's core invariant. Benchmark qwen2.5:3b against Haiku on the Phase 0 eval; swap in local if quality holds.
- Streamable HTTP MCP + connector-link flow, mirroring minimi's "paste into Claude" onboarding; tray icon (or plain systemd service + CLI) with pause/status.
- One-command install: `curl | sh` → installs screenpipe if absent, homnd, agidb, prints the MCP link. (Reuses the v1 installer + checksum verification.)
- **README rewrite** — "homn is the local human: memory, permissions, presence." Retire the two-meanings problem decisively.
- macOS decision point: dogfood is Linux; minimi's audience is Mac. Decide port scope from v1 interest, not upfront. (convox-voice is Linux-only — Mac ambient audio would lean on Screenpipe's whisper alone initially.)
- **Demo script:** live-ask Claude the seven queries about the real week, then run `homn forget "<test entity>"` and show the audit receipt. That's the launch video. HN/X: *"minimi promised local. here's local."*

*Est: 2 weeks. → Ship.*

## Phase 3.5 — account connectors (post-launch, pre-proactive-loop)

The OpenHuman-inspired layer, and the highest-signal-per-byte source in the design: email and Slack are where commitments live in **explicit text** — clean prose, real timestamps, real sender identity — versus OCR soup from the screen. Cheap to build (the `Source` trait and `Observation.source` variants `Email | Slack` already accommodate it) and it directly strengthens `commitments()` and `whodis()` before Phase 4's whisper loop depends on them.

- `GmailSource`, `SlackSource`, `GitHubSource` impls of the Phase 1 `Source` trait — poll-based, read-only scopes, incremental cursors (history id / `oldest` ts / events API).
- OAuth tokens in the system keyring (same pattern convox-voice already uses); no homn cloud component — each connector talks to the provider directly from the daemon.
- Everything passes the same gate: third-party PII policy applies to senders/recipients; per-account exclusion (`homn exclude gmail:work@…`) rides the existing deny-list UX.
- Sessionizer treats a thread / channel-day as the session boundary, so consolidation mints "the pricing thread with Chris" as an episode.

*Est: 1–2 weeks (one connector ≈ 3 days once the first lands). Order by eval failures from Phase 3 — build the connector whose absence loses the most `commitments()` questions first.*

## Phase 4 — proactive loop (v2 begins; only after launch signal)

- Meeting detector → live transcript stream (convox-voice ASR) → per-utterance: primd predictive retrieval against agidb → local triage model scores "is there a suggestion worth making?" → above threshold, one Claude call composes the whisper.
- Triage model: **qwen2.5:3b** (the 6 GB card must also hold whisper; 12B-class is out — see [architecture §9](./architecture.md#9-reference-hardware-dogfood-machine)).
- Latency budget: ASR partials <300 ms → retrieval <100 ms (primd) → triage <400 ms local → suggestion in ~2–4 s total. Retrieval pre-warmed per attendee (whodis dossiers loaded at meeting start).
- Surface: minimal overlay first (native panel), not the full buddy.

*Est: 3–4 weeks.*

## Phase 5 — body

- Un-park `homn-face` (the Tauri 2 spike scaffold already builds); fork clicky's MIT skeleton for cursor presence + pointing; local TTS for voice-out; "hey homn" wake on the existing dictation stack.
- **Proactive interruption policy is `homn-policy`'s territory** — this is where old homn and new homn become one product: the same Rhai engine that gated Claude Code's `rm -rf` now governs what the clone may say, do, and interrupt, with the same audit trail.

*Est: 3+ weeks, deliberately after v1 traction data.*

---

## What happens to the v1 roadmap

- `specs/001-policy-engine/` — complete; stays as the record of Phase 1 v1. (Note: its tasks.md checkboxes were never ticked; the git log and the 142 green tests are the source of truth.)
- `docs/phases/phase-2-face.md` (face launch) and `phase-3-brain.md` (ctxgraph brain) — **superseded by this plan.** The face returns in Phase 5; agidb replaces ctxgraph as the brain.
- `docs/phases/milestones.md` calendar — void; this plan's estimates replace it.
- The `homn-face` spike branch (`feat/phase-2-face-spike`) — merge or park as-is; its scaffold is Phase 5 raw material and its CI job already guards the build.

## Risks & open questions

| Risk | Mitigation |
|---|---|
| HDC recall degrades at real-world scale/noise | Phase 0 gate before any building; ctxgraph fallback path defined |
| Two-brains split (agidb vs ctxgraph) burns months | Decision forced by Phase 0 data; merge timeboxed at 3 weeks |
| Screenpipe dependency shifts (YC company, their roadmap) | `Source` trait abstraction; MIT core is forkable |
| Commitment/belief extraction quality | Start narrow: explicit promise phrases + calendar/email signals; expand from eval failures; cloud/local A-B on the eval set |
| Disk growth (OCR text is voluminous) | Dedupe + consolidation decay; measured in Phase 0 |
| Battery/CPU on laptop | Event-driven capture only; ingest batched; consolidation on AC power |
| Cloud-at-write-time undercuts the "local" story | Scope the claim honestly (memory local, reasoning your-key-through-gate); receipts make it verifiable; local swap-in path defined |
| Focus cost | v1 scope ends at Phase 3; Phases 4–5 gated on launch signal |

## Definition of done (v1)

- 7 days ingested, <5% average CPU, redaction ledger populated.
- Claude answers the seven query types about the real week, with receipts.
- `homn forget` end-to-end with audit receipt.
- Install-to-first-answer under 5 minutes on a clean machine.
