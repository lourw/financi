//! Transfer pairing (4.2): runs automatically after every import, inside
//! the import's own transaction ([`crate::import::commit_import`]).
//!
//! Two kinds of pair, per `plan.md` (*Transfers*):
//!
//! - **Card payment**: a chequing row of `−X` and a credit-card row of `+X`
//!   already categorized `Transfers > Card Payment` (the parser's built-in
//!   detection from 2.5), posted dates within **5 days** of each other.
//! - **Own-account transfer**: rows in two *different* chequing/savings
//!   accounts, `−X` and `+X`, dates within **2 days**, and at least one
//!   `description_raw` containing `TRANSFER` (case-insensitive).
//!
//! A row is eligible only while it is *unpaired* (`transfer_pair_id IS
//! NULL`), *not excluded* (see below) and — except for the credit-card side
//! of a card payment, which the category condition already pins down —
//! *not category-locked*: a manually categorized row is a human decision
//! pairing never overrides.
//!
//! **Exactly one candidate → pair**: both rows get `transfer_pair_id`
//! pointing at each other and are categorized `Card Payment` or
//! `Between Accounts` with `category_locked = 1`, so rule re-runs (2.5)
//! never move them. **More than one candidate → review**: the ambiguous row
//! is recorded in `transfer_reviews` (migration 0003) with its candidate
//! ids, and the Transfers page asks.
//!
//! Determinism: candidates form a symmetric "could pair with" relation. A
//! row pairs automatically only when the relation between the two is
//! mutual-singleton — each row's *only* candidate is the other. Two rows
//! that both claim the same single candidate therefore never race: the
//! shared candidate has more than one candidate and goes to review instead,
//! its candidate list ordered by posted date then id. Rows processed in id
//! order, so a run's writes never depend on iteration accidents.
//!
//! **Exclusion**: "Not a transfer" sets `transfer_excluded = 1` on the row,
//! the pairing-side mirror of `category_locked` — later runs skip the row
//! for good. [`unpair_transaction`] excludes both sides too, so an unpair
//! the user chose by hand is never silently re-paired by the next import.
//!
//! Every run is idempotent: it recomputes `transfer_reviews` from scratch
//! (deleting stale ones the user's other edits may have produced) and only
//! ever touches rows that are still unpaired.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::{Connection, params};

use crate::categories::{between_accounts_category, card_payment_category};
use crate::overrides::clear_override;
use crate::{DbError, Result};

/// Card payments pair within this many days.
const CARD_PAYMENT_WINDOW_DAYS: i64 = 5;
/// Own-account transfers pair within this many days.
const OWN_ACCOUNT_WINDOW_DAYS: i64 = 2;

/// What one [`run_transfer_pairing`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PairingSummary {
    /// Rows paired this run (counted per pair).
    pub paired: usize,
    /// Reviews left open afterwards.
    pub reviews: usize,
}

/// One transaction eligible to be paired: unpaired, not excluded, with a
/// parseable posted date. `description_lower` is `description_raw`
/// lowercased once at load, so matching never re-lowercases per pair (3.73).
struct Candidate {
    id: i64,
    account_id: i64,
    kind: String,
    posted: NaiveDate,
    amount_cents: i64,
    description_lower: String,
    category_id: Option<i64>,
    category_locked: bool,
}

/// Load the transactions that can still pair, with their account kind:
/// unpaired, not excluded, and either an unlocked chequing/savings row
/// (card-payment debits, own-account transfers) or a credit-card row
/// already categorized `Card Payment` (the credit side, whose category
/// condition replaces the lock check). Every other row can match nothing,
/// so it never loads (3.73: at 100 000 rows loading — and comparing —
/// everything dominated every import, unpair and review).
fn load_candidates(conn: &Connection, card_payment: i64) -> Result<Vec<Candidate>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.account_id, a.kind, t.posted_date, t.amount_cents,
                t.description_raw, t.category_id, t.category_locked
           FROM transactions t
           JOIN accounts a ON a.id = t.account_id
          WHERE t.transfer_pair_id IS NULL AND t.transfer_excluded = 0
            AND ((a.kind IN ('chequing', 'savings') AND t.category_locked = 0)
                 OR (a.kind = 'credit_card' AND t.category_id = ?1))",
    )?;
    let rows = stmt.query_map(params![card_payment], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, bool>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, account_id, kind, posted, amount_cents, description, category_id, category_locked) =
            row?;
        // A row whose date cannot be read is left out of matching rather
        // than failing the whole import.
        let Ok(posted) = NaiveDate::parse_from_str(&posted, "%Y-%m-%d") else {
            continue;
        };
        out.push(Candidate {
            id,
            account_id,
            kind,
            posted,
            amount_cents,
            description_lower: description.to_lowercase(),
            category_id,
            category_locked,
        });
    }
    Ok(out)
}

/// `|a − b| <= window` in days.
fn within(a: NaiveDate, b: NaiveDate, window: i64) -> bool {
    (a - b).num_days().abs() <= window
}

/// The symmetric "could pair with" relation for one pairing kind, as an
/// adjacency list. Only rows `keep` accepts can be endpoints; each pair is
/// checked once and inserted in both directions, ids ascending.
///
/// Cheap on large tables (3.73): a pair needs equal absolute amounts, so
/// rows are bucketed by `amount_cents.abs()` and compared only within their
/// bucket; each bucket is then walked in (posted date, id) order and the
/// inner loop stops at the first row further than `window_days` ahead.
/// `matches` still decides every pair it is reached for — the bucketing and
/// the window only skip pairs it would reject anyway — so the relation is
/// the same one the naive all-pairs scan produced.
fn adjacency(
    rows: &[Candidate],
    window_days: i64,
    keep: &dyn Fn(&Candidate) -> bool,
    matches: &dyn Fn(&Candidate, &Candidate) -> bool,
) -> HashMap<i64, Vec<i64>> {
    let mut adj: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut buckets: HashMap<i64, Vec<&Candidate>> = HashMap::new();
    for row in rows.iter().filter(|c| keep(c)) {
        buckets.entry(row.amount_cents.abs()).or_default().push(row);
    }
    for bucket in buckets.values_mut() {
        bucket.sort_unstable_by_key(|c| (c.posted, c.id));
        for (i, a) in bucket.iter().enumerate() {
            for b in bucket[i + 1..].iter() {
                if (b.posted - a.posted).num_days() > window_days {
                    break;
                }
                let (lo, hi) = if a.id < b.id { (a, b) } else { (b, a) };
                if matches(lo, hi) {
                    adj.entry(lo.id).or_default().push(hi.id);
                    adj.entry(hi.id).or_default().push(lo.id);
                }
            }
        }
    }
    for list in adj.values_mut() {
        list.sort_unstable();
    }
    adj
}

/// Write one pair: the ids point at each other, both rows take the
/// transfer category and are locked against rule re-runs.
fn write_pair(conn: &Connection, a: i64, b: i64, category_id: i64) -> Result<()> {
    for (row, partner) in [(a, b), (b, a)] {
        conn.execute(
            "UPDATE transactions
                SET transfer_pair_id = ?1, category_id = ?2, category_locked = 1
              WHERE id = ?3",
            params![partner, category_id, row],
        )?;
    }
    Ok(())
}

/// Pair every mutual singleton in `adj` (rows in id order), skipping rows
/// `paired` already holds, and return the pairs written.
fn pair_mutuals(
    conn: &Connection,
    adj: &HashMap<i64, Vec<i64>>,
    category_id: i64,
    paired: &mut std::collections::BTreeSet<i64>,
) -> Result<usize> {
    let mut ids: Vec<i64> = adj.keys().copied().collect();
    ids.sort_unstable();
    let mut count = 0;
    for id in ids {
        if paired.contains(&id) {
            continue;
        }
        // Mutual singleton: my only candidate, and I am theirs.
        let Some([other]) = adj.get(&id).map(Vec::as_slice) else {
            continue;
        };
        let other = *other;
        if paired.contains(&other) || adj.get(&other).map(Vec::as_slice) != Some(&[id][..]) {
            continue;
        }
        write_pair(conn, id, other, category_id)?;
        paired.insert(id);
        paired.insert(other);
        count += 1;
    }
    Ok(count)
}

/// Record a review for every row with more than one still-unpaired
/// candidate, across *all* adjacency maps at once: a row ambiguous under
/// both rules simultaneously gets one review holding the union of its
/// candidates, never two rows colliding on `transaction_id` (rows ordered
/// by id). Returns the reviews written; candidates are stored by posted
/// date then id.
fn record_reviews(
    conn: &Connection,
    rows: &HashMap<i64, &Candidate>,
    adjacencies: &[&HashMap<i64, Vec<i64>>],
    paired: &std::collections::BTreeSet<i64>,
) -> Result<usize> {
    let mut ids: Vec<i64> = adjacencies
        .iter()
        .flat_map(|adj| adj.keys().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut count = 0;
    for id in ids {
        if paired.contains(&id) {
            continue;
        }
        // Union of the kinds' candidates, still-unpaired only.
        let mut cands: Vec<i64> = Vec::new();
        for adj in adjacencies {
            if let Some(list) = adj.get(&id) {
                for c in list {
                    if !paired.contains(c) && !cands.contains(c) {
                        cands.push(*c);
                    }
                }
            }
        }
        if cands.len() < 2 {
            continue;
        }
        let mut cands: Vec<(NaiveDate, i64)> =
            cands.into_iter().map(|c| (rows[&c].posted, c)).collect();
        cands.sort_unstable();
        let list = cands
            .into_iter()
            .map(|(_, c)| c.to_string())
            .collect::<Vec<_>>()
            .join(",");
        conn.execute(
            "INSERT INTO transfer_reviews (transaction_id, candidate_ids) VALUES (?1, ?2)",
            params![id, list],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Run the pairing engine. Called from [`crate::import::commit_import`]
/// after the new rows land, inside the same transaction; safe to call any
/// time (idempotent). Reviews are recomputed from scratch: resolving or
/// working around one changes the candidates other rows see.
pub fn run_transfer_pairing(conn: &Connection) -> Result<PairingSummary> {
    conn.execute("DELETE FROM transfer_reviews", [])?;
    let card_payment = card_payment_category(conn)?;
    let between_accounts = between_accounts_category(conn)?;
    let candidates = load_candidates(conn, card_payment)?;
    let rows: HashMap<i64, &Candidate> = candidates.iter().map(|c| (c.id, c)).collect();
    let mut paired = std::collections::BTreeSet::new();
    let mut summary = PairingSummary::default();

    // Card payments: chequing −X against a credit-card +X already
    // categorized Card Payment. The credit side's category condition
    // replaces the lock check (import locks it); the debit side must be
    // unlocked for pairing to rewrite its category.
    let card_adj = adjacency(&candidates, CARD_PAYMENT_WINDOW_DAYS, &|_| true, &|a, b| {
        let (debit, credit) = if a.amount_cents < b.amount_cents {
            (a, b)
        } else {
            (b, a)
        };
        debit.amount_cents < 0
            && credit.amount_cents > 0
            && debit.amount_cents == -credit.amount_cents
            && debit.kind == "chequing"
            && !debit.category_locked
            && credit.kind == "credit_card"
            && credit.category_id == Some(card_payment)
            && within(debit.posted, credit.posted, CARD_PAYMENT_WINDOW_DAYS)
    });
    summary.paired += pair_mutuals(conn, &card_adj, card_payment, &mut paired)?;

    // Own-account transfers: two different chequing/savings accounts,
    // ∓X, at least one description containing TRANSFER (case-insensitive).
    let own_adj = adjacency(
        &candidates,
        OWN_ACCOUNT_WINDOW_DAYS,
        &|c| matches!(c.kind.as_str(), "chequing" | "savings") && !c.category_locked,
        &|a, b| {
            let (debit, credit) = if a.amount_cents < b.amount_cents {
                (a, b)
            } else {
                (b, a)
            };
            debit.account_id != credit.account_id
                && debit.amount_cents < 0
                && credit.amount_cents > 0
                && debit.amount_cents == -credit.amount_cents
                && (debit.description_lower.contains("transfer")
                    || credit.description_lower.contains("transfer"))
                && within(debit.posted, credit.posted, OWN_ACCOUNT_WINDOW_DAYS)
        },
    );
    summary.paired += pair_mutuals(conn, &own_adj, between_accounts, &mut paired)?;

    // Ambiguity: anything still unpaired with several candidates asks the
    // user — one review per row across both kinds (card first, then
    // own-account), carrying the union of its candidates.
    summary.reviews += record_reviews(conn, &rows, &[&card_adj, &own_adj], &paired)?;
    Ok(summary)
}

/// One candidate row of a [`TransferReview`], as the Transfers page shows
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferCandidate {
    pub id: i64,
    pub account: String,
    pub posted_date: String,
    pub description: String,
    pub amount_cents: i64,
}

/// An open transfer review: the transaction under review and every row it
/// could pair with, in the order stored (posted date, then id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferReview {
    pub transaction_id: i64,
    pub account: String,
    pub posted_date: String,
    pub description: String,
    pub amount_cents: i64,
    pub candidates: Vec<TransferCandidate>,
}

/// Open transfer reviews, oldest transaction first. Reviews whose row was
/// paired or excluded by another path are dropped (and disappear from the
/// table) rather than shown.
pub fn list_transfer_reviews(conn: &Connection) -> Result<Vec<TransferReview>> {
    let mut stmt = conn.prepare(
        "SELECT r.transaction_id, t.account_id, t.posted_date, t.description_raw,
                t.amount_cents, r.candidate_ids
           FROM transfer_reviews r
           JOIN transactions t ON t.id = r.transaction_id
          WHERE t.transfer_pair_id IS NULL AND t.transfer_excluded = 0
          ORDER BY t.posted_date, t.id",
    )?;
    let raw = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let label = |conn: &Connection, account_id: i64| -> Result<String> {
        Ok(conn.query_row(
            "SELECT i.name || ' · ' || a.name FROM accounts a
               JOIN institutions i ON i.id = a.institution_id WHERE a.id = ?1",
            [account_id],
            |r| r.get(0),
        )?)
    };
    let mut out = Vec::with_capacity(raw.len());
    for (transaction_id, account_id, posted_date, description, amount_cents, list) in raw {
        let mut candidates = Vec::new();
        for part in list.split(',').filter_map(|p| p.trim().parse::<i64>().ok()) {
            // A candidate deleted or paired since the review was written
            // simply stops being offered.
            if let Ok(c) = conn.query_row(
                "SELECT i.name || ' · ' || a.name, t.posted_date, t.description_raw,
                        t.amount_cents
                   FROM transactions t
                   JOIN accounts a ON a.id = t.account_id
                   JOIN institutions i ON i.id = a.institution_id
                  WHERE t.id = ?1 AND t.transfer_pair_id IS NULL
                    AND t.transfer_excluded = 0",
                [part],
                |r| {
                    Ok(TransferCandidate {
                        id: part,
                        account: r.get(0)?,
                        posted_date: r.get(1)?,
                        description: r.get(2)?,
                        amount_cents: r.get(3)?,
                    })
                },
            ) {
                candidates.push(c);
            }
        }
        out.push(TransferReview {
            transaction_id,
            account: label(conn, account_id)?,
            posted_date,
            description,
            amount_cents,
            candidates,
        });
    }
    Ok(out)
}

/// Resolve one open review (Transfers page, 4.2): pair the row with
/// `candidate`, or mark it *not a transfer* when `candidate` is `None`.
///
/// Pairing writes what [`run_transfer_pairing`] would have written (mutual
/// `transfer_pair_id`, the transfer category, a lock). "Not a transfer"
/// sets `transfer_excluded = 1` so no later run offers the row again. The
/// review is deleted either way and the engine re-runs: the decision
/// changes what the *other* rows see, so an ambiguity the user just
/// resolved can free a mutual singleton elsewhere, and reviews left stale
/// by it are recomputed away.
pub fn resolve_transfer_review(
    conn: &mut Connection,
    transaction_id: i64,
    candidate: Option<i64>,
) -> Result<PairingSummary> {
    let tx = conn.transaction()?;
    let stored: String = tx
        .query_row(
            "SELECT candidate_ids FROM transfer_reviews WHERE transaction_id = ?1",
            [transaction_id],
            |r| r.get(0),
        )
        .map_err(|_| DbError::NoTransferReview(transaction_id))?;
    match candidate {
        Some(candidate) => {
            let offered = candidate != transaction_id
                && stored
                    .split(',')
                    .filter_map(|p| p.trim().parse::<i64>().ok())
                    .any(|c| c == candidate);
            let category = match offered {
                true => pair_categories_for(&tx, transaction_id, candidate),
                false => None,
            };
            let Some(category) = category else {
                return Err(DbError::NotTransferCandidate {
                    transaction: transaction_id,
                    candidate,
                });
            };
            write_pair(&tx, transaction_id, candidate, category)?;
        }
        None => {
            let updated = tx.execute(
                "UPDATE transactions SET transfer_excluded = 1
                  WHERE id = ?1 AND transfer_pair_id IS NULL",
                [transaction_id],
            )?;
            if updated == 0 {
                return Err(DbError::NoTransferReview(transaction_id));
            }
        }
    }
    tx.execute(
        "DELETE FROM transfer_reviews WHERE transaction_id = ?1",
        [transaction_id],
    )?;
    let summary = run_transfer_pairing(&tx)?;
    tx.commit()?;
    Ok(summary)
}

/// The category the pair `a`/`b` gets, or `None` when they cannot pair:
/// both rows must exist, still be unpaired and not excluded, and be either
/// a credit-card/chequing pair (Card Payment) or two different
/// chequing/savings accounts (Between Accounts).
fn pair_categories_for(conn: &Connection, a: i64, b: i64) -> Option<i64> {
    let kinds = |id: i64| -> Option<(String, i64, Option<i64>, bool)> {
        conn.query_row(
            "SELECT k.kind, t.account_id, t.transfer_pair_id, t.transfer_excluded
               FROM transactions t JOIN accounts k ON k.id = t.account_id
              WHERE t.id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, bool>(3)?)),
        )
        .ok()
    };
    let (a_kind, a_account, a_pair, a_excluded) = kinds(a)?;
    let (b_kind, b_account, b_pair, b_excluded) = kinds(b)?;
    if a_pair.is_some() || b_pair.is_some() || a_excluded || b_excluded {
        return None;
    }
    let is_card = a_kind == "credit_card" || b_kind == "credit_card";
    if is_card {
        Some(card_payment_category(conn).ok()?)
    } else {
        let own = |k: &str| matches!(k, "chequing" | "savings");
        if a_account != b_account && own(&a_kind) && own(&b_kind) {
            Some(between_accounts_category(conn).ok()?)
        } else {
            None
        }
    }
}

/// *Unpair* (row context menu, 2.9): clear `transfer_pair_id` on both
/// sides, re-apply the rules to each row (the [`clear_override`] path from
/// 2.6: a credit-card row goes back to its locked `Card Payment`, every
/// other row takes its winning rule or becomes Uncategorized again), and
/// exclude both sides so the next import never re-pairs them.
pub fn unpair_transaction(conn: &mut Connection, transaction_id: i64) -> Result<()> {
    let tx = conn.transaction()?;
    let partner: Option<i64> = tx
        .query_row(
            "SELECT transfer_pair_id FROM transactions WHERE id = ?1",
            [transaction_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let Some(partner) = partner else {
        return Err(DbError::NotPaired(transaction_id));
    };
    for row in [transaction_id, partner] {
        tx.execute(
            "UPDATE transactions SET transfer_pair_id = NULL, transfer_excluded = 1
              WHERE id = ?1",
            [row],
        )?;
        clear_override(&tx, row)?;
    }
    run_transfer_pairing(&tx)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categories::card_payment_category;
    use crate::{create_account, open_in_memory};
    use financi_core::AccountKind;

    fn chequing(conn: &Connection, name: &str) -> i64 {
        create_account(conn, "amex", name, AccountKind::Chequing).unwrap()
    }

    fn savings(conn: &Connection, name: &str) -> i64 {
        create_account(conn, "amex", name, AccountKind::Savings).unwrap()
    }

    fn credit(conn: &Connection, name: &str) -> i64 {
        create_account(conn, "amex", name, AccountKind::CreditCard).unwrap()
    }

    /// Store a transaction on its own import (parser `amex`) with explicit
    /// date/amount/description and an optional category.
    fn row(
        conn: &Connection,
        account_id: i64,
        posted: &str,
        amount_cents: i64,
        description: &str,
        category_id: Option<i64>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'activity.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [account_id],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw, category_id)
             VALUES (?1, ?2, 0, X'00', ?3, ?4, ?5, ?6, ?7)",
            (
                account_id,
                import,
                uuid::Uuid::new_v4().to_string(),
                posted,
                amount_cents,
                description,
                category_id,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// A credit-card card-payment row: +X, already `Card Payment`, locked
    /// exactly as import leaves it (2.5).
    fn card_payment_row(conn: &Connection, account_id: i64, posted: &str, amount: i64) -> i64 {
        let card = card_payment_category(conn).unwrap();
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'activity.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [account_id],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw,
                                       category_id, category_locked)
             VALUES (?1, ?2, 0, X'00', ?3, ?4, ?5, 'PAYMENT RECEIVED - THANK YOU', ?6, 1)",
            (
                account_id,
                import,
                uuid::Uuid::new_v4().to_string(),
                posted,
                amount,
                card,
            ),
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn state(conn: &Connection, id: i64) -> (Option<i64>, Option<i64>, bool, bool) {
        conn.query_row(
            "SELECT transfer_pair_id, category_id, category_locked, transfer_excluded
               FROM transactions WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0).unwrap(),
                    r.get(1).unwrap(),
                    r.get(2).unwrap(),
                    r.get(3).unwrap(),
                ))
            },
        )
        .unwrap()
    }

    fn reviews(conn: &Connection) -> Vec<(i64, String)> {
        conn.prepare(
            "SELECT transaction_id, candidate_ids FROM transfer_reviews ORDER BY transaction_id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    #[test]
    fn migration_adds_reviews_table_and_exclusion_column() {
        let conn = open_in_memory().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM transfer_reviews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('transactions') WHERE name = 'transfer_excluded'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn single_card_payment_match_pairs_both() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let credit_row = card_payment_row(&conn, card, "2026-10-25", 150_000);

        let out = run_transfer_pairing(&conn).unwrap();
        assert_eq!(out.paired, 1);
        assert_eq!(out.reviews, 0);
        let card_payment = card_payment_category(&conn).unwrap();
        assert_eq!(
            state(&conn, debit),
            (Some(credit_row), Some(card_payment), true, false)
        );
        assert_eq!(
            state(&conn, credit_row),
            (Some(debit), Some(card_payment), true, false)
        );
    }

    #[test]
    fn single_own_account_match_pairs_both() {
        let conn = open_in_memory().unwrap();
        let (cheq, sav) = (chequing(&conn, "Chequing"), savings(&conn, "Savings"));
        let debit = row(
            &conn,
            cheq,
            "2026-11-02",
            -200_000,
            "TRANSFER TO SAVINGS",
            None,
        );
        let credit_row = row(
            &conn,
            sav,
            "2026-11-02",
            200_000,
            "TRANSFER FROM CHEQUING",
            None,
        );

        run_transfer_pairing(&conn).unwrap();
        let between = crate::categories::between_accounts_category(&conn).unwrap();
        assert_eq!(
            state(&conn, debit),
            (Some(credit_row), Some(between), true, false)
        );
        assert_eq!(
            state(&conn, credit_row),
            (Some(debit), Some(between), true, false)
        );
    }

    #[test]
    fn own_account_needs_transfer_in_a_description() {
        let conn = open_in_memory().unwrap();
        let (cheq, sav) = (chequing(&conn, "Chequing"), savings(&conn, "Savings"));
        row(&conn, cheq, "2026-11-02", -200_000, "SOMETHING ELSE", None);
        row(&conn, sav, "2026-11-02", 200_000, "ANOTHER THING", None);

        run_transfer_pairing(&conn).unwrap();
        // Unrelated equal amounts across accounts are not paired.
        assert!(reviews(&conn).is_empty());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE transfer_pair_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "an unrelated equal amount is not paired");
    }

    #[test]
    fn unrelated_equal_amounts_are_not_paired() {
        // A credit-card refund (+X) looks exactly like a card payment in
        // amount and date, but it is not categorized `Card Payment`, so the
        // matching chequing debit must not pair with it.
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        row(
            &conn,
            cheq,
            "2026-11-02",
            -100_000,
            "REFUND SEND BACK",
            None,
        );
        row(&conn, card, "2026-11-02", 100_000, "MERCHANT REFUND", None);

        run_transfer_pairing(&conn).unwrap();
        assert!(
            reviews(&conn).is_empty(),
            "a refund is not an ambiguous payment"
        );
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE transfer_pair_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "an unrelated equal amount is not paired");
    }

    #[test]
    fn pairing_is_fast_among_tens_of_thousands_of_rows() {
        // 3.73: the engine used to load nearly every unpaired row and test
        // every pair — ~200 million predicate checks at 20 000 rows, on
        // every import, unpair and review. It now loads only rows that can
        // pair, buckets them by absolute amount and compares within the
        // date window, so the pair below is found almost instantly. The
        // bound is deliberately generous (debug CI builds vary; the naive
        // all-pairs scan over this data took minutes): it fails on the old
        // behaviour while leaving the real runtime — milliseconds — room.
        let mut conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'noise.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 21000)",
            [cheq],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        {
            let tx = conn.transaction().unwrap();
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                                   row_hash, posted_date, amount_cents, description_raw)
                         VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, 'GROCERY STORE')",
                    )
                    .unwrap();
                for i in 0..20_000i64 {
                    // Distinct amounts: each row sits alone in its bucket.
                    let date = start + chrono::Duration::days(i % 365);
                    stmt.execute(params![
                        cheq,
                        import,
                        i,
                        uuid::Uuid::new_v4().to_string(),
                        date.format("%Y-%m-%d").to_string(),
                        -(1_000_000 + i),
                    ])
                    .unwrap();
                }
                for i in 0..1_000i64 {
                    // One shared amount: a single big bucket spread over the
                    // year, so the windowed scan compares few pairs inside it.
                    let date = start + chrono::Duration::days(i % 365);
                    stmt.execute(params![
                        cheq,
                        import,
                        20_000 + i,
                        uuid::Uuid::new_v4().to_string(),
                        date.format("%Y-%m-%d").to_string(),
                        -250_000i64,
                    ])
                    .unwrap();
                }
            }
            tx.commit().unwrap();
        }
        // The one real pair: chequing −X and a card +X already `Card Payment`.
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -50_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let credit_row = card_payment_row(&conn, card, "2026-10-25", 50_000);

        let began = std::time::Instant::now();
        let out = run_transfer_pairing(&conn).unwrap();
        let elapsed = began.elapsed();
        assert_eq!(out.paired, 1, "the one real pair is still found");
        assert_eq!(out.reviews, 0, "the noise rows ask for nothing");
        assert_eq!(state(&conn, debit).0, Some(credit_row));
        assert_eq!(state(&conn, credit_row).0, Some(debit));
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "pairing 21 000 rows took {elapsed:?} — well over even the \
             generous bound, the quadratic scan is back",
        );
    }

    #[test]
    fn transfer_match_is_case_insensitive() {
        let conn = open_in_memory().unwrap();
        let (cheq, sav) = (chequing(&conn, "Chequing"), savings(&conn, "Savings"));
        row(
            &conn,
            cheq,
            "2026-11-02",
            -50_000,
            "E-transfer to savings",
            None,
        );
        row(
            &conn,
            sav,
            "2026-11-02",
            50_000,
            "INTERAC E-TRANSFER RECEIVE",
            None,
        );

        run_transfer_pairing(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE transfer_pair_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn more_than_one_candidate_goes_to_review_not_pairing() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let c1 = card_payment_row(&conn, card, "2026-10-25", 150_000);
        let c2 = card_payment_row(&conn, card, "2026-10-28", 150_000);

        let out = run_transfer_pairing(&conn).unwrap();
        assert_eq!(out.paired, 0, "a tie is never guessed");
        assert_eq!(reviews(&conn), [(debit, format!("{c1},{c2}"))]);
        assert_eq!(state(&conn, debit).0, None);
        assert_eq!(state(&conn, c1).0, None);
    }

    #[test]
    fn two_rows_claiming_one_candidate_review_the_shared_side() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let d1 = row(
            &conn,
            cheq,
            "2026-10-24",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let d2 = row(
            &conn,
            cheq,
            "2026-10-27",
            -150_000,
            "PAYMENT FOR CARD",
            None,
        );
        let c = card_payment_row(&conn, card, "2026-10-25", 150_000);

        run_transfer_pairing(&conn).unwrap();
        // The shared candidate carries the review, ordered by date then id.
        assert_eq!(reviews(&conn), [(c, format!("{d1},{d2}"))]);
        assert_eq!(state(&conn, c).0, None);
    }

    #[test]
    fn one_row_ambiguous_under_both_rules_gets_one_unioned_review() {
        let conn = open_in_memory().unwrap();
        let (cheq, card, sav) = (
            chequing(&conn, "Chequing"),
            credit(&conn, "Cobalt"),
            savings(&conn, "Savings"),
        );
        // One chequing debit: two card payments within 5 days AND two
        // savings transfer credits within 2 days — ambiguous under both
        // rules at once. It must get a single review, never two rows
        // colliding on transaction_id.
        let debit = row(&conn, cheq, "2026-10-26", -150_000, "MONEY OUT", None);
        let c1 = card_payment_row(&conn, card, "2026-10-25", 150_000);
        let own1 = row(&conn, sav, "2026-10-26", 150_000, "TRANSFER IN", None);
        let own2 = row(&conn, sav, "2026-10-27", 150_000, "TRANSFER IN", None);
        let c2 = card_payment_row(&conn, card, "2026-10-28", 150_000);

        let out = run_transfer_pairing(&conn).unwrap();
        assert_eq!(out.paired, 0, "a tie is never guessed");
        assert_eq!(out.reviews, 1, "one review per row, not per rule");
        // The union of both kinds' candidates, ordered by date then id.
        assert_eq!(
            reviews(&conn),
            [(debit, format!("{c1},{own1},{own2},{c2}"))]
        );
    }

    #[test]
    fn resolving_a_review_pairs_the_chosen_candidate() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        card_payment_row(&conn, card, "2026-10-25", 150_000);
        let chosen = card_payment_row(&conn, card, "2026-10-28", 150_000);
        run_transfer_pairing(&conn).unwrap();

        let mut conn = conn;
        resolve_transfer_review(&mut conn, debit, Some(chosen)).unwrap();
        let card_payment = card_payment_category(&conn).unwrap();
        assert_eq!(state(&conn, debit).0, Some(chosen));
        assert_eq!(state(&conn, chosen).0, Some(debit));
        assert_eq!(state(&conn, debit).1, Some(card_payment));
        assert!(reviews(&conn).is_empty(), "the review closes");
    }

    #[test]
    fn not_a_transfer_excludes_the_row_forever() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let c1 = card_payment_row(&conn, card, "2026-10-25", 150_000);
        card_payment_row(&conn, card, "2026-10-28", 150_000);
        run_transfer_pairing(&conn).unwrap();

        let mut conn = conn;
        resolve_transfer_review(&mut conn, debit, None).unwrap();
        assert!(reviews(&conn).is_empty());
        assert!(state(&conn, debit).3, "the row is excluded");
        // Later runs never offer or pair it again; the two card rows, now
        // without their ambiguous partner, go quiet too.
        run_transfer_pairing(&conn).unwrap();
        assert!(reviews(&conn).is_empty());
        assert_eq!(state(&conn, debit).0, None);
        assert_eq!(state(&conn, c1).0, None);
    }

    #[test]
    fn resolve_refuses_unknown_reviews_and_bad_candidates() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let c1 = card_payment_row(&conn, card, "2026-10-25", 150_000);
        card_payment_row(&conn, card, "2026-10-28", 150_000);
        run_transfer_pairing(&conn).unwrap();
        let mut conn = conn;
        assert!(matches!(
            resolve_transfer_review(&mut conn, 999, None).unwrap_err(),
            DbError::NoTransferReview(999)
        ));
        assert!(matches!(
            resolve_transfer_review(&mut conn, debit, Some(999)).unwrap_err(),
            DbError::NotTransferCandidate { transaction, candidate } if transaction == debit && candidate == 999
        ));
        // c2 (not c1) is offered, but a row the review does not list fails:
        let outside = row(&conn, cheq, "2026-10-26", -100, "COFFEE", None);
        assert!(matches!(
            resolve_transfer_review(&mut conn, debit, Some(outside)).unwrap_err(),
            DbError::NotTransferCandidate { .. }
        ));
        // c1 IS offered but the review stays open until it resolves.
        let open = reviews(&conn);
        assert_eq!(open.len(), 1);
        assert!(open[0].1.contains(&c1.to_string()));
    }

    #[test]
    fn unpair_clears_both_sides_and_reapplies_rules() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let credit_row = card_payment_row(&conn, card, "2026-10-25", 150_000);
        run_transfer_pairing(&conn).unwrap();
        assert_eq!(state(&conn, debit).0, Some(credit_row));

        let mut conn = conn;
        unpair_transaction(&mut conn, debit).unwrap();
        assert_eq!(state(&conn, debit).0, None, "pair cleared");
        assert_eq!(state(&conn, credit_row).0, None);
        // The debit row is back under the rules: unlocked, Uncategorized.
        assert_eq!(state(&conn, debit), (None, None, false, true));
        // The credit-card row keeps its built-in Card Payment (locked), as
        // clear_override restores it — an unpaired card payment is still a
        // transfer.
        let card_payment = card_payment_category(&conn).unwrap();
        assert_eq!(state(&conn, credit_row).1, Some(card_payment));
        assert!(state(&conn, credit_row).2);
        // And never re-paired by a later run.
        run_transfer_pairing(&conn).unwrap();
        assert_eq!(state(&conn, debit).0, None);
    }

    #[test]
    fn unpair_refuses_unpaired_rows() {
        let conn = open_in_memory().unwrap();
        let cheq = chequing(&conn, "Chequing");
        let id = row(&conn, cheq, "2026-10-26", -150_000, "COFFEE", None);
        let mut conn = conn;
        assert!(matches!(
            unpair_transaction(&mut conn, id).unwrap_err(),
            DbError::NotPaired(x) if x == id
        ));
    }

    #[test]
    fn dates_outside_the_window_do_not_pair() {
        let conn = open_in_memory().unwrap();
        let (cheq, card, sav) = (
            chequing(&conn, "Chequing"),
            credit(&conn, "Cobalt"),
            savings(&conn, "Savings"),
        );
        // 6 days apart: outside the card payment window.
        row(
            &conn,
            cheq,
            "2026-10-31",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        card_payment_row(&conn, card, "2026-10-25", 150_000);
        // 3 days apart: outside the own-account window.
        row(&conn, cheq, "2026-11-05", -200_000, "TRANSFER OUT", None);
        row(&conn, sav, "2026-11-02", 200_000, "TRANSFER IN", None);

        run_transfer_pairing(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE transfer_pair_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
        assert!(reviews(&conn).is_empty());
    }

    #[test]
    fn window_edges_are_inclusive() {
        let conn = open_in_memory().unwrap();
        let (cheq, card, sav) = (
            chequing(&conn, "Chequing"),
            credit(&conn, "Cobalt"),
            savings(&conn, "Savings"),
        );
        // Exactly 5 days apart.
        row(
            &conn,
            cheq,
            "2026-10-30",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        card_payment_row(&conn, card, "2026-10-25", 150_000);
        // Exactly 2 days apart.
        row(&conn, cheq, "2026-11-04", -200_000, "TRANSFER OUT", None);
        row(&conn, sav, "2026-11-02", 200_000, "TRANSFER IN", None);

        run_transfer_pairing(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE transfer_pair_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 4);
    }

    #[test]
    fn locked_rows_are_never_pairing_fodder() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let manual = {
            use crate::categories::{create_child, create_parent};
            let parent =
                create_parent(&conn, "Bills", financi_core::CategoryKind::Expense).unwrap();
            create_child(&conn, parent, "Card Bill").unwrap()
        };
        // A manually categorized (locked) chequing row is not a candidate.
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'activity.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [cheq],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw,
                                       category_id, category_locked)
             VALUES (?1, ?2, 0, X'00', 'lock-me', '2026-10-26', -150000, 'CREDIT CARD PAYMENT', ?3, 1)",
            (cheq, import, manual),
        )
        .unwrap();
        card_payment_row(&conn, card, "2026-10-25", 150_000);

        run_transfer_pairing(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE transfer_pair_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "a manual categorization beats pairing");
    }

    #[test]
    fn runs_are_idempotent() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let c1 = card_payment_row(&conn, card, "2026-10-25", 150_000);
        let c2 = card_payment_row(&conn, card, "2026-10-28", 150_000);

        run_transfer_pairing(&conn).unwrap();
        let first = reviews(&conn);
        run_transfer_pairing(&conn).unwrap();
        run_transfer_pairing(&conn).unwrap();
        assert_eq!(
            reviews(&conn),
            first,
            "reviews are recomputed, never stacked"
        );
        assert_eq!(state(&conn, debit).0, None);
        assert_eq!(state(&conn, c1).0, None);
        assert_eq!(state(&conn, c2).0, None);
    }

    #[test]
    fn resolving_frees_a_mutual_singleton_elsewhere() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        // D1 is ambiguous between C1 and C2; D2 pairs only with C3.
        let d1 = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        card_payment_row(&conn, card, "2026-10-25", 150_000);
        let _c2 = card_payment_row(&conn, card, "2026-10-28", 150_000);
        let d2 = row(
            &conn,
            cheq,
            "2026-11-10",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let c3 = card_payment_row(&conn, card, "2026-11-10", 150_000);
        run_transfer_pairing(&conn).unwrap();
        assert_eq!(state(&conn, d2).0, Some(c3), "the clean pair went through");
        assert_eq!(reviews(&conn).len(), 1);

        // "Not a transfer" on D1 also refreshes the review table; nothing
        // new pairs, and no review survives for D1.
        let mut conn = conn;
        resolve_transfer_review(&mut conn, d1, None).unwrap();
        assert!(reviews(&conn).is_empty());
    }

    #[test]
    fn list_reviews_shows_rows_and_candidates() {
        let conn = open_in_memory().unwrap();
        let (cheq, card) = (chequing(&conn, "Chequing"), credit(&conn, "Cobalt"));
        let debit = row(
            &conn,
            cheq,
            "2026-10-26",
            -150_000,
            "CREDIT CARD PAYMENT",
            None,
        );
        let c1 = card_payment_row(&conn, card, "2026-10-25", 150_000);
        let c2 = card_payment_row(&conn, card, "2026-10-28", 150_000);
        run_transfer_pairing(&conn).unwrap();

        let list = list_transfer_reviews(&conn).unwrap();
        assert_eq!(list.len(), 1);
        let r = &list[0];
        assert_eq!(r.transaction_id, debit);
        assert_eq!(r.amount_cents, -150_000);
        assert_eq!(r.account, "American Express · Chequing");
        let ids: Vec<i64> = r.candidates.iter().map(|c| c.id).collect();
        assert_eq!(ids, [c1, c2]);
        assert_eq!(r.candidates[0].posted_date, "2026-10-25");
        assert_eq!(r.candidates[0].amount_cents, 150_000);
    }
}
