//! Manual edits (2.6): assign a category by hand, undo that, and turn a
//! manual edit into a rule.
//!
//! [`set_category`] locks the row (`category_locked = 1`) so the rule
//! engine never moves it again. [`clear_override`] unlocks it and hands it
//! back to the rules — except for built-in card payments, which are locked
//! at import without being user overrides and go back to
//! `Transfers > Card Payment`, still locked.
//! [`apply_rule_from_merchant`] is the backend of *"Apply to all matching
//! \"MERCHANT\""*: a substring rule from the cleaned merchant, which
//! categorizes the whole matching group.

use financi_core::rules::pick_category;
use financi_core::{RawTransaction, parse::merchant};
use rusqlite::{Connection, OptionalExtension};

use crate::categories::{card_payment_category, ensure_assignable};
use crate::rules::{add_rule, list_rules};
use crate::{DbError, Result};

/// The stored fields needed to judge and re-categorize one transaction.
struct StoredRow {
    posted_date: String,
    processed_date: Option<String>,
    amount_cents: i64,
    original_currency: Option<String>,
    original_amount_cents: Option<i64>,
    description_raw: String,
    merchant: Option<String>,
    parser_id: String,
}

fn fetch_row(conn: &Connection, transaction_id: i64) -> Result<StoredRow> {
    conn.query_row(
        "SELECT t.posted_date, t.processed_date, t.amount_cents, t.original_currency,
                t.original_amount_cents, t.description_raw, t.merchant, i.parser_id
           FROM transactions t
           JOIN imports i ON i.id = t.import_id
          WHERE t.id = ?1",
        [transaction_id],
        |r| {
            Ok(StoredRow {
                posted_date: r.get(0)?,
                processed_date: r.get(1)?,
                amount_cents: r.get(2)?,
                original_currency: r.get(3)?,
                original_amount_cents: r.get(4)?,
                description_raw: r.get(5)?,
                merchant: r.get(6)?,
                parser_id: r.get(7)?,
            })
        },
    )
    .optional()?
    .ok_or(DbError::UnknownTransaction(transaction_id))
}

/// Assign `category_id` to a transaction by hand and lock it: rules never
/// touch the row again (see [`crate::rules::reapply_rules`]). Refuses
/// parent categories and unknown transactions. Setting the same category
/// again is fine and (re)locks the row.
pub fn set_category(conn: &Connection, transaction_id: i64, category_id: i64) -> Result<()> {
    ensure_assignable(conn, category_id)?;
    let updated = conn.execute(
        "UPDATE transactions SET category_id = ?1, category_locked = 1 WHERE id = ?2",
        (category_id, transaction_id),
    )?;
    if updated == 0 {
        return Err(DbError::UnknownTransaction(transaction_id));
    }
    Ok(())
}

/// Undo a manual override on `transaction_id`: unlock it and re-apply the
/// rules to that row only (the winning rule's category, or `NULL` when
/// nothing matches).
///
/// Card-payment rows are locked at import (2.5) even though nobody
/// overrode them, so unlocking one must not hand it to the rules: the
/// stored row is checked again with its import's parser
/// (`imports.parser_id` → [`financi_parsers::by_id`]) and, when
/// `is_card_payment` still says yes, the row is restored to
/// `Transfers > Card Payment` and stays locked — built-in detection beats
/// user rules permanently.
///
/// The row is rebuilt as a [`RawTransaction`] from its stored columns.
/// Marker checks such as Amex's read only stored, already-mapped fields
/// (the description), so the rebuilt row is faithful for `is_card_payment`
/// without keeping the original `RawRecord` around.
pub fn clear_override(conn: &Connection, transaction_id: i64) -> Result<()> {
    let row = fetch_row(conn, transaction_id)?;
    let parser = financi_parsers::by_id(&row.parser_id)
        .ok_or(DbError::UnknownParser(row.parser_id.clone()))?;
    let txn = RawTransaction {
        posted_date: row.posted_date,
        processed_date: row.processed_date,
        amount_cents: row.amount_cents,
        original_currency: row.original_currency,
        original_amount_cents: row.original_amount_cents,
        description_raw: row.description_raw.clone(),
        merchant: row.merchant.clone(),
    };
    if parser.is_card_payment(&txn) {
        let card_payment = card_payment_category(conn)?;
        conn.execute(
            "UPDATE transactions SET category_id = ?1, category_locked = 1 WHERE id = ?2",
            (card_payment, transaction_id),
        )?;
        return Ok(());
    }
    let category_id = pick_category(&list_rules(conn)?, &row.description_raw);
    conn.execute(
        "UPDATE transactions SET category_id = ?1, category_locked = 0 WHERE id = ?2",
        (category_id, transaction_id),
    )?;
    Ok(())
}

/// Backend of *"Apply to all matching \"MERCHANT\""*: create a substring
/// rule from the cleaned merchant of this transaction
/// ([`financi_core::parse::merchant`] over `description_raw`) targeting
/// `category_id`, via [`add_rule`] — which re-runs the rules, so every
/// unlocked row of the matching group is categorized at once. The edited
/// row itself stays locked with its manual category. Refuses unknown
/// transactions, parent targets and rows whose cleaned merchant is empty.
/// Returns the new rule's id.
pub fn apply_rule_from_merchant(
    conn: &Connection,
    transaction_id: i64,
    category_id: i64,
) -> Result<i64> {
    let row = fetch_row(conn, transaction_id)?;
    ensure_assignable(conn, category_id)?;
    let merchant = merchant(&row.description_raw);
    if merchant.is_empty() {
        return Err(DbError::EmptyMerchant(transaction_id));
    }
    add_rule(conn, &merchant, false, category_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{create_child, create_parent};
    use crate::open_in_memory;
    use financi_core::{AccountKind, CategoryKind};

    fn child_category(conn: &Connection, name: &str) -> i64 {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let parent = create_parent(
            conn,
            &format!(
                "Parent{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            CategoryKind::Spend,
        )
        .unwrap();
        create_child(conn, parent, name).unwrap()
    }

    /// A stored transaction on its own account and import (parser
    /// `amex`), so card-payment detection and rule re-runs see it.
    fn add_transaction(
        conn: &Connection,
        description: &str,
        locked: bool,
        category_id: Option<i64>,
    ) -> i64 {
        let account = crate::create_account(
            conn,
            "amex",
            &uuid::Uuid::new_v4().to_string(),
            AccountKind::CreditCard,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'activity.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [account],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw,
                                       category_id, category_locked)
             VALUES (?1, ?2, 0, X'00', ?3, '2026-10-31', -450, ?4, ?5, ?6)",
            (
                account,
                import,
                uuid::Uuid::new_v4().to_string(),
                description,
                category_id,
                locked,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn row_state(conn: &Connection, id: i64) -> (Option<i64>, bool) {
        conn.query_row(
            "SELECT category_id, category_locked FROM transactions WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn set_category_assigns_and_locks_against_rules() {
        let conn = open_in_memory().unwrap();
        let manual = child_category(&conn, "Coffee");
        let ruled = child_category(&conn, "Snacks");
        let id = add_transaction(&conn, "COFFEE VANCOUVER", false, None);

        set_category(&conn, id, manual).unwrap();
        assert_eq!(row_state(&conn, id), (Some(manual), true));

        crate::rules::add_rule(&conn, "COFFEE", false, ruled).unwrap();
        assert_eq!(
            row_state(&conn, id),
            (Some(manual), true),
            "a locked row is never moved by rules"
        );
    }

    #[test]
    fn set_category_same_category_again_locks() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Groceries");
        crate::rules::add_rule(&conn, "COFFEE", false, cat).unwrap();
        let id = add_transaction(&conn, "COFFEE VANCOUVER", false, Some(cat));
        assert_eq!(row_state(&conn, id), (Some(cat), false), "by the rule");

        set_category(&conn, id, cat).unwrap();
        assert_eq!(
            row_state(&conn, id),
            (Some(cat), true),
            "same category again is fine and locks"
        );
    }

    #[test]
    fn set_category_refusals() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Groceries");
        let parent = create_parent(&conn, "Home", CategoryKind::Spend).unwrap();
        let id = add_transaction(&conn, "COFFEE VANCOUVER", false, None);

        assert!(matches!(
            set_category(&conn, 999, cat).unwrap_err(),
            DbError::UnknownTransaction(999)
        ));
        assert!(matches!(
            set_category(&conn, id, parent).unwrap_err(),
            DbError::ParentCategoryNotAssignable(id_) if id_ == parent
        ));
        assert!(matches!(
            set_category(&conn, id, 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        assert_eq!(
            row_state(&conn, id),
            (None, false),
            "refusals change nothing"
        );
    }

    #[test]
    fn clear_override_unlocks_and_reapplies_rules() {
        let conn = open_in_memory().unwrap();
        let coffee = child_category(&conn, "Coffee");
        let manual = child_category(&conn, "Fun");
        crate::rules::add_rule(&conn, "COFFEE", false, coffee).unwrap();
        let matched = add_transaction(&conn, "COFFEE VANCOUVER", false, Some(coffee));
        let unmatched = add_transaction(&conn, "PIZZA PLACE", false, None);

        set_category(&conn, matched, manual).unwrap();
        set_category(&conn, unmatched, manual).unwrap();

        clear_override(&conn, matched).unwrap();
        assert_eq!(
            row_state(&conn, matched),
            (Some(coffee), false),
            "back under the rule, unlocked"
        );
        clear_override(&conn, unmatched).unwrap();
        assert_eq!(
            row_state(&conn, unmatched),
            (None, false),
            "no rule matches, becomes Uncategorized"
        );

        let pizza = child_category(&conn, "Takeout");
        crate::rules::add_rule(&conn, "PIZZA", false, pizza).unwrap();
        assert_eq!(
            row_state(&conn, unmatched),
            (Some(pizza), false),
            "unlocked rows take future rules"
        );
    }

    #[test]
    fn clear_override_touches_only_that_row() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let id = add_transaction(&conn, "COFFEE VANCOUVER", true, Some(cat));
        let untouched = add_transaction(&conn, "COFFEE BAR", true, None);

        clear_override(&conn, id).unwrap();
        assert_eq!(
            row_state(&conn, untouched),
            (None, true),
            "other locked rows stay locked"
        );
    }

    #[test]
    fn clear_override_restores_card_payment_and_stays_locked() {
        let conn = open_in_memory().unwrap();
        let manual = child_category(&conn, "Fun");
        let card = card_payment_category(&conn).unwrap();
        let id = add_transaction(&conn, "PAYMENT RECEIVED - THANK YOU", true, Some(card));

        set_category(&conn, id, manual).unwrap();
        assert_eq!(row_state(&conn, id), (Some(manual), true));

        clear_override(&conn, id).unwrap();
        assert_eq!(
            row_state(&conn, id),
            (Some(card), true),
            "card payments go back to Transfers > Card Payment, still locked"
        );

        let ruled = child_category(&conn, "Misc");
        crate::rules::add_rule(&conn, "PAYMENT", false, ruled).unwrap();
        assert_eq!(
            row_state(&conn, id),
            (Some(card), true),
            "and user rules never win over them"
        );
    }

    #[test]
    fn clear_override_refusals() {
        let conn = open_in_memory().unwrap();
        assert!(matches!(
            clear_override(&conn, 999).unwrap_err(),
            DbError::UnknownTransaction(999)
        ));
    }

    #[test]
    fn apply_rule_from_merchant_categorizes_the_group() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let edited = add_transaction(&conn, "COFFEE    VANCOUVER BC", false, None);
        let twin = add_transaction(&conn, "COFFEE    NEW WEST", false, None);
        let other = add_transaction(&conn, "BISTRO  VANCOUVER", false, None);
        set_category(&conn, edited, cat).unwrap();

        let rule = apply_rule_from_merchant(&conn, edited, cat).unwrap();
        assert_eq!(
            crate::rules::list_rules(&conn)
                .unwrap()
                .iter()
                .map(|r| r.pattern.as_str())
                .collect::<Vec<_>>(),
            ["COFFEE"],
            "the rule is the cleaned merchant"
        );
        assert_eq!(row_state(&conn, twin), (Some(cat), false), "the group");
        assert_eq!(row_state(&conn, other), (None, false), "non-matches stay");
        assert_eq!(
            row_state(&conn, edited),
            (Some(cat), true),
            "the edited row keeps its manual lock"
        );
        assert!(rule > 0, "add_rule returned the new rule's id");
    }

    #[test]
    fn apply_rule_uses_the_cleaned_merchant_not_the_description() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Rides");
        let id = add_transaction(&conn, "LYFT   *RIDE 11-05  VANCOUVER", false, None);

        apply_rule_from_merchant(&conn, id, cat).unwrap();
        assert_eq!(
            crate::rules::list_rules(&conn).unwrap()[0].pattern,
            "LYFT",
            "text before the first double space"
        );
    }

    #[test]
    fn apply_rule_refusals() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let parent = create_parent(&conn, "Home", CategoryKind::Spend).unwrap();
        let id = add_transaction(&conn, "COFFEE VANCOUVER", false, None);
        let blank = add_transaction(&conn, "   ", false, None);

        assert!(matches!(
            apply_rule_from_merchant(&conn, 999, cat).unwrap_err(),
            DbError::UnknownTransaction(999)
        ));
        assert!(matches!(
            apply_rule_from_merchant(&conn, id, parent).unwrap_err(),
            DbError::ParentCategoryNotAssignable(id_) if id_ == parent
        ));
        assert!(matches!(
            apply_rule_from_merchant(&conn, blank, cat).unwrap_err(),
            DbError::EmptyMerchant(id_) if id_ == blank
        ));
        assert!(
            crate::rules::list_rules(&conn).unwrap().is_empty(),
            "refusals create no rule"
        );
    }
}
