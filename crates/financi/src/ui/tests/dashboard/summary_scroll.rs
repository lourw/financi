//! Summary grid scrolling: pinned columns, swipe and range changes.

use super::*;

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
        dash(cx, shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });
    wait_until(cx, "summary covers all the seeded months", |cx| {
        dash(cx, shell).read(cx).summary.months.len() == months
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
        dash(cx, &shell).read(cx).summary.months.len() == 24
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
    let off0 = cx.update(|cx| dash(cx, &shell).read(cx).summary_scroll.offset().x);
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
    let off1 = cx.update(|cx| dash(cx, &shell).read(cx).summary_scroll.offset().x);
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

    // Vertical scrolling still works over the month zone — 3.45: the
    // wheel bubbles to the page's scroller (the summary is content-height
    // now, it no longer virtualizes its rows), and the name column moves
    // with its row either way.
    let y_before = bounds(cx, handle, "summary:name-1").origin.y;
    wheel(cx, handle, viewport_centre(zone), 0., -40.);
    draw_frame(cx, handle);
    let y_after = bounds(cx, handle, "summary:name-1").origin.y;
    assert!(
        y_after < y_before,
        "the rows scroll vertically under the month zone, names included"
    );
}

/// 3.45: the summary spans the page's full width, so a 12-month range
/// shows nearly all of it — at least 8 whole month columns at 1600 px,
/// where the old beside-the-chart panel showed about five.
#[gpui::test]
fn a_full_width_summary_shows_most_of_a_twelve_month_range(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 12));
    resize_and_draw(cx, handle, 1600., 900.);
    set_all_time_covering(cx, handle, &shell, 12);
    wait_until(cx, "12 months in the grid", |cx| {
        dash(cx, &shell).read(cx).summary.months.len() == 12
    });
    draw_frame(cx, handle);
    let zone = bounds(cx, handle, "summary:months-0");
    let mut whole = 0;
    for col in 0..12 {
        let cell = bounds(cx, handle, format!("summary:cell:0:{col}"));
        if cell.origin.x >= zone.origin.x - px(1.) && cell.right() <= zone.right() + px(1.) {
            whole += 1;
        }
    }
    assert!(
        whole >= 8,
        "the full-width grid shows only {whole} whole month columns at \
         1600 px of a 12-month range (3.45 wants at least 8)",
    );
}

/// 3.30, 3.45: a horizontal swipe over the months scrolls the month strip
/// only. A y-only scroller remaps a horizontal wheel delta onto its own
/// axis, so the month viewports and the page scroller keep the swipe
/// horizontal (`restrict_scroll_to_axis`) — otherwise the same swipe that
/// moves the months also jumps the page.
#[gpui::test]
fn a_horizontal_swipe_does_not_jump_the_summary_rows_vertically(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 24));
    resize_and_draw(cx, handle, 1200., 720.);
    set_all_time_covering(cx, handle, &shell, 24);
    wait_idle(cx, &shell);
    draw_frame(cx, handle);
    let zone = bounds(cx, handle, "summary:months-0");

    // Wheel the page down first, so there is somewhere to jump back to.
    // 3.45: the wheel over the month zone rides the page scroller — the
    // second row is tracked, because a 40 px scroll carries the first
    // row away entirely.
    let y_top = bounds(cx, handle, "summary:name-1").origin.y;
    wheel(cx, handle, viewport_centre(zone), 0., -40.);
    draw_frame(cx, handle);
    let y_scrolled = bounds(cx, handle, "summary:name-1").origin.y;
    assert!(y_scrolled < y_top, "the page is scrolled down");

    // A horizontal swipe over the month zone moves the months…
    let x0 = cx.update(|cx| dash(cx, &shell).read(cx).summary_scroll.offset().x);
    wheel(cx, handle, viewport_centre(zone), 200., 0.);
    draw_frame(cx, handle);
    let x1 = cx.update(|cx| dash(cx, &shell).read(cx).summary_scroll.offset().x);
    assert_ne!(x1, x0, "the month strip scrolled sideways");

    // …and leaves the body exactly where it was.
    let y_after = bounds(cx, handle, "summary:name-1").origin.y;
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
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::Last12Months, window, cx)
        });
    });
    wait_until(cx, "12 months in the grid", |cx| {
        dash(cx, &shell).read(cx).summary.months.len() == 12
    });
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        (d.summary_scroll.offset().x, d.summary_scroll.max_offset().x)
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
    let scrolled = cx.update(|cx| dash(cx, &shell).read(cx).summary_scroll.offset().x);
    assert!(scrolled < max, "the strip is no longer at its right end");

    // …and change the range again: the latest month is beside `Total`.
    set_all_time_covering(cx, handle, &shell, 24);
    wait_until(cx, "24 months in the grid", |cx| {
        dash(cx, &shell).read(cx).summary.months.len() == 24
    });
    wait_idle(cx, &shell);
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        (d.summary_scroll.offset().x, d.summary_scroll.max_offset().x)
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
/// This runs at 1600 × 900 rather than 1200 × 720 so the month zone is
/// comfortably wider than three 88 px columns even before 3.45 widened
/// the summary to the full page; the point stays that a range that fits
/// has nothing to scroll.
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
        let d = dash(cx, &shell).read(cx);
        (d.summary_scroll.offset().x, d.summary_scroll.max_offset().x)
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
