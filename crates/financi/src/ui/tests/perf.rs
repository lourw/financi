//! 3.55: UI frame budgets at the scale fixture.
//!
//! The `perf_*` tests run against the same seeds as `financi-db`'s perf
//! suite and are `#[ignore]`d so they only run on demand, in a release
//! build:
//!
//! ```sh
//! cargo test --release -p financi --bin financi --features financi-db/dev-seed -- ui::tests::perf --ignored --nocapture
//! ```
//!
//! Methodology matches `financi-db/tests/perf_scale.rs`: one untimed
//! warm-up (page cache, allocator, first paint), then timed passes; the
//! **median** is asserted, min/median/max all print.
//!
//! Budgets: a frame never exceeds 16 ms while interacting (table, rules
//! list, the de-virtualized summary grid at a 20-year range), opening
//! the Categories/Rules pages paints within a frame, the main-thread
//! apply of a refresh result stays under ~100 µs per loaded row (the
//! page is 200 rows), and a dashboard filter change lands within 100 ms.
//!
//! `debug_frame_budgets` runs in the normal suite on the one-year seed
//! with the budgets ×10, so a regression fails CI.

use std::time::{Duration, Instant};

use super::*;
use crate::ui::DashboardData;

/// Open the shell over a scale seed (same shape as the DB perf suite).
fn open_shell_scaled(
    cx: &mut TestAppContext,
    cfg: financi_db::seed::ScaleConfig,
) -> (AnyWindowHandle, Entity<Shell>) {
    let mut conn = open_in_memory().unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
    let started = Instant::now();
    let s = financi_db::seed::seed_scaled(&mut conn, end, cfg).unwrap();
    eprintln!(
        "seeded {} rules / {} categories / {} transactions in {:?}",
        s.rules,
        s.categories,
        s.transactions,
        started.elapsed()
    );
    open_shell_with_conn(cx, conn, temp_db_path())
}

/// Same reporting as the DB perf suite: median asserted, spread printed.
fn report(name: &str, ts: &[Duration], budget: Duration) {
    let mut ts = ts.to_vec();
    ts.sort();
    let (min, median, max) = (ts[0], ts[ts.len() / 2], ts[ts.len() - 1]);
    println!("{name}: min {min:?} median {median:?} max {max:?} (budget {budget:?})");
    assert!(
        median < budget,
        "{name}: median {median:?} over the {budget:?} budget (min {min:?}, max {max:?})"
    );
}

/// Time `n` paint passes of the settled window.
fn frame_times(cx: &mut TestAppContext, handle: AnyWindowHandle, n: usize) -> Vec<Duration> {
    (0..n)
        .map(|_| {
            let started = Instant::now();
            draw_frame(cx, handle);
            started.elapsed()
        })
        .collect()
}

/// The 16 ms frame budget (and its debug ×10 sibling).
const FRAME: Duration = Duration::from_millis(16);

/// The main-thread half of a refresh: turning a loaded page into view
/// state and cached cells. The apply rebuilds the whole loaded page
/// deliberately (a page is a UI update, not a layout), so the budget is
/// *no growth per row* — 100 µs a row, 200 rows. The queries themselves
/// are budgeted in `financi-db`'s suite (250 ms for the whole set).
#[gpui::test]
#[ignore = "scale perf (3.55): cargo test --release -p financi --bin financi -- ui::tests::perf --ignored"]
fn perf_apply_refresh_result(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell_scaled(cx, financi_db::seed::LARGE);
    wait_idle(cx, &shell);
    let dash = cx.update(|cx| dash(cx, &shell));
    let mut times = Vec::new();
    for _ in 0..5 {
        // Snapshot the data outside the timed span: the apply consumes
        // it, and the clone is not what is being measured.
        let data = cx.update(|cx| {
            let d = dash.read(cx);
            DashboardData {
                rows: d.table.read(cx).delegate().rows.clone(),
                total: d.row_count,
                chart: d.chart_rows.clone(),
                summary: d.summary.clone(),
                view_filter: d.view_filter.clone(),
                settings: None,
            }
        });
        let started = Instant::now();
        cx.update(|cx| {
            dash.update(cx, |d, cx| d.apply_dashboard_rows(data, cx));
        });
        times.push(started.elapsed());
    }
    report(
        "apply refresh result (200-row page)",
        &times,
        Duration::from_micros(100) * 200,
    );
}

/// Opening the Categories page (80 categories) and the Rules page
/// (1 000 rules) at scale: once loaded, a repaint of each sits inside a
/// frame — both lists are virtualized, so a frame draws the visible
/// rows only.
#[gpui::test]
#[ignore = "scale perf (3.55): cargo test --release -p financi --bin financi -- ui::tests::perf --ignored"]
fn perf_open_pages_at_scale(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_scaled(cx, financi_db::seed::LARGE);
    wait_idle(cx, &shell);
    for (page, name) in [
        (Page::Categories, "categories (80 cats)"),
        (Page::Rules, "rules (1 000 rules)"),
    ] {
        cx.update(|cx| shell.update(cx, |s, cx| s.go_page(page, cx)));
        wait_idle(cx, &shell);
        settle(cx, handle);
        let times = frame_times(cx, handle, 10);
        report(&format!("frame painting the {name} page"), &times, FRAME);
    }
}

/// A dashboard filter change at scale lands within 100 ms: one page +
/// COUNT + the aggregations, not the whole range (3.55 paging).
#[gpui::test]
#[ignore = "scale perf (3.55): cargo test --release -p financi --bin financi -- ui::tests::perf --ignored"]
fn perf_filter_change_lands_fast(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_scaled(cx, financi_db::seed::LARGE);
    wait_idle(cx, &shell);
    settle(cx, handle);
    let dash = cx.update(|cx| dash(cx, &shell));
    let mut times = Vec::new();
    for term in ["cafe", "kiosk", "market"] {
        // `apply_search` dispatches synchronously (the generation is up
        // before this line returns), so `wait_idle` waits for exactly
        // this change's load.
        let started = Instant::now();
        with_window(cx, handle, |window, cx| {
            dash.update(cx, |d, cx| d.apply_search(term, window, cx));
        });
        wait_idle(cx, &shell);
        times.push(started.elapsed());
    }
    report(
        "dashboard filter change lands",
        &times,
        Duration::from_millis(100),
    );
}

/// Hover, scroll and typing never blow a frame on the one-year seed
/// (10 000 rows — one page shows the top, paging keeps the delegate at
/// 200 rows no matter how deep the range).
#[gpui::test]
#[ignore = "scale perf (3.55): cargo test --release -p financi --bin financi -- ui::tests::perf --ignored"]
fn perf_frames_while_interacting(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_scaled(cx, financi_db::seed::YEAR);
    wait_idle(cx, &shell);
    settle(cx, handle);
    let dash = cx.update(|cx| dash(cx, &shell));
    let mut times = Vec::new();
    // Wheel the table down and back: the virtual list recycles rows.
    for _ in 0..8 {
        let at = {
            let mut window = VisualTestContext::from_window(handle, cx);
            window
                .debug_bounds("panel:table")
                .map(viewport_centre)
                .unwrap_or_else(|| point(px(600.), px(700.)))
        };
        wheel(cx, handle, at, 0., -300.);
        let started = Instant::now();
        draw_frame(cx, handle);
        times.push(started.elapsed());
    }
    report("frame while wheeling the table", &times, FRAME);
    // Hover across rows: a hovered row repaints, the frame does not.
    let mut times = Vec::new();
    for step in 0..8u32 {
        {
            let mut window = VisualTestContext::from_window(handle, cx);
            window.simulate_mouse_move(
                point(px(600.), px(500.) + px(24.) * step as f32),
                None,
                Modifiers::default(),
            );
        }
        let started = Instant::now();
        draw_frame(cx, handle);
        times.push(started.elapsed());
    }
    report("frame while hovering rows", &times, FRAME);
    // Typing in the search box: keystroke handling and the debounced
    // dispatch, measured per frame.
    let mut times = Vec::new();
    for ch in "coffee".chars() {
        with_window(cx, handle, |window, cx| {
            dash.update(cx, |d, cx| {
                d.search_input.update(cx, |input, cx| {
                    input.set_value(format!("query {ch}"), window, cx)
                });
                let _ = window;
            });
        });
        let started = Instant::now();
        draw_frame(cx, handle);
        times.push(started.elapsed());
    }
    report("frame while typing the search", &times, FRAME);
}

/// The Rules page with 1 000 rules: scrolling the list never blows a
/// frame (the list is virtual; a frame draws the visible rules).
#[gpui::test]
#[ignore = "scale perf (3.55): cargo test --release -p financi --bin financi -- ui::tests::perf --ignored"]
fn perf_rules_page_scrolls_at_1000(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_scaled(cx, financi_db::seed::LARGE);
    wait_idle(cx, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    wait_idle(cx, &shell);
    settle(cx, handle);
    let mut times = Vec::new();
    for _ in 0..12 {
        // The rules list fills the page; aim at its middle.
        wheel(cx, handle, point(px(700.), px(700.)), 0., -400.);
        let started = Instant::now();
        draw_frame(cx, handle);
        times.push(started.elapsed());
    }
    report("frame while wheeling 1 000 rules", &times, FRAME);
}

/// The Summary panel is not virtualized (3.100): at an All-time range
/// over a 20-year seed the grid is ~80 category rows × ~240 month
/// columns. A frame of it must still hold the budget; this is the test
/// that says what it actually costs.
#[gpui::test]
#[ignore = "scale perf (3.55): cargo test --release -p financi --bin financi -- ui::tests::perf --ignored"]
fn perf_summary_twenty_years(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_scaled(
        cx,
        financi_db::seed::ScaleConfig {
            rules: 300,
            transactions: 100_000,
            years: 20,
        },
    );
    wait_idle(cx, &shell);
    let dash = cx.update(|cx| dash(cx, &shell));
    with_window(cx, handle, |window, cx| {
        dash.update(cx, |d, cx| {
            d.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });
    wait_idle(cx, &shell);
    settle(cx, handle);
    cx.update(|cx| {
        let d = dash.read(cx);
        assert!(
            d.summary.months.len() >= 239,
            "a 20-year seed must span ~240 months, spans {}",
            d.summary.months.len()
        );
    });
    let times = frame_times(cx, handle, 10);
    report("frame painting 240-month summary", &times, FRAME);
}

/// The debug CI net (×10 budgets) on the one-year seed: the same paths
/// the release budgets measure, so a regression fails a normal `cargo
/// test`. The 10 000-row seed loads one page, paints the pages and
/// lands a filter change within ten times the shipped budgets.
#[gpui::test]
fn debug_frame_budgets(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_scaled(cx, financi_db::seed::YEAR);
    wait_idle(cx, &shell);
    settle(cx, handle);
    let dash = cx.update(|cx| dash(cx, &shell));
    let mut times = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        draw_frame(cx, handle);
        times.push(started.elapsed());
    }
    report("debug frame, dashboard", &times, FRAME * 10);

    let started = Instant::now();
    with_window(cx, handle, |window, cx| {
        dash.update(cx, |d, cx| d.apply_search("cafe", window, cx));
    });
    wait_idle(cx, &shell);
    report(
        "debug filter change lands",
        &[started.elapsed()],
        Duration::from_millis(1_000),
    );

    for page in [Page::Categories, Page::Rules] {
        cx.update(|cx| shell.update(cx, |s, cx| s.go_page(page, cx)));
        wait_idle(cx, &shell);
        settle(cx, handle);
        let mut times = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            draw_frame(cx, handle);
            times.push(started.elapsed());
        }
        report(&format!("debug frame, {page:?} page"), &times, FRAME * 10);
    }
}
