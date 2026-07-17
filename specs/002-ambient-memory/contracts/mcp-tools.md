# Contract: the seven MCP query tools

`homn-mcp` (extends the v1 rmcp server), served over MCP to Claude. **Read path — zero network egress** (Invariant 2); every result carries provenance (FR-018). Tool names and params match architecture §6.

Result envelopes are illustrative JSON; the binding contract is: params, provenance on every hit, and the read-path/forget invariants.

## 1. `recall(cue, as_of?)`

Cue-based recall against the temporal store.
- **Params**: `cue: string`, `as_of?: datetime` (bi-temporal — recall as the world was known then).
- **Returns**: ranked hits, each `{ text, source, app, captured_at, confidence, observation_id }`.
- **Invariant**: computed by local math only; no network call.

## 2. `timeline(subject, from, to)`

Ordered "what happened."
- **Params**: `subject: { entity | topic }`, `from: datetime`, `to: datetime`.
- **Returns**: chronological `[{ at, text, session?, source, observation_id }]`.

## 3. `commitments(status?, due_before?)`

Extracted promises, mine and theirs.
- **Params**: `status?: Open|Fulfilled|Overdue|Cancelled`, `due_before?: datetime`.
- **Returns**: `[{ text, owner, counterpart?, due?, status, source_obs, extracted_by }]`.
- **Note**: populated by write-time extraction (FR-020); read is pure query.

## 4. `beliefs(topic)`

Current position + revision history.
- **Params**: `topic: string`.
- **Returns**: `{ current: { position, confidence, valid_from }, history: [{ position, valid_from, superseded_at }] }`.

## 5. `whodis(name)`

Relationship dossier.
- **Params**: `name: string`.
- **Returns**: `{ entity, interactions_count, last_thread: {...}, open_loops: [{ text, since }], recent: [...] }` — aggregates every interaction, last thread, unanswered follow-ups.

## 6. `today()` / `standup()`

Daily recap.
- **Params**: `date?: date` (default: today).
- **Returns**: `{ sessions: [...], did: [...], commitments_touched: [...], notable: [...] }`.

## 7. `forget(scope)`

The unlearn primitive — the one write tool.
- **Params**: `scope: { entity: string } | { timerange: {from,to} } | { pattern: string }`.
- **Returns**: `{ receipt_id, scope, match_count, at }`.
- **Invariants**: after success, matched memory no longer surfaces in tools 1–6; a `DeletionReceipt` is written that proves scope **without re-exposing content** (Invariant 3, FR-023/024). Also invokable as `homn forget` on the CLI.

## Cross-cutting

- **Provenance everywhere** (FR-018): every recall/timeline/whodis hit carries `source, app, captured_at, observation_id`.
- **No read-path egress** (FR-019/SC-006): none of tools 1–6 make a network call. Answer *synthesis* is done by Claude (the MCP client) itself — outside this server.
- **Transport**: streamable HTTP MCP + connector-link onboarding (paste into Claude), mirroring the minimi flow; local stdio also supported. Rate-limited via the v1 limiter.
- **Introspection (Constitution IV)**: the server also exposes why a given span was redacted/withheld, without escalating privilege.
