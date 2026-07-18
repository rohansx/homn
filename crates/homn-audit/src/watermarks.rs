//! Source watermark persistence (specs/002-ambient-memory, R-3).
//!
//! Each capture source has one row here: its last durably-consumed [`Cursor`](homn_types::Cursor),
//! serialized to canonical JSON. The daemon advances it **only** after an item is stored or
//! durably dropped by policy, so a crash re-reads from here and dedupe collapses the replay (R7).
//! See [`contracts/gate-pipeline.md`] §"Watermark after durability".

use chrono::Utc;
use homn_types::{Cursor, Watermark};

use crate::Db;

impl Db {
    /// Read the watermark for `source_id`, or `None` if this source has never been consumed.
    pub async fn get_watermark(&self, source_id: &str) -> anyhow::Result<Option<Watermark>> {
        let source_id = source_id.to_owned();
        let row: Option<(String, String, String)> = self
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT source_id, cursor, updated_at FROM watermarks WHERE source_id = ?",
                )?;
                let row = stmt
                    .query_row([source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .ok();
                Ok(row)
            })
            .await?;
        match row {
            None => Ok(None),
            Some((source_id, cursor_json, updated_at)) => {
                let cursor: Cursor =
                    serde_json::from_str(&cursor_json).map_err(|e| anyhow::anyhow!(e))?;
                let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                Ok(Some(Watermark {
                    source_id,
                    cursor,
                    updated_at,
                }))
            }
        }
    }

    /// Upsert the watermark for `source_id`. Called only after the batch it represents has been
    /// durably stored (or durably dropped by policy).
    pub async fn set_watermark(&self, source_id: &str, cursor: &Cursor) -> anyhow::Result<()> {
        let source_id = source_id.to_owned();
        let cursor_json = serde_json::to_string(cursor)?;
        let now = Utc::now().to_rfc3339();
        self.conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO watermarks (source_id, cursor, updated_at) VALUES (?, ?, ?)
                     ON CONFLICT(source_id) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at",
                    rusqlite::params![source_id, cursor_json, now],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn watermark_round_trips_and_upserts() {
        let db = Db::in_memory().await.unwrap();
        // None before any write.
        assert!(db.get_watermark("ocr").await.unwrap().is_none());

        db.set_watermark("ocr", &Cursor::new(42i64)).await.unwrap();
        let w = db.get_watermark("ocr").await.unwrap().expect("present");
        assert_eq!(w.cursor, Cursor::new(42i64));

        // Upsert advances, does not duplicate.
        db.set_watermark("ocr", &Cursor::new(100i64)).await.unwrap();
        let w = db.get_watermark("ocr").await.unwrap().expect("present");
        assert_eq!(w.cursor, Cursor::new(100i64));

        // A second source gets its own row.
        db.set_watermark("dict", &Cursor::new(7i64)).await.unwrap();
        let w = db.get_watermark("dict").await.unwrap().expect("present");
        assert_eq!(w.cursor, Cursor::new(7i64));
        let w = db
            .get_watermark("ocr")
            .await
            .unwrap()
            .expect("still present");
        assert_eq!(w.cursor, Cursor::new(100i64));
    }

    #[tokio::test]
    async fn watermark_persists_opaque_json_cursor() {
        // A poll-cursor shape (Gmail history id as a struct) must survive the round trip — the
        // daemon never interprets the cursor, only the source does.
        let db = Db::in_memory().await.unwrap();
        let cursor = Cursor::new(serde_json::json!({"history_id": 1234567890, "page": 2}));
        db.set_watermark("gmail", &cursor).await.unwrap();
        let back = db.get_watermark("gmail").await.unwrap().expect("present");
        assert_eq!(back.cursor, cursor);
    }
}
