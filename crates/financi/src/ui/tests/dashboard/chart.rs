//! Chart rendering: tooltips, labels at window sizes, the legend key.

use super::*;

/// The legend draws no numbers since 3.31, but the model rows still
/// carry each parent's amount and share — that is what the bar and
/// legend-row tooltips show.
#[gpui::test]
fn chart_rows_carry_amounts_and_shares_for_the_tooltips(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    cx.update(|cx| {
        let rows = &dash(cx, &shell).read(cx).chart_rows;
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
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    // 3.86: the label plan reads the *measured* plot width, so the
    // probes must have settled before the plan says anything.
    settle(cx, handle);

    // 3.45: the plot spans the full page width now, so even at 900 px
    // three bands share enough of it to keep the whole 16-char label
    // budget — the truncation the old 45 %-wide plot needed is gone.
    // What the plan still tracks is the *measured* width: the plot grows
    // with the window even when the capped plan no longer shows it.
    resize_and_draw(cx, handle, 900., 720.);
    settle(cx, handle);
    let (small_chars, small_labels) = cx.update(|cx| dash(cx, &shell).read(cx).chart_label_plan());
    assert_eq!(small_labels, 3);
    assert_eq!(
        small_chars, CHART_LABEL_MAX_CHARS,
        "a full-width plot budgets three bands to the cap"
    );
    let small_plot = cx.update(|cx| dash(cx, &shell).read(cx).chart_plot_width());

    // A large window: every band keeps its label, and the measured plot
    // grew — the `chart:plot` probe re-measured after the resize.
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(2000.), px(900.)));
        window.bounds_changed(cx);
    });
    settle(cx, handle);
    let (big_chars, big_labels) = cx.update(|cx| dash(cx, &shell).read(cx).chart_label_plan());
    assert_eq!(big_labels, 3, "a wide window labels every band");
    assert_eq!(big_chars, CHART_LABEL_MAX_CHARS);
    let big_plot = cx.update(|cx| dash(cx, &shell).read(cx).chart_plot_width());
    assert!(big_plot > small_plot, "{big_plot} vs {small_plot}");

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
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    // 25 categories with long names — the worst case for the band axis.
    dash(cx, &shell).update(cx, |d, cx| {
        d.chart_rows = (0..25)
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
    // test window's width and at the minimum size. 3.45: across the
    // full-width plot all 25 bands keep a label (thinned characters);
    // the plan never squeezes them to nothing either way.
    settle(cx, _handle);
    let (chars, labels) = cx.update(|cx| dash(cx, &shell).read(cx).chart_label_plan());
    assert!((1..=25).contains(&labels), "{labels} labels for 25 bands");
    assert!((1..=CHART_LABEL_MAX_CHARS).contains(&chars));
    draw_frame(cx, _handle);
}

/// 3.36: the legend is a wrapping chip row *under* the plot, not a column
/// beside it. At 1200 × 720 it paints below the plot's bottom edge, stays
/// inside the chart panel, and leaves the plot the panel's full inner
/// width (3.45 added a checkbox and count per row without changing that).
/// Each chip is still just swatch and name (3.31): no amount, no share.
#[gpui::test]
fn the_legend_is_just_a_key(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    settle(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    let rows: Vec<(String, i64, f64)> = cx.update(|cx| {
        dash(cx, &shell)
            .read(cx)
            .chart_rows
            .iter()
            .map(|r| (r.name.to_string(), r.spend_cents, r.share))
            .collect()
    });
    assert_eq!(rows.len(), 3);

    let legend = window
        .debug_bounds("chart:legend")
        .expect("the legend paints");
    let plot = window.debug_bounds("chart:plot").expect("the plot paints");
    let panel = window
        .debug_bounds("panel:chart")
        .expect("the chart panel paints");

    // Below the plot: the legend's top is at or under the plot's bottom.
    assert!(
        f32::from(legend.origin.y) >= f32::from(plot.origin.y + plot.size.height),
        "the legend top y {:?} is not below the plot bottom {:?}",
        legend.origin.y,
        plot.origin.y + plot.size.height,
    );
    // Inside the panel.
    let (panel_left, panel_right, panel_bottom) = (
        f32::from(panel.origin.x),
        f32::from(panel.origin.x + panel.size.width),
        f32::from(panel.origin.y + panel.size.height),
    );
    assert!(
        f32::from(legend.origin.x) >= panel_left
            && f32::from(legend.origin.x + legend.size.width) <= panel_right + 0.5
            && f32::from(legend.origin.y + legend.size.height) <= panel_bottom + 0.5,
        "the legend {:?} spills out of the panel {:?}",
        legend,
        panel,
    );
    // The plot owns the panel's inner width: the panel is 16 px padding
    // and a 1 px border per side, so 34 px in all.
    let plot_w = f32::from(plot.size.width);
    assert!(
        (plot_w - (f32::from(panel.size.width) - 34.)).abs() <= 1.,
        "the plot is {plot_w:.1} wide, not the panel's inner width {:?}",
        panel.size.width,
    );
    assert!(
        plot_w >= 380.,
        "the plot is {plot_w:.1} wide at 1200 × 720 — with the legend \
         beside it, as 3.31 left it, it was ~240 px",
    );

    // Each chip draws its name and nothing else: no amount, no percentage.
    for (ix, (name, cents, share)) in rows.iter().enumerate() {
        assert!(
            paints(cx, handle, format!("chart:legend-text:{ix}:{name}")),
            "legend chip {ix} draws its name"
        );
        for gone in [crate::flow::money(*cents), format!("{:.1}%", share * 100.)] {
            assert!(
                !paints(cx, handle, format!("chart:legend-text:{ix}:{gone}")),
                "legend chip {ix} no longer draws {gone:?}",
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
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let value = cx.update(|cx| {
        let r = &dash(cx, &shell).read(cx).chart_rows[0];
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
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    dash(cx, &shell).update(cx, |d, cx| {
        d.chart_rows = (0..15)
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
    // 3.86: settle the probes, and read the measured per-character width.
    settle(cx, handle);
    let char_px = cx.update(|cx| dash(cx, &shell).read(cx).chart_char_px);
    let mut window = VisualTestContext::from_window(handle, cx);

    let plot = window.debug_bounds("chart:plot").expect("the plot paints");
    let panel = window
        .debug_bounds("panel:chart")
        .expect("the chart panel paints");
    let plan_w = cx.update(|cx| dash(cx, &shell).read(cx).chart_plot_width());
    assert!(
        (f32::from(plot.size.width) - plan_w).abs() <= 0.5,
        "the plan reads the plot's own measured bounds: the probe stored \
         {plan_w:.1}px, the container painted {:?} (3.86)",
        plot.size.width,
    );

    // Share labels: 3.45 gives the plot the full page width, so at
    // 1200 px fifteen bars have room for every label — and the plan
    // still comes out of the measured plot width. The clearance checks
    // below hold whatever the plan decides to draw.
    let plot_w = f32::from(plot.size.width);
    let labels = bar_label_plan(plot_w, 15, char_px);
    assert!((1..=15).contains(&labels), "{labels} labels for 15 bars");

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
        dash(cx, &shell).read(cx).chart_bars(
            plan_w,
            &Default::default(),
            Default::default(),
            gpui::black(),
        )
    });
    assert_eq!(bars.len(), 15);
    let wired_slots = bar_label_slots(15, bar_label_plan(plan_w, 15, char_px));
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
    let half = BAR_LABEL_MAX_CHARS as f32 * char_px / 2.;
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

/// 3.86: the label plan is derived from *measured* bounds. At 1200 × 900
/// and at 1600 × 900 the stored plot width equals the painted
/// `chart:plot` container's own width, the plan equals the budget computed
/// from that painted width and the measured per-character width, and
/// widening the window grows the measured plot — no
/// window-width-minus-constants arithmetic anywhere. 3.45: with the plot
/// full width six bands hold the capped character budget at both sizes,
/// so the test watches the *plot* grow rather than the capped budget.
#[gpui::test]
fn the_label_plan_follows_the_measured_plot_at_1200_and_1600(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    dash(cx, &shell).update(cx, |d, cx| {
        d.chart_rows = (0..6)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Cat {i}"),
                spend_cents: (i + 1) * 10_000,
                share: 0.1,
            })
            .collect();
        cx.notify();
    });
    let mut plans = Vec::new();
    for w in [1200., 1600.] {
        resize_and_draw(cx, handle, w, 900.);
        settle(cx, handle);
        let painted = {
            let mut window = VisualTestContext::from_window(handle, cx);
            f32::from(
                window
                    .debug_bounds("chart:plot")
                    .expect("the plot paints")
                    .size
                    .width,
            )
        };
        let (stored, chars, labels, char_px) = cx.update(|cx| {
            let page = dash(cx, &shell).read(cx);
            let (chars, labels) = page.chart_label_plan();
            (page.chart_plot_width(), chars, labels, page.chart_char_px)
        });
        assert!(
            (stored - painted).abs() <= 0.5,
            "at {w}px the probe stored {stored:.1}px but `chart:plot` \
             painted {painted:.1}px",
        );
        assert!(
            char_px > 0. && char_px < 12.,
            "measured char width {char_px}"
        );
        assert_eq!(
            (chars, labels),
            band_label_budget(painted, 6, char_px),
            "at {w}px the plan must be the budget of the measured width"
        );
        plans.push((stored, chars, labels));
    }
    // 3.45: full width, six bands get the capped 16-char budget at both
    // sizes — the budget no longer grows with the window at this band
    // count. What must still grow is the measured plot itself.
    assert!(
        plans[1].0 > plans[0].0,
        "a wider window measures more plot: {}px vs {}px",
        plans[1].0,
        plans[0].0,
    );
    assert_eq!(plans[0].1, CHART_LABEL_MAX_CHARS);
    assert_eq!(plans[1].1, CHART_LABEL_MAX_CHARS);
}

/// 3.36, 3.45: fifteen categories are more than two lines of chips can
/// carry, so the legend shows a prefix of them plus a "+N more" chip that
/// lists the rest — and the hidden chips are not built at all, so nothing
/// clipped still answers a click. The names are wide enough that even the
/// full-width plot cannot fit all fifteen chips in two lines.
#[gpui::test]
fn the_legend_caps_at_two_lines_with_more(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    dash(cx, &shell).update(cx, |d, cx| {
        d.chart_rows = (0..15)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Category {i:02} long name"),
                spend_cents: (i + 1) * 10_000,
                share: (i + 1) as f64 / 120.,
            })
            .collect();
        cx.notify();
    });
    resize_and_draw(cx, handle, 1200., 720.);
    settle(cx, handle);

    // The same plan the view ran, recomputed from the measured state:
    // fifteen chips must overflow two lines at this width.
    let (shown, hidden, names) = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        let char_xs = d.chart_char_px * 12. / 10.;
        let widths: Vec<f32> = d
            .chart_rows
            .iter()
            .map(|r| legend_chip_w(r.name.chars().count(), char_xs))
            .collect();
        let shown = legend_visible(
            &widths,
            d.chart_plot_width(),
            LEGEND_GAP_X,
            LEGEND_LINES,
            char_xs,
        )
        .unwrap_or(widths.len());
        let names = d
            .chart_rows
            .iter()
            .skip(shown)
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        (shown, widths.len() - shown, names)
    });
    assert!(
        (1..15).contains(&shown) && hidden > 0,
        "fifteen chips must not fit two lines at 1200 px: {shown} shown, \
         {hidden} hidden"
    );

    // The "+N more" chip paints with the right count; the last visible
    // chip paints and the first hidden one does not exist to paint.
    assert!(paints(cx, handle, format!("chart:legend-more:{hidden}")));
    assert!(paints(
        cx,
        handle,
        format!("chart:legend-row:{}", shown - 1)
    ));
    assert!(
        !paints(cx, handle, format!("chart:legend-row:{shown}")),
        "the first hidden chip must not be built, not merely clipped"
    );

    // Two lines, no more: the legend stays inside its height cap.
    let mut window = VisualTestContext::from_window(handle, cx);
    let legend = window
        .debug_bounds("chart:legend")
        .expect("the legend paints");
    assert!(
        f32::from(legend.size.height) <= LEGEND_MAX_H + 0.5,
        "the legend is {:?} tall, over the {LEGEND_MAX_H} two-line cap",
        legend.size.height,
    );

    // Hovering "+N more" lists the hidden names.
    let more = window
        .debug_bounds(leaked(format!("chart:legend-more:{hidden}")))
        .expect("the more chip paints");
    window.simulate_mouse_move(more.center(), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(
            cx,
            handle,
            format!("chart:legend-more-tooltip:{hidden}:{names}")
        ),
        "the more chip's tooltip lists the hidden categories"
    );

    // Clicking it does nothing: no drill, no selection.
    click(cx, handle, format!("chart:legend-more:{hidden}"));
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.filter.legend_selection, None);
        assert_eq!(d.filter.chart_parent, None);
    });
}

/// 3.36: the pure chip plan — everything shown when it fits, the longest
/// prefix plus "+N more" when it doesn't, and everything when the width
/// has not been measured yet (the first frame must not flash an empty
/// legend).
#[test]
fn legend_plan_fits_overflows_and_waits_for_measurement() {
    let widths = vec![50.; 10];
    // Fits: two lines of `50 + 12 + 50` each.
    assert_eq!(legend_lines(&widths[..4], 120., LEGEND_GAP_X), 2);
    assert_eq!(
        legend_visible(&widths[..4], 120., LEGEND_GAP_X, 2, 8.),
        None
    );
    // Overflows: only four chips of ten fit two lines with the more chip
    // after them (`+6 more` is 8 chars at 8 px plus padding).
    assert_eq!(legend_lines(&widths, 120., LEGEND_GAP_X), 5);
    let shown = legend_visible(&widths, 120., LEGEND_GAP_X, 2, 8.);
    assert_eq!(shown, Some(2));
    // Whatever it returns, the shown chips plus "+N more" fit the cap.
    let Some(v) = shown else {
        unreachable!("ten 50 px chips overflow 120 px");
    };
    let mut shown_widths = widths[..v].to_vec();
    shown_widths.push(legend_more_w(widths.len() - v, 8.));
    assert!(legend_lines(&shown_widths, 120., LEGEND_GAP_X) <= 2);
    // Unmeasured: show everything.
    assert_eq!(legend_visible(&widths, 0., LEGEND_GAP_X, 2, 8.), None);
    assert_eq!(legend_visible(&widths, -1., LEGEND_GAP_X, 2, 8.), None);
}

/// 3.36: while drilled, the "← All categories" chip is the first chip of
/// the row — before the child chips — and clicking it clears the drill
/// and the selection.
#[gpui::test]
fn the_all_categories_chip_is_first_and_clears_the_drill(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled into Food", |cx| {
        dash(cx, &shell).read(cx).filter.chart_parent == Some(food)
            && dash(cx, &shell)
                .read(cx)
                .chart_rows
                .iter()
                .any(|r| r.name == "Groceries")
    });
    settle(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    let all = window
        .debug_bounds("chart:legend-all")
        .expect("the all-categories chip paints while drilled");
    let first = window
        .debug_bounds("chart:legend-row:0")
        .expect("the first child chip paints");
    assert!(
        f32::from(all.origin.x) < f32::from(first.origin.x),
        "the all-categories chip is at x {:?}, not before the first \
         category chip at x {:?}",
        all.origin.x,
        first.origin.x,
    );

    click(cx, handle, "chart:legend-all");
    wait_until(cx, "drill and selection cleared", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.filter.chart_parent.is_none() && d.filter.legend_selection.is_none() && d.row_count == 4
    });
}
