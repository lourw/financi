//! Dashboard table: the uncategorized and show-hidden toggles, and sorting.

use super::*;

#[gpui::test]
fn uncategorized_only_toggle_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    cx.update(|cx| assert!(!dash(cx, &shell).read(cx).filter.uncategorized_only));

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_uncategorized_only(true, window, cx));
    });
    wait_until(cx, "two uncategorized rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 2
    });
    cx.update(|cx| {
        assert!(dash(cx, &shell).read(cx).filter.uncategorized_only);
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.category_id.is_none()),
            "only uncategorized rows remain"
        );
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_uncategorized_only(false, window, cx));
    });
    wait_until(cx, "three rows back", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
}

#[gpui::test]
fn show_hidden_toggle_reveals_hidden_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "hidden row not shown by default", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    cx.update(|cx| {
        assert!(!dash(cx, &shell).read(cx).filter.show_hidden);
        assert!(table_rows(&shell, cx).iter().all(|r| !r.hidden));
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_show_hidden(true, window, cx));
    });
    wait_until(cx, "hidden row appears", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        let hidden = rows
            .iter()
            .find(|r| r.hidden)
            .expect("showing hidden reveals the hidden row");
        assert_eq!(hidden.description, "HIDDEN SPEND");
    });
}

#[gpui::test]
fn table_cells_are_built_once_per_load_and_never_per_repaint(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    let built = |cx: &App| {
        dash(cx, &shell)
            .read(cx)
            .table
            .read(cx)
            .delegate()
            .cells_built
            .get()
    };
    let before = cx.update(|cx| built(cx));
    assert!(before > 0, "the first load built the cells");
    // An unrelated notify and a repaint rebuild nothing: the cells live
    // in the delegate, not in `render_td` (3.85).
    cx.update(|cx| dash(cx, &shell).update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert_eq!(built(cx), before, "a repaint must not rebuild cells");
    });
    // A row load does.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_show_hidden(true, window, cx));
    });
    wait_until(cx, "hidden row appears", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
    });
    cx.update(|cx| {
        assert!(built(cx) > before, "a row load must rebuild cells");
    });
}

#[gpui::test]
fn sorting_the_amount_header_reorders_rows_without_a_reentrant_panic(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });

    let descriptions = |cx: &App| -> Vec<String> {
        table_rows(&shell, cx)
            .iter()
            .map(|r| r.description.clone())
            .collect()
    };
    // Newest-first default; every row shares a date, so they fall back to
    // source order.
    assert_eq!(
        cx.read(descriptions),
        ["CORNER CAFE", "GROCER STORE", "PAYROLL DEPOSIT"]
    );

    // Drive the header sort exactly as the table's click listener does:
    // `perform_sort` runs while the table entity is leased and hands the new
    // sort to the shell, which must not update that same entity in response
    // (that double-lease is the crash). The click test-support gpui-kit ships
    // is disabled in this crate, so we enter the sort through the same
    // delegate call the header's `on_click` makes, under the table's lease.
    let table = cx.update(|cx| dash(cx, &shell).read(cx).table.clone());
    with_window(cx, handle, |window, cx| {
        table.update(cx, |t, cx| {
            t.delegate_mut()
                .perform_sort(4, ColumnSort::Descending, window, cx);
        });
    });

    // The shell adopted the sort and the async re-query reordered by amount
    // descending: +3000, -500, -1200.
    cx.update(|cx| {
        assert_eq!(
            dash(cx, &shell).read(cx).filter.tx_sort,
            TxSort {
                column: TxSortColumn::Amount,
                descending: true,
            }
        );
    });
    wait_until(cx, "rows reordered by amount", |cx| {
        descriptions(cx) == ["PAYROLL DEPOSIT", "CORNER CAFE", "GROCER STORE"]
    });
}

/// Three rows over two accounts with mixed-case descriptions and a
/// categorized / uncategorized mix, so every sortable column (3.44) has its
/// own expected order. Amounts are all distinct.
fn seed_sort_columns_rows(conn: &financi_db::rusqlite::Connection) {
    let cobalt = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let no_fee = create_account(conn, "simplii", "No Fee", AccountKind::Chequing).unwrap();
    let import = |account: i64| {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
            [account],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let cobalt_import = import(cobalt);
    let no_fee_import = import(no_fee);
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    let mut line = 0i64;
    let mut txn = |account: i64,
                   import: i64,
                   date: &str,
                   cents: i64,
                   description: &str,
                   category: Option<i64>| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, 0, X'00', ?3, ?4, ?5, ?6, ?7)",
            (
                account,
                import,
                format!("sort-hash-{line}"),
                date,
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn(
        cobalt,
        cobalt_import,
        "2026-11-01",
        -100,
        "corner cafe",
        Some(groceries),
    );
    txn(no_fee, no_fee_import, "2026-11-02", 300, "Best Buy", None);
    txn(cobalt, cobalt_import, "2026-11-03", -50, "amazon", None);
}

#[gpui::test]
fn every_transactions_table_column_sorts(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_sort_columns_rows);
    wait_until(cx, "three rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });

    let descriptions = |cx: &App| -> Vec<String> {
        table_rows(&shell, cx)
            .iter()
            .map(|r| r.description.clone())
            .collect()
    };
    // Newest-first default (3.4): 11-03, 11-02, 11-01.
    assert_eq!(cx.read(descriptions), ["amazon", "Best Buy", "corner cafe"]);

    let table = cx.update(|cx| dash(cx, &shell).read(cx).table.clone());
    // Drive the header sort as the 3.4 test above does, passing the
    // `ColumnSort` the table's own icon cycle hands the delegate: the off
    // step cycles to Descending (the first click), then Ascending, back to
    // off, …
    let click = |cx: &mut TestAppContext, col_ix: usize, value: ColumnSort| {
        with_window(cx, handle, |window, cx| {
            table.update(cx, |t, cx| {
                t.delegate_mut().perform_sort(col_ix, value, window, cx);
            });
        });
    };
    // Every expected order below differs from the one before it, so
    // waiting for it also waits for the re-query to settle before the
    // next click.
    let assert_sorted = |cx: &mut TestAppContext, want: TxSort, order: [&str; 3], col_ix: usize| {
        wait_until(cx, "rows re-sorted", |cx| descriptions(cx) == order);
        cx.update(|cx| {
            assert_eq!(
                dash(cx, &shell).read(cx).filter.tx_sort,
                want,
                "{:?}",
                want.column
            );
            // The active header shows the direction the rows are in.
            assert_eq!(
                table.read(cx).delegate().column(col_ix, cx).sort,
                Some(if want.descending {
                    ColumnSort::Descending
                } else {
                    ColumnSort::Ascending
                }),
                "header of column {col_ix} shows the direction"
            );
        });
    };

    // Account, first click (icon off → Descending): A→Z on
    // "Institution · Account"; the tied Cobalt rows fall back to date.
    click(cx, 1, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Account,
            descending: false,
        },
        ["amazon", "corner cafe", "Best Buy"],
        1,
    );
    // Second click (Ascending → off step): Z→A.
    click(cx, 1, ColumnSort::Default);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Account,
            descending: true,
        },
        ["Best Buy", "amazon", "corner cafe"],
        1,
    );
    // Description, first click: A→Z, case-insensitively.
    click(cx, 2, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Description,
            descending: false,
        },
        ["amazon", "Best Buy", "corner cafe"],
        2,
    );
    // Category, first click: A→Z with the uncategorized rows sorted as
    // "Uncategorized" — after Groceries, before nothing here, and their
    // tie keeps the newest-first date order.
    click(cx, 3, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Category,
            descending: false,
        },
        ["corner cafe", "amazon", "Best Buy"],
        3,
    );
    // Second click: Z→A, so "Uncategorized" leads.
    click(cx, 3, ColumnSort::Default);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Category,
            descending: true,
        },
        ["amazon", "Best Buy", "corner cafe"],
        3,
    );
    // Date and Amount keep their first directions (3.4): Date's active
    // descending column cycles to Ascending, Amount's off step to
    // Descending (biggest first).
    click(cx, 0, ColumnSort::Ascending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Date,
            descending: false,
        },
        ["corner cafe", "Best Buy", "amazon"],
        0,
    );
    click(cx, 4, ColumnSort::Descending);
    assert_sorted(
        cx,
        TxSort {
            column: TxSortColumn::Amount,
            descending: true,
        },
        ["Best Buy", "amazon", "corner cafe"],
        4,
    );
}

// ------------------------------------------------------------------ 3.5 chart
