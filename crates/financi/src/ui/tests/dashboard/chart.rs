//! Chart rendering: the donut's slices, its hole total, the tooltip and
//! legend key, and the measured donut/legend layout (3.99).

use super::*;

/// The legend draws no numbers since 3.31, but the model rows still
/// carry each parent's amount and share — that is what the slice and
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

/// 3.99: the range's total sits in the donut's hole — the amount over a
/// muted "Expense" at the top level, and while drilled the parent's
/// total (the sum of its children's rows).
#[gpui::test]
fn the_donut_shows_the_range_total_in_its_hole(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    settle(cx, handle);
    let total = crate::flow::money(2000);
    assert!(
        paints(cx, handle, format!("chart:donut-center:{total}")),
        "the hole shows the range total",
    );

    // Drilled into Food, the hole carries the parent's total instead:
    // the children (Groceries + Restaurants) sum to 1500.
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
    let parent_total = crate::flow::money(1500);
    assert!(
        paints(cx, handle, format!("chart:donut-center:{parent_total}")),
        "the drilled hole shows the parent's total",
    );
    assert!(
        !paints(cx, handle, format!("chart:donut-center:{total}")),
        "the range total leaves the hole once drilled",
    );
}

/// 3.99: the donut draws the largest [`PIE_TOP_N`] categories and folds
/// the tail into one muted "Other" slice. The slices are the `chart_rows`
/// in draw order (largest first), and they still sum to the range total.
#[gpui::test]
fn the_donut_slices_are_the_top_eight_plus_other(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    set_twelve_categories(cx, &shell);
    let slices = cx.update(|cx| {
        dash(cx, &shell).read(cx).chart_slices(
            &Default::default(),
            Default::default(),
            gpui::black(),
        )
    });
    assert_eq!(
        slices.len(),
        PIE_TOP_N + 1,
        "twelve categories draw {PIE_TOP_N} named slices plus Other"
    );
    let named: Vec<&str> = slices
        .iter()
        .filter(|s| !s.is_other)
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(
        named,
        [
            "Cat 00", "Cat 01", "Cat 02", "Cat 03", "Cat 04", "Cat 05", "Cat 06", "Cat 07"
        ],
        "the largest eight, biggest first"
    );
    let other = slices.last().expect("a last slice");
    assert!(other.is_other);
    assert_eq!(other.name.as_str(), OTHER_NAME);
    // Cat 08..Cat 11 are 40_000 + 30_000 + 20_000 + 10_000.
    assert_eq!(other.cents, 100_000);
    let slice_total: i64 = slices.iter().map(|s| s.cents).sum();
    assert_eq!(slice_total, 780_000, "Other keeps the tail's spend");
}

/// 3.99: "Other" is a legend chip too — a muted swatch, the merged
/// amount in its tooltip, and the member names the slice holds. It is
/// not a click target: clicking it drills and selects nothing.
#[gpui::test]
fn the_other_chip_lists_its_members_and_ignores_clicks(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    set_twelve_categories(cx, &shell);
    resize_and_draw(cx, handle, 1200., 720.);
    settle(cx, handle);
    assert!(
        paints(cx, handle, "chart:legend-other"),
        "the Other chip paints beside the named chips"
    );

    // Its tooltip lists the categories the slice holds.
    let mut window = VisualTestContext::from_window(handle, cx);
    let other = window
        .debug_bounds("chart:legend-other")
        .expect("the Other chip paints");
    window.simulate_mouse_move(other.center(), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(
            cx,
            handle,
            "chart:legend-other-tooltip:Cat 08, Cat 09, Cat 10, Cat 11",
        ),
        "the Other tooltip lists its members",
    );

    // And a click is a no-op: no drill, no selection.
    click(cx, handle, "chart:legend-other");
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.filter.legend_selection, None);
        assert_eq!(d.filter.chart_parent, None);
    });
}

/// 3.46 carried to the donut: selecting a category keeps its slice at
/// full colour and dims the others (and `Other`) to about a third.
#[gpui::test]
fn selecting_a_category_dims_the_other_slices(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    set_twelve_categories(cx, &shell);
    dash(cx, &shell).update(cx, |d, cx| {
        d.filter.legend_selection = Some(LegendSelection::Category(3));
        cx.notify();
    });
    // An empty id→index map and palette, so every category paints
    // `muted` and only the dimming changes the alpha.
    let slices = cx.update(|cx| {
        dash(cx, &shell).read(cx).chart_slices(
            &Default::default(),
            Default::default(),
            gpui::black(),
        )
    });
    assert_eq!(
        slices.len(),
        PIE_TOP_N + 1,
        "a selection dims, never filters"
    );
    for s in &slices {
        let want = if s.name == "Cat 03" { 1.0 } else { 0.35 };
        assert!(
            (s.color.a - want).abs() < 0.01,
            "{} alpha {}, want {want}",
            s.name,
            s.color.a
        );
    }
}

/// 3.99: a category whose net is ≤ 0 — refunds can net a category out —
/// gets no slice; the positive ones do.
#[gpui::test]
fn categories_that_net_nothing_have_no_slice(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    dash(cx, &shell).update(cx, |d, cx| {
        d.chart_rows = vec![
            CategorySpend {
                id: Some(1),
                name: "Food".into(),
                spend_cents: 1000,
                share: 2.0,
            },
            CategorySpend {
                id: Some(2),
                name: "Refunds".into(),
                spend_cents: -500,
                share: -1.0,
            },
            CategorySpend {
                id: Some(3),
                name: "Zero".into(),
                spend_cents: 0,
                share: 0.0,
            },
        ];
        cx.notify();
    });
    let slices = cx.update(|cx| {
        dash(cx, &shell).read(cx).chart_slices(
            &Default::default(),
            Default::default(),
            gpui::black(),
        )
    });
    let names: Vec<&str> = slices.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Food"], "only positive nets draw a slice");
}

/// An empty range shows the "No spending in this range" state: no
/// donut, no legend.
#[gpui::test]
fn an_empty_range_draws_no_donut_and_no_legend(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    dash(cx, &shell).update(cx, |d, cx| {
        d.chart_rows.clear();
        cx.notify();
    });
    resize_and_draw(cx, handle, 1200., 720.);
    settle(cx, handle);
    assert!(!paints(cx, handle, "chart:plot"), "no donut without rows");
    assert!(
        !paints(cx, handle, "chart:legend"),
        "no legend without rows"
    );
}

/// 3.36 kept, 3.99 reshaped: each chip is still just swatch and name —
/// no amount, no share — and in the donut's side-by-side layout the
/// legend sits to the right of the donut, inside the chart panel.
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
    let plot = window.debug_bounds("chart:plot").expect("the donut paints");
    let panel = window
        .debug_bounds("panel:chart")
        .expect("the chart panel paints");

    // 3.99: the content area is wider than tall at 1200 × 720, so the
    // legend sits beside the donut, not under it.
    assert!(
        f32::from(legend.origin.x) >= f32::from(plot.origin.x + plot.size.width),
        "the legend x {:?} is not right of the donut's right edge {:?}",
        legend.origin.x,
        f32::from(plot.origin.x + plot.size.width),
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

/// 3.86, 3.99: the donut/legend plan comes from the measured content
/// area. The stored size matches the painted `chart:plot-area`, the
/// donut is a square of the area's height, and the legend gets the rest
/// of the width to its right.
#[gpui::test]
fn the_donut_and_legend_plan_off_the_measured_area(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    settle(cx, handle);

    let (stored_w, stored_h) = cx.update(|cx| dash(cx, &shell).read(cx).chart_area());
    let mut window = VisualTestContext::from_window(handle, cx);
    let area = window
        .debug_bounds("chart:plot-area")
        .expect("the measured area paints");
    assert!(
        (f32::from(area.size.width) - stored_w).abs() <= 0.5
            && (f32::from(area.size.height) - stored_h).abs() <= 0.5,
        "the probe stored {stored_w:.1} × {stored_h:.1} but the area \
         painted {:?} × {:?}",
        area.size.width,
        area.size.height,
    );
    assert!(
        stored_w > stored_h,
        "the 1200 × 720 area is wider than tall"
    );

    let plot = window.debug_bounds("chart:plot").expect("the donut paints");
    // A square of the area's height.
    assert!(
        (f32::from(plot.size.width) - f32::from(plot.size.height)).abs() <= 0.5,
        "the donut box is {:?} × {:?}, not a square",
        plot.size.width,
        plot.size.height,
    );
    assert!(
        (f32::from(plot.size.height) - stored_h).abs() <= 0.5,
        "the donut is {:?} tall, not the area's {stored_h:.1} px",
        plot.size.height,
    );
    assert!(
        f32::from(plot.size.height) >= DONUT_MIN - 0.5,
        "the donut is under the {DONUT_MIN} px floor: {:?}",
        plot.size.height,
    );
}

/// 3.99: a taller-than-wide content area stacks the legend under the
/// donut, capped at two lines' height. A tall, narrow window makes the
/// dock's chart cell taller than wide; the probe measures it and the
/// next frame paints the stacked form. (Forcing `set_plot_size` cannot
/// work: the probe restores the real size within the same settle.)
#[gpui::test]
fn a_tall_area_stacks_the_legend_under_the_donut(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 900., 1600.);
    settle(cx, handle);

    let (w, h) = cx.update(|cx| dash(cx, &shell).read(cx).chart_area());
    assert!(
        w > 0. && h > w,
        "the tall window's chart area must be taller than wide, got {w} × {h}"
    );
    let mut window = VisualTestContext::from_window(handle, cx);
    let plot = window.debug_bounds("chart:plot").expect("the donut paints");
    let legend = window
        .debug_bounds("chart:legend")
        .expect("the legend paints");
    assert!(
        f32::from(legend.origin.y) >= f32::from(plot.origin.y + plot.size.height) - 0.5,
        "the legend top y {:?} is not under the donut bottom {:?}",
        legend.origin.y,
        f32::from(plot.origin.y + plot.size.height),
    );
    assert!(
        f32::from(legend.size.height) <= LEGEND_MAX_H + 0.5,
        "the stacked legend is {:?} tall, over the {LEGEND_MAX_H} two-line cap",
        legend.size.height,
    );
    // The stacked donut takes the area's width, capped by the height
    // left after the legend; at least the floor's worth across.
    assert!(
        f32::from(plot.size.width) <= w + 0.5 && f32::from(plot.size.width) >= DONUT_MIN - 0.5,
        "the stacked donut is {:?} wide, area {w} wide",
        plot.size.width,
    );
}

/// 3.99: the pure donut/legend split — unmeasured areas show the legend
/// alone with every chip, a wider-than-tall area puts the donut left as
/// a square of the height, a tall area stacks, and under the floor the
/// legend takes the panel by itself.
#[test]
fn chart_layout_switches_on_the_measured_area() {
    let chips = vec![100.; 3];
    // Unmeasured: legend alone, no flash of a donut that will move.
    let l = chart_layout(0., 0., &chips, 14.);
    assert!(!l.side_by_side);
    assert_eq!(l.donut, 0.);
    assert_eq!(
        legend_visible(&chips, l.legend_w, LEGEND_GAP_X, l.legend_lines, 8.),
        None
    );

    // Wider than tall: the donut is a square of the height, the legend
    // wraps to its right with the leftover width.
    let l = chart_layout(600., 300., &chips, 14.);
    assert!(l.side_by_side);
    assert_eq!(l.donut, 300.);
    assert!((l.legend_w - (600. - 300. - DONUT_GAP)).abs() < 0.01);
    assert!(
        l.legend_lines > LEGEND_LINES,
        "beside the donut the legend may use more lines than the stacked cap: {}",
        l.legend_lines
    );

    // Wide but too short for the donut floor: legend alone, full width.
    let l = chart_layout(600., DONUT_MIN - 10., &chips, 14.);
    assert_eq!(l.donut, 0.);
    assert_eq!(l.legend_w, 600.);

    // Taller than wide: stacked, two capped lines, the donut gets what
    // is left and stays at or above the floor.
    let l = chart_layout(300., 600., &chips, 14.);
    assert!(!l.side_by_side);
    assert_eq!(l.legend_lines, LEGEND_LINES);
    assert!(l.donut >= DONUT_MIN && l.donut <= 300., "donut {}", l.donut);

    // Stacked but too short once the legend takes its lines: legend alone.
    let l = chart_layout(150., 170., &chips, 14.);
    assert_eq!(l.donut, 0.);

    // Radii: the hole is 60 % of the outer radius, and the outer one
    // stays clear of the box edge for the hover lift.
    let (inner, outer) = donut_radii(200.);
    assert!(
        outer < 100.,
        "the outer radius {outer} leaves no room for the hover lift"
    );
    assert!((inner - outer * 0.6).abs() < 0.01);
}

/// 3.31: hovering a legend row shows the slice's own tooltip text — full
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

/// 3.36, 3.99: chips past the lines the legend's measured width fits are
/// not built at all — a clipped-but-laid-out chip would still answer a
/// click — and a "+N more" chip lists them instead. The plan is the one
/// the view ran for the same measured area.
#[gpui::test]
fn the_legend_overflows_into_a_more_chip(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    dash(cx, &shell).update(cx, |d, cx| {
        d.chart_rows = (0..48)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Category {i:02} long name"),
                spend_cents: (i + 1) * 10_000,
                share: (i + 1) as f64 / 1176.,
            })
            .collect();
        cx.notify();
    });
    resize_and_draw(cx, handle, 1200., 720.);
    settle(cx, handle);

    // The same plan the view ran, recomputed from the measured state:
    // the capped-width chips plus "Other" must overflow the legend's
    // column beside the donut.
    let (shown, hidden, names) = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        let char_xs = d.chart_char_px * 12. / 10.;
        let mut widths: Vec<f32> = d
            .chart_rows
            .iter()
            .map(|r| legend_chip_w(r.name.chars().count(), char_xs))
            .collect();
        let has_other = d.chart_rows.iter().filter(|r| r.spend_cents > 0).count() > PIE_TOP_N;
        if has_other {
            widths.push(legend_chip_w(OTHER_NAME.chars().count(), char_xs));
        }
        let (w, h) = d.chart_area();
        let layout = chart_layout(w, h, &widths, d.chart_label_h);
        let shown = legend_visible(
            &widths,
            layout.legend_w,
            LEGEND_GAP_X,
            layout.legend_lines,
            char_xs,
        )
        .unwrap_or(widths.len());
        let names = d
            .chart_rows
            .iter()
            .map(|r| r.name.as_str())
            .chain(has_other.then_some(OTHER_NAME))
            .enumerate()
            .filter(|(ix, _)| *ix >= shown)
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
            .join(", ");
        (shown, widths.len() - shown, names)
    });
    assert!(
        (1..48).contains(&shown) && hidden > 0,
        "capped chips plus Other must overflow the legend beside the \
         donut at 1200 px: {shown} shown, {hidden} hidden"
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

    // Hovering "+N more" lists the hidden names.
    let mut window = VisualTestContext::from_window(handle, cx);
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
        "the more chip's tooltip lists the hidden categories: shown \
         {shown}, hidden {hidden}, names {names:?}"
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

/// The worst case — 25 categories with long names — draws without
/// panicking at the minimum and at a large window, and the donut still
/// shows [`PIE_TOP_N`] named slices plus `Other`.
#[gpui::test]
fn the_chart_renders_long_names_and_many_categories(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
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
    let slices = cx.update(|cx| {
        dash(cx, &shell).read(cx).chart_slices(
            &Default::default(),
            Default::default(),
            gpui::black(),
        )
    });
    assert_eq!(
        slices.len(),
        PIE_TOP_N + 1,
        "25 categories: eight named plus Other"
    );
    draw_frame(cx, handle);
    resize_and_draw(cx, handle, 900., 600.);
    settle(cx, handle);
    resize_and_draw(cx, handle, 2560., 1440.);
    settle(cx, handle);
}

/// Replace the seeded rows with twelve parents, `Cat 00` (120 000) down
/// to `Cat 11` (10 000) — enough to exercise the top-eight merge.
fn set_twelve_categories(cx: &mut TestAppContext, shell: &Entity<Shell>) {
    dash(cx, shell).update(cx, |d, cx| {
        d.chart_rows = (0..12)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Cat {i:02}"),
                spend_cents: (12 - i) * 10_000,
                share: (12 - i) as f64 / 78.,
            })
            .collect();
        cx.notify();
    });
}
