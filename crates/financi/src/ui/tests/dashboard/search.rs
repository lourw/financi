//! Dashboard search box: filtering, debounce, Enter and cancellation.

use gpui_kit::component::input::InputEvent;

use super::*;

#[gpui::test]
fn search_filters_the_table_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    // A case-insensitive substring of the description.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.apply_search("cafe", window, cx));
    });
    wait_until(cx, "one cafe row", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "CORNER CAFE");
    });
    // A match on the merchant column.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.apply_search("fresh", window, cx));
    });
    wait_until(cx, "merchant match", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
            && table_rows(&shell, cx)[0].merchant.as_deref() == Some("Fresh Mart")
    });
    // Clearing the box brings every row back.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.apply_search("", window, cx));
    });
    wait_until(cx, "all rows back", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
}

/// 3.56: a filter change runs the dashboard's one load and nothing
/// else — the QueryLog shows exactly its two queries, none of the other
/// pages', and exactly one worker job is accepted.
#[gpui::test]
fn a_filter_change_runs_only_the_dashboard_job(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    let jobs_before = cx.update(|cx| {
        let s = shell.read(cx);
        s.db.query_log().clear();
        assert!(s.db.query_log().names().is_empty());
        s.db.job_count()
    });
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.apply_search("cafe", window, cx));
    });
    // Not `wait_idle`: its drain probes would be counted as jobs.
    wait_until(cx, "search applied", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    let (names, jobs) = cx.update(|cx| {
        let s = shell.read(cx);
        (s.db.query_log().names(), s.db.job_count())
    });
    assert_eq!(
        names,
        vec![
            "list_transactions_filtered".to_string(),
            "dashboard".to_string()
        ],
        "a filter change must run the dashboard's one load and nothing else"
    );
    assert_eq!(jobs, jobs_before + 1, "one filter change, one job");
}

/// 3.56: typing debounces — five keystrokes replace one another and run
/// one query after the last key, not five. The clock is virtual, so
/// nothing fires until the test advances it.
#[gpui::test]
fn typing_debounces_into_one_query(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    let jobs_before = cx.update(|cx| shell.read(cx).db.job_count());
    // Five keystrokes, all inside the debounce window.
    for text in ["c", "ca", "caf", "cafe", "cafe "] {
        with_window(cx, handle, |window, cx| {
            dash(cx, &shell).update(cx, |d, cx| {
                d.search_input.update(cx, |i, cx| {
                    i.set_value(text, window, cx);
                    cx.emit(InputEvent::Change);
                });
            });
        });
    }
    // No query yet: each keystroke replaced the pending one.
    cx.run_until_parked();
    cx.update(|cx| {
        let s = shell.read(cx);
        let d = dash(cx, &shell).read(cx);
        assert_eq!(
            s.db.job_count(),
            jobs_before,
            "typing must not query before the debounce settles"
        );
        assert!(
            d.filter.search.is_empty(),
            "the search filter is still unset while typing"
        );
    });
    // Just short of the deadline: still nothing.
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| shell.read(cx).db.job_count()),
        jobs_before,
        "the debounce has not expired yet"
    );
    // Past it: one job, for the final text only.
    cx.executor().advance_clock(Duration::from_millis(100));
    wait_until(cx, "the last keystroke's query ran", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    let jobs = cx.update(|cx| shell.read(cx).db.job_count());
    assert_eq!(jobs, jobs_before + 1, "five keystrokes, one job");
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "CORNER CAFE");
    });
}

/// 3.56: a burst of filter changes ends with the newest filter's rows —
/// a superseded load never overwrites them.
#[gpui::test]
fn a_burst_of_filter_changes_lands_the_last_one(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    // Three changes dispatched back to back, no settling between them.
    for term in ["fresh", "payroll", "cafe"] {
        with_window(cx, handle, |window, cx| {
            dash(cx, &shell).update(cx, |d, cx| d.apply_search(term, window, cx));
        });
    }
    wait_until(cx, "the last search's rows", |cx| {
        let rows = table_rows(&shell, cx);
        rows.len() == 1 && rows[0].description == "CORNER CAFE"
    });
    // With everything idle — every in-flight and superseded job has
    // resolved — the screen still shows the newest filter's rows.
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.filter.search, "cafe");
        let rows = table_rows(&shell, cx);
        assert_eq!(
            rows.len(),
            1,
            "a superseded result overwrote the newest one: {rows:?}"
        );
        assert_eq!(rows[0].description, "CORNER CAFE");
    });
}

/// 3.56: Enter applies the search at once — it does not wait for the
/// debounce — and the pending debounced task is cancelled, not fired
/// later on top of the applied result.
#[gpui::test]
fn enter_applies_the_search_at_once(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.search_input.update(cx, |i, cx| {
                i.set_value("payroll", window, cx);
                cx.emit(InputEvent::Change);
            });
        });
    });
    // Enter jumps the queue: the term applies without the clock moving.
    with_window(cx, handle, |_window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.search_input.update(cx, |_, cx| {
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
    });
    wait_until(cx, "Enter applied the term at once", |cx| {
        let rows = table_rows(&shell, cx);
        rows.len() == 1 && rows[0].description == "PAYROLL DEPOSIT"
    });
    // Past the debounce window: the cancelled task fires nothing.
    let jobs = cx.update(|cx| shell.read(cx).db.job_count());
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| shell.read(cx).db.job_count()),
        jobs,
        "the debounced task Enter replaced still fired a query"
    );
}

/// 3.56: "Clear filters" inside the search's 200 ms debounce window drops
/// the pending task — the cleared box must not be undone by the dead term's
/// query firing after the click.
#[gpui::test]
fn clearing_filters_cancels_the_pending_search(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    let jobs_before = cx.update(|cx| {
        let s = shell.read(cx);
        s.db.query_log().clear();
        s.db.job_count()
    });
    // One keystroke: the debounced query is pending, nothing has run.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.search_input.update(cx, |i, cx| {
                i.set_value("cafe", window, cx);
                cx.emit(InputEvent::Change);
            });
        });
    });
    // Inside the window — 150 ms after the keystroke — Clear wins.
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| shell.read(cx).db.job_count()),
        jobs_before,
        "the debounce has not expired yet"
    );
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.clear_all_filters(window, cx));
    });
    // Past the window: the dropped task fires nothing, and only Clear's
    // own re-query runs.
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    wait_until(cx, "the clear's re-query ran", |cx| {
        shell
            .read(cx)
            .db
            .query_log()
            .names()
            .iter()
            .any(|n| n == "dashboard")
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        let d = dash(cx, &shell).read(cx);
        // 3.46: "Clear all" also persists the reset date range and
        // transfers flag, so count the chart/summary loads, not raw jobs.
        let dash_runs =
            s.db.query_log()
                .names()
                .iter()
                .filter(|n| *n == "dashboard")
                .count();
        assert_eq!(
            dash_runs, 1,
            "Clear runs its own one re-query; the cancelled debounce must add none"
        );
        assert!(
            d.search_input.read(cx).value().is_empty(),
            "the search box stays empty"
        );
        assert!(d.filter.search.is_empty());
        assert_eq!(d.row_count, 3, "the rows stay unfiltered");
    });
}
