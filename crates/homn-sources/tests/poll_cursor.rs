//! Forward-compatibility proof for FR-005a (task T062).
//!
//! A Phase 3.5 account connector is a *poll-based cursor source* — it advances an opaque cursor
//! (a Gmail history id here) rather than tailing a local sqlite by row id. This test implements
//! such a source against the **current** [`Source`] trait to prove no breaking change is needed
//! when the real Gmail/Slack/GitHub connectors land. If this stops compiling, the trait regressed
//! its connector forward-compatibility.

use async_trait::async_trait;
use homn_sources::{Batch, Source, SourceError};
use homn_types::{Cursor, RawCapture, SourceKind};

/// A stand-in Gmail-style source: its cursor is an incrementing history id, not a row id.
struct FakeGmailSource {
    id: String,
    // Simulated inbox: (history_id, message text).
    messages: Vec<(u64, &'static str)>,
}

#[async_trait]
impl Source for FakeGmailSource {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Email // a reserved connector variant — already in the type system
    }

    async fn fetch_since(&self, cursor: Option<&Cursor>) -> Result<Batch, SourceError> {
        // Interpret the opaque cursor as a history id (0 if none).
        let since = match cursor {
            Some(Cursor(v)) => v
                .as_u64()
                .ok_or_else(|| SourceError::InvalidCursor(v.to_string()))?,
            None => 0,
        };

        let fresh: Vec<_> = self
            .messages
            .iter()
            .filter(|(hid, _)| *hid > since)
            .collect();

        let next_hid = fresh.last().map(|(hid, _)| *hid).unwrap_or(since);
        let items = fresh
            .iter()
            .map(|(hid, text)| RawCapture {
                upstream_ref: format!("gmail-msg-{hid}"),
                source: SourceKind::Email,
                app: Some("gmail:me@example.com".to_owned()),
                captured_at: chrono::DateTime::from_timestamp(*hid as i64, 0).unwrap(),
                text: (*text).to_owned(),
                speaker: None,
            })
            .collect();

        Ok(Batch {
            items,
            next: Cursor::new(next_hid),
            exhausted: true,
        })
    }
}

#[tokio::test]
async fn poll_cursor_source_drives_through_the_same_trait() {
    let src = FakeGmailSource {
        id: "gmail-work".to_owned(),
        messages: vec![
            (100, "I'll send the quote by Friday"),
            (101, "thanks, confirmed"),
            (102, "moved the call to 3pm"),
        ],
    };

    // First poll from the beginning: all three arrive, cursor advances to the latest history id.
    let batch = src.fetch_since(None).await.unwrap();
    assert_eq!(batch.items.len(), 3);
    assert_eq!(batch.next, Cursor::new(102u64));
    assert_eq!(src.kind(), SourceKind::Email);

    // Resume from the advanced cursor: nothing new (monotonic, no re-delivery beyond the cursor).
    let batch2 = src.fetch_since(Some(&batch.next)).await.unwrap();
    assert!(batch2.items.is_empty());
    assert_eq!(batch2.next, Cursor::new(102u64));

    // A stale cursor re-reads a superset (at-least-once), which dedupe would collapse downstream.
    let batch3 = src.fetch_since(Some(&Cursor::new(100u64))).await.unwrap();
    assert_eq!(batch3.items.len(), 2);
}

#[tokio::test]
async fn non_numeric_cursor_is_rejected_not_panicked() {
    let src = FakeGmailSource {
        id: "g".to_owned(),
        messages: vec![],
    };
    let err = src
        .fetch_since(Some(&Cursor::new("not-a-history-id")))
        .await
        .unwrap_err();
    assert!(matches!(err, SourceError::InvalidCursor(_)));
}
