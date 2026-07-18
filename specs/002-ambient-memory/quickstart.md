# Quickstart — homn v2 ambient memory (v1)

> Onboarding for someone picking up the ambient-memory work. The critical path is **Phase 0**: get real capture, ingest it, and score recall before building anything on top.

## What you need installed

- Rust stable (1.88+). `rustup default stable`.
- `cargo`, `git`, `sqlite3` (ad-hoc store/ledger inspection).
- **Screenpipe** — the screen-capture source. **Not yet installed on the dogfood machine; installing it is Phase 0 task #1.**
- **convox-voice** — dictation ASR. Already running as a systemd user service (Linux).
- A Claude Desktop / claude.ai account for the connector (Phase 3).
- Host quirk: if a GTK/webkit build fails, `export PKG_CONFIG_PATH=/usr/lib/pkgconfig:/usr/share/pkgconfig` (Homebrew's `pkg-config` shadows the system one).

## Build

```sh
git clone https://github.com/rohansx/homn.git
cd homn
cargo build --workspace
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Phase 0 — the validation week (do this first)

The whole product is gated on one number: does the brain's recall survive real life?

1. **Install + run capture** for 5–7 normal working days:
   ```sh
   screenpipe record          # screen OCR + a11y tree + audio → local sqlite
   # convox-voice dictation is already running
   ```
2. **Replay-ingest** the captured week into the memory store (throwaway path — own data only, no redaction, cloud OFF):
   ```sh
   homn eval ingest ~/.local/share/screenpipe/db.sqlite
   ```
3. **Author the 30-question set** from *your actual week* into `eval/questions/<date>.toml`:
   10 factual · 10 temporal · 10 commitment/belief. (Template: `eval/questions/TEMPLATE.toml`.)
4. **Score it**:
   ```sh
   homn eval run eval/questions/<date>.toml --k 3
   ```
   Records recall@1, recall@3, observations/day, disk growth, ingest CPU, and GLiNER precision on a 100-extraction sample.

**Read the gate** (research R1):

| recall@3 | Do this |
|---|---|
| ≥ 70% | proceed with agidb as-is; skip Phase 2b |
| 40–70% | Phase 2b (ctxgraph retrieval merge) is mandatory before Phase 3 |
| < 40% | ctxgraph becomes the store; port agidb's belief/goal/unlearn types on top |

Keep `eval/questions/<date>.toml` — it becomes the CI regression suite.

## Phase 1 — run the ingestion daemon (once built)

```sh
homn capture start          # boots homnd; tails screenpipe + convox-voice
homn status --json          # sources, watermarks, obs/day, paused?
homn pause                  # halt all capture (Invariant 5)
```

## Phase 2 — the gate (once built)

```sh
homn exclude 1password      # add a deny rule (hot-reloaded)
homn exclude gmail.com      # domain deny
homn ledger verify          # verify the redaction hash chain is intact
```
Edit `policies/ingest.rhai` directly for richer rules; changes hot-reload.

## Phase 3 — wire into Claude (once built)

```sh
homn key set                # store your cloud API key (opt-in; enables write-time extraction)
homn connect --print-link   # prints the MCP connector link → paste into Claude
```
Then ask Claude the seven queries about your real week. Finally:
```sh
homn forget "Test Person"   # prints a deletion receipt
homn destroy --yes          # remove everything (Invariant 5)
```

## Invariants to keep in mind while hacking

1. Gate precedes store — never persist pre-redaction text.
2. No network in the read path (tools 1–6).
3. Every memory has provenance; every deletion a receipt.
4. Cloud only past the gate, per-policy, your key, receipted.
5. `homn pause` / `homn destroy` always work.

## Tests-first (Constitution VI)

Write failing tests before implementing: `homn-gate` (redaction, fail-closed, policy), `homn-audit` ledger (hash chain, no plaintext), `homnd` pipeline (watermark recovery, dedupe collapse, sessionizer boundaries).
