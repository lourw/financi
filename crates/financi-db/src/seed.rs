//! Synthetic development data (feature `dev-seed`). Contains no real
//! financial data: merchants, amounts and dates are generated
//! deterministically from a fixed seed.
//!
//! Rows are shaped like the Amex export (`Date,Date Processed,Description,Amount`,
//! charges positive) and stored with real raw bytes and row hashes, so later
//! milestones can exercise dedup and export against them.

use chrono::{Duration, NaiveDate};
use financi_core::{AccountKey, CategoryKind, row_hash};
use rusqlite::{Connection, params};

use crate::Result;

/// Parent category -> (kind, children).
const CATEGORIES: &[(&str, CategoryKind, &[&str])] = &[
    (
        "Food",
        CategoryKind::Expense,
        &["Groceries", "Restaurants", "Coffee"],
    ),
    (
        "Transportation",
        CategoryKind::Expense,
        &["Transit", "Rideshare", "Fuel"],
    ),
    (
        "Home",
        CategoryKind::Expense,
        &["Utilities", "Internet & Phone"],
    ),
    (
        "Shopping",
        CategoryKind::Expense,
        &["General", "Electronics"],
    ),
    (
        "Entertainment",
        CategoryKind::Expense,
        &["Streaming", "Events"],
    ),
    ("Travel", CategoryKind::Expense, &["Flights", "Hotels"]),
    ("Income", CategoryKind::Income, &["Refunds & Credits"]),
    // `Transfers` / `Card Payment` are system categories inserted by
    // migration 0002; the rules and rows below reference them by name.
];

/// (merchant, city, child category, typical charge in cents, charges per month)
const MERCHANTS: &[(&str, &str, &str, i64, u32)] = &[
    ("FRESHMART #112", "VANCOUVER", "Groceries", 8_500, 6),
    ("GREENLEAF GROCERS", "VANCOUVER", "Groceries", 4_200, 3),
    ("BLUE DOOR BISTRO", "VANCOUVER", "Restaurants", 6_400, 2),
    ("NOODLE HOUSE 88", "BURNABY", "Restaurants", 2_800, 2),
    ("BEANERY COFFEE", "VANCOUVER", "Coffee", 575, 10),
    ("TRANSLINK FARE", "VANCOUVER", "Transit", 1_000, 4),
    ("ZIPRIDE *TRIP", "VANCOUVER", "Rideshare", 1_850, 2),
    ("NORTHGAS STN 41", "RICHMOND", "Fuel", 6_000, 1),
    ("CITY HYDRO", "VANCOUVER", "Utilities", 7_200, 1),
    ("MAPLETEL MOBILE", "TORONTO", "Internet & Phone", 8_500, 1),
    ("EVERYTHING STORE", "SEATTLE", "General", 3_900, 2),
    ("CIRCUIT CENTRAL", "VANCOUVER", "Electronics", 21_000, 0),
    ("STREAMFLIX.COM", "LOS GATOS", "Streaming", 1_699, 1),
    ("TUNEBOX MUSIC", "STOCKHOLM", "Streaming", 1_199, 1),
    ("ARENA TICKETS", "VANCOUVER", "Events", 12_000, 0),
    ("SKYHOP AIR", "CALGARY", "Flights", 38_000, 0),
    ("HARBOUR INN", "VICTORIA", "Hotels", 24_000, 0),
];

/// Substring rules matching the merchants above.
const RULES: &[(&str, &str)] = &[
    ("FRESHMART", "Groceries"),
    ("GREENLEAF", "Groceries"),
    ("BISTRO", "Restaurants"),
    ("NOODLE", "Restaurants"),
    ("BEANERY", "Coffee"),
    ("TRANSLINK", "Transit"),
    ("ZIPRIDE", "Rideshare"),
    ("NORTHGAS", "Fuel"),
    ("HYDRO", "Utilities"),
    ("MAPLETEL", "Internet & Phone"),
    ("EVERYTHING STORE", "General"),
    ("CIRCUIT CENTRAL", "Electronics"),
    ("STREAMFLIX", "Streaming"),
    ("TUNEBOX", "Streaming"),
    ("ARENA", "Events"),
    ("SKYHOP", "Flights"),
    ("HARBOUR INN", "Hotels"),
    ("PAYMENT RECEIVED", "Card Payment"),
];

pub const SEED_HEADER: &str = "Date,Date Processed,Description,Amount";

/// Small deterministic PRNG (SplitMix64), so seeded data is identical on
/// every run without pulling in a rand dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedSummary {
    pub categories: usize,
    pub rules: usize,
    pub transactions: usize,
}

/// Fill an empty database with categories, rules, one synthetic Amex account
/// and `months` of transactions ending at `end`. Runs in one transaction.
pub fn seed(conn: &mut Connection, end: NaiveDate, months: u32) -> Result<SeedSummary> {
    let tx = conn.transaction()?;

    let mut categories = 0;
    for (parent, kind, children) in CATEGORIES {
        tx.execute(
            "INSERT INTO categories (name, parent_id, kind) VALUES (?1, NULL, ?2)",
            params![parent, kind.as_str()],
        )?;
        let parent_id = tx.last_insert_rowid();
        categories += 1;
        for child in *children {
            tx.execute(
                "INSERT INTO categories (name, parent_id, kind) VALUES (?1, ?2, ?3)",
                params![child, parent_id, kind.as_str()],
            )?;
            categories += 1;
        }
    }
    let child_id = |name: &str| -> rusqlite::Result<i64> {
        tx.query_row(
            "SELECT id FROM categories WHERE name = ?1 AND parent_id IS NOT NULL",
            [name],
            |r| r.get(0),
        )
    };

    for (priority, (pattern, child)) in RULES.iter().enumerate() {
        tx.execute(
            "INSERT INTO category_rules (pattern, category_id, priority) VALUES (?1, ?2, ?3)",
            params![pattern, child_id(child)?, priority as i64],
        )?;
    }

    let account_id = crate::create_account(
        &tx,
        "amex",
        "Dev Cobalt (synthetic)",
        financi_core::AccountKind::CreditCard,
    )?;
    let account_key = AccountKey(tx.query_row(
        "SELECT account_key FROM accounts WHERE id = ?1",
        [account_id],
        |r| r.get(0),
    )?);

    // Generate rows (charges positive, Amex style), newest first like the export.
    let start = end - Duration::days(i64::from(months) * 30);
    let mut rng = Rng(0x00F1_7A9C_1000_0001);
    let mut rows: Vec<(NaiveDate, NaiveDate, String, i64, &str)> = Vec::new();
    for (merchant, city, child, typical, per_month) in MERCHANTS {
        // Occasional merchants (per_month == 0) appear about every 4 months.
        let count = if *per_month == 0 {
            months / 4
        } else {
            per_month * months
        };
        for _ in 0..count {
            let day = start + Duration::days(rng.below((end - start).num_days() as u64 + 1) as i64);
            let processed = day + Duration::days(rng.below(4) as i64);
            let jitter = (*typical as f64 * (0.7 + rng.below(61) as f64 / 100.0)) as i64;
            let desc = format!("{merchant:<24}{city}");
            rows.push((day, processed.min(end), desc, jitter.max(100), child));
        }
    }
    // A few refunds and a monthly card payment.
    for m in 0..months {
        let day = start + Duration::days(i64::from(m) * 30 + 25);
        rows.push((
            day,
            day,
            "PAYMENT RECEIVED - THANK YOU".into(),
            -150_000,
            "Card Payment",
        ));
        if m % 5 == 2 {
            let desc = format!("{:<24}{}", "EVERYTHING STORE", "SEATTLE");
            rows.push((
                day - Duration::days(3),
                day - Duration::days(2),
                desc,
                -2_499,
                "General",
            ));
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));

    tx.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'dev-seed.csv', ?2, ?3, 'utf-8', 0, 'lf', 0, ?4)",
        params![
            account_id,
            end.to_string(),
            SEED_HEADER.as_bytes(),
            rows.len() as i64
        ],
    )?;
    let import_id = tx.last_insert_rowid();

    {
        let mut insert = tx.prepare(
            "INSERT INTO transactions
               (account_id, import_id, source_line, raw_bytes, row_hash, posted_date,
                processed_date, amount_cents, description_raw, merchant, category_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for (line, (date, processed, desc, charge_cents, child)) in rows.iter().enumerate() {
            let date_raw = date.format("%d %b %Y").to_string();
            let processed_raw = processed.format("%d %b %Y").to_string();
            let amount_raw = format!("{}.{:02}", charge_cents / 100, (charge_cents % 100).abs());
            let amount_raw = if *charge_cents < 0 && charge_cents / 100 == 0 {
                format!("-{amount_raw}")
            } else {
                amount_raw
            };
            let raw = format!("{date_raw},{processed_raw},{desc},{amount_raw}");
            let hash = row_hash(
                &account_key,
                &[date_raw.as_bytes(), desc.as_bytes(), amount_raw.as_bytes()],
            );
            let merchant = desc.split("  ").next().unwrap_or(desc).trim().to_string();
            insert.execute(params![
                account_id,
                import_id,
                line as i64,
                raw.as_bytes(),
                hash.0,
                date.to_string(),
                processed.to_string(),
                -charge_cents, // financi convention: money out is negative
                desc,
                merchant,
                child_id(child)?,
            ])?;
        }
    }

    let rules = RULES.len();
    let transactions = rows.len();
    tx.commit()?;
    Ok(SeedSummary {
        categories,
        rules,
        transactions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> (Connection, SeedSummary) {
        let mut conn = crate::open_in_memory().unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let summary = seed(&mut conn, end, 18).unwrap();
        (conn, summary)
    }

    #[test]
    fn seeds_two_level_categories_rules_and_transactions() {
        let (conn, s) = seeded();
        assert_eq!(s.categories, 22, "7 parents + 15 children");
        assert_eq!(s.rules, RULES.len());
        // The system Transfers/Card Payment from migration 0002 are reused,
        // not duplicated, and card payments point at the system row.
        let dupes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE name = 'Transfers'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dupes, 1);
        let on_system_category: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions t
                   JOIN categories c ON c.id = t.category_id
                  WHERE t.description_raw LIKE 'PAYMENT RECEIVED%'
                    AND c.name = 'Card Payment' AND c.is_system = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            on_system_category > 0,
            "card payments use the system category"
        );
        // ~38 a month over 18 months
        assert!(
            (600..800).contains(&s.transactions),
            "got {}",
            s.transactions
        );
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE category_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0);
    }

    #[test]
    fn seed_is_deterministic() {
        let (a, _) = seeded();
        let (b, _) = seeded();
        let hashes = |c: &Connection| -> Vec<(String, String)> {
            let mut s = c
                .prepare("SELECT row_hash, CAST(raw_bytes AS TEXT) FROM transactions ORDER BY id")
                .unwrap();
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        // Hashes differ (fresh account_key UUID each run); raw rows must not.
        let raw = |v: Vec<(String, String)>| v.into_iter().map(|(_, r)| r).collect::<Vec<_>>();
        assert_eq!(raw(hashes(&a)), raw(hashes(&b)));
    }

    #[test]
    fn raw_rows_look_like_amex_and_signs_are_flipped() {
        let (conn, _) = seeded();
        let (raw, cents): (String, i64) = conn
            .query_row(
                "SELECT CAST(raw_bytes AS TEXT), amount_cents FROM transactions
                  WHERE description_raw LIKE 'PAYMENT RECEIVED%' LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(raw.split(',').count(), 4);
        assert!(raw.ends_with(",-1500.00"), "{raw}");
        assert_eq!(
            cents, 150_000,
            "a card payment is money in on the card account"
        );
    }

    #[test]
    fn transfers_are_excluded_from_spend_totals() {
        let (conn, _) = seeded();
        let totals = crate::spend_by_parent_category(&conn).unwrap();
        assert!(
            totals
                .iter()
                .all(|(name, _)| name != "Transfers" && name != "Income")
        );
        assert_eq!(totals.first().map(|t| t.0.as_str()), Some("Food"));
    }
}
