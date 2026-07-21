//! Throwaway Phase-0 replay-ingest: Screenpipe sqlite → naive chunks → `agidb.observe` (T018).
//!
//! Reads OCR/audio rows out of a Screenpipe capture DB, groups consecutive rows per app+minute,
//! and stores each chunk as one agidb observation. Also provides [`AgidbRecaller`], the
//! [`crate::score::Recaller`] backed by `agidb.recall_cue`, so `homn eval run` can score a real
//! store. Own data only, no redaction, cloud OFF — this code dies after the Phase 0 gate run.
//!
//! Screenpipe's schema drifts between versions, so table/column names are introspected from
//! `sqlite_master` at runtime and overridable via [`IngestConfig`].
//!
//! Two known shapes are supported by introspection:
//! - **v1 fixture**: `frames(id, timestamp)` + `ocr_text(frame_id, text, app_name)` (join) +
//!   `audio_transcriptions(id, transcription, timestamp, device)`.
//! - **v2 real**: `frames(id, timestamp, app_name, full_text)` (denormalized) +
//!   `audio_transcriptions(...)` + optional `elements(frame_id, text, source)`.
//!
//! Explicit [`IngestConfig`] overrides win over introspection in all cases.

use std::path::{Path, PathBuf};

use rusqlite::types::Value as SqlValue;

use crate::score::{Hit, Recaller};

/// One pre-chunk row read from a capture source: the app, the minute bucket, and the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRow {
    /// App / window / account name, if the source provides one.
    pub app: String,
    /// Minute-granularity bucket key (e.g. `2026-07-10T09:00`), from [`minute_key`].
    pub minute: String,
    /// The captured text (OCR line, audio transcription, …).
    pub text: String,
}

/// Per-source column overrides. Any `None` field falls back to introspection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceOverride {
    /// Table name. If set, a direct `SELECT <text>, <ts>, <app> FROM <table>` is used (no join).
    pub table: Option<String>,
    /// Column holding the text.
    pub text_col: Option<String>,
    /// Column holding the timestamp.
    pub ts_col: Option<String>,
    /// Column holding the app name.
    pub app_col: Option<String>,
}

/// introspection + override config for [`read_chunks`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestConfig {
    /// OCR / screen-text source.
    pub ocr: SourceOverride,
    /// Audio transcription source.
    pub audio: SourceOverride,
    /// Optional convox-voice `dictation.jsonl` path. `None` auto-discovers
    /// `~/.local/share/convox-voice/dictation.jsonl` (the intentional, push-to-talk speech
    /// sense — far cleaner than ambient mic capture, which is background-media noise).
    pub dictation_path: Option<PathBuf>,
}

/// Errors from reading or replaying a capture DB.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    /// No recognisable source table was found in the DB (and no override was given).
    #[error("no capture sources found in {0}")]
    NoSources(String),
    /// A sqlite operation failed.
    #[error("sqlite: {0}")]
    Sqlite(String),
    /// An agidb operation failed.
    #[error("agidb: {0}")]
    Agidb(String),
}

impl From<rusqlite::Error> for IngestError {
    fn from(e: rusqlite::Error) -> Self {
        IngestError::Sqlite(e.to_string())
    }
}

/// Bucket a sqlite timestamp value into a minute-granularity key.
///
/// - ISO-8601 text (`2026-07-10T12:34:56.789Z`) → `2026-07-10T12:34`.
/// - Unix seconds or millis (integer) → the UTC minute of that instant.
/// - Anything else (including NULL) → `None` (fail closed; the row is skipped).
pub fn minute_key(v: &SqlValue) -> Option<String> {
    match v {
        SqlValue::Text(s) => {
            // Truncate at the 16th char: "YYYY-MM-DDTHH:MM".
            if s.len() >= 16 {
                Some(s[..16].to_owned())
            } else {
                None
            }
        }
        SqlValue::Integer(n) => {
            // Heuristic: if the number is large enough to be millis, divide by 1000.
            let secs = if *n > 9_999_999_999 { n / 1000 } else { *n };
            use chrono::{TimeZone, Utc};
            let dt = Utc.timestamp_opt(secs, 0).single()?;
            Some(dt.format("%Y-%m-%dT%H:%M").to_string())
        }
        _ => None,
    }
}

/// Collapse raw rows into chunks: consecutive rows sharing `(app, minute)` become one chunk,
/// with their text joined by `\n`. Within a run, blank text and consecutive duplicate text are
/// dropped (OCR repeats). Returns one [`RawRow`] per chunk.
pub fn chunk_rows(rows: &[RawRow]) -> Vec<RawRow> {
    let mut out: Vec<RawRow> = Vec::new();
    for row in rows {
        let text = row.text.trim();
        if text.is_empty() {
            continue;
        }
        // Same (app, minute) as the current chunk → fold in (dedupe consecutive identical text).
        if let Some(last) = out.last() {
            if last.app == row.app && last.minute == row.minute {
                if !last.text.ends_with(text) && !text_already_present(&last.text, text) {
                    let mut merged = last.clone();
                    merged.text = format!("{}\n{}", last.text, text);
                    *out.last_mut().expect("just checked non-empty") = merged;
                }
                continue;
            }
        }
        out.push(RawRow {
            app: row.app.clone(),
            minute: row.minute.clone(),
            text: text.to_owned(),
        });
    }
    out
}

/// True if `text` already appears as a full line in `haystack` (consecutive-duplicate guard).
fn text_already_present(haystack: &str, text: &str) -> bool {
    haystack.split('\n').any(|line| line == text)
}

/// Read raw (pre-chunk) rows out of a capture DB, per source label.
///
/// Returns `Vec<(label, rows)>` in a stable order (`ocr` first, then `audio`). Each row carries
/// its minute bucket and app. Fails closed with [`IngestError::NoSources`] if no recognisable
/// source table exists and no override is configured.
pub fn read_rows(
    db: &Path,
    cfg: &IngestConfig,
) -> Result<Vec<(&'static str, Vec<RawRow>)>, IngestError> {
    let conn = rusqlite::Connection::open(db)
        .map_err(|e| IngestError::Sqlite(format!("open {}: {e}", db.display())))?;
    let mut out: Vec<(&'static str, Vec<RawRow>)> = Vec::new();

    if let Some(rows) = read_ocr(&conn, &cfg.ocr, db)? {
        out.push(("ocr", rows));
    }
    if let Some(rows) = read_audio(&conn, &cfg.audio)? {
        out.push(("audio", rows));
    }
    if let Some(rows) = read_dictation(cfg)? {
        out.push(("dictation", rows));
    }

    if out.is_empty() {
        return Err(IngestError::NoSources(db.display().to_string()));
    }
    Ok(out)
}

/// Read + chunk in one call (the common path for inspection / tests).
pub fn read_chunks(
    db: &Path,
    cfg: &IngestConfig,
) -> Result<Vec<(&'static str, Vec<RawRow>)>, IngestError> {
    let raw = read_rows(db, cfg)?;
    Ok(raw
        .into_iter()
        .map(|(label, rows)| (label, chunk_rows(&rows)))
        .collect())
}

fn read_ocr(
    conn: &rusqlite::Connection,
    ov: &SourceOverride,
    db: &Path,
) -> Result<Option<Vec<RawRow>>, IngestError> {
    // Explicit override: direct SELECT, no join.
    if let Some(table) = &ov.table {
        let text_col = ov.text_col.as_deref().unwrap_or("text");
        let ts_col = ov.ts_col.as_deref().unwrap_or("timestamp");
        let app_col = ov.app_col.as_deref().unwrap_or("app_name");
        let sql = format!("SELECT {text_col}, {ts_col}, {app_col} FROM {table} ORDER BY {ts_col}");
        return Ok(Some(query_rows(conn, &sql, &|row| {
            let text: String = row.get(0).unwrap_or_default();
            let ts: SqlValue = row.get(1).unwrap_or(SqlValue::Null);
            let app: String = row.get(2).unwrap_or_default();
            Some(RawRow {
                app,
                minute: minute_key(&ts)?,
                text,
            })
        })?));
    }

    // Introspect: v1 fixture shape (ocr_text JOIN frames).
    if has_table(conn, "ocr_text") && has_table(conn, "frames") {
        let sql = "SELECT ocr_text.text, frames.timestamp, ocr_text.app_name \
                   FROM ocr_text JOIN frames ON ocr_text.frame_id = frames.id \
                   ORDER BY frames.id";
        return Ok(Some(query_rows(conn, sql, &|row| {
            let text: String = row.get(0).unwrap_or_default();
            let ts: SqlValue = row.get(1).unwrap_or(SqlValue::Null);
            let app: String = row.get(2).unwrap_or_default();
            Some(RawRow {
                app,
                minute: minute_key(&ts)?,
                text,
            })
        })?));
    }

    // Introspect: v2 real shape (frames.full_text denormalized).
    if has_table(conn, "frames") && has_column(conn, "frames", "full_text") {
        let sql = "SELECT full_text, app_name, timestamp FROM frames \
                   WHERE full_text IS NOT NULL AND full_text != '' ORDER BY id";
        return Ok(Some(query_rows(conn, sql, &|row| {
            let text: String = row.get(0).unwrap_or_default();
            let app: String = row.get(1).unwrap_or_default();
            let ts: SqlValue = row.get(2).unwrap_or(SqlValue::Null);
            Some(RawRow {
                app,
                minute: minute_key(&ts)?,
                text,
            })
        })?));
    }

    // No OCR source recognised. Not an error yet — audio alone is still a valid capture.
    let _ = db;
    Ok(None)
}

fn read_audio(
    conn: &rusqlite::Connection,
    ov: &SourceOverride,
) -> Result<Option<Vec<RawRow>>, IngestError> {
    if let Some(table) = &ov.table {
        let text_col = ov.text_col.as_deref().unwrap_or("transcription");
        let ts_col = ov.ts_col.as_deref().unwrap_or("timestamp");
        let app_col = ov.app_col.as_deref().unwrap_or("device");
        let sql = format!("SELECT {text_col}, {ts_col}, {app_col} FROM {table} ORDER BY {ts_col}");
        return Ok(Some(query_rows(conn, &sql, &|row| {
            let text: String = row.get(0).unwrap_or_default();
            let ts: SqlValue = row.get(1).unwrap_or(SqlValue::Null);
            let app: String = row.get(2).unwrap_or_default();
            Some(RawRow {
                app,
                minute: minute_key(&ts)?,
                text,
            })
        })?));
    }

    if has_table(conn, "audio_transcriptions") {
        let sql = "SELECT transcription, timestamp, device FROM audio_transcriptions \
                   WHERE transcription IS NOT NULL AND transcription != '' ORDER BY id";
        return Ok(Some(query_rows(conn, sql, &|row| {
            let text: String = row.get(0).unwrap_or_default();
            let ts: SqlValue = row.get(1).unwrap_or(SqlValue::Null);
            let app: String = row.get(2).unwrap_or_default();
            Some(RawRow {
                app,
                minute: minute_key(&ts)?,
                text,
            })
        })?));
    }

    Ok(None)
}

/// Read convox-voice's `dictation.jsonl` (one `{"ts","text"}` per line) into rows. Each line is
/// one intentional, push-to-talk utterance → one row. `app` is `"dictation"`; the minute bucket
/// comes from the `ts` field. Auto-discovers the default path when `cfg.dictation_path` is None.
fn read_dictation(cfg: &IngestConfig) -> Result<Option<Vec<RawRow>>, IngestError> {
    let path: PathBuf = match cfg.dictation_path.clone() {
        Some(p) => p,
        None => {
            // ~/.local/share/convox-voice/dictation.jsonl (XDG data home, with fallback).
            let base = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
                });
            base.join("convox-voice").join("dictation.jsonl")
        }
    };
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| IngestError::Sqlite(format!("read {}: {e}", path.display())))?;
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed lines (fail closed per-row, not whole-file)
        };
        let body = v.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        let ts = v.get("ts").and_then(|t| t.as_str()).unwrap_or("");
        let minute = minute_key(&SqlValue::Text(ts.to_owned())).unwrap_or_default();
        if minute.is_empty() {
            continue;
        }
        rows.push(RawRow {
            app: "dictation".to_owned(),
            minute,
            text: body.to_owned(),
        });
    }
    Ok(Some(rows))
}

/// Run `sql` and collect rows where the mapper returns `Some`.
fn query_rows(
    conn: &rusqlite::Connection,
    sql: &str,
    map: &dyn Fn(&rusqlite::Row) -> Option<RawRow>,
) -> Result<Vec<RawRow>, IngestError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |row| Ok(map(row)))?
        .filter_map(|r| r.ok())
        .flatten()
        .collect();
    Ok(rows)
}

fn has_table(conn: &rusqlite::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

fn has_column(conn: &rusqlite::Connection, table: &str, col: &str) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect();
    names.iter().any(|n| n == col)
}

/// Counters from a replay run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Raw rows read from the capture DB (pre-chunk).
    pub rows_read: u64,
    /// Chunks actually stored in the brain.
    pub chunks_stored: u64,
}

/// Replay a capture DB into an agidb brain: read rows, chunk, and `observe_with` each chunk
/// under its source label. Own data only, no redaction, cloud OFF (Phase 0 throwaway).
pub async fn replay_ingest(
    db: &Path,
    brain: &agidb::Agidb,
    cfg: &IngestConfig,
) -> Result<ReplayReport, IngestError> {
    let raw = read_rows(db, cfg)?;
    let mut rows_read: u64 = 0;
    let mut chunks_stored: u64 = 0;
    for (label, rows) in raw {
        rows_read += rows.len() as u64;
        for chunk in chunk_rows(&rows) {
            // Stamp the episode with the capture time (chunk.minute) as valid_time — not
            // ingest time — so bi-temporal / time-window recall can filter by when the
            // activity actually happened. Falls back to now() if the minute won't parse.
            let valid_time = parse_minute(&chunk.minute).unwrap_or_else(chrono::Utc::now);
            let ctx = agidb::ObserveContext {
                observation_time: valid_time,
                provenance: agidb::Provenance {
                    source: label.to_owned(),
                    ..agidb::Provenance::default()
                },
            };
            brain
                .observe_with_context(&chunk.text, ctx)
                .await
                .map_err(|e| IngestError::Agidb(e.to_string()))?;
            chunks_stored += 1;
        }
    }
    Ok(ReplayReport {
        rows_read,
        chunks_stored,
    })
}

/// Parse a `chunk.minute` bucket ("YYYY-MM-DDTHH:MM") into a UTC instant, appending :00Z.
fn parse_minute(minute: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(&format!("{minute}:00Z"))
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// A [`Recaller`] backed by an agidb brain, for `homn eval run`.
///
/// Bridges the sync [`Recaller::recall`] trait to agidb's async `recall_cue`. Because the trait
/// is sync, this holds a private Tokio runtime. When `recall` is invoked *inside* an existing
/// Tokio runtime (e.g. the `homn` CLI's `#[tokio::main]`), it uses `block_in_place` on the
/// current handle rather than nesting a second `block_on` (which would panic); when invoked
/// from plain sync code (e.g. a unit test), it falls back to its own runtime.
pub struct AgidbRecaller {
    brain: std::sync::Arc<agidb::Agidb>,
    rt: std::sync::Arc<tokio::runtime::Runtime>,
    #[allow(dead_code)]
    _root: PathBuf,
}

/// Run `f` to completion. If a Tokio runtime is already running on this thread, use `block_in_place`
/// on its handle (multi-threaded runtimes only); otherwise drive `f` on `rt`.
fn block_on_with<F>(rt: &tokio::runtime::Runtime, f: F) -> F::Output
where
    F: std::future::Future,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(f)),
        Err(_) => rt.block_on(f),
    }
}

impl AgidbRecaller {
    /// Open an existing agidb brain (read-only recall; the Null extractor is fine for scoring).
    pub fn open(root: impl AsRef<Path>) -> Result<Self, IngestError> {
        let root = root.as_ref().to_path_buf();
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| IngestError::Agidb(format!("runtime: {e}")))?;
        let brain = block_on_with(&rt, async {
            agidb::Agidb::open_with(
                agidb::AgidbConfig::new(&root).with_extractor(agidb::ExtractorSetup::Null),
            )
            .await
        })
        .map_err(|e| IngestError::Agidb(e.to_string()))?;
        Ok(Self {
            brain: std::sync::Arc::new(brain),
            rt: std::sync::Arc::new(rt),
            _root: root,
        })
    }
}

impl Recaller for AgidbRecaller {
    fn recall(&self, cue: &str, k: usize) -> Vec<Hit> {
        let brain = self.brain.clone();
        let cue = cue.to_owned();
        let result = block_on_with(&self.rt, async move { brain.recall_cue(cue).await });
        to_hits(result, k)
    }

    fn recall_with_window(
        &self,
        cue: &str,
        from: Option<&str>,
        to: Option<&str>,
        k: usize,
    ) -> Vec<Hit> {
        let Some(from) = from else {
            return self.recall(cue, k);
        };
        let Some(to) = to else {
            return self.recall(cue, k);
        };
        let (Ok(from_dt), Ok(to_dt)) = (
            chrono::DateTime::parse_from_rfc3339(from).map(|d| d.with_timezone(&chrono::Utc)),
            chrono::DateTime::parse_from_rfc3339(to).map(|d| d.with_timezone(&chrono::Utc)),
        ) else {
            return self.recall(cue, k);
        };
        let brain = self.brain.clone();
        let cue = cue.to_owned();
        let query = agidb::Query::cue(cue)
            .with_time_window(from_dt, to_dt)
            .with_k(k);
        let result = block_on_with(&self.rt, async move { brain.recall(query).await });
        to_hits(result, k)
    }
}

/// Map an agidb `Recall` result into scored [`Hit`]s, taking the top `k`.
fn to_hits(result: Result<agidb::Recall, agidb::core::AgidbError>, k: usize) -> Vec<Hit> {
    match result {
        Ok(recall) => recall
            .matches
            .into_iter()
            .take(k)
            .map(|m| Hit {
                reference: format!("ep-{}", m.episode_id),
                text: m.text,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

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
        let ocr = sources
            .iter()
            .find(|(l, _)| *l == "ocr")
            .expect("ocr source");
        let audio = sources
            .iter()
            .find(|(l, _)| *l == "audio")
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
            dictation_path: None,
        };
        let sources = read_chunks(&db, &cfg).unwrap();
        let ocr = sources
            .iter()
            .find(|(l, _)| *l == "ocr")
            .expect("ocr source");
        assert_eq!(ocr.1[0].app, "Zed");
        assert_eq!(ocr.1[0].text, "custom text");
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn read_dictation_parses_convox_voice_jsonl() {
        let path =
            std::env::temp_dir().join(format!("homn-eval-dictation-{}.jsonl", std::process::id()));
        std::fs::write(
            &path,
            "{\"ts\":\"2026-07-18T19:04:28+00:00\",\"text\":\"ship the eval gate today\"}\n\
             {\"ts\":\"2026-07-18T19:05:00+00:00\",\"text\":\"\"}\n\
             not-json-at-all\n\
             {\"ts\":\"2026-07-18T19:06:30+00:00\",\"text\":\"  spliced words  \"}\n",
        )
        .unwrap();
        let cfg = IngestConfig {
            dictation_path: Some(path.clone()),
            ..Default::default()
        };
        let rows = read_dictation(&cfg).unwrap().expect("parsed some rows");
        assert_eq!(rows.len(), 2, "blank + malformed lines skipped");
        assert_eq!(rows[0].app, "dictation");
        assert_eq!(rows[0].minute, "2026-07-18T19:04");
        assert_eq!(rows[0].text, "ship the eval gate today");
        assert_eq!(rows[1].text, "spliced words", "trimmed");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_dictation_returns_none_when_path_absent() {
        let cfg = IngestConfig {
            dictation_path: Some(std::env::temp_dir().join("homn-eval-nope-does-not-exist.jsonl")),
            ..Default::default()
        };
        assert!(read_dictation(&cfg).unwrap().is_none());
    }

    #[test]
    fn agidb_recaller_round_trips_an_observation() {
        let root =
            std::env::temp_dir().join(format!("homn-eval-recaller-{}.agidb", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        // Observe with a short-lived handle, then reopen through the recaller.
        {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let brain = rt
                .block_on(agidb::Agidb::open_with(
                    agidb::AgidbConfig::new(&root).with_extractor(agidb::ExtractorSetup::Null),
                ))
                .unwrap();
            rt.block_on(brain.observe_with("Sarah promised the quote by Friday", "screenpipe:ocr"))
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
        let root =
            std::env::temp_dir().join(format!("homn-eval-replay-{}.agidb", std::process::id()));
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
