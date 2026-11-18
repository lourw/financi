//! Dashboard page layout: table width plan, panels, filter row, alignment.

use super::*;

/// 3.25, 3.86: the dashboard table's width plan — the flexed `Account`,
/// `Description` and `Category` columns plus the fixed ones always sum to
/// exactly the *measured* container width, so the table fills its panel
/// without overhanging it at any width (the wrapper alone cannot pin
/// this, and the old fixed 300/360/260 columns overflowed 1200 px).
#[test]
fn dashboard_table_columns_fit_every_page_width() {
    for w in [1200., 1600., 1920., 2560.] {
        let (account, description, category) = tx_flex_widths(w);
        let total = TX_FIXED_W + TABLE_BORDER_X + account + description + category;
        assert!(
            (total - w).abs() <= 0.01,
            "container {w}: columns sum to {total}, width is {w}"
        );
        assert!(
            account >= HISTORY_FLEX_MIN_W
                && description >= HISTORY_FLEX_MIN_W
                && category >= HISTORY_FLEX_MIN_W,
            "container {w}: flexed columns below the floor"
        );
    }
    // The columns grow with the measured width...
    let small = tx_flex_widths(1200.).1;
    let big = tx_flex_widths(2560.).1;
    assert!(big > small, "{big} vs {small}");
    // ...and below any real container the flexed total stops at its floor.
    let (a, d, c) = tx_flex_widths(100.);
    assert!(
        (a + d + c - 3. * HISTORY_FLEX_MIN_W).abs() <= 0.01,
        "floored total {} is not 3 × {HISTORY_FLEX_MIN_W}",
        a + d + c,
    );
}

/// 3.86: the table's column widths follow the measured container. At
/// 1200 × 720 and at 1600 × 900 the stored panel width equals the
/// `panel:table` frame's content width, the columns the table renders at
/// least fill it (below the columns' floor the table scrolls sideways),
/// and widening the window grows the flex columns — no `SIDEBAR_W`
/// arithmetic in the plan.
#[gpui::test]
fn the_dashboard_table_follows_the_measured_panel_at_1200_and_1600(cx: &mut TestAppContext) {
    let mut sums = Vec::new();
    for w in [1200., 1600.] {
        let (handle, shell) = open_dashboard_for_geometry(cx, w, 900.);
        // 3.86: `open_dashboard_for_geometry` draws once; settle so the
        // `panel:table` probe's report has landed and been applied.
        settle(cx, handle);
        let painted = {
            let mut window = VisualTestContext::from_window(handle, cx);
            f32::from(
                window
                    .debug_bounds("panel:table")
                    .expect("the table panel paints")
                    .size
                    .width,
            )
        };
        let (stored, widths) = cx.update(|cx| {
            let page = dash(cx, &shell).read(cx);
            let table = page.table.read(cx);
            let widths: Vec<f32> = (0..5)
                .map(|i| table.col_width(i).unwrap().as_f32())
                .collect();
            (page.tx_panel_w, widths)
        });
        // 3.100: `panel:table` is the dock frame; the probe measures the
        // content inside the 6 px ring and the 1 px border — 14 px in all.
        let content = painted - 14.;
        assert!(
            (stored - content).abs() <= 0.5,
            "at {w}px the probe stored {stored:.1}px but `panel:table`'s \
             content is {content:.1}px (frame {painted:.1}px)",
        );
        assert!(
            widths.iter().sum::<f32>() + TABLE_BORDER_X >= content - 0.5,
            "at {w}px the columns plus the table's 1 px borders sum to {} \
             but the panel's content measures {content:.1}px",
            widths.iter().sum::<f32>() + TABLE_BORDER_X,
        );
        sums.push(widths[1]);
    }
    assert!(
        sums[1] > sums[0],
        "the flex columns grow with the measured panel: {:?} vs {:?}",
        sums[1],
        sums[0]
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
        dash(cx, &shell).update(cx, |d, cx| d.clear_all_filters(window, cx));
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
pub(super) fn open_dashboard_for_geometry(
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
            "filters:reset-layout",
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

/// 3.25, 3.45, 3.100 (dashboard layout): the filter region spans the page's
/// edges, and so do the dock's painted panel boxes: the left column's
/// titles start at the page's left edge, the right column's end at its
/// right edge (the 6 px rings cross the page padding — that is what the
/// 12 px gutters are made of). The left column reads top-to-bottom: the
/// summary's title sits below the chart's, in one aligned column. At both
/// window sizes.
#[gpui::test]
fn the_dashboard_regions_share_the_page_edges(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, _shell) = open_dashboard_for_geometry(cx, w, h);
        let mut window = VisualTestContext::from_window(handle, cx);
        let left = SIDEBAR_W + PAGE_PAD;
        let right = w - PAGE_PAD;
        for sel in [
            "filters:region",
            "dashboard:title:chart",
            "dashboard:title:summary",
        ] {
            let b = window
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
            assert!(
                (f32::from(b.origin.x) - left).abs() <= 1.,
                "{sel} starts at x {}, not the page's left edge {left} ({w}x{h})",
                f32::from(b.origin.x),
            );
        }
        for sel in [
            "filters:region",
            "dashboard:title:stacked",
            "dashboard:title:table",
        ] {
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
        // 3.45: stacked rows — the summary panel begins below the chart
        // panel, it no longer shares the chart's row beside it.
        assert!(
            f32::from(summary.origin.y) >= f32::from(chart.origin.y + chart.size.height) - 1.,
            "the summary panel top is at y {}, not below the chart panel's \
             bottom {} ({w}x{h})",
            f32::from(summary.origin.y),
            f32::from(chart.origin.y + chart.size.height),
        );
        // The 3.25 title alignment, restacked: the two panel titles share
        // the page's left inner edge — one reading column — and the
        // summary's title reads below the chart's.
        let chart_title = window
            .debug_bounds("label:Spending by category")
            .expect("the chart title paints");
        let summary_title = window
            .debug_bounds("label:Monthly summary")
            .expect("the summary title paints");
        assert!(
            (f32::from(chart_title.origin.x) - f32::from(summary_title.origin.x)).abs() <= 1.,
            "the panel titles start at x {} and x {}, not one aligned \
             column ({w}x{h})",
            f32::from(chart_title.origin.x),
            f32::from(summary_title.origin.x),
        );
        assert!(
            f32::from(summary_title.origin.y) > f32::from(chart_title.origin.y),
            "the summary title (y {}) does not read below the chart title \
             (y {}) ({w}x{h})",
            f32::from(summary_title.origin.y),
            f32::from(chart_title.origin.y),
        );
    }
}

/// 3.25, 3.35 (dashboard alignment), 3.46: opening the custom date range
/// adds the From and To pickers without breaking anything — they join the
/// filter row's centre line, every control and the count still paint inside
/// the page (at 1200 × 720 nothing is clipped and nothing wraps), and the
/// region still spans the page exactly. With a filter set the Clear all
/// button joins the row and fits too. At both window sizes.
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
        // A search makes the row's filter non-default, so Clear all shows.
        with_window(cx, handle, |window, cx| {
            dash(cx, &shell).update(cx, |d, cx| {
                d.search_input
                    .update(cx, |i, cx| i.set_value("cafe", window, cx));
                d.apply_search("cafe", window, cx);
            });
        });
        wait_until(cx, "clear all shows", |cx| {
            dash(cx, &shell).read(cx).filter.search == "cafe"
        });
        draw_frame(cx, handle);
        let mut window = VisualTestContext::from_window(handle, cx);
        const CONTROLS: [&str; 9] = [
            "filters:date-range",
            "filters:from",
            "filters:to",
            "accounts:filter",
            "filters:search",
            "filters:options",
            "filters:clear-all",
            "filters:count",
            "filters:reset-layout",
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

/// 3.100: the dock replaced the page scroller — the page itself never
/// scrolls. Wheeling over panels that fit their content moves nothing:
/// the filter row stays pinned and the panels stay in their quadrants.
/// (Panel-internal scrolling is the table's own, tested in *table*;
/// the summary's own scroller is tested in *dock*.)
#[gpui::test]
fn the_page_does_not_scroll_and_the_panels_stay_put(cx: &mut TestAppContext) {
    let (handle, shell) = open_dashboard_for_geometry(cx, 1200., 720.);
    settle(cx, handle);
    let chart_top = f32::from(bounds(cx, handle, "panel:chart").origin.y);
    let filters_top = f32::from(bounds(cx, handle, "filters:region").origin.y);
    for sel in ["panel:chart", "panel:stacked", "panel:summary"] {
        let at = viewport_centre(bounds(cx, handle, sel));
        wheel(cx, handle, at, 0., -120.);
        draw_frame(cx, handle);
    }
    assert_eq!(
        f32::from(bounds(cx, handle, "filters:region").origin.y),
        filters_top,
        "the filter row stays pinned while the wheel turns over the panels"
    );
    assert_eq!(
        f32::from(bounds(cx, handle, "panel:chart").origin.y),
        chart_top,
        "a panel rode the wheel — the page must not scroll (3.100)"
    );
    let off = cx.update(|cx| dash(cx, &shell).read(cx).summary_rows_scroll.offset().y);
    assert_eq!(
        off,
        px(0.),
        "the summary fits its panel, so its own scroller stays at the top"
    );
}

/// 3.100: the default arrangement is the 2×2 — chart | monthly bars
/// above summary | transactions — with 12 px gutters between
/// neighbours (measured between the painted panel boxes, at both
/// window sizes) and every panel at or above its minimum, inside
/// 1200 × 720 and 1600 × 1000.
#[gpui::test]
fn the_default_layout_is_a_two_by_two_with_gutters(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, _shell) = open_dashboard_for_geometry(cx, w, h);
        settle(cx, handle);
        let chart = bounds(cx, handle, "panel:chart");
        let stacked = bounds(cx, handle, "panel:stacked");
        let summary = bounds(cx, handle, "panel:summary");
        let table = bounds(cx, handle, "panel:table");
        let top = |b: gpui::Bounds<Pixels>| f32::from(b.origin.y);
        assert!(
            (top(chart) - top(stacked)).abs() <= 1.,
            "at {w}×{h}: chart and stacked are not on one row ({} vs {})",
            top(chart),
            top(stacked),
        );
        assert!(
            (top(summary) - top(table)).abs() <= 1.,
            "at {w}×{h}: summary and table are not on one row ({} vs {})",
            top(summary),
            top(table),
        );
        assert!(
            f32::from(chart.origin.x) < f32::from(stacked.origin.x),
            "at {w}×{h}: the chart is not the top-left panel"
        );
        assert!(
            f32::from(summary.origin.x) < f32::from(table.origin.x),
            "at {w}×{h}: the summary is not the bottom-left panel"
        );
        assert!(
            top(summary) > top(chart),
            "at {w}×{h}: the bottom row is not below the top row"
        );
        // The frames tile the dock (the divider straddles the seam), so
        // the gutter is measured between the painted boxes: the chart's
        // title row's right edge and the stacked panel's title row's
        // left edge.
        let chart_box = bounds(cx, handle, "dashboard:title:chart");
        let stacked_box = bounds(cx, handle, "dashboard:title:stacked");
        let gutter = f32::from(stacked_box.origin.x)
            - (f32::from(chart_box.origin.x) + f32::from(chart_box.size.width));
        assert!(
            (gutter - 12.).abs() <= 1.,
            "at {w}×{h}: the gutter between the top-row panels is \
             {gutter:.1} px, not 12 px"
        );
        // The vertical gutter, between the Chart panel's painted bottom
        // edge (its frame's bottom less the 6 px ring) and the Summary
        // panel's painted top edge (the top of its title row).
        let summary_title = bounds(cx, handle, "dashboard:title:summary");
        let ring = f32::from(crate::ui::dashboard::dock::DOCK_GUTTER);
        let v_gutter =
            f32::from(summary_title.origin.y) - (top(chart) + f32::from(chart.size.height) - ring);
        assert!(
            (v_gutter - 12.).abs() <= 1.,
            "at {w}×{h}: the gutter between the rows is {v_gutter:.1} px, \
             not 12 px (chart {chart:?}, summary title {summary_title:?})"
        );
        // Every panel clears its minimum (240×160; the table 320×200) and
        // the dock reaches the window's bottom-right corner (page padding
        // 16 px, minus the 6 px the dock pulls back for its outer ring).
        for (name, b, min_w, min_h) in [
            ("chart", chart, 240., 160.),
            ("stacked", stacked, 240., 160.),
            ("summary", summary, 240., 160.),
            ("table", table, 320., 200.),
        ] {
            let (pw, ph) = (f32::from(b.size.width), f32::from(b.size.height));
            assert!(
                pw >= min_w && ph >= min_h,
                "at {w}×{h}: {name} painted {pw:.1}×{ph:.1}, below its \
                 {min_w}×{min_h} minimum"
            );
        }
        let bottom = top(table) + f32::from(table.size.height);
        let right = f32::from(table.origin.x) + f32::from(table.size.width);
        assert!(
            (bottom - (h - 10.)).abs() <= 2.,
            "at {w}×{h}: the dock's bottom edge is at {bottom:.1}, not \
             {} ({h:.0} − 16 padding + 6 ring)",
            h - 10.,
        );
        assert!(
            (right - (w - 10.)).abs() <= 2.,
            "at {w}×{h}: the dock's right edge is at {right:.1}, not \
             {} ({w:.0} − 16 padding + 6 ring)",
            w - 10.,
        );
    }
}

/// 3.100: the wheel over the transactions table scrolls the table
/// inside its own panel — the other panels do not move and the page
/// does not scroll.
#[gpui::test]
fn wheel_over_the_table_scrolls_the_table_only(cx: &mut TestAppContext) {
    let (handle, shell) = open_dashboard_for_geometry(cx, 1200., 720.);
    settle(cx, handle);
    let chart_top = f32::from(bounds(cx, handle, "panel:chart").origin.y);
    let summary_top = f32::from(bounds(cx, handle, "panel:summary").origin.y);
    let at = viewport_centre(bounds(cx, handle, "panel:table"));
    wheel(cx, handle, at, 0., -120.);
    draw_frame(cx, handle);
    assert_eq!(
        f32::from(bounds(cx, handle, "panel:chart").origin.y),
        chart_top,
        "the chart moved while the table scrolled"
    );
    assert_eq!(
        f32::from(bounds(cx, handle, "panel:summary").origin.y),
        summary_top,
        "the summary moved while the table scrolled"
    );
    let off = cx.update(|cx| dash(cx, &shell).read(cx).summary_rows_scroll.offset().y);
    assert_eq!(off, px(0.), "the wheel over the table reached the summary");
}

/// 3.100: entering the dashboard resets the summary panel's own
/// scroller — 3.45's page position is gone, and a page entry starts
/// every panel at its top.
#[gpui::test]
fn entering_the_dashboard_starts_at_the_top_of_the_panels(cx: &mut TestAppContext) {
    let (handle, shell) = open_dashboard_for_geometry(cx, 1200., 720.);
    settle(cx, handle);
    cx.update(|cx| {
        dash(cx, &shell).update(cx, |d, _cx| {
            d.summary_rows_scroll.set_offset(point(px(0.), px(-80.)));
        })
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_idle(cx, &shell);
    settle(cx, handle);
    let off = cx.update(|cx| dash(cx, &shell).read(cx).summary_rows_scroll.offset().y);
    assert_eq!(off, px(0.), "the revisit starts the summary at the top");
    assert!(
        f32::from(bounds(cx, handle, "panel:table").origin.y) > 0.,
        "the dock is laid out below the filter row"
    );
}

// -- categories page -------------------------------------------------------
