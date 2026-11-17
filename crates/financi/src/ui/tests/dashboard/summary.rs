//! Summary grid: month and total rows, cell clicks and expansion.

use super::*;

/// 3.85: a cached view row carries its cells twice — raw cents (what
/// the sign-dependent colours read) and the drawn text. Tests that
/// check amounts read `.cents`; this builds the matching drawn strings.
pub(super) fn money_cells(values: &[i64]) -> Vec<gpui_kit::SharedString> {
    values
        .iter()
        .map(|&c| gpui_kit::SharedString::from(crate::flow::money(c)))
        .collect()
}

/// Two fixed months (March and April 2026): Food (Groceries -1000 -600,
/// Restaurants -500), Transport (Bus -300), one uncategorized -200, and a
/// $2,105.00 salary each month. The tests switch to "All time", so the
/// grid covers exactly the seeded months whatever today's date is.
pub(super) fn seed_summary_rows(conn: &financi_db::rusqlite::Connection) {
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
pub(super) fn set_all_time(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
) {
    with_window(cx, handle, |window, cx| {
        dash(cx, shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::AllTime, window, cx);
        });
    });
    wait_until(cx, "summary covers the two seeded months", |cx| {
        dash(cx, shell).read(cx).summary.months == ["2026-03", "2026-04"]
    });
}

/// 3.75: click a summary grid cell for real. `id` is the category row
/// (`None` — the Uncategorized row), `month` the column (`None` — Total).
pub(super) fn click_summary_cell(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    id: Option<i64>,
    month: Option<&str>,
) {
    let found = cx.update(|cx| {
        let d = dash(cx, shell).read(cx);
        let ix = d.summary_rows().iter().position(|r| {
            r.id == id
                && !matches!(
                    r.kind,
                    SummaryRowKind::Income | SummaryRowKind::Spend | SummaryRowKind::Net
                )
        })?;
        let months = &d.summary.months;
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
pub(super) fn expand_summary_parent(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    parent: i64,
) {
    let ix = cx.update(|cx| {
        dash(cx, shell)
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
            dash(cx, shell)
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
    dash(cx, shell)
        .read(cx)
        .summary_rows()
        .into_iter()
        .map(|r| r.name.to_string())
        .collect()
}

pub(super) fn summary_parent_id(shell: &Entity<Shell>, cx: &App, name: &str) -> i64 {
    dash(cx, shell)
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
        dash(cx, &shell).read(cx).row_count == 7
    });
    cx.update(|cx| {
        let rows = dash(cx, &shell).read(cx).summary_rows();
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
        assert_eq!(row("Food").cents, [1500, 600, 2100]);
        assert_eq!(row("Uncategorized").cents, [200, 0, 200]);
        assert_eq!(row("Income").cents, [210500, 210500, 421000]);
        assert_eq!(row("Spend").cents, [1700, 900, 2600]);
        assert_eq!(row("Net").cents, [208800, 209600, 418400]);
        // The drawn text is the cents through `flow::money`, formatted
        // once when the view was built (3.85).
        assert_eq!(row("Food").cells, money_cells(&[1500, 600, 2100]));
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
fn summary_views_are_built_once_per_load_and_never_per_repaint(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    draw_frame(cx, handle);
    let built = |cx: &App| dash(cx, &shell).read(cx).summary_views_built.get();
    let before = cx.update(|cx| built(cx));
    assert!(before > 0, "the first painted frame built the views");
    // An unrelated notify and a repaint rebuild nothing: the grid paints
    // the cached views (3.85).
    cx.update(|cx| dash(cx, &shell).update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert_eq!(built(cx), before, "a repaint must not rebuild views");
    });
    // A load that lands new summary data does. The rebuild happens while
    // painting, so the frame after the load is what counts it.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_show_hidden(true, window, cx));
    });
    wait_idle(cx, &shell);
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert!(built(cx) > before, "a load must rebuild the views");
    });
}

/// 3.85 (review follow-up): the summary grid paints every row from
/// `summary_views`, and keeps no item cache, so a data change is enough.
/// Categorizing the uncategorized March purchase moves 200 onto the Food
/// rows; when the write's reload lands, the views the row closure paints —
/// `summary_views`, what `render` draws each frame — show the new month
/// text on the next plain repaint. Nothing scrolls.
#[gpui::test]
fn summary_rows_repaint_after_a_data_change_without_scrolling(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    draw_frame(cx, handle);
    let food_cells = |cx: &App| {
        dash(cx, &shell)
            .read(cx)
            .summary_views
            .iter()
            .find(|r| r.name == "Food")
            .map(|r| r.cells.clone())
    };
    let paints_uncategorized = |cx: &App| {
        dash(cx, &shell)
            .read(cx)
            .summary_views
            .iter()
            .any(|r| r.name == "Uncategorized")
    };
    cx.update(|cx| {
        assert_eq!(
            food_cells(cx).expect("Food is a painted row"),
            money_cells(&[1500, 600, 2100]),
            "the grid paints the seeded month cells"
        );
        assert!(
            paints_uncategorized(cx),
            "the -200 row starts uncategorized"
        );
    });

    let (mystery, groceries) = cx.update(|cx| {
        let mystery = table_rows(&shell, cx)
            .iter()
            .find(|r| r.description == "Mystery Vendor")
            .unwrap()
            .id;
        (mystery, find_child(&shell, cx, "Groceries").unwrap())
    });
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_row_category(mystery, groceries, window, cx)
        });
    });
    // Wait for the write's reload to land the new summary data on the
    // model; the views rebuild during the next paint.
    wait_until(cx, "the model shows the moved 200", |cx| {
        dash(cx, &shell)
            .read(cx)
            .summary_rows()
            .iter()
            .find(|r| r.name == "Food")
            .is_some_and(|r| r.cents == [1700, 600, 2300])
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert_eq!(
            food_cells(cx).expect("Food is still a painted row"),
            money_cells(&[1700, 600, 2300]),
            "the next plain repaint draws the new cells — no scroll involved"
        );
        assert!(
            !paints_uncategorized(cx),
            "the emptied Uncategorized row leaves the painted grid"
        );
    });
}

#[gpui::test]
fn clicking_a_summary_cell_filters_the_table_to_category_and_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));

    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "only March's Food rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 2
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        let mut descs: Vec<&str> = rows.iter().map(|r| r.description.as_str()).collect();
        descs.sort_unstable();
        assert_eq!(descs, ["CORNER CAFE MAR", "GROCER STORE MAR"]);
        assert_eq!(
            dash(cx, &shell).read(cx).filter.summary_month.as_deref(),
            Some("2026-03")
        );
        assert_eq!(
            dash(cx, &shell).read(cx).filter.legend_selection,
            Some(LegendSelection::Category(food))
        );
        // The parent is visible at the top level, so the chart stays put.
        assert!(dash(cx, &shell).read(cx).filter.chart_parent.is_none());
    });

    // Clicking the same cell again clears both halves of the filter.
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "all rows back", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
            && dash(cx, &shell).read(cx).filter.summary_month.is_none()
            && dash(cx, &shell).read(cx).filter.legend_selection.is_none()
    });
}

#[gpui::test]
fn clicking_a_child_summary_cell_drills_the_chart_to_its_parent(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    // Children only paint once the parent is expanded — click its cell,
    // not a row the user cannot see.
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    expand_summary_parent(cx, handle, &shell, food);

    click_summary_cell(cx, handle, &shell, Some(groceries), Some("2026-04"));
    wait_until(cx, "only April's Groceries row", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "GROCER STORE APR");
        assert_eq!(
            dash(cx, &shell).read(cx).filter.legend_selection,
            Some(LegendSelection::Category(groceries))
        );
        assert_eq!(
            dash(cx, &shell).read(cx).filter.summary_month.as_deref(),
            Some("2026-04")
        );
        // The selection must be one of the legend's rows, so the chart
        // drills to the child's parent and stays there across refreshes.
        assert_eq!(dash(cx, &shell).read(cx).filter.chart_parent, Some(food));
    });
}

#[gpui::test]
fn expanding_a_summary_parent_shows_its_child_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));

    cx.update(|cx| dash(cx, &shell).update(cx, |d, _| d.toggle_summary_parent(food)));
    cx.update(|cx| {
        let rows = dash(cx, &shell).read(cx).summary_rows();
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
        assert_eq!(rows[1].cents, [1000, 600, 1600]);
        assert_eq!(rows[2].kind, SummaryRowKind::Child);
    });

    // Collapse again: the children disappear.
    cx.update(|cx| dash(cx, &shell).update(cx, |d, _| d.toggle_summary_parent(food)));
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

/// 3.46: the month is an independent global filter — clearing
/// `legend_selection` by re-clicking the legend row leaves it, and the
/// table stays narrowed to the month alone. The compound controls still
/// clear it: "← All categories", Esc, and turning on "Uncategorized only".
#[gpui::test]
fn clearing_the_legend_selection_leaves_the_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
    });
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    // The Groceries cell only exists once Food is expanded.
    expand_summary_parent(cx, handle, &shell, food);

    // Re-clicking the selected legend row: the selection clears, the
    // month stays — the table keeps April's three rows.
    click_summary_cell(cx, handle, &shell, Some(groceries), Some("2026-04"));
    wait_until(cx, "only April's Groceries row", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "the month stays with the table", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.row_count == 3
            && d.filter.legend_selection.is_none()
            && d.filter.summary_month.as_deref() == Some("2026-04")
    });

    // Re-clicking the `Uncategorized` legend row: the same — March's
    // four rows stay.
    click_summary_cell(cx, handle, &shell, None, Some("2026-03"));
    wait_until(cx, "only March's uncategorized row", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "the month stays with the uncategorized clear", |cx| {
        let d = dash(cx, &shell).read(cx);
        d.row_count == 4
            && d.filter.legend_selection.is_none()
            && d.filter.summary_month.as_deref() == Some("2026-03")
    });

    // "← All categories".
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "only March's Food rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 2
    });
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.show_all_categories(window, cx));
    });
    wait_until(cx, "the month goes with all-categories", |cx| {
        dash(cx, &shell).read(cx).row_count == 7
            && dash(cx, &shell).read(cx).filter.legend_selection.is_none()
            && dash(cx, &shell).read(cx).filter.summary_month.is_none()
    });

    // Turning on "Uncategorized only" over a category selection. Without
    // the month clearing this would show the empty intersection of
    // uncategorized April rows instead of the one uncategorized row.
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-04"));
    wait_until(cx, "only April's Food row", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "uncategorized rows without the month", |cx| {
        dash(cx, &shell).read(cx).filter.legend_selection.is_none()
            && dash(cx, &shell).read(cx).filter.summary_month.is_none()
            && dash(cx, &shell).read(cx).filter.uncategorized_only
            && table_rows(&shell, cx)
                .iter()
                .map(|r| r.description.as_str())
                .eq(["Mystery Vendor"])
    });
    cx.update(|cx| {
        assert_eq!(dash(cx, &shell).read(cx).row_count, 1);
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
        dash(cx, &shell).read(cx).row_count == 8
    });
    cx.update(|cx| {
        // The query really does put the biggest row — Uncategorized —
        // first, so the ordering below comes from the view's sort.
        assert_eq!(dash(cx, &shell).read(cx).summary.rows[0].id, None);
        let rows = dash(cx, &shell).read(cx).summary_rows();
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
        assert_eq!(rows[2].cents, [5200, 0, 5200]);
    });
}
