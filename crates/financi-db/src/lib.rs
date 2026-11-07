//! SQLite storage for financi.
//!
//! - [`open`] opens (or creates) a database, enables WAL and foreign keys, and
//!   applies pending migrations.
//! - [`DbWorker`] owns the connection on its own thread, so imports and
//!   queries never block the GPUI thread.
//! - [`seed`] (feature `dev-seed`) fills a database with synthetic data.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

mod worker;
pub use worker::{DbWorker, WorkerGone};

#[cfg(feature = "dev-seed")]
pub mod seed;

pub use rusqlite;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not determine an app data directory for this platform")]
    NoDataDir,
    #[error("io error creating {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    TooNew { found: u32, supported: u32 },
}

pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// Ordered migrations. Index + 1 is the schema version (`PRAGMA user_version`).
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// `~/Library/Application Support/financi/financi.sqlite` on macOS,
/// `$XDG_DATA_HOME/financi/financi.sqlite` on Linux.
pub fn default_db_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "financi").ok_or(DbError::NoDataDir)?;
    Ok(dirs.data_dir().join("financi.sqlite"))
}

/// Open the database at `path`, creating parent directories and applying
/// migrations as needed.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DbError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// An in-memory database with the full schema, for tests.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    // WAL is a no-op (stays "memory") for in-memory databases.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

/// Apply every migration newer than the database's `user_version`, each in
/// its own transaction.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(DbError::TooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for (ix, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (ix + 1) as u32)?;
        tx.commit()?;
    }
    Ok(())
}

/// Create an account with a fresh stable `account_key`. Returns its row id.
pub fn create_account(conn: &Connection, institution: &str, name: &str, kind: &str) -> Result<i64> {
    let key = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO accounts (account_key, institution, name, kind) VALUES (?1, ?2, ?3, ?4)",
        (&key, institution, name, kind),
    )?;
    Ok(conn.last_insert_rowid())
}

/// Spend per top-level category (parents roll up their children), largest
/// first. Transfers and hidden rows are excluded; refunds net against spend.
pub fn spend_by_parent_category(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(p.name, c.name, 'Uncategorized') AS cat,
                -SUM(t.amount_cents) AS spend
           FROM transactions t
           LEFT JOIN categories c ON c.id = t.category_id
           LEFT JOIN categories p ON p.id = c.parent_id
          WHERE t.hidden = 0
            AND COALESCE(c.kind, 'spend') = 'spend'
          GROUP BY cat
          ORDER BY spend DESC",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_fresh_database_to_latest() {
        let conn = open_in_memory().unwrap();
        let v: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let fk: bool = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(fk);
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut conn = open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
    }

    #[test]
    fn rejects_newer_schema() {
        let mut conn = open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(migrate(&mut conn), Err(DbError::TooNew { .. })));
    }

    #[test]
    fn file_database_uses_wal() {
        let dir = std::env::temp_dir().join(format!("financi-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("nested").join("financi.sqlite");
        let conn = open(&path).unwrap();
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn top_level_category_names_are_unique() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO categories (name, kind) VALUES ('Food', 'spend')",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO categories (name, kind) VALUES ('Food', 'spend')",
            [],
        );
        assert!(dup.is_err());
    }

    #[test]
    fn undo_import_cascades_to_transactions_and_aliases() {
        let conn = open_in_memory().unwrap();
        let acct = create_account(&conn, "amex", "Amex Test", "credit").unwrap();
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'activity.csv', '2026-11-07', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [acct],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, 0, X'00', 'h1', '2026-11-05', -450, 'COFFEE')",
            (acct, import),
        )
        .unwrap();
        let tx = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO row_aliases (transaction_id, row_hash, import_id) VALUES (?1, 'h0', ?2)",
            (tx, import),
        )
        .unwrap();

        conn.execute("DELETE FROM imports WHERE id = ?1", [import])
            .unwrap();

        let left: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM transactions) + (SELECT COUNT(*) FROM row_aliases)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }
}
