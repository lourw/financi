//! One-time data repairs, recorded in `settings` so each runs exactly
//! once after an upgrade. A repair that must re-run Rust rule logic can't
//! be a migration (migrations are plain SQL), so [`crate::open`] calls
//! these after [`crate::migrate`] instead.

use std::collections::HashMap;

use financi_core::rules::CompiledRules;
use financi_core::{AccountKey, Encoding, StatementParser, raw_record_from_bytes};
use rusqlite::{Connection, params};

use crate::Result;
use crate::categories::card_payment_category;
use crate::rules::list_rules;
use crate::settings;

/// The `settings` key recording that
/// [`repair_misclassified_card_payments`] ran. The value is the number of
/// rows it unlocked and re-ran rules on.
pub const MISCLASSIFIED_CARD_PAYMENTS: &str = "repair.misclassified_card_payments";

/// 3.66: until the parser check learned the account kind, Simplii's Visa
/// rule (money in whose description contains `PAYMENT`) also ran on
/// chequing imports, locking chequing income such as ` GST PAYMENT CANADA`
/// into `Transfers › Card Payment` and out of the totals. This repair runs
/// once on the first open after the upgrade: for chequing and savings
/// accounts it finds rows sitting in `Transfers › Card Payment` with
/// `amount_cents > 0` and no `transfer_pair_id`, clears `category_locked`
/// and re-runs the rules on them (the winning rule's category, or `NULL`
/// when nothing matches, so the row resurfaces as income in
/// To-categorize). Rows already paired as transfers, money-out rows (the
/// real card-payment debits) and credit-card rows are left alone.
///
/// Completion is recorded under [`MISCLASSIFIED_CARD_PAYMENTS`] in the
/// same transaction as the updates, so a failed repair retries on the next
/// open and a completed one never runs again. Returns the number of rows
/// repaired (0 when already done).
pub fn repair_misclassified_card_payments(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    if settings::get(&tx, MISCLASSIFIED_CARD_PAYMENTS)?.is_some() {
        return Ok(0);
    }
    let card_payment = card_payment_category(&tx)?;
    let rows: Vec<(i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT t.id, t.description_raw
               FROM transactions t
               JOIN accounts a ON a.id = t.account_id
              WHERE a.kind IN ('chequing', 'savings')
                AND t.category_id = ?1
                AND t.amount_cents > 0
                AND t.transfer_pair_id IS NULL",
        )?;
        let rows = stmt.query_map([card_payment], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    {
        let rules = CompiledRules::new(&list_rules(&tx)?);
        let mut update = tx.prepare(
            "UPDATE transactions SET category_id = ?1, category_locked = 0 WHERE id = ?2",
        )?;
        for (id, description) in &rows {
            update.execute(params![rules.pick(description), id])?;
        }
    }
    settings::set(&tx, MISCLASSIFIED_CARD_PAYMENTS, &rows.len().to_string())?;
    tx.commit()?;
    Ok(rows.len())
}

/// The `settings` key recording that [`repair_row_hashes_from_bytes`] ran.
/// The value is the number of stored hashes it rewrote.
pub const ROW_HASH_BYTES: &str = "repair.row_hash_bytes";

/// Parsers looked up once per repair run; `None` caches "unknown id".
#[derive(Default)]
struct ParserCache(HashMap<String, Option<Box<dyn StatementParser + Send + Sync>>>);

impl ParserCache {
    /// The 3.71 hash of one stored record's bytes: the same parser, CSV
    /// split and hash [`crate::import`] applies when importing them again.
    /// `None` when the stored parser id or encoding is no longer recognised;
    /// the row is then left as it is.
    fn hash(&mut self, source: &Source, bytes: &[u8]) -> Option<String> {
        let encoding = Encoding::parse(&source.encoding)?;
        let parser = self
            .0
            .entry(source.parser_id.clone())
            .or_insert_with(|| financi_parsers::by_id(&source.parser_id))
            .as_ref()?;
        let record = raw_record_from_bytes(bytes, encoding);
        Some(parser.row_hash(&AccountKey(source.key.clone()), &record).0)
    }
}

/// The recipe for re-hashing one stored record: which parser and encoding
/// produced the row, and the account key its hash includes.
struct Source {
    key: String,
    parser_id: String,
    encoding: String,
}

/// 3.71: a row's hash is computed over its identity fields' exported
/// bytes, not over the decoded text, so a file's encoding guess can never
/// move a row's identity. Rows imported before that fix carry the decoded
/// hash, which differs whenever decoding changed the bytes (any non-ASCII
/// row, and nothing else). This repair runs once on the first open after
/// the upgrade and recomputes every stored hash from the row's own
/// `raw_bytes` — through the same parser and record splitting a re-import
/// uses, so re-importing an overlapping file afterwards still reconciles.
/// It rewrites `transactions.row_hash`, the `row_hash` kept in
/// `replacements` (so `undo_last_import` restores a current-style hash),
/// and `row_aliases` (so near-match exclusion keeps seeing the old rows).
/// An alias's bytes are its replacement's `old_raw_bytes`, which belong to
/// the replaced row's own import, not to the import that replaced it.
///
/// Nothing is deleted: two rows that the bug let in twice (same logical
/// row, two encoding guesses, two hashes) end up sharing one hash and
/// stay visible for the user to deal with. Completion is recorded under
/// [`ROW_HASH_BYTES`] in the same transaction as the updates, so a failed
/// repair retries on the next open and a completed one never runs again.
/// Returns the number of hashes rewritten across the three tables.
pub fn repair_row_hashes_from_bytes(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    if settings::get(&tx, ROW_HASH_BYTES)?.is_some() {
        return Ok(0);
    }
    let mut parsers = ParserCache::default();
    let mut changed = 0usize;

    // Current rows: each row's bytes came from its own import.
    let rows: Vec<(i64, String, Vec<u8>, Source)> = {
        let mut stmt = tx.prepare(
            "SELECT t.id, t.row_hash, t.raw_bytes, a.account_key, i.parser_id, i.encoding
               FROM transactions t
               JOIN accounts a ON a.id = t.account_id
               JOIN imports i ON i.id = t.import_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                Source {
                    key: r.get(3)?,
                    parser_id: r.get(4)?,
                    encoding: r.get(5)?,
                },
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    {
        let mut update = tx.prepare("UPDATE transactions SET row_hash = ?2 WHERE id = ?1")?;
        for (id, old, bytes, source) in rows {
            let Some(new) = parsers.hash(&source, &bytes) else {
                continue;
            };
            if new != old {
                changed += update.execute(params![id, new])?;
            }
        }
    }

    // Replacement snapshots: the old bytes belong to the replaced row's
    // own import (`t.import_id`), not to `r.import_id` (the new import).
    let rows: Vec<(i64, i64, String, Vec<u8>, Source)> = {
        let mut stmt = tx.prepare(
            "SELECT r.import_id, r.transaction_id, r.old_row_hash, r.old_raw_bytes,
                    a.account_key, i.parser_id, i.encoding
               FROM replacements r
               JOIN transactions t ON t.id = r.transaction_id
               JOIN accounts a ON a.id = t.account_id
               JOIN imports i ON i.id = t.import_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                Source {
                    key: r.get(4)?,
                    parser_id: r.get(5)?,
                    encoding: r.get(6)?,
                },
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    {
        let mut update = tx.prepare(
            "UPDATE replacements SET old_row_hash = ?3 WHERE import_id = ?1 AND transaction_id = ?2",
        )?;
        for (import_id, transaction_id, old, bytes, source) in rows {
            let Some(new) = parsers.hash(&source, &bytes) else {
                continue;
            };
            if new != old {
                changed += update.execute(params![import_id, transaction_id, new])?;
            }
        }
    }

    // Aliases: each one's bytes are its replacement's old bytes; the pair
    // is written together, so an alias with no (surviving) replacement is
    // skipped. Rewrite as DELETE + INSERT OR IGNORE so two aliases of one
    // row that converge on the same hash can't collide on the primary key.
    let rows: Vec<(i64, String, i64, Vec<u8>, Source)> = {
        let mut stmt = tx.prepare(
            "SELECT al.transaction_id, al.row_hash, al.import_id, r.old_raw_bytes,
                    a.account_key, i.parser_id, i.encoding
               FROM row_aliases al
               JOIN replacements r
                 ON r.transaction_id = al.transaction_id AND r.import_id = al.import_id
               JOIN transactions t ON t.id = al.transaction_id
               JOIN accounts a ON a.id = t.account_id
               JOIN imports i ON i.id = t.import_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                Source {
                    key: r.get(4)?,
                    parser_id: r.get(5)?,
                    encoding: r.get(6)?,
                },
            ))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    {
        let mut rewrite =
            tx.prepare("DELETE FROM row_aliases WHERE transaction_id = ?1 AND row_hash = ?2")?;
        let mut insert = tx.prepare(
            "INSERT OR IGNORE INTO row_aliases (transaction_id, row_hash, import_id)
             VALUES (?1, ?2, ?3)",
        )?;
        for (transaction_id, old, import_id, bytes, source) in rows {
            let Some(new) = parsers.hash(&source, &bytes) else {
                continue;
            };
            if new != old {
                rewrite.execute(params![transaction_id, old])?;
                insert.execute(params![transaction_id, new, import_id])?;
                changed += 1;
            }
        }
    }

    settings::set(&tx, ROW_HASH_BYTES, &changed.to_string())?;
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{create_child, create_parent};
    use crate::{add_rule, create_account, open_in_memory};
    use financi_core::{AccountKind, CategoryKind, row_hash};

    /// An account with one import, so stored rows have everything the
    /// repair and the rule re-run read.
    fn account_with_import(conn: &Connection, kind: AccountKind) -> (i64, i64) {
        let account =
            create_account(conn, "simplii", &uuid::Uuid::new_v4().to_string(), kind).unwrap();
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'simplii', 'chequing.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [account],
        )
        .unwrap();
        (account, conn.last_insert_rowid())
    }

    /// A stored row as the buggy import left it: in `Transfers › Card
    /// Payment`, locked. Returns its id.
    fn locked_card_payment_row(
        conn: &Connection,
        account: i64,
        import: i64,
        description: &str,
        amount_cents: i64,
    ) -> i64 {
        let card_payment = card_payment_category(conn).unwrap();
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw,
                                       category_id, category_locked)
             VALUES (?1, ?2, 0, X'00', ?3, '2026-10-12', ?4, ?5, ?6, 1)",
            params![
                account,
                import,
                uuid::Uuid::new_v4().to_string(),
                amount_cents,
                description,
                card_payment,
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn stored(conn: &Connection, id: i64) -> (Option<i64>, bool) {
        conn.query_row(
            "SELECT category_id, category_locked FROM transactions WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn unlocks_affected_rows_and_reruns_rules() {
        let mut conn = open_in_memory().unwrap();
        let (chequing, import) = account_with_import(&conn, AccountKind::Chequing);
        let food = create_parent(&conn, "Income", CategoryKind::Income).unwrap();
        let gst = create_child(&conn, food, "GST").unwrap();
        add_rule(&conn, "GST", false, gst).unwrap();

        let repaired =
            locked_card_payment_row(&conn, chequing, import, " GST PAYMENT CANADA", 10_000);
        // No rule matches this one: it goes back to Uncategorized (income
        // the user can categorize), not Card Payment.
        let uncategorized =
            locked_card_payment_row(&conn, chequing, import, "PAYMENT REVERSAL", 5_000);

        assert_eq!(repair_misclassified_card_payments(&mut conn).unwrap(), 2);
        assert_eq!(stored(&conn, repaired), (Some(gst), false));
        assert_eq!(stored(&conn, uncategorized), (None, false));
    }

    #[test]
    fn covers_savings_too() {
        let mut conn = open_in_memory().unwrap();
        let (savings, import) = account_with_import(&conn, AccountKind::Savings);
        let row = locked_card_payment_row(&conn, savings, import, " PAYMENT REVERSAL", 8_000);
        assert_eq!(repair_misclassified_card_payments(&mut conn).unwrap(), 1);
        assert_eq!(stored(&conn, row), (None, false));
    }

    #[test]
    fn leaves_paired_negative_and_credit_card_rows_alone() {
        let mut conn = open_in_memory().unwrap();
        let card_payment = card_payment_category(&conn).unwrap();
        let (chequing, import) = account_with_import(&conn, AccountKind::Chequing);
        let (visa, visa_import) = account_with_import(&conn, AccountKind::CreditCard);

        // A genuine card payment from chequing: money out.
        let debit = locked_card_payment_row(
            &conn,
            chequing,
            import,
            "CREDIT CARD PAYMENT - AMEX",
            -15_000,
        );
        // A positive row already paired as one side of a transfer: the
        // pairing, not the old parser rule, put it where it belongs.
        let paired = locked_card_payment_row(&conn, chequing, import, " PAYMENT RECEIVED", 7_000);
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, 1, X'00', 'partner', '2026-10-12', 7_000, ' BETWEEN ACCOUNTS')",
            params![visa, visa_import],
        )
        .unwrap();
        let partner = conn.last_insert_rowid();
        conn.execute(
            "UPDATE transactions SET transfer_pair_id = ?2 WHERE id = ?1",
            params![paired, partner],
        )
        .unwrap();
        // The Visa's own payments: a different account kind, out of scope.
        let visa_row = locked_card_payment_row(
            &conn,
            visa,
            visa_import,
            "PAYMENT RECEIVED THANK YOU",
            25_000,
        );

        assert_eq!(
            repair_misclassified_card_payments(&mut conn).unwrap(),
            0,
            "nothing here was misclassified"
        );
        assert_eq!(stored(&conn, debit), (Some(card_payment), true));
        assert_eq!(stored(&conn, paired), (Some(card_payment), true));
        assert_eq!(stored(&conn, visa_row), (Some(card_payment), true));
    }

    #[test]
    fn runs_once_and_records_completion() {
        let mut conn = open_in_memory().unwrap();
        let (chequing, import) = account_with_import(&conn, AccountKind::Chequing);
        let row = locked_card_payment_row(&conn, chequing, import, " GST PAYMENT CANADA", 10_000);

        assert_eq!(repair_misclassified_card_payments(&mut conn).unwrap(), 1);
        assert_eq!(
            settings::get(&conn, MISCLASSIFIED_CARD_PAYMENTS)
                .unwrap()
                .as_deref(),
            Some("1")
        );

        // A second open never re-touches rows, even one that looks
        // affected again (idempotent).
        let card_payment = card_payment_category(&conn).unwrap();
        conn.execute(
            "UPDATE transactions SET category_id = ?1, category_locked = 1 WHERE id = ?2",
            params![card_payment, row],
        )
        .unwrap();
        assert_eq!(repair_misclassified_card_payments(&mut conn).unwrap(), 0);
        assert_eq!(stored(&conn, row), (Some(card_payment), true));
    }

    // --- repair_row_hashes_from_bytes (3.71) ---

    /// An account with one import recorded with a given parser id and
    /// encoding. Returns (account id, import id, account key).
    fn account_import(conn: &Connection, parser_id: &str, encoding: &str) -> (i64, i64, String) {
        let account = create_account(
            conn,
            "simplii",
            &uuid::Uuid::new_v4().to_string(),
            AccountKind::Chequing,
        )
        .unwrap();
        let import = add_import(conn, account, parser_id, encoding);
        let account_key: String = conn
            .query_row(
                "SELECT account_key FROM accounts WHERE id = ?1",
                [account],
                |r| r.get(0),
            )
            .unwrap();
        (account, import, account_key)
    }

    fn add_import(conn: &Connection, account: i64, parser_id: &str, encoding: &str) -> i64 {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, ?2, 'bank.csv', '2026-11-01', X'00', ?3, 0, 'lf', 0, 1)",
            params![account, parser_id, encoding],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// A stored row with exactly these bytes and this hash. Returns its id.
    fn row_with_bytes(
        conn: &Connection,
        account: i64,
        import: i64,
        bytes: &[u8],
        hash: &str,
    ) -> i64 {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, 0, ?3, ?4, '2026-10-12', -1234, 'CAFÉ')",
            params![account, import, bytes, hash],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn hash_of_row(conn: &Connection, id: i64) -> String {
        conn.query_row(
            "SELECT row_hash FROM transactions WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// The current (3.71) hash of these bytes — the same computation the
    /// repair, and an import of the same row, perform.
    fn bytes_hash(key: &str, bytes: &[u8]) -> String {
        ParserCache::default()
            .hash(
                &Source {
                    key: key.to_string(),
                    parser_id: "simplii".to_string(),
                    encoding: "windows-1252".to_string(),
                },
                bytes,
            )
            .unwrap()
    }

    /// The buggy hash: the decoded text's UTF-8 bytes, as the pre-3.71
    /// parser computed it from `fields`.
    fn decoded_hash(key: &str, decoded_description: &str) -> String {
        row_hash(
            &AccountKey(key.to_string()),
            &[b"10/07/2026", decoded_description.as_bytes(), b"", b"12.34"],
        )
        .0
    }

    const ACCENTED: &[u8] = b"10/07/2026,CAF\xc9,,12.34";

    #[test]
    fn rehashes_non_ascii_rows_from_stored_bytes() {
        let mut conn = open_in_memory().unwrap();
        let (account, import, key) = account_import(&conn, "simplii", "windows-1252");
        let decoded = decoded_hash(&key, "CAFÉ");
        let row = row_with_bytes(&conn, account, import, ACCENTED, &decoded);
        // An ASCII row already carrying its current hash: nothing to change.
        let ascii = b"10/08/2026,COFFEE,,4.50";
        let ascii_hash = bytes_hash(&key, ascii);
        let ascii_row = row_with_bytes(&conn, account, import, ascii, &ascii_hash);

        assert_eq!(repair_row_hashes_from_bytes(&mut conn).unwrap(), 1);
        assert_eq!(hash_of_row(&conn, row), bytes_hash(&key, ACCENTED));
        assert_eq!(hash_of_row(&conn, ascii_row), ascii_hash);
    }

    #[test]
    fn converges_duplicate_rows_the_bug_let_in_twice() {
        let mut conn = open_in_memory().unwrap();
        let (account, import, key) = account_import(&conn, "simplii", "windows-1252");
        // The same logical row imported twice under two encoding guesses:
        // decoded as Windows-1252 and as lossy UTF-8 it hashed differently,
        // so the bug let both in.
        let h1 = decoded_hash(&key, "CAFÉ");
        let h2 = decoded_hash(&key, "CAF\u{FFFD}");
        assert_ne!(h1, h2);
        let a = row_with_bytes(&conn, account, import, ACCENTED, &h1);
        let b = row_with_bytes(&conn, account, import, ACCENTED, &h2);

        assert_eq!(repair_row_hashes_from_bytes(&mut conn).unwrap(), 2);
        let want = bytes_hash(&key, ACCENTED);
        assert_eq!(hash_of_row(&conn, a), want);
        assert_eq!(
            hash_of_row(&conn, b),
            want,
            "both rows survive, sharing one hash"
        );
    }

    #[test]
    fn rehashes_replacement_snapshots_and_aliases() {
        let mut conn = open_in_memory().unwrap();
        let (account, import1, key) = account_import(&conn, "simplii", "windows-1252");
        let old_decoded = decoded_hash(&key, "CAFÉ");
        let row = row_with_bytes(&conn, account, import1, ACCENTED, &old_decoded);
        // A second import replaced this row (4.3): the replacement and the
        // alias hold the old buggy hash, the row the replacement's.
        let import2 = add_import(&conn, account, "simplii", "windows-1252");
        let new_bytes = b"10/07/2026,CAF\xc9 MARKET,,12.34";
        conn.execute(
            "INSERT INTO replacements (import_id, transaction_id, old_raw_bytes, old_row_hash,
                                       old_posted_date, old_processed_date, old_amount_cents,
                                       old_description_raw, old_merchant)
             VALUES (?1, ?2, ?3, ?4, '2026-10-12', NULL, -1234, 'CAFÉ', NULL)",
            params![import2, row, ACCENTED, old_decoded],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO row_aliases (transaction_id, row_hash, import_id) VALUES (?1, ?2, ?3)",
            params![row, old_decoded, import2],
        )
        .unwrap();
        conn.execute(
            "UPDATE transactions SET raw_bytes = ?2, row_hash = ?3 WHERE id = ?1",
            params![row, new_bytes, decoded_hash(&key, "CAFÉ MARKET")],
        )
        .unwrap();

        // Row, replacement snapshot and alias: three rewrites.
        assert_eq!(repair_row_hashes_from_bytes(&mut conn).unwrap(), 3);
        assert_eq!(hash_of_row(&conn, row), bytes_hash(&key, new_bytes));
        let old_now = bytes_hash(&key, ACCENTED);
        let (rhash, ahash): (String, String) = conn
            .query_row(
                "SELECT (SELECT old_row_hash FROM replacements WHERE transaction_id = ?1),
                        (SELECT row_hash FROM row_aliases WHERE transaction_id = ?1)",
                [row],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rhash, old_now, "undo restores a current-style hash");
        assert_eq!(ahash, old_now, "near-match exclusion sees the byte hash");

        // Undoing the replacing import puts the byte hash back on the row.
        crate::import::undo_last_import(&mut conn, account, import2)
            .unwrap()
            .unwrap();
        assert_eq!(hash_of_row(&conn, row), old_now);
    }

    #[test]
    fn hash_repair_runs_once() {
        let mut conn = open_in_memory().unwrap();
        let (account, import, _key) = account_import(&conn, "simplii", "windows-1252");
        row_with_bytes(&conn, account, import, ACCENTED, "decoded");

        assert_eq!(repair_row_hashes_from_bytes(&mut conn).unwrap(), 1);
        assert_eq!(
            repair_row_hashes_from_bytes(&mut conn).unwrap(),
            0,
            "the flag stops a second run"
        );
        assert_eq!(
            settings::get(&conn, ROW_HASH_BYTES).unwrap().as_deref(),
            Some("1"),
            "the value records how many hashes it rewrote"
        );
        // Re-running with the flag cleared changes nothing: all rows are
        // current after the first run.
        conn.execute("DELETE FROM settings WHERE key = ?1", [ROW_HASH_BYTES])
            .unwrap();
        assert_eq!(repair_row_hashes_from_bytes(&mut conn).unwrap(), 0);
    }

    #[test]
    fn leaves_rows_with_unknown_parser_alone() {
        let mut conn = open_in_memory().unwrap();
        let (account, import, _key) = account_import(&conn, "gone-bank", "windows-1252");
        let row = row_with_bytes(&conn, account, import, b"10/07/2026,CAF\xc9,,12.34", "kept");
        assert_eq!(repair_row_hashes_from_bytes(&mut conn).unwrap(), 0);
        assert_eq!(hash_of_row(&conn, row), "kept");
    }
}
