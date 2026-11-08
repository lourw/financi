//! The "To categorize" queue (2.7): uncategorized transactions grouped by
//! cleaned merchant, and the action that categorizes a whole group at once.
//!
//! Uncategorized means `category_id IS NULL` — locked rows always carry a
//! category, so no lock check is needed. The group key is the *cleaned*
//! merchant ([`financi_core::parse::merchant`] over `description_raw`),
//! which SQL cannot compute, so the rows are grouped in Rust; fine at this
//! scale. Dates are stored as ISO `yyyy-mm-dd` strings, whose lexicographic
//! order matches chronological order, so min/max are plain string compares.
//!
//! [`categorize_group`] is the queue's action: it creates a substring rule
//! from the group's merchant via [`add_rule`], which re-runs the rules
//! (2.5) and categorizes the whole group.

use std::collections::HashMap;

use financi_core::parse::merchant;
use rusqlite::Connection;

use crate::Result;
use crate::rules::add_rule;

/// One queue entry: every uncategorized transaction of one merchant.
/// `total_cents` is the signed sum (negative = money out), never absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncategorizedGroup {
    pub merchant: String,
    pub count: i64,
    pub total_cents: i64,
    pub first_date: String,
    pub last_date: String,
}

/// The uncategorized transactions grouped by cleaned merchant, sorted by
/// count descending, ties by merchant ascending. Rows whose cleaned
/// merchant is empty are left out: they cannot become a rule pattern
/// ([`add_rule`] rejects empty patterns), so the queue could offer no
/// action for them.
pub fn uncategorized_by_merchant(conn: &Connection) -> Result<Vec<UncategorizedGroup>> {
    let mut groups: HashMap<String, UncategorizedGroup> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT description_raw, amount_cents, posted_date
               FROM transactions
              WHERE category_id IS NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (description_raw, amount_cents, posted_date) = row?;
            let merchant = merchant(&description_raw);
            if merchant.is_empty() {
                continue;
            }
            match groups.get_mut(&merchant) {
                Some(group) => {
                    group.count += 1;
                    group.total_cents += amount_cents;
                    if posted_date < group.first_date {
                        group.first_date = posted_date;
                    } else if posted_date > group.last_date {
                        group.last_date = posted_date;
                    }
                }
                None => {
                    groups.insert(
                        merchant,
                        UncategorizedGroup {
                            count: 1,
                            total_cents: amount_cents,
                            first_date: posted_date.clone(),
                            last_date: posted_date,
                            merchant: String::new(),
                        },
                    );
                }
            }
        }
    }
    let mut groups: Vec<UncategorizedGroup> = groups
        .into_iter()
        .map(|(name, mut group)| {
            group.merchant = name;
            group
        })
        .collect();
    groups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.merchant.cmp(&b.merchant))
    });
    Ok(groups)
}

/// Choose a category for a whole queue group: create a substring rule from
/// the group's merchant (as [`add_rule`] would, non-regex), which re-runs
/// the rules and categorizes every unlocked row of the group (2.5).
/// Refusals are `add_rule`'s: empty pattern, parent or unknown target
/// category. Returns the new rule's id.
pub fn categorize_group(conn: &Connection, merchant: &str, category_id: i64) -> Result<i64> {
    add_rule(conn, merchant, false, category_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DbError;
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

    /// An uncategorized (unlocked, `category_id NULL`) transaction.
    fn add_tx(conn: &Connection, description: &str, amount_cents: i64, posted_date: &str) -> i64 {
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
                                       posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, 0, X'00', ?3, ?4, ?5, ?6)",
            (
                account,
                import,
                uuid::Uuid::new_v4().to_string(),
                posted_date,
                amount_cents,
                description,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn category_of(conn: &Connection, id: i64) -> Option<i64> {
        conn.query_row(
            "SELECT category_id FROM transactions WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn groups_by_cleaned_merchant_with_signed_sums_and_dates() {
        let conn = open_in_memory().unwrap();
        add_tx(&conn, "COFFEE    VANCOUVER BC", -450, "2026-10-31");
        add_tx(&conn, "COFFEE    NEW WEST", 150, "2026-10-05");
        add_tx(&conn, "COFFEE    BURNABY", -200, "2026-11-02");
        add_tx(&conn, "BISTRO  VANCOUVER", -3000, "2026-10-20");

        assert_eq!(
            uncategorized_by_merchant(&conn).unwrap(),
            vec![
                UncategorizedGroup {
                    merchant: "COFFEE".into(),
                    count: 3,
                    total_cents: -500,
                    first_date: "2026-10-05".into(),
                    last_date: "2026-11-02".into(),
                },
                UncategorizedGroup {
                    merchant: "BISTRO".into(),
                    count: 1,
                    total_cents: -3000,
                    first_date: "2026-10-20".into(),
                    last_date: "2026-10-20".into(),
                },
            ],
            "different raw descriptions clean to one merchant; refund nets in"
        );
    }

    #[test]
    fn sorted_by_count_then_merchant() {
        let conn = open_in_memory().unwrap();
        for date in ["2026-10-01", "2026-10-02"] {
            add_tx(&conn, "COFFEE  X", -100, date);
            add_tx(&conn, "AMAZON.COM  SEATTLE", -1000, date);
        }
        add_tx(&conn, "BISTRO  X", -100, "2026-10-03");
        add_tx(&conn, "DELI  X", -100, "2026-10-04");

        assert_eq!(
            uncategorized_by_merchant(&conn)
                .unwrap()
                .into_iter()
                .map(|g| g.merchant)
                .collect::<Vec<_>>(),
            ["AMAZON.COM", "COFFEE", "BISTRO", "DELI"],
            "count desc, merchant asc on ties"
        );
    }

    #[test]
    fn categorized_rows_are_left_out() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let categorized = add_tx(&conn, "COFFEE  X", -100, "2026-10-01");
        conn.execute(
            "UPDATE transactions SET category_id = ?1 WHERE id = ?2",
            (cat, categorized),
        )
        .unwrap();
        add_tx(&conn, "COFFEE  Y", -200, "2026-10-02");

        let groups = uncategorized_by_merchant(&conn).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 1);
    }

    #[test]
    fn empty_cleaned_merchants_are_skipped() {
        let conn = open_in_memory().unwrap();
        add_tx(&conn, "   ", -100, "2026-10-01");
        add_tx(&conn, "", -100, "2026-10-02");
        add_tx(&conn, "DELI  X", -100, "2026-10-03");

        let groups = uncategorized_by_merchant(&conn).unwrap();
        assert_eq!(
            groups
                .iter()
                .map(|g| g.merchant.as_str())
                .collect::<Vec<_>>(),
            ["DELI"]
        );
    }

    #[test]
    fn categorize_group_creates_a_rule_and_categorizes_the_group() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let first = add_tx(&conn, "COFFEE    VANCOUVER BC", -450, "2026-10-31");
        let twin = add_tx(&conn, "COFFEE    NEW WEST", -350, "2026-11-02");
        let bistro = add_tx(&conn, "BISTRO  VANCOUVER", -3000, "2026-10-20");

        let rule = categorize_group(&conn, "COFFEE", cat).unwrap();
        assert!(rule > 0, "the new rule's id");
        assert_eq!(
            crate::rules::list_rules(&conn)
                .unwrap()
                .iter()
                .map(|r| r.pattern.as_str())
                .collect::<Vec<_>>(),
            ["COFFEE"],
            "a substring rule on the group's merchant"
        );
        assert_eq!(category_of(&conn, first), Some(cat));
        assert_eq!(category_of(&conn, twin), Some(cat), "the whole group");
        assert_eq!(category_of(&conn, bistro), None, "non-matches stay");
        assert!(
            uncategorized_by_merchant(&conn)
                .unwrap()
                .iter()
                .all(|g| g.merchant != "COFFEE"),
            "the group leaves the queue"
        );
    }

    #[test]
    fn categorize_group_refusals() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let parent = create_parent(&conn, "Home", CategoryKind::Spend).unwrap();
        add_tx(&conn, "COFFEE  X", -100, "2026-10-01");

        assert!(matches!(
            categorize_group(&conn, "COFFEE", parent).unwrap_err(),
            DbError::ParentCategoryNotAssignable(id) if id == parent
        ));
        assert!(matches!(
            categorize_group(&conn, "COFFEE", 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        assert!(matches!(
            categorize_group(&conn, "", cat).unwrap_err(),
            DbError::EmptyRulePattern
        ));
        assert!(
            crate::rules::list_rules(&conn).unwrap().is_empty(),
            "refusals create no rule"
        );
        assert_eq!(category_of(&conn, first_id(&conn)), None);
    }

    fn first_id(conn: &Connection) -> i64 {
        conn.query_row("SELECT id FROM transactions ORDER BY id LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap()
    }
}
