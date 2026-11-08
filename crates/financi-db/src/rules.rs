//! Storage for category rules (table `category_rules`): CRUD plus
//! validation-on-save. The matching semantics live in
//! [`financi_core::rules`]; a rule targets a child category, enforced here
//! with [`ensure_assignable`]. Re-running rules over transactions on
//! create/edit/delete is task 2.5.

use financi_core::rules::{Rule, validate_regex};
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
    let pattern = validate_rule(conn, pattern, is_regex, category_id)?;
    conn.execute(
        "INSERT INTO category_rules (pattern, is_regex, category_id) VALUES (?1, ?2, ?3)",
        (pattern, is_regex, category_id),
    )?;
    Ok(conn.last_insert_rowid())
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
    let pattern = validate_rule(conn, pattern, is_regex, category_id)?;
    let updated = conn.execute(
        "UPDATE category_rules SET pattern = ?1, is_regex = ?2, category_id = ?3 WHERE id = ?4",
        (pattern, is_regex, category_id, id),
    )?;
    if updated == 0 {
        return Err(DbError::UnknownRule(id));
    }
    Ok(())
}

/// Delete a rule. Rows it used to match are left as they are; re-running
/// the remaining rules over them is task 2.5.
pub fn delete_rule(conn: &Connection, id: i64) -> Result<()> {
    fetch_rule(conn, id)?;
    conn.execute("DELETE FROM category_rules WHERE id = ?1", [id])?;
    Ok(())
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
}
