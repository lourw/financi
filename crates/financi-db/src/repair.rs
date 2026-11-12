//! One-time data repairs, recorded in `settings` so each runs exactly
//! once after an upgrade. A repair that must re-run Rust rule logic can't
//! be a migration (migrations are plain SQL), so [`crate::open`] calls
//! these after [`crate::migrate`] instead.

use financi_core::rules::CompiledRules;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{create_child, create_parent};
    use crate::{add_rule, create_account, open_in_memory};
    use financi_core::{AccountKind, CategoryKind};

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
}
