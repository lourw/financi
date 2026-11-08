//! Storage for category rules (table `category_rules`): CRUD plus
//! validation-on-save. The matching semantics live in
//! [`financi_core::rules`]; a rule targets a child category, enforced here
//! with [`ensure_assignable`]. Every create, edit and delete re-runs all
//! rules over the unlocked transactions ([`reapply_rules`]) in the same SQL
//! transaction as the rule change.

use financi_core::rules::{Rule, pick_category, validate_regex};
use rusqlite::{Connection, OptionalExtension};

use crate::categories::ensure_assignable;
use crate::{DbError, Result};

fn fetch_rule(conn: &Connection, id: i64) -> Result<Rule> {
    conn.query_row(
        "SELECT id, pattern, is_regex, category_id FROM category_rules WHERE id = ?1",
        [id],
        |r| {
            Ok(Rule {
                id: r.get(0)?,
                pattern: r.get(1)?,
                is_regex: r.get(2)?,
                category_id: r.get(3)?,
            })
        },
    )
    .optional()?
    .ok_or(DbError::UnknownRule(id))
}

/// Shared validation for add and update: non-empty pattern (trimmed), a
/// valid regex when `is_regex`, and a child target category.
fn validate_rule(
    conn: &Connection,
    pattern: &str,
    is_regex: bool,
    category_id: i64,
) -> Result<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(DbError::EmptyRulePattern);
    }
    if is_regex {
        validate_regex(pattern).map_err(|message| DbError::InvalidRulePattern {
            pattern: pattern.to_string(),
            message,
        })?;
    }
    ensure_assignable(conn, category_id)?;
    Ok(pattern.to_string())
}

/// All rules, oldest first (by id). Feed straight to
/// [`financi_core::rules::pick_category`].
pub fn list_rules(conn: &Connection) -> Result<Vec<Rule>> {
    let mut stmt =
        conn.prepare("SELECT id, pattern, is_regex, category_id FROM category_rules ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(Rule {
            id: r.get(0)?,
            pattern: r.get(1)?,
            is_regex: r.get(2)?,
            category_id: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Create a rule targeting the child category `category_id`. Returns its
/// id. Rejects empty patterns, invalid regex patterns and parent targets.
pub fn add_rule(conn: &Connection, pattern: &str, is_regex: bool, category_id: i64) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let pattern = validate_rule(&tx, pattern, is_regex, category_id)?;
    tx.execute(
        "INSERT INTO category_rules (pattern, is_regex, category_id) VALUES (?1, ?2, ?3)",
        (pattern, is_regex, category_id),
    )?;
    let id = tx.last_insert_rowid();
    reapply_rules(&tx)?;
    tx.commit()?;
    Ok(id)
}

/// Replace a rule's pattern, kind and target category, with the same
/// validation as [`add_rule`].
pub fn update_rule(
    conn: &Connection,
    id: i64,
    pattern: &str,
    is_regex: bool,
    category_id: i64,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let pattern = validate_rule(&tx, pattern, is_regex, category_id)?;
    let updated = tx.execute(
        "UPDATE category_rules SET pattern = ?1, is_regex = ?2, category_id = ?3 WHERE id = ?4",
        (pattern, is_regex, category_id, id),
    )?;
    if updated == 0 {
        return Err(DbError::UnknownRule(id));
    }
    reapply_rules(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Delete a rule. Rows it used to match are recategorized by the remaining
/// rules (or become Uncategorized) via [`reapply_rules`].
pub fn delete_rule(conn: &Connection, id: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    fetch_rule(&tx, id)?;
    tx.execute("DELETE FROM category_rules WHERE id = ?1", [id])?;
    reapply_rules(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Re-run every rule over all unlocked transactions: each row gets the
/// winning rule's category, or `NULL` (Uncategorized) when nothing matches.
/// Locked rows (`category_locked = 1`) are never touched — manual overrides
/// (2.6) and the card payments locked at import keep their category.
pub fn reapply_rules(conn: &Connection) -> Result<()> {
    let rules = list_rules(conn)?;
    let rows: Vec<(i64, String, Option<i64>)> = {
        let mut stmt = conn.prepare(
            "SELECT id, description_raw, category_id FROM transactions WHERE category_locked = 0",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (id, description, current) in rows {
        let category_id = pick_category(&rules, &description);
        if category_id != current {
            conn.execute(
                "UPDATE transactions SET category_id = ?1 WHERE id = ?2",
                (category_id, id),
            )?;
        }
    }
    Ok(())
}

/// How many unlocked transactions the pattern matches, for the live
/// preview in the rule form ("matches 14 transactions"). Matching follows
/// [`financi_core::rules::pattern_matches`]: case-insensitive substring or
/// case-insensitive regex; an empty or invalid-regex pattern matches
/// nothing. When `exclude_rule_id` is set (editing an existing rule), rows
/// the excluded rule already matches are left out, so the count shows what
/// the edit would newly categorize.
pub fn count_rule_matches(
    conn: &Connection,
    pattern: &str,
    is_regex: bool,
    exclude_rule_id: Option<i64>,
) -> Result<i64> {
    let excluded = match exclude_rule_id {
        Some(id) => Some(fetch_rule(conn, id)?),
        None => None,
    };
    let mut stmt =
        conn.prepare("SELECT description_raw FROM transactions WHERE category_locked = 0")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut count = 0;
    for row in rows {
        let description = row?;
        if !financi_core::rules::pattern_matches(pattern, is_regex, &description) {
            continue;
        }
        if let Some(old) = &excluded
            && financi_core::rules::pattern_matches(&old.pattern, old.is_regex, &description)
        {
            continue;
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::{create_child, create_parent};
    use crate::open_in_memory;
    use financi_core::CategoryKind;

    fn child_category(conn: &Connection) -> i64 {
        let parent = create_parent(conn, "Food", CategoryKind::Spend).unwrap();
        create_child(conn, parent, "Groceries").unwrap()
    }

    /// A stored transaction on its own account and import, so re-runs see
    /// it. Returns the transaction id.
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
            financi_core::AccountKind::CreditCard,
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

    fn category_of(conn: &Connection, id: i64) -> Option<i64> {
        conn.query_row(
            "SELECT category_id FROM transactions WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn add_and_list_round_trip() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let id1 = add_rule(&conn, "COFFEE", false, cat).unwrap();
        let id2 = add_rule(&conn, r"CAFE\s+\w+", true, cat).unwrap();
        assert!(id1 < id2);

        let rules = list_rules(&conn).unwrap();
        assert_eq!(
            rules,
            vec![
                Rule {
                    id: id1,
                    pattern: "COFFEE".into(),
                    is_regex: false,
                    category_id: cat,
                },
                Rule {
                    id: id2,
                    pattern: r"CAFE\s+\w+".into(),
                    is_regex: true,
                    category_id: cat,
                },
            ],
            "listed by id"
        );
    }

    #[test]
    fn add_rejects_empty_and_whitespace_patterns() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        assert!(matches!(
            add_rule(&conn, "", false, cat).unwrap_err(),
            DbError::EmptyRulePattern
        ));
        assert!(matches!(
            add_rule(&conn, "   ", true, cat).unwrap_err(),
            DbError::EmptyRulePattern
        ));
    }

    #[test]
    fn add_trims_the_pattern() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let id = add_rule(&conn, "  COFFEE  ", false, cat).unwrap();
        assert_eq!(list_rules(&conn).unwrap()[0].pattern, "COFFEE");
        assert_eq!(fetch_rule(&conn, id).unwrap().pattern, "COFFEE");
    }

    #[test]
    fn add_rejects_invalid_regex_but_not_regex_like_substrings() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let err = add_rule(&conn, "(unclosed", true, cat).unwrap_err();
        match err {
            DbError::InvalidRulePattern { pattern, message } => {
                assert_eq!(pattern, "(unclosed");
                assert!(
                    !message.is_empty(),
                    "the regex error is readable: {message}"
                );
            }
            other => panic!("expected InvalidRulePattern, got {other:?}"),
        }
        // The same text as a plain substring is literal and fine.
        add_rule(&conn, "(unclosed", false, cat).unwrap();
    }

    #[test]
    fn add_rejects_parents_and_unknown_categories() {
        let conn = open_in_memory().unwrap();
        let parent = create_parent(&conn, "Food", CategoryKind::Spend).unwrap();
        assert!(matches!(
            add_rule(&conn, "COFFEE", false, parent).unwrap_err(),
            DbError::ParentCategoryNotAssignable(id) if id == parent
        ));
        assert!(matches!(
            add_rule(&conn, "COFFEE", false, 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        assert!(list_rules(&conn).unwrap().is_empty());
    }

    #[test]
    fn update_replaces_pattern_kind_and_category() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let other = create_parent(&conn, "Home", CategoryKind::Spend).unwrap();
        let other_cat = create_child(&conn, other, "Utilities").unwrap();
        let id = add_rule(&conn, "COFFEE", false, cat).unwrap();

        update_rule(&conn, id, r"cafe\s*\d", true, other_cat).unwrap();
        assert_eq!(
            fetch_rule(&conn, id).unwrap(),
            Rule {
                id,
                pattern: r"cafe\s*\d".into(),
                is_regex: true,
                category_id: other_cat,
            }
        );
    }

    #[test]
    fn update_refusals_leave_the_rule_in_place() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let id = add_rule(&conn, "COFFEE", false, cat).unwrap();
        let before = fetch_rule(&conn, id).unwrap();

        assert!(matches!(
            update_rule(&conn, id, "", false, cat).unwrap_err(),
            DbError::EmptyRulePattern
        ));
        assert!(matches!(
            update_rule(&conn, id, "[bad", true, cat).unwrap_err(),
            DbError::InvalidRulePattern { .. }
        ));
        assert!(matches!(
            update_rule(&conn, id, "TEA", false, 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        assert!(matches!(
            update_rule(&conn, 999, "TEA", false, cat).unwrap_err(),
            DbError::UnknownRule(999)
        ));
        assert_eq!(fetch_rule(&conn, id).unwrap(), before);
    }

    #[test]
    fn delete_removes_only_that_rule() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let keep = add_rule(&conn, "COFFEE", false, cat).unwrap();
        let gone = add_rule(&conn, "TEA", false, cat).unwrap();

        delete_rule(&conn, gone).unwrap();
        assert_eq!(
            list_rules(&conn)
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            [keep]
        );
        assert!(matches!(
            delete_rule(&conn, gone).unwrap_err(),
            DbError::UnknownRule(id) if id == gone
        ));
    }

    #[test]
    fn adding_a_rule_categorizes_unlocked_rows_only() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let other_parent = create_parent(&conn, "Home", CategoryKind::Spend).unwrap();
        let other = create_child(&conn, other_parent, "Utilities").unwrap();

        let coffee = add_transaction(&conn, "COFFEE VANCOUVER", false, None);
        let bistro = add_transaction(&conn, "BISTRO VANCOUVER", false, None);
        let locked = add_transaction(&conn, "COFFEE EXPRESS", true, None);
        let locked_with_cat = add_transaction(&conn, "COFFEE BAR", true, Some(other));

        add_rule(&conn, "COFFEE", false, cat).unwrap();

        assert_eq!(category_of(&conn, coffee), Some(cat));
        assert_eq!(category_of(&conn, bistro), None, "no rule, stays NULL");
        assert_eq!(
            category_of(&conn, locked),
            None,
            "locked rows are never touched"
        );
        assert_eq!(
            category_of(&conn, locked_with_cat),
            Some(other),
            "a locked row keeps its own category"
        );
    }

    #[test]
    fn deleting_a_rule_uncategorizes_its_rows() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let coffee = add_transaction(&conn, "COFFEE VANCOUVER", false, None);
        let rule = add_rule(&conn, "COFFEE", false, cat).unwrap();
        assert_eq!(category_of(&conn, coffee), Some(cat), "added by the rule");

        delete_rule(&conn, rule).unwrap();
        assert_eq!(
            category_of(&conn, coffee),
            None,
            "matching nothing makes it Uncategorized again"
        );
    }

    #[test]
    fn editing_a_rule_moves_rows_to_the_new_winner() {
        let conn = open_in_memory().unwrap();
        let coffee_cat = child_category(&conn);
        let home = create_parent(&conn, "Home", CategoryKind::Spend).unwrap();
        let bistro_cat = create_child(&conn, home, "Restaurants").unwrap();

        let coffee = add_transaction(&conn, "COFFEE VANCOUVER", false, None);
        let bistro = add_transaction(&conn, "BISTRO VANCOUVER", false, None);
        let rule = add_rule(&conn, "COFFEE", false, coffee_cat).unwrap();
        assert_eq!(category_of(&conn, coffee), Some(coffee_cat));

        update_rule(&conn, rule, "BISTRO", false, bistro_cat).unwrap();
        assert_eq!(category_of(&conn, coffee), None, "the old winner is undone");
        assert_eq!(category_of(&conn, bistro), Some(bistro_cat));
    }

    #[test]
    fn a_refused_rule_change_leaves_rows_untouched() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        let coffee = add_transaction(&conn, "COFFEE VANCOUVER", false, None);
        let rule = add_rule(&conn, "COFFEE", false, cat).unwrap();

        assert!(update_rule(&conn, rule, "[bad", true, cat).is_err());
        assert_eq!(category_of(&conn, coffee), Some(cat));
    }

    #[test]
    fn count_rule_matches_unlocked_rows_only() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        add_transaction(&conn, "COFFEE VANCOUVER", false, None);
        add_transaction(&conn, "coffee bean roastery", false, None);
        add_transaction(&conn, "COFFEE LOCKED", true, Some(cat));
        add_transaction(&conn, "GROCERY STORE", false, None);

        assert_eq!(count_rule_matches(&conn, "COFFEE", false, None).unwrap(), 2);
        assert_eq!(
            count_rule_matches(&conn, r"^coffee\b", true, None).unwrap(),
            2
        );
        assert_eq!(count_rule_matches(&conn, "[bad", true, None).unwrap(), 0);
        assert_eq!(count_rule_matches(&conn, "", false, None).unwrap(), 0);
    }

    #[test]
    fn count_rule_matches_excludes_the_edited_rule() {
        let conn = open_in_memory().unwrap();
        let cat = child_category(&conn);
        add_transaction(&conn, "COFFEE VANCOUVER", false, None);
        add_transaction(&conn, "BEAN BURRITO", false, None);
        let rule = add_rule(&conn, "COFFEE", false, cat).unwrap();

        let without = count_rule_matches(&conn, "COFFEE|BEAN", true, None).unwrap();
        let edited = count_rule_matches(&conn, "COFFEE|BEAN", true, Some(rule)).unwrap();
        assert_eq!(without, 2);
        assert_eq!(
            edited, 1,
            "the row the old pattern already matched is left out"
        );
    }
}
