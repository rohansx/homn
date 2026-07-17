# Contract: the `homn` CLI surface (v2 additions)

One binary, subcommand-driven (Constitution VII). Every subcommand supports `--json` for scripting. New/extended subcommands for v1 ambient memory; v1 policy subcommands (`rule`, `log`, `install`, `run`, `daemon`, `setup`, `uninstall`) are retained.

| Command | Purpose | Story | Notes |
|---|---|---|---|
| `homn capture start` / `stop` | start/stop the ingestion daemon (`homnd`) | US2/US7 | `stop` == pause all capture (Invariant 5) |
| `homn pause` | alias: halt all capture immediately | US7 | reflected in `status`; resume with `capture start` |
| `homn status [--json]` | daemon state, sources, watermarks, obs/day, paused? | US7 | ops visibility; used by tray icon |
| `homn exclude <app\|domain>` | add a deny rule to `policies/ingest.rhai` | US3 | hot-reloaded; `--list` / `--remove` too |
| `homn forget <entity> \| --since/--until \| --pattern` | unlearn + print the deletion receipt | US4 | same op as MCP `forget`; prints `receipt_id` |
| `homn destroy [--yes]` | remove ALL captured memory + derived data | US7 | Invariant 5; requires confirm unless `--yes` |
| `homn connect [--print-link]` | print/generate the MCP connector link | US2 | paste-into-Claude onboarding |
| `homn eval run <question-set> [--k 3]` | score recall@k + ops metrics over ingested data | US1 | Phase 0 gate; becomes CI regression |
| `homn eval ingest <screenpipe.db>` | throwaway replay-ingest for Phase 0 (no redaction, own data) | US1 | write-time-cloud OFF; local only |
| `homn ledger verify` | verify the redaction/receipt hash chain | US3 | tamper-evidence check (FR-015) |
| `homn key set` | store the user's cloud API key (opt-in) | US5 | enables `AllowCloud` extraction; OFF until set |

## Rules

- **Conservative defaults** (FR-026): fresh install captures nothing sensitive; cloud extraction OFF until `homn key set` + an `AllowCloud` policy rule.
- **`--json` everywhere** (Constitution technical standards): machine-readable output for every subcommand.
- **Pause/destroy are always reachable** (Invariant 5): they work even if the MCP server or a source is wedged.
- **Receipts printed, not swallowed**: `forget`, `destroy`, and cloud-enabling ops surface their receipt ids.
