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

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::NaiveDate;
use financi_core::rules::CompiledRules;
use financi_core::{
    AccountKind, ParseError, RawFile, RawTransaction, RowHash, StatementParser, parse,
};
use rusqlite::{Connection, params};

use crate::categories::card_payment_category;
use crate::overrides::clear_override;
use crate::rules::list_rules;
use crate::{Account, DbError, get_account, list_accounts};

/// Whether to import even though some rows already exist under a different
/// account. Off by default: the user must explicitly override.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub allow_other_account_overlap: bool,
    /// Near-match pairs (4.3) the user confirmed as "Same charge" in the
    /// preview, keyed by `(source_line of the file row, existing transaction
    /// id)` — the same pair the preview listed. Any other near-match stays
    /// "Different charge" and imports as new.
    pub same_charges: Vec<(usize, i64)>,
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
    /// Suspected changed charges to review (4.3), one entry per new row
    /// that looks like a re-export of an existing transaction.
    pub near_matches: Vec<NearMatch>,
}

/// One suspected pair from the near-match review (4.3): a new row that
/// could be the same charge as an existing transaction whose raw fields
/// changed (e.g. a tip added), so its hash differs and count reconciliation
/// cannot see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearMatch {
    /// Index of the new row in [`ImportPlan::new_rows`].
    pub new_row: usize,
    /// Its record index in the source file — part of the stable
    /// preview/commit key.
    pub source_line: usize,
    pub new_date: String,
    pub new_amount_cents: i64,
    pub new_description_raw: String,
    /// The existing transaction it may replace.
    pub existing_id: i64,
    pub existing_date: String,
    pub existing_amount_cents: i64,
    pub existing_description_raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    /// `None` when every row was already present (nothing to record or undo).
    pub import_id: Option<i64>,
    pub inserted: usize,
    /// Near-match replacements applied (4.3): the existing transactions
    /// overwritten by rows the user confirmed as "Same charge".
    pub replaced: usize,
    pub already_present: usize,
    /// Calendar years of the inserted rows, whose master CSVs need rewriting.
    /// A replacement contributes both its old and its new date's year.
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
    // Every hash the file carries, reconciled or new: the near-match review
    // skips existing rows the file still holds unchanged (3.68).
    let file_hashes: HashSet<String> = rows.iter().map(|r| r.row_hash.0.clone()).collect();
    let new_rows = reconcile(rows, &existing_hash_counts(conn, account.id)?);
    let already_present = total_rows - new_rows.len();
    let overlap = other_account_overlap(conn, &account, parser, file)?;
    let near_matches = find_near_matches(conn, account.id, &new_rows, &file_hashes, file)?;

    Ok(ImportPlan {
        account,
        parser_id: parser.id(),
        file_name: file_name.to_string(),
        total_rows,
        new_rows,
        already_present,
        date_range,
        overlap,
        near_matches,
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
            replaced: 0,
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
    // Near-match review (4.3): rows the user marked "Same charge" overwrite
    // their existing pair instead of being inserted. The pair keys come from
    // the preview, so only matches still present in the fresh plan apply.
    let confirmed: HashSet<(usize, i64)> = options.same_charges.iter().copied().collect();
    let replacements: Vec<&NearMatch> = plan
        .near_matches
        .iter()
        .filter(|m| confirmed.contains(&(m.source_line, m.existing_id)))
        .collect();
    let replacing: HashSet<usize> = replacements.iter().map(|m| m.new_row).collect();

    let categories = categorize_new_rows(&tx, parser, plan.account.kind, &plan.new_rows)?;
    {
        let mut insert = tx.prepare(
            "INSERT INTO transactions
               (account_id, import_id, source_line, raw_bytes, row_hash, posted_date,
                processed_date, amount_cents, original_currency, original_amount_cents,
                description_raw, merchant, category_id, category_locked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?;
        for (ix, (row, (category_id, category_locked))) in
            plan.new_rows.iter().zip(&categories).enumerate()
        {
            if replacing.contains(&ix) {
                continue;
            }
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
    let mut affected_years = years_of(
        plan.new_rows
            .iter()
            .enumerate()
            .filter(|(ix, _)| !replacing.contains(ix))
            .map(|(_, row)| row.txn.posted_date.as_str()),
    );
    for m in &replacements {
        let old_posted_date =
            apply_replacement(&tx, import_id, &plan.new_rows[m.new_row], m.existing_id)?;
        // The old row leaves its year's master CSV, the new values join the
        // new date's (often the same year).
        affected_years.extend(years_of([
            old_posted_date.as_str(),
            plan.new_rows[m.new_row].txn.posted_date.as_str(),
        ]));
    }
    // Transfer pairing (4.2) runs automatically after every import, in
    // this same transaction: new rows and their older unpaired partners
    // are matched before anything else can observe the import. Running it
    // after the replacements means pairing sees the final dates/amounts.
    crate::transfers::run_transfer_pairing(&tx)?;
    tx.commit()?;
    Ok(ImportOutcome {
        import_id: Some(import_id),
        inserted: plan.new_rows.len() - replacements.len(),
        replaced: replacements.len(),
        already_present: plan.already_present,
        affected_years,
    })
}

/// Apply one confirmed near-match (4.3): keep the existing transaction's id
/// and category (and its import), overwrite its date/amount/description/
/// merchant/raw bytes/`row_hash`/FX columns with the new row's, add the old
/// hash to `row_aliases` under the new import — so later overlapping exports
/// reconcile against it silently — and record the old values in
/// `replacements` so undo can restore them. Returns the old posted date.
fn apply_replacement(
    tx: &Connection,
    import_id: i64,
    row: &PreparedRow,
    existing_id: i64,
) -> Result<String, ImportError> {
    struct OldValues {
        raw_bytes: Vec<u8>,
        row_hash: String,
        posted_date: String,
        processed_date: Option<String>,
        amount_cents: i64,
        description_raw: String,
        merchant: Option<String>,
        original_currency: Option<String>,
        original_amount_cents: Option<i64>,
    }
    let old: OldValues = tx.query_row(
        "SELECT raw_bytes, row_hash, posted_date, processed_date, amount_cents,
                description_raw, merchant, original_currency, original_amount_cents
           FROM transactions WHERE id = ?1",
        [existing_id],
        |r| {
            Ok(OldValues {
                raw_bytes: r.get(0)?,
                row_hash: r.get(1)?,
                posted_date: r.get(2)?,
                processed_date: r.get(3)?,
                amount_cents: r.get(4)?,
                description_raw: r.get(5)?,
                merchant: r.get(6)?,
                original_currency: r.get(7)?,
                original_amount_cents: r.get(8)?,
            })
        },
    )?;
    tx.execute(
        "INSERT INTO replacements
           (import_id, transaction_id, old_raw_bytes, old_row_hash, old_posted_date,
            old_processed_date, old_amount_cents, old_description_raw, old_merchant,
            old_original_currency, old_original_amount_cents)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            import_id,
            existing_id,
            old.raw_bytes,
            old.row_hash,
            old.posted_date,
            old.processed_date,
            old.amount_cents,
            old.description_raw,
            old.merchant,
            old.original_currency,
            old.original_amount_cents,
        ],
    )?;
    tx.execute(
        "INSERT INTO row_aliases (transaction_id, row_hash, import_id) VALUES (?1, ?2, ?3)",
        params![existing_id, old.row_hash, import_id],
    )?;
    let t = &row.txn;
    tx.execute(
        "UPDATE transactions
            SET posted_date = ?2, processed_date = ?3, amount_cents = ?4,
                description_raw = ?5, merchant = ?6, raw_bytes = ?7, row_hash = ?8,
                original_currency = ?9, original_amount_cents = ?10
          WHERE id = ?1",
        params![
            existing_id,
            t.posted_date,
            t.processed_date,
            t.amount_cents,
            t.description_raw,
            t.merchant,
            row.raw_bytes,
            row.row_hash.0,
            t.original_currency,
            t.original_amount_cents,
        ],
    )?;
    Ok(old.posted_date)
}

/// The `(category_id, category_locked)` pair for each new row, in order.
/// The parser's built-in card-payment check runs first and beats user
/// rules: those rows get `Transfers > Card Payment` and are locked, so
/// re-running rules on rule changes never moves them. The check sees the
/// account's kind, since the same marker can mean income on a chequing
/// account and a transfer on the card (3.66). Every other row takes the
/// winning rule, or stays `NULL` (Uncategorized). The rule set is compiled
/// once for the whole call, not per row (3.73).
fn categorize_new_rows(
    conn: &Connection,
    parser: &dyn StatementParser,
    kind: AccountKind,
    rows: &[PreparedRow],
) -> Result<Vec<(Option<i64>, bool)>, ImportError> {
    let rules = CompiledRules::new(&list_rules(conn)?);
    let mut card_payment: Option<i64> = None;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if parser.is_card_payment(&row.txn, kind) {
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
            out.push((rules.pick(&row.txn.description_raw), false));
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
    let mut affected_years: BTreeSet<i32> = {
        let mut stmt = tx.prepare("SELECT posted_date FROM transactions WHERE import_id = ?1")?;
        let dates = stmt.query_map([import_id], |r| r.get::<_, String>(0))?;
        years_of(dates.filter_map(|r| r.ok()))
    };
    // Surviving partners of the undone rows' transfer pairs (4.2). The
    // `transfer_pair_id` foreign key NULLs the link on delete, but the lock
    // and the transfer category would stick — release them the way *Unpair*
    // does, so rules re-apply and the row is pairable again.
    let partners: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT id FROM transactions
              WHERE import_id <> ?1
                AND transfer_pair_id IN (SELECT id FROM transactions WHERE import_id = ?1)",
        )?;
        let ids = stmt.query_map([import_id], |r| r.get::<_, i64>(0))?;
        ids.collect::<Result<Vec<_>, _>>()?
    };
    for partner in partners {
        clear_override(&tx, partner)?;
    }
    // Near-match replacements (4.3): restore every transaction this import
    // overwrote *before* deleting the import — `replacements` and the
    // aliases it added cascade with the import row. Affected master CSV
    // years include both the restored (old) and the replaced (current) dates.
    struct Replacement {
        id: i64,
        new_posted_date: String,
        old_raw_bytes: Vec<u8>,
        old_row_hash: String,
        old_posted_date: String,
        old_processed_date: Option<String>,
        old_amount_cents: i64,
        old_description_raw: String,
        old_merchant: Option<String>,
        old_original_currency: Option<String>,
        old_original_amount_cents: Option<i64>,
    }
    let replacements: Vec<Replacement> = {
        let mut stmt = tx.prepare(
            "SELECT r.transaction_id, t.posted_date, r.old_raw_bytes, r.old_row_hash,
                    r.old_posted_date, r.old_processed_date, r.old_amount_cents,
                    r.old_description_raw, r.old_merchant, r.old_original_currency,
                    r.old_original_amount_cents
               FROM replacements r
               JOIN transactions t ON t.id = r.transaction_id
              WHERE r.import_id = ?1",
        )?;
        let rows = stmt.query_map([import_id], |r| {
            Ok(Replacement {
                id: r.get(0)?,
                new_posted_date: r.get(1)?,
                old_raw_bytes: r.get(2)?,
                old_row_hash: r.get(3)?,
                old_posted_date: r.get(4)?,
                old_processed_date: r.get(5)?,
                old_amount_cents: r.get(6)?,
                old_description_raw: r.get(7)?,
                old_merchant: r.get(8)?,
                old_original_currency: r.get(9)?,
                old_original_amount_cents: r.get(10)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for r in &replacements {
        affected_years.extend(years_of([
            r.old_posted_date.as_str(),
            r.new_posted_date.as_str(),
        ]));
        tx.execute(
            "UPDATE transactions
                SET raw_bytes = ?2, row_hash = ?3, posted_date = ?4, processed_date = ?5,
                    amount_cents = ?6, description_raw = ?7, merchant = ?8,
                    original_currency = ?9, original_amount_cents = ?10
              WHERE id = ?1",
            params![
                r.id,
                r.old_raw_bytes,
                r.old_row_hash,
                r.old_posted_date,
                r.old_processed_date,
                r.old_amount_cents,
                r.old_description_raw,
                r.old_merchant,
                r.old_original_currency,
                r.old_original_amount_cents,
            ],
        )?;
    }
    tx.execute("DELETE FROM imports WHERE id = ?1", [import_id])?;
    // Re-pair (3.69): partners released above may match an existing
    // unpaired row right away, and recomputing the reviews from scratch
    // means none is left pointing at a row this undo deleted.
    crate::transfers::run_transfer_pairing(&tx)?;
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

/// Posted dates of a near-match pair may be at most this many days apart
/// (inclusive: exactly 3 days matches).
const NEAR_MATCH_MAX_DAYS: i64 = 3;

/// Whether `new_cents` could be the same charge as `existing_cents` (4.3):
/// same direction, and within **20%** of the existing amount — inclusive,
/// so exactly +20% (a tip) matches. Integer cents, so the comparison is
/// exact (`i128` to keep the products away from overflow).
fn amount_near(new_cents: i64, existing_cents: i64) -> bool {
    let (new, old) = (
        new_cents.unsigned_abs() as i128,
        existing_cents.unsigned_abs() as i128,
    );
    new_cents.signum() == existing_cents.signum() && new * 100 >= old * 80 && new * 100 <= old * 120
}

/// A stored transaction as the near-match review sees it.
struct ExistingRow {
    id: i64,
    posted_date: String,
    amount_cents: i64,
    description_raw: String,
}

/// Near-match review (4.3): for each new row, the existing row in the same
/// account it could be a changed re-export of — same cleaned merchant
/// ([`financi_core::parse::merchant`] over `description_raw`, compared
/// case-insensitively, as the rules queue does), amount within 20% and
/// posted date within ±3 days (both inclusive). Hidden rows (deleted) and
/// rows whose cleaned merchant is empty (nothing to compare) never pair.
///
/// Only *changed* charges pair (3.68):
/// - an existing row whose hash — or any alias hash — appears in
///   `file_hashes` is still in the file unchanged: a repeat purchase, not a
///   changed charge, so it never pairs (yesterday's coffee in the same
///   export as today's is not "Same charge?").
/// - a replacement keeps the existing row's `import_id`, so its new bytes
///   would be written to the master CSV under that import's header and
///   encoding. Rows whose import variant — header bytes, encoding, BOM, the
///   key [`crate::master_csv`] groups by — differs from `file`'s don't pair;
///   they import as new instead.
///
/// The scan is bounded (3.68): only stored rows posted between the new
/// rows' earliest date − [`NEAR_MATCH_MAX_DAYS`] and their latest date +
/// [`NEAR_MATCH_MAX_DAYS`] can pair, each candidate's lowercased merchant
/// is computed once, and candidates are looked up per merchant in a map
/// instead of being rescanned for every new row.
///
/// Greedy and deterministic: new rows are considered in file order and each
/// takes the best remaining candidate — nearest posted date, ties by lowest
/// transaction id — and an existing row is never matched to two new rows.
pub fn find_near_matches(
    conn: &Connection,
    account_id: i64,
    new_rows: &[PreparedRow],
    file_hashes: &HashSet<String>,
    file: &RawFile,
) -> Result<Vec<NearMatch>, DbError> {
    if new_rows.is_empty() {
        return Ok(Vec::new());
    }
    let parse_date = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();

    // Widest window any candidate could fall in: the new rows' date range,
    // grown by the pairing tolerance on both sides.
    let mut range: Option<(NaiveDate, NaiveDate)> = None;
    for row in new_rows {
        let Some(d) = parse_date(&row.txn.posted_date) else {
            continue; // parser-produced dates always parse; such a row never pairs
        };
        range = Some(match range {
            Some((lo, hi)) => (lo.min(d), hi.max(d)),
            None => (d, d),
        });
    }
    let tolerance = chrono::Days::new(NEAR_MATCH_MAX_DAYS as u64);
    let Some((lo, hi)) = range.and_then(|(lo, hi)| {
        Some((
            lo.checked_sub_days(tolerance)?,
            hi.checked_add_days(tolerance)?,
        ))
    }) else {
        return Ok(Vec::new());
    };
    let (lo, hi) = (
        lo.format("%Y-%m-%d").to_string(),
        hi.format("%Y-%m-%d").to_string(),
    );

    // The file's master CSV variant (3.68): a replacement rewrites the
    // existing row under its own import, so only same-variant rows pair.
    let variant = (
        file.header_raw.as_slice(),
        file.encoding.as_str(),
        file.has_bom,
    );

    // Rows the file still carries through an alias hash (a prior
    // replacement's old hash): same exclusion as a direct hash hit.
    let aliased_in_file: HashSet<i64> = {
        let mut stmt = conn.prepare(
            "SELECT a.transaction_id, a.row_hash
               FROM row_aliases a
               JOIN transactions t ON t.id = a.transaction_id
              WHERE t.account_id = ?1 AND t.posted_date BETWEEN ?2 AND ?3",
        )?;
        let rows = stmt.query_map(params![account_id, &lo, &hi], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.filter_map(|row| row.ok())
            .filter(|(_, hash)| file_hashes.contains(hash))
            .map(|(id, _)| id)
            .collect()
    };

    let mut by_merchant: HashMap<String, Vec<ExistingRow>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.posted_date, t.amount_cents, t.description_raw, t.row_hash,
                    i.header_raw, i.encoding, i.has_bom
               FROM transactions t
               JOIN imports i ON i.id = t.import_id
              WHERE t.account_id = ?1 AND t.hidden = 0
                AND t.posted_date BETWEEN ?2 AND ?3",
        )?;
        let rows = stmt.query_map(params![account_id, &lo, &hi], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Vec<u8>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, bool>(7)?,
            ))
        })?;
        for row in rows {
            let (
                id,
                posted_date,
                amount_cents,
                description_raw,
                row_hash,
                header_raw,
                encoding,
                has_bom,
            ) = row?;
            if file_hashes.contains(&row_hash) || aliased_in_file.contains(&id) {
                continue; // still in the file unchanged: a repeat purchase
            }
            if (header_raw.as_slice(), encoding.as_str(), has_bom) != variant {
                continue; // the replacement would corrupt the old import's CSV
            }
            // Lowercased once per row (3.68). ASCII folding matches the
            // previous `eq_ignore_ascii_case` comparison exactly.
            let merchant = parse::merchant(&description_raw).to_ascii_lowercase();
            if merchant.is_empty() {
                continue;
            }
            by_merchant.entry(merchant).or_default().push(ExistingRow {
                id,
                posted_date,
                amount_cents,
                description_raw,
            });
        }
    }

    let mut taken: HashSet<i64> = HashSet::new();
    let mut matches = Vec::new();
    for (ix, row) in new_rows.iter().enumerate() {
        let merchant = parse::merchant(&row.txn.description_raw).to_ascii_lowercase();
        // Parser-produced dates always parse; skip rather than panic.
        let Some(new_date) = parse_date(&row.txn.posted_date) else {
            continue;
        };
        if merchant.is_empty() {
            continue;
        }
        let mut best: Option<(i64, i64, &ExistingRow)> = None; // (|days|, id, row)
        for e in by_merchant.get(&merchant).map_or(&[][..], Vec::as_slice) {
            if taken.contains(&e.id) || !amount_near(row.txn.amount_cents, e.amount_cents) {
                continue;
            }
            let Some(days) = parse_date(&e.posted_date).map(|d| (new_date - d).num_days().abs())
            else {
                continue;
            };
            if days > NEAR_MATCH_MAX_DAYS {
                continue;
            }
            if best.is_none_or(|(best_days, best_id, _)| (days, e.id) < (best_days, best_id)) {
                best = Some((days, e.id, e));
            }
        }
        if let Some((_, _, e)) = best {
            taken.insert(e.id);
            matches.push(NearMatch {
                new_row: ix,
                source_line: row.source_line,
                new_date: row.txn.posted_date.clone(),
                new_amount_cents: row.txn.amount_cents,
                new_description_raw: row.txn.description_raw.clone(),
                existing_id: e.id,
                existing_date: e.posted_date.clone(),
                existing_amount_cents: e.amount_cents,
                existing_description_raw: e.description_raw.clone(),
            });
        }
    }
    Ok(matches)
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
    /// `category_locked` (2.5): the category was set by hand or locked at
    /// import (a built-in card payment), so rules never move the row. The
    /// dashboard menu reads it to disable *Show rule* (3.29).
    pub category_locked: bool,
}

/// Transactions newest first, optionally for one account. Hidden rows are
/// left out.
pub fn list_transactions(
    conn: &Connection,
    account_id: Option<i64>,
) -> Result<Vec<TransactionRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT t.id, i.name || ' · ' || a.name, t.posted_date, t.description_raw, t.merchant,
                c.name, t.amount_cents, t.category_id, t.hidden, t.transfer_pair_id,
                t.category_locked
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
            category_locked: r.get(10)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
