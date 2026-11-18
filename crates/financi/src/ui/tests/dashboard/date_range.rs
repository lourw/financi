//! Dashboard date range: presets, custom ranges and their persistence.

use super::*;

#[gpui::test]
fn shell_starts_on_the_default_last_three_months(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |_| {});
    // Before the restore lands `view_filter` is still the open default;
    // narrower bounds prove the saved setting (here: none) was applied.
    wait_until(cx, "default date range restored", |cx| {
        dash(cx, &shell).read(cx).view_filter.from > ViewFilter::default().from
    });
    let (from, to) = preset_range(DatePreset::Last3Months, DashboardState::today());
    let state = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        (d.filter.date_range, d.view_filter.from, d.view_filter.to)
    });
    assert_eq!(
        state,
        (DateRange::Preset(DatePreset::Last3Months), from, to)
    );
    // The default is not persisted; the setting only appears on change.
    assert_eq!(db_setting(cx, &shell, UI_DATE_RANGE), None);
}

#[gpui::test]
fn choosing_a_preset_updates_the_filter_and_persists(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "date range restored", |cx| {
        dash(cx, &shell).read(cx).view_filter.from > ViewFilter::default().from
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::LastMonth, window, cx)
        });
    });

    let (from, to) = preset_range(DatePreset::LastMonth, DashboardState::today());
    let state = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        (d.filter.date_range, d.view_filter.from, d.view_filter.to)
    });
    assert_eq!(state, (DateRange::Preset(DatePreset::LastMonth), from, to));
    // Let the persist task run until it awaits the worker, so its write
    // job is queued ahead of the read below (the worker is FIFO).
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("last1"),
        "ui.date_range was written"
    );
}

/// One spend and one income row in the current month and one pair three
/// calendar months back (just outside "Last 3 months"), so the default
/// preset and "All time" show different totals. Dates are derived from
/// `DashboardState::today()`, so the test passes whenever it runs.
fn seed_preset_totals_rows(conn: &financi_db::rusqlite::Connection) {
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
    let salary =
        financi_db::categories::create_parent(conn, "Salary", CategoryKind::Income).unwrap();

    // Last day of the month three before the current one: always outside
    // "Last 3 months" (which opens on the first of two months back) and
    // exactly three month columns away, so the All-time grid is four
    // months wide whatever day the test runs.
    let today = DashboardState::today();
    let old = today
        .with_day(1)
        .expect("first of the month")
        .checked_sub_months(chrono::Months::new(2))
        .expect("two months back")
        .checked_sub_days(chrono::Days::new(1))
        .expect("last day of the month before that");
    let mut line = 0i64;
    let mut txn = |description: &str, date: NaiveDate, cents: i64, category: i64| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("preset-totals-hash-{line}"),
                date.format("%Y-%m-%d").to_string(),
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn("GROCER STORE OLD", old, -5000, groceries);
    txn("PAYROLL OLD", old, 100000, salary);
    txn("GROCER STORE CUR", today, -1500, groceries);
    txn("PAYROLL CUR", today, 210500, salary);
}

/// The month columns plus the `Total`, for the three total rows, and the
/// Food bar of the chart — everything the dashboard shows as a number.
fn dashboard_totals(
    shell: &Entity<Shell>,
    cx: &App,
) -> (Vec<String>, Vec<i64>, Vec<i64>, Vec<i64>, i64) {
    let d = dash(cx, shell).read(cx);
    let cells = |name: &str| {
        d.summary_rows()
            .into_iter()
            .find(|r| r.name == name)
            .expect("a total row")
            .cents
    };
    let food = d
        .chart_rows
        .iter()
        .find(|r| r.name == "Food")
        .expect("Food legend row")
        .spend_cents;
    (
        d.summary.months.clone(),
        cells("Income"),
        cells("Spend"),
        cells("Net"),
        food,
    )
}

#[gpui::test]
fn changing_the_preset_updates_the_dashboard_totals(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_preset_totals_rows);
    let today = DashboardState::today();
    let month_label = |n: u32| {
        today
            .checked_sub_months(chrono::Months::new(n))
            .expect("month back")
            .format("%Y-%m")
            .to_string()
    };
    let last3: Vec<String> = [2, 1, 0].iter().map(|n| month_label(*n)).collect();
    // All time clamps the grid to the data span: the old row's month plus
    // the three columns of the default range.
    let all_months: Vec<String> = [3, 2, 1, 0].iter().map(|n| month_label(*n)).collect();

    // Default "Last 3 months": only the current month's rows count; the
    // grid shows the three months of the range.
    wait_until(cx, "last-3-months totals", |cx| {
        dash(cx, &shell).read(cx).row_count == 2
            && dash(cx, &shell).read(cx).summary.months == last3
    });
    let totals = cx.update(|cx| dashboard_totals(&shell, cx));
    assert_eq!(
        totals,
        (
            last3.clone(),
            vec![0, 0, 210500, 210500],
            vec![0, 0, 1500, 1500],
            vec![0, 0, 209000, 209000],
            1500,
        ),
        "only the current month is inside the default range"
    );

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });

    // "All time" pulls in the old pair: the grid clamps to the data span
    // and every number grows.
    wait_until(cx, "all-time totals", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
            && dash(cx, &shell).read(cx).summary.months == all_months
    });
    assert_eq!(
        cx.update(|cx| dashboard_totals(&shell, cx)),
        (
            all_months,
            vec![100000, 0, 0, 210500, 310500],
            vec![5000, 0, 0, 1500, 6500],
            vec![95000, 0, 0, 209000, 304000],
            6500,
        ),
        "the preset change moved the visible totals"
    );
}

#[gpui::test]
fn a_saved_custom_range_is_restored_at_launch(cx: &mut TestAppContext) {
    let from = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let (_handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "custom:2026-03-01..2026-03-31").unwrap();
    });
    wait_until(cx, "custom range restored", |cx| {
        dash(cx, &shell).read(cx).filter.date_range == DateRange::Custom { from, to }
    });
    let state = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        (d.view_filter.from, d.view_filter.to)
    });
    assert_eq!(state, (from, to));
}

/// 3.65: launch issues one full reload plus the dashboard's own load
/// (3.56) — two jobs, no more — and the dashboard's load already applies
/// the saved settings: the first data frame shows the saved custom
/// range, and nothing re-queries. `job_count` counts every job the
/// worker accepted, so two launch jobs mean no fix-up refresh.
#[gpui::test]
fn launch_runs_one_refresh_with_the_saved_date_range(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        seed_two_accounts_and_a_transfer(c);
        settings::set(c, UI_DATE_RANGE, "custom:2026-11-05..2026-11-05").unwrap();
    });
    // The saved range keeps only the 2026-11-05 row. The default range
    // would show two, so a table with one row proves the very first query
    // used the saved setting — there is no second refresh fixing it up.
    wait_until(cx, "first data uses the saved range", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    // Read the count before anything else can queue a job (`db_setting`
    // and friends push their own).
    let (jobs, range, from, to) = cx.update(|cx| {
        let s = shell.read(cx);
        let d = dash(cx, &shell).read(cx);
        (
            s.db.job_count(),
            d.filter.date_range,
            d.view_filter.from,
            d.view_filter.to,
        )
    });
    assert_eq!(jobs, 2, "one launch = the full reload + the dashboard load");
    let day = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
    assert_eq!(range, DateRange::Custom { from: day, to: day });
    assert_eq!(
        (from, to),
        (day, day),
        "the table filter shows the saved range"
    );
}

#[gpui::test]
fn a_garbage_setting_falls_back_to_the_default(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "last13").unwrap();
    });
    wait_until(cx, "garbage replaced by default", |cx| {
        dash(cx, &shell).read(cx).view_filter.from > ViewFilter::default().from
    });
    let range = cx.update(|cx| dash(cx, &shell).read(cx).filter.date_range);
    assert_eq!(range, DateRange::Preset(DatePreset::Last3Months));
}

#[gpui::test]
fn picking_a_custom_range_persists_and_normalizes(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "date range restored", |cx| {
        dash(cx, &shell).read(cx).view_filter.from > ViewFilter::default().from
    });

    // Switch to "Custom…", then pick both dates with the pickers.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::Custom, window, cx)
        });
    });
    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.date_picked(false, start, window, cx);
            d.date_picked(true, end, window, cx);
        });
    });
    // Let the persist task queue its write before reading (the worker is FIFO).
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("custom:2026-03-01..2026-03-31"),
        "the picker writes the custom range"
    );
    let state = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        (d.filter.date_range, d.view_filter.from, d.view_filter.to)
    });
    assert_eq!(
        state,
        (
            DateRange::Custom {
                from: start,
                to: end
            },
            start,
            end
        )
    );

    // Picking the end before the start persists the swapped pair, and the
    // pickers show the corrected order rather than the reversed pick.
    let early = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.date_picked(true, early, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("custom:2026-01-15..2026-03-01"),
        "a reversed pick is stored swapped"
    );
    let shown = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        (
            d.view_filter.from,
            d.view_filter.to,
            d.date_from_picker.read(cx).date(),
            d.date_to_picker.read(cx).date(),
        )
    });
    assert_eq!(
        shown,
        (
            early,
            start,
            CalendarDate::Single(Some(early)),
            CalendarDate::Single(Some(start)),
        ),
        "the pickers show the swapped range"
    );

    // Picking the start after the end normalizes the same way.
    let late = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.date_picked(false, late, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("custom:2026-03-01..2026-09-01"),
        "the start handler normalizes too"
    );
}

// ---------------------------------------------------------------------------
// Accounts filter + transfers toggle (task 3.3)
