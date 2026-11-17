//! 3.46: the dashboard filters are global — search, "Uncategorized only",
//! the category selection and the month narrow the chart, the summary and
//! the table together. Covers the dimmed bars, the retitled chart, the
//! month heading, the chips row, Clear all and the no-matches link.

use super::summary::{
    click_summary_cell, expand_summary_parent, seed_summary_rows, set_all_time, summary_parent_id,
};
use super::*;
use crate::ui::dashboard::DashboardFilter;

/// 3.46: selecting a category keeps its bar at full colour and dims the
/// others to about a third rather than filtering them away (`BarChart`
/// has no click handler, so the summary grid and the legend rows are the
/// selection paths the tests drive; the bars paint from `chart_bars`).
/// The summary row highlights through the same `legend_selection` the
/// legend and the summary share, and the table filters to the category.
#[gpui::test]
fn a_selection_dims_the_other_bars_and_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    let bars = |cx: &App| {
        // Same inputs the share-label test uses: an empty id→index map
        // and palette, so every category paints `muted` and only the
        // 3.46 dimming changes the alpha.
        dash(cx, &shell).read(cx).chart_bars(
            800.,
            &Default::default(),
            Default::default(),
            gpui::black(),
        )
    };
    let alpha = |cx: &App, name: &str| {
        bars(cx)
            .into_iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("{name} is a bar"))
            .color
            .a
    };

    // The parent selected from the grid: it stays at the top level, so
    // the other bars dim beside it and the table narrows to Food.
    click_summary_cell(cx, handle, &shell, Some(food), None);
    wait_until(cx, "the table keeps only the Food rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    cx.update(|cx| {
        assert_eq!(bars(cx).len(), 3, "a selection dims, never filters, bars");
        assert!((alpha(cx, "Food") - 1.0).abs() < 0.01);
        assert!((alpha(cx, "Transport") - 0.35).abs() < 0.01);
        assert!((alpha(cx, "Uncategorized") - 0.35).abs() < 0.01);
        let d = dash(cx, &shell).read(cx);
        assert!(matches!(
            d.filter.legend_selection,
            Some(LegendSelection::Category(id)) if id == food
        ));
    });
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("filters:chip-category").is_some(),
        "the category chip shows while the selection holds"
    );

    // The legend row for the parent (3.45): it selects and drills in one
    // click. The chart's rows are then the parent's children — all part
    // of the selection — so none of them dims.
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "the chart drills into the selected parent", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.chart_parent == Some(food)
            && matches!(d.filter.legend_selection,
                Some(LegendSelection::Category(id)) if id == food)
            && d.chart_rows.iter().any(|r| r.name == "Groceries")
    });
    cx.update(|cx| {
        let bars = bars(cx);
        // Drilled in, the legend's rows are the parent's children; every
        // one belongs to the selection, so none of them dims.
        assert_eq!(bars.len(), 2);
        assert!((alpha(cx, "Groceries") - 1.0).abs() < 0.01);
        assert!((alpha(cx, "Restaurants") - 1.0).abs() < 0.01);
    });

    // A child selected from the grid: the chart stays drilled into the
    // parent, the child keeps its colour and its sibling dims.
    let groceries = cx.update(|cx| {
        dash(cx, &shell)
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food")
            .unwrap()
            .children
            .iter()
            .find(|c| c.name == "Groceries")
            .unwrap()
            .id
    });
    expand_summary_parent(cx, handle, &shell, food);
    click_summary_cell(cx, handle, &shell, Some(groceries), None);
    wait_until(cx, "the table keeps only the Groceries rows", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.chart_parent == Some(food) && d.row_count == 2
    });
    cx.update(|cx| {
        assert!((alpha(cx, "Groceries") - 1.0).abs() < 0.01);
        assert!((alpha(cx, "Restaurants") - 0.35).abs() < 0.01);
    });
}

/// 3.46: a summary cell's month narrows the chart to that month and
/// retitles it, filters the table, and leaves the summary grid itself
/// whole (its column tint shows the month).
#[gpui::test]
fn a_summary_cell_narrows_the_chart_and_its_title(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));

    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-04"));
    wait_until(cx, "the chart narrows to April", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.summary_month.as_deref() == Some("2026-04")
            && d.row_count == 1
            && d.chart_rows.len() == 2
    });

    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        let spend = |name: &str| {
            d.chart_rows
                .iter()
                .find(|r| r.name == name)
                .map(|r| r.spend_cents)
        };
        assert_eq!(
            (spend("Food"), spend("Transport")),
            (Some(600), Some(300)),
            "the chart totals cover April only"
        );
        // The grid keeps every month; only the cell's month filters the
        // chart, the table and the tint.
        let rows = d.summary_rows();
        let food = rows.iter().find(|r| r.name == "Food").unwrap();
        assert_eq!(food.cents, [1500, 600, 2100]);
    });
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window
            .debug_bounds("label:Spending by category · Apr 2026")
            .is_some(),
        "the chart panel retitles to the narrowed month"
    );
}

/// 3.46: the month heading selects its month on its own — no category is
/// involved — and clicking it again clears it.
#[gpui::test]
fn a_month_heading_selects_the_month_alone(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });

    click(cx, handle, "summary:month-label-1");
    wait_until(cx, "April alone is selected", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.summary_month.as_deref() == Some("2026-04")
            && d.row_count == 3
            && d.chart_rows.len() == 2
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert!(
            d.filter.legend_selection.is_none(),
            "the heading selects the month alone"
        );
        assert_eq!(d.chart_rows.iter().map(|r| r.spend_cents).sum::<i64>(), 900);
    });
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window
            .debug_bounds("label:Spending by category · Apr 2026")
            .is_some()
    );
    assert!(
        window.debug_bounds("filters:chip-month").is_some(),
        "the month chip shows while the heading holds"
    );

    click(cx, handle, "summary:month-label-1");
    wait_until(cx, "the heading toggles off", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.summary_month.is_none() && d.row_count == 7
    });
}

/// 3.46: the search box is global too — one term narrows the chart totals,
/// the summary grid and the table at once.
#[gpui::test]
fn search_narrows_the_chart_summary_and_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.apply_search("GROCER", window, cx));
    });
    wait_until(cx, "all three views answer the search", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.row_count == 2 && d.chart_rows.len() == 1 && d.chart_rows[0].spend_cents == 1600
    });

    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.chart_rows[0].name, "Food");
        let rows = d.summary_rows();
        let names: Vec<String> = rows.iter().map(|r| r.name.to_string()).collect();
        assert!(names.contains(&"Food".to_string()), "{names:?}");
        assert!(
            !names
                .iter()
                .any(|n| n == "Transport" || n == "Uncategorized"),
            "the grid only holds what the search leaves: {names:?}",
        );
        let food = rows.iter().find(|r| r.name == "Food").unwrap();
        assert_eq!(food.cents, [1000, 600, 1600]);
    });
}

/// 3.46: Clear all resets every filter back to its default — the range,
/// the accounts, the search, the three Options toggles, the category and
/// month selection and the drill-down — in one click, and then hides
/// itself with the chips row.
#[gpui::test]
fn clear_all_resets_every_filter_and_hides_itself(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });

    // A selection that carries the category, the month and the
    // drill-down level in one click…
    let (food, groceries) = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        let food = d
            .summary_rows()
            .iter()
            .find(|r| r.name == "Food")
            .unwrap()
            .id
            .unwrap();
        let groceries = d
            .categories
            .iter()
            .find(|p| p.name == "Food")
            .unwrap()
            .children
            .iter()
            .find(|c| c.name == "Groceries")
            .unwrap()
            .id;
        (food, groceries)
    });
    expand_summary_parent(cx, handle, &shell, food);
    click_summary_cell(cx, handle, &shell, Some(groceries), Some("2026-04"));
    // …plus the row's own controls: a custom range, a search and two of
    // the Options toggles. ("Uncategorized only" refuses a category
    // selection, so it stays off here; the next test covers it.)
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::Custom, window, cx);
            d.apply_search("STORE", window, cx);
            d.set_include_transfers(true, window, cx);
            d.set_show_hidden(true, window, cx);
        });
    });
    wait_until(cx, "every filter is set", |cx| {
        let d = dash(cx, &shell).read(cx);
        matches!(d.filter.date_range, DateRange::Custom { .. })
            && d.filter.search == "STORE"
            && d.filter.summary_month.as_deref() == Some("2026-04")
            && d.filter.chart_parent == Some(food)
            && d.filter.include_transfers
            && d.filter.show_hidden
    });
    draw_frame(cx, handle);
    {
        let mut window = VisualTestContext::from_window(handle, cx);
        assert!(window.debug_bounds("filters:clear-all").is_some());
        assert!(window.debug_bounds("filters:chips").is_some());
    }

    click(cx, handle, "filters:clear-all");
    wait_until(cx, "every filter is back to its default", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter == DashboardFilter::default()
            && d.filter.date_range == DateRange::default()
            && !d.view_filter.include_transfers
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        // One assertion per filter.
        assert_eq!(d.filter.date_range, DateRange::default());
        assert!(
            d.filter.excluded_accounts.is_empty(),
            "all accounts back in"
        );
        assert!(d.filter.search.is_empty());
        assert!(
            d.search_input.read(cx).value().is_empty(),
            "the box empties with the term"
        );
        assert!(!d.filter.include_transfers);
        assert!(!d.filter.uncategorized_only);
        assert!(!d.filter.show_hidden);
        assert!(d.filter.legend_selection.is_none());
        assert!(d.filter.summary_month.is_none());
        assert!(d.filter.chart_parent.is_none(), "the drill-down resets");
    });
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("filters:clear-all").is_none(),
        "at the default there is nothing to clear, so the button hides"
    );
    assert!(window.debug_bounds("filters:chips").is_none());
}

/// 3.46: the empty state's "Clear filters" link is the same reset — it
/// calls `clear_all_filters` and the whole filter set returns to default.
#[gpui::test]
fn the_no_matches_link_clears_every_filter(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    set_uncategorized_only(cx, handle, &shell, true);
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.apply_search("zzz-no-match", window, cx);
        });
    });
    wait_until(cx, "nothing matches", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.row_count == 0 && d.dashboard_body() == DashboardBody::NoMatches
    });

    // The link paints below the fold on the page scroller; scroll it in.
    cx.update(|cx| {
        dash(cx, &shell).update(cx, |d, _cx| {
            d.page_scroll.set_offset(point(px(0.), px(-400.)));
        });
    });
    click(cx, handle, "dashboard:no-matches-clear");
    wait_until(cx, "the link resets every filter", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter == DashboardFilter::default()
            && d.filter.date_range == DateRange::default()
            && !d.view_filter.include_transfers
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert!(d.filter.search.is_empty());
        assert!(d.search_input.read(cx).value().is_empty());
        assert!(!d.filter.uncategorized_only);
    });
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("filters:clear-all").is_none(),
        "the row's Clear all agrees: the filters are at their default"
    );
}
