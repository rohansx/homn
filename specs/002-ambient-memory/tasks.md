---
description: "Task list for homn v2 ambient-memory v1 (Phases 0-3)"
---

# Tasks: homn v2 — Ambient Memory (local human, v1)

**Input**: Design documents from `specs/002-ambient-memory/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)

**Tests**: Test-first is **mandatory** (Constitution VI) for `homn-gate`, the `homn-audit` ledger, and the `homnd` pipeline — those test tasks are not optional here. Other crates (CLI glue, MCP wiring) are tested loosely.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable (different files, no dependency on an incomplete task)
- **[Story]**: US1–US7 from spec.md
- Every task names an exact path.

## Legend — external prerequisite blockers

Tasks are tagged with what (if anything) they wait on. Everything else is **buildable now** in this repo.

- 🟢 **NOW** — buildable now (no external dep)
- 🟠 **SCREENPIPE** — needs Screenpipe installed + a real capture DB (dogfood machine)
- 🔵 **AGIDB** — needs the `agidb` crate wired as a dependency
- 🟣 **CLOAKPIPE** — needs the `cloakpipe` crate wired as a dependency
- ⚪ **CTXGRAPH** — conditional on the Phase 0 gate (only if recall@3 < 70%)

---

## Phase 1: Setup (shared infrastructure)

- [x] T001 🟢 Add new workspace members (`homnd`, `homn-sources`, `homn-gate`, `homn-eval`) to `Cargo.toml` `[workspace].members` and `[workspace.dependencies]`, mirroring the existing path+version pattern.
- [x] T002 [P] 🟢 Scaffold `crates/homn-sources/` (Cargo.toml + `src/lib.rs`) with deps: `homn-types`, `async-trait`, `serde`, `chrono`, `thiserror`.
- [x] T003 [P] 🟢 Scaffold `crates/homn-gate/` (Cargo.toml + `src/lib.rs`) with deps: `homn-types`, `homn-policy`, `homn-audit`, `regex`, `blake3`, `thiserror`.
- [x] T004 [P] 🟢 Scaffold `crates/homnd/` (Cargo.toml + `src/lib.rs` + `src/main.rs` behind the bin) with deps: `homn-daemon`, `homn-sources`, `homn-gate`, `homn-types`, `tokio`, `tracing`.
- [x] T005 [P] 🟢 Scaffold `crates/homn-eval/` (Cargo.toml + `src/lib.rs`) with deps: `homn-types`, `serde`, `toml`, `chrono`, `clap`.
- [x] T006 [P] 🟢 Create `eval/questions/TEMPLATE.toml` (10 factual / 10 temporal / 10 commitment-belief slots, each with `id`, `kind`, `question`, `expected_ref`, `notes`) and `eval/README.md` explaining the Phase 0 gate.
- [ ] T007 🟢 Add `agidb`, `cloakpipe` (and `ctxgraph`, feature-gated) as optional workspace dependencies with a git/path source and a `TODO(source)` note; guard them behind cargo features `brain-agidb`, `redact-cloakpipe`, `brain-ctxgraph` so the workspace still builds before they're vendored.

**Checkpoint**: `cargo build --workspace` and `cargo clippy` pass with empty crates.

---

## Phase 2: Foundational (blocking prerequisites for all stories)

- [x] T008 🟢 Add memory types to `crates/homn-types/src/` **additively** (new modules `observation.rs`, `source.rs`, `session.rs`, `receipt.rs`): `Observation`, `SourceKind` (incl. reserved `Email|Slack|GitHub`), `RawCapture`, `RedactionRef`, `RedactionKind`, `SpanRef`, `SessionId`, `Session`, `SessionKind`, `Cursor`, `Watermark`, `Provenance`, `Receipt` enum. No breaking edits to existing types (change-freeze). Per [data-model.md](./data-model.md).
- [x] T009 [P] 🟢 Unit-test the new types in `crates/homn-types/tests/memory_types.rs`: serde round-trips, `SourceKind` open-set stability, `content_hash` determinism helper.
- [x] T010 🟢 Define the `Source` trait + `Batch`/`SourceError` in `crates/homn-sources/src/source.rs` exactly per [contracts/source-trait.md](./contracts/source-trait.md) (opaque serializable `Cursor`, `fetch_since`, read-only, no gate knowledge).

**Checkpoint**: shared types + Source trait compile; every user story can now build against them.

---

## Phase 3: User Story 1 — Prove recall survives real life (Priority: P1) 🎯 Phase 0 gate

**Goal**: Measure recall@3 on a real captured week and pick the brain-architecture branch. This is the near-term deliverable; the harness is buildable now, the *run* waits on capture.

**Independent test**: `homn eval run eval/questions/<date>.toml --k 3` prints recall@1/@3 + ops metrics; the gate branch is chosen from the number.

### Harness (buildable now)

- [x] T011 [P] [US1] 🟢 Define the eval schema in `crates/homn-eval/src/schema.rs`: `QuestionSet`, `Question { id, kind: Factual|Temporal|Commitment, question, expected_ref }`, `RunResult { recall_at_1, recall_at_3, per_kind, ops }`, `OpsMetrics { observations_per_day, disk_growth_bytes, ingest_cpu_pct, extraction_precision }`.
- [x] T012 [US1] 🟢 Implement `QuestionSet` TOML load/validate in `crates/homn-eval/src/schema.rs` (reject a set that isn't 10/10/10) + test in `crates/homn-eval/tests/schema.rs`.
- [x] T013 [US1] 🔵 Implement recall scoring in `crates/homn-eval/src/score.rs`: given a `recall(cue)` callable over the store, compute recall@k by checking whether `expected_ref` appears in top-k hits; hand-score fallback path when auto-match is ambiguous. (Interface-level now; wired to agidb at T036.)
- [ ] T014 [P] [US1] 🟢 Implement ops-metric collection in `crates/homn-eval/src/ops.rs` (observations/day from store counts, disk delta from file sizes, CPU sampling hook, extraction-precision sampler over N extractions).
- [x] T015 [US1] 🟢 Wire `homn eval run <set> --k` and `homn eval ingest <db>` subcommands into `crates/homn-bin/src/main.rs` (clap), `--json` output; `eval run` prints the gate verdict table (≥70 / 40–70 / <40).
- [x] T016 [P] [US1] 🟢 Document the gate + how to author a question set in `eval/README.md` and cross-link from [quickstart.md](./quickstart.md) (already drafted) — keep them consistent.

### Throwaway ingest + the run (needs capture)

- [ ] T017 [US1] 🟠 Install Screenpipe on the dogfood machine and run `screenpipe record` + convox-voice for 5–7 working days (the passive validation week). **BLOCKS the run tasks below.**
- [x] T018 [US1] 🔵🟠 Implement the throwaway replay-ingest in `crates/homn-eval/src/ingest.rs`: tail the Screenpipe sqlite → naive chunk → `agidb.observe` (no redaction, own data only, cloud OFF). Reuses the T024 tail reader where possible.
- [ ] T019 [US1] 🟠 Author `eval/questions/<date>.toml` from the actual captured week (10 factual, 10 temporal, 10 commitment/belief).
- [ ] T020 [US1] 🔵🟠 Run `homn eval run` and record recall@1/@3 + ops metrics into `eval/results/<date>.md`.
- [ ] T021 [US1] 🟠 **Decide the brain branch** from recall@3 and record it in `specs/002-ambient-memory/research.md` (append a dated "R1 outcome" note): ≥70 → agidb as-is (skip Phase 2b) · 40–70 → Phase 2b mandatory · <40 → ctxgraph as store.

**Checkpoint**: a recall@3 number exists and the architecture branch is chosen. This gate governs Phase 2b (T041–T043).

---

## Phase 4: User Story 2 — Recall my week through Claude (Priority: P1) — the wedge

**Goal**: End-to-end capture → gate → store → `recall`/`timeline` via MCP, grounded + provenance, zero read-path egress.

**Independent test**: with capture running + connector linked, ask `recall`/`timeline` and get grounded, provenance-carrying answers; confirm no network call while answering.

### Ingestion spine — tests first (pipeline correctness, Constitution VI)

- [ ] T022 [P] [US2] 🟢 Write failing tests in `crates/homnd/tests/watermark.rs`: crash-safe watermark advances only after durable store; resume-from-cursor re-reads and dedupe collapses the replay (R7).
- [ ] T023 [P] [US2] 🟢 Write failing tests in `crates/homnd/tests/dedupe.rs`: exact content-hash drops identical frames; shingle-overlap collapses near-dupe OCR frames; SC-009 ratio assertion on a fixture.
- [ ] T024 [P] [US2] 🟢 Write failing tests in `crates/homnd/tests/sessionizer.rs`: app-focus + meeting-app boundaries mint sessions; source-agnostic boundary reused for a synthetic thread fixture (R8).

### Ingestion spine — implementation

- [ ] T025 [US2] 🟠 Implement `ScreenpipeTail` in `crates/homn-sources/src/screenpipe.rs` (poll sqlite `id > cursor`, map rows → `RawCapture`, `kind()` per row). Contract test may use a fixture DB; live run needs T017.
- [ ] T026 [P] [US2] 🟢 Implement `DictationPipe` in `crates/homn-sources/src/dictation.rs` (push-based unix-socket/stdin from convox-voice, monotonic cursor, `kind()=Dictation`).
- [ ] T027 [US2] 🟢 Implement dedupe (`crates/homnd/src/dedupe.rs`), sessionizer (`crates/homnd/src/sessionize.rs`), and chunker (`crates/homnd/src/chunk.rs`) to pass T022–T024.
- [ ] T028 [US2] 🟢 Implement the daemon pipeline loop + bounded-channel backpressure + watermark store in `crates/homnd/src/pipeline.rs` on the `homn-daemon` chassis (runtime, supervisor, unix socket).
- [ ] T029 [US2] 🟢 Add `homn capture start|stop`, `homn pause`, `homn status [--json]` subcommands in `crates/homn-bin/src/main.rs` talking to `homnd` over the unix socket.

### Store + MCP recall

- [ ] T030 [US2] 🔵 Wire `agidb` behind a `Store` trait in `crates/homnd/src/store.rs` (`observe(Observation)`, `recall(cue, as_of)`, `timeline(subject, from, to)`), feature `brain-agidb`.
- [x] T031 [US2] 🔵 Extend the rmcp server in `crates/homn-mcp/src/` with `recall` and `timeline` tools per [contracts/mcp-tools.md](./contracts/mcp-tools.md); every hit carries provenance; assert no network in the handler.
- [x] T032 [P] [US2] 🟢 Integration test `tests/read_path_no_egress.rs`: serving `recall`/`timeline` performs zero network syscalls (SC-006) — e.g. via a deny-all network guard in the test harness.
- [ ] T033 [US2] 🔵 Add `homn connect --print-link` (MCP connector link) in `crates/homn-bin/src/main.rs`; streamable-HTTP + stdio transports.

**Checkpoint**: MVP demo — Claude answers `recall`/`timeline` about a real session, provenance shown, no read-path egress.

---

## Phase 5: User Story 3 — The gate keeps sensitive data off disk (Priority: P1)

**Goal**: In-process redaction + Rhai ingest policy **before** the store; fail closed; hash-chained receipts.

**Independent test**: feed known secrets + PII + excluded-app content → store holds only redacted text, excluded app stores nothing, ledger has a plaintext-free chained entry per redaction.

### Gate — tests first (Constitution VI)

- [ ] T034 [P] [US3] 🟢 Failing tests in `crates/homn-gate/tests/redact.rs`: secrets (key/token/card/Aadhaar/PAN) + third-party PII are redacted; placeholders present, plaintext absent (FR-011).
- [ ] T035 [P] [US3] 🟢 Failing tests in `crates/homn-gate/tests/fail_closed.rs`: a redaction/policy error drops the item and persists nothing (FR-012, R-2).
- [ ] T036 [P] [US3] 🟢 Failing tests in `crates/homn-audit/tests/ledger_chain.rs`: hash chain verifies; tamper breaks verification; no ledger row contains plaintext (FR-015).
- [ ] T037 [P] [US3] 🟢 Failing tests in `crates/homn-gate/tests/ingest_policy.rs`: per-app/domain deny prevents any stored observation; hot-reload of `policies/ingest.rhai` takes effect without restart (FR-013).

### Gate — implementation

- [ ] T038 [US3] 🟣 Implement the redaction stage in `crates/homn-gate/src/redact.rs` (regex bank + NER via `cloakpipe`, feature `redact-cloakpipe`) → `(redacted_text, Vec<RedactionEvent>)`.
- [ ] T039 [US3] 🟢 Implement ingest-policy evaluation in `crates/homn-gate/src/policy.rs` reusing `homn-policy` (input `{app,domain,source_kind,window_title,incognito}` → `IngestAction`); seed `policies/ingest.rhai` with conservative defaults (FR-026).
- [ ] T040 [US3] 🟢 Extend `homn-audit` with the hash-chained redaction/receipt ledger (`Decision`/`Disclosure`/`Deletion`) in `crates/homn-audit/src/ledger.rs`; `homn ledger verify` subcommand.
- [ ] T041 [US3] 🟢 Assemble the gate as the pipeline stage in `crates/homn-gate/src/lib.rs` per [contracts/gate-pipeline.md](./contracts/gate-pipeline.md) (POLICY→REDACT→dedupe→sessionize→store, fail-closed, gate output *is* the storable Observation) and insert it into `homnd`'s `pipeline.rs` before store (T028).
- [x] T042 [P] [US3] 🟢 Add `homn exclude <app|domain> [--list|--remove]` in `crates/homn-bin/src/main.rs` (edits `policies/ingest.rhai`, hot-reloaded).

**Checkpoint**: nothing unredacted reaches disk; excluded apps produce zero observations; ledger verifiable (SC-007).

---

## Phase 5b: Brain merge — CONDITIONAL on the Phase 0 gate (T021)

> Do this phase **only if** recall@3 < 70%. Timeboxed 3 weeks (riskiest line). Skip entirely if ≥70%.

- [ ] T043 [US1] ⚪ Port `ctxgraph`'s retrieval tier into the `Store` behind a parallel index in `crates/homnd/src/store.rs`; recall fuses HDC-tier + graph-tier scores (40–70% branch) OR ctxgraph becomes primary with agidb belief/goal/unlearn types ported on top (<40% branch).
- [ ] T044 [US1] ⚪ Make `eval/questions/<date>.toml` the automated regression suite (`homn eval run` in CI); assert recall@3 does not regress below the chosen branch's threshold.

---

## Phase 6: User Story 4 — Forget on demand, with a receipt (Priority: P1)

**Goal**: `forget(entity|timerange|pattern)` removes from recall + emits a `DeletionReceipt` proving scope without re-exposing content.

**Independent test**: ingest a test entity, `homn forget "<entity>"`, confirm recall no longer surfaces it, confirm a receipt exists.

- [ ] T045 [P] [US4] 🟢 Failing test `crates/homn-audit/tests/deletion_receipt.rs`: a forget writes a chained `DeletionReceipt { scope, match_count, at }` with no forgotten plaintext (FR-024).
- [ ] T046 [US4] 🔵 Implement `forget` over the `Store` (agidb unlearn) in `crates/homnd/src/store.rs` for entity/timerange/pattern scopes; matched memory stops surfacing in tools 1–6 (FR-023).
- [ ] T047 [US4] 🔵 Add the `forget` MCP tool in `crates/homn-mcp/src/` and `homn forget <entity>|--since/--until|--pattern` in `crates/homn-bin/src/main.rs`; both print the `receipt_id`.
- [ ] T048 [P] [US4] 🟢 Integration test `tests/forget_end_to_end.rs`: ingest → forget → recall-miss → receipt present (SC-004).

**Checkpoint**: forget works end-to-end with an audit receipt.

---

## Phase 7: User Story 5 — Commitments and beliefs over time (Priority: P2)

**Goal**: Write-time extraction of commitments + beliefs; `commitments()`/`beliefs()` query them; cloud only post-gate, per-policy, receipted.

**Independent test**: ingest an explicit promise + a changed opinion; `commitments`/`beliefs` return them with status/due and revision history.

- [ ] T049 [P] [US5] 🟢 Failing test `crates/homn-gate/tests/disclosure_receipt.rs`: a cloud extraction call emits a `DisclosureReceipt` and runs only on `AllowCloud`, post-redaction content (Invariant 4, FR-021).
- [ ] T050 [US5] 🔵 Implement write-time extraction in `crates/homnd/src/extract.rs`: narrow explicit-promise + calendar/email signals → `Commitment`/`Belief`; pluggable backend `Cloud{Haiku}` | `Local{qwen2.5:3b}` (R10); runs off the store-write path, never on read (FR-020).
- [ ] T051 [US5] 🟢 Add `homn key set` (store cloud key in keyring; OFF until set) + an `AllowCloud` policy example in `policies/ingest.rhai`.
- [ ] T052 [US5] 🔵 Add `commitments(status?, due_before?)` and `beliefs(topic)` MCP tools in `crates/homn-mcp/src/` per [contracts/mcp-tools.md](./contracts/mcp-tools.md).
- [ ] T053 [P] [US5] 🔵 Extend the eval harness to A/B cloud-vs-local extraction on the commitment/belief questions (`crates/homn-eval/src/extract_ab.rs`); record precision.

**Checkpoint**: `commitments`/`beliefs` answer temporal questions; cloud disclosures receipted; read-path still offline (FR-022).

---

## Phase 8: User Story 6 — Relationship dossiers and a daily recap (Priority: P2)

**Goal**: `whodis(name)` + `today()`/`standup()` over the same store/extraction.

**Independent test**: after multi-session interactions with a person, `whodis` aggregates interactions/last thread/open loops; `today`/`standup` recap the day.

- [ ] T054 [US6] 🔵 Implement `whodis` aggregation in `crates/homnd/src/store.rs` (interactions, last thread, open loops from commitments/threads).
- [ ] T055 [P] [US6] 🔵 Implement `today`/`standup` aggregation in `crates/homnd/src/store.rs` (sessions + did + commitments-touched for a date).
- [ ] T056 [US6] 🔵 Add `whodis(name)` and `today()`/`standup()` MCP tools in `crates/homn-mcp/src/`.
- [ ] T057 [P] [US6] 🟢 Integration test `tests/seven_tools.rs`: all seven tools answer their query type with provenance (SC-003).

**Checkpoint**: the full seven-tool surface is live.

---

## Phase 9: User Story 7 — Pause everything, destroy everything (Priority: P2)

**Goal**: one command to pause all capture (with status), one to destroy all data; conservative defaults on fresh install.

**Independent test**: `homn pause` halts capture (status reflects it); resume works; `homn destroy` removes all captured data.

- [ ] T058 [US7] 🟢 Ensure `homn pause`/`homn status` (T029) truly halt every source and report paused state; add a resume path (`homn capture start`).
- [ ] T059 [US7] 🟢 Implement `homn destroy [--yes]` in `crates/homn-bin/src/main.rs`: remove the agidb store, the ledger, and watermarks; require confirm unless `--yes` (Invariant 5).
- [ ] T060 [P] [US7] 🟢 Enforce conservative defaults on fresh install: sensitive surfaces OFF, cloud OFF, in `homn setup` + seeded `policies/ingest.rhai` (FR-026); test in `tests/defaults_conservative.rs`.
- [ ] T061 [P] [US7] 🟢 Integration test `tests/pause_destroy.rs`: pause halts + status reflects; destroy leaves no captured data (SC-008).

**Checkpoint**: pause/destroy verified end-to-end.

---

## Phase 10: Polish & cross-cutting (Phase 3 ship prep)

- [x] T062 [P] 🟢 Forward-compat test `crates/homn-sources/tests/poll_cursor.rs`: a trivial poll-cursor `Source` (history-id style) compiles + drives via the same trait, proving no breaking change is needed for Phase 3.5 connectors (FR-005a).
- [ ] T063 [P] 🟢 Extend the installer (`install/` + `install.sh`) to install Screenpipe if absent, install `homnd`+agidb, print the MCP link; reuse checksum verification; target < 5 min install-to-first-answer (SC-005).
- [ ] T064 🟢 Rewrite `README.md` — "homn is the local human: memory, permissions, presence" — retire the two-meanings framing (Phase 3 task from tech-plan).
- [ ] T065 [P] 🟢 Ensure every new subcommand has `--json` output and derived help (Constitution technical standard); test in `tests/cli_json.rs`.
- [ ] T066 [P] 🟢 Record ADRs in `docs/architecture/adr/` for the load-bearing v2 decisions (brain gate, Rhai-not-Cedar, cloud-at-write-time, Source poll-cursor) referencing research.md R1/R3/R4/R9.
- [ ] T067 🟢 Full gate: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green; the demo script (seven queries + `forget` receipt) runs clean.

---

## Dependencies & completion order

```
Setup (T001–T007)
  └─> Foundational (T008–T010)
        ├─> US1 harness (T011–T016) ──┐  [buildable now]
        │      └─> US1 run (T017–T021) │ [needs Screenpipe capture] → gate → Phase 5b?
        ├─> US2 (T022–T033) ───────────┤  P1 wedge / MVP
        ├─> US3 (T034–T042) ───────────┤  P1 gate (US2's store write goes THROUGH US3's gate)
        ├─> US4 (T045–T048) ───────────┤  P1 forget
        ├─> US5 (T049–T053) ───────────┤  P2 (needs US2 store)
        ├─> US6 (T054–T057) ───────────┤  P2 (needs US2 store + US5 extraction)
        └─> US7 (T058–T061) ───────────┘  P2 (needs US2 capture)
Phase 5b (T043–T044): ONLY if T021 says recall@3 < 70%.
Polish (T062–T067): after the stories it touches.
```

**Hard ordering note**: US2's store-write (T028/T030) MUST route through US3's gate (T041) before persisting — Invariant 1. Build US2's spine and US3's gate together; do not ship US2 persistence without the gate.

## Parallel execution examples

- **Setup**: T002, T003, T004, T005, T006 in parallel (different crates).
- **US1 harness**: T011, T014, T016 in parallel while T012→T013 proceed.
- **US2 tests-first**: T022, T023, T024 in parallel (different test files) before T025–T028.
- **US3 tests-first**: T034, T035, T036, T037 in parallel before T038–T042.
- **Polish**: T062, T063, T065, T066 in parallel.

## Implementation strategy

- **MVP = US1 gate + US2 + US3** (the demo: capture → gate → store → Claude recalls, nothing unredacted on disk). US4 (forget receipt) completes the launch-video pair.
- **Drive Phase 0 (US1) first**: T011–T016 are buildable now with no external deps; land them so that the moment Screenpipe capture exists (T017), T018–T021 produce the gate number that unblocks the rest.
- **Respect the branch**: don't start Phase 5b (T043–T044) until T021 records recall@3.
- Ship at end of Phase 3 (through Phase 10 polish). Phase 3.5 connectors + Phases 4–5 are separate specs.

## External-prerequisite summary

| Blocker | Tasks gated | Unblock by |
|---|---|---|
| 🟠 Screenpipe capture | T017–T020, live side of T025 | install Screenpipe, run the validation week |
| 🔵 agidb crate | T013, T018, T030, T031, T033, T046, T047, T050, T052, T054–T056, T053 | vendor `agidb` + enable `brain-agidb` |
| 🟣 cloakpipe crate | T038 | vendor `cloakpipe` + enable `redact-cloakpipe` |
| ⚪ ctxgraph crate | T043, T044 | only if Phase 0 gate < 70% |

Everything tagged 🟢 (the eval harness scaffolding, all tests-first tasks, types, Source trait, CLI wiring, dedupe/sessionizer, docs) is **buildable now** in this repo without any external dependency.
