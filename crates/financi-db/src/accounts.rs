//! Institutions (parents) and their accounts (siblings under one
//! institution), each with an [`AccountKind`].

use financi_core::{AccountKey, AccountKind};
use rusqlite::{Connection, OptionalExtension, Row};

use crate::{DbError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Institution {
    pub id: i64,
    /// Stable key matching `StatementParser::institution()`, e.g. `"simplii"`.
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub account_key: AccountKey,
    pub institution: Institution,
    pub name: String,
    pub kind: AccountKind,
    pub currency: String,
}

impl Account {
    /// `"Simplii Financial · No Fee Chequing"`.
    pub fn display_name(&self) -> String {
        format!("{} · {}", self.institution.name, self.name)
    }
}

pub fn list_institutions(conn: &Connection) -> Result<Vec<Institution>> {
    let mut stmt = conn.prepare("SELECT id, key, name FROM institutions ORDER BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok(Institution {
            id: r.get(0)?,
            key: r.get(1)?,
            name: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Create an account under the institution with `institution_key`, with a
/// fresh stable `account_key`. Returns its row id.
pub fn create_account(
    conn: &Connection,
    institution_key: &str,
    name: &str,
    kind: AccountKind,
) -> Result<i64> {
    let institution_id: i64 = conn
        .query_row(
            "SELECT id FROM institutions WHERE key = ?1",
            [institution_key],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| DbError::UnknownInstitution(institution_key.to_string()))?;
    let name = name.trim();
    let key = uuid::Uuid::new_v4().to_string();
    let inserted = conn.execute(
        "INSERT INTO accounts (account_key, institution_id, name, kind) VALUES (?1, ?2, ?3, ?4)",
        (&key, institution_id, name, kind.as_str()),
    );
    match inserted {
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Err(DbError::DuplicateAccount {
                name: name.to_string(),
            })
        }
        Err(e) => Err(e.into()),
        Ok(_) => Ok(conn.last_insert_rowid()),
    }
}

const ACCOUNT_SELECT: &str =
    "SELECT a.id, a.account_key, a.name, a.kind, a.currency, i.id, i.key, i.name
       FROM accounts a JOIN institutions i ON i.id = a.institution_id";

fn account_from_row(r: &Row<'_>) -> rusqlite::Result<Account> {
    let kind: String = r.get(3)?;
    Ok(Account {
        id: r.get(0)?,
        account_key: AccountKey(r.get(1)?),
        name: r.get(2)?,
        kind: AccountKind::parse(&kind).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, kind.into())
        })?,
        currency: r.get(4)?,
        institution: Institution {
            id: r.get(5)?,
            key: r.get(6)?,
            name: r.get(7)?,
        },
    })
}

/// All accounts, grouped by institution then name.
pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn.prepare(&format!("{ACCOUNT_SELECT} ORDER BY i.name, a.name"))?;
    let rows = stmt.query_map([], account_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get_account(conn: &Connection, id: i64) -> Result<Account> {
    conn.query_row(
        &format!("{ACCOUNT_SELECT} WHERE a.id = ?1"),
        [id],
        account_from_row,
    )
    .optional()?
    .ok_or(DbError::UnknownAccount(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_in_memory;

    #[test]
    fn built_in_institutions_exist() {
        let conn = open_in_memory().unwrap();
        let keys: Vec<_> = list_institutions(&conn)
            .unwrap()
            .into_iter()
            .map(|i| i.key)
            .collect();
        assert_eq!(keys, ["amex", "simplii"]);
    }

    #[test]
    fn accounts_are_siblings_under_an_institution() {
        let conn = open_in_memory().unwrap();
        let chq =
            create_account(&conn, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();
        create_account(&conn, "simplii", "Visa", AccountKind::CreditCard).unwrap();
        create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();

        let all = list_accounts(&conn).unwrap();
        let names: Vec<_> = all.iter().map(Account::display_name).collect();
        assert_eq!(
            names,
            [
                "American Express · Cobalt",
                "Simplii Financial · No Fee Chequing",
                "Simplii Financial · Visa"
            ]
        );

        let a = get_account(&conn, chq).unwrap();
        assert_eq!(a.kind, AccountKind::Chequing);
        assert_eq!(a.institution.key, "simplii");
        assert_eq!(a.account_key.0.len(), 36, "a UUID");
    }

    #[test]
    fn account_names_are_unique_per_institution_only() {
        let conn = open_in_memory().unwrap();
        create_account(&conn, "simplii", "Visa", AccountKind::CreditCard).unwrap();
        create_account(&conn, "amex", "Visa", AccountKind::CreditCard)
            .expect("other institution is fine");
        let dup = create_account(&conn, "simplii", " Visa ", AccountKind::CreditCard).unwrap_err();
        assert!(matches!(dup, DbError::DuplicateAccount { .. }), "{dup}");
    }

    #[test]
    fn unknown_institution_and_account_are_errors() {
        let conn = open_in_memory().unwrap();
        let err = create_account(&conn, "td", "X", AccountKind::Chequing).unwrap_err();
        assert!(matches!(err, DbError::UnknownInstitution(_)));
        assert!(matches!(
            get_account(&conn, 99).unwrap_err(),
            DbError::UnknownAccount(99)
        ));
    }
}
