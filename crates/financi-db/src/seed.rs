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

// ---------------------------------------------------------------------------
// 3.55: the scale fixture — the user's shape (thousands of rules, dozens of
// categories, years of rows) for the perf budgets. Synthetic only, fully
// deterministic like the small seed.
// ---------------------------------------------------------------------------

/// Shape of a scale seed (3.55). `rules` must be a multiple of 5 (four
/// substring rules and one regex rule per generated brand) and at least 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaleConfig {
    /// Total rules: 80 % substring, 20 % regex, overlapping on purpose.
    pub rules: u32,
    /// Transactions spread over `years`.
    pub transactions: u32,
    pub years: u32,
}

/// The 3.55 scale shape: 1 000 rules, 80 categories, 100 000 rows over
/// 10 years (~10 000 a year, the user's expectation).
pub const LARGE: ScaleConfig = ScaleConfig {
    rules: 1_000,
    transactions: 100_000,
    years: 10,
};

/// The CI shape for the debug-build budgets (3.55): one year of the user's
/// volume against a few hundred rules.
pub const YEAR: ScaleConfig = ScaleConfig {
    rules: 300,
    transactions: 10_000,
    years: 1,
};

/// The scale fixture's categories: 15 parents, 65 children, 80 in total.
const SCALE_PARENTS: &[(&str, &[&str])] = &[
    (
        "Groceries",
        &[
            "Supermarkets",
            "Specialty Foods",
            "Butchers",
            "Fishmongers",
            "Farmers Markets",
        ],
    ),
    (
        "Dining",
        &["Fast Food", "Family Restaurants", "Fine Dining", "Takeout"],
    ),
    (
        "Coffee & Bakery",
        &["Coffee Shops", "Bakeries", "Donuts", "Desserts"],
    ),
    (
        "Transportation",
        &[
            "Transit Fares",
            "Rideshare",
            "Parking",
            "Tolls",
            "Bike Share",
        ],
    ),
    (
        "Shopping",
        &[
            "Department Stores",
            "Clothing",
            "Shoes",
            "Home Goods",
            "Toys",
        ],
    ),
    (
        "Utilities",
        &["Electricity", "Water", "Gas", "Phone", "Internet"],
    ),
    (
        "Entertainment",
        &["Cinemas", "Concerts", "Streaming", "Games", "Museums"],
    ),
    (
        "Travel",
        &["Flights", "Hotels", "Rental Cars", "Travel Agencies"],
    ),
    (
        "Health & Fitness",
        &["Pharmacies", "Dentists", "Gyms", "Doctors", "Optometrists"],
    ),
    (
        "Home",
        &[
            "Rent",
            "Maintenance",
            "Furniture",
            "Appliances",
            "Gardening",
        ],
    ),
    (
        "Auto",
        &["Fuel", "Repairs", "Insurance", "Car Washes", "Parts"],
    ),
    (
        "Education",
        &["Tuition", "Courses", "Supplies", "Libraries"],
    ),
    ("Subscriptions", &["News", "Music", "Cloud Storage", "Apps"]),
    ("Gifts", &["Presents", "Charitable Donations", "Flowers"]),
    ("Other", &["Miscellaneous Spend", "Uncategorized Spend"]),
];

/// Word stems for the generated brand names: 20 × 10 = 200 brands.
const SCALE_STEMS1: &[&str] = &[
    "BLEND", "CREST", "NOVA", "PINE", "RIVER", "SUN", "STAR", "BLUE", "GREEN", "URBAN", "PRIME",
    "VALUE", "CROWN", "EAGLE", "HARBOR", "MAPLE", "ROCKY", "SILVER", "GOLDEN", "NORTH",
];
const SCALE_STEMS2: &[&str] = &[
    "WELL", "MARKET", "MART", "SHOP", "DEPOT", "PLAZA", "WORKS", "FIELDS", "GATE", "LAND",
];
const SCALE_CITIES: &[&str] = &[
    "VANCOUVER",
    "BURNABY",
    "RICHMOND",
    "SURREY",
    "VICTORIA",
    "CALGARY",
    "TORONTO",
    "OTTAWA",
    "MONTRÉAL",
    "EDMONTON",
    "WINNIPEG",
    "HALIFAX",
];
const SCALE_SUFFIXES: &[&str] = &["STORE", "OUTLET", "MARKET", "EXPRESS", "DEPOT"];

/// One generated scale rule, before insertion.
struct ScaleRule {
    pattern: String,
    is_regex: bool,
    /// Index into the flat 65-child list.
    child: usize,
}

/// The scale rule set: four substring rules and one regex rule per brand,
/// overlapping on purpose (the brand token matches everything its longer
/// rules match, and the four substring variants and the regex disagree on
/// their target child, so every row exercises the tie-break too).
fn scale_rules(brands: u32, child_count: u32) -> Vec<ScaleRule> {
    let mut rules = Vec::with_capacity((brands * 5) as usize);
    for b in 0..brands {
        let brand = format!(
            "{}{}",
            SCALE_STEMS1[(b % 20) as usize],
            SCALE_STEMS2[(b / 20) as usize]
        );
        let city = SCALE_CITIES[(b % SCALE_CITIES.len() as u32) as usize];
        let suffix = SCALE_SUFFIXES[(b % SCALE_SUFFIXES.len() as u32) as usize];
        rules.push(ScaleRule {
            pattern: brand.clone(),
            is_regex: false,
            child: ((b * 5) % child_count) as usize,
        });
        rules.push(ScaleRule {
            pattern: format!("{brand} STORE"),
            is_regex: false,
            child: ((b * 5 + 1) % child_count) as usize,
        });
        rules.push(ScaleRule {
            pattern: format!("{brand} {city}"),
            is_regex: false,
            child: ((b * 5 + 2) % child_count) as usize,
        });
        rules.push(ScaleRule {
            pattern: format!("{brand} {suffix}"),
            is_regex: false,
            child: ((b * 5 + 3) % child_count) as usize,
        });
        rules.push(ScaleRule {
            pattern: format!("{brand} #\\d+"),
            is_regex: true,
            child: ((b * 5 + 4) % child_count) as usize,
        });
    }
    rules
}

/// The row's description for brand `b`: `BRAND SUFFIX #NNN CITY`, padded
/// like an Amex descriptor. Deterministic under `rng`.
fn scale_description(b: u32, rng: &mut Rng) -> String {
    let brand = format!(
        "{}{}",
        SCALE_STEMS1[(b % 20) as usize],
        SCALE_STEMS2[(b / 20) as usize]
    );
    let suffix = SCALE_SUFFIXES[(rng.below(SCALE_SUFFIXES.len() as u64)) as usize];
    let city = SCALE_CITIES[(rng.below(SCALE_CITIES.len() as u64)) as usize];
    let num = 100 + rng.below(900);
    format!("{brand} {suffix} #{num} {city}")
}

/// Fill an empty database with the 3.55 scale shape: 80 categories (15
/// parents), `cfg.rules` overlapping rules (80 % substring, 20 % regex) and
/// `cfg.transactions` synthetic transactions over `cfg.years` years,
/// categorized by those rules. Everything derives from a fixed PRNG seed,
/// so the rows are identical on every run. Only synthetic data.
pub fn seed_scaled(conn: &mut Connection, end: NaiveDate, cfg: ScaleConfig) -> Result<SeedSummary> {
    assert!(
        cfg.rules >= 5 && cfg.rules.is_multiple_of(5),
        "one substring group and one regex rule per brand: rules must be a positive multiple of 5"
    );
    let brands = cfg.rules / 5;
    let tx = conn.transaction()?;

    // Categories: 15 parents + 65 children = 80. Children are addressed by
    // their flat index; names are globally unique.
    let mut child_names: Vec<&str> = Vec::new();
    let mut categories = 0usize;
    for (parent, children) in SCALE_PARENTS {
        tx.execute(
            "INSERT INTO categories (name, parent_id, kind) VALUES (?1, NULL, ?2)",
            params![parent, CategoryKind::Expense.as_str()],
        )?;
        let parent_id = tx.last_insert_rowid();
        categories += 1;
        for child in *children {
            tx.execute(
                "INSERT INTO categories (name, parent_id, kind) VALUES (?1, ?2, ?3)",
                params![child, parent_id, CategoryKind::Expense.as_str()],
            )?;
            categories += 1;
            child_names.push(child);
        }
    }
    let child_count = child_names.len() as u32;
    // Children were inserted in `child_names` order, so ids follow that
    // order too (the migration's system children are excluded).
    let child_ids: Vec<i64> = {
        let mut ids = Vec::with_capacity(child_names.len());
        let mut stmt = tx.prepare(
            "SELECT id FROM categories WHERE parent_id IS NOT NULL AND is_system = 0 ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        for id in rows {
            ids.push(id?);
        }
        ids
    };
    assert_eq!(
        child_ids.len(),
        child_names.len(),
        "scale categories: 65 non-system children"
    );

    // Rules, inserted directly (one pass; add_rule would re-run the rules
    // once per rule).
    let rules = scale_rules(brands, child_count);
    {
        let mut insert_rule = tx.prepare(
            "INSERT INTO category_rules (pattern, is_regex, category_id) VALUES (?1, ?2, ?3)",
        )?;
        for rule in &rules {
            insert_rule.execute(params![rule.pattern, rule.is_regex, child_ids[rule.child]])?;
        }
    }
    // The same compiled set the app would use, to categorize the rows as
    // they are generated (the steady state the budgets measure).
    let core_rules: Vec<financi_core::rules::Rule> = rules
        .iter()
        .enumerate()
        .map(|(ix, r)| financi_core::rules::Rule {
            id: ix as i64 + 1,
            pattern: r.pattern.clone(),
            is_regex: r.is_regex,
            category_id: child_ids[r.child],
        })
        .collect();
    let compiled = financi_core::rules::CompiledRules::new(&core_rules);

    let account_id = crate::create_account(
        &tx,
        "amex",
        "Scale Cobalt (synthetic)",
        financi_core::AccountKind::CreditCard,
    )?;
    let account_key = AccountKey(tx.query_row(
        "SELECT account_key FROM accounts WHERE id = ?1",
        [account_id],
        |r| r.get(0),
    )?);

    // Rows: charges positive (Amex style), ~3 % refunds, spread over the
    // window; newest first like the export.
    let days = i64::from(cfg.years) * 365;
    let start = end - Duration::days(days);
    let mut rng = Rng(0x5C41_E57A_9355_0001);
    let mut rows: Vec<(NaiveDate, String, i64, Option<i64>)> = Vec::new();
    for _ in 0..cfg.transactions {
        let b = rng.below(brands as u64) as u32;
        let day = start + Duration::days(rng.below(days as u64 + 1) as i64);
        let desc = scale_description(b, &mut rng);
        let mut cents = 200 + rng.below(40_000) as i64;
        if rng.below(100) < 3 {
            cents = -cents / 2; // refunds are smaller and negative in the export
        }
        let category = compiled.pick(&desc);
        rows.push((day, desc, cents, category));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));

    tx.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'dev-seed-large.csv', ?2, ?3, 'utf-8', 0, 'lf', 0, ?4)",
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
        for (line, (date, desc, charge_cents, category)) in rows.iter().enumerate() {
            let date_raw = date.format("%d %b %Y").to_string();
            let processed_raw = (*date + Duration::days(2)).format("%d %b %Y").to_string();
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
                processed_raw,
                -charge_cents, // financi convention: money out is negative
                desc,
                merchant,
                category,
            ])?;
        }
    }

    let rule_count = rules.len();
    let transactions = rows.len();
    tx.commit()?;
    Ok(SeedSummary {
        categories,
        rules: rule_count,
        transactions,
    })
}

/// The 1 000-rule / 100 000-row scale fixture (3.55): [`seed_scaled`]
/// with [`LARGE`]. `FINANCI_DEV_SEED=large` runs this at startup.
pub fn seed_large(conn: &mut Connection, end: NaiveDate) -> Result<SeedSummary> {
    seed_scaled(conn, end, LARGE)
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

    fn scale_seeded(cfg: ScaleConfig) -> (Connection, SeedSummary) {
        let mut conn = crate::open_in_memory().unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let summary = seed_scaled(&mut conn, end, cfg).unwrap();
        (conn, summary)
    }

    /// 3.55: the scale generator's shape — categories, the 80/20 rule mix,
    /// every row categorized, all deterministic. (The full 1 000-rule /
    /// 100 000-row run is covered by the ignored perf tests in
    /// `tests/perf_scale.rs`; this pins the generator on a CI-sized shape.)
    #[test]
    fn scale_seed_shape_and_determinism() {
        let (conn, s) = scale_seeded(ScaleConfig {
            rules: 50,
            transactions: 500,
            years: 1,
        });
        assert_eq!(s.categories, 80, "15 parents + 65 children");
        assert_eq!(s.rules, 50);
        assert_eq!(s.transactions, 500);
        let (regexes, total_rules): (i64, i64) = conn
            .query_row(
                "SELECT SUM(is_regex), COUNT(*) FROM category_rules",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (regexes, total_rules),
            (10, 50),
            "80 % substring, 20 % regex"
        );
        let parents: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE parent_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parents, 16, "15 scale parents + Transfers from 0002");
        let unclassified: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE category_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            unclassified, 0,
            "every generated row matches its brand rule"
        );
        let merchants: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT description_raw) FROM transactions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(merchants > 100, "descriptions vary ({merchants} distinct)");

        let (other, _) = scale_seeded(ScaleConfig {
            rules: 50,
            transactions: 500,
            years: 1,
        });
        let raw = |c: &Connection| -> Vec<String> {
            let mut s = c
                .prepare("SELECT CAST(raw_bytes AS TEXT) FROM transactions ORDER BY id")
                .unwrap();
            s.query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(raw(&conn), raw(&other), "the scale seed is deterministic");
    }

    /// 3.55: the rules overlap on purpose — shorter patterns are contained
    /// in longer ones with different targets, so matching exercises the
    /// longest-match winner and the tie-breaks.
    #[test]
    fn scale_rules_overlap_on_purpose() {
        let rules = scale_rules(2, 65);
        let find = |p: &str| rules.iter().find(|r| r.pattern == p);
        let brand = find("BLENDWELL").expect("brand token rule");
        let longer = find("BLENDWELL STORE").expect("longer overlapping rule");
        assert_ne!(
            brand.child, longer.child,
            "overlapping rules disagree on target"
        );
        let regex = find(r"BLENDWELL #\d+").expect("regex rule per brand");
        assert!(regex.is_regex);
        let substrings = rules.iter().filter(|r| !r.is_regex).count();
        assert_eq!(substrings, 8, "4 substring rules per brand");
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
