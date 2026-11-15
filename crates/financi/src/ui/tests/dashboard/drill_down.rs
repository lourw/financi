//! Chart legend clicks: drilling into a category and clearing the drill.

use super::*;

#[gpui::test]
fn clicking_a_parent_drills_the_chart_and_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));

    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "chart drilled to Food's children", |cx| {
        shell.read(cx).dashboard.filter.chart_parent == Some(food)
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
            shell.read(cx).dashboard.filter.legend_selection,
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
        shell.read(cx).dashboard.filter.chart_parent == Some(food)
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_all_categories(window, cx));
    });
    wait_until(cx, "back to parents and all rows", |cx| {
        shell.read(cx).dashboard.filter.chart_parent.is_none()
            && shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).dashboard.filter.legend_selection, None);
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
        shell.read(cx).dashboard.filter.chart_parent == Some(food)
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
            shell.read(cx).dashboard.filter.legend_selection,
            Some(LegendSelection::Category(groceries))
        );
        // The chart stays on the children.
        assert_eq!(shell.read(cx).dashboard.filter.chart_parent, Some(food));
    });

    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "filter cleared, children still shown", |cx| {
        shell.read(cx).dashboard.filter.legend_selection.is_none()
            && shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| assert_eq!(shell.read(cx).dashboard.filter.chart_parent, Some(food)));
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
            shell.read(cx).dashboard.filter.legend_selection,
            Some(LegendSelection::Uncategorized)
        );
        // The pseudo-row never drills the chart.
        assert!(shell.read(cx).dashboard.filter.chart_parent.is_none());
        assert_eq!(shell.read(cx).dashboard.chart_rows.len(), 3);
    });

    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "all rows back", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
    cx.update(|cx| assert!(shell.read(cx).dashboard.filter.legend_selection.is_none()));
}

/// 3.65: a legend click paints the row's highlight in the very next frame,
/// while the refresh it started is still queued behind a blocked worker;
/// only the filtered rows wait for the answer.
#[gpui::test]
fn legend_row_paints_selected_before_the_answer(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table and legend loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4 && shell.read(cx).dashboard.chart_rows.len() == 3
    });
    let done = block_worker(cx, &shell);
    let (food, ix) = cx.update(|cx| {
        let food = top_category_id(&shell, cx, "Food");
        let ix = shell
            .read(cx)
            .dashboard
            .chart_rows
            .iter()
            .position(|r| r.id == Some(food))
            .unwrap();
        (food, ix)
    });
    click_chart_legend(cx, handle, &shell, Some(food));
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("chart:legend-row:{ix}:active")),
        "the row paints selected while the refresh it started is still in flight"
    );
    assert_eq!(
        cx.update(|cx| shell.read(cx).dashboard.row_count),
        4,
        "only the data waits for the DB"
    );
    drop(done);
    wait_until(cx, "Food's two rows arrive with the answer", |cx| {
        shell.read(cx).dashboard.row_count == 2
    });
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
        shell.read(cx).dashboard.filter.chart_parent == Some(food)
            && shell.read(cx).dashboard.filter.legend_selection
                == Some(LegendSelection::Category(food))
    });

    // Actions dispatch along the focus path, so focus the search input
    // first — like a user who clicked into the window. The real Esc
    // keybinding (registered in `ui::init`) must map the keystroke to
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
        shell.read(cx).dashboard.filter.chart_parent.is_none()
            && shell.read(cx).dashboard.filter.legend_selection.is_none()
            && shell.read(cx).dashboard.row_count == 4
    });
}

/// 3.65: Esc repaints in the very next frame too. With the worker held on
/// the refresh Esc starts, one frame must already show the legend row no
/// longer highlighted — only the filtered rows wait for the answer.
#[gpui::test]
fn escape_clears_the_legend_highlight_before_the_answer(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table and legend loaded", |cx| {
        shell.read(cx).dashboard.row_count == 4 && shell.read(cx).dashboard.chart_rows.len() == 3
    });
    // The `Uncategorized` pseudo-row carries the highlight without drilling
    // the chart, so its selector stays visible throughout.
    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "Uncategorized selected", |cx| {
        shell.read(cx).dashboard.filter.legend_selection == Some(LegendSelection::Uncategorized)
            && shell.read(cx).dashboard.row_count == 1
    });
    let ix = cx.update(|cx| {
        shell
            .read(cx)
            .dashboard
            .chart_rows
            .iter()
            .position(|r| r.id.is_none())
            .unwrap()
    });
    let done = block_worker(cx, &shell);

    // The real Esc binding, focused like a user who clicked into the window
    // (see `escape_clears_the_drill_and_the_selection`).
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
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, format!("chart:legend-row:{ix}:active")),
        "the highlight is gone in the frame right after Esc, while the \
         refresh it started is still in flight"
    );
    assert_eq!(
        cx.update(|cx| shell.read(cx).dashboard.row_count),
        1,
        "only the data waits for the DB"
    );
    drop(done);
    wait_until(cx, "all rows back with the answer", |cx| {
        shell.read(cx).dashboard.row_count == 4
    });
}

// -- Responsive charts (3.15) -------------------------------------------
