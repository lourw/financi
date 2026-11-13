//! Tests for `ui/dashboard.rs` (3.62 split from `ui_tests.rs`).

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
        shell.read(cx).dashboard.date_range == DateRange::Preset(DatePreset::AllTime)
    });

    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");

    // Nothing else touches the shell: these numbers can only come from the
    // refresh the finished import triggers.
    wait_until(cx, "dashboard shows the import", |cx| {
        shell.read(cx).dashboard.row_count == 100 && shell.read(cx).total_visible == 102
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
        let s = shell.read(cx);
        assert_eq!(
            s.dashboard.summary.months,
            ["2026-09", "2026-10", "2026-11"]
        );
        let row = |name: &str| {
            s.dashboard
                .summary
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
            s.dashboard
                .chart_rows
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
            shell.read(cx).dashboard.date_range,
            DateRange::Custom { .. }
        )
    });

    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");

    // The 2020 range keeps every 2026 row out of the table, while the
    // unfiltered count still sees them.
    wait_until(cx, "table empty over a full database", |cx| {
        shell.read(cx).total_visible == 102 && shell.read(cx).dashboard.row_count == 0
    });

    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.dashboard_body(),
            DashboardBody::NoMatches,
            "data exists but the range selects none: the table's empty \
             state shows, not the import prompt"
        );
        assert!(s.dashboard.chart_rows.is_empty());
        assert!(
            !s.summary_has_data(),
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
        assert_eq!(shell.read(cx).dashboard_body(), DashboardBody::NoData);
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
        shell.read(cx).dashboard.date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(cx, "import shows", |cx| {
        shell.read(cx).dashboard.row_count == 100
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.dashboard
                .search_input
                .update(cx, |i, cx| i.set_value("zzz-no-match", window, cx));
            s.apply_search("zzz-no-match", window, cx);
        });
    });
    wait_until(cx, "search empties the table", |cx| {
        shell.read(cx).dashboard.row_count == 0 && shell.read(cx).total_visible == 102
    });
    cx.update(|cx| assert_eq!(shell.read(cx).dashboard_body(), DashboardBody::NoMatches));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.clear_filters(window, cx));
    });
    wait_until(cx, "clear filters restores the rows", |cx| {
        shell.read(cx).dashboard.row_count == 100
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.dashboard_body(), DashboardBody::Rows);
        assert!(s.dashboard.search.is_empty());
        assert!(
            s.dashboard.search_input.read(cx).value().is_empty(),
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
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::Custom, window, cx)
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
        shell.read(cx).dashboard.date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(cx, "dashboard shows the import", |cx| {
        shell.read(cx).dashboard.row_count == 100 && shell.read(cx).total_visible == 102
    });
    // Opening the file switched to the Imports page; go back.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
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
        shell.read(cx).dashboard.date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(cx, "dashboard shows the import", |cx| {
        shell.read(cx).dashboard.row_count == 100 && shell.read(cx).total_visible == 102
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
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
            shell.update(cx, |s, cx| {
                s.change_date_preset(DatePreset::Custom, window, cx)
            });
        });
        wait_until(cx, "custom range open", |cx| {
            matches!(
                shell.read(cx).dashboard.date_range,
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

/// A minimal row for the pure menu-model test.
fn browse_row(hidden: bool, paired: bool) -> TransactionRow {
    TransactionRow {
        id: 1,
        account: "American Express · Cobalt".into(),
        posted_date: "2026-10-31".into(),
        description: "COFFEE    VANCOUVER".into(),
        merchant: Some("COFFEE".into()),
        category: None,
        category_id: None,
        amount_cents: -450,
        hidden,
        transfer_pair_id: paired.then_some(2),
        category_locked: false,
    }
}

#[test]
fn menu_shows_hide_for_a_visible_row_and_unhide_only_for_a_hidden_one() {
    assert_eq!(
        row_menu_items(&browse_row(false, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Hide],
        "a visible row offers Hide, never Unhide"
    );
    assert_eq!(
        row_menu_items(&browse_row(true, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Unhide],
        "a hidden row offers Unhide, never Hide"
    );
    assert_eq!(
        row_menu_items(&browse_row(false, true)),
        [
            RowMenuItem::ChangeCategory,
            RowMenuItem::Hide,
            RowMenuItem::Unpair
        ],
        "a paired row also offers Unpair"
    );
    assert_eq!(
        row_menu_items(&browse_row(true, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Unhide],
        "an unpaired row never offers Unpair"
    );
}

/// 3.29: the *Show rule* item finds the rule that decided the row, or says
/// why it cannot: a hand-set (locked) row, the built-in card payment, or no
/// rule matching the description. The reason rides in the item's label.
#[test]
fn show_rule_finds_the_deciding_rule_or_gives_the_disabled_reason() {
    let coffee = Rule {
        id: 7,
        pattern: "COFFEE".into(),
        is_regex: false,
        category_id: 3,
    };
    let rules = [coffee];
    // An unlocked row that a rule matches: the item works, plain label.
    let row = browse_row(false, false);
    assert_eq!(row_show_rule(&row, &rules, None), RowShowRule::Found(7));
    assert_eq!(row_show_rule(&row, &rules, None).menu_label(), "Show rule");
    // A locked row was set by hand, whatever the rules would say.
    let mut locked = browse_row(false, false);
    locked.category_locked = true;
    locked.category_id = Some(9);
    assert_eq!(row_show_rule(&locked, &rules, None), RowShowRule::SetByHand);
    assert_eq!(
        row_show_rule(&locked, &rules, Some(4)),
        RowShowRule::SetByHand
    );
    // Locked to the built-in card-payment category: its own reason, even
    // though a rule would have matched (card payments beat the rules).
    assert_eq!(
        row_show_rule(&locked, &rules, Some(9)),
        RowShowRule::CardPayment
    );
    // A locked row with no category at all still reads as hand-set.
    let mut locked_none = browse_row(false, false);
    locked_none.category_locked = true;
    assert_eq!(
        row_show_rule(&locked_none, &rules, Some(9)),
        RowShowRule::SetByHand
    );
    // Nothing matches: there is no rule to show.
    let mut unmatched = browse_row(false, false);
    unmatched.description = "PARKING FEE METRO".into();
    assert_eq!(row_show_rule(&unmatched, &rules, None), RowShowRule::NoRule);
    // The disabled labels say why, in the menu.
    assert_eq!(
        RowShowRule::SetByHand.menu_label(),
        "Show rule — Set by hand"
    );
    assert_eq!(
        RowShowRule::CardPayment.menu_label(),
        "Show rule — Card payment (built in)"
    );
    assert_eq!(
        RowShowRule::NoRule.menu_label(),
        "Show rule — No rule matches"
    );
}

#[gpui::test]
fn changing_a_rows_category_locks_it_and_offers_the_rule(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (m, target, group) = import_and_top_merchant(cx, handle, &shell);
    assert!(group.len() >= 2, "the merchant has several rows");
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    add_parent(cx, handle, &shell, "Home");
    add_child(cx, handle, &shell, "Groceries");
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_row_category(target, online, window, cx)
        });
    });
    wait_until(cx, "target categorized and offer up", |cx| {
        shell.read(cx).dashboard.rule_offer == Some((target, online))
            && table_rows(&shell, cx)
                .iter()
                .any(|r| r.id == target && r.category.as_deref() == Some("Online"))
    });

    // A later rule for the same merchant moves the siblings but never the
    // manually-set (locked) row.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_pattern.update(cx, |i, cx| {
                i.set_value(m.clone(), window, cx);
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == groceries)
                .unwrap();
            s.rule_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.add_rule(window, cx);
        });
    });
    wait_until(cx, "siblings moved, locked target held", |cx| {
        let rows = table_rows(&shell, cx);
        let cat = |id: i64| {
            rows.iter()
                .find(|r| r.id == id)
                .and_then(|r| r.category.clone())
        };
        cat(target).as_deref() == Some("Online")
            && group
                .iter()
                .filter(|id| **id != target)
                .any(|id| cat(*id).as_deref() == Some("Groceries"))
    });
}

#[gpui::test]
fn applying_the_row_offer_categorizes_the_whole_merchant(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (m, target, group) = import_and_top_merchant(cx, handle, &shell);
    assert!(group.len() >= 2, "the merchant has several rows");
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let all_uncategorized = cx.update(|cx| {
        table_rows(&shell, cx)
            .iter()
            .all(|r| !group.contains(&r.id) || r.category.is_none())
    });
    assert!(all_uncategorized, "the group starts uncategorized");

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_row_category(target, online, window, cx)
        });
    });
    wait_until(cx, "offer up", |cx| {
        shell.read(cx).dashboard.rule_offer == Some((target, online))
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_row_rule(target, window, cx));
    });
    wait_until(cx, "rule created and offer cleared", |cx| {
        shell.read(cx).dashboard.rule_offer.is_none()
            && shell.read(cx).rules.iter().any(|r| r.pattern == m)
    });
    let rows = cx.update(|cx| table_rows(&shell, cx));
    for id in &group {
        assert_eq!(
            rows.iter()
                .find(|r| r.id == *id)
                .and_then(|r| r.category.as_deref()),
            Some("Online"),
            "every row of {m} is now Online"
        );
    }
}

#[gpui::test]
fn hiding_and_unhiding_a_row_updates_the_table_and_sidebar_counts(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (_merchant, target, _group) = import_and_top_merchant(cx, handle, &shell);
    let (before_total, before_uncategorized) = cx.update(|cx| {
        (
            shell.read(cx).total_visible,
            shell.read(cx).uncategorized_visible,
        )
    });
    let target_visible = cx.update(|cx| table_rows(&shell, cx).iter().any(|r| r.id == target));
    assert!(target_visible, "the target row is visible before hiding it");

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_row_hidden(target, true, window, cx));
    });
    wait_until(cx, "hidden row leaves table and counts", |cx| {
        !table_rows(&shell, cx).iter().any(|r| r.id == target)
            && shell.read(cx).total_visible == before_total - 1
            && shell.read(cx).uncategorized_visible == before_uncategorized - 1
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_row_hidden(target, false, window, cx));
    });
    wait_until(cx, "unhidden row returns to table and counts", |cx| {
        table_rows(&shell, cx).iter().any(|r| r.id == target)
            && shell.read(cx).total_visible == before_total
            && shell.read(cx).uncategorized_visible == before_uncategorized
    });
}

// ---------------------------------------------------------------------------
// Date range control (task 3.2)

#[gpui::test]
fn shell_starts_on_the_default_last_three_months(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |_| {});
    // Before the restore lands `view_filter` is still the open default;
    // narrower bounds prove the saved setting (here: none) was applied.
    wait_until(cx, "default date range restored", |cx| {
        shell.read(cx).dashboard.view_filter.from > ViewFilter::default().from
    });
    let (from, to) = preset_range(DatePreset::Last3Months, Shell::today());
    let state = cx.update(|cx| {
        let s = shell.read(cx);
        (
            s.dashboard.date_range,
            s.dashboard.view_filter.from,
            s.dashboard.view_filter.to,
        )
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
        shell.read(cx).dashboard.view_filter.from > ViewFilter::default().from
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::LastMonth, window, cx)
        });
    });

    let (from, to) = preset_range(DatePreset::LastMonth, Shell::today());
    let state = cx.update(|cx| {
        let s = shell.read(cx);
        (
            s.dashboard.date_range,
            s.dashboard.view_filter.from,
            s.dashboard.view_filter.to,
        )
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
/// `Shell::today()`, so the test passes whenever it runs.
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
    let today = Shell::today();
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
    let s = shell.read(cx);
    let cells = |name: &str| {
        s.summary_rows()
            .into_iter()
            .find(|r| r.name == name)
            .expect("a total row")
            .cells
    };
    let food = s
        .dashboard
        .chart_rows
        .iter()
        .find(|r| r.name == "Food")
        .expect("Food legend row")
        .spend_cents;
    (
        s.dashboard.summary.months.clone(),
        cells("Income"),
        cells("Spend"),
        cells("Net"),
        food,
    )
}

#[gpui::test]
fn changing_the_preset_updates_the_dashboard_totals(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_preset_totals_rows);
    let today = Shell::today();
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
        shell.read(cx).dashboard.row_count == 2 && shell.read(cx).dashboard.summary.months == last3
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
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });

    // "All time" pulls in the old pair: the grid clamps to the data span
    // and every number grows.
    wait_until(cx, "all-time totals", |cx| {
        shell.read(cx).dashboard.row_count == 4
            && shell.read(cx).dashboard.summary.months == all_months
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
        shell.read(cx).dashboard.date_range == DateRange::Custom { from, to }
    });
    let state = cx.update(|cx| {
        let s = shell.read(cx);
        (s.dashboard.view_filter.from, s.dashboard.view_filter.to)
    });
    assert_eq!(state, (from, to));
}

#[gpui::test]
fn a_garbage_setting_falls_back_to_the_default(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "last13").unwrap();
    });
    wait_until(cx, "garbage replaced by default", |cx| {
        shell.read(cx).dashboard.view_filter.from > ViewFilter::default().from
    });
    let range = cx.update(|cx| shell.read(cx).dashboard.date_range);
    assert_eq!(range, DateRange::Preset(DatePreset::Last3Months));
}

#[gpui::test]
fn picking_a_custom_range_persists_and_normalizes(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "date range restored", |cx| {
        shell.read(cx).dashboard.view_filter.from > ViewFilter::default().from
    });

    // Switch to "Custom…", then pick both dates with the pickers.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::Custom, window, cx)
        });
    });
    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.date_picked(false, start, window, cx);
            s.date_picked(true, end, window, cx);
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
        let s = shell.read(cx);
        (
            s.dashboard.date_range,
            s.dashboard.view_filter.from,
            s.dashboard.view_filter.to,
        )
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
        shell.update(cx, |s, cx| s.date_picked(true, early, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("custom:2026-01-15..2026-03-01"),
        "a reversed pick is stored swapped"
    );
    let shown = cx.update(|cx| {
        let s = shell.read(cx);
        (
            s.dashboard.view_filter.from,
            s.dashboard.view_filter.to,
            s.dashboard.date_from_picker.read(cx).date(),
            s.dashboard.date_to_picker.read(cx).date(),
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
        shell.update(cx, |s, cx| s.date_picked(false, late, window, cx));
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

#[gpui::test]
fn toggling_include_transfers_filters_rows_and_persists(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        shell.read(cx).dashboard.row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(!s.dashboard.include_transfers, "default off");
        assert!(!s.dashboard.view_filter.include_transfers);
    });
    // The default is not persisted; the setting only appears on change.
    assert_eq!(db_setting(cx, &shell, UI_INCLUDE_TRANSFERS), None);

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_include_transfers(true, window, cx));
    });
    wait_until(cx, "transfer row appears", |cx| {
        shell.read(cx).dashboard.row_count == 3
            && shell.read(cx).dashboard.view_filter.include_transfers
    });
    // Let the persist task queue its write before reading (the worker is FIFO).
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_INCLUDE_TRANSFERS).as_deref(),
        Some("1"),
        "ui.include_transfers was written"
    );

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_include_transfers(false, window, cx));
    });
    wait_until(cx, "transfer row hidden again", |cx| {
        shell.read(cx).dashboard.row_count == 2
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_INCLUDE_TRANSFERS).as_deref(),
        Some("0")
    );
}

/// 3.35: the Options popover carries the old filter-row toggles — same ids,
/// same state and handlers. A real click opens it, all three checkboxes
/// paint, and Include transfers toggles the rows exactly as before.
#[gpui::test]
fn clicking_the_include_transfers_checkbox_toggles_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        shell.read(cx).dashboard.row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    click(cx, handle, "filters:options");
    draw_frame(cx, handle);
    for sel in [
        "checkbox:include-transfers",
        "checkbox:uncategorized-only",
        "checkbox:show-hidden",
    ] {
        assert!(paints(cx, handle, sel), "the Options popover renders {sel}");
    }
    click(cx, handle, "checkbox:include-transfers");
    wait_until(cx, "click checks the box and shows the transfer", |cx| {
        shell.read(cx).dashboard.include_transfers && shell.read(cx).dashboard.row_count == 3
    });
    click(cx, handle, "checkbox:include-transfers");
    wait_until(cx, "clicking again unchecks", |cx| {
        !shell.read(cx).dashboard.include_transfers && shell.read(cx).dashboard.row_count == 2
    });
}

/// 3.35: the Options trigger counts its toggles — plain "Options" while
/// none are on, "Options · 2" when two are.
#[gpui::test]
fn the_options_button_counts_the_toggles_that_are_on(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        shell.read(cx).dashboard.row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "filters:options-btn:Options"),
        "with no toggle on, the button reads just \"Options\""
    );
    // Flip two toggles through the popover, the way a user does.
    click(cx, handle, "filters:options");
    click(cx, handle, "checkbox:include-transfers");
    click(cx, handle, "checkbox:uncategorized-only");
    wait_until(cx, "both toggles on", |cx| {
        shell.read(cx).dashboard.include_transfers && shell.read(cx).dashboard.uncategorized_only
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "filters:options-btn:Options \u{b7} 2"),
        "with two toggles on, the button reads \"Options \u{b7} 2\""
    );
}

#[gpui::test]
fn a_saved_include_transfers_setting_is_restored_at_launch(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        seed_two_accounts_and_a_transfer(c);
        settings::set(c, UI_INCLUDE_TRANSFERS, "1").unwrap();
    });
    // A fresh shell over the stored "1" re-opens with transfers included,
    // and the table query follows the restored filter.
    wait_until(cx, "saved transfers flag restored", |cx| {
        shell.read(cx).dashboard.include_transfers && shell.read(cx).dashboard.row_count == 3
    });
    cx.update(|cx| assert!(shell.read(cx).dashboard.view_filter.include_transfers));
}

#[gpui::test]
fn unchecking_an_account_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "accounts and rows loaded", |cx| {
        shell.read(cx).accounts.len() == 2 && shell.read(cx).dashboard.row_count == 2
    });
    let (cobalt, gold) = cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.dashboard.excluded_accounts.is_empty(),
            "default: all checked"
        );
        assert!(
            s.dashboard.view_filter.account_ids.is_empty(),
            "empty means all"
        );
        let id = |name: &str| s.accounts.iter().find(|a| a.name == name).unwrap().id;
        (id("Cobalt"), id("Gold"))
    });

    // Open the accounts popover and click the checkboxes themselves.
    click(cx, handle, "accounts:filter");
    click(cx, handle, format!("checkbox:account-{cobalt}"));
    wait_until(cx, "only Gold's row shows", |cx| {
        shell.read(cx).dashboard.row_count == 1
            && table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Gold"))
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.dashboard.view_filter.account_ids, vec![gold]);
    });

    // Unchecking every account shows no rows (empty in `ViewFilter`
    // means all, so the table must not fall back to everything).
    click(cx, handle, format!("checkbox:account-{gold}"));
    wait_until(cx, "nothing selected: empty table", |cx| {
        shell.read(cx).dashboard.row_count == 0
    });

    click(cx, handle, format!("checkbox:account-{cobalt}"));
    wait_until(cx, "Cobalt's row back", |cx| {
        shell.read(cx).dashboard.row_count == 1
            && table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Cobalt"))
    });
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).dashboard.view_filter.account_ids,
            vec![cobalt]
        );
    });
}

/// Task 3.17: `none_selected` must resolve against the accounts loaded *by
/// this* refresh, not the previous refresh's list. An import that adds an
/// account while every existing checkbox is unchecked leaves that new
/// account checked, so every view must show its rows as soon as the import
/// finishes — not stay blank until some other filter changes.
#[gpui::test]
fn an_import_that_adds_an_account_shows_it_with_every_box_unchecked(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "Gold loaded and the all-time range restored", |cx| {
        shell.read(cx).accounts.len() == 1
            && shell.read(cx).dashboard.date_range == DateRange::Preset(DatePreset::AllTime)
    });

    // Uncheck the only account: nothing is selected, so the views are
    // legitimately blank.
    let gold = cx.update(|cx| shell.read(cx).accounts[0].id);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_account_checked(gold, false, window, cx));
    });
    wait_until(cx, "no account checked", |cx| {
        shell.read(cx).dashboard.excluded_accounts == [gold]
            && shell.read(cx).dashboard.row_count == 0
    });

    // Import into a brand-new account: its checkbox is on, so the finished
    // import must fill the views with its rows on its own.
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(
        cx,
        "the imported rows show without a further filter change",
        |cx| shell.read(cx).dashboard.row_count == 100,
    );

    cx.update(|cx| {
        let s = shell.read(cx);
        let amex = s
            .accounts
            .iter()
            .find(|a| a.name == "Amex")
            .expect("the import created Amex");
        assert!(
            !s.dashboard.excluded_accounts.contains(&amex.id),
            "a new account starts checked"
        );
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Amex")),
            "only the new account is selected, so only its rows show"
        );
        assert_eq!(
            s.dashboard.summary.months,
            ["2026-09", "2026-10", "2026-11"]
        );
        assert!(
            !s.dashboard.chart_rows.is_empty(),
            "the chart shows the imported spend"
        );
    });
}

/// Task 3.17 (regression): the refresh triggered when an import finishes
/// starts with the *previous* refresh's `self.accounts` — the database
/// already has the new account, the shell does not. Judging "no accounts
/// checked" from that stale list skipped the queries, so every view stayed
/// blank until some other filter changed; `none_selected` must resolve
/// against the accounts the refresh itself loads.
#[gpui::test]
fn a_refresh_resolves_the_account_filter_against_the_accounts_it_loads(cx: &mut TestAppContext) {
    // Gold's rows were imported long ago; Amex's import is the one that has
    // just landed, after the shell's previous refresh.
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
        let gold = create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
        let amex = create_account(c, "amex", "Amex", AccountKind::CreditCard).unwrap();
        for (account, line, date) in [
            (gold, 0, "2026-11-05"),
            (amex, 0, "2026-10-01"),
            (amex, 1, "2026-10-02"),
        ] {
            c.execute(
                "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                      encoding, has_bom, line_ending, trailing_newline, row_count)
                 VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
                [account],
            )
            .unwrap();
            let import = c.last_insert_rowid();
            c.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, category_id)
                 VALUES (?1, ?2, ?3, X'00', ?4, ?5, -100, 'ROW', NULL)",
                (
                    account,
                    import,
                    line,
                    format!("hash-{account}-{line}"),
                    date,
                ),
            )
            .unwrap();
        }
    });
    wait_until(cx, "both accounts and three rows loaded", |cx| {
        shell.read(cx).accounts.len() == 2 && shell.read(cx).dashboard.row_count == 3
    });
    let gold = cx.update(|cx| {
        shell
            .read(cx)
            .accounts
            .iter()
            .find(|a| a.name == "Gold")
            .unwrap()
            .id
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            // The state the import's refresh starts from: the user has
            // unchecked every account the previous refresh showed — which
            // knew only Gold, not the account the import added.
            s.accounts.retain(|a| a.id == gold);
            s.dashboard.excluded_accounts = vec![gold];
            s.sync_account_filter();
            s.refresh(window, cx);
        });
    });

    // The accounts loaded by this refresh include Amex, whose checkbox is on,
    // so its two rows show: the filter must not be judged from the stale list.
    wait_until(cx, "the checked new account's rows show", |cx| {
        shell.read(cx).dashboard.row_count == 2
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Amex")),
            "only the new account is checked, so only its rows show"
        );
        let amex = s.accounts.iter().find(|a| a.name == "Amex").unwrap().id;
        assert_eq!(s.dashboard.view_filter.account_ids, vec![amex]);
        assert!(!s.dashboard.chart_rows.is_empty(), "the chart is not blank");
        assert!(
            !s.dashboard.summary.months.is_empty(),
            "the summary is not blank"
        );
    });
}

// Dashboard table search and toggles (task 3.4)

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

#[gpui::test]
fn search_filters_the_table_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        shell.read(cx).dashboard.row_count == 3
    });
    // A case-insensitive substring of the description.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_search("cafe", window, cx));
    });
    wait_until(cx, "one cafe row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "CORNER CAFE");
    });
    // A match on the merchant column.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_search("fresh", window, cx));
    });
    wait_until(cx, "merchant match", |cx| {
        shell.read(cx).dashboard.row_count == 1
            && table_rows(&shell, cx)[0].merchant.as_deref() == Some("Fresh Mart")
    });
    // Clearing the box brings every row back.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_search("", window, cx));
    });
    wait_until(cx, "all rows back", |cx| {
        shell.read(cx).dashboard.row_count == 3
    });
}

#[gpui::test]
fn uncategorized_only_toggle_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three rows", |cx| {
        shell.read(cx).dashboard.row_count == 3
    });
    cx.update(|cx| assert!(!shell.read(cx).dashboard.uncategorized_only));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_uncategorized_only(true, window, cx));
    });
    wait_until(cx, "two uncategorized rows", |cx| {
        shell.read(cx).dashboard.row_count == 2
    });
    cx.update(|cx| {
        assert!(shell.read(cx).dashboard.uncategorized_only);
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.category_id.is_none()),
            "only uncategorized rows remain"
        );
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_uncategorized_only(false, window, cx));
    });
    wait_until(cx, "three rows back", |cx| {
        shell.read(cx).dashboard.row_count == 3
    });
}

#[gpui::test]
fn show_hidden_toggle_reveals_hidden_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "hidden row not shown by default", |cx| {
        shell.read(cx).dashboard.row_count == 3
    });
    cx.update(|cx| {
        assert!(!shell.read(cx).dashboard.show_hidden);
        assert!(table_rows(&shell, cx).iter().all(|r| !r.hidden));
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_show_hidden(true, window, cx));
    });
    wait_until(cx, "hidden row appears", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        let hidden = rows
            .iter()
            .find(|r| r.hidden)
            .expect("showing hidden reveals the hidden row");
        assert_eq!(hidden.description, "HIDDEN SPEND");
    });
}

#[gpui::test]
fn sorting_the_amount_header_reorders_rows_without_a_reentrant_panic(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        shell.read(cx).dashboard.row_count == 3
    });

    let descriptions = |cx: &App| -> Vec<String> {
        table_rows(&shell, cx)
            .iter()
            .map(|r| r.description.clone())
            .collect()
    };
    // Newest-first default; every row shares a date, so they fall back to
    // source order.
    assert_eq!(
        cx.read(descriptions),
        ["CORNER CAFE", "GROCER STORE", "PAYROLL DEPOSIT"]
    );

    // Drive the header sort exactly as the table's click listener does:
    // `perform_sort` runs while the table entity is leased and hands the new
    // sort to the shell, which must not update that same entity in response
    // (that double-lease is the crash). The click test-support gpui-kit ships
    // is disabled in this crate, so we enter the sort through the same
    // delegate call the header's `on_click` makes, under the table's lease.
    let table = cx.update(|cx| shell.read(cx).dashboard.table.clone());
    with_window(cx, handle, |window, cx| {
        table.update(cx, |t, cx| {
            t.delegate_mut()
                .perform_sort(4, ColumnSort::Descending, window, cx);
        });
    });

    // The shell adopted the sort and the async re-query reordered by amount
    // descending: +3000, -500, -1200.
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).dashboard.tx_sort,
            TxSort {
                column: TxSortColumn::Amount,
                descending: true,
            }
        );
    });
    wait_until(cx, "rows reordered by amount", |cx| {
        descriptions(cx) == ["PAYROLL DEPOSIT", "CORNER CAFE", "GROCER STORE"]
    });
}

/// Three rows over two accounts with mixed-case descriptions and a
/// categorized / uncategorized mix, so every sortable column (3.44) has its
/// own expected order. Amounts are all distinct.
fn seed_sort_columns_rows(conn: &financi_db::rusqlite::Connection) {
    let cobalt = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let no_fee = create_account(conn, "simplii", "No Fee", AccountKind::Chequing).unwrap();
    let import = |account: i64| {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
            [account],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let cobalt_import = import(cobalt);
    let no_fee_import = import(no_fee);
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    let mut line = 0i64;
    let mut txn = |account: i64,
                   import: i64,
                   date: &str,
                   cents: i64,
                   description: &str,
                   category: Option<i64>| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, 0, X'00', ?3, ?4, ?5, ?6, ?7)",
            (
                account,
                import,
                format!("sort-hash-{line}"),
                date,
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn(
        cobalt,
        cobalt_import,
        "2026-11-01",
        -100,
        "corner cafe",
        Some(groceries),
    );
    txn(no_fee, no_fee_import, "2026-11-02", 300, "Best Buy", None);
    txn(cobalt, cobalt_import, "2026-11-03", -50, "amazon", None);
}

#[gpui::test]
fn every_transactions_table_column_sorts(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_sort_columns_rows);
    wait_until(cx, "three rows", |cx| {
        shell.read(cx).dashboard.row_count == 3
    });

    let descriptions = |cx: &App| -> Vec<String> {
        table_rows(&shell, cx)
            .iter()
            .map(|r| r.description.clone())
            .collect()
    };
    // Newest-first default (3.4): 11-03, 11-02, 11-01.
    assert_eq!(cx.read(descriptions), ["amazon", "Best Buy", "corner cafe"]);

    let table = cx.update(|cx| shell.read(cx).dashboard.table.clone());
    // Drive the header sort as the 3.4 test above does, passing the
    // `ColumnSort` the table's own icon cycle hands the delegate: the off
    // step cycles to Descending (the first click), then Ascending, back to
    // off, …
    let click = |cx: &mut TestAppContext, col_ix: usize, value: ColumnSort| {
        with_window(cx, handle, |window, cx| {
            table.update(cx, |t, cx| {
                t.delegate_mut().perform_sort(col_ix, value, window, cx);
            });
        });
    };
    // Every expected order below differs from the one before it, so
    // waiting for it also waits for the re-query to settle before the
    // next click.
    let assert_sorted = |cx: &mut TestAppContext, want: TxSort, order: [&str; 3], col_ix: usize| {
        wait_until(cx, "rows re-sorted", |cx| descriptions(cx) == order);
        cx.update(|cx| {
            assert_eq!(shell.read(cx).dashboard.tx_sort, want, "{:?}", want.column);
            // The active header shows the direction the rows are in.
            assert_eq!(
                table.read(cx).delegate().column(col_ix, cx).sort,
                Some(if want.descending {
                    ColumnSort::Descending
                } else {
                    ColumnSort::Ascending
                }),
                "header of column {col_ix} shows the direction"
            );
        });
    };

    // Account, first click (icon off → Descending): A→Z on
    // "Institution · Account"; the tied Cobalt rows fall back to date.
    click(cx, 1, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Account,
            descending: false,
        },
        ["amazon", "corner cafe", "Best Buy"],
        1,
    );
    // Second click (Ascending → off step): Z→A.
    click(cx, 1, ColumnSort::Default);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Account,
            descending: true,
        },
        ["Best Buy", "amazon", "corner cafe"],
        1,
    );
    // Description, first click: A→Z, case-insensitively.
    click(cx, 2, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Description,
            descending: false,
        },
        ["amazon", "Best Buy", "corner cafe"],
        2,
    );
    // Category, first click: A→Z with the uncategorized rows sorted as
    // "Uncategorized" — after Groceries, before nothing here, and their
    // tie keeps the newest-first date order.
    click(cx, 3, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Category,
            descending: false,
        },
        ["corner cafe", "amazon", "Best Buy"],
        3,
    );
    // Second click: Z→A, so "Uncategorized" leads.
    click(cx, 3, ColumnSort::Default);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Category,
            descending: true,
        },
        ["amazon", "Best Buy", "corner cafe"],
        3,
    );
    // Date and Amount keep their first directions (3.4): Date's active
    // descending column cycles to Ascending, Amount's off step to
    // Descending (biggest first).
    click(cx, 0, ColumnSort::Ascending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Date,
            descending: false,
        },
        ["corner cafe", "Best Buy", "amazon"],
        0,
    );
    click(cx, 4, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Amount,
            descending: true,
        },
        ["Best Buy", "amazon", "corner cafe"],
        4,
    );
}

// ------------------------------------------------------------------ 3.5 chart

/// Food (Groceries -1000, Restaurants -500), Transport (Bus -300) and one
/// uncategorized -200, all inside the default date range: the parent chart
/// rows are Food 1500, Transport 300, Uncategorized 200.
fn seed_chart_rows(conn: &financi_db::rusqlite::Connection) {
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
fn click_chart_legend(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    id: Option<i64>,
) {
    let ix = cx.update(|cx| {
        shell
            .read(cx)
            .dashboard
            .chart_rows
            .iter()
            .position(|r| r.id == id)
    });
    let Some(ix) = ix else {
        panic!("category {id:?} is one of the legend's rows (wait for the refresh that shows it)")
    };
    click(cx, handle, format!("chart:legend-row:{ix}"));
}

fn top_category_id(shell: &Entity<Shell>, cx: &App, name: &str) -> i64 {
    shell
        .read(cx)
        .dashboard
        .chart_rows
        .iter()
        .find(|r| r.name == name)
        .expect("chart row")
        .id
        .expect("a real category")
}

/// The legend draws no numbers since 3.31, but the model rows still
/// carry each parent's amount and share — that is what the bar and
/// legend-row tooltips show.
#[gpui::test]
fn chart_rows_carry_amounts_and_shares_for_the_tooltips(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).dashboard.chart_rows.len() == 3
    });
    cx.update(|cx| {
        let rows = &shell.read(cx).dashboard.chart_rows;
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Food", "Transport", "Uncategorized"]);
        let cents: Vec<i64> = rows.iter().map(|r| r.spend_cents).collect();
        assert_eq!(cents, [1500, 300, 200]);
        for (row, share) in rows.iter().zip([0.75, 0.15, 0.10]) {
            assert!(
                (row.share - share).abs() < 1e-9,
                "{} share {}",
                row.name,
                row.share
            );
        }
    });
}

#[gpui::test]
fn clicking_a_parent_drills_the_chart_and_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));

    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "chart drilled to Food's children", |cx| {
        shell.read(cx).dashboard.chart_parent == Some(food)
            && shell.read(cx).dashboard.row_count == 2
    });
    cx.update(|cx| {
        let names: Vec<&str> = shell
            .read(cx)
            .dashboard
            .chart_rows
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["Groceries", "Restaurants"]);
        assert_eq!(
            shell.read(cx).dashboard.legend_selection,
            Some(LegendSelection::Category(food))
        );
        let rows = table_rows(&shell, cx);
        let mut descs: Vec<&str> = rows.iter().map(|r| r.description.as_str()).collect();
        descs.sort_unstable();
        assert_eq!(descs, ["CORNER CAFE", "GROCER STORE"]);
    });
}

#[gpui::test]
fn all_categories_row_clears_drill_and_filter(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled", |cx| {
        shell.read(cx).dashboard.chart_parent == Some(food)
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_all_categories(window, cx));
    });
    wait_until(cx, "back to parents and all rows", |cx| {
        shell.read(cx).dashboard.chart_parent.is_none() && shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).dashboard.legend_selection, None);
        assert_eq!(shell.read(cx).dashboard.chart_rows.len(), 3);
    });
}

#[gpui::test]
fn child_click_filters_further_and_a_second_click_clears(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled into Food", |cx| {
        shell.read(cx).dashboard.chart_parent == Some(food)
    });
    // The chart's *rows* follow on the refresh the click started; the
    // child is only a legend row once they have landed.
    wait_idle(cx, &shell);
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "only the Groceries row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "GROCER STORE");
        assert_eq!(
            shell.read(cx).dashboard.legend_selection,
            Some(LegendSelection::Category(groceries))
        );
        // The chart stays on the children.
        assert_eq!(shell.read(cx).dashboard.chart_parent, Some(food));
    });

    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "filter cleared, children still shown", |cx| {
        shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| assert_eq!(shell.read(cx).dashboard.chart_parent, Some(food)));
}

#[gpui::test]
fn uncategorized_legend_row_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });

    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "only the uncategorized row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "Mystery Vendor");
        assert_eq!(
            shell.read(cx).dashboard.legend_selection,
            Some(LegendSelection::Uncategorized)
        );
        // The pseudo-row never drills the chart.
        assert!(shell.read(cx).dashboard.chart_parent.is_none());
        assert_eq!(shell.read(cx).dashboard.chart_rows.len(), 3);
    });

    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "all rows back", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| assert!(shell.read(cx).dashboard.legend_selection.is_none()));
}

#[gpui::test]
fn escape_clears_the_drill_and_the_selection(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled with a selection", |cx| {
        shell.read(cx).dashboard.chart_parent == Some(food)
            && shell.read(cx).dashboard.legend_selection == Some(LegendSelection::Category(food))
    });

    // Actions dispatch along the focus path, so focus the search input
    // first — like a user who clicked into the window. The real Esc
    // keybinding (registered in `Shell::new`) must map the keystroke to
    // `ClearCategorySelection`; dispatching the action directly would not
    // test the binding.
    with_window(cx, handle, |window, cx| {
        let input = shell
            .read(cx)
            .dashboard
            .search_input
            .read(cx)
            .focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "escape");
    // The state clears synchronously in the action handler; the rows
    // follow when the refresh it started lands, so wait for both.
    wait_until(cx, "drill, selection and rows cleared", |cx| {
        shell.read(cx).dashboard.chart_parent.is_none()
            && shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.row_count == 4
    });
}

// -- Responsive charts (3.15) -------------------------------------------

#[test]
fn band_labels_truncate_to_the_character_budget() {
    let names = vec![
        "Food".to_string(),
        "Public Transport Costs".to_string(),
        "Café du Marché".to_string(),
    ];
    let shorts = short_chart_labels(&names, 8);
    assert_eq!(shorts[0].as_str(), "Food", "a short name is left alone");
    assert_eq!(shorts[1].as_str(), "Public…");
    assert_eq!(shorts[2].as_str(), "Café du…", "truncates by characters");

    // No budget means no truncation.
    let all = short_chart_labels(&names, 0);
    assert_eq!(all[1].as_str(), "Public Transport Costs");
}

#[test]
fn colliding_short_labels_fall_back_to_their_full_names() {
    // The band text is also `BarChart`'s band key, so two rows must never
    // be shortened to the same string.
    let names = vec![
        "Travel International".to_string(),
        "Travel Domestic".to_string(),
    ];
    let shorts = short_chart_labels(&names, 8);
    assert_eq!(shorts[0].as_str(), "Travel International");
    assert_eq!(shorts[1].as_str(), "Travel Domestic");

    // Without a collision the shortening stands.
    let names = vec!["Travel International".to_string(), "Food".to_string()];
    assert_eq!(short_chart_labels(&names, 8)[0].as_str(), "Travel…");
}

#[test]
fn band_label_budget_keeps_labels_inside_their_spacing() {
    // A wide plot with few bands: label every band with room to spare.
    let (chars, labels) = band_label_budget(779., 7);
    assert_eq!(labels, 7);
    assert!(chars >= 12, "{chars} chars at 779px across 7 bands");

    // The minimum-window plot (~167px) with 7 bands: too tight for all
    // seven, so labels are thinned out but still legible.
    let (chars, labels) = band_label_budget(167., 7);
    assert!((1..7).contains(&labels), "{labels} labels at 167px");
    assert!(chars >= CHART_LABEL_MIN_CHARS, "{chars}");

    // The invariant for every window size and category count: each drawn
    // label owns `chart_width / labels` pixels, enough for its characters
    // plus the gap, so centred labels cannot overlap or run past the
    // plot's edges.
    for width in [100., 167., 400., 779., 1600., 3000.] {
        for count in 1..=30usize {
            let (chars, labels) = band_label_budget(width, count);
            assert!(
                labels >= 1 && labels <= count,
                "width {width}, count {count}"
            );
            assert!((1..=CHART_LABEL_MAX_CHARS).contains(&chars));
            let per = width / labels as f32;
            assert!(
                chars as f32 * CHART_CHAR_PX + CHART_LABEL_GAP <= per + 0.001,
                "width {width}, count {count}: {chars} chars need more than {per:.1}px"
            );
        }
    }
    assert_eq!(band_label_budget(779., 0), (0, 0));
}

// -- Bar share labels and the slim legend (3.31) ------------------------

#[test]
fn bar_share_labels_read_as_whole_percents() {
    assert_eq!(bar_share_label(0.75), "75%");
    assert_eq!(bar_share_label(0.182), "18%");
    assert_eq!(bar_share_label(1.0), "100%");
    assert_eq!(
        bar_share_label(0.999),
        "100%",
        "rounds up to a whole percent"
    );
    assert_eq!(
        bar_share_label(0.01),
        "1%",
        "one percent is a whole percent"
    );
    assert_eq!(
        bar_share_label(0.009),
        "0.9%",
        "below one percent, one decimal"
    );
    assert_eq!(bar_share_label(0.004), "0.4%");
    assert_eq!(bar_share_label(0.0), "0%");
    assert_eq!(
        bar_share_label(0.0095),
        "0.9%",
        "0.95–1% still reads with its one decimal"
    );
    assert_eq!(bar_share_label(0.0094), "0.9%");
    assert_eq!(
        bar_share_label(0.0099),
        "1.0%",
        "0.99% rounds to one percent with its decimal, not a bare 1%"
    );
    assert_eq!(
        bar_share_label(0.0004),
        "0.1%",
        "a sliver below 0.05% ($1 of $2,500) rounds up, never to 0.0%"
    );
    assert_eq!(bar_share_label(0.00004), "0.1%");

    // The tooltip body is the bar's and the legend row's one string.
    assert_eq!(
        chart_tooltip_value(123456, 0.182, " of Food"),
        "$1,234.56 · 18.2% of Food"
    );
    assert_eq!(chart_tooltip_value(1500, 0.75, ""), "$15.00 · 75.0%");
}

#[test]
fn bar_label_plan_thins_share_labels_without_collisions() {
    // A wide plot with few bands: every bar keeps its share label.
    assert_eq!(bar_label_plan(779., 7), 7);
    // The 1200 × 720 plot (~247 px) with 15 bars: too narrow for all
    // fifteen, so the labels are thinned out.
    let labels = bar_label_plan(247., 15);
    assert!((1..15).contains(&labels), "{labels} labels at 247px");
    assert_eq!(bar_label_plan(0., 5), 1, "a degenerate plot labels one bar");
    assert_eq!(bar_label_plan(100., 0), 0);
    assert_eq!(bar_label_plan(100., 1), 1);

    // The invariant for every window size and category count: drawn
    // labels are spread first-to-last and their band centres sit at
    // least a full label apart, so centred labels cannot overlap.
    let need = BAR_LABEL_MAX_CHARS as f32 * CHART_CHAR_PX;
    for width in [100., 247., 400., 779., 1600., 3000.] {
        for count in 1..=30usize {
            let labels = bar_label_plan(width, count);
            assert!(
                (1..=count).contains(&labels),
                "width {width}, count {count}: {labels} labels"
            );
            let slots = bar_label_slots(count, labels);
            let drawn: Vec<usize> = slots
                .iter()
                .enumerate()
                .filter(|(_, on)| **on)
                .map(|(ix, _)| ix)
                .collect();
            assert_eq!(drawn.len(), labels, "width {width}, count {count}");
            assert_eq!(drawn.first(), Some(&0), "the first bar is labelled");
            assert_eq!(drawn.last(), Some(&(count - 1)), "the last bar is labelled");
            let band_w = width / count as f32;
            for pair in drawn.windows(2) {
                let spacing = (pair[1] - pair[0]) as f32 * band_w;
                assert!(
                    spacing >= need - 0.001,
                    "width {width}, count {count}: labels on bars {} and {} \
                     are {spacing:.1}px apart, need {need:.1}",
                    pair[0],
                    pair[1],
                );
            }
        }
    }
}

#[gpui::test]
fn chart_labels_adapt_to_the_window_size(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).dashboard.chart_rows.len() == 3
    });

    // `open_shell` opens 1200 wide: with the legend beside it the plot is
    // narrow, so the labels are truncated to a small character budget.
    let (small_chars, small_labels) = cx.update(|cx| shell.read(cx).chart_label_plan());
    assert_eq!(small_labels, 3);
    assert!(
        small_chars < CHART_LABEL_MAX_CHARS,
        "a narrow window truncates, got {small_chars} chars"
    );

    // A large window: every band gets its label again, and the labels
    // grow. The window-bounds observer must have updated the plan.
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(2000.), px(900.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| shell.read(cx).window_width), 2000.);
    let (big_chars, big_labels) = cx.update(|cx| shell.read(cx).chart_label_plan());
    assert_eq!(big_labels, 3, "a wide window labels every band");
    assert!(big_chars > small_chars, "{big_chars} vs {small_chars}");

    // And the chart draws at both the minimum and a large window.
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(2560.), px(1440.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
}

#[gpui::test]
fn the_chart_renders_long_names_and_many_categories(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).dashboard.chart_rows.len() == 3
    });
    // 25 categories with long names — the worst case for the band axis.
    shell.update(cx, |s, cx| {
        s.dashboard.chart_rows = (0..25)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Category number {i} with a very long descriptive name"),
                spend_cents: (i + 1) * 12_345,
                share: 0.04,
            })
            .collect();
        cx.notify();
    });
    // The plan stays bounded, and the chart draws without panic at the
    // test window's width and at the minimum size. 25 bands across the
    // narrow plot must thin the labels out, not squeeze them flat.
    let (chars, labels) = cx.update(|cx| shell.read(cx).chart_label_plan());
    assert!((1..25).contains(&labels), "{labels} labels for 25 bands");
    assert!((1..=CHART_LABEL_MAX_CHARS).contains(&chars));
    draw_frame(cx, _handle);
}

/// The legend column is just the key (3.31), so it must fit 140 px.
const _: () = assert!(LEGEND_W <= 140.);

/// 3.31: the legend is just swatch and name. The amount (76 px) and share
/// (52 px) columns are gone, the column is 140 px wide, and the plot gets
/// the width back — at 1200 × 720 it is far wider than the ~167 px the
/// 220 px legend left it.
#[gpui::test]
fn the_legend_is_just_a_key(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).dashboard.chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let rows: Vec<(String, i64, f64)> = cx.update(|cx| {
        shell
            .read(cx)
            .dashboard
            .chart_rows
            .iter()
            .map(|r| (r.name.to_string(), r.spend_cents, r.share))
            .collect()
    });
    assert_eq!(rows.len(), 3);

    let legend = window
        .debug_bounds("chart:legend")
        .expect("the legend paints");
    assert!(
        f32::from(legend.size.width) <= LEGEND_W + 0.5,
        "the legend column is {:?} wide, expected ≤ {LEGEND_W}",
        legend.size.width,
    );
    let plot = window.debug_bounds("chart:plot").expect("the plot paints");
    assert!(
        f32::from(plot.size.width) >= 230.,
        "the plot is {:?} wide at 1200 × 720 — the 220 px legend left it ~167 px",
        plot.size.width,
    );

    // Each row draws its name and nothing else: no amount, no percentage.
    for (ix, (name, cents, share)) in rows.iter().enumerate() {
        assert!(
            paints(cx, handle, format!("chart:legend-text:{ix}:{name}")),
            "legend row {ix} draws its name"
        );
        for gone in [crate::flow::money(*cents), format!("{:.1}%", share * 100.)] {
            assert!(
                !paints(cx, handle, format!("chart:legend-text:{ix}:{gone}")),
                "legend row {ix} no longer draws {gone:?}"
            );
        }
    }
}

/// 3.31: hovering a legend row shows the bar's own tooltip text — full
/// name, exact amount and share.
#[gpui::test]
fn hovering_a_legend_row_shows_amount_and_share(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).dashboard.chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let value = cx.update(|cx| {
        let r = &shell.read(cx).dashboard.chart_rows[0];
        assert_eq!(r.name.as_str(), "Food");
        chart_tooltip_value(r.spend_cents, r.share, "")
    });
    assert_eq!(value, "$15.00 · 75.0%");
    let tip_sel = format!("chart:legend-tooltip:Food:{value}");
    assert!(
        !paints(cx, handle, tip_sel.clone()),
        "nothing hovers a row, so no tooltip is drawn yet"
    );

    let row = window
        .debug_bounds("chart:legend-row:0")
        .expect("the first legend row paints");
    let centre = point(
        row.origin.x + row.size.width / 2.,
        row.origin.y + row.size.height / 2.,
    );
    window.simulate_mouse_move(centre, None, Modifiers::default());
    // The tooltip appears after a 500 ms hover; the test clock is fake.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, tip_sel.clone()),
        "hovering the legend row shows a tooltip with the amount and share",
    );
}

/// 3.31: at the minimum window with 15 categories the bars are too narrow
/// for every share label, so the plan thins them the way band labels are
/// thinned (3.15) — and every drawn label, centred over its bar, stays
/// inside the chart panel and clear of its neighbours.
#[gpui::test]
fn fifteen_category_bars_keep_their_share_labels_clear(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).dashboard.chart_rows.len() == 3
    });
    shell.update(cx, |s, cx| {
        s.dashboard.chart_rows = (0..15)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Cat {i:02}"),
                spend_cents: (i + 1) * 10_000,
                // The share of the total this row's spend represents, so
                // every bar's share label is a different string.
                share: (i + 1) as f64 / 120.,
            })
            .collect();
        cx.notify();
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);

    let plot = window.debug_bounds("chart:plot").expect("the plot paints");
    let panel = window
        .debug_bounds("panel:chart")
        .expect("the chart panel paints");
    let plan_w = cx.update(|cx| shell.read(cx).chart_plot_width());
    assert!(
        (f32::from(plot.size.width) - plan_w).abs() <= 3.,
        "the plan measured the plot at {plan_w:.1}px, it painted {:?} \
         (the panel's 1 px borders are the only expected slack)",
        plot.size.width,
    );

    // The bars are too narrow for fifteen labels: the plan thins them.
    let plot_w = f32::from(plot.size.width);
    let labels = bar_label_plan(plot_w, 15);
    assert!((1..15).contains(&labels), "{labels} labels for 15 bars");

    // Drawn labels: first-to-last, centres at least a whole label apart,
    // and the first and last ones inside the panel's borders even when
    // centred over the edge bars.
    let slots = bar_label_slots(15, labels);
    let drawn: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .map(|(ix, _)| ix)
        .collect();
    assert_eq!(drawn.first(), Some(&0));
    assert_eq!(drawn.last(), Some(&14));

    // The wiring (3.31): the rows `chart_panel` hands the chart carry
    // their own share's label on the planned bars and nothing on the
    // thinned-out ones — drop the label call in `chart_bars` or misalign
    // it and this fails.
    let bars = cx.update(|cx| {
        shell
            .read(cx)
            .chart_bars(plan_w, &Default::default(), gpui::black())
    });
    assert_eq!(bars.len(), 15);
    let wired_slots = bar_label_slots(15, bar_label_plan(plan_w, 15));
    for (ix, bar) in bars.iter().enumerate() {
        let want = if wired_slots[ix] {
            bar_share_label(bar.share)
        } else {
            String::new()
        };
        assert_eq!(bar.bar_label.to_string(), want, "bar {ix}");
    }
    assert!(
        !bars[0].bar_label.is_empty() && !bars[14].bar_label.is_empty(),
        "the first and last bars carry their share"
    );

    let band_w = plot_w / 15.;
    let half = BAR_LABEL_MAX_CHARS as f32 * CHART_CHAR_PX / 2.;
    for pair in drawn.windows(2) {
        assert!(
            (pair[1] - pair[0]) as f32 * band_w >= 2. * half,
            "the labels on bars {} and {} overlap: {:.1}px apart",
            pair[0],
            pair[1],
            (pair[1] - pair[0]) as f32 * band_w,
        );
    }
    let first_left = plot.origin.x + px(0.5 * band_w - half);
    let last_right = plot.origin.x + px(14.5 * band_w + half);
    assert!(
        f32::from(first_left) >= f32::from(panel.origin.x),
        "the first bar's label spills past the panel's left border",
    );
    assert!(
        f32::from(last_right) <= f32::from(panel.origin.x + panel.size.width),
        "the last bar's label spills past the panel's right border",
    );
}

/// `seed_chart_rows` plus a childless Gifts parent whose only transaction
/// (2026-10-05) falls in the default range but not in "this month", so a
/// preset change can empty a selection out from under the legend.
fn seed_chart_rows_split(conn: &financi_db::rusqlite::Connection) {
    seed_chart_rows(conn);
    let gifts =
        financi_db::categories::create_parent(conn, "Gifts", CategoryKind::Expense).unwrap();
    let acct = conn
        .query_row("SELECT id FROM accounts WHERE name = 'Cobalt'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    let import = conn
        .query_row("SELECT id FROM imports LIMIT 1", [], |r| r.get::<_, i64>(0))
        .unwrap();
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents,
                                   description_raw, category_id)
         VALUES (?1, ?2, 9, X'00', 'chart-hash-gifts', '2026-10-05', -400, 'GIFT SHOP', ?3)",
        (acct, import, gifts),
    )
    .unwrap();
}

fn set_uncategorized_only(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    on: bool,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_uncategorized_only(on, window, cx));
    });
}

#[gpui::test]
fn a_filter_change_drops_the_stale_legend_selection(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows_split);
    wait_until(cx, "four chart rows", |cx| {
        shell.read(cx).dashboard.chart_rows.len() == 4
    });
    let gifts = cx.update(|cx| top_category_id(&shell, cx, "Gifts"));

    // Gifts has no children: the drill snaps back to the top level while
    // the selection (and the table filter) stay on Gifts.
    click_chart_legend(cx, handle, &shell, Some(gifts));
    wait_until(cx, "Gifts selected, table filtered", |cx| {
        shell.read(cx).dashboard.legend_selection == Some(LegendSelection::Category(gifts))
            && shell.read(cx).dashboard.chart_parent.is_none()
            && shell.read(cx).dashboard.row_count == 1
    });

    // "This month" drops the October Gifts spend: the stale selection must
    // go with it, and the table must widen back to the remaining rows.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::ThisMonth, window, cx)
        });
    });
    wait_until(cx, "selection and drill cleared, rows back", |cx| {
        shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.chart_parent.is_none()
            // Four rows remain this month: two Food, one Transport, one
            // uncategorized (the October Gifts row is out of range).
            && shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| {
        let names: Vec<&str> = shell
            .read(cx)
            .dashboard
            .chart_rows
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["Food", "Transport", "Uncategorized"]);
    });
}

#[gpui::test]
fn the_uncategorized_toggle_and_a_category_selection_are_exclusive(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));

    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "only the uncategorized row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });

    // A category legend row wins: the checkbox goes off, the table shows
    // the category again.
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "Food filters the table", |cx| {
        !shell.read(cx).dashboard.uncategorized_only
            && shell.read(cx).dashboard.legend_selection == Some(LegendSelection::Category(food))
            && shell.read(cx).dashboard.row_count == 2
    });

    // Toggling the checkbox back on clears the category selection (the
    // chart stays drilled) instead of stacking both clauses.
    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "uncategorized only again", |cx| {
        shell.read(cx).dashboard.uncategorized_only
            && shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).dashboard.chart_parent, Some(food));
        let rows = table_rows(&shell, cx);
        assert!(rows.iter().all(|r| r.category_id.is_none()));
    });
}

// ------------------------------------------------------- 3.6 monthly summary

/// Two fixed months (March and April 2026): Food (Groceries -1000 -600,
/// Restaurants -500), Transport (Bus -300), one uncategorized -200, and a
/// $2,105.00 salary each month. The tests switch to "All time", so the
/// grid covers exactly the seeded months whatever today's date is.
fn seed_summary_rows(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-01-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
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
    let salary =
        financi_db::categories::create_parent(conn, "Salary", CategoryKind::Income).unwrap();

    let mut line = 0i64;
    let mut txn = |description: &str, date: &str, cents: i64, category: Option<i64>| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("summary-hash-{line}"),
                date,
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn("GROCER STORE MAR", "2026-03-10", -1000, Some(groceries));
    txn("CORNER CAFE MAR", "2026-03-10", -500, Some(restaurants));
    txn("Mystery Vendor", "2026-03-10", -200, None);
    txn("PAYROLL DEPOSIT", "2026-03-15", 210500, Some(salary));
    txn("GROCER STORE APR", "2026-04-10", -600, Some(groceries));
    txn("CITY BUS", "2026-04-10", -300, Some(bus));
    txn("PAYROLL DEPOSIT", "2026-04-15", 210500, Some(salary));
}

/// [`seed_summary_rows`] plus a large uncategorized March purchase, so the
/// uncategorized total beats every category's and the query — which orders
/// rows by amount — puts `Uncategorized` first. The view must still render
/// it as the last category row.
fn seed_summary_rows_uncategorized_largest(conn: &financi_db::rusqlite::Connection) {
    seed_summary_rows(conn);
    let (acct, import) = conn
        .query_row(
            "SELECT account_id, import_id FROM transactions LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents,
                                   description_raw, category_id)
         VALUES (?1, ?2, 7, X'00', 'summary-hash-largest', '2026-03-20', -5000,
                 'MYSTERY MEGA PURCHASE', NULL)",
        (acct, import),
    )
    .unwrap();
}

/// Switch the dashboard to "All time" and wait until the summary grid
/// covers exactly the two seeded months.
fn set_all_time(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::AllTime, window, cx);
        });
    });
    wait_until(cx, "summary covers the two seeded months", |cx| {
        shell.read(cx).dashboard.summary.months == ["2026-03", "2026-04"]
    });
}

/// 3.75: click a summary grid cell for real. `id` is the category row
/// (`None` — the Uncategorized row), `month` the column (`None` — Total).
fn click_summary_cell(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    id: Option<i64>,
    month: Option<&str>,
) {
    let found = cx.update(|cx| {
        let s = shell.read(cx);
        let ix = s.summary_rows().iter().position(|r| {
            r.id == id
                && !matches!(
                    r.kind,
                    SummaryRowKind::Income | SummaryRowKind::Spend | SummaryRowKind::Net
                )
        })?;
        let months = &s.dashboard.summary.months;
        let col = match month {
            Some(m) => months.iter().position(|x| x == m)?,
            None => months.len(),
        };
        Some((ix, col))
    });
    let Some((ix, col)) = found else {
        panic!(
            "category {id:?} paints a summary row (children only after \
             `expand_summary_parent`) and month {month:?} is a column"
        )
    };
    click(cx, handle, format!("summary:cell:{ix}:{col}"));
}

/// Expand a collapsed summary parent by clicking its name cell, so its
/// child rows paint and can be clicked (3.75).
fn expand_summary_parent(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    parent: i64,
) {
    let ix = cx.update(|cx| {
        shell
            .read(cx)
            .summary_rows()
            .iter()
            .position(|r| r.id == Some(parent))
    });
    let Some(ix) = ix else {
        panic!("parent {parent} is a summary row")
    };
    click(cx, handle, format!("summary:name-{ix}"));
    wait_until(cx, "parent expanded", |cx| {
        matches!(
            shell
                .read(cx)
                .summary_rows()
                .iter()
                .find(|r| r.id == Some(parent))
                .map(|r| r.kind),
            Some(SummaryRowKind::Parent { expanded: true, .. })
        )
    });
}

fn summary_row_names(shell: &Entity<Shell>, cx: &App) -> Vec<String> {
    shell
        .read(cx)
        .summary_rows()
        .into_iter()
        .map(|r| r.name.to_string())
        .collect()
}

fn summary_parent_id(shell: &Entity<Shell>, cx: &App, name: &str) -> i64 {
    shell
        .read(cx)
        .summary_rows()
        .into_iter()
        .find(|r| r.name == name)
        .expect("summary row")
        .id
        .expect("a real category")
}

#[gpui::test]
fn summary_shows_months_total_and_the_three_total_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 7
    });
    cx.update(|cx| {
        let rows = shell.read(cx).summary_rows();
        let names: Vec<String> = rows.iter().map(|r| r.name.to_string()).collect();
        // Children are collapsed by default; the three total rows close
        // the grid.
        assert_eq!(
            names,
            [
                "Food",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
        let row = |name: &str| rows.iter().find(|r| r.name == name).unwrap();
        assert_eq!(row("Food").cells, [1500, 600, 2100]);
        assert_eq!(row("Uncategorized").cells, [200, 0, 200]);
        assert_eq!(row("Income").cells, [210500, 210500, 421000]);
        assert_eq!(row("Spend").cells, [1700, 900, 2600]);
        assert_eq!(row("Net").cells, [208800, 209600, 418400]);
        assert_eq!(
            row("Food").kind,
            SummaryRowKind::Parent {
                expandable: true,
                expanded: false
            }
        );
        assert_eq!(row("Uncategorized").kind, SummaryRowKind::Uncategorized);
        assert_eq!(row("Income").kind, SummaryRowKind::Income);
        assert_eq!(row("Spend").kind, SummaryRowKind::Spend);
        assert_eq!(row("Net").kind, SummaryRowKind::Net);
    });
}

#[gpui::test]
fn clicking_a_summary_cell_filters_the_table_to_category_and_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 7
    });
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));

    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "only March's Food rows", |cx| {
        shell.read(cx).dashboard.row_count == 2
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        let mut descs: Vec<&str> = rows.iter().map(|r| r.description.as_str()).collect();
        descs.sort_unstable();
        assert_eq!(descs, ["CORNER CAFE MAR", "GROCER STORE MAR"]);
        assert_eq!(
            shell.read(cx).dashboard.summary_month.as_deref(),
            Some("2026-03")
        );
        assert_eq!(
            shell.read(cx).dashboard.legend_selection,
            Some(LegendSelection::Category(food))
        );
        // The parent is visible at the top level, so the chart stays put.
        assert!(shell.read(cx).dashboard.chart_parent.is_none());
    });

    // Clicking the same cell again clears both halves of the filter.
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "all rows back", |cx| {
        shell.read(cx).dashboard.row_count == 7
            && shell.read(cx).dashboard.summary_month.is_none()
            && shell.read(cx).dashboard.legend_selection.is_none()
    });
}

#[gpui::test]
fn clicking_a_child_summary_cell_drills_the_chart_to_its_parent(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 7
    });
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    // Children only paint once the parent is expanded — click its cell,
    // not a row the user cannot see.
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    expand_summary_parent(cx, handle, &shell, food);

    click_summary_cell(cx, handle, &shell, Some(groceries), Some("2026-04"));
    wait_until(cx, "only April's Groceries row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "GROCER STORE APR");
        assert_eq!(
            shell.read(cx).dashboard.legend_selection,
            Some(LegendSelection::Category(groceries))
        );
        assert_eq!(
            shell.read(cx).dashboard.summary_month.as_deref(),
            Some("2026-04")
        );
        // The selection must be one of the legend's rows, so the chart
        // drills to the child's parent and stays there across refreshes.
        assert_eq!(shell.read(cx).dashboard.chart_parent, Some(food));
    });
}

#[gpui::test]
fn expanding_a_summary_parent_shows_its_child_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));

    cx.update(|cx| shell.update(cx, |s, _| s.toggle_summary_parent(food)));
    cx.update(|cx| {
        let rows = shell.read(cx).summary_rows();
        let names: Vec<String> = rows.iter().map(|r| r.name.to_string()).collect();
        assert_eq!(
            names,
            [
                "Food",
                "Groceries",
                "Restaurants",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
        assert_eq!(
            rows[0].kind,
            SummaryRowKind::Parent {
                expandable: true,
                expanded: true
            }
        );
        assert_eq!(rows[1].kind, SummaryRowKind::Child);
        assert_eq!(rows[1].cells, [1000, 600, 1600]);
        assert_eq!(rows[2].kind, SummaryRowKind::Child);
    });

    // Collapse again: the children disappear.
    cx.update(|cx| shell.update(cx, |s, _| s.toggle_summary_parent(food)));
    cx.update(|cx| {
        assert_eq!(
            summary_row_names(&shell, cx),
            [
                "Food",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
    });
}

/// The month is only half of a summary-cell selection: every handler that
/// clears `legend_selection` must clear `summary_month` too, or the table
/// stays silently month-filtered — no cell highlight, and the refresh
/// revalidation only drops a month missing from the grid.
#[gpui::test]
fn clearing_the_legend_selection_clears_the_summary_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 7
    });
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    // The Groceries cell only exists once Food is expanded.
    expand_summary_parent(cx, handle, &shell, food);

    // Re-clicking the selected legend row: the pair clears together.
    click_summary_cell(cx, handle, &shell, Some(groceries), Some("2026-04"));
    wait_until(cx, "only April's Groceries row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "the month goes with the selection", |cx| {
        shell.read(cx).dashboard.row_count == 7
            && shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.summary_month.is_none()
    });

    // Re-clicking the `Uncategorized` legend row: the same.
    click_summary_cell(cx, handle, &shell, None, Some("2026-03"));
    wait_until(cx, "only March's uncategorized row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "the month goes with the uncategorized row", |cx| {
        shell.read(cx).dashboard.row_count == 7
            && shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.summary_month.is_none()
    });

    // "← All categories".
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "only March's Food rows", |cx| {
        shell.read(cx).dashboard.row_count == 2
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_all_categories(window, cx));
    });
    wait_until(cx, "the month goes with all-categories", |cx| {
        shell.read(cx).dashboard.row_count == 7
            && shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.summary_month.is_none()
    });

    // Turning on "Uncategorized only" over a category selection. Without
    // the month clearing this would show the empty intersection of
    // uncategorized April rows instead of the one uncategorized row.
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-04"));
    wait_until(cx, "only April's Food row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "uncategorized rows without the month", |cx| {
        shell.read(cx).dashboard.legend_selection.is_none()
            && shell.read(cx).dashboard.summary_month.is_none()
            && shell.read(cx).dashboard.uncategorized_only
            && table_rows(&shell, cx)
                .iter()
                .map(|r| r.description.as_str())
                .eq(["Mystery Vendor"])
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).dashboard.row_count, 1);
    });
}

/// The grid comes from the query in amount order; the view still renders
/// `Uncategorized` as the last category row, before Income/Spend/Net,
/// whatever its amount (ui-conventions, *Numbers, dates and labels*).
#[gpui::test]
fn uncategorized_is_the_last_summary_category_row_whatever_its_amount(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows_uncategorized_largest);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 8
    });
    cx.update(|cx| {
        // The query really does put the biggest row — Uncategorized —
        // first, so the ordering below comes from the view's sort.
        assert_eq!(shell.read(cx).dashboard.summary.rows[0].id, None);
        let rows = shell.read(cx).summary_rows();
        let names: Vec<String> = rows.iter().map(|r| r.name.to_string()).collect();
        assert_eq!(
            names,
            [
                "Food",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
        assert_eq!(rows[2].kind, SummaryRowKind::Uncategorized);
        assert_eq!(rows[2].cells, [5200, 0, 5200]);
    });
}

/// 3.30: one spend per month for `months` consecutive months ending
/// March 2026, plus eight spend parents (so the rows overflow the panel
/// and vertical scrolling has somewhere to go). The grid gets exactly
/// `months` columns.
fn seed_summary_months(conn: &financi_db::rusqlite::Connection, months: usize) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-01-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let mut parents = Vec::new();
    for ix in 0..8 {
        parents.push(
            financi_db::categories::create_parent(
                conn,
                &format!("Cat {ix}"),
                CategoryKind::Expense,
            )
            .unwrap(),
        );
    }
    // The last `months` months, oldest first, ending 2026-03.
    let mut dates: Vec<String> = Vec::with_capacity(months);
    let (mut year, mut month) = (2026usize, 3usize);
    for _ in 0..months {
        dates.push(format!("{year:04}-{month:02}-10"));
        if month == 1 {
            year -= 1;
            month = 12;
        } else {
            month -= 1;
        }
    }
    dates.reverse();
    let mut line = 0i64;
    for (ix, date) in dates.iter().enumerate() {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("hscroll-hash-{line}"),
                date,
                -1000 - ix as i64,
                "MONTHLY SPEND",
                parents[0],
            ),
        )
        .unwrap();
        line += 1;
    }
    // One transaction per parent so every row appears in the grid.
    let last = dates.last().unwrap();
    for parent in &parents[1..] {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("hscroll-hash-{line}"),
                last,
                -500 - line,
                "ONE-OFF SPEND",
                *parent,
            ),
        )
        .unwrap();
        line += 1;
    }
}

/// 3.30: like [`set_all_time`], but waits for the grid to cover exactly
/// `months` months (the seeds of these tests span more than two).
fn set_all_time_covering(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    months: usize,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });
    wait_until(cx, "summary covers all the seeded months", |cx| {
        shell.read(cx).dashboard.summary.months.len() == months
    });
}

/// 3.30, measured at 1200 × 720: the pinned name column and `Total` paint
/// inside the panel, every visible month cell lies between them, and the
/// header and body months scroll by the same amount while the pinned
/// columns stay put.
#[gpui::test]
fn summary_months_scroll_under_pinned_name_and_total(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 24));
    resize_and_draw(cx, handle, 1200., 720.);
    set_all_time_covering(cx, handle, &shell, 24);
    wait_until(cx, "24 months in the grid", |cx| {
        shell.read(cx).dashboard.summary.months.len() == 24
    });
    draw_frame(cx, handle);

    let panel = bounds(cx, handle, "panel:summary");
    let name = bounds(cx, handle, "summary:name-0");
    let total = bounds(cx, handle, "summary:cell:0:24");
    let zone = bounds(cx, handle, "summary:months-0");
    // Both pinned columns paint inside the panel…
    assert!(
        panel.origin.x <= name.origin.x && name.right() <= panel.right(),
        "the name column paints inside the panel"
    );
    assert!(
        panel.origin.x <= total.origin.x && total.right() <= panel.right(),
        "the Total column paints inside the panel"
    );
    // …and the scrolling month zone sits between them.
    assert!(
        zone.origin.x >= name.right() - px(1.) && zone.right() <= total.origin.x + px(1.),
        "the month zone lies between the pinned columns"
    );
    // The month cells paint between the two pinned columns: the zone shows
    // whole columns, and a column scrolled past an edge is clipped there
    // (the viewport hides the overflow rather than painting over the names
    // or the Total), so no cell's visible part can leave the zone.
    let mut whole = 0;
    for col in 0..24 {
        let cell = bounds(cx, handle, format!("summary:cell:0:{col}"));
        if cell.origin.x >= zone.origin.x - px(1.) && cell.right() <= zone.right() + px(1.) {
            whole += 1;
        } else {
            // A partially visible column hangs over exactly one edge only.
            let hangs_left = cell.origin.x < zone.origin.x - px(1.);
            let hangs_right = cell.right() > zone.right() + px(1.);
            assert!(
                !(hangs_left && hangs_right),
                "month column {col} is wider than the month zone"
            );
        }
    }
    assert!(whole >= 1, "at least one whole month cell is visible");
    // A range change (the All-time switch above) lands on the most recent
    // month: the newest column is on screen beside `Total`, the oldest is
    // scrolled away to the left.
    let off0 = cx.update(|cx| shell.read(cx).dashboard.summary_scroll.offset().x);
    assert!(off0 < px(0.), "the 24-month strip is scrolled sideways");
    let newest = bounds(cx, handle, "summary:cell:0:23");
    assert!(
        newest.right() > zone.origin.x && newest.origin.x < zone.right(),
        "the latest month is on screen"
    );
    let oldest = bounds(cx, handle, "summary:cell:0:0");
    assert!(
        oldest.right() <= zone.origin.x + px(1.),
        "the oldest month is scrolled out to the left"
    );

    // Shift+wheel over the month zone scrolls it, and the header moves by
    // exactly the same amount as the body cells.
    let before_label = bounds(cx, handle, "summary:month-label-12").origin.x;
    let before_cell = bounds(cx, handle, "summary:cell:0:12").origin.x;
    let before_name = bounds(cx, handle, "summary:name-0").origin.x;
    let before_total = bounds(cx, handle, "summary:cell:0:24").origin.x;
    wheel(cx, handle, viewport_centre(zone), 240., 0.);
    draw_frame(cx, handle);
    let off1 = cx.update(|cx| shell.read(cx).dashboard.summary_scroll.offset().x);
    assert!(off1 > off0, "the strip scrolled back towards its left end");
    let moved = f32::from(off1 - off0);
    assert!(moved > 0.);
    let after_label = f32::from(bounds(cx, handle, "summary:month-label-12").origin.x);
    let after_cell = f32::from(bounds(cx, handle, "summary:cell:0:12").origin.x);
    assert!(
        (after_label - f32::from(before_label) - moved).abs() < 1.,
        "the header months moved with the handle"
    );
    assert!(
        (after_cell - f32::from(before_cell) - moved).abs() < 1.,
        "the body months moved by the same amount as the header"
    );
    // The pinned columns did not move.
    assert_eq!(
        bounds(cx, handle, "summary:name-0").origin.x,
        before_name,
        "the name column stays pinned"
    );
    assert_eq!(
        bounds(cx, handle, "summary:cell:0:24").origin.x,
        before_total,
        "the Total column stays pinned"
    );

    // Vertical scrolling of the body still works over the month zone,
    // and the name column moves with the rows (it is part of the body).
    let y_before = bounds(cx, handle, "summary:name-0").origin.y;
    wheel(cx, handle, viewport_centre(zone), 0., -40.);
    draw_frame(cx, handle);
    let y_after = bounds(cx, handle, "summary:name-0").origin.y;
    assert!(
        y_after < y_before,
        "the rows scroll vertically under the month zone, names included"
    );
}

/// 3.30: a horizontal swipe over the months scrolls the month strip only.
/// A y-only scroller remaps a horizontal wheel delta onto its own axis,
/// so `summary-rows` must restrict itself to the vertical axis — otherwise
/// the same swipe that moves the months also jumps the rows.
#[gpui::test]
fn a_horizontal_swipe_does_not_jump_the_summary_rows_vertically(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 24));
    resize_and_draw(cx, handle, 1200., 720.);
    set_all_time_covering(cx, handle, &shell, 24);
    wait_idle(cx, &shell);
    draw_frame(cx, handle);
    let zone = bounds(cx, handle, "summary:months-0");

    // Wheel the body down first, so there is somewhere to jump back to.
    let y_top = bounds(cx, handle, "summary:name-0").origin.y;
    wheel(cx, handle, viewport_centre(zone), 0., -40.);
    draw_frame(cx, handle);
    let y_scrolled = bounds(cx, handle, "summary:name-0").origin.y;
    assert!(y_scrolled < y_top, "the body is scrolled down");

    // A horizontal swipe over the month zone moves the months…
    let x0 = cx.update(|cx| shell.read(cx).dashboard.summary_scroll.offset().x);
    wheel(cx, handle, viewport_centre(zone), 200., 0.);
    draw_frame(cx, handle);
    let x1 = cx.update(|cx| shell.read(cx).dashboard.summary_scroll.offset().x);
    assert_ne!(x1, x0, "the month strip scrolled sideways");

    // …and leaves the body exactly where it was.
    let y_after = bounds(cx, handle, "summary:name-0").origin.y;
    assert_eq!(
        y_after, y_scrolled,
        "the horizontal swipe left the rows' vertical position alone"
    );
}

/// 3.30: changing the range re-lands the month strip on its most recent
/// month — even after the user had scrolled away.
#[gpui::test]
fn changing_the_summary_range_lands_on_the_latest_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 24));
    resize_and_draw(cx, handle, 1200., 720.);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::Last12Months, window, cx)
        })
    });
    wait_until(cx, "12 months in the grid", |cx| {
        shell.read(cx).dashboard.summary.months.len() == 12
    });
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let s = shell.read(cx);
        (
            s.dashboard.summary_scroll.offset().x,
            s.dashboard.summary_scroll.max_offset().x,
        )
    });
    assert!(max > px(0.), "twelve months overflow the month zone");
    assert!(
        (offset + max).abs() < px(1.),
        "the strip starts at its right end (offset {offset}, max {max})"
    );

    // Scroll away from the right end…
    let zone = bounds(cx, handle, "summary:months-0");
    wheel(cx, handle, viewport_centre(zone), 200., 0.);
    draw_frame(cx, handle);
    let scrolled = cx.update(|cx| shell.read(cx).dashboard.summary_scroll.offset().x);
    assert!(scrolled < max, "the strip is no longer at its right end");

    // …and change the range again: the latest month is beside `Total`.
    set_all_time_covering(cx, handle, &shell, 24);
    wait_until(cx, "24 months in the grid", |cx| {
        shell.read(cx).dashboard.summary.months.len() == 24
    });
    wait_idle(cx, &shell);
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let s = shell.read(cx);
        (
            s.dashboard.summary_scroll.offset().x,
            s.dashboard.summary_scroll.max_offset().x,
        )
    });
    assert!(
        (offset + max).abs() < px(1.),
        "the new range lands scrolled to the last month"
    );
    let zone = bounds(cx, handle, "summary:months-0");
    let newest = bounds(cx, handle, "summary:cell:0:23");
    assert!(
        newest.right() > zone.origin.x && newest.origin.x < zone.right(),
        "the latest month is on screen"
    );
}

/// 3.30: when every month fits there is nothing to scroll and the
/// horizontal scrollbar does not show.
///
/// This runs at 1600 × 900 rather than 1200 × 720: with the name pinned at
/// 160 px and Total at 96 px, the 1200-wide panel leaves only a ~200 px
/// month zone, so even three 88 px columns overflow it by construction.
/// "Everything fits" needs a window wide enough for three months plus the
/// two pinned columns.
#[gpui::test]
fn a_short_summary_range_shows_no_horizontal_scrollbar(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 3));
    resize_and_draw(cx, handle, 1600., 900.);
    set_all_time_covering(cx, handle, &shell, 3);
    // 3.75: let every in-flight refresh land before the clamp frames — a
    // late summary result would re-stamp the stick-to-end sentinel and the
    // frames below would read it half-clamped.
    wait_idle(cx, &shell);
    // Two frames: the scroll handle clamps the app's stick-to-end sentinel
    // only on the pass after the new months first paint.
    draw_frame(cx, handle);
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let s = shell.read(cx);
        (
            s.dashboard.summary_scroll.offset().x,
            s.dashboard.summary_scroll.max_offset().x,
        )
    });
    assert_eq!(
        max,
        px(0.),
        "three months must fit the month zone without scrolling"
    );
    assert_eq!(offset, px(0.), "and nothing is scrolled");
    // The strip still reads as today: the newest month sits flush against
    // `Total`.
    let total = bounds(cx, handle, "summary:cell:0:3");
    let newest = bounds(cx, handle, "summary:cell:0:2");
    assert!(
        (total.origin.x - newest.right()).abs() < px(9.),
        "the months sit right-aligned beside the Total column"
    );
}
