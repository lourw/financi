//! 3.93: the *Monthly spending by category* panel — `monthly_stacks`
//! unit tests (the GPUI-free shaping) and the panel's UI tests.

use super::*;
use crate::ui::dashboard::panels::stacked::{
    MonthStack, StackSegment, axis_amount, monthly_stacks,
};
use financi_db::categories::CategoryNode;
use financi_db::{MonthlySummary, MonthlySummaryRow};

// -- monthly_stacks unit tests -----------------------------------------

/// Food (Groceries, Restaurants), Transport (Bus) and the system
/// `Transfers` parent.
fn tree() -> Vec<CategoryNode> {
    let node = |id: i64, name: &str, children: Vec<CategoryNode>| CategoryNode {
        id,
        name: name.into(),
        kind: CategoryKind::Expense,
        is_system: false,
        children,
    };
    vec![
        node(
            1,
            "Food",
            vec![
                node(11, "Groceries", vec![]),
                node(12, "Restaurants", vec![]),
            ],
        ),
        node(2, "Transport", vec![node(21, "Bus", vec![])]),
        node(3, "Transfers", vec![]),
    ]
}

fn row(id: Option<i64>, name: &str, months: &[i64]) -> MonthlySummaryRow {
    MonthlySummaryRow {
        id,
        name: name.into(),
        months: months.to_vec(),
        total: months.iter().sum(),
    }
}

/// The summary shape `monthly_summary` produces: parents each followed
/// by their children (ordered by total over the range — Food, Transport,
/// then Transfers and Uncategorized tied at 200 by name), then `Income`,
/// `Spend` and `Net`. March has the most spend, April less, May none.
/// With transfers the `Spend` row is `[2100, 700, 0]`, without it
/// `[2000, 600, 0]`.
fn summary(with_transfers: bool) -> MonthlySummary {
    let spend = if with_transfers {
        [2100, 700, 0]
    } else {
        [2000, 600, 0]
    };
    let income = [210_500i64, 210_500, 0];
    let mut rows = vec![
        row(Some(1), "Food", &[1500, 600, 0]),
        row(Some(11), "Groceries", &[1000, 600, 0]),
        row(Some(12), "Restaurants", &[500, 0, 0]),
        row(Some(2), "Transport", &[300, 0, 0]),
        row(Some(21), "Bus", &[300, 0, 0]),
    ];
    if with_transfers {
        rows.push(row(Some(3), "Transfers", &[100, 100, 0]));
    }
    rows.push(row(None, "Uncategorized", &[200, 0, 0]));
    rows.push(row(None, "Income", &income));
    rows.push(row(None, "Spend", &spend));
    rows.push(row(
        None,
        "Net",
        &(0..3)
            .map(|ix| income[ix] - spend[ix])
            .collect::<Vec<i64>>(),
    ));
    MonthlySummary {
        months: ["2026-03", "2026-04", "2026-05"]
            .iter()
            .map(|m| m.to_string())
            .collect(),
        rows,
    }
}

#[test]
fn stack_segments_sum_to_the_spend_row_for_each_month() {
    let stacks = monthly_stacks(&summary(true), None, &tree());
    assert_eq!(stacks.len(), 3, "one bar per month");
    for stack in &stacks {
        let seg_sum: i64 = stack.segments.iter().map(|s| s.cents).sum();
        assert_eq!(
            stack.total, seg_sum,
            "{}: bar height {} is not the sum of its segments {seg_sum}",
            stack.month, stack.total
        );
    }
    assert_eq!(
        stacks.iter().map(|s| s.total).collect::<Vec<i64>>(),
        [2100, 700, 0],
        "the bar height is the month's `Spend` row"
    );
}

#[test]
fn income_net_and_child_rows_are_left_out_of_the_segments() {
    let stacks = monthly_stacks(&summary(true), None, &tree());
    let names: Vec<&str> = stacks[0].segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["Food", "Transport", "Transfers", "Uncategorized"],
        "parents plus Uncategorized last; no children, no total rows"
    );
    for stack in &stacks {
        for seg in &stack.segments {
            assert!(
                !["Groceries", "Restaurants", "Bus", "Income", "Spend", "Net"]
                    .contains(&seg.name.as_str()),
                "{} must not be a segment",
                seg.name
            );
        }
    }
}

#[test]
fn drilling_swaps_in_the_children_and_the_parents_row_is_the_height() {
    let stacks = monthly_stacks(&summary(true), Some(1), &tree());
    let names: Vec<&str> = stacks[0].segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Groceries", "Restaurants"]);
    assert_eq!(
        stacks.iter().map(|s| s.total).collect::<Vec<i64>>(),
        [1500, 600, 0],
        "the bar height is Food's own row"
    );
    // April: only Groceries still has spend.
    assert_eq!(stacks[1].segments.len(), 1);
    assert_eq!(stacks[1].segments[0].name.as_str(), "Groceries");
}

#[test]
fn transfers_appears_only_when_the_summary_includes_it() {
    let with = monthly_stacks(&summary(true), None, &tree());
    assert!(
        with[0]
            .segments
            .iter()
            .any(|s| s.name.as_str() == "Transfers"),
        "transfers included: the row is a segment"
    );
    let without = monthly_stacks(&summary(false), None, &tree());
    assert!(
        !without
            .iter()
            .any(|s| s.segments.iter().any(|g| g.name.as_str() == "Transfers")),
        "transfers excluded: no segment anywhere"
    );
    assert_eq!(
        without.iter().map(|s| s.total).collect::<Vec<i64>>(),
        [2000, 600, 0],
        "and the bar follows the (transfer-free) Spend row"
    );
}

#[test]
fn segment_order_is_stable_across_months() {
    let stacks = monthly_stacks(&summary(true), None, &tree());
    fn order(s: &MonthStack) -> Vec<&str> {
        s.segments
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<&str>>()
    }
    assert_eq!(
        order(&stacks[0]),
        ["Food", "Transport", "Transfers", "Uncategorized"]
    );
    // April is Groceries-only plus transfers — but the order is fixed by
    // the totals over the range, so the survivors keep their places.
    assert_eq!(order(&stacks[1]), ["Food", "Transfers"]);
    // May has no spend at all: the slot stays, empty.
    assert_eq!(stacks[2].segments, Vec::<StackSegment>::new());
    assert_eq!(stacks[2].total, 0);
}

#[test]
fn a_negative_month_clamps_to_an_empty_bar() {
    // A refund-heavy month: the Spend row goes negative, so the bar
    // clamps to zero and no segment paints.
    let mut s = summary(false);
    let n = s.rows.len();
    s.rows[n - 2].months[0] = -500; // Spend
    let stacks = monthly_stacks(&s, None, &tree());
    assert_eq!(stacks[0].total, 0);
    assert!(
        stacks[0].segments.iter().all(|g| g.cents > 0),
        "negative segments are dropped, never drawn below the axis"
    );
}

#[test]
fn a_summary_without_the_three_total_rows_yields_no_bars() {
    let s = MonthlySummary {
        months: vec!["2026-03".into()],
        rows: vec![row(Some(1), "Food", &[100])],
    };
    assert!(monthly_stacks(&s, None, &tree()).is_empty());
}

#[test]
fn axis_amounts_stay_compact() {
    assert_eq!(axis_amount(42), "$0.42");
    assert_eq!(axis_amount(100), "$1");
    assert_eq!(axis_amount(8_642), "$86");
    assert_eq!(axis_amount(99_999), "$999");
    assert_eq!(axis_amount(100_000), "$1k");
    assert_eq!(axis_amount(123_456), "$1.2k");
    assert_eq!(axis_amount(1_234_567), "$12.3k");
}

// -- panel UI tests ------------------------------------------------------

use super::summary::{seed_summary_rows, set_all_time, summary_parent_id};
use crate::ui::dashboard::panels::stacked::StackColPaint;

/// One parent, one spend per month for March–May 2026: bars of 1000,
/// 2000 and 1500 cents, April the tallest.
fn seed_three_months(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-01-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let alpha =
        financi_db::categories::create_parent(conn, "Alpha", CategoryKind::Expense).unwrap();
    for (line, (date, cents)) in [
        ("2026-03-10", -1000i64),
        ("2026-04-10", -2000),
        ("2026-05-10", -1500),
    ]
    .into_iter()
    .enumerate()
    {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line as i64,
                format!("stacks-hash-{line}"),
                date,
                cents,
                "MONTHLY ALPHA",
                alpha,
            ),
        )
        .unwrap();
    }
}

/// Switch to "All time" and wait for exactly the three seeded months.
fn set_all_time_three_months(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
) {
    with_window(cx, handle, |window, cx| {
        dash(cx, shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::AllTime, window, cx);
        });
    });
    wait_until(cx, "summary covers the three seeded months", |cx| {
        dash(cx, shell).read(cx).summary.months == ["2026-03", "2026-04", "2026-05"]
    });
}

/// 3.100: the monthly bars sit in the dock's top-right cell — one row
/// with the chart, to its right, at the same width; the summary shares
/// the chart's column below it.
#[gpui::test]
fn the_stacked_panel_paints_right_of_the_chart(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    draw_frame(cx, handle);
    let chart = bounds(cx, handle, "panel:chart");
    let stacked = bounds(cx, handle, "panel:stacked");
    let summary = bounds(cx, handle, "panel:summary");
    assert!(
        (f32::from(stacked.origin.y) - f32::from(chart.origin.y)).abs() <= 1.,
        "the stacked panel is not on the chart's row ({} vs {})",
        f32::from(stacked.origin.y),
        f32::from(chart.origin.y),
    );
    assert!(
        f32::from(stacked.origin.x) >= f32::from(chart.right()) - 1.,
        "the stacked panel is not right of the chart panel",
    );
    assert_eq!(
        stacked.size.width, chart.size.width,
        "both top-row cells are the same width"
    );
    assert_eq!(stacked.size.height, chart.size.height);
    assert!(
        summary.origin.y >= chart.origin.y + chart.size.height - px(1.),
        "the summary panel sits below the chart panel"
    );
    assert_eq!(
        summary.origin.x, chart.origin.x,
        "the summary shares the chart's column"
    );
}

/// Three seeded months paint three bars; the tallest fills the plot
/// (bar area = plot minus the 16 px label row and the 2 px gap) and the
/// others are proportional shares of it. The `$0` gridline sits on the
/// bars' baseline, not on the panel's bottom edge (3.93 review).
#[gpui::test]
fn three_months_paint_three_bars_and_the_tallest_fills_the_plot(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_months);
    set_all_time_three_months(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    draw_frame(cx, handle);
    let plot = bounds(cx, handle, "stacked:plot");
    // 3.98: the bar area is the plot minus the *measured* label row and
    // its 2 px gap — the same inset the grid and gutter use.
    let inset = cx.update(|cx| dash(cx, &shell).read(cx).stacked_bar_inset_h());
    let full = f32::from(plot.size.height) - inset;
    let grid = bounds(cx, handle, "stacked:grid");
    let tallest = bounds(cx, handle, "stacked:bar:1");
    let mut bar = |ix: usize| bounds(cx, handle, format!("stacked:bar:{ix}")).size.height;
    assert!(
        (f32::from(grid.bottom()) - f32::from(tallest.bottom())).abs() < 2.,
        "the $0 gridline shares the tallest bar's bottom edge: grid {grid:?}, bar {tallest:?}"
    );
    assert!(
        (f32::from(bar(1)) - full).abs() < 2.,
        "April's bar fills the plot height: {} vs {full}",
        bar(1)
    );
    assert!(
        (f32::from(bar(0)) - full / 2.).abs() < 2.,
        "March's 1000 ¢ is half of April's 2000 ¢: {}",
        bar(0)
    );
    assert!(
        (f32::from(bar(2)) - full * 0.75).abs() < 2.,
        "May's 1500 ¢ is three quarters of April's: {}",
        bar(2)
    );
}

/// Hovering a segment shows its tooltip; clicking it selects the
/// category and the month and filters the table to their intersection.
#[gpui::test]
fn clicking_a_segment_filters_the_table_to_category_and_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    draw_frame(cx, handle);
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    // March segments, bottom-up, are Food then Uncategorized; the widget
    // renders them top-down, so Food is child index 1 of 2.
    let si = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        let stacks = monthly_stacks(&d.summary, d.filter.chart_parent, &d.categories);
        let segs = &stacks[0].segments;
        segs.len() - 1 - segs.iter().position(|s| s.name == "Food").unwrap()
    });
    assert_eq!(si, 1);
    // The tooltip carries `Category · Month` and the exact amount.
    let mut window = VisualTestContext::from_window(handle, cx);
    let seg = window
        .debug_bounds(leaked(format!("stacked:seg:0:{si}")))
        .expect("March's Food segment paints");
    window.simulate_mouse_move(viewport_centre(seg), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("stacked:seg-tooltip:0:{si}")),
        "hovering the segment shows its tooltip"
    );
    let tip = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        let stacks = monthly_stacks(&d.summary, d.filter.chart_parent, &d.categories);
        let cols = d.stacked_paint(
            &stacks,
            &Default::default(),
            Default::default(),
            gpui::black(),
        );
        (
            cols[0].segments[0].tooltip_title.to_string(),
            cols[0].segments[0].tooltip_value.to_string(),
        )
    });
    assert_eq!(tip.0, "Food · Mar 2026");
    assert_eq!(tip.1, "$15.00 · 88.2%");

    click(cx, handle, format!("stacked:seg:0:{si}"));
    wait_until(cx, "only March's Food rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 2
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert!(matches!(
            d.filter.legend_selection,
            Some(LegendSelection::Category(id)) if id == food
        ));
        assert_eq!(d.filter.summary_month.as_deref(), Some("2026-03"));
        let mut descs: Vec<String> = table_rows(&shell, cx)
            .iter()
            .map(|r| r.description.clone())
            .collect();
        descs.sort();
        assert_eq!(descs, ["CORNER CAFE MAR", "GROCER STORE MAR"]);
    });
}

/// Clicking a month label selects the month alone — no category.
#[gpui::test]
fn clicking_a_month_label_selects_the_month_alone(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    draw_frame(cx, handle);
    click(cx, handle, "stacked:month-label:0");
    wait_until(cx, "only March's rows", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.row_count == 4
            && d.filter.summary_month.as_deref() == Some("2026-03")
            && d.filter.legend_selection.is_none()
    });
}

// -- 3.98: month labels are never cut off --------------------------------

/// One spend per month for `months` consecutive months ending March
/// 2026, all in one parent, amounts growing with the month.
fn seed_n_months(conn: &financi_db::rusqlite::Connection, months: usize) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-01-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let parent =
        financi_db::categories::create_parent(conn, "Alpha", CategoryKind::Expense).unwrap();
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
    for (ix, date) in dates.iter().enumerate() {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                ix as i64,
                format!("labels-hash-{ix}"),
                date,
                -1000 - ix as i64,
                "MONTHLY ALPHA",
                parent,
            ),
        )
        .unwrap();
    }
}

/// Switch to "All time" and wait for exactly `months` months.
fn set_all_time_n(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    months: usize,
) {
    with_window(cx, handle, |window, cx| {
        dash(cx, shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::AllTime, window, cx);
        });
    });
    wait_until(cx, "summary covers all the seeded months", |cx| {
        dash(cx, shell).read(cx).summary.months.len() == months
    });
}

/// 3.98 at `w × h`: every painted month label's box is at least as wide
/// as its text's shaped width (so nothing ellipsized), no two boxes
/// overlap, and every box sits inside the plot. Returns the shown
/// columns' indices.
fn check_labels_fit(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    w: f32,
    h: f32,
) -> Vec<usize> {
    resize_and_draw(cx, handle, w, h);
    let plot = bounds(cx, handle, "stacked:plot");
    let months: Vec<String> = cx.update(|cx| dash(cx, shell).read(cx).summary.months.clone());
    let shaped = |cx: &mut TestAppContext, text: String| {
        with_window(cx, handle, |window, _cx| {
            gpui_kit::base::plot::label::measure_text_width(&text.into(), px(12.), window)
        })
    };
    let mut shown: Vec<usize> = Vec::new();
    let mut prev_right: Option<f32> = None;
    for (ix, m) in months.iter().enumerate() {
        let sel = format!("stacked:month-label:{ix}");
        if !paints(cx, handle, &sel) {
            continue;
        }
        shown.push(ix);
        let b = bounds(cx, handle, sel);
        // 3.100: the strip scrolls sideways inside the plot (parked at
        // the latest month). A planned label scrolled past the viewport
        // keeps its layout box but paints nothing — only boxes that
        // meet the viewport take part in the checks below.
        let (b_left, b_right) = (f32::from(b.origin.x), f32::from(b.right()));
        if b_right <= f32::from(plot.origin.x) - 0.01 || b_left >= f32::from(plot.right()) + 0.01 {
            continue;
        }
        let text = crate::flow::month_label(m);
        let text_w = shaped(cx, text.clone());
        assert!(
            f32::from(b.size.width) >= text_w - 0.01,
            "month {ix} ({text}) paints {text_w} px of text in a {} px box",
            f32::from(b.size.width),
        );
        if let Some(right) = prev_right {
            assert!(
                b_left >= right - 0.01,
                "month {ix} ({text}) overlaps the previous label: {b_left} < {right}",
            );
        }
        prev_right = Some(b_right);
        // 3.100: horizontally, a label may hang over a scroll fold —
        // that is the strip's own scroll, not the plan cutting text.
        // Vertically it must stay inside the plot.
        assert!(
            f32::from(b.origin.y) >= f32::from(plot.origin.y) - 0.01
                && f32::from(b.bottom()) <= f32::from(plot.bottom()) + 0.01,
            "month {ix} ({text}) box {b:?} lies outside the plot {plot:?}",
        );
    }
    assert!(!shown.is_empty(), "at least one month label paints");
    shown
}

/// 3.98: three months at the minimum window — every label fits its own
/// third of the plot, and clicking one still selects its month.
#[gpui::test]
fn three_month_labels_fit_and_stay_clickable_at_1200x720(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_n_months(conn, 3));
    set_all_time_n(cx, handle, &shell, 3);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    let shown = check_labels_fit(cx, handle, &shell, 1200., 720.);
    assert_eq!(shown, [0, 1, 2], "three months all keep their labels");
    // The panel may sit partly below the fold at 720 px tall: ride the
    // page scroller until the label is on screen, then click it. The
    // strip parks at the latest month, so March (index 2) is on screen.
    let sel = "stacked:month-label:2";
    for _ in 0..10 {
        let b = bounds(cx, handle, sel);
        if f32::from(b.center().y) < 700. {
            break;
        }
        let mid = bounds(cx, handle, "panel:stacked");
        wheel(cx, handle, viewport_centre(mid), 0., -200.);
        draw_frame(cx, handle);
    }
    click(cx, handle, sel);
    wait_until(cx, "only March's rows", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.summary_month.as_deref() == Some("2026-03") && d.filter.legend_selection.is_none()
    });
}

/// 3.98: twelve months at the minimum window — the labels stay whole,
/// disjoint and inside the plot.
#[gpui::test]
fn twelve_month_labels_are_never_cut_off_at_1200x720(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_n_months(conn, 12));
    set_all_time_n(cx, handle, &shell, 12);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 12
    });
    check_labels_fit(cx, handle, &shell, 1200., 720.);
}

/// 3.98: a three-year range at the minimum window — the thinning plan
/// leaves gaps, and every label that does paint is whole, disjoint from
/// its neighbours (including the first and last, clamped inside the
/// plot) and clickable.
#[gpui::test]
fn thirty_six_month_labels_are_never_cut_off_at_1200x720(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_n_months(conn, 36));
    set_all_time_n(cx, handle, &shell, 36);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 36
    });
    let shown = check_labels_fit(cx, handle, &shell, 1200., 720.);
    assert!(shown.len() < 36, "a 36-month range thins its labels");
    assert!(
        shown.contains(&0) && shown.contains(&35),
        "the plan still shows the first and last months: {shown:?}"
    );
    // The strip is parked at the latest month; the last label is on
    // screen and still selects its month.
    let sel = "stacked:month-label:35";
    for _ in 0..10 {
        let b = bounds(cx, handle, sel);
        if f32::from(b.center().y) < 700. {
            break;
        }
        let mid = bounds(cx, handle, "panel:stacked");
        wheel(cx, handle, viewport_centre(mid), 0., -200.);
        draw_frame(cx, handle);
    }
    click(cx, handle, sel);
    wait_until(cx, "only March 2026's rows", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.summary_month.as_deref() == Some("2026-03") && d.filter.legend_selection.is_none()
    });
}

/// Selecting through a legend chip selects the category with no month
/// and no drill (3.93: this chart is not the drill's front door), and
/// the paint model dims every other segment to 35 % alpha.
#[gpui::test]
fn a_chip_selection_dims_the_other_segments(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    draw_frame(cx, handle);
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    // The chips run in first-seen order: Food, Uncategorized, Transport.
    click(cx, handle, "stacked:legend-row:0");
    wait_until(cx, "the table keeps only the Food rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert!(matches!(
            d.filter.legend_selection,
            Some(LegendSelection::Category(id)) if id == food
        ));
        assert!(
            d.filter.summary_month.is_none(),
            "a chip selects the category alone"
        );
        assert_eq!(d.filter.chart_parent, None, "a chip never drills");
    });
    let cols = |cx: &App| -> Vec<StackColPaint> {
        let d = dash(cx, &shell).read(cx);
        let stacks = monthly_stacks(&d.summary, d.filter.chart_parent, &d.categories);
        // Empty colours and palette, like the 3.46 chart test: only the
        // dimming moves the alpha.
        d.stacked_paint(
            &stacks,
            &Default::default(),
            Default::default(),
            gpui::black(),
        )
    };
    cx.update(|cx| {
        let cols = cols(cx);
        let alpha = |col: usize, name: &str| -> f32 {
            cols[col]
                .segments
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("{} is a segment", name))
                .color
                .a
        };
        assert!(
            (alpha(0, "Food") - 1.).abs() < 0.01,
            "the selection keeps its colour"
        );
        assert!((alpha(0, "Uncategorized") - 0.35).abs() < 0.01);
        assert!((alpha(1, "Food") - 1.).abs() < 0.01);
        assert!((alpha(1, "Transport") - 0.35).abs() < 0.01);
    });
}

/// A range with no spending shows the empty state instead of bars.
#[gpui::test]
fn a_range_with_no_spending_shows_the_empty_state(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        settings::set(conn, UI_DATE_RANGE, "custom:2020-01-01..2020-12-31").unwrap();
        seed_summary_rows(conn);
    });
    wait_until(cx, "nothing in the 2020 range", |cx| {
        let d = dash(cx, &shell).read(cx);
        shell.read(cx).total_visible == 7 && d.row_count == 0 && !d.summary_has_data()
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "stacked:empty"),
        "the panel shows 'No spending in this range'"
    );
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("stacked:plot").is_none(),
        "no bars paint in the empty state"
    );
}
