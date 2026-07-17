# Data Model: homn v2 — Ambient Memory (v1)

Phase 1 output. Entity shapes, relationships, validation rules, and state transitions derived from the spec's Key Entities + Functional Requirements. Types live in `homn-types` (additive) and the new crates; nothing here breaks the change-frozen v1 types.

Field types are illustrative Rust — the contract is the shape and the invariants, not the exact syntax.

---

## Observation

The normalized unit of captured activity **after** it passes the gate. This is what reaches the store; unredacted text never becomes an Observation.

```rust
struct Observation {
    id: Ulid,                        // sortable, time-ordered
    source: SourceKind,              // see enum below
    app: Option<String>,            // "Slack", "Chrome:gmail.com", "Zoom", or account id for connectors
    captured_at: DateTime<Utc>,     // valid-time start (when it happened)
    ingested_at: DateTime<Utc>,     // transaction-time start (when we recorded it) — bi-temporal
    text: String,                    // POST-redaction text only
    redactions: Vec<RedactionRef>,   // references into the ledger; never plaintext
    session: Option<SessionId>,      // meeting / work-focus grouping
    speaker: Option<SpeakerTag>,     // audio only: Me | Other | Unknown
    content_hash: Blake3Hash,        // dedupe key (over post-redaction text + source + app)
    provenance: Provenance,          // source cursor position + upstream ref (Invariant 3)
}
```

**SourceKind** (open set; connector variants reserved for Phase 3.5):

```rust
enum SourceKind {
    ScreenOcr,
    A11yTree,
    AmbientAudio,
    Dictation,
    // reserved — Phase 3.5, not emitted in v1:
    Email,
    Slack,
    GitHub,
}
```

**Validation / invariants**
- `text` MUST be post-gate. Constructing an Observation from pre-gate text is a type error the pipeline design forbids (the gate returns the Observation, not the raw capture).
- `content_hash` MUST be stable for identical post-redaction content from the same source+app (dedupe correctness).
- `provenance` MUST be sufficient to trace back to the upstream source + cursor position (FR-028).
- `redactions` entries reference ledger rows; they carry no plaintext.

---

## RedactionRef & RedactionEvent

`RedactionRef` lives on the Observation; the full `RedactionEvent` lives in the hash-chained ledger.

```rust
struct RedactionRef {
    kind: RedactionKind,     // ApiKey | Token | Card | Aadhaar | Pan | PersonPii | Email | Phone | ...
    span: SpanRef,           // offset+len OR placeholder token id in the redacted text — NOT the original bytes
    policy_id: PolicyId,     // which rule/detector caused it
    ledger_seq: u64,         // position in the hash chain
}

struct RedactionEvent {      // ledger row (homn-audit)
    seq: u64,
    prev_hash: Blake3Hash,   // hash chain link
    this_hash: Blake3Hash,   // = H(prev_hash || canonical(payload))
    observation_id: Ulid,
    kind: RedactionKind,
    span: SpanRef,
    policy_id: PolicyId,
    at: DateTime<Utc>,
    // NO plaintext field — ever
}
```

**Invariants**
- The chain is verifiable: recomputing `this_hash` from `prev_hash` + payload MUST match for every row (tamper-evident, FR-015).
- No row contains redacted plaintext (FR-015).

---

## Session

First-class grouping enabling episode-level consolidation.

```rust
struct Session {
    id: SessionId,
    kind: SessionKind,       // AppFocus | Meeting | (Phase 3.5) MailThread | ChannelDay
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,  // None = open/active
    label: Option<String>,   // e.g., "call with X" once an entity is known
    app_or_channel: Option<String>,
}
```

**State transitions**: `Open` → (boundary detected) → `Closed`. Boundaries: app-focus change, meeting-app stop / mic-inactive, idle timeout. The boundary detector is **source-agnostic** (R8/R9) so connectors reuse it.

---

## Entity

Extracted subject of `whodis` and node in temporal queries. (agidb-owned; shown for completeness.)

```rust
struct Entity {
    id: EntityId,
    kind: EntityKind,        // Person | Org | Project | Topic | Place | ...
    canonical_name: String,
    aliases: Vec<String>,
    provenance: Vec<Ulid>,   // observation ids that mention it
}
```

**Validation**: extraction precision is measured in Phase 0 (FR-003); low-confidence entities from OCR noise MUST NOT poison recall (kept below a confidence floor or quarantined).

---

## Commitment

Extracted promise, mine or theirs. Write-time only (FR-020).

```rust
struct Commitment {
    id: CommitmentId,
    text: String,            // post-redaction
    owner: Party,            // Me | Entity(EntityId)
    counterpart: Option<Party>,
    due: Option<DateTime<Utc>>,
    status: CommitmentStatus,   // Open | Fulfilled | Overdue | Cancelled
    source_obs: Ulid,
    extracted_by: ExtractionSource,  // Cloud{model} | Local{model}
    disclosure_receipt: Option<ReceiptId>,  // if cloud saw it
    created_at: DateTime<Utc>,
}
```

**Validation**: `disclosure_receipt` MUST be present iff `extracted_by == Cloud` (Invariant 4 traceability).

---

## Belief

Stated position with revision history (bi-temporal supersession).

```rust
struct Belief {
    id: BeliefId,
    topic: String,
    position: String,        // current stated position, post-redaction
    confidence: f32,
    provenance: Vec<Ulid>,
    supersedes: Option<BeliefId>,   // revision chain — the previous position
    valid_from: DateTime<Utc>,
    recorded_at: DateTime<Utc>,
}
```

**State transitions**: a new position on the same topic creates a new `Belief` with `supersedes = Some(prev)`; `beliefs(topic)` returns the head plus the chain (FR / US5).

---

## Receipts (homn-audit ledger family)

Three receipt kinds, all hash-chained, all plaintext-free.

```rust
enum Receipt {
    Decision(DecisionReceipt),       // ingest/policy decision: allow | deny | redact, rule id, surface, latency
    Disclosure(DisclosureReceipt),   // what left the gate to a cloud model, under which policy, when, digest of payload
    Deletion(DeletionReceipt),       // forget: scope (entity|timerange|pattern), match_count, at — proves what was removed
}
```

**Invariants**
- Every ingest decision → a `Decision` receipt (Constitution III).
- Every cloud call → a `Disclosure` receipt (Invariant 4).
- Every `forget` → a `Deletion` receipt sufficient to prove scope **without re-exposing content** (Invariant 3, FR-024).

---

## IngestPolicy

A deterministic Rhai rule governing capture + gate-pass. Evaluated by `homn-policy` (hot-reload, wall-clock budget, trace).

```rust
// conceptual — expressed as Rhai rules in policies/ingest.rhai
struct IngestPolicy {
    id: PolicyId,
    match_on: Match,         // app glob, domain, source kind, window title incognito flag
    action: IngestAction,    // Deny (never capture) | Redact(kinds) | Allow | AllowCloud(kinds)
}
```

**Defaults (conservative, FR-026)**: sensitive surfaces DENY by default; `AllowCloud` requires an explicit opt-in rule + a configured key.

---

## Ingestion watermark

Crash-safe resume position, one per source, in SQLite.

```rust
struct Watermark {
    source_id: String,       // stable per Source instance
    cursor: Cursor,          // opaque, serializable — last row id OR history id OR oldest ts
    updated_at: DateTime<Utc>,
}
```

**Invariants**: advanced only after an item is durably stored (or durably dropped by policy); on restart, resume from `cursor` with dedupe as the idempotency backstop (R7).

---

## Relationships (summary)

```
Source --emits--> RawCapture --(gate)--> Observation --grouped-by--> Session
Observation --references--> RedactionEvent (ledger, hash-chained)
Observation --mentions--> Entity
Observation --(write-time extraction)--> Commitment, Belief   [+ Disclosure receipt if cloud]
forget(scope) --produces--> DeletionReceipt ; removes matching Observations/Entities/Commitments/Beliefs from recall
every ingest decision --produces--> DecisionReceipt
```

All five invariants are structural here: no Observation without a completed gate; no read touches the network; every Observation carries provenance; every cloud disclosure and every deletion carries a receipt; pause/destroy operate over the whole store + ledger.
