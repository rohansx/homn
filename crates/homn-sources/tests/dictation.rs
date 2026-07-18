//! Integration test for `DictationPipe` (task T026).
//!
//! Drives the pipe through the [`Source`] trait exactly as homnd will: a convox-voice stand-in
//! connects to the unix socket, writes newline-delimited utterances, and the daemon-side loop
//! calls `fetch_since` — from the beginning, from an advanced cursor (resume), and from a stale
//! cursor (at-least-once superset re-read).

use homn_sources::{Batch, DictationPipe, Source};
use homn_types::{Cursor, SourceKind, SpeakerTag};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

/// A per-test socket path in the system tmp dir (unix socket paths must stay short).
fn sock_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("homn-dict-{name}-{}.sock", std::process::id()))
}

/// Poll `fetch_since` until at least `n` items arrive (the reader task is async to the writer).
async fn wait_for_lines(pipe: &DictationPipe, cursor: Option<&Cursor>, n: usize) -> Batch {
    for _ in 0..200 {
        let batch = pipe.fetch_since(cursor).await.unwrap();
        if batch.items.len() >= n {
            return batch;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {n} dictation line(s)");
}

#[tokio::test]
async fn pushed_lines_become_captures_and_resume_from_cursor() {
    let path = sock_path("basic");
    let pipe = DictationPipe::bind("dictation-test", Some(path.clone())).unwrap();
    assert_eq!(pipe.id(), "dictation-test");
    assert_eq!(pipe.kind(), SourceKind::Dictation);

    let mut client = UnixStream::connect(&path).await.unwrap();
    client
        .write_all(b"take out the trash\nemail bob about the quote\n")
        .await
        .unwrap();
    client.flush().await.unwrap();

    // First fetch from the beginning: both lines, in push order, as Dictation captures.
    let batch = wait_for_lines(&pipe, None, 2).await;
    assert_eq!(batch.items.len(), 2);
    assert_eq!(batch.items[0].text, "take out the trash");
    assert_eq!(batch.items[1].text, "email bob about the quote");
    assert_eq!(batch.items[0].source, SourceKind::Dictation);
    assert_eq!(batch.items[0].speaker, Some(SpeakerTag::Me));
    // Cursor is the monotonic line sequence number, serialized in the opaque Cursor.
    assert_eq!(batch.next, Cursor::new(2u64));
    assert!(batch.exhausted);

    // Nothing new strictly after the advanced cursor; cursor does not regress.
    let empty = pipe.fetch_since(Some(&batch.next)).await.unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.next, batch.next);

    // Resume-from-cursor: push one more line, only the new one arrives.
    client.write_all(b"call mum at five\n").await.unwrap();
    client.flush().await.unwrap();
    let resumed = wait_for_lines(&pipe, Some(&batch.next), 1).await;
    assert_eq!(resumed.items.len(), 1);
    assert_eq!(resumed.items[0].text, "call mum at five");
    assert_eq!(resumed.next, Cursor::new(3u64));

    // A stale cursor re-reads a superset (at-least-once; dedupe collapses downstream).
    let superset = pipe.fetch_since(None).await.unwrap();
    assert_eq!(superset.items.len(), 3);
}

#[tokio::test]
async fn non_numeric_cursor_is_rejected_not_panicked() {
    let path = sock_path("badcursor");
    let pipe = DictationPipe::bind("dictation-badcursor", Some(path)).unwrap();
    let err = pipe
        .fetch_since(Some(&Cursor::new("not-a-sequence-number")))
        .await
        .unwrap_err();
    assert!(matches!(err, homn_sources::SourceError::InvalidCursor(_)));
}
