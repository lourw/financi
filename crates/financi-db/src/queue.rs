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
//! [`categorize_group`] is the queue's action: it creates a rule via
//! [`add_rule`], which re-runs the rules (2.5) and categorizes the whole
//! group and everything else the rule matches (3.95: the rule's pattern
//! may be written by the user, not just the merchant name).

use std::collections::HashMap;

use financi_core::parse::merchant;
use financi_core::rules::{CompiledPattern, validate_regex};
use rusqlite::Connection;

use crate::DbError;
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

/// The uncategorized, unhidden transactions grouped by cleaned merchant,
/// sorted by count descending, ties by merchant ascending. Hidden rows
/// (`set_hidden`, 2.9) are left out like everywhere else. Rows whose cleaned
/// merchant is empty are left out: they cannot become a rule pattern
/// ([`add_rule`] rejects empty patterns), so the queue could offer no
/// action for them.
pub fn uncategorized_by_merchant(conn: &Connection) -> Result<Vec<UncategorizedGroup>> {
    let mut groups: HashMap<String, UncategorizedGroup> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT description_raw, amount_cents, posted_date
               FROM transactions
              WHERE category_id IS NULL AND hidden = 0",
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

/// Choose a category for a whole queue group: create a rule and re-run
/// the rules, which categorizes every unlocked row the rule matches
/// (2.7, with the user-written pattern of 3.95). `pattern` is the rule's
/// pattern; `None` or whitespace-only means the group's merchant name as
/// a substring rule whatever `is_regex` says (2.7's behaviour), and any
/// other pattern is trimmed and added as given — a substring rule that
/// matches descriptions case-insensitively, or a regex rule when
/// `is_regex`. Refusals: a pattern that matches none of the group's
/// transactions (a typo must not leave the rows stuck in the queue while
/// adding a rule) errors with [`DbError::RuleDoesNotMatchMerchant`] and
/// adds nothing; the others are [`add_rule`]'s — empty pattern, invalid
/// regex, parent or unknown target category. Returns the new rule's id.
pub fn categorize_group(
    conn: &Connection,
    merchant: &str,
    pattern: Option<&str>,
    is_regex: bool,
    category_id: i64,
) -> Result<i64> {
    // A blank or absent pattern means "use the merchant": the merchant
    // name as a substring rule, whatever the regex switch says.
    let (pattern, is_regex) = match pattern.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => (p, is_regex),
        None => (merchant, false),
    };
    if pattern.is_empty() {
        return Err(DbError::EmptyRulePattern);
    }
    // Checked before the match test because `CompiledPattern` treats an
    // invalid regex as never-matching: without this a bad regex would
    // report "doesn't match" instead of `add_rule`'s regex error.
    if is_regex {
        validate_regex(pattern).map_err(|message| DbError::InvalidRulePattern {
            pattern: pattern.to_string(),
            message,
        })?;
    }
    if !pattern_matches_group(conn, merchant, &CompiledPattern::new(pattern, is_regex))? {
        return Err(DbError::RuleDoesNotMatchMerchant(merchant.to_string()));
    }
    add_rule(conn, pattern, is_regex, category_id)
}

/// Does `pattern` match at least one of the group's transactions — the
/// same rows [`uncategorized_by_merchant`] groups (uncategorized, not
/// hidden, cleaned merchant equal)? The comparison runs in Rust because
/// the merchant is parsed, not stored.
fn pattern_matches_group(
    conn: &Connection,
    merchant: &str,
    pattern: &CompiledPattern,
) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT description_raw FROM transactions
          WHERE category_id IS NULL AND hidden = 0",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        let description_raw = row?;
        if financi_core::parse::merchant(&description_raw) == merchant
            && pattern.is_match(&description_raw)
        {
            return Ok(true);
        }
    }
    Ok(false)
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
            CategoryKind::Expense,
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

        let rule = categorize_group(&conn, "COFFEE", None, false, cat).unwrap();
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
        let parent = create_parent(&conn, "Home", CategoryKind::Expense).unwrap();
        add_tx(&conn, "COFFEE  X", -100, "2026-10-01");

        assert!(matches!(
            categorize_group(&conn, "COFFEE", None, false, parent).unwrap_err(),
            DbError::ParentCategoryNotAssignable(name) if name == "Home"
        ));
        assert!(matches!(
            categorize_group(&conn, "COFFEE", None, false, 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        assert!(matches!(
            categorize_group(&conn, "", None, false, cat).unwrap_err(),
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

    // -- 3.95: the rule pattern the user writes while categorizing.

    /// A blank pattern — absent, empty or whitespace — means the merchant
    /// name as a substring rule, today's behaviour.
    #[test]
    fn blank_patterns_add_the_merchant_name_rule() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let coffee = add_tx(&conn, "COFFEE    VANCOUVER BC", -450, "2026-10-31");
        let deli = add_tx(&conn, "DELI    VANCOUVER", -800, "2026-11-01");
        let bistro = add_tx(&conn, "BISTRO    WEST", -900, "2026-11-02");

        categorize_group(&conn, "COFFEE", None, false, cat).unwrap();
        categorize_group(&conn, "DELI", Some("  "), false, cat).unwrap();
        categorize_group(&conn, "BISTRO", Some(""), false, cat).unwrap();

        assert_eq!(
            crate::rules::list_rules(&conn)
                .unwrap()
                .into_iter()
                .map(|r| (r.pattern, r.is_regex))
                .collect::<Vec<_>>(),
            vec![
                ("COFFEE".to_string(), false),
                ("DELI".to_string(), false),
                ("BISTRO".to_string(), false),
            ],
            "each blank form added its merchant's name as a substring rule"
        );
        assert_eq!(category_of(&conn, coffee), Some(cat));
        assert_eq!(category_of(&conn, deli), Some(cat));
        assert_eq!(category_of(&conn, bistro), Some(cat));
    }

    /// User follow-up (2026-11-15): a blank pattern is the merchant name
    /// even with the regex switch on.
    #[test]
    fn a_blank_pattern_with_the_switch_on_adds_the_merchant_substring_rule() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let row = add_tx(&conn, "COFFEE    VANCOUVER BC", -450, "2026-10-31");

        categorize_group(&conn, "COFFEE", Some("   "), true, cat).unwrap();

        let rules = crate::rules::list_rules(&conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, "COFFEE");
        assert!(
            !rules[0].is_regex,
            "the switch is ignored for the merchant rule"
        );
        assert_eq!(category_of(&conn, row), Some(cat));
    }

    /// A shorter written pattern replaces the merchant name, and every
    /// queue row it matches — other merchants' rows included — leaves the
    /// queue with it.
    #[test]
    fn a_written_pattern_categorizes_every_row_it_matches() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let a = add_tx(&conn, "CAFENZO    VANCOUVER BC", -450, "2026-10-31");
        let b = add_tx(&conn, "CAFENZO    BURNABY", -550, "2026-11-01");
        let c = add_tx(&conn, "CAFENZO EXPRESS  SEATTLE", -650, "2026-11-02");
        let other = add_tx(&conn, "BISTRO  VANCOUVER", -3000, "2026-10-20");

        let rule = categorize_group(&conn, "CAFENZO", Some(" cafenzo "), false, cat).unwrap();
        assert!(rule > 0);
        let rules = crate::rules::list_rules(&conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, "cafenzo", "the pattern is stored trimmed");
        assert!(!rules[0].is_regex);
        for id in [a, b, c] {
            assert_eq!(
                category_of(&conn, id),
                Some(cat),
                "every row the pattern matches is categorized"
            );
        }
        assert_eq!(category_of(&conn, other), None, "non-matches stay");
        assert_eq!(
            uncategorized_by_merchant(&conn)
                .unwrap()
                .iter()
                .map(|g| g.merchant.as_str())
                .collect::<Vec<_>>(),
            ["BISTRO"],
            "both merchants the pattern caught leave the queue"
        );
    }

    /// A typo'd pattern that matches none of the group's rows is refused,
    /// so it can't leave the rows stuck in the queue while adding a rule.
    #[test]
    fn a_non_matching_pattern_errors_and_adds_no_rule() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let row = add_tx(&conn, "COFFEE    VANCOUVER BC", -450, "2026-10-31");

        let err = categorize_group(&conn, "COFFEE", Some("TEA LEAF"), false, cat).unwrap_err();
        assert!(
            matches!(&err, DbError::RuleDoesNotMatchMerchant(m) if m == "COFFEE"),
            "{err}"
        );
        assert!(
            crate::rules::list_rules(&conn).unwrap().is_empty(),
            "a refused write adds no rule"
        );
        assert_eq!(category_of(&conn, row), None, "and categorizes nothing");
    }

    /// A regex pattern is added as a regex rule and categorizes the rows
    /// it matches — inside the group and in other merchants' groups; the
    /// group rows it misses stay in the queue.
    #[test]
    fn a_regex_pattern_adds_a_regex_rule() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let store = add_tx(&conn, "TIM HORTONS  VANCOUVER BC", -450, "2026-10-31");
        let outlet = add_tx(&conn, "TIM HORTONS  BURNABY REG 12", -550, "2026-11-01");
        let other = add_tx(
            &conn,
            "TIM HORTONS EXPRESS  REG 34 CALGARY",
            -650,
            "2026-11-02",
        );
        let bistro = add_tx(&conn, "BISTRO  VANCOUVER", -3000, "2026-10-20");

        categorize_group(&conn, "TIM HORTONS", Some(r"REG \d+"), true, cat).unwrap();

        let rules = crate::rules::list_rules(&conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, r"REG \d+");
        assert!(rules[0].is_regex, "added as a regex rule");
        assert_eq!(category_of(&conn, outlet), Some(cat));
        assert_eq!(
            category_of(&conn, other),
            Some(cat),
            "another merchant's matching row"
        );
        assert_eq!(
            category_of(&conn, store),
            None,
            "the group row it misses stays"
        );
        assert_eq!(category_of(&conn, bistro), None);
        assert!(
            uncategorized_by_merchant(&conn)
                .unwrap()
                .iter()
                .all(|g| g.merchant != "TIM HORTONS EXPRESS"),
            "the caught group is fully categorized and gone"
        );
    }

    /// An invalid regex fails with `add_rule`'s existing error — not the
    /// doesn't-match error — and adds nothing.
    #[test]
    fn an_invalid_regex_errors_and_adds_nothing() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn, "Coffee");
        let row = add_tx(&conn, "COFFEE    VANCOUVER BC", -450, "2026-10-31");

        let err = categorize_group(&conn, "COFFEE", Some("(unclosed"), true, cat).unwrap_err();
        assert!(
            matches!(&err, DbError::InvalidRulePattern { pattern, .. } if pattern == "(unclosed"),
            "{err}"
        );
        assert!(crate::rules::list_rules(&conn).unwrap().is_empty());
        assert_eq!(category_of(&conn, row), None);
    }
}
