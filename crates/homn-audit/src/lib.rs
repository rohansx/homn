//! SQLite-backed audit log for `homn` decisions.
//!
//! Owns the schema, migrations, and the writer task. The query API for `homn log` lands in
//! later tasks (T027); this module exposes only what T015 promises: a `Db` you can open with
//! migrations applied, plus low-level inspection helpers used by tests.
//!
//! See [`docs/technical/audit-log.md`](../../../docs/technical/audit-log.md) and
//! [`specs/001-policy-engine/data-model.md`](../../../specs/001-policy-engine/data-model.md).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

use tokio_rusqlite::Connection;

/// Current schema version shipped with this crate. Bump and add a new migration when changing
/// the schema.
pub const SCHEMA_VERSION: i64 = 1;

const MIGRATION_0001: &str = include_str!("../migrations/0001_initial.sql");

/// Ordered list of `(version, sql)` migrations applied at startup. Append-only.
const MIGRATIONS: &[(i64, &str)] = &[(1, MIGRATION_0001)];

/// Audit database handle.
///
/// Holds a single-writer SQLite connection (Tokio-aware via `tokio-rusqlite`). Migrations are
/// applied automatically when the database is opened.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open the audit database at the given path, applying any pending migrations.
    ///
    /// The path is created if it doesn't exist; its parent directory must already exist.
    pub async fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(path.as_ref()).await?;
        set_pragmas(&conn, /*persistent*/ true).await?;
        run_migrations(&conn).await?;
        Ok(Self { conn })
    }

    /// Open an in-memory audit database — useful for tests.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().await?;
        set_pragmas(&conn, /*persistent*/ false).await?;
        run_migrations(&conn).await?;
        Ok(Self { conn })
    }

    /// Borrow the underlying connection — primarily for tests and (later) the writer task.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Return the schema version recorded in the database.
    pub async fn current_version(&self) -> anyhow::Result<i64> {
        let v: i64 = self
            .conn
            .call(|c| {
                let row: i64 = c.query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                    [],
                    |r| r.get(0),
                )?;
                Ok(row)
            })
            .await?;
        Ok(v)
    }

    /// Return the names of all user tables in the database (for inspection / tests).
    pub async fn table_names(&self) -> anyhow::Result<Vec<String>> {
        let names: Vec<String> = self
            .conn
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(names)
            })
            .await?;
        Ok(names)
    }
}

async fn set_pragmas(conn: &Connection, persistent: bool) -> anyhow::Result<()> {
    conn.call(move |c| {
        // PRAGMAs run as plain statements (NOT inside a transaction).
        c.pragma_update(None, "foreign_keys", "ON")?;
        c.pragma_update(None, "temp_store", "MEMORY")?;
        if persistent {
            // WAL is only meaningful for file-backed DBs.
            c.pragma_update(None, "journal_mode", "WAL")?;
            c.pragma_update(None, "synchronous", "NORMAL")?;
        }
        Ok(())
    })
    .await?;
    Ok(())
}

async fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    // Ensure the bookkeeping table exists.
    conn.call(|c| {
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version    INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
             );",
        )?;
        Ok(())
    })
    .await?;

    // Find the highest applied version.
    let current: i64 = conn
        .call(|c| {
            let v: i64 = c.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |r| r.get(0),
            )?;
            Ok(v)
        })
        .await?;

    for (version, sql) in MIGRATIONS.iter().copied() {
        if version > current {
            tracing::info!(target: "homn_audit::migrate", version, "applying migration");
            let sql_owned = sql.to_owned();
            conn.call(move |c| {
                let tx = c.transaction()?;
                tx.execute_batch(&sql_owned)?;
                tx.execute(
                    "INSERT INTO schema_version (version, applied_at)
                     VALUES (?, strftime('%s', 'now') * 1000)",
                    [version],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_in_memory_applies_migrations() {
        let db = Db::in_memory().await.unwrap();
        assert_eq!(db.current_version().await.unwrap(), SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn migrations_create_expected_tables() {
        let db = Db::in_memory().await.unwrap();
        let tables = db.table_names().await.unwrap();
        for expected in ["decisions", "decisions_fts", "schema_version"] {
            assert!(
                tables.iter().any(|t| t == expected),
                "expected table `{expected}` to exist; got {tables:?}"
            );
        }
    }

    #[tokio::test]
    async fn decisions_check_constraints_reject_bad_values() {
        let db = Db::in_memory().await.unwrap();
        let err = db
            .conn
            .call(|c| {
                c.execute(
                    "INSERT INTO decisions (ts, session_id, cwd, tool_name, tool_input, decision, latency_ms, source)
                     VALUES (1, 's', '/', 'Bash', '{}', 'bogus', 1, 'hook')",
                    [],
                )?;
                Ok(())
            })
            .await;
        assert!(err.is_err(), "expected CHECK constraint to reject bad decision value");
    }

    #[tokio::test]
    async fn fts_index_picks_up_inserts() {
        let db = Db::in_memory().await.unwrap();
        db.conn
            .call(|c| {
                c.execute(
                    "INSERT INTO decisions
                       (ts, session_id, cwd, tool_name, tool_input, decision, latency_ms, source)
                     VALUES (1, 's', '/home/x', 'Bash', '{\"command\":\"npm install foo\"}', 'allow', 1, 'hook')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let hits: i64 = db
            .conn
            .call(|c| {
                let v: i64 = c.query_row(
                    "SELECT COUNT(*) FROM decisions_fts WHERE decisions_fts MATCH 'foo'",
                    [],
                    |r| r.get(0),
                )?;
                Ok(v)
            })
            .await
            .unwrap();
        assert_eq!(hits, 1);
    }

    #[tokio::test]
    async fn double_open_is_idempotent() {
        // Opening twice on the same in-memory connection isn't possible (separate connections
        // see separate databases), but opening the SAME path twice via a tempdir works.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");

        {
            let db = Db::open(&path).await.unwrap();
            assert_eq!(db.current_version().await.unwrap(), SCHEMA_VERSION);
        }
        {
            let db = Db::open(&path).await.unwrap();
            assert_eq!(
                db.current_version().await.unwrap(),
                SCHEMA_VERSION,
                "re-opening should not re-apply migrations"
            );
        }
    }
}
