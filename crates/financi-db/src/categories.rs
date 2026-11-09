//! Category tree: top-level parents with children underneath, each with a
//! [`CategoryKind`]. Children inherit their parent's kind; transactions are
//! only ever assigned to children. The `Transfers` tree inserted by
//! migration 0002 is `is_system` and cannot be renamed, moved or deleted.

use std::collections::HashMap;

use financi_core::CategoryKind;
use rusqlite::{Connection, OptionalExtension};

use crate::{DbError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryNode {
    pub id: i64,
    pub name: String,
    pub kind: CategoryKind,
    /// System categories (the `Transfers` tree) can't be renamed, moved
    /// or deleted; the UI uses this to disable those actions.
    pub is_system: bool,
    /// Empty for children and for parents without any.
    pub children: Vec<CategoryNode>,
}

fn kind_from_str(s: &str) -> Option<CategoryKind> {
    match s {
        "expense" => Some(CategoryKind::Expense),
        "income" => Some(CategoryKind::Income),
        "transfer" => Some(CategoryKind::Transfer),
        _ => None,
    }
}

/// All parents with their children, both sorted by name.
pub fn list_categories(conn: &Connection) -> Result<Vec<CategoryNode>> {
    let mut stmt =
        conn.prepare("SELECT id, name, kind, is_system, parent_id FROM categories ORDER BY name")?;
    let mut rows = stmt.query([])?;
    let mut parents: Vec<CategoryNode> = Vec::new();
    let mut children: HashMap<i64, Vec<CategoryNode>> = HashMap::new();
    while let Some(row) = rows.next()? {
        let kind_text: String = row.get(2)?;
        let kind = kind_from_str(&kind_text).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                kind_text.clone().into(),
            )
        })?;
        let node = CategoryNode {
            id: row.get(0)?,
            name: row.get(1)?,
            kind,
            is_system: row.get(3)?,
            children: Vec::new(),
        };
        let parent_id: Option<i64> = row.get(4)?;
        match parent_id {
            None => parents.push(node),
            Some(p) => children.entry(p).or_default().push(node),
        }
    }
    for parent in &mut parents {
        if let Some(kids) = children.remove(&parent.id) {
            parent.children = kids;
        }
    }
    Ok(parents)
}

/// How many visible transactions each category owns. Categories without
/// any are absent from the map; parents never hold rows directly.
pub fn transaction_counts(conn: &Connection) -> Result<HashMap<i64, i64>> {
    let mut stmt = conn.prepare(
        "SELECT category_id, COUNT(*) FROM transactions
          WHERE hidden = 0 AND category_id IS NOT NULL
          GROUP BY category_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

struct CategoryRow {
    kind: CategoryKind,
    parent_id: Option<i64>,
    is_system: bool,
}

fn fetch_category(conn: &Connection, id: i64) -> Result<CategoryRow> {
    conn.query_row(
        "SELECT kind, parent_id, is_system FROM categories WHERE id = ?1",
        [id],
        |r| {
            let kind_text: String = r.get(0)?;
            Ok(CategoryRow {
                kind: kind_from_str(&kind_text).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        kind_text.clone().into(),
                    )
                })?,
                parent_id: r.get(1)?,
                is_system: r.get(2)?,
            })
        },
    )
    .optional()?
    .ok_or(DbError::UnknownCategory(id))
}

fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _)
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

fn insert_category(
    conn: &Connection,
    name: &str,
    parent_id: Option<i64>,
    kind: CategoryKind,
) -> Result<i64> {
    let inserted = conn.execute(
        "INSERT INTO categories (name, parent_id, kind) VALUES (?1, ?2, ?3)",
        (name, parent_id, kind.as_str()),
    );
    match inserted {
        Err(e) if is_unique_violation(&e) => Err(DbError::DuplicateCategory {
            name: name.to_string(),
        }),
        Err(e) => Err(e.into()),
        Ok(_) => Ok(conn.last_insert_rowid()),
    }
}

/// Create a top-level category. Refuses a duplicate top-level name.
pub fn create_parent(conn: &Connection, name: &str, kind: CategoryKind) -> Result<i64> {
    insert_category(conn, name.trim(), None, kind)
}

/// Create a child under the parent `parent_id`; the child inherits the
/// parent's kind. Refuses a duplicate name under the same parent.
pub fn create_child(conn: &Connection, parent_id: i64, name: &str) -> Result<i64> {
    let parent = fetch_category(conn, parent_id)?;
    if parent.parent_id.is_some() {
        return Err(DbError::CategoryHasParent(parent_id));
    }
    insert_category(conn, name.trim(), Some(parent_id), parent.kind)
}

/// Rename a category. System categories can't be renamed; the new name
/// must not collide with a sibling.
pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let category = fetch_category(conn, id)?;
    if category.is_system {
        return Err(DbError::SystemCategory {
            id,
            action: "renamed",
        });
    }
    let name = name.trim();
    let updated = conn.execute("UPDATE categories SET name = ?1 WHERE id = ?2", (name, id));
    match updated {
        Err(e) if is_unique_violation(&e) => Err(DbError::DuplicateCategory {
            name: name.to_string(),
        }),
        Err(e) => Err(e.into()),
        Ok(_) => Ok(()),
    }
}

/// Move a child under another parent of the same kind. Parents, system
/// categories and new parents that are themselves children are refused.
pub fn move_child(conn: &Connection, child_id: i64, new_parent_id: i64) -> Result<()> {
    let child = fetch_category(conn, child_id)?;
    if child.parent_id.is_none() {
        return Err(DbError::CannotMoveParent(child_id));
    }
    if child.is_system {
        return Err(DbError::SystemCategory {
            id: child_id,
            action: "moved",
        });
    }
    let new_parent = fetch_category(conn, new_parent_id)?;
    if new_parent.parent_id.is_some() {
        return Err(DbError::CategoryHasParent(new_parent_id));
    }
    if new_parent.kind != child.kind {
        return Err(DbError::CategoryKindMismatch {
            child: child_id,
            parent: new_parent_id,
        });
    }
    conn.execute(
        "UPDATE categories SET parent_id = ?1 WHERE id = ?2",
        (new_parent_id, child_id),
    )?;
    Ok(())
}

/// Delete a category after moving its transactions and rules to
/// `reassign_to`, which must be an existing child. Deleting a parent
/// requires it to have no children. System categories can't be deleted.
/// The reassignment and the delete happen in one SQL transaction.
pub fn delete_category(conn: &Connection, id: i64, reassign_to: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let category = fetch_category(&tx, id)?;
    if category.is_system {
        return Err(DbError::SystemCategory {
            id,
            action: "deleted",
        });
    }
    if id == reassign_to {
        return Err(DbError::CategoryReassignToSelf(id));
    }
    let target = fetch_category(&tx, reassign_to)?;
    if target.parent_id.is_none() {
        return Err(DbError::ParentCategoryNotAssignable(reassign_to));
    }
    let children: i64 = tx.query_row(
        "SELECT COUNT(*) FROM categories WHERE parent_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if children > 0 {
        return Err(DbError::CategoryHasChildren(id));
    }
    tx.execute(
        "UPDATE transactions SET category_id = ?1 WHERE category_id = ?2",
        (reassign_to, id),
    )?;
    tx.execute(
        "UPDATE category_rules SET category_id = ?1 WHERE category_id = ?2",
        (reassign_to, id),
    )?;
    tx.execute("DELETE FROM categories WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

/// Refuse assigning a transaction to a parent or to a category that
/// doesn't exist. Every code path that writes `transactions.category_id`
/// (rules in 2.5, manual edits in 2.6) must call this first.
pub fn ensure_assignable(conn: &Connection, category_id: i64) -> Result<()> {
    let category = fetch_category(conn, category_id)?;
    if category.parent_id.is_none() {
        return Err(DbError::ParentCategoryNotAssignable(category_id));
    }
    Ok(())
}

/// The system `Transfers > Card Payment` category that migration 0002 puts
/// in every database, looked up by name and parent — ids are never
/// hardcoded. A missing category means a broken database.
pub fn card_payment_category(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT c.id
           FROM categories c
           JOIN categories p ON p.id = c.parent_id
          WHERE c.name = 'Card Payment' AND p.name = 'Transfers' AND p.parent_id IS NULL",
        [],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(DbError::MissingSystemCategory("Transfers > Card Payment"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_account, open_in_memory};
    use financi_core::AccountKind;

    /// A transaction on its own account and import, categorized with
    /// `category_id`. Returns the transaction id.
    fn add_transaction(conn: &Connection, category_id: Option<i64>) -> i64 {
        let account = create_account(
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
                                       posted_date, amount_cents, description_raw, category_id)
             VALUES (?1, ?2, 0, X'00', ?3, '2026-10-31', -450, 'COFFEE', ?4)",
            (
                account,
                import,
                uuid::Uuid::new_v4().to_string(),
                category_id,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn add_rule(conn: &Connection, category_id: i64) -> i64 {
        conn.execute(
            "INSERT INTO category_rules (pattern, category_id) VALUES ('BEANERY', ?1)",
            [category_id],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn category_id_of(conn: &Connection, name: &str) -> i64 {
        conn.query_row("SELECT id FROM categories WHERE name = ?1", [name], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn parent_id_of(conn: &Connection, id: i64) -> Option<i64> {
        conn.query_row(
            "SELECT parent_id FROM categories WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn list_returns_parents_with_children_sorted_by_name() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        create_child(&conn, food, "Restaurants").unwrap();
        create_child(&conn, food, "Groceries").unwrap();

        let tree = list_categories(&conn).unwrap();
        let names: Vec<_> = tree
            .iter()
            .map(|p| (p.name.clone(), p.kind, p.is_system))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Food".to_string(), CategoryKind::Expense, false),
                ("Transfers".to_string(), CategoryKind::Transfer, true),
            ]
        );
        assert_eq!(
            tree[1].children.iter().map(|c| &c.name).collect::<Vec<_>>(),
            ["Between Accounts", "Card Payment"]
        );
        assert!(tree[1].children[0].is_system);
        let food = &tree[0];
        assert_eq!(
            food.children.iter().map(|c| &c.name).collect::<Vec<_>>(),
            ["Groceries", "Restaurants"],
            "children sorted by name"
        );
        assert_eq!(food.children[0].kind, CategoryKind::Expense);
    }

    #[test]
    fn child_inherits_the_parents_kind() {
        let conn = open_in_memory().unwrap();
        let spend = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let income = create_parent(&conn, "Income", CategoryKind::Income).unwrap();
        create_child(&conn, spend, "Groceries").unwrap();
        create_child(&conn, income, "Refunds").unwrap();

        let kinds: Vec<_> = list_categories(&conn)
            .unwrap()
            .iter()
            .flat_map(|p| p.children.iter())
            .filter(|c| !c.is_system)
            .map(|c| (c.name.clone(), c.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("Groceries".to_string(), CategoryKind::Expense),
                ("Refunds".to_string(), CategoryKind::Income),
            ]
        );
    }

    #[test]
    fn duplicate_names_are_refused_within_the_same_level() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let dup = create_parent(&conn, "Food", CategoryKind::Income).unwrap_err();
        assert!(matches!(dup, DbError::DuplicateCategory { .. }), "{dup}");
        create_child(&conn, food, "Groceries").unwrap();
        let dup = create_child(&conn, food, "Groceries").unwrap_err();
        assert!(matches!(dup, DbError::DuplicateCategory { .. }), "{dup}");
        let home = create_parent(&conn, "Home", CategoryKind::Expense).unwrap();
        create_child(&conn, home, "Groceries").expect("other parent is fine");
    }

    #[test]
    fn create_child_refuses_unknown_parents_and_children_as_parents() {
        let conn = open_in_memory().unwrap();
        assert!(matches!(
            create_child(&conn, 999, "X").unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        let err = create_child(&conn, groceries, "Beans").unwrap_err();
        assert!(matches!(err, DbError::CategoryHasParent(id) if id == groceries));
    }

    #[test]
    fn rename_updates_parents_and_children() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        rename(&conn, groceries, "Supermarket").unwrap();
        rename(&conn, food, "Food & Drinks").unwrap();
        let tree = list_categories(&conn).unwrap();
        assert_eq!(tree[0].name, "Food & Drinks");
        assert_eq!(tree[0].children[0].name, "Supermarket");
    }

    #[test]
    fn rename_refuses_duplicates_and_system_categories() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        let home = create_parent(&conn, "Home", CategoryKind::Expense).unwrap();
        create_child(&conn, home, "Utilities").unwrap();
        create_child(&conn, food, "Beans").unwrap();
        // Same name under a *different* parent is fine; it's unique per parent.
        rename(&conn, groceries, "Utilities").expect("no sibling collision");
        let err = rename(&conn, groceries, "Beans").unwrap_err();
        assert!(matches!(err, DbError::DuplicateCategory { .. }), "{err}");

        let transfers = category_id_of(&conn, "Transfers");
        let card_payment = category_id_of(&conn, "Card Payment");
        let err = rename(&conn, transfers, "Moving").unwrap_err();
        assert!(matches!(err, DbError::SystemCategory { id, .. } if id == transfers));
        let err = rename(&conn, card_payment, "Card x").unwrap_err();
        assert!(matches!(err, DbError::SystemCategory { id, .. } if id == card_payment));
    }

    #[test]
    fn move_child_between_parents_of_the_same_kind() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        let eating_out = create_parent(&conn, "Eating Out", CategoryKind::Expense).unwrap();
        move_child(&conn, groceries, eating_out).unwrap();
        assert_eq!(parent_id_of(&conn, groceries), Some(eating_out));

        let tree = list_categories(&conn).unwrap();
        let names: Vec<_> = tree
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.children.iter().map(|c| c.id).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(names[0], ("Eating Out".to_string(), vec![groceries]));
        assert_eq!(names[1], ("Food".to_string(), vec![]));
    }

    #[test]
    fn move_child_refusals() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        let income = create_parent(&conn, "Income", CategoryKind::Income).unwrap();
        let card_payment = category_id_of(&conn, "Card Payment");
        let transfers = category_id_of(&conn, "Transfers");

        let err = move_child(&conn, groceries, income).unwrap_err();
        assert!(
            matches!(err, DbError::CategoryKindMismatch { child, parent }
                if child == groceries && parent == income)
        );
        let err = move_child(&conn, food, transfers).unwrap_err();
        assert!(matches!(err, DbError::CannotMoveParent(id) if id == food));
        let err = move_child(&conn, card_payment, transfers).unwrap_err();
        assert!(matches!(err, DbError::SystemCategory { id, .. } if id == card_payment));
        let err = move_child(&conn, groceries, groceries).unwrap_err();
        assert!(matches!(err, DbError::CategoryHasParent(id) if id == groceries));
        assert!(matches!(
            move_child(&conn, 999, food).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        assert!(matches!(
            move_child(&conn, groceries, 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
    }

    #[test]
    fn delete_category_reassigns_transactions_and_rules() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let coffee = create_child(&conn, food, "Coffee").unwrap();
        let restaurants = create_child(&conn, food, "Restaurants").unwrap();
        let tx1 = add_transaction(&conn, Some(coffee));
        let tx2 = add_transaction(&conn, Some(coffee));
        let rule = add_rule(&conn, coffee);

        delete_category(&conn, coffee, restaurants).unwrap();

        let gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE id = ?1",
                [coffee],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0);
        let moved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE category_id = ?1",
                [restaurants],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(moved, 2);
        let still_assigned: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE id IN (?1, ?2)",
                (tx1, tx2),
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            still_assigned, 2,
            "the transactions themselves are untouched"
        );
        let rule_category: i64 = conn
            .query_row(
                "SELECT category_id FROM category_rules WHERE id = ?1",
                [rule],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rule_category, restaurants);
    }

    #[test]
    fn deleting_a_parent_requires_no_children() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        let home = create_parent(&conn, "Home", CategoryKind::Expense).unwrap();
        let target = create_child(&conn, home, "Utilities").unwrap();

        let err = delete_category(&conn, food, target).unwrap_err();
        assert!(matches!(err, DbError::CategoryHasChildren(id) if id == food));

        delete_category(&conn, groceries, target).unwrap();
        delete_category(&conn, food, target).expect("empty parent deletes");
        let gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE id = ?1",
                [food],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0);
    }

    #[test]
    fn delete_refusals_leave_everything_in_place() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let coffee = create_child(&conn, food, "Coffee").unwrap();
        let restaurants = create_child(&conn, food, "Restaurants").unwrap();
        let tx = add_transaction(&conn, Some(coffee));
        let card_payment = category_id_of(&conn, "Card Payment");

        let err = delete_category(&conn, coffee, food).unwrap_err();
        assert!(matches!(err, DbError::ParentCategoryNotAssignable(id) if id == food));
        let err = delete_category(&conn, coffee, coffee).unwrap_err();
        assert!(matches!(err, DbError::CategoryReassignToSelf(id) if id == coffee));
        assert!(matches!(
            delete_category(&conn, coffee, 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
        let err = delete_category(&conn, card_payment, restaurants).unwrap_err();
        assert!(matches!(err, DbError::SystemCategory { id, .. } if id == card_payment));
        assert!(matches!(
            delete_category(&conn, 999, restaurants).unwrap_err(),
            DbError::UnknownCategory(999)
        ));

        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE id = ?1 AND category_id = ?2",
                (tx, coffee),
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "nothing moved or deleted after the refusals");
    }

    #[test]
    fn deleting_a_system_category_is_refused_even_with_a_valid_target() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let target = create_child(&conn, food, "Groceries").unwrap();
        for name in ["Transfers", "Card Payment", "Between Accounts"] {
            let id = category_id_of(&conn, name);
            let err = delete_category(&conn, id, target).unwrap_err();
            assert!(
                matches!(err, DbError::SystemCategory { id: s, .. } if s == id),
                "{name}: {err}"
            );
        }
    }

    #[test]
    fn transactions_may_only_be_assigned_to_children() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        ensure_assignable(&conn, groceries).unwrap();
        let err = ensure_assignable(&conn, food).unwrap_err();
        assert!(matches!(err, DbError::ParentCategoryNotAssignable(id) if id == food));
        assert!(matches!(
            ensure_assignable(&conn, 999).unwrap_err(),
            DbError::UnknownCategory(999)
        ));
    }

    #[test]
    fn card_payment_category_finds_the_system_child() {
        let conn = open_in_memory().unwrap();
        let id = card_payment_category(&conn).unwrap();
        assert_eq!(id, category_id_of(&conn, "Card Payment"));
        assert_eq!(
            parent_id_of(&conn, id),
            Some(category_id_of(&conn, "Transfers"))
        );

        // A database without it (a broken one) is reported, not hardcoded to.
        conn.execute("DELETE FROM categories WHERE id = ?1", [id])
            .unwrap();
        assert!(matches!(
            card_payment_category(&conn).unwrap_err(),
            DbError::MissingSystemCategory(_)
        ));
    }

    #[test]
    fn transaction_counts_per_category() {
        let conn = open_in_memory().unwrap();
        let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
        let groceries = create_child(&conn, food, "Groceries").unwrap();
        add_transaction(&conn, Some(groceries));
        add_transaction(&conn, Some(groceries));
        add_transaction(&conn, None);

        let counts = transaction_counts(&conn).unwrap();
        assert_eq!(counts.get(&groceries), Some(&2));
        assert_eq!(counts.get(&food), None, "a parent holds no rows itself");
    }
}
