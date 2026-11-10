//! Import pipeline: plan (preview) -> commit, plus undo.
//!
//! Dedup follows the plan's "Import and deduplication" section:
//! - each row's hash comes from its parser over raw identity fields plus the
//!   account's stable key;
//! - count reconciliation: if the file holds n rows with a hash and the
//!   account already holds m (transactions plus aliases), the last n - m
//!   occurrences in file order are new. Real identical repeats survive and
//!   overlapping exports never double-count.
//!
//! [`commit_import`] re-plans inside its own transaction, so a stale preview
//! can never insert duplicates. New rows are categorized in the same
//! transaction: built-in card-payment detection first, then the user rules.

use std::collections::{BTreeSet, HashMap};

use financi_core::rules::pick_category;
use financi_core::{ParseError, RawFile, RawTransaction, RowHash, StatementParser};
use rusqlite::{Connection, params};

use crate::categories::card_payment_category;
use crate::rules::list_rules;
use crate::{Account, DbError, get_account, list_accounts};

/// Whether to import even though some rows already exist under a different
/// account. Off by default: the user must explicitly override.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub allow_other_account_overlap: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("this is a {parser} export, but the account belongs to {account}")]
    WrongInstitution { parser: String, account: String },
    #[error("this export comes from {expected} accounts, but the account is a {actual} account")]
    WrongAccountKind { expected: String, actual: String },
    #[error("{} row(s) could not be read; first: {}", .0.len(), .0[0])]
    Parse(Vec<ParseError>),
    #[error(
        "{matching_rows} row(s) in this file already exist in {account}. \
         Check you picked the right account, or override to import anyway."
    )]
    OtherAccountOverlap {
        account: String,
        matching_rows: usize,
    },
}

impl From<rusqlite::Error> for ImportError {
    fn from(e: rusqlite::Error) -> Self {
        ImportError::Db(e.into())
    }
}

#[derive(Debug, Clone)]
pub struct PreparedRow {
    pub source_line: usize,
    pub raw_bytes: Vec<u8>,
    pub row_hash: RowHash,
    pub txn: RawTransaction,
}

#[derive(Debug, Clone)]
pub struct OverlapWarning {
    pub account: Account,
    /// File rows whose hash (under that account's key) is already there.
    pub matching_rows: usize,
}

/// What an import would do. Nothing is written.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub account: Account,
    pub parser_id: &'static str,
    pub file_name: String,
    pub total_rows: usize,
    pub new_rows: Vec<PreparedRow>,
    pub already_present: usize,
    /// Earliest and latest posted date in the file (ISO).
    pub date_range: (String, String),
    pub overlap: Option<OverlapWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    /// `None` when every row was already present (nothing to record or undo).
    pub import_id: Option<i64>,
    pub inserted: usize,
    pub already_present: usize,
    /// Calendar years of the inserted rows, whose master CSVs need rewriting.
    pub affected_years: BTreeSet<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoneImport {
    pub import_id: i64,
    pub file_name: String,
    pub removed: usize,
    /// Calendar years of the removed rows, whose master CSVs need rewriting.
    pub affected_years: BTreeSet<i32>,
}

/// Map and hash every row, then reconcile against the account.
pub fn plan_import(
    conn: &Connection,
    account_id: i64,
    parser: &dyn StatementParser,
    file: &RawFile,
    file_name: &str,
) -> Result<ImportPlan, ImportError> {
    let account = get_account(conn, account_id)?;
    if parser.institution() != account.institution.key {
        return Err(ImportError::WrongInstitution {
            parser: parser.institution().to_string(),
            account: account.institution.name.clone(),
        });
    }
    if !parser.account_kinds().contains(&account.kind) {
        let expected: Vec<_> = parser.account_kinds().iter().map(|k| k.label()).collect();
        return Err(ImportError::WrongAccountKind {
            expected: expected.join(" or "),
            actual: account.kind.label().to_string(),
        });
    }

    let mut rows = Vec::with_capacity(file.records.len());
    let mut errors = Vec::new();
    for record in &file.records {
        match parser.map(record) {
            Ok(txn) => rows.push(PreparedRow {
                source_line: record.source_line,
                raw_bytes: record.bytes.clone(),
                row_hash: parser.row_hash(&account.account_key, record),
                txn,
            }),
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(ImportError::Parse(errors));
    }

    let total_rows = rows.len();
    let date_range = date_range(&rows);
    let new_rows = reconcile(rows, &existing_hash_counts(conn, account.id)?);
    let already_present = total_rows - new_rows.len();
    let overlap = other_account_overlap(conn, &account, parser, file)?;

    Ok(ImportPlan {
        account,
        parser_id: parser.id(),
        file_name: file_name.to_string(),
        total_rows,
        new_rows,
        already_present,
        date_range,
        overlap,
    })
}

/// Write an import in one transaction. Re-plans inside the transaction, so
/// the counts reflect the database at commit time. Refuses when rows already
/// exist under another account, unless `options` overrides it.
pub fn commit_import(
    conn: &mut Connection,
    account_id: i64,
    parser: &dyn StatementParser,
    file: &RawFile,
    file_name: &str,
    imported_at: &str,
    options: CommitOptions,
) -> Result<ImportOutcome, ImportError> {
    let tx = conn.transaction()?;
    let plan = plan_import(&tx, account_id, parser, file, file_name)?;
    if let Some(overlap) = &plan.overlap
        && !options.allow_other_account_overlap
    {
        return Err(ImportError::OtherAccountOverlap {
            account: overlap.account.display_name(),
            matching_rows: overlap.matching_rows,
        });
    }
    if plan.new_rows.is_empty() {
        return Ok(ImportOutcome {
            import_id: None,
            inserted: 0,
            already_present: plan.already_present,
            affected_years: BTreeSet::new(),
        });
    }

    tx.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            account_id,
            plan.parser_id,
            file_name,
            imported_at,
            file.header_raw,
            file.encoding.as_str(),
            file.has_bom,
            file.line_ending.as_str(),
            file.trailing_newline,
            file.records.len() as i64,
        ],
    )?;
    let import_id = tx.last_insert_rowid();
    let categories = categorize_new_rows(&tx, parser, &plan.new_rows)?;
    {
        let mut insert = tx.prepare(
            "INSERT INTO transactions
               (account_id, import_id, source_line, raw_bytes, row_hash, posted_date,
                processed_date, amount_cents, original_currency, original_amount_cents,
                description_raw, merchant, category_id, category_locked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?;
        for (row, (category_id, category_locked)) in plan.new_rows.iter().zip(&categories) {
            let t = &row.txn;
            insert.execute(params![
                account_id,
                import_id,
                row.source_line as i64,
                row.raw_bytes,
                row.row_hash.0,
                t.posted_date,
                t.processed_date,
                t.amount_cents,
                t.original_currency,
                t.original_amount_cents,
                t.description_raw,
                t.merchant,
                category_id,
                *category_locked,
            ])?;
        }
    }
    // Transfer pairing (4.2) runs automatically after every import, in
    // this same transaction: new rows and their older unpaired partners
    // are matched before anything else can observe the import.
    crate::transfers::run_transfer_pairing(&tx)?;
    tx.commit()?;
    Ok(ImportOutcome {
        import_id: Some(import_id),
        inserted: plan.new_rows.len(),
        already_present: plan.already_present,
        affected_years: years_of(plan.new_rows.iter().map(|row| row.txn.posted_date.as_str())),
    })
}

/// The `(category_id, category_locked)` pair for each new row, in order.
/// The parser's built-in card-payment check runs first and beats user
/// rules: those rows get `Transfers > Card Payment` and are locked, so
/// re-running rules on rule changes never moves them. Every other row takes
/// the winning rule, or stays `NULL` (Uncategorized).
fn categorize_new_rows(
    conn: &Connection,
    parser: &dyn StatementParser,
    rows: &[PreparedRow],
) -> Result<Vec<(Option<i64>, bool)>, ImportError> {
    let rules = list_rules(conn)?;
    let mut card_payment: Option<i64> = None;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if parser.is_card_payment(&row.txn) {
            let id = match card_payment {
                Some(id) => id,
                None => {
                    let id = card_payment_category(conn)?;
                    card_payment = Some(id);
                    id
                }
            };
            out.push((Some(id), true));
        } else {
            out.push((pick_category(&rules, &row.txn.description_raw), false));
        }
    }
    Ok(out)
}

/// Hard-delete the most recent import of an account, with its rows and
/// aliases (via cascade). `None` if the account has no imports.
pub fn undo_last_import(
    conn: &mut Connection,
    account_id: i64,
) -> Result<Option<UndoneImport>, DbError> {
    let tx = conn.transaction()?;
    let last: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, file_name FROM imports WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
            [account_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    let Some((import_id, file_name)) = last else {
        return Ok(None);
    };
    let removed: usize = tx.query_row(
        "SELECT COUNT(*) FROM transactions WHERE import_id = ?1",
        [import_id],
        |r| r.get(0),
    )?;
    let affected_years: BTreeSet<i32> = {
        let mut stmt = tx.prepare("SELECT posted_date FROM transactions WHERE import_id = ?1")?;
        let dates = stmt.query_map([import_id], |r| r.get::<_, String>(0))?;
        years_of(dates.filter_map(|r| r.ok()))
    };
    tx.execute("DELETE FROM imports WHERE id = ?1", [import_id])?;
    tx.commit()?;
    Ok(Some(UndoneImport {
        import_id,
        file_name,
        removed,
        affected_years,
    }))
}

/// The calendar years present in a sequence of ISO `yyyy-mm-dd` dates.
fn years_of<I, S>(dates: I) -> BTreeSet<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    dates
        .into_iter()
        .filter_map(|date| {
            let date = date.as_ref();
            date.get(0..4).and_then(|year| year.parse().ok())
        })
        .collect()
}

/// How many times each hash is already present in the account, counting
/// aliases left by confirmed near-matches.
fn existing_hash_counts(
    conn: &Connection,
    account_id: i64,
) -> Result<HashMap<String, usize>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT row_hash FROM transactions WHERE account_id = ?1
         UNION ALL
         SELECT a.row_hash FROM row_aliases a
           JOIN transactions t ON t.id = a.transaction_id
          WHERE t.account_id = ?1",
    )?;
    let mut counts = HashMap::new();
    for hash in stmt.query_map([account_id], |r| r.get::<_, String>(0))? {
        *counts.entry(hash?).or_insert(0) += 1;
    }
    Ok(counts)
}

/// Keep, per hash, only the occurrences beyond what already exists. The
/// earliest occurrences in file order are the ones treated as present.
fn reconcile(rows: Vec<PreparedRow>, existing: &HashMap<String, usize>) -> Vec<PreparedRow> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    rows.into_iter()
        .filter(|row| {
            let n = seen.entry(row.row_hash.0.clone()).or_insert(0);
            *n += 1;
            *n > existing.get(&row.row_hash.0).copied().unwrap_or(0)
        })
        .collect()
}

fn date_range(rows: &[PreparedRow]) -> (String, String) {
    let dates = rows.iter().map(|r| &r.txn.posted_date);
    let lo = dates.clone().min().cloned().unwrap_or_default();
    let hi = dates.max().cloned().unwrap_or_default();
    (lo, hi)
}

/// The other account holding the most of this file's rows, if it holds any.
/// Even one matching row blocks the import unless overridden.
fn other_account_overlap(
    conn: &Connection,
    target: &Account,
    parser: &dyn StatementParser,
    file: &RawFile,
) -> Result<Option<OverlapWarning>, DbError> {
    let mut best: Option<OverlapWarning> = None;
    for other in list_accounts(conn)?
        .into_iter()
        .filter(|a| a.id != target.id)
    {
        let mut existing = existing_hash_counts(conn, other.id)?;
        if existing.is_empty() {
            continue;
        }
        let matching_rows = file
            .records
            .iter()
            .filter(|r| {
                let h = parser.row_hash(&other.account_key, r).0;
                match existing.get_mut(&h) {
                    Some(n) if *n > 0 => {
                        *n -= 1;
                        true
                    }
                    _ => false,
                }
            })
            .count();
        if best
            .as_ref()
            .is_none_or(|b| matching_rows > b.matching_rows)
        {
            best = Some(OverlapWarning {
                account: other,
                matching_rows,
            });
        }
    }
    Ok(best.filter(|b| b.matching_rows > 0))
}

/// One row of the Imports page history: an import still in the database
/// (an undone import disappears from `imports` with its rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEntry {
    pub id: i64,
    pub account_id: i64,
    /// `Institution · Name`, as shown in the table.
    pub account: String,
    pub file_name: String,
    /// Stored ISO timestamp.
    pub imported_at: String,
    /// Transactions this import currently owns.
    pub rows_added: i64,
    /// ISO dates of those rows, `None` when the import owns none left.
    pub first_date: Option<String>,
    pub last_date: Option<String>,
}

/// Recorded imports, newest first.
pub fn list_imports(conn: &Connection) -> Result<Vec<ImportEntry>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT im.id, im.account_id, i.name || ' · ' || a.name, im.file_name, im.imported_at,
                COUNT(t.id), MIN(t.posted_date), MAX(t.posted_date)
           FROM imports im
           JOIN accounts a ON a.id = im.account_id
           JOIN institutions i ON i.id = a.institution_id
           LEFT JOIN transactions t ON t.import_id = im.id
          GROUP BY im.id
          ORDER BY im.id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ImportEntry {
            id: r.get(0)?,
            account_id: r.get(1)?,
            account: r.get(2)?,
            file_name: r.get(3)?,
            imported_at: r.get(4)?,
            rows_added: r.get(5)?,
            first_date: r.get(6)?,
            last_date: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A transaction as the browse table shows it. `category_id` and `hidden`
/// drive the row context menu (2.9): the first marks the current category
/// `checked` in the *Change category…* submenu, the second decides between
/// *Hide* and *Unhide*. `list_transactions` filters hidden rows out, so
/// `hidden` is `false` for every row it returns until the "show hidden"
/// toggle (3.4) widens the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRow {
    pub id: i64,
    pub account: String,
    pub posted_date: String,
    pub description: String,
    pub merchant: Option<String>,
    pub category: Option<String>,
    pub category_id: Option<i64>,
    pub amount_cents: i64,
    pub hidden: bool,
    /// The row this one is paired with as a transfer (4.2), `Some` for
    /// paired rows — the table draws a link icon and offers *Unpair*.
    pub transfer_pair_id: Option<i64>,
}

/// Transactions newest first, optionally for one account. Hidden rows are
/// left out.
pub fn list_transactions(
    conn: &Connection,
    account_id: Option<i64>,
) -> Result<Vec<TransactionRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT t.id, i.name || ' · ' || a.name, t.posted_date, t.description_raw, t.merchant,
                c.name, t.amount_cents, t.category_id, t.hidden, t.transfer_pair_id
           FROM transactions t
           JOIN accounts a ON a.id = t.account_id
           JOIN institutions i ON i.id = a.institution_id
           LEFT JOIN categories c ON c.id = t.category_id
          WHERE t.hidden = 0 AND (?1 IS NULL OR t.account_id = ?1)
          ORDER BY t.posted_date DESC, t.import_id DESC, t.source_line",
    )?;
    let rows = stmt.query_map([account_id], |r| {
        Ok(TransactionRow {
            id: r.get(0)?,
            account: r.get(1)?,
            posted_date: r.get(2)?,
            description: r.get(3)?,
            merchant: r.get(4)?,
            category: r.get(5)?,
            amount_cents: r.get(6)?,
            category_id: r.get(7)?,
            hidden: r.get(8)?,
            transfer_pair_id: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
