# Phase 3 — The brain (ctxgraph integration)

> Weeks 11–20. Wire `ctxgraph` into the daemon. Make policy memory-aware. Make the face context-aware. Surface what you've forgotten.

## What ships

A `ctxgraph` integration that ingests git activity and (opt-in) Claude transcripts; surfaces session-resume offers; surfaces open loops; exposes ctxgraph-aware Rhai helpers for policy rules; proxies ctxgraph's MCP tools through `homn`.

### Concretely

```
$ homn brain                       # status: what's ingesting, what's idle
$ homn brain status

$ homn brain enable git            # default ON, just for explicitness
$ homn brain enable transcripts    # opt-in — privacy disclaimer printed
$ homn brain enable calendar       # opt-in
$ homn brain enable gmail          # opt-in
$ homn brain enable browser        # opt-in (requires browser extension)

$ homn brain query "when did i last talk to naman about prior inventions"

$ homn brain loops                 # list open loops
$ homn brain loops --dismiss <id>
$ homn brain loops --snooze <id> --until tomorrow
```

## Milestone breakdown

### Week 11 — ctxgraph readiness audit

Before any new code, **audit ctxgraph**:

- Confirm current API supports the ingestor pattern.
- Confirm schema can be extended with new node types (`session`, `command`, `open_loop`) without breaking existing users.
- Confirm Rhai-callable helpers can be exposed without an FFI dance.
- File issues against ctxgraph for any gap; sequence them ahead of Phase 3 work.

**Exit:** a written audit doc that says either "ctxgraph is ready, here's the integration plan" or "ctxgraph needs X, Y, Z first — we're delaying Phase 3 by N weeks."

### Weeks 12–13 — Ingestors v0 (git + transcripts opt-in)

- Git watcher: `notify-rs` on watched repos; ingest commits, branches, push events
- Claude transcript subscriber: SessionStart + Stop hooks → ctxgraph
- Redaction layer (regex-based) for transcripts (per [architecture/brain.md](../architecture/brain.md))
- ctxgraph schema extensions: `session`, `command`, `commit`, `pr` nodes; relations
- `homn brain status` and `homn brain enable/disable` CLI

**Exit:** running `homn brain status` shows git events flowing in for the author's main repos.

### Weeks 14–15 — Session resumption

- `SessionStart` hook → ctxgraph query: "any prior session in this cwd within N days, any open loops?"
- If yes, daemon emits `SessionResumeOffer` BusEvent
- Face renders thought bubble; click → injects context via `UserPromptSubmit` hook
- A/B testing harness to tune the summarization prompt against author's own data

**Exit:** offer is shown >80% of the time when relevant; click-through >30%.

### Weeks 16–17 — Open-loop surfacing

- Open-loop detection: drafts in Gmail not sent in 48h (opt-in), PRs ahead-of-main without merge, branches with commits but no PR, "TODO" mentions in transcripts without follow-up
- Surfacing engine: max 1 nudge per hour, only when idle ≥N min
- Dismissal / snooze flow
- Tuning loop: dismissal rate >40% → cadence increase; <20% → cadence safe

**Exit:** the author personally finds at least 3 forgotten loops the daemon surfaced in week 17.

### Weeks 18–19 — Context-aware policy rules + searchable memory

- Rhai helpers: `ctxgraph.recently_edited`, `ctxgraph.has_open_pr_passing_ci`, `ctxgraph.previous_decisions`, etc.
- Example rules added to `default.rhai`'s shipped version
- Face hover panel: search box → ctxgraph query → results with provenance
- MCP server: proxy ctxgraph tools through `homn` (`ctxgraph_search`, `ctxgraph_session_history`, `ctxgraph_open_loops`)

**Exit:** at least 3 rules in the author's `default.rhai` use ctxgraph helpers and meaningfully improve allow/deny accuracy.

### Week 20 — Launch prep

- Blog post: "the brain inside homn — wiring ctxgraph as a memory subsystem for coding agents"
- ctxgraph case study: showcase the standalone library
- Demo video v2: same daemon, now with memory
- Reach out to RAG / agentic-AI infrastructure communities (LangChain, LlamaIndex, etc.)

## Out of scope for Phase 3

- Cloud sync of ctxgraph data. Open-core's paid tier is *rules* sync, not *graph* sync.
- Multi-user / team-shared ctxgraph. Personal memory is personal.
- LLM-summarization of memory beyond what the user opts into. The brain stores raw events; LLMs summarize on-demand at query time.

## Success metrics

| Metric                                                   | Target (30 days post-Phase-3 launch)   |
|----------------------------------------------------------|----------------------------------------|
| Session-resumption click-through                         | >30%                                   |
| Open-loop nudge dismissal rate                           | <40%                                   |
| Active users who run ≥1 `homn brain query` per week      | ≥30% of opt-ins                        |
| ctxgraph repo stars / installs (causal lift from launch) | ≥250 new stars within 30 days          |
| Audit log: rules referencing ctxgraph helpers            | ≥10% of the author's default.rhai      |

## Risks (Phase 3-specific)

- **ctxgraph readiness slips** — the Week 11 audit gates the whole phase. If gaps are bigger than expected, Phase 3 delays.
- **Transcript privacy footgun** — even with redaction, a regression could leak. Mitigation: opt-in default, encrypted at rest, per-project allowlist, ship a `homn brain audit-redaction` tool that shows what would be persisted for a given transcript.
- **Schema migration breaks existing ctxgraph users** — every migration lands behind a feature flag until ctxgraph users have upgraded. Coordinate with ctxgraph release cadence.
- **Open-loop noise** — if the heuristics surface stale or false loops, dismissal rate goes >40% and the face becomes annoying. Plan B: tighter idle-detection + fewer loop sources in v0.

## What this phase is NOT trying to do

- It's **not** trying to be a personal-CRM. It's trying to be the memory of *what you're working on*.
- It's **not** trying to ingest everything. Conservative defaults; opt-in for the long tail.
- It's **not** an LLM-judgment layer. Rules + memory, deterministic.
