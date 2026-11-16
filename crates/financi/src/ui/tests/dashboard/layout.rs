//! Dashboard page layout: table width plan, panels, filter row, alignment.

use super::*;

/// 3.25: the dashboard table's width plan — the flexed `Account`,
/// `Description` and `Category` columns plus the fixed ones always sum to
/// exactly the page width, so the table reaches the page's right edge
/// without overhanging it at any window size (the wrapper alone cannot
/// pin this, and the old fixed 300/360/260 columns overflowed 1200 px).
#[test]
fn dashboard_table_columns_fit_every_page_width() {
    for win in [1200., 1600., 1920., 2560.] {
        let page_w = win - SIDEBAR_W - 2. * PAGE_PAD;
        let (account, description, category) = tx_flex_widths(page_w);
        let total = TX_FIXED_W + TABLE_BORDER_X + account + description + category;
        assert!(
            (total - page_w).abs() <= 0.01,
            "window {win}: columns sum to {total}, page is {page_w}"
        );
        assert!(
            account >= HISTORY_FLEX_MIN_W
                && description >= HISTORY_FLEX_MIN_W
                && category >= HISTORY_FLEX_MIN_W,
            "window {win}: flexed columns below the floor"
        );
    }
    // The columns grow with the window...
    let small = tx_flex_widths(1200. - SIDEBAR_W - 2. * PAGE_PAD).1;
    let big = tx_flex_widths(2560. - SIDEBAR_W - 2. * PAGE_PAD).1;
    assert!(big > small, "{big} vs {small}");
    // ...and below any real window the flexed total stops at its floor.
    let (a, d, c) = tx_flex_widths(100.);
    assert!(
        (a + d + c - 3. * HISTORY_FLEX_MIN_W).abs() <= 0.01,
        "floored total {} is not 3 × {HISTORY_FLEX_MIN_W}",
        a + d + c,
    );
}

/// The import itself must put the data on the Dashboard: the shell
/// refreshes when an import finishes, with no filter change from the user.
/// A missing refresh leaves the Dashboard on its "no data" state while the
/// sidebar already counts the imported rows (task 3.10). The range is
/// pinned to "All time" through the persisted setting — the fixture dates
/// are fixed 2026 dates, so the test passes whenever it runs.
#[gpui::test]
fn the_dashboard_shows_the_imported_totals_without_touching_the_range(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        dash(cx, &shell).read(cx).filter.date_range == DateRange::Preset(DatePreset::AllTime)
    });

    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    // Opening the file switched to the Imports page; go back, where the
    // hidden page reloads (3.82).
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));

    // Nothing else touches the shell: these numbers can only come from the
    // load the page switch runs (3.82), fetching the finished import.
    wait_until(cx, "dashboard shows the import", |cx| {
        dash(cx, &shell).read(cx).row_count == 100 && shell.read(cx).total_visible == 102
    });

    // Spend per month from expected.json: the two money-in rows are the
    // card payments, auto-categorized as transfers, so they leave the
    // default view entirely instead of offsetting the spend.
    let expected = expected_totals();
    let want: Vec<i64> = ["2026-09", "2026-10", "2026-11"]
        .iter()
        .map(|m| {
            expected["amex-sep-nov.csv"]["months"][m]["money_out_cents"]
                .as_i64()
                .unwrap()
        })
        .collect();
    let total: i64 = want.iter().sum();
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.summary.months, ["2026-09", "2026-10", "2026-11"]);
        let row = |name: &str| {
            d.summary
                .rows
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("no summary row {name:?}"))
        };
        assert_eq!(row("Spend").months, want);
        assert_eq!(row("Spend").total, total);
        assert_eq!(row("Income").total, 0);
        assert_eq!(row("Net").total, -total);
        assert_eq!(
            d.chart_rows
                .iter()
                .map(|r| (r.name.as_str(), r.spend_cents))
                .collect::<Vec<_>>(),
            [("Uncategorized", total)],
            "the chart shows the imported spend with no further interaction"
        );
    });
}

/// The blank-dashboard bug (3.10): a date range that selects none of the
/// imported rows must show the range/filter empty states — never the
/// whole-window "Drop a CSV" prompt, which belongs to an empty database.
#[gpui::test]
fn a_range_that_excludes_everything_shows_no_matches_not_the_import_prompt(
    cx: &mut TestAppContext,
) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "custom:2020-01-01..2020-12-31").unwrap();
    });
    wait_until(cx, "custom range restored", |cx| {
        matches!(
            dash(cx, &shell).read(cx).filter.date_range,
            DateRange::Custom { .. }
        )
    });

    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    // Opening the file switched to the Imports page; go back, where the
    // hidden page reloads (3.82).
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));

    // The 2020 range keeps every 2026 row out of the table, while the
    // unfiltered count still sees them.
    wait_until(cx, "table empty over a full database", |cx| {
        shell.read(cx).total_visible == 102 && dash(cx, &shell).read(cx).row_count == 0
    });

    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(
            d.dashboard_body(),
            DashboardBody::NoMatches,
            "data exists but the range selects none: the table's empty \
             state shows, not the import prompt"
        );
        assert!(d.chart_rows.is_empty());
        assert!(
            !d.summary_has_data(),
            "the padded all-zero grid must read as empty"
        );
    });
}

/// A fresh database has nothing to show, so the dashboard is the
/// whole-window import prompt (ui-conventions, *States*).
#[gpui::test]
fn an_empty_database_shows_the_import_prompt(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |_c| {});
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            dash(cx, &shell).read(cx).dashboard_body(),
            DashboardBody::NoData
        );
    });
}

/// "Clear filters" drops the table-narrowing filters — search, the toggles
/// and the category/month selection — and the rows come back (3.10).
#[gpui::test]
fn clear_filters_restores_a_table_emptied_by_search(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        dash(cx, &shell).read(cx).filter.date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    // Opening the file switched to the Imports page; go back, where the
    // hidden page reloads (3.82).
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "import shows", |cx| {
        dash(cx, &shell).read(cx).row_count == 100
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.search_input
                .update(cx, |i, cx| i.set_value("zzz-no-match", window, cx));
            d.apply_search("zzz-no-match", window, cx);
        });
    });
    wait_until(cx, "search empties the table", |cx| {
        dash(cx, &shell).read(cx).row_count == 0 && shell.read(cx).total_visible == 102
    });
    cx.update(|cx| {
        assert_eq!(
            dash(cx, &shell).read(cx).dashboard_body(),
            DashboardBody::NoMatches
        )
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.clear_filters(window, cx));
    });
    wait_until(cx, "clear filters restores the rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 100
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.dashboard_body(), DashboardBody::Rows);
        assert!(d.filter.search.is_empty());
        assert!(
            d.search_input.read(cx).value().is_empty(),
            "the search box clears with the filter"
        );
    });
}

/// 3.35: the dashboard's filter region is one row — no "Filters" title, no
/// label lines (this replaces 3.16's visible-label rule for the dashboard
/// filters). The row never grows past one 32 px control line, with the
/// custom range closed or open.
#[gpui::test]
fn the_dashboard_filter_region_is_one_untitled_row(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    let region = window
        .debug_bounds("filters:region")
        .expect("the filter region paints");
    assert!(
        f32::from(region.size.height) <= 40.,
        "the filter region is {} tall, not a single row",
        f32::from(region.size.height)
    );
    // The region title and every label line are gone.
    for sel in [
        "label:Filters",
        "label:Date range",
        "label:Accounts",
        "label:Search",
    ] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "{sel:?} must not paint any more (3.35 removed the region title \
             and the label lines)"
        );
    }
    // Custom range: the pickers join the row and carry no labels either.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::Custom, window, cx)
        });
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    for sel in ["label:From", "label:To"] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "{sel:?} must not paint (3.35: the pickers name themselves in \
             tooltips, not labels)"
        );
    }
    let region = window
        .debug_bounds("filters:region")
        .expect("the filter region paints with the custom range open");
    assert!(
        f32::from(region.size.height) <= 40.,
        "with the custom range the filter region is {} tall, not a single row",
        f32::from(region.size.height)
    );
}

/// 3.35: every dashboard filter control that lost its visible label names
/// itself in a hover tooltip — Date range, Accounts, Options.
#[gpui::test]
fn the_unlabelled_filter_controls_show_their_names_on_hover(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for (sel, tip) in [
        ("filters:date-range", "filters:tooltip:Date range"),
        ("accounts:filter", "filters:tooltip:Accounts"),
        ("filters:options", "filters:tooltip:Options"),
    ] {
        let b = window
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
        window.simulate_mouse_move(b.center(), None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        draw_frame(cx, handle);
        assert!(
            paints(cx, handle, tip),
            "hovering {sel} shows the tooltip {tip:?}"
        );
        // Move off so no stale tooltip lingers into the next hover.
        window.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::default());
        cx.run_until_parked();
        draw_frame(cx, handle);
    }
}

/// 3.16: the chart and the monthly summary panels name themselves, as the
/// ui-conventions mockups show (with data, so both draw their bodies).
#[gpui::test]
fn the_dashboard_panels_name_themselves(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        dash(cx, &shell).read(cx).filter.date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    // Opening the file switched to the Imports page; go back, where the
    // hidden page reloads (3.82).
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "dashboard shows the import", |cx| {
        dash(cx, &shell).read(cx).row_count == 100 && shell.read(cx).total_visible == 102
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for sel in ["label:Spending by category", "label:Monthly summary"] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the dashboard panel shows the title {sel:?}"
        );
    }
}

/// 3.25: the dashboard with the demo import, measured at `w` × `h`.
fn open_dashboard_for_geometry(
    cx: &mut TestAppContext,
    w: f32,
    h: f32,
) -> (AnyWindowHandle, Entity<Shell>) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        dash(cx, &shell).read(cx).filter.date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    // Opening the file switched to the Imports page; go back, where the
    // hidden page reloads (3.82).
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "dashboard shows the import", |cx| {
        dash(cx, &shell).read(cx).row_count == 100 && shell.read(cx).total_visible == 102
    });
    resize_and_draw(cx, handle, w, h);
    (handle, shell)
}

/// 3.25, 3.35 (dashboard alignment): every control in the single filter
/// row — dropdowns, the search box, Options, the transaction count — sits
/// on one centre line, vertically centred in the 32 px row. The row stays
/// one control line tall (≤ 40 px). At both window sizes.
#[gpui::test]
fn the_dashboard_filter_row_controls_share_one_centre_line(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, _shell) = open_dashboard_for_geometry(cx, w, h);
        let mut window = VisualTestContext::from_window(handle, cx);
        let centre = centre_y(cx, handle, "filters:date-range");
        for sel in [
            "accounts:filter",
            "filters:search",
            "filters:options",
            "filters:count",
        ] {
            let cy = centre_y(cx, handle, sel);
            assert!(
                (cy - centre).abs() <= 1.,
                "{sel} centres at y {cy}, not on the filter row's centre line {centre} ({w}x{h})"
            );
        }
        let region = window
            .debug_bounds("filters:region")
            .expect("the filter region paints");
        assert!(
            f32::from(region.size.height) <= 40.,
            "the filter region is {} tall, not one 32 px row ({w}x{h})",
            f32::from(region.size.height)
        );
    }
}

/// 3.25 (dashboard alignment): the filter region, the chart panel, the
/// monthly summary and the table share the page's left and right edges
/// exactly; the two middle panels are the same height and their titles sit
/// on one line. At both window sizes.
#[gpui::test]
fn the_dashboard_regions_share_the_page_edges(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, _shell) = open_dashboard_for_geometry(cx, w, h);
        let mut window = VisualTestContext::from_window(handle, cx);
        let left = SIDEBAR_W + PAGE_PAD;
        let right = w - PAGE_PAD;
        for sel in ["filters:region", "panel:chart", "panel:table"] {
            let b = window
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
            assert!(
                (f32::from(b.origin.x) - left).abs() <= 1.,
                "{sel} starts at x {}, not the page's left edge {left} ({w}x{h})",
                f32::from(b.origin.x),
            );
        }
        for sel in ["filters:region", "panel:summary", "panel:table"] {
            let b = window
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
            let r = f32::from(b.origin.x) + f32::from(b.size.width);
            assert!(
                (r - right).abs() <= 1.,
                "{sel} ends at x {r}, not the page's right edge {right} ({w}x{h})",
            );
        }
        let chart = window
            .debug_bounds("panel:chart")
            .expect("the chart panel paints");
        let summary = window
            .debug_bounds("panel:summary")
            .expect("the summary panel paints");
        assert!(
            (f32::from(chart.size.height) - f32::from(summary.size.height)).abs() <= 1.,
            "the chart panel is {} tall, the summary {} ({w}x{h})",
            f32::from(chart.size.height),
            f32::from(summary.size.height),
        );
        let chart_title = window
            .debug_bounds("label:Spending by category")
            .expect("the chart title paints");
        let summary_title = window
            .debug_bounds("label:Monthly summary")
            .expect("the summary title paints");
        assert!(
            (f32::from(chart_title.origin.y) - f32::from(summary_title.origin.y)).abs() <= 1.,
            "the panel titles sit at y {} and y {}, not on one line ({w}x{h})",
            f32::from(chart_title.origin.y),
            f32::from(summary_title.origin.y),
        );
    }
}

/// 3.25, 3.35 (dashboard alignment): opening the custom date range adds the
/// From and To pickers without breaking anything — they join the filter
/// row's centre line, every control and the count still paint inside the
/// page (at 1200 × 720 nothing is clipped and nothing wraps), and the
/// region still spans the page exactly. At both window sizes.
#[gpui::test]
fn the_dashboard_stays_aligned_with_a_custom_date_range(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, shell) = open_dashboard_for_geometry(cx, w, h);
        with_window(cx, handle, |window, cx| {
            dash(cx, &shell).update(cx, |d, cx| {
                d.change_date_preset(DatePreset::Custom, window, cx)
            });
        });
        wait_until(cx, "custom range open", |cx| {
            matches!(
                dash(cx, &shell).read(cx).filter.date_range,
                DateRange::Custom { .. }
            )
        });
        draw_frame(cx, handle);
        let mut window = VisualTestContext::from_window(handle, cx);
        const CONTROLS: [&str; 7] = [
            "filters:date-range",
            "filters:from",
            "filters:to",
            "accounts:filter",
            "filters:search",
            "filters:options",
            "filters:count",
        ];
        let centre = centre_y(cx, handle, "filters:date-range");
        for sel in CONTROLS {
            let cy = centre_y(cx, handle, sel);
            assert!(
                (cy - centre).abs() <= 1.,
                "{sel} centres at y {cy}, not on the filter row's centre line {centre} (custom range, {w}x{h})"
            );
        }
        // Every control and the count paint inside the page: no control
        // hangs off either edge, and the row does not wrap to a second
        // line (its height stays one control).
        let left = SIDEBAR_W + PAGE_PAD;
        let right = w - PAGE_PAD;
        for sel in CONTROLS {
            let b = window
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} paints on the dashboard (custom range, {w}x{h})"));
            assert!(
                f32::from(b.origin.x) >= left - 0.01
                    && f32::from(b.origin.x) + f32::from(b.size.width) <= right + 0.01,
                "{sel} spans x {}–{}, outside the page's {left}–{right} (custom range, {w}x{h})",
                f32::from(b.origin.x),
                f32::from(b.origin.x) + f32::from(b.size.width),
            );
        }
        let b = window
            .debug_bounds("filters:region")
            .expect("the filter region paints");
        assert!(
            f32::from(b.size.height) <= 40.,
            "with the custom range the filter region wraps to {} px, not one row ({w}x{h})",
            f32::from(b.size.height)
        );
        assert!(
            (f32::from(b.origin.x) - left).abs() <= 1.,
            "with the custom range the filter region starts at x {}, not the page's left edge ({w}x{h})",
            f32::from(b.origin.x),
        );
        let r = f32::from(b.origin.x) + f32::from(b.size.width);
        assert!(
            (r - right).abs() <= 1.,
            "with the custom range the filter region ends at x {r}, not the page's right edge {right} ({w}x{h})",
        );
    }
}

/// 3.35: with the filters collapsed to one row, the chart panel starts
/// where the filter rows used to end — at least 80 px higher than the
/// 177.5 px its top measured at 1200 × 720 on the pre-3.35 layout.
#[gpui::test]
fn the_chart_panel_starts_higher_now_the_filters_are_one_row(cx: &mut TestAppContext) {
    const CHART_TOP_BEFORE: f32 = 177.5; // measured at HEAD~ on the 3.25 two-row layout
    let (handle, _shell) = open_dashboard_for_geometry(cx, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let chart = window
        .debug_bounds("panel:chart")
        .expect("the chart panel paints");
    assert!(
        f32::from(chart.origin.y) <= CHART_TOP_BEFORE - 80.,
        "the chart panel's top is at y {}, not at least 80 px above the \
         pre-3.35 y {CHART_TOP_BEFORE}",
        f32::from(chart.origin.y)
    );
}

// -- categories page -------------------------------------------------------
