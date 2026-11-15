//! Chart rendering: tooltips, labels at window sizes, the legend key.

use super::*;

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
