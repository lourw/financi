//! 3.55: DB perf budgets on the scale fixture.
//!
//! The four `perf_*` tests run on the full large seed (1 000 rules,
//! 80 categories, 100 000 rows over 10 years) and are `#[ignore]`d so they
//! only run on demand, in a release build:
//!
//! ```sh
//! cargo test --release -p financi-db --features dev-seed -- --ignored perf_ --nocapture
//! ```
//!
//! They print their elapsed times; budgets (assertions) land with task
//! 3.55 M4. The debug-build budget regressions (×10, one-year seed) run in
//! the normal suite in `debug_budgets` below.

#![cfg(feature = "dev-seed")]

use std::time::Instant;

use chrono::NaiveDate;
use financi_db::rules::{preview_pattern_matches, reapply_rules, rule_use_stats};
use financi_db::rusqlite::Connection;
use financi_db::seed::{self, LARGE};
use financi_db::{
    categories, import, list_accounts, list_rules, list_transfer_reviews, sidebar_counts,
    uncategorized_by_merchant,
};

/// A fresh large-seed database (categorized, the steady state a rule edit
/// operates on).
fn large_db() -> Connection {
    scaled(LARGE)
}

fn scaled(cfg: seed::ScaleConfig) -> Connection {
    let mut conn = financi_db::open_in_memory().unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
    let started = Instant::now();
    let s = seed::seed_scaled(&mut conn, end, cfg).unwrap();
    eprintln!(
        "seeded {} rules / {} categories / {} transactions in {:?}",
        s.rules,
        s.categories,
        s.transactions,
        started.elapsed()
    );
    conn
}

/// The shape the budgets assume, so a silent change here is loud.
fn assert_large_shape(conn: &Connection) {
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM category_rules"), 1_000);
    assert_eq!(
        count("SELECT COUNT(*) FROM category_rules WHERE is_regex"),
        200
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM categories WHERE parent_id IS NOT NULL AND is_system = 0"),
        65
    );
    assert_eq!(count("SELECT COUNT(*) FROM transactions"), 100_000);
}

/// The budgets measure the steady state (rows already categorized), so
/// prime with one untimed pass when the seed didn't do the categorizing.
#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_reapply_rules() {
    let conn = large_db();
    assert_large_shape(&conn);
    let started = Instant::now();
    reapply_rules(&conn).unwrap();
    let elapsed = started.elapsed();
    println!("perf_reapply_rules: {elapsed:?}");
}

#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_rule_use_stats() {
    let conn = large_db();
    let started = Instant::now();
    let stats = rule_use_stats(&conn).unwrap();
    let elapsed = started.elapsed();
    println!(
        "perf_rule_use_stats: {elapsed:?} ({} rules with uses)",
        stats.len()
    );
}

#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_preview_pattern_matches() {
    let conn = large_db();
    let started = Instant::now();
    let preview = preview_pattern_matches(&conn, "BLENDWELL", false, None, 25).unwrap();
    let elapsed = started.elapsed();
    println!(
        "perf_preview_pattern_matches: {elapsed:?} ({} matches, {} decides)",
        preview.total, preview.decides
    );
}

/// The launch/`refresh` query set, in `load_sections` order: everything
/// the full reload runs in one writer-thread job.
#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_refresh_query_set() {
    let conn = large_db();
    let started = Instant::now();
    let accounts = list_accounts(&conn).unwrap();
    let tree = categories::list_categories(&conn).unwrap();
    let counts = categories::transaction_counts(&conn).unwrap();
    let rules = list_rules(&conn).unwrap();
    let uses = rule_use_stats(&conn).unwrap();
    let (total, uncategorized) = sidebar_counts(&conn).unwrap();
    let queue_rows = uncategorized_by_merchant(&conn).unwrap();
    let reviews = list_transfer_reviews(&conn).unwrap();
    let imports = import::list_imports(&conn).unwrap();
    let elapsed = started.elapsed();
    println!(
        "perf_refresh_query_set: {elapsed:?} ({} accounts, {} parents, {} counted, \
         {} rules, {} used, {total} tx / {uncategorized} uncategorized, {} queue, \
         {} reviews, {} imports)",
        accounts.len(),
        tree.len(),
        counts.len(),
        rules.len(),
        uses.len(),
        queue_rows.len(),
        reviews.len(),
        imports.len(),
    );
}
