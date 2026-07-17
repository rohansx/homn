# Contract: the ingest → gate → store pipeline

`homnd` + `homn-gate`. The single place where Invariant 1 (nothing unredacted touches disk) is enforced. The gate **precedes** the store, always, and **fails closed**.

## Pipeline stages (per `RawCapture`)

```
RawCapture (pre-gate)
  │
  1. POLICY  (homn-policy, Rhai)         → IngestAction { Deny | Redact(kinds) | Allow | AllowCloud }
  │      Deny  → drop; write DecisionReceipt(deny); advance cursor; STOP
  │
  2. REDACT  (homn-gate / cloakpipe)     → redacted text + Vec<RedactionEvent>
  │      redaction error → FAIL CLOSED: drop item, write DecisionReceipt(error), do NOT store; STOP
  │
  3. DEDUPE  (content-hash + shingle)    → if duplicate: drop; (optionally note); STOP
  │
  4. SESSIONIZE                          → assign/extend SessionId
  │
  5. STORE   (agidb.observe)             → Observation persisted (post-redaction only)
  │      write RedactionEvents to ledger (hash-chained); write DecisionReceipt(allow)
  │
  6. WATERMARK advance                   → only now is the source cursor persisted
  │
  7. (async, write-time, optional) EXTRACT commitments/beliefs
         if cloud: only on AllowCloud content, user's key → DisclosureReceipt
```

## Hard rules

- **R-1 Gate precedes store.** No code path constructs a stored Observation from pre-redaction text. The gate function's output type *is* the storable Observation; there is no other constructor. (Invariant 1)
- **R-2 Fail closed.** Any error in POLICY or REDACT ⇒ the item is dropped, a receipt records the failure, nothing is persisted. Never fall through to storing unredacted text. (FR-012)
- **R-3 Watermark after durability.** The source cursor advances only after the item is durably stored *or* durably dropped by policy — so a crash re-reads, and dedupe collapses the replay. (R7)
- **R-4 Ledger completeness.** Every redaction ⇒ a hash-chained ledger row with no plaintext. Every ingest decision (allow/deny/redact/error) ⇒ a DecisionReceipt. (Constitution III, FR-015)
- **R-5 Cloud only past the gate.** EXTRACT may call cloud only on `AllowCloud` content, post-redaction, with the user's key, and MUST emit a DisclosureReceipt. Read path is never in this diagram. (Invariant 4)
- **R-6 Backpressure.** Stages are connected by bounded channels; if STORE lags, POLICY/REDACT slow, and sources see back-off — no unbounded buffering, no silent drops. (FR-010)

## Policy evaluation contract (reuses v1 `homn-policy`)

- Input: `{ app, domain, source_kind, window_title, incognito }`.
- Output: `IngestAction` + the rule id that fired (for the receipt + trace).
- Hot-reload: editing `policies/ingest.rhai` takes effect without restarting capture (FR-013). Wall-clock budget enforced as in v1.
- Default when no rule matches: conservative — DENY for sensitive-flagged surfaces, ALLOW+Redact otherwise, never AllowCloud (FR-026).
