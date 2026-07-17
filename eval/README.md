# eval/ — the Phase 0 validation gate (and ongoing regression suite)

This directory holds the recall evaluation that **gates the entire product**. Before we build the ingestion spine, the gate, or the MCP surface on top of the memory store, we answer one question with data, not vibes:

> **Does the brain's recall survive real life?**

See [`specs/002-ambient-memory/`](../specs/002-ambient-memory/) — spec US1, plan Phase 0, research R1.

## The gate

Score recall@3 on a 30-question set drawn from a real captured week, then branch:

| recall@3 | Consequence |
|---|---|
| **≥ 70%** | agidb as-is. Skip Phase 2b. |
| **40–70%** | Phase 2b mandatory: fuse ctxgraph's retrieval tier into agidb before Phase 3. |
| **< 40%** | ctxgraph becomes the store; port agidb's belief/goal/unlearn types on top. |

The chosen branch is recorded (dated) in `research.md` under "R1 outcome".

## Layout

```
eval/
├── README.md               # this file
├── questions/
│   ├── TEMPLATE.toml       # the shape — copy per run
│   └── <YYYY-MM-DD>.toml    # a real set authored from that week's capture
└── results/
    └── <YYYY-MM-DD>.md      # recall@1/@3 + ops metrics for that run
```

## Running it (once the harness lands — tasks T011–T016)

```sh
# 1. (dogfood machine) install Screenpipe, then capture 5–7 normal working days
screenpipe record                        # + convox-voice dictation already running

# 2. throwaway replay-ingest — own data only, NO redaction, cloud OFF
homn eval ingest ~/.local/share/screenpipe/db.sqlite

# 3. author eval/questions/<date>.toml from YOUR actual week (see TEMPLATE.toml)

# 4. score
homn eval run eval/questions/<date>.toml --k 3
```

`homn eval run` prints recall@1, recall@3, per-kind breakdown, and the ops metrics
(observations/day, disk growth, ingest CPU, GLiNER extraction precision over a
100-extraction sample), plus the gate-verdict table above.

## Authoring a good question set

- **Exactly 10 / 10 / 10** across `factual`, `temporal`, `commitment` (the loader rejects other splits).
- Draw every question from the *actual* captured week — nothing synthetic.
- `expected_ref` is the ground-truth anchor (an observation id, a distinctive phrase, a person+date) the scorer checks for in the top-k hits. When auto-matching is ambiguous, fall back to hand-scoring and note it.
- Keep the set once authored — it becomes the CI regression suite (task T044); recall@3 must not regress below the chosen branch's threshold.

## Invariants honored by the eval path

- The throwaway ingest is **own data only, nothing leaves the machine** (no redaction is acceptable *only* because it never leaves; the real pipeline always gates before disk).
- Cloud extraction is **OFF** during Phase 0 — we measure the store's native recall, not cloud synthesis.
