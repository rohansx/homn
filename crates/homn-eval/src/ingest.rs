//! Throwaway Phase-0 replay-ingest: Screenpipe sqlite → naive chunks → `agidb.observe` (T018).
//!
//! Reads OCR/audio rows out of a Screenpipe capture DB, groups consecutive rows per app+minute,
//! and stores each chunk as one agidb observation. Also provides [`AgidbRecaller`], the
//! [`crate::score::Recaller`] backed by `agidb.recall_cue`, so `homn eval run` can score a real
//! store. Own data only, no redaction, cloud OFF — this code dies after the Phase 0 gate run.
//!
//! Screenpipe's schema drifts between versions, so table/column names are introspected from
//! `sqlite_master` at runtime and overridable via [`IngestConfig`].

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::Recaller;
    use rusqlite::Connection;

    /// A unique temp sqlite file per test (std-only; no tempfile dep).
    fn test_db(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "homn-eval-ingest-{}-{name}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn row(app: &str, minute: &str, text: &str) -> RawRow {
        RawRow {
            app: app.to_owned(),
            minute: minute.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn minute_key_truncates_iso_timestamps() {
        let v = rusqlite::types::Value::Text("2026-07-10T12:34:56.789Z".to_owned());
        assert_eq!(minute_key(&v).as_deref(), Some("2026-07-10T12:34"));
    }

    #[test]
    fn minute_key_buckets_unix_seconds_and_millis() {
        let secs = rusqlite::types::Value::Integer(1_752_150_000);
        let millis = rusqlite::types::Value::Integer(1_752_150_000_000);
        assert_eq!(minute_key(&secs), minute_key(&millis));
        assert!(minute_key(&secs).is_some());
    }

    #[test]
    fn minute_key_fails_closed_on_null() {
        assert_eq!(minute_key(&rusqlite::types::Value::Null), None);
    }

    #[test]
    fn chunk_rows_groups_consecutive_app_minute_runs() {
        let chunks = chunk_rows(&[
            row("Code", "m1", "fn main"),
            row("Code", "m1", "cargo test"),
            row("Code", "m2", "still coding"),
            row("Firefox", "m2", "docs page"),
        ]);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].app, "Code");
        assert_eq!(chunks[0].text, "fn main\ncargo test");
        assert_eq!(chunks[1].text, "still coding");
        assert_eq!(chunks[2].app, "Firefox");
    }

    #[test]
    fn chunk_rows_skips_blank_and_consecutive_duplicate_texts() {
        let chunks = chunk_rows(&[
            row("Code", "m1", "hello world"),
            row("Code", "m1", "hello world"), // OCR repeat → collapsed
            row("Code", "m1", "   "),         // blank → skipped
            row("Code", "m1", "new line"),
        ]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world\nnew line");
    }

    /// Build a Screenpipe-shaped DB: OCR text hangs off `frames` (join needed for the
    /// timestamp), audio carries its own timestamp.
    fn write_screenpipe_fixture(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE frames (id INTEGER PRIMARY KEY, timestamp TEXT);
            CREATE TABLE ocr_text (frame_id INTEGER, text TEXT, app_name TEXT);
            CREATE TABLE audio_transcriptions (
                id INTEGER PRIMARY KEY, transcription TEXT, timestamp TEXT, device TEXT
            );
            INSERT INTO frames VALUES (1, '2026-07-10T09:00:01Z');
            INSERT INTO frames VALUES (2, '2026-07-10T09:00:30Z');
            INSERT INTO frames VALUES (3, '2026-07-10T09:02:00Z');
            INSERT INTO ocr_text VALUES (1, 'hello world', 'Code');
            INSERT INTO ocr_text VALUES (2, 'hello world', 'Code');
            INSERT INTO ocr_text VALUES (3, 'other page', 'Firefox');
            INSERT INTO audio_transcriptions VALUES (1, 'let us ship it', '2026-07-10T09:00:10Z', 'mic');
            "#,
        )
        .unwrap();
    }

    #[test]
    fn read_chunks_introspects_a_screenpipe_shaped_db() {
        let db = test_db("introspect");
        write_screenpipe_fixture(&db);

        let sources = read_chunks(&db, &IngestConfig::default()).unwrap();
        let ocr = sources.iter().find(|(l, _)| l == "ocr").expect("ocr source");
        let audio = sources
            .iter()
            .find(|(l, _)| l == "audio")
            .expect("audio source");

        // Duplicate OCR text collapsed; app change splits the chunk.
        assert_eq!(ocr.1.len(), 2);
        assert_eq!(ocr.1[0].app, "Code");
        assert_eq!(ocr.1[0].minute, "2026-07-10T09:00");
        assert_eq!(ocr.1[0].text, "hello world");
        assert_eq!(ocr.1[1].app, "Firefox");

        assert_eq!(audio.1.len(), 1);
        assert_eq!(audio.1[0].text, "let us ship it");

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn read_chunks_fails_closed_on_a_db_with_no_known_tables() {
        let db = test_db("nosources");
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (x TEXT);")
            .unwrap();
        assert!(matches!(
            read_chunks(&db, &IngestConfig::default()),
            Err(IngestError::NoSources(_))
        ));
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn config_overrides_beat_introspection() {
        let db = test_db("override");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE my_ocr (body TEXT, at TEXT, program TEXT);
            INSERT INTO my_ocr VALUES ('custom text', '2026-07-10T10:00:00Z', 'Zed');
            "#,
        )
        .unwrap();
        drop(conn);

        let cfg = IngestConfig {
            ocr: SourceOverride {
                table: Some("my_ocr".to_owned()),
                text_col: Some("body".to_owned()),
                ts_col: Some("at".to_owned()),
                app_col: Some("program".to_owned()),
            },
            audio: SourceOverride::default(),
        };
        let sources = read_chunks(&db, &cfg).unwrap();
        let ocr = sources.iter().find(|(l, _)| l == "ocr").expect("ocr source");
        assert_eq!(ocr.1[0].app, "Zed");
        assert_eq!(ocr.1[0].text, "custom text");
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn agidb_recaller_round_trips_an_observation() {
        let root = std::env::temp_dir().join(format!(
            "homn-eval-recaller-{}.agidb",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);

        // Observe with a short-lived handle, then reopen through the recaller.
        {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let brain = rt
                .block_on(agidb::Agidb::open_with(
                    agidb::AgidbConfig::new(&root).with_extractor(agidb::ExtractorSetup::Null),
                ))
                .unwrap();
            rt.block_on(brain.observe_with(
                "Sarah promised the quote by Friday",
                "screenpipe:ocr",
            ))
            .unwrap();
            rt.block_on(brain.flush()).unwrap();
        }

        let recaller = AgidbRecaller::open(&root).unwrap();
        let hits = recaller.recall("what did Sarah promise", 3);
        assert!(!hits.is_empty(), "recall must surface the observation");
        assert!(
            hits.iter().any(|h| h.text.contains("quote by Friday")),
            "hit text carries the observed sentence: {hits:?}"
        );

        drop(recaller);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The full replay against a *real* Screenpipe capture DB. Needs `SCREENPIPE_DB` pointing at
    /// one, so it can't run in CI — run manually before the gate run:
    /// `SCREENPIPE_DB=~/.screenpipe/db.sqlite cargo test -p homn-eval --features brain-agidb -- --ignored`
    #[test]
    #[ignore = "needs a real Screenpipe DB via SCREENPIPE_DB"]
    fn replay_ingest_against_a_real_screenpipe_db() {
        let db = std::path::PathBuf::from(
            std::env::var("SCREENPIPE_DB").expect("set SCREENPIPE_DB to a Screenpipe sqlite file"),
        );
        let root = std::env::temp_dir().join(format!(
            "homn-eval-replay-{}.agidb",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let brain = rt
            .block_on(agidb::Agidb::open_with(
                agidb::AgidbConfig::new(&root).with_extractor(agidb::ExtractorSetup::Null),
            ))
            .unwrap();
        let report = rt
            .block_on(replay_ingest(&db, &brain, &IngestConfig::default()))
            .unwrap();
        assert!(report.rows_read > 0, "a real capture DB has rows");
        assert!(report.chunks_stored > 0, "chunks must land in the store");
    }
}
