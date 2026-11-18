//! Tests for `ui/dashboard.rs` (3.62 split from `ui_tests.rs`).
//!
//! H1 split this file by feature: each sub-module below holds one feature's
//! tests, and the seeds and click helpers used by two or more of them (plus
//! the three `pub(super)` ones `tests/categories.rs` imports) live here.
//! New tests go next to related ones in the right sub-file.

use super::*;
use crate::ui::dashboard::DashboardState;

mod accounts;
mod chart;

// 3.82: the Dashboard is an entity. Tests reach its state through this
// accessor instead of `shell.read(cx).dashboard`, and drive its handlers on
// the page entity. Shell stays free whenever a test updates an entity, so
// a handler that forwards to Shell (a toast, a page switch) works.
pub(super) fn dash<C: gpui::AppContext>(cx: &C, shell: &Entity<Shell>) -> Entity<DashboardState> {
    shell.read_with(cx, |s, _| s.dashboard.clone())
}
mod chart_labels;
mod date_range;
mod drill_down;
mod global_filters;
mod layout;
mod legend_selection;
mod row_menu;
mod search;
mod stacked_bars;
mod summary;
mod summary_scroll;
mod table;

/// Food (Groceries -1000, Restaurants -500), Transport (Bus -300) and one
/// uncategorized -200, all inside the default date range: the parent chart
/// rows are Food 1500, Transport 300, Uncategorized 200.
pub(super) fn seed_chart_rows(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    let restaurants = financi_db::categories::create_child(conn, food, "Restaurants").unwrap();
    let transport =
        financi_db::categories::create_parent(conn, "Transport", CategoryKind::Expense).unwrap();
    let bus = financi_db::categories::create_child(conn, transport, "Bus").unwrap();

    let mut line = 0i64;
    let mut txn = |description: &str, cents: i64, category: Option<i64>| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-05', ?5, ?6, ?7)",
            (
                acct,
                import,
                line,
                format!("chart-hash-{line}"),
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn("GROCER STORE", -1000, Some(groceries));
    txn("CORNER CAFE", -500, Some(restaurants));
    txn("CITY BUS", -300, Some(bus));
    txn("Mystery Vendor", -200, None);
}

/// 3.75: click a legend row for real — the same `on_click` the user hits.
pub(super) fn click_chart_legend(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    id: Option<i64>,
) {
    // 3.65: a selected row paints `chart:legend-row:<ix>:active` — click
    // the selector the row answers to *now*, same rule as the checkboxes.
    let (ix, selected) = cx.update(|cx| {
        let d = dash(cx, shell).read(cx);
        let ix = d.chart_rows.iter().position(|r| r.id == id);
        let selected = match (id, d.filter.legend_selection) {
            (None, _) => {
                matches!(
                    d.filter.legend_selection,
                    Some(LegendSelection::Uncategorized)
                ) || d.filter.uncategorized_only
            }
            (Some(id), Some(LegendSelection::Category(c))) => c == id,
            _ => false,
        };
        (ix, selected)
    });
    let Some(ix) = ix else {
        panic!("category {id:?} is one of the legend's rows (wait for the refresh that shows it)")
    };
    let suffix = if selected { ":active" } else { "" };
    click(cx, handle, format!("chart:legend-row:{ix}{suffix}"));
}

pub(super) fn top_category_id(shell: &Entity<Shell>, cx: &App, name: &str) -> i64 {
    dash(cx, shell)
        .read(cx)
        .chart_rows
        .iter()
        .find(|r| r.name == name)
        .expect("chart row")
        .id
        .expect("a real category")
}

/// Two Amex accounts: Cobalt gets a normal spend row and a
/// transfer-category row, Gold one normal spend row. With transfers off
/// the table starts at two rows, one per account.
fn seed_two_accounts_and_a_transfer(conn: &financi_db::rusqlite::Connection) {
    let cobalt = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let gold = create_account(conn, "amex", "Gold", AccountKind::CreditCard).unwrap();
    let import = |account: i64| -> i64 {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
            [account],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let txn =
        |account: i64, import: i64, line: i64, date: &str, cents: i64, category: Option<i64>| {
            conn.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, category_id)
                 VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, 'ROW', ?7)",
                (
                    account,
                    import,
                    line,
                    format!("hash-{account}-{line}"),
                    date,
                    cents,
                    category,
                ),
            )
            .unwrap();
        };
    let transfer = financi_db::categories::card_payment_category(conn).unwrap();
    let ci = import(cobalt);
    let gi = import(gold);
    txn(cobalt, ci, 0, "2026-11-03", -150, None);
    txn(cobalt, ci, 1, "2026-11-04", -5000, Some(transfer));
    txn(gold, gi, 0, "2026-11-05", -250, None);
}

/// One account with a categorized spend, an uncategorized spend, a money-in
/// row and a hidden spend, so the search box and the two table-only toggles
/// each change which rows the table shows.
fn seed_table_filter_rows(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();

    let mut line = 0i64;
    let mut txn =
        |description: &str, merchant: Option<&str>, cents: i64, category: Option<i64>| -> i64 {
            conn.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, merchant, category_id)
                 VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-05', ?5, ?6, ?7, ?8)",
                (
                    acct,
                    import,
                    line,
                    format!("hash-{line}"),
                    cents,
                    description,
                    merchant,
                    category,
                ),
            )
            .unwrap();
            line += 1;
            conn.last_insert_rowid()
        };
    txn("CORNER CAFE", None, -500, None);
    txn("GROCER STORE", Some("Fresh Mart"), -1200, Some(groceries));
    txn("PAYROLL DEPOSIT", None, 3000, None);
    let hidden = txn("HIDDEN SPEND", None, -700, None);
    financi_db::set_hidden(conn, hidden, true).unwrap();
}

fn set_uncategorized_only(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    on: bool,
) {
    with_window(cx, handle, |window, cx| {
        dash(cx, shell).update(cx, |d, cx| d.set_uncategorized_only(on, window, cx));
    });
}
