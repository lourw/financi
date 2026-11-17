//! Legend selection state: stale selections and exclusivity with filters.

use super::*;

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

#[gpui::test]
fn a_filter_change_drops_the_stale_legend_selection(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows_split);
    wait_until(cx, "four chart rows", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 4
    });
    let gifts = cx.update(|cx| top_category_id(&shell, cx, "Gifts"));

    // Gifts has no children: the drill snaps back to the top level while
    // the selection (and the table filter) stay on Gifts.
    click_chart_legend(cx, handle, &shell, Some(gifts));
    wait_until(cx, "Gifts selected, table filtered", |cx| {
        dash(cx, &shell).read(cx).filter.legend_selection == Some(LegendSelection::Category(gifts))
            && dash(cx, &shell).read(cx).filter.chart_parent.is_none()
            && dash(cx, &shell).read(cx).row_count == 1
    });

    // "This month" drops the October Gifts spend: the stale selection must
    // go with it, and the table must widen back to the remaining rows.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::ThisMonth, window, cx)
        });
    });
    wait_until(cx, "selection and drill cleared, rows back", |cx| {
        dash(cx, &shell).read(cx).filter.legend_selection.is_none()
            && dash(cx, &shell).read(cx).filter.chart_parent.is_none()
            // Four rows remain this month: two Food, one Transport, one
            // uncategorized (the October Gifts row is out of range).
            && dash(cx, &shell).read(cx).row_count == 4
    });
    cx.update(|cx| {
        let names: Vec<&str> = dash(cx, &shell)
            .read(cx)
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
        dash(cx, &shell).read(cx).row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));

    // 3.46: the toggle narrows the whole page, so the chart keeps only
    // the Uncategorized legend row while it is on.
    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "only the uncategorized row", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.row_count == 1 && d.chart_rows.iter().all(|r| r.id.is_none())
    });
    set_uncategorized_only(cx, handle, &shell, false);
    wait_until(cx, "the chart is whole again", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });

    // A category legend row wins: the checkbox goes off, the table shows
    // the category again.
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "Food filters the table", |cx| {
        !dash(cx, &shell).read(cx).filter.uncategorized_only
            && dash(cx, &shell).read(cx).filter.legend_selection
                == Some(LegendSelection::Category(food))
            && dash(cx, &shell).read(cx).row_count == 2
    });

    // Toggling the checkbox back on clears the category selection (the
    // chart stays drilled) instead of stacking both clauses.
    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "uncategorized only again", |cx| {
        // 3.46: drilled into Food with the toggle on, the child chart
        // would be empty (no uncategorized rows live under a named
        // category), so the 3.56 revalidation un-drills and the top-level
        // chart keeps only its Uncategorized row.
        let d = dash(cx, &shell).read(cx);
        d.filter.uncategorized_only
            && d.filter.legend_selection.is_none()
            && d.row_count == 1
            && d.filter.chart_parent.is_none()
            && d.chart_rows.iter().all(|r| r.id.is_none())
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        assert!(rows.iter().all(|r| r.category_id.is_none()));
    });
}

// ------------------------------------------------------- 3.6 monthly summary
