# Feature Specification: homn v2 — Ambient Memory (local human, v1 product)

**Feature Branch**: `claude/v2-docs-implementation-m8eixq`

**Created**: 2026-07-17

**Status**: Draft

**Input**: User description: "homn v2 — the local human: a local-first ambient memory + clone daemon. Ingest the user's digital life continuously (screen OCR/a11y tree via Screenpipe, ambient + dictation audio via convox-voice), pass it through a privacy gate (CloakPipe redaction + Rhai per-app ingest policies + hash-chained audit receipts), build a bi-temporal model of entities/commitments/beliefs in agidb, and expose it to Claude via an MCP server with seven query tools (recall, timeline, commitments, beliefs, whodis, today/standup, forget). Absorb the shipped v1 policy engine, audit crate, and daemon chassis as the governance layer. Scope this spec to the shippable v1 product (Phases 0–3 in docs/v2/tech-plan.md)."

> **Source of truth**: [`docs/v2/README.md`](../../docs/v2/README.md), [`product-overview.md`](../../docs/v2/product-overview.md), [`architecture.md`](../../docs/v2/architecture.md), [`tech-plan.md`](../../docs/v2/tech-plan.md). This spec covers **Phases 0–3 only** (the shippable v1). The proactive meeting copilot (Phase 4) and cursor-buddy body (Phase 5) are out of scope here and get their own spec once launch signal justifies them.
>
> **Forward compatibility with Phase 3.5 (account connectors).** The immediate next phase after the v1 ship is Gmail/Slack/GitHub connectors — the highest-signal-per-byte sources, where commitments live in explicit text rather than OCR soup. Building them is *not* in scope for this spec, but the `Source` abstraction and observation data model defined here **must anticipate poll-based cursor sources** (incremental history-id / `oldest`-ts / events cursors), not only sqlite-tail sources. See FR-005a and the Observation/Session entity notes.

## Scope note: the five invariants

These bind every requirement below. They are the product, not decoration:

1. **Nothing unredacted touches disk.** The gate precedes the store, always.
2. **No network calls in the read path.** Recall is local math.
3. **Every memory has provenance; every deletion has a receipt.**
4. **Cloud models see only what a policy explicitly allows past the gate, per-call.**
5. **One command to pause everything; one command to destroy everything.**

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Prove recall survives real life (Priority: P1)

Before anything is built on top of the memory store, the maintainer runs continuous capture on their own machine for 5–7 normal working days, ingests it into the memory store with naive chunking (no redaction — own data only, nothing leaves the machine), and hand-scores a 30-question evaluation set drawn from that actual week: 10 factual, 10 temporal, 10 commitment/belief questions. The measured recall@3 decides the memory-store architecture for every later story.

**Why this priority**: This is the validation gate that de-risks the entire product. The whole thesis rests on "does the temporal memory store answer real questions?" — and that must be answered with data, not vibes, before the ingestion spine, the gate, or the query surface are worth building. A bad number here changes the architecture (see acceptance scenarios), so it must come first.

**Independent Test**: Run capture for a week, ingest, score the 30-question set by hand, and read off recall@1 / recall@3 plus the operational metrics (observations/day, disk growth, ingest CPU, extraction precision on OCR noise). The story is "done" when a number exists and the architecture branch is chosen.

**Acceptance Scenarios**:

1. **Given** a week of the maintainer's own captured activity ingested into the memory store, **When** the 30-question eval set is scored by hand, **Then** recall@1 and recall@3 are recorded, along with observations/day, disk growth, average ingest CPU, and extraction precision sampled over 100 extractions.
2. **Given** a measured recall@3 of **≥70%**, **When** the gate is evaluated, **Then** the existing memory store is adopted as-is and later stories proceed unchanged.
3. **Given** a measured recall@3 between **40% and 70%**, **When** the gate is evaluated, **Then** a retrieval-tier merge (a benchmarked graph/retrieval index fused alongside the existing signature index) becomes a mandatory prerequisite before the query-surface story ships.
4. **Given** a measured recall@3 **<40%**, **When** the gate is evaluated, **Then** the benchmarked retrieval store becomes the primary store and the belief/goal/unlearn types are ported on top of it.
5. **Given** the eval set exists, **When** the query surface is later built, **Then** the same 30-question set is retained as an automated regression suite.

---

### User Story 2 - Recall my week through Claude (Priority: P1)

The user installs homn, lets it run, and after it has captured activity, asks Claude (via the MCP connector) questions about their real day/week — "who sent the screenshot about the login bug", "what did I work on Tuesday afternoon", "what happened in the call with the design vendor" — and gets grounded answers with provenance, produced entirely from local data with no network call in the read path.

**Why this priority**: This is the wedge and the launch demo — "minimi, but honest about locality, with a real brain." It is the smallest end-to-end slice (capture → gate → store → MCP recall) that delivers the core value. Without it there is no product.

**Independent Test**: With capture running and the connector linked into Claude, ask the `recall` and `timeline` questions against a real captured session and confirm the answers are grounded in actual observations and cite provenance. Verify no outbound network request occurs while answering.

**Acceptance Scenarios**:

1. **Given** homn has ingested a working session, **When** the user asks Claude a cue-based question ("who sent the screenshot about the bug"), **Then** Claude, via the `recall` tool, returns an answer grounded in stored observations, each carrying provenance (source, app, time) and a confidence signal.
2. **Given** ingested activity spanning a time range, **When** the user asks "what did I work on Tuesday afternoon" (via the `timeline` tool for an entity/topic and time window), **Then** the ordered sequence of what happened is returned.
3. **Given** any read-path query, **When** it is served, **Then** no network request leaves the machine to answer it (recall is local math).
4. **Given** the connector link, **When** the user pastes it into Claude, **Then** setup to first grounded answer completes in under 5 minutes on a clean machine.

---

### User Story 3 - The gate keeps sensitive data off disk (Priority: P1)

Everything captured passes through an in-process privacy gate **before** it is persisted. Secrets (API keys, tokens, card numbers, government IDs such as Aadhaar/PAN) and third-party personal information are redacted; per-app rules keep whole categories out (password managers, banking tabs, incognito windows). Every redaction and every ingest decision is recorded in a tamper-evident ledger that stores what was stripped and why — never the stripped plaintext.

**Why this priority**: Invariant 1 makes this non-optional for any persisted data. It is also differentiation #3 ("privacy as architecture") and the thing that separates homn from raw-capture competitors. Because it sits *before* the store, it must ship alongside the store, not after.

**Independent Test**: Feed captured content containing known secrets and third-party PII plus content from an excluded app; confirm the persisted store contains only redacted text, the excluded app produced no stored observation, and the ledger has an entry (type, span reference, policy id) for each redaction with no plaintext present.

**Acceptance Scenarios**:

1. **Given** captured text containing an API key and a card number, **When** it passes the gate, **Then** the persisted observation contains redaction placeholders (not the secret), and the ledger records each redaction's type, a span reference, and the policy id — with no plaintext.
2. **Given** a per-app deny rule for a password manager (or `homn exclude <app|domain>`), **When** content from that app/domain is captured, **Then** no observation from it is ever persisted, and the skip is recorded.
3. **Given** the redaction ledger, **When** its entries are verified, **Then** the hash chain is intact (tamper-evident), demonstrating a cryptographic log of what was stripped and why.
4. **Given** a per-app ingest policy file, **When** the user edits it, **Then** the change takes effect without restarting capture (hot-reload), consistent with the existing policy engine's behavior.

---

### User Story 4 - Forget on demand, with a receipt (Priority: P1)

The user tells homn to forget everything about a person, a time range, or a matching pattern. The memory is removed from recall, and an audit receipt is produced that proves what was forgotten and when.

**Why this priority**: Invariant 3 ("every deletion has a receipt") and differentiation #3. "Forget everything about this person" and showing the receipt is half of the launch demo. It is also a hard trust requirement — a memory product no one can prune is a liability.

**Independent Test**: Ingest data about a test entity, run `homn forget "<entity>"`, confirm subsequent recall no longer surfaces it, and confirm an audit receipt exists describing the scope and time of the deletion.

**Acceptance Scenarios**:

1. **Given** stored memories referencing a person, **When** the user runs `forget(entity)` (via CLI or the MCP tool), **Then** those memories no longer surface in recall and an audit receipt records the scope, match count, and timestamp.
2. **Given** a `forget` by time range or by pattern, **When** it completes, **Then** the same receipting applies (scope described, receipt produced).
3. **Given** a completed forget, **When** the receipt is inspected, **Then** it is sufficient to prove what was removed without re-exposing the forgotten content.

---

### User Story 5 - Commitments and beliefs over time (Priority: P2)

The user asks Claude about promises and positions: "what did I promise Chris by Friday", "what are my open commitments", "how has my framing of the pitch changed since March". homn answers from a structured temporal model — commitments (mine and others', with status and due dates) and beliefs (current position plus revision history) — not from similarity search.

**Why this priority**: This is differentiation #2 (temporal memory the vector-only competitors cannot match) and the second half of the demo's "wow." It depends on the recall slice (US2) being in place and on write-time extraction, so it follows P1.

**Independent Test**: Ingest a session containing an explicit promise and a changed opinion; ask the `commitments` and `beliefs` tools and confirm the promise appears with status/due metadata and the belief shows a revision history.

**Acceptance Scenarios**:

1. **Given** ingested content containing an explicit promise ("I'll send the quote by Friday"), **When** commitment extraction runs at write time, **Then** a commitment is recorded with owner, counterpart, due date, and status, queryable via `commitments(status?, due_before?)`.
2. **Given** ingested content where the user's stated position on a topic changes over time, **When** `beliefs(topic)` is queried, **Then** the current position is returned along with its revision history.
3. **Given** write-time extraction uses a cloud model, **When** it runs, **Then** it runs **only after** the gate (post-redaction), only on content a policy allows, uses the user's own API key, and produces a disclosure receipt — and it never runs in the read path.

---

### User Story 6 - Relationship dossiers and a daily recap (Priority: P2)

The user asks "who is Priya and where do we stand" and gets a relationship dossier — every interaction, the last thread, open loops — and asks "what did I actually do today" for a standup-style recap.

**Why this priority**: `whodis` and `today`/`standup` round out the seven-tool surface and map directly to the competitor's marketing questions, but they are compositional over the same store and extraction as US2/US5, so they come after the core recall and temporal-model slices.

**Independent Test**: After ingesting interactions with a named person across several sessions, call `whodis(name)` and confirm it aggregates interactions, surfaces the last thread and open loops; call `today()`/`standup()` and confirm a recap of the day's activity.

**Acceptance Scenarios**:

1. **Given** multiple interactions with a named person over time, **When** `whodis(name)` is queried, **Then** a dossier is returned aggregating interactions with the last thread and any open loops (e.g., unanswered follow-ups).
2. **Given** a day of ingested activity, **When** `today()` / `standup()` is queried, **Then** a recap of what the user did that day is returned.

---

### User Story 7 - Pause everything, destroy everything (Priority: P2)

The user can stop all capture with a single command and can see current status; and can destroy all captured data with a single command. Capture is conservative by default — sensitive surfaces are off until explicitly enabled.

**Why this priority**: Invariant 5 and Constitution V (conservative defaults, loud opt-ins). It is small but non-negotiable for trust; grouped at P2 because the core value slices must exist for pause/destroy to be meaningful, but it must ship within v1.

**Independent Test**: Run `homn pause` and confirm capture halts and `homn status` reflects it; resume and confirm capture continues; run the destroy command and confirm all captured data is gone.

**Acceptance Scenarios**:

1. **Given** capture is running, **When** the user runs `homn pause`, **Then** all capture halts and `homn status` reports the paused state; resuming restores capture.
2. **Given** captured data exists, **When** the user runs the destroy command, **Then** all captured memory and derived data are removed from the machine.
3. **Given** a fresh install, **When** homn starts, **Then** sensitive capture surfaces are off by default and require an explicit opt-in.

---

### Edge Cases

- **Screen text repeats constantly.** OCR frames within an app-focus block are near-duplicates; without dedupe the store fills with confetti. Dedupe (content-hash + shingle overlap) must collapse repeats — the plan calls out that this is where ~80% of the noise dies.
- **Capture source restarts or the daemon crashes mid-stream.** Ingestion must resume from a crash-safe watermark without re-ingesting or dropping rows.
- **Extraction garbage from OCR noise.** Entity/relation extraction over OCR junk produces false entities; precision is measured in US1 and low-quality extractions must not poison recall.
- **Session boundaries.** Meeting start/stop and app-focus blocks must be detected so consolidation mints episode-level memories ("the July 14 call with X") rather than isolated fragments.
- **A redaction policy or the gate itself fails.** If the gate cannot run, capture must fail closed (nothing persists) rather than fail open.
- **Cloud extraction is unavailable (offline, no API key, rate-limited).** Read-path recall must remain fully functional; only write-time enrichment degrades, and its absence must not block ingestion or recall.
- **`forget` targets an entity that also appears in shared/episodic memories.** Deletion scope must be well-defined and the receipt must reflect exactly what was removed.
- **Backpressure.** If capture outruns ingestion, the pipeline must apply backpressure rather than drop data silently or exhaust memory.

## Requirements *(mandatory)*

### Functional Requirements

**Validation gate (US1)**

- **FR-001**: The system MUST provide a reproducible way to ingest a multi-day capture of the maintainer's own activity into the memory store with naive chunking and no redaction (own-data-only), for evaluation purposes.
- **FR-002**: The system MUST support scoring a 30-question evaluation set (10 factual, 10 temporal, 10 commitment/belief) against the ingested week, recording recall@1 and recall@3.
- **FR-003**: The evaluation MUST also capture operational metrics: observations/day, disk growth, average ingest CPU, and extraction precision sampled over ≥100 extractions.
- **FR-004**: The recall@3 result MUST drive the documented architecture branch (≥70% as-is / 40–70% retrieval-merge / <40% store-swap), and the eval set MUST be retained as an automated regression suite.

**Ingestion spine (US2 foundation)**

- **FR-005**: The system MUST ingest activity continuously from multiple capture sources behind a common source abstraction, including screen capture (OCR + accessibility-tree text + app metadata) and audio (ambient + dictation).
- **FR-005a**: The common source abstraction MUST accommodate **poll-based cursor sources** (a source that advances an opaque incremental cursor — e.g., a history id, an `oldest` timestamp, or an events cursor — and returns new items since that cursor), not only tail-a-local-store sources. This is a forward-compatibility requirement for the Phase 3.5 account connectors (Gmail/Slack/GitHub); the v1 deliverable is the abstraction plus the screen and audio sources, but the trait contract MUST NOT hard-code a sqlite-tail shape that would need breaking changes to add a read-only OAuth poll source later.
- **FR-006**: The system MUST normalize captured input into a single observation unit carrying at minimum: source kind, app, capture time, post-gate text, redaction references, optional session/speaker, and a content hash.
- **FR-007**: The system MUST dedupe near-duplicate captured content (content-hash + shingle overlap) before storage.
- **FR-008**: The system MUST coalesce/chunk raw capture into coherent observations (OCR frames per app-focus block; sentence-split audio).
- **FR-009**: The system MUST assign session boundaries (app-focus blocks, meeting-app heuristics) so memories can be consolidated at episode level.
- **FR-010**: The system MUST use crash-safe watermarks so ingestion resumes without loss or duplication after a restart, and MUST apply backpressure when capture outruns ingestion.

**The gate (US3)**

- **FR-011**: The system MUST redact secrets (keys, tokens, cards, government IDs such as Aadhaar/PAN) and third-party personal information from captured content **before** it is persisted; nothing unredacted may reach disk (Invariant 1).
- **FR-012**: If the gate cannot run for a given item, the system MUST fail closed (do not persist) rather than persist unredacted content.
- **FR-013**: The system MUST evaluate per-app / per-domain ingest policies (e.g., never ingest password managers, banking tabs, incognito) via the existing deterministic rule engine, with hot-reload.
- **FR-014**: Users MUST be able to exclude an app or domain from capture via a command (`homn exclude <app|domain>`) and via an editable policy/config file.
- **FR-015**: The system MUST record every redaction and every ingest decision in a hash-chained, tamper-evident ledger that stores redaction type, a span reference, and policy id — and never the redacted plaintext.

**Query surface (US2, US5, US6)**

- **FR-016**: The system MUST expose its memory to an MCP client (Claude) via a connector, with an onboarding flow that is paste-a-link simple.
- **FR-017**: The system MUST provide the seven query tools: `recall(cue, as_of?)`, `timeline(entity|topic, from, to)`, `commitments(status?, due_before?)`, `beliefs(topic)`, `whodis(name)`, `today()`/`standup()`, and `forget(entity|timerange|pattern)`.
- **FR-018**: Every recall/query result MUST carry provenance (source, app, time) and, where applicable, a confidence signal.
- **FR-019**: The read path MUST make no network calls (Invariant 2); answering any query is local computation only.
- **FR-020**: The system MUST extract commitments (owner, counterpart, due date, status) and beliefs (current position + revision history) at **write time**, never in the read path.

**Cloud governance (US5)**

- **FR-021**: When write-time enrichment uses a cloud model, it MUST run only post-gate (on redacted content), only on content a policy allows, using the user's own API key, and MUST produce a per-call disclosure receipt (Invariant 4).
- **FR-022**: The system MUST remain fully functional in the read path when cloud enrichment is unavailable; ingestion and recall MUST NOT depend on network availability.

**Forget & receipts (US4)**

- **FR-023**: Users MUST be able to forget by entity, time range, or pattern, after which the matched memories no longer surface in recall.
- **FR-024**: Every deletion MUST produce an audit receipt describing scope, match count, and timestamp, sufficient to prove what was removed without re-exposing it (Invariant 3).

**Controls & defaults (US7)**

- **FR-025**: The system MUST provide a single command to pause all capture and a `status` command reflecting current state, and a single command to destroy all captured data (Invariant 5).
- **FR-026**: Sensitive capture surfaces MUST be off by default and require explicit opt-in (conservative defaults).
- **FR-027**: The system MUST ship as extensions of the existing single binary (new behaviors as subcommands) and reuse the existing daemon chassis, policy engine, and audit crate as its governance layer.

**Provenance (cross-cutting)**

- **FR-028**: Every stored memory MUST carry provenance sufficient to trace it back to its capture source and time (Invariant 3, first clause).

### Key Entities *(include if feature involves data)*

- **Observation**: The normalized unit of captured activity after it passes the gate. Attributes: id, source kind (screen OCR / a11y tree / ambient audio / dictation, plus reserved variants for Phase 3.5 account connectors: Email / Slack / GitHub), app/account, capture time (valid-time start), post-redaction text, redaction references, optional session id, optional speaker tag, content hash. The source-kind set is designed to grow to account connectors without a schema break. Never stores unredacted text.
- **RedactionEvent**: A record of one redaction: type (secret/PII category), a span reference (not the plaintext), and the policy id that caused it. Chained into the tamper-evident ledger.
- **Session**: A first-class grouping of observations (a meeting or work-focus block) with boundaries, enabling episode-level consolidation. The boundary concept is source-agnostic so that a Phase 3.5 connector can treat a mail thread or a channel-day as one session (minting episodes like "the pricing thread with Chris") using the same mechanism as an app-focus block.
- **Entity**: A person/place/thing/topic extracted from observations, with provenance; the subject of `whodis` and a node in temporal queries.
- **Commitment**: An extracted promise with owner, counterpart, due date, and status (mine or theirs).
- **Belief**: A stated position on a topic with confidence, provenance, and a revision history (bi-temporal supersession).
- **DecisionReceipt / DisclosureReceipt**: Audit records — for ingest/policy decisions, for cloud disclosures (what left the gate, under which policy), and for deletions (what was forgotten).
- **IngestPolicy**: A deterministic rule (per-app/per-domain) governing what may be captured and what may pass the gate to a cloud model.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A full week (≥7 days) of the maintainer's activity is ingested with average ingest CPU under 5%, and the redaction ledger is populated.
- **SC-002**: The recall@3 number from the 30-question eval set is measured and recorded, and the architecture branch is selected from it (the Phase 0 gate is passed with a decision, not a guess).
- **SC-003**: Via the connector, Claude answers all seven query types about the real week, each answer carrying provenance/receipts.
- **SC-004**: `forget` works end-to-end and produces an audit receipt for a test entity.
- **SC-005**: Install-to-first-grounded-answer takes under 5 minutes on a clean machine.
- **SC-006**: Zero network requests occur while serving any read-path query (verifiable by observation).
- **SC-007**: No unredacted sensitive content (secrets or third-party PII from the test corpus) appears in the persisted store, and content from excluded apps produces zero stored observations.
- **SC-008**: A single command pauses all capture (reflected in status) and a single command destroys all captured data; both verified end-to-end.
- **SC-009**: Dedupe collapses repeated screen text such that the stored observation count is a small fraction of raw captured frames (measured against the Phase 0 corpus).

## Assumptions

- **Reused, verified assets**: The existing daemon chassis (async runtime, unix sockets, event bus, supervisor), policy engine (deterministic rules, hot-reload, wall-clock budgets, rule trace), audit crate (single-writer, tamper-evident ledger), MCP server patterns, and installer are adopted directly as the governance/transport layer. The v1 policy engine (142 tests green as of 2026-07-17) is absorbed, not rewritten.
- **External dependencies as separate crates**: The memory store (temporal observe/recall/beliefs/goals/unlearn, on-device entity extraction and embeddings), the redaction library (regex bank + NER + evidence ledger), and the retrieval-tier fallback are consumed as separate crates. homn is the composition. The capture source (Screenpipe) is consumed, not rebuilt, behind a swappable source abstraction.
- **Capture status**: The screen-capture source is **not yet installed** on the dogfood machine — installing it is the first task of the validation gate. The dictation ASR pipe is already running.
- **Policy language**: The existing deterministic rule language is used for per-app ingest policies; a second policy language is explicitly rejected for v1.
- **Cloud at write-time only**: Commitment/belief extraction and answer synthesis may use a cloud model, always post-gate, per-policy, with the user's own key and a receipt. Answer synthesis is performed by the MCP client (Claude) itself and costs the product nothing. A local model swap-in for extraction is evaluated on the eval set but not required for v1.
- **Platform**: The dogfood and definition-of-done target is the maintainer's Linux machine. A macOS port is a launch-audience decision deferred to the end of v1, not an upfront requirement. The dictation source is Linux-only.
- **Hardware ceiling**: Anything resident is budgeted against a 6 GB GPU; no heavy local model is required for v1 (the read path is deterministic local math; extraction is cloud-first).
- **Phase 3.5 account connectors are the immediate next phase, not this spec's deliverable.** Gmail/Slack/GitHub sources (poll-based, read-only scopes, OAuth tokens in the system keyring, everything through the same gate, thread/channel-day session boundaries) build directly on this spec's `Source` abstraction and reserved source-kind variants. Build order there is driven by which Phase 3 `commitments()`/`whodis()` eval failures a connector's absence causes — decided from data, not upfront. The only obligation this spec carries is the forward-compat design constraint (FR-005a and the Observation/Session notes).
- **Out of scope for this spec**: The Phase 3.5 connector implementations themselves; the proactive live-meeting copilot (predictive retrieval + local triage + whispered suggestions); and the cursor-buddy/voice body. Phase 4–5 get their own spec after launch signal.
- **The 30-question eval set** is authored from the maintainer's actual captured week; it is both the Phase 0 gate and the ongoing regression suite.
