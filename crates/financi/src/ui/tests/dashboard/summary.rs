//! Summary grid: month and total rows, cell clicks and expansion.

use super::*;

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
            shell.read(cx).dashboard.filter.summary_month.as_deref(),
            Some("2026-03")
        );
        assert_eq!(
            shell.read(cx).dashboard.filter.legend_selection,
            Some(LegendSelection::Category(food))
        );
        // The parent is visible at the top level, so the chart stays put.
        assert!(shell.read(cx).dashboard.filter.chart_parent.is_none());
    });

    // Clicking the same cell again clears both halves of the filter.
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "all rows back", |cx| {
        shell.read(cx).dashboard.row_count == 7
            && shell.read(cx).dashboard.filter.summary_month.is_none()
            && shell.read(cx).dashboard.filter.legend_selection.is_none()
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
            shell.read(cx).dashboard.filter.legend_selection,
            Some(LegendSelection::Category(groceries))
        );
        assert_eq!(
            shell.read(cx).dashboard.filter.summary_month.as_deref(),
            Some("2026-04")
        );
        // The selection must be one of the legend's rows, so the chart
        // drills to the child's parent and stays there across refreshes.
        assert_eq!(shell.read(cx).dashboard.filter.chart_parent, Some(food));
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
            && shell.read(cx).dashboard.filter.legend_selection.is_none()
            && shell.read(cx).dashboard.filter.summary_month.is_none()
    });

    // Re-clicking the `Uncategorized` legend row: the same.
    click_summary_cell(cx, handle, &shell, None, Some("2026-03"));
    wait_until(cx, "only March's uncategorized row", |cx| {
        shell.read(cx).dashboard.row_count == 1
    });
    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "the month goes with the uncategorized row", |cx| {
        shell.read(cx).dashboard.row_count == 7
            && shell.read(cx).dashboard.filter.legend_selection.is_none()
            && shell.read(cx).dashboard.filter.summary_month.is_none()
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
            && shell.read(cx).dashboard.filter.legend_selection.is_none()
            && shell.read(cx).dashboard.filter.summary_month.is_none()
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
        shell.read(cx).dashboard.filter.legend_selection.is_none()
            && shell.read(cx).dashboard.filter.summary_month.is_none()
            && shell.read(cx).dashboard.filter.uncategorized_only
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
