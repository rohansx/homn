# Contract: the `Source` abstraction

`homn-sources`. The swappable capture abstraction. It MUST express **both** a sqlite-tail source and a poll-based OAuth cursor source with the *same* trait, so Phase 3.5 connectors add no breaking change (FR-005a, research R9).

## Shape

```rust
/// Opaque, serializable resume position. sqlite-tail: last row id.
/// Gmail: history id. Slack: `oldest` ts. GitHub: events cursor.
type Cursor = serde_json::Value;   // or a small enum; the point is: opaque + serializable

#[async_trait]
trait Source: Send + Sync {
    /// Stable id used to key the watermark row.
    fn id(&self) -> &str;

    /// The kind of observations this source produces.
    fn kind(&self) -> SourceKind;

    /// Fetch items strictly after `cursor` (None = from the beginning / a sane default window).
    /// Returns raw, PRE-gate items plus the advanced cursor.
    /// Poll sources block/sleep internally to respect provider rate limits; tail sources
    /// return promptly. Either way this is called in a loop by homnd.
    async fn fetch_since(&self, cursor: Option<&Cursor>) -> Result<Batch, SourceError>;
}

struct Batch {
    items: Vec<RawCapture>,   // pre-gate; text may contain secrets/PII
    next: Cursor,             // advanced position; persisted ONLY after items are gated+stored
    exhausted: bool,          // true = caller may back off (no more right now)
}

struct RawCapture {
    upstream_ref: String,     // provenance anchor (row id, message id, event id)
    source: SourceKind,
    app: Option<String>,
    captured_at: DateTime<Utc>,
    text: String,             // PRE-redaction
    speaker: Option<SpeakerTag>,
}
```

## Behavioral contract

- **Read-only upstream.** A source never mutates its origin (Screenpipe sqlite, provider API). Tokens for poll sources are read-only scopes (Phase 3.5).
- **Cursor monotonicity.** `next` always ≥ the input cursor; re-`fetch_since(cursor)` after a crash returns a superset that dedupe collapses (at-least-once upstream, exactly-once stored — R7).
- **No gate knowledge.** A source emits pre-gate `RawCapture`. It never persists, redacts, or decides policy — that is the pipeline's job. This keeps Invariant 1 enforced in exactly one place.
- **Backpressure-friendly.** `fetch_since` returns bounded batches; the daemon controls cadence. A source must not buffer unbounded upstream data internally.

## v1 implementations

- `ScreenpipeTail` — polls Screenpipe sqlite by `id > cursor`; `kind()` varies per row (ScreenOcr / A11yTree / AmbientAudio).
- `DictationPipe` — push-based convox-voice over a unix socket / stdin; cursor is a monotonic sequence; `kind() = Dictation`.

## Phase 3.5 (scaffold only in v1)

- `GmailSource` / `SlackSource` / `GitHubSource` — poll-based, cursor = history id / `oldest` ts / events cursor, OAuth tokens in the system keyring. **Not built in v1**; a compile-time test implements a trivial poll-cursor source to prove the trait accommodates it without change.
