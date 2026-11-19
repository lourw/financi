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
//! They measure the interactive steady state: one untimed warm-up pass
//! (page cache, allocator, lazy `RegexSet` build), then timed passes whose
//! **median** is asserted against the budget — min, median and max all
//! print, so a noisy machine is visible rather than a flaky assert.
//! Budgets (median): `reapply_rules` and `rule_use_stats` < 150 ms,
//! `preview_pattern_matches` < 30 ms, the full `refresh` query set
//! < 250 ms.
//!
//! `debug_budgets` runs in the normal (debug) suite on the one-year seed
//! (300 rules, 10 000 rows) with the budgets ×10, so a regression fails CI.

#![cfg(feature = "dev-seed")]

use std::time::{Duration, Instant};

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

/// One untimed warm-up pass, then `runs` timed passes, sorted.
fn timed(runs: usize, mut f: impl FnMut()) -> Vec<Duration> {
    f();
    let mut ts: Vec<Duration> = (0..runs)
        .map(|_| {
            let started = Instant::now();
            f();
            started.elapsed()
        })
        .collect();
    ts.sort();
    ts
}

/// Print min/median/max and assert the median against `budget`.
fn report(name: &str, ts: &[Duration], budget: Duration) {
    let (min, median, max) = (ts[0], ts[ts.len() / 2], ts[ts.len() - 1]);
    println!("{name}: min {min:?} median {median:?} max {max:?} (budget {budget:?})");
    assert!(
        median < budget,
        "{name}: median {median:?} over the {budget:?} budget (min {min:?}, max {max:?})"
    );
}

/// The launch/`refresh` query set, in `load_sections` order: everything
/// the full reload runs in one writer-thread job.
fn refresh_query_set(conn: &Connection) -> String {
    let accounts = list_accounts(conn).unwrap();
    let tree = categories::list_categories(conn).unwrap();
    let counts = categories::transaction_counts(conn).unwrap();
    let rules = list_rules(conn).unwrap();
    let uses = rule_use_stats(conn).unwrap();
    let (total, uncategorized) = sidebar_counts(conn).unwrap();
    let queue_rows = uncategorized_by_merchant(conn).unwrap();
    let reviews = list_transfer_reviews(conn).unwrap();
    let imports = import::list_imports(conn).unwrap();
    format!(
        "{} accounts, {} parents, {} counted, {} rules, {} used, {total} tx / \
         {uncategorized} uncategorized, {} queue, {} reviews, {} imports",
        accounts.len(),
        tree.len(),
        counts.len(),
        rules.len(),
        uses.len(),
        queue_rows.len(),
        reviews.len(),
        imports.len(),
    )
}

/// The steady state (rows already categorized) is what the budgets
/// measure, and `reapply_rules` is idempotent, so the warm-up pass and
/// every timed pass do the same work.
fn perf_reapply(budget: Duration, runs: usize) {
    let conn = large_db();
    assert_large_shape(&conn);
    report(
        "perf_reapply_rules",
        &timed(runs, || {
            reapply_rules(&conn).unwrap();
        }),
        budget,
    );
}

fn perf_stats(budget: Duration, runs: usize) {
    let conn = large_db();
    let mut with = 0usize;
    let ts = timed(runs, || {
        with = rule_use_stats(&conn).unwrap().len();
    });
    report("perf_rule_use_stats", &ts, budget);
    println!("  {with} rules with uses");
}

fn perf_preview(budget: Duration, runs: usize) {
    let conn = large_db();
    let mut shape = String::new();
    let ts = timed(runs, || {
        let p = preview_pattern_matches(&conn, "BLENDWELL", false, None, 25).unwrap();
        shape = format!("{} matches, {} decides", p.total, p.decides);
    });
    report("perf_preview_pattern_matches", &ts, budget);
    println!("  {shape}");
}

fn perf_refresh(budget: Duration, runs: usize) {
    let conn = large_db();
    let mut shape = String::new();
    let ts = timed(runs, || {
        shape = refresh_query_set(&conn);
    });
    report("perf_refresh_query_set", &ts, budget);
    println!("  {shape}");
}

#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_reapply_rules() {
    perf_reapply(Duration::from_millis(150), 5);
}

#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_rule_use_stats() {
    perf_stats(Duration::from_millis(150), 5);
}

#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_preview_pattern_matches() {
    perf_preview(Duration::from_millis(30), 7);
}

#[test]
#[ignore = "3.55 perf test: run with --release -- --ignored perf_"]
fn perf_refresh_query_set() {
    perf_refresh(Duration::from_millis(250), 5);
}

/// The CI regression net: the same four DB budgets at ×10 (debug builds
/// run these passes roughly ten times slower than release), on the
/// one-year seed (300 rules, 10 000 transactions, one year) so the whole
/// test costs seconds, not minutes.
#[test]
fn debug_budgets() {
    let conn = scaled(seed::YEAR);
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM category_rules"), 300);
    assert_eq!(count("SELECT COUNT(*) FROM transactions"), 10_000);

    perf_debug(
        "debug_reapply_rules",
        &conn,
        Duration::from_millis(1_500),
        |conn| {
            reapply_rules(conn).unwrap();
        },
    );
    perf_debug(
        "debug_rule_use_stats",
        &conn,
        Duration::from_millis(1_500),
        |conn| {
            rule_use_stats(conn).unwrap();
        },
    );
    perf_debug(
        "debug_preview_pattern_matches",
        &conn,
        Duration::from_millis(300),
        |conn| {
            preview_pattern_matches(conn, "BLENDWELL", false, None, 25).unwrap();
        },
    );
    perf_debug(
        "debug_refresh_query_set",
        &conn,
        Duration::from_millis(2_500),
        |conn| {
            refresh_query_set(conn);
        },
    );
}

fn perf_debug(name: &str, conn: &Connection, budget: Duration, f: impl Fn(&Connection)) {
    let ts = timed(3, || f(conn));
    report(name, &ts, budget);
}
