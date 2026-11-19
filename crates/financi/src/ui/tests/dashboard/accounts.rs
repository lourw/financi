//! Dashboard accounts panel and the options sheet: account boxes and the
//! include-transfers toggle.

use super::*;

#[gpui::test]
fn toggling_include_transfers_filters_rows_and_persists(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        dash(cx, &shell).read(cx).row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert!(!d.filter.include_transfers, "default off");
        assert!(!d.view_filter.include_transfers);
    });
    // The default is not persisted; the setting only appears on change.
    assert_eq!(db_setting(cx, &shell, UI_INCLUDE_TRANSFERS), None);

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_include_transfers(true, window, cx));
    });
    wait_until(cx, "transfer row appears", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
            && dash(cx, &shell).read(cx).view_filter.include_transfers
    });
    // Let the persist task queue its write before reading (the worker is FIFO).
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_INCLUDE_TRANSFERS).as_deref(),
        Some("1"),
        "ui.include_transfers was written"
    );

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_include_transfers(false, window, cx));
    });
    wait_until(cx, "transfer row hidden again", |cx| {
        dash(cx, &shell).read(cx).row_count == 2
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_INCLUDE_TRANSFERS).as_deref(),
        Some("0")
    );
}

/// 3.35: the Options popover carries the old filter-row toggles — same ids,
/// same state and handlers. A real click opens it, all three checkboxes
/// paint, and Include transfers toggles the rows exactly as before.
#[gpui::test]
fn clicking_the_include_transfers_checkbox_toggles_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        dash(cx, &shell).read(cx).row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    click(cx, handle, "filters:options");
    draw_frame(cx, handle);
    for sel in [
        "checkbox:include-transfers",
        "checkbox:uncategorized-only",
        "checkbox:show-hidden",
    ] {
        assert!(paints(cx, handle, sel), "the Options popover renders {sel}");
    }
    click(cx, handle, "checkbox:include-transfers");
    wait_until(cx, "click checks the box and shows the transfer", |cx| {
        dash(cx, &shell).read(cx).filter.include_transfers
            && dash(cx, &shell).read(cx).row_count == 3
    });
    // 3.65: a checked box answers to `checkbox:<id>:checked`.
    click(cx, handle, "checkbox:include-transfers:checked");
    wait_until(cx, "clicking again unchecks", |cx| {
        !dash(cx, &shell).read(cx).filter.include_transfers
            && dash(cx, &shell).read(cx).row_count == 2
    });
}

/// 3.35: the Options trigger counts its toggles — plain "Options" while
/// none are on, "Options · 2" when two are.
#[gpui::test]
fn the_options_button_counts_the_toggles_that_are_on(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        dash(cx, &shell).read(cx).row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "filters:options-btn:Options"),
        "with no toggle on, the button reads just \"Options\""
    );
    // Flip two toggles through the popover, the way a user does.
    click(cx, handle, "filters:options");
    click(cx, handle, "checkbox:include-transfers");
    click(cx, handle, "checkbox:uncategorized-only");
    wait_until(cx, "both toggles on", |cx| {
        dash(cx, &shell).read(cx).filter.include_transfers
            && dash(cx, &shell).read(cx).filter.uncategorized_only
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "filters:options-btn:Options \u{b7} 2"),
        "with two toggles on, the button reads \"Options \u{b7} 2\""
    );
}

#[gpui::test]
fn a_saved_include_transfers_setting_is_restored_at_launch(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        seed_two_accounts_and_a_transfer(c);
        settings::set(c, UI_INCLUDE_TRANSFERS, "1").unwrap();
    });
    // A fresh shell over the stored "1" re-opens with transfers included,
    // and the table query follows the restored filter.
    wait_until(cx, "saved transfers flag restored", |cx| {
        dash(cx, &shell).read(cx).filter.include_transfers
            && dash(cx, &shell).read(cx).row_count == 3
    });
    cx.update(|cx| assert!(dash(cx, &shell).read(cx).view_filter.include_transfers));
}

#[gpui::test]
fn unchecking_an_account_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "accounts and rows loaded", |cx| {
        shell.read(cx).accounts.len() == 2 && dash(cx, &shell).read(cx).row_count == 2
    });
    let (cobalt, gold) = cx.update(|cx| {
        let s = shell.read(cx);
        let d = dash(cx, &shell).read(cx);
        assert!(
            d.filter.excluded_accounts.is_empty(),
            "default: all checked"
        );
        assert!(d.view_filter.account_ids.is_empty(), "empty means all");
        let id = |name: &str| s.accounts.iter().find(|a| a.name == name).unwrap().id;
        (id("Cobalt"), id("Gold"))
    });

    // Open the accounts popover and click the checkboxes themselves.
    // 3.65: a checked box answers to `checkbox:account-<id>:checked`,
    // an unchecked one to the plain selector.
    click(cx, handle, "accounts:filter");
    click(cx, handle, format!("checkbox:account-{cobalt}:checked"));
    wait_until(cx, "only Gold's row shows", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
            && table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Gold"))
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.view_filter.account_ids, vec![gold]);
    });

    // Unchecking every account shows no rows (empty in `ViewFilter`
    // means all, so the table must not fall back to everything).
    click(cx, handle, format!("checkbox:account-{gold}:checked"));
    wait_until(cx, "nothing selected: empty table", |cx| {
        dash(cx, &shell).read(cx).row_count == 0
    });

    click(cx, handle, format!("checkbox:account-{cobalt}"));
    wait_until(cx, "Cobalt's row back", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
            && table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Cobalt"))
    });
    cx.update(|cx| {
        assert_eq!(
            dash(cx, &shell).read(cx).view_filter.account_ids,
            vec![cobalt]
        );
    });
}

/// 3.65: the click repaints before the answer. With the worker held on
/// the refresh the click starts, a single frame shows the box checked and
/// the Options button counting the toggle, while the table still holds the
/// old rows; the transfer row arrives only when the answer lands.
#[gpui::test]
fn include_transfers_checkbox_paints_before_the_answer(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        dash(cx, &shell).read(cx).row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    click(cx, handle, "filters:options");
    let done = block_worker(cx, &shell);
    click(cx, handle, "checkbox:include-transfers");
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "checkbox:include-transfers:checked"),
        "the box paints checked while the refresh it started is still in flight"
    );
    assert!(
        paints(cx, handle, "filters:options-btn:Options \u{b7} 1"),
        "the button already reads \"Options \u{b7} 1\" before the answer"
    );
    assert_eq!(
        cx.update(|cx| dash(cx, &shell).read(cx).row_count),
        2,
        "only the data waits for the DB"
    );
    drop(done);
    wait_until(cx, "the transfer row arrives with the answer", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
}

/// 3.65: the same for the account checkboxes in the popover — one frame
/// after the click the box paints unchecked (and no longer `:checked`),
/// while the table still shows the rows of the previous answer.
#[gpui::test]
fn account_checkbox_paints_before_the_answer(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "accounts and rows loaded", |cx| {
        shell.read(cx).accounts.len() == 2 && dash(cx, &shell).read(cx).row_count == 2
    });
    let cobalt = cx.update(|cx| {
        shell
            .read(cx)
            .accounts
            .iter()
            .find(|a| a.name == "Cobalt")
            .unwrap()
            .id
    });
    click(cx, handle, "accounts:filter");
    let done = block_worker(cx, &shell);
    click(cx, handle, format!("checkbox:account-{cobalt}:checked"));
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("checkbox:account-{cobalt}")),
        "the box paints (unchecked) while the refresh it started is still in flight"
    );
    assert!(
        !paints(cx, handle, format!("checkbox:account-{cobalt}:checked")),
        "the checked selector is gone the moment the click lands"
    );
    assert_eq!(
        cx.update(|cx| dash(cx, &shell).read(cx).row_count),
        2,
        "only the data waits for the DB"
    );
    drop(done);
    wait_until(cx, "Gold's row alone arrives with the answer", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
}

/// Task 3.17: `none_selected` must resolve against the accounts loaded *by
/// this* refresh, not the previous refresh's list. An import that adds an
/// account while every existing checkbox is unchecked leaves that new
/// account checked, so every view must show its rows as soon as the import
/// finishes — not stay blank until some other filter changes.
#[gpui::test]
fn an_import_that_adds_an_account_shows_it_with_every_box_unchecked(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "Gold loaded and the all-time range restored", |cx| {
        shell.read(cx).accounts.len() == 1
            && dash(cx, &shell).read(cx).filter.date_range == DateRange::Preset(DatePreset::AllTime)
    });

    // Uncheck the only account: nothing is selected, so the views are
    // legitimately blank.
    let gold = cx.update(|cx| shell.read(cx).accounts[0].id);
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_account_checked(gold, false, window, cx));
    });
    wait_until(cx, "no account checked", |cx| {
        dash(cx, &shell).read(cx).filter.excluded_accounts == [gold]
            && dash(cx, &shell).read(cx).row_count == 0
    });

    // Import into a brand-new account: its checkbox is on, so the finished
    // import must fill the views with its rows on its own.
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(
        cx,
        "the imported rows show without a further filter change",
        |cx| dash(cx, &shell).read(cx).row_count == 100,
    );

    cx.update(|cx| {
        let s = shell.read(cx);
        let d = dash(cx, &shell).read(cx);
        let amex = s
            .accounts
            .iter()
            .find(|a| a.name == "Amex")
            .expect("the import created Amex");
        assert!(
            !d.filter.excluded_accounts.contains(&amex.id),
            "a new account starts checked"
        );
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Amex")),
            "only the new account is selected, so only its rows show"
        );
        assert_eq!(d.summary.months, ["2026-09", "2026-10", "2026-11"]);
        assert!(
            !d.chart_rows.is_empty(),
            "the chart shows the imported spend"
        );
    });
}

/// Task 3.17 (regression): the refresh triggered when an import finishes
/// starts with the *previous* refresh's `self.accounts` — the database
/// already has the new account, the shell does not. Judging "no accounts
/// checked" from that stale list skipped the queries, so every view stayed
/// blank until some other filter changed; `none_selected` must resolve
/// against the accounts the refresh itself loads.
#[gpui::test]
fn a_refresh_resolves_the_account_filter_against_the_accounts_it_loads(cx: &mut TestAppContext) {
    // Gold's rows were imported long ago; Amex's import is the one that has
    // just landed, after the shell's previous refresh.
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
        let gold = create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
        let amex = create_account(c, "amex", "Amex", AccountKind::CreditCard).unwrap();
        for (account, line, date) in [
            (gold, 0, "2026-11-05"),
            (amex, 0, "2026-10-01"),
            (amex, 1, "2026-10-02"),
        ] {
            c.execute(
                "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                      encoding, has_bom, line_ending, trailing_newline, row_count)
                 VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
                [account],
            )
            .unwrap();
            let import = c.last_insert_rowid();
            c.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, category_id)
                 VALUES (?1, ?2, ?3, X'00', ?4, ?5, -100, 'ROW', NULL)",
                (
                    account,
                    import,
                    line,
                    format!("hash-{account}-{line}"),
                    date,
                ),
            )
            .unwrap();
        }
    });
    wait_until(cx, "both accounts and three rows loaded", |cx| {
        shell.read(cx).accounts.len() == 2 && dash(cx, &shell).read(cx).row_count == 3
    });
    let gold = cx.update(|cx| {
        shell
            .read(cx)
            .accounts
            .iter()
            .find(|a| a.name == "Gold")
            .unwrap()
            .id
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, _| {
            // The state the import's refresh starts from: the user has
            // unchecked every account the previous refresh showed — which
            // knew only Gold, not the account the import added.
            s.accounts.retain(|a| a.id == gold);
        });
        // 3.82: the filter is the page's; the page re-derives its checked
        // accounts from the fresh snapshot on the refresh that follows.
        dash(cx, &shell).update(cx, |d, _| d.filter.excluded_accounts = vec![gold]);
        shell.update(cx, |s, cx| s.refresh(window, cx));
    });

    // The accounts loaded by this refresh include Amex, whose checkbox is on,
    // so its two rows show: the filter must not be judged from the stale list.
    wait_until(cx, "the checked new account's rows show", |cx| {
        dash(cx, &shell).read(cx).row_count == 2
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        let d = dash(cx, &shell).read(cx);
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Amex")),
            "only the new account is checked, so only its rows show"
        );
        let amex = s.accounts.iter().find(|a| a.name == "Amex").unwrap().id;
        assert_eq!(d.view_filter.account_ids, vec![amex]);
        assert!(!d.chart_rows.is_empty(), "the chart is not blank");
        assert!(!d.summary.months.is_empty(), "the summary is not blank");
    });
}

// Dashboard table search and toggles (task 3.4)

/// 3.26: with more accounts than the popover's capped list shows, the
/// list scrolls and the always-visible bar paints inside the list's
/// right edge; a scrolled position survives an unrelated redraw.
#[gpui::test]
fn the_accounts_filter_shows_a_scrollbar_when_it_overflows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        for n in 0..15 {
            create_account(
                conn,
                "amex",
                &format!("Acct {n:02}"),
                AccountKind::CreditCard,
            )
            .unwrap();
        }
    });
    wait_until(cx, "15 accounts", |cx| shell.read(cx).accounts.len() == 15);
    click(cx, handle, "accounts:filter");

    let region = bounds(cx, handle, "accounts:filter-list");
    let bar = bounds(cx, handle, "accounts:filter-list-vscrollbar");
    assert!(
        bar.origin.y >= region.origin.y
            && bar.origin.y + bar.size.height <= region.origin.y + region.size.height
            && bar.origin.x <= region.origin.x + region.size.width
            && bar.origin.x + bar.size.width <= region.origin.x + region.size.width,
        "the bar sits inside the popover list's right edge: bar {bar:?} region {region:?}"
    );
    assert!(
        f32::from(region.size.height) <= 241.,
        "the list is capped so the popover stays on screen ({region:?})"
    );

    let max = cx.update(|cx| {
        dash(cx, &shell)
            .read(cx)
            .accounts_filter_scroll
            .max_offset()
            .y
    });
    assert!(f32::from(max) > 0., "15 accounts overflow the capped list");
    // A plain scroll div builds every row, so the last row is recorded
    // even while clipped: prove it starts below the visible strip.
    let last = {
        let id = cx.update(|cx| {
            shell
                .read(cx)
                .accounts
                .iter()
                .find(|a| a.name == "Acct 14")
                .unwrap()
                .id
        });
        leaked(format!("checkbox:account-{id}:checked"))
    };
    let row = bounds(cx, handle, last);
    assert!(
        f32::from(row.origin.y) > f32::from(region.origin.y + region.size.height),
        "the last account starts below the clipped strip: row {row:?} region {region:?}"
    );
    with_window(cx, handle, |_, cx| {
        dash(cx, &shell)
            .read(cx)
            .accounts_filter_scroll
            .set_offset(point(px(0.), -max));
    });
    draw_frame(cx, handle);
    let region = bounds(cx, handle, "accounts:filter-list");
    let row = bounds(cx, handle, last);
    assert!(
        f32::from(row.origin.y) >= f32::from(region.origin.y)
            && f32::from(row.origin.y + row.size.height)
                <= f32::from(region.origin.y + region.size.height),
        "scrolling to the bottom brings the last account into the strip: row {row:?} region {region:?}"
    );
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    let held = bounds(cx, handle, last);
    assert_eq!(
        f32::from(held.origin.y),
        f32::from(row.origin.y),
        "the position holds a redraw"
    );
}

/// 3.26: two accounts fit the popover's list, so no thumb shows.
#[gpui::test]
fn the_accounts_filter_shows_no_scrollbar_when_it_fits(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "accounts loaded", |cx| {
        shell.read(cx).accounts.len() == 2
    });
    click(cx, handle, "accounts:filter");
    draw_frame(cx, handle);
    let max = cx.update(|cx| {
        dash(cx, &shell)
            .read(cx)
            .accounts_filter_scroll
            .max_offset()
            .y
    });
    assert_eq!(f32::from(max), 0., "two accounts fit the capped list");
}
