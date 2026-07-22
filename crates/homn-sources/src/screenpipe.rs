//! `ScreenpipeTail` — a tail [`Source`] over Screenpipe's local sqlite (task T025).
//!
//! Polls the `frames` table by `id > cursor` (the cursor is the last-read frame id, a JSON
//! integer), maps each OCR frame → a pre-gate [`RawCapture`], and advances the cursor. Read-only
//! upstream — it never mutates Screenpipe's DB. The gate (not this source) redacts and decides
//! persistence, so Invariant 1 stays enforced in one place.
//!
//! Screenpipe's v2 schema stores OCR text denormalized in `frames.full_text`; this source reads
//! that column (falling back to `window_name` + `app_name` when `full_text` is empty, so a frame
//! with only a window title still produces an observation). Audio lives in
//! `audio_transcriptions` and is a separate, later source — this one is screen OCR only.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use homn_types::{Cursor, RawCapture, SourceKind};

use crate::{Batch, Source, SourceError};

/// Tail Screenpipe's sqlite `frames` table by ascending `id`.
pub struct ScreenpipeTail {
    /// Path to Screenpipe's `db.sqlite`.
    db_path: PathBuf,
    /// Source id (keyed in the watermark table). Defaults to `"screenpipe"`.
    source_id: String,
    /// Max rows per fetch batch.
    batch_size: usize,
}

impl ScreenpipeTail {
    /// Open a tail over the Screenpipe sqlite at `db_path` with a sane default batch size.
    pub fn new(db_path: impl AsRef<Path>) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
            source_id: "screenpipe".to_owned(),
            batch_size: 200,
        }
    }

    /// Override the source id (e.g. `"screenpipe:ocr"` when audio is a separate source).
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.source_id = id.into();
        self
    }

    /// Override the max rows per batch.
    pub fn with_batch_size(mut self, n: usize) -> Self {
        self.batch_size = n.max(1);
        self
    }

    /// Decode the cursor into the last-read frame id (0 when None / unparseable → from start).
    fn last_id(cursor: Option<&Cursor>) -> i64 {
        cursor.and_then(|c| c.0.as_i64()).unwrap_or(0)
    }
}

#[async_trait]
impl Source for ScreenpipeTail {
    fn id(&self) -> &str {
        &self.source_id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::ScreenOcr
    }

    async fn fetch_since(&self, cursor: Option<&Cursor>) -> Result<Batch, SourceError> {
        let last_id = Self::last_id(cursor);
        let db_path = self.db_path.clone();
        let batch_size = self.batch_size;
        // sqlite is sync; run on a blocking thread so the async runtime isn't held.
        tokio::task::spawn_blocking(move || read_batch(&db_path, last_id, batch_size))
            .await
            .map_err(|e| SourceError::Other(format!("join: {e}")))?
    }
}

/// Read up to `batch_size` frames with `id > last_id`, ordered by id. Returns the items, the
/// new cursor (the highest id read, or `last_id` if none), and whether the source is exhausted
/// for now (fewer than `batch_size` rows returned).
fn read_batch(db_path: &Path, last_id: i64, batch_size: usize) -> Result<Batch, SourceError> {
    let conn = rusqlite::Connection::open(db_path)
        .map_err(|e| SourceError::Unavailable(format!("open {}: {e}", db_path.display())))?;
    // Screenpipe v2: frames(id, timestamp, app_name, window_name, full_text, ...).
    // If the table or column is missing, the source is unavailable (e.g. wrong DB).
    let mut stmt = conn
        .prepare(
            "SELECT id, timestamp, app_name, window_name, full_text \
             FROM frames WHERE id > ?1 ORDER BY id LIMIT ?2",
        )
        .map_err(|e| SourceError::Unavailable(format!("prepare frames query: {e}")))?;
    let mut items: Vec<RawCapture> = Vec::new();
    let mut max_id = last_id;
    let rows = stmt
        .query_map(rusqlite::params![last_id, batch_size as i64], |row| {
            let id: i64 = row.get(0)?;
            let ts: String = row
                .get::<_, Option<String>>(1)
                .unwrap_or_default()
                .unwrap_or_default();
            let app: Option<String> = row.get(2).unwrap_or(None);
            let win: Option<String> = row.get::<_, Option<String>>(3).unwrap_or(None);
            let full: Option<String> = row.get::<_, Option<String>>(4).unwrap_or(None);
            Ok((id, ts, app, win, full))
        })
        .map_err(|e| SourceError::Unavailable(format!("query frames: {e}")))?;
    for r in rows {
        let (id, ts, app, win, full) = r.map_err(|e| SourceError::Other(format!("row: {e}")))?;
        // Prefer full_text; fall back to the window title so a menu-only frame still registers.
        let text = match full {
            Some(t) if !t.trim().is_empty() => t,
            _ => win.clone().unwrap_or_default(),
        };
        if text.trim().is_empty() {
            max_id = id; // still advance past empty frames
            continue;
        }
        let captured_at = parse_ts(&ts).unwrap_or_else(Utc::now);
        items.push(RawCapture {
            upstream_ref: id.to_string(),
            source: SourceKind::ScreenOcr,
            app: app.or(win),
            captured_at,
            text,
            speaker: None,
        });
        max_id = id;
    }
    let exhausted = items.len() < batch_size;
    Ok(Batch {
        items,
        next: Cursor::new(max_id),
        exhausted,
    })
}

/// Parse a Screenpipe timestamp (ISO-8601, possibly with high-precision fractional seconds and
/// a `+00:00`/`Z` offset) into a UTC instant.
fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    // chrono's RFC3339 parser accepts up to 9 fractional digits + offset.
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            // Some Screenpipe rows drop the offset; assume UTC.
            DateTime::parse_from_rfc3339(&format!("{s}+00:00"))
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn build_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE frames (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                app_name TEXT,
                window_name TEXT,
                full_text TEXT
            );
            INSERT INTO frames (timestamp, app_name, window_name, full_text) VALUES
              ('2026-07-18T09:15:00Z', 'VS Code', 'main.rs', 'fn main(){}'),
              ('2026-07-18T09:16:00Z', 'VS Code', 'main.rs', 'fn main(){}'),  -- near-dupe text, still emitted (dedupe is the gate's job)
              ('2026-07-18T09:30:00Z', 'Slack', 'team', 'standup notes'),
              ('2026-07-18T09:45:00Z', 'Firefox', 'docs', NULL);             -- empty full_text → falls back to window_name
            "#,
        )
        .unwrap();
    }

    fn tmp_db(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "homn-screenpipe-tail-{}-{label}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[tokio::test]
    async fn tails_by_id_and_advances_cursor() {
        let db = tmp_db("tail");
        build_db(&db);
        let src = ScreenpipeTail::new(&db).with_batch_size(2);

        // First fetch: 2 rows (ids 1,2), not exhausted=false (got a full batch).
        let b1 = src.fetch_since(None).await.unwrap();
        assert_eq!(b1.items.len(), 2);
        assert!(!b1.exhausted);
        assert_eq!(
            b1.next.0.as_i64().unwrap(),
            2,
            "cursor advanced to last id read"
        );

        // Second fetch from cursor=2: rows 3,4. Row 4 has NULL full_text → window_name fallback.
        let b2 = src.fetch_since(Some(&b1.next)).await.unwrap();
        assert_eq!(b2.items.len(), 2);
        // A full batch was returned (2 == batch_size), so we can't yet know it's the last —
        // exhaustion is confirmed by the next fetch returning nothing.
        assert!(!b2.exhausted);
        assert_eq!(b2.items[0].text, "standup notes");
        assert_eq!(b2.items[0].app.as_deref(), Some("Slack"));
        assert_eq!(
            b2.items[1].text, "docs",
            "NULL full_text fell back to window_name"
        );
        assert_eq!(b2.items[1].app.as_deref(), Some("Firefox"));
        assert_eq!(b2.next.0.as_i64().unwrap(), 4);

        // Third fetch: nothing new, exhausted, cursor unchanged.
        let b3 = src.fetch_since(Some(&b2.next)).await.unwrap();
        assert!(b3.items.is_empty());
        assert!(b3.exhausted);
        assert_eq!(
            b3.next.0.as_i64().unwrap(),
            4,
            "cursor holds at the last id"
        );

        let _ = std::fs::remove_file(&db);
    }

    #[tokio::test]
    async fn unavailable_when_db_missing_or_not_screenpipe() {
        let src = ScreenpipeTail::new("/nonexistent/homn-screenpipe-tail-nope.sqlite");
        let err = src.fetch_since(None).await.unwrap_err();
        assert!(matches!(err, SourceError::Unavailable(_)));

        // A DB without the frames table is also unavailable.
        let db = tmp_db("notscreenpipe");
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (x TEXT);")
            .unwrap();
        let err = src_fetch(&src, &db).await;
        assert!(matches!(err, SourceError::Unavailable(_)), "{err:?}");
        let _ = std::fs::remove_file(&db);
    }

    async fn src_fetch(_src: &ScreenpipeTail, db: &Path) -> SourceError {
        let s = ScreenpipeTail::new(db);
        s.fetch_since(None).await.unwrap_err()
    }

    #[test]
    fn cursor_decode_handles_none_and_garbage() {
        assert_eq!(ScreenpipeTail::last_id(None), 0);
        assert_eq!(
            ScreenpipeTail::last_id(Some(&Cursor::new(serde_json::json!(42)))),
            42
        );
        // Non-integer cursor → 0 (re-read from start; dedupe collapses the replay).
        assert_eq!(
            ScreenpipeTail::last_id(Some(&Cursor::new(serde_json::json!("oops")))),
            0
        );
    }

    /// Tail the *real* Screenpipe capture DB. Needs `SCREENPIPE_DB` pointing at one, so it
    /// can't run in CI — run manually:
    /// `SCREENPIPE_DB=~/.screenpipe/db.sqlite cargo test -p homn-sources -- --ignored`
    #[tokio::test]
    #[ignore = "needs a real Screenpipe DB via SCREENPIPE_DB"]
    async fn tails_a_real_screenpipe_db() {
        let db = std::path::PathBuf::from(
            std::env::var("SCREENPIPE_DB").expect("set SCREENPIPE_DB to a Screenpipe sqlite file"),
        );
        let src = ScreenpipeTail::new(&db).with_batch_size(50);
        let b = src.fetch_since(None).await.expect("tail the real DB");
        assert!(!b.items.is_empty(), "a real capture DB has OCR frames");
        assert_eq!(b.items[0].source, SourceKind::ScreenOcr);
        assert!(!b.items[0].text.is_empty());
        assert!(
            b.items[0].upstream_ref.parse::<i64>().is_ok(),
            "upstream_ref is the frame id"
        );
    }
}
