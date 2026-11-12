//! SQLite storage for financi.
//!
//! - [`open`] opens (or creates) a database, enables WAL and foreign keys, and
//!   applies pending migrations.
//! - [`DbWorker`] owns the connections on worker threads — a writer and, for
//!   file databases, a read-only reader — so imports and queries never block
//!   the GPUI thread.
//! - [`seed`] (feature `dev-seed`) fills a database with synthetic data.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

mod worker;
pub use worker::{DbWorker, WorkerGone, on_db_typed};

pub mod accounts;
pub use accounts::{
    Account, Institution, create_account, get_account, list_accounts, list_institutions,
};

pub mod categories;

pub mod import;

pub mod master_csv;

pub mod overrides;
pub use overrides::{apply_rule_from_merchant, clear_override, set_category, set_hidden};

pub mod queue;
pub use queue::{UncategorizedGroup, categorize_group, uncategorized_by_merchant};

pub mod rules;
pub use rules::{add_rule, delete_rule, list_rules, update_rule};

pub mod settings;

pub mod transfers;
pub use transfers::{
    PairingSummary, TransferCandidate, TransferReview, list_transfer_reviews,
    resolve_transfer_review, run_transfer_pairing, unpair_transaction,
};

pub mod views;
pub use views::{
    CategorySpend, FilterSql, MonthlySummary, MonthlySummaryRow, TxSort, TxSortColumn,
    TxTableFilter, ViewFilter, dashboard, list_transactions_filtered, monthly_summary,
    spend_by_category,
};

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
    #[error("the database job panicked; the worker survived")]
    JobPanicked,
    #[error("job superseded by a newer one with the same key")]
    Superseded,
    #[error(transparent)]
    WorkerGone(#[from] worker::WorkerGone),
    #[error("unknown institution {0:?}")]
    UnknownInstitution(String),
    #[error("no parser with id {0:?}")]
    UnknownParser(String),
    #[error("no account with id {0}")]
    UnknownAccount(i64),
    #[error("an account named {name:?} already exists at this institution")]
    DuplicateAccount { name: String },
    #[error("no category with id {0}")]
    UnknownCategory(i64),
    #[error("a category named {name:?} already exists under the same category")]
    DuplicateCategory { name: String },
    #[error("{0} is a category; only subcategories can hold transactions and rules")]
    ParentCategoryNotAssignable(String),
    #[error("only subcategories can be moved; {0} is a category")]
    CannotMoveParent(String),
    #[error("{0} is a subcategory; subcategories cannot have subcategories")]
    CategoryHasParent(String),
    #[error("{0} still has subcategories; delete or move them first")]
    CategoryHasChildren(String),
    #[error("cannot move subcategory {child:?} under category {parent:?}: the kinds differ")]
    CategoryKindMismatch { child: String, parent: String },
    #[error("category {id} is a system category and cannot be {action}")]
    SystemCategory { id: i64, action: &'static str },
    #[error("the system category {0:?} is missing from the database")]
    MissingSystemCategory(&'static str),
    #[error("cannot reassign category {0} to itself")]
    CategoryReassignToSelf(i64),
    #[error("no rule with id {0}")]
    UnknownRule(i64),
    #[error("no transaction with id {0}")]
    UnknownTransaction(i64),
    #[error("transaction {0} has no merchant to build a rule from")]
    EmptyMerchant(i64),
    #[error("a rule needs a non-empty pattern")]
    EmptyRulePattern,
    #[error("invalid regex pattern {pattern:?}: {message}")]
    InvalidRulePattern { pattern: String, message: String },
    #[error("transaction {0} is not paired with anything")]
    NotPaired(i64),
    #[error("transaction {0} has no open transfer review")]
    NoTransferReview(i64),
    #[error("transaction {candidate} is not a pairable candidate for transaction {transaction}")]
    NotTransferCandidate { transaction: i64, candidate: i64 },
}

pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// Ordered migrations. Index + 1 is the schema version (`PRAGMA user_version`).
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_categorize.sql"),
    include_str!("../migrations/0003_transfers.sql"),
    include_str!("../migrations/0004_replacements.sql"),
    include_str!("../migrations/0005_perf_indexes.sql"),
];

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
    optimize(&conn)?;
    Ok(conn)
}

/// An in-memory database with the full schema, for tests.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&mut conn)?;
    optimize(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    // WAL is a no-op (stays "memory") for in-memory databases. It is what
    // lets the reader thread query while the writer thread writes (3.77).
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(worker::BUSY_TIMEOUT)?;
    Ok(())
}

/// Refresh the query planner's statistics after migrations (3.77). Cheap
/// when nothing changed.
fn optimize(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA optimize;")?;
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

/// Spend per top-level category (parents roll up their children), largest
/// first. Transfers and hidden rows are excluded; refunds net against spend.
///
/// 3.88: not a dashboard query any more — the dashboard uses
/// [`crate::views::dashboard`]. Kept only for the tests as an independent
/// reference implementation of the spend rules, with its own hand-written
/// conditions to check the shared [`FilterSql`] builder against.
#[cfg(test)]
pub fn spend_by_parent_category(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(p.name, c.name, 'Uncategorized') AS cat,
                -SUM(t.amount_cents) AS spend
           FROM transactions t
           LEFT JOIN categories c ON c.id = t.category_id
           LEFT JOIN categories p ON p.id = c.parent_id
          WHERE t.hidden = 0
            AND COALESCE(c.kind, 'expense') = 'expense'
          GROUP BY cat
          ORDER BY spend DESC",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Sidebar numbers: `(visible transactions, uncategorized visible
/// transactions)` — the footer count and the To categorize pill. Counts
/// every non-hidden row whatever the dashboard filter (transfers included),
/// so the conditions come from the shared [`FilterSql`] builder with an
/// open, transfers-inclusive filter — one query, not two.
pub fn sidebar_counts(conn: &Connection) -> Result<(i64, i64)> {
    let open = views::FilterSql::new(
        &views::ViewFilter {
            include_transfers: true,
            ..Default::default()
        },
        None,
    );
    let sql = format!(
        "SELECT COUNT(*), COALESCE(SUM(t.category_id IS NULL), 0)
           FROM transactions t
           LEFT JOIN categories c ON c.id = t.category_id
          WHERE 1 = 1{}",
        open.sql
    );
    let (total, uncategorized) =
        conn.query_row(&sql, rusqlite::params_from_iter(open.args), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
    Ok((total, uncategorized))
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
    fn perf_indexes_exist() {
        // 3.73: undo resolves `transfer_pair_id` links per deleted row,
        // category reassign/counts/filters look rows up by `category_id`,
        // and the transactions table's default order is
        // (posted_date, import_id, source_line). All three must exist.
        let conn = open_in_memory().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT name, sql FROM sqlite_master
                  WHERE type = 'index' AND tbl_name = 'transactions'
                    AND name IN ('tx_transfer_pair', 'tx_category', 'tx_date_import_line')",
            )
            .unwrap();
        let mut indexes: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        indexes.sort();
        assert_eq!(indexes.len(), 3, "all three indexes exist: {indexes:?}");
        assert_eq!(indexes[0].0, "tx_category");
        assert!(indexes[0].1.contains("(category_id)"));
        assert_eq!(indexes[1].0, "tx_date_import_line");
        assert!(
            indexes[1]
                .1
                .contains("(posted_date, import_id, source_line)")
        );
        assert_eq!(indexes[2].0, "tx_transfer_pair");
        assert!(indexes[2].1.contains("WHERE transfer_pair_id IS NOT NULL"));
    }

    #[test]
    fn sidebar_counts_see_visible_rows_only() {
        let conn = open_in_memory().unwrap();
        let account = crate::accounts::create_account(
            &conn,
            "amex",
            &uuid::Uuid::new_v4().to_string(),
            financi_core::AccountKind::CreditCard,
        )
        .unwrap();
        let parent =
            crate::categories::create_parent(&conn, "Food", financi_core::CategoryKind::Expense)
                .unwrap();
        let child = crate::categories::create_child(&conn, parent, "Groceries").unwrap();
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'activity.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 3)",
            [account],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        let add = |category: Option<i64>, hidden: i64| {
            conn.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, category_id, hidden)
                 VALUES (?1, ?2, 0, X'00', ?3, '2026-10-31', -450, 'SHOP', ?4, ?5)",
                (
                    account,
                    import,
                    uuid::Uuid::new_v4().to_string(),
                    category,
                    hidden,
                ),
            )
            .unwrap();
        };
        add(None, 0);
        add(None, 0);
        add(Some(child), 0);
        add(None, 1);

        assert_eq!(sidebar_counts(&conn).unwrap(), (3, 2));
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
    fn fresh_database_has_only_the_system_transfer_categories() {
        let conn = open_in_memory().unwrap();
        let rows = conn
            .prepare(
                "SELECT c.name, p.name, c.kind, c.is_system FROM categories c
                   LEFT JOIN categories p ON p.id = c.parent_id
                  ORDER BY c.name",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let transfers = Some("Transfers".to_string());
        assert_eq!(
            rows,
            vec![
                (
                    "Between Accounts".to_string(),
                    transfers.clone(),
                    "transfer".to_string(),
                    1
                ),
                (
                    "Card Payment".to_string(),
                    transfers.clone(),
                    "transfer".to_string(),
                    1
                ),
                ("Transfers".to_string(), None, "transfer".to_string(), 1),
            ]
        );
        // The 0002 columns exist (is_system was exercised above via the query).
        let has_is_regex: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('category_rules')
                  WHERE name = 'is_regex'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_is_regex, 1);
    }

    #[test]
    fn migrating_a_version_one_database_keeps_existing_transactions() {
        // Build a database the way the M1 build left them: only 0001 applied,
        // user_version = 1, and a transaction already stored.
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1u32).unwrap();

        let acct = create_account(
            &conn,
            "amex",
            "Cobalt",
            financi_core::AccountKind::CreditCard,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'activity.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [acct],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, 0, X'00', 'h1', '2026-10-31', -450, 'COFFEE')",
            (acct, import),
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let v: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions
                  WHERE row_hash = 'h1' AND amount_cents = -450 AND description_raw = 'COFFEE'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "pre-existing transactions survive the migration");
        let system: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE is_system = 1 AND kind = 'transfer'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            system, 3,
            "the migration inserts exactly the 3 system categories"
        );
    }

    #[test]
    fn top_level_category_names_are_unique() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO categories (name, kind) VALUES ('Food', 'expense')",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO categories (name, kind) VALUES ('Food', 'expense')",
            [],
        );
        assert!(dup.is_err());
    }

    #[test]
    fn expense_kind_round_trips_as_expense() {
        use financi_core::CategoryKind;

        let conn = open_in_memory().unwrap();
        let id = crate::categories::create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let stored: String = conn
            .query_row("SELECT kind FROM categories WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, "expense");

        let tree = crate::categories::list_categories(&conn).unwrap();
        let food = tree.iter().find(|p| p.name == "Food").unwrap();
        assert_eq!(food.kind, CategoryKind::Expense);
    }

    #[test]
    fn category_kind_check_rejects_the_old_spend_value() {
        let conn = open_in_memory().unwrap();
        let old = conn.execute(
            "INSERT INTO categories (name, kind) VALUES ('Food', 'spend')",
            [],
        );
        assert!(old.is_err(), "the CHECK no longer allows 'spend'");
    }

    #[test]
    fn undo_import_cascades_to_transactions_and_aliases() {
        let conn = open_in_memory().unwrap();
        let acct =
            create_account(&conn, "amex", "Test", financi_core::AccountKind::CreditCard).unwrap();
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
