//! Tests for `ui/queue.rs` (3.62 split from `ui_tests.rs`). 3.83: the page
//! is a `QueuePage` entity on `Shell::queue_view`; waits that may run while
//! the page is hidden read the `Catalog` snapshot instead.

use super::dashboard::{click_chart_legend, dash, seed_chart_rows, top_category_id};
use super::*;

/// Pick `child` in the merchant row's picker (the same get-or-create the
/// row's render makes) and notify the page, as the `Confirm` event would.
fn queue_pick(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    merchant: &str,
    child: i64,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let picker = s
                .queue_view
                .update(cx, |p, cx| p.queue_picker(merchant, window, cx));
            // 3.59: the grouped picker items carry the category id as
            // their value, so the pick resolves by value (a sectioned
            // menu's `IndexPath` is not a flat row number).
            picker.update(cx, |sel, cx| {
                sel.set_selected_value(&child, window, cx);
            });
            // The popup's pick emits `Confirm`, which re-renders the rows;
            // `set_selected_index` drives the state directly, so the test
            // notifies on the subscription's behalf.
            s.queue_view.update(cx, |_, cx| cx.notify());
        });
    });
}

/// Drive the same `categorize_merchant` call the row's button makes.
fn queue_categorize(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    merchant: String,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.queue_view.update(cx, |p, cx| {
                p.categorize_merchant(merchant.clone(), window, cx)
            })
        });
    });
}

#[gpui::test]
fn categorizing_a_queue_group_locks_in_a_rule(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts", |cx| shell.read(cx).accounts.len() == 1);
    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "import", |cx| dash(cx, &shell).read(cx).row_count == 67);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());

    // 3.83: the page applies the queue when shown, so visit it first.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "queue populated", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    let (merchant, before) = cx.update(|cx| top_group(&shell, cx));

    queue_pick(cx, handle, &shell, &merchant, child);
    queue_categorize(cx, handle, &shell, merchant.clone());
    let gone = merchant.clone();
    let shell_for_gone = shell.clone();
    // 3.84: the group leaves the list in the click's frame, so "gone" no
    // longer means "written" — wait for the write's report too.
    wait_until(cx, "queue group gone and counted", move |cx| {
        shell_for_gone.read(cx).cat_counts.contains_key(&child)
            && shell_for_gone
                .read(cx)
                .queue_view
                .read(cx)
                .groups
                .iter()
                .all(|g| g.merchant != gone)
    });
    let online = cx.update(|cx| *shell.read(cx).cat_counts.get(&child).unwrap());
    assert!(online >= before, "{online} >= {before} categorized");
    cx.update(|cx| {
        assert!(
            shell.read(cx).rules.iter().any(|r| r.pattern == merchant),
            "a rule was created for the merchant"
        );
    });
}

/// 3.84: Categorize answers in the click's frame — the group leaves the
/// list before the write runs (the worker is held). The write is then
/// made to fail: the picked category is deleted behind the page's back,
/// so `add_rule` refuses the unknown id while the row's picker still
/// holds its (mirrored) pick. The rollback puts the group back at its
/// exact index; the page is hidden before the failure lands so the
/// write's reload can't overwrite the rollback before it is observed.
#[gpui::test]
fn categorize_removes_the_group_in_the_click_frame_and_rolls_back(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 3));
    wait_until(cx, "3 queue groups", |cx| queue_len(&shell, cx) == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "queue applied", |cx| {
        shell.read(cx).queue_view.read(cx).groups.len() == 3
    });
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    // The middle group, so the rollback's index is not the boundary.
    let merchant = cx.update(|cx| {
        shell.read(cx).queue_view.read(cx).groups[1]
            .merchant
            .clone()
    });

    // Make the write fail, after the pick resolves against the mirror.
    queue_pick(cx, handle, &shell, &merchant, child);
    let db = cx.update(|cx| shell.read(cx).db.clone());
    block_on(db.call(move |c| {
        c.execute("DELETE FROM categories WHERE id = ?1", [child])
            .map(|_| ())
    }))
    .expect("the delete job ran")
    .expect("the category delete succeeds");

    let done = block_worker(cx, &shell);
    queue_categorize(cx, handle, &shell, merchant.clone());
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert!(
            !p.groups.iter().any(|g| g.merchant == merchant),
            "the group leaves the queue before the write has run"
        );
    });

    // Hide the page, then let the write fail.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    drop(done);
    // 3.95: a failed write reports through the error toast, not the page.
    wait_until(cx, "the failed write reports", |cx| {
        last_toast(&shell, cx).is_some_and(|(_, error)| error)
    });
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert_eq!(p.groups.len(), 3, "the rolled-back group is back");
        assert_eq!(
            p.groups[1].merchant, merchant,
            "at its exact index, picker and all"
        );
        assert!(
            p.has_picker(&merchant),
            "the row's picker survived the round trip"
        );
    });
}

// -- 3.95: editing the rule while categorizing.

/// Type into the merchant row's rule-pattern input (the same `set_value`
/// a user's keystrokes end up as). Builds the row's input if the paint
/// has not yet, exactly as the row's render does.
fn queue_set_pattern(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    merchant: &str,
    value: &str,
) {
    let value = value.to_string();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.queue_view.update(cx, |p, cx| {
                let input = p.queue_input(merchant, window, cx);
                input.update(cx, |i, cx| i.set_value(value, window, cx));
            });
        });
    });
}

/// The pattern text the merchant row's input holds.
fn queue_pattern(cx: &mut TestAppContext, shell: &Entity<Shell>, merchant: &str) -> String {
    cx.update(|cx| {
        shell
            .read(cx)
            .queue_view
            .read(cx)
            .row_input(merchant)
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default()
    })
}

/// Click the merchant row's regex switch.
fn queue_toggle_regex(cx: &mut TestAppContext, handle: AnyWindowHandle, merchant: &str) {
    click(cx, handle, format!("queue:regex-switch:{merchant}"));
}

/// 3.95: the row's pattern input is blank and its placeholder is the
/// merchant — blank visibly means "use the merchant".
#[gpui::test]
fn the_row_rule_input_carries_the_merchant_as_placeholder(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 3));
    wait_until(cx, "3 queue groups", |cx| queue_len(&shell, cx) == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    draw_frame(cx, handle);
    let merchant = cx.update(|cx| {
        shell.read(cx).queue_view.read(cx).groups[0]
            .merchant
            .clone()
    });
    cx.update(|cx| {
        let input = shell
            .read(cx)
            .queue_view
            .read(cx)
            .row_input(&merchant)
            .expect("the visible row built its pattern input");
        let state = input.read(cx);
        assert_eq!(
            state.presentation().placeholder().to_string(),
            merchant,
            "the placeholder is the merchant name"
        );
        assert!(state.value().is_empty(), "the input starts blank");
    });
}

/// 3.95: type a pattern, pick a category, click Categorize — the rule is
/// the typed pattern, not the merchant, and it is there on the Rules
/// page.
#[gpui::test]
fn a_written_pattern_categorizes_the_group_through_a_rule(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    import_and_top_merchant(cx, handle, &shell);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "queue applied", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    let (merchant, before) = cx.update(|cx| top_group(&shell, cx));

    queue_set_pattern(cx, handle, &shell, &merchant, "BEAN");
    queue_pick(cx, handle, &shell, &merchant, child);
    queue_categorize(cx, handle, &shell, merchant.clone());
    wait_until(cx, "the BEAN rule exists", |cx| {
        shell.read(cx).rules.iter().any(|r| r.pattern == "BEAN")
    });
    let gone = merchant.clone();
    let shell_for_gone = shell.clone();
    wait_until(cx, "queue group gone", move |cx| {
        shell_for_gone
            .read(cx)
            .queue_view
            .read(cx)
            .groups
            .iter()
            .all(|g| g.merchant != gone)
    });
    cx.update(|cx| {
        let rule = shell
            .read(cx)
            .rules
            .iter()
            .find(|r| r.pattern == "BEAN")
            .expect("the rule holds the typed pattern, not the merchant");
        assert!(!rule.is_regex, "a substring rule while the switch is off");
        let online = *shell.read(cx).cat_counts.get(&child).unwrap();
        assert!(online >= before, "{online} >= {before} categorized");
    });
    // The success toast names the rule it added.
    assert_eq!(
        cx.update(|cx| last_toast(&shell, cx)),
        Some((format!("Categorized {merchant} · rule: BEAN"), false)),
        "the toast names the pattern the user wrote"
    );
    // And it is on the Rules page, listed under its pattern.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.run_until_parked();
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .rules
                .iter()
                .any(|r| r.pattern == "BEAN" && !r.is_regex),
            "the Rules page lists the BEAN rule"
        );
    });
}

/// 3.95: with the regex switch on the written pattern becomes a regex
/// rule, and the Rules page shows it as regex.
#[gpui::test]
fn the_regex_switch_makes_the_written_pattern_a_regex_rule(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    import_and_top_merchant(cx, handle, &shell);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "queue applied", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    let (merchant, _) = cx.update(|cx| top_group(&shell, cx));

    queue_set_pattern(cx, handle, &shell, &merchant, r"^BEANERY");
    queue_toggle_regex(cx, handle, &merchant);
    queue_pick(cx, handle, &shell, &merchant, child);
    queue_categorize(cx, handle, &shell, merchant.clone());
    wait_until(cx, "the regex rule exists", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.pattern == "^BEANERY" && r.is_regex)
    });
    cx.update(|cx| {
        let rule = shell
            .read(cx)
            .rules
            .iter()
            .find(|r| r.pattern == "^BEANERY")
            .expect("the regex pattern was added as written");
        assert!(rule.is_regex, "the switch on means a regex rule");
    });
}

/// 3.95: Enter in the pattern input is the row's Categorize button.
#[gpui::test]
fn enter_in_the_rule_input_categorizes_the_group(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    import_and_top_merchant(cx, handle, &shell);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "queue applied", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    let (merchant, _) = cx.update(|cx| top_group(&shell, cx));

    queue_set_pattern(cx, handle, &shell, &merchant, "BEAN");
    queue_pick(cx, handle, &shell, &merchant, child);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.queue_view.update(cx, |p, cx| {
                let input = p.row_input(&merchant).expect("the row built its input");
                window.focus(&input.read(cx).focus_handle(cx), cx);
            });
        });
    });
    cx.simulate_keystrokes(handle, "enter");
    wait_until(cx, "the BEAN rule exists", |cx| {
        shell.read(cx).rules.iter().any(|r| r.pattern == "BEAN")
    });
    assert!(
        cx.update(|cx| shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.pattern == "BEAN" && !r.is_regex)),
        "Enter added the typed pattern's rule"
    );
}

/// 3.95: a pattern that matches none of the group's rows is refused with
/// the error toast, adds no rule, and leaves the row — and its typed
/// text — for another try.
#[gpui::test]
fn a_pattern_matching_nothing_is_refused_and_keeps_the_row(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    import_and_top_merchant(cx, handle, &shell);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "queue applied", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    let (merchant, _) = cx.update(|cx| top_group(&shell, cx));

    queue_set_pattern(cx, handle, &shell, &merchant, "NOTTHISTORE");
    queue_pick(cx, handle, &shell, &merchant, child);
    queue_categorize(cx, handle, &shell, merchant.clone());
    wait_until(cx, "the refusal toast", |cx| {
        last_toast(&shell, cx) == Some((format!("That rule doesn't match {merchant}"), true))
    });
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .queue_view
                .read(cx)
                .groups
                .iter()
                .any(|g| g.merchant == merchant),
            "the row stays in the queue"
        );
        assert!(
            !shell
                .read(cx)
                .rules
                .iter()
                .any(|r| r.pattern == "NOTTHISTORE"),
            "the refused write adds no rule"
        );
    });
    assert_eq!(
        queue_pattern(cx, &shell, &merchant),
        "NOTTHISTORE",
        "the input keeps its text"
    );
}

/// 3.19: synthetic queue data — `groups` merchant groups of two uncategorized
/// transactions each, plus one parent/child pair to categorize onto.
fn seed_queue_thousands(conn: &financi_db::rusqlite::Connection, groups: usize) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    for i in 0..groups {
        for n in 0..2 {
            conn.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents, description_raw)
                 VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
                (
                    acct,
                    import,
                    (i * 2 + n) as i64,
                    format!("queue-hash-{i}-{n}"),
                    format!("QUEUE MERCHANT {i:04}"),
                ),
            )
            .unwrap();
        }
    }
}

/// The queue snapshot length — current on any page, since a hidden page
/// defers applying (3.83).
fn queue_len(shell: &Entity<Shell>, cx: &App) -> usize {
    shell.read(cx).catalog.read(cx).snap.queue.len()
}

/// 3.19: with thousands of merchant groups the To-categorize page must still
/// render, building only the visible rows — and only pickers for those.
#[gpui::test]
fn the_queue_page_builds_only_the_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| queue_len(&shell, cx) == 2_000);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert!(
            p.rows_built.get() > 0 && p.rows_built.get() < 100,
            "the queue built {} rows of 2000 — it must render a window",
            p.rows_built.get()
        );
        // 3.19: pickers come with the rows that rendered, not one per group.
        assert!(
            p.picker_count() < 100,
            "{} pickers for a {}-row window — pickers must be built per visible row",
            p.picker_count(),
            p.rows_built.get()
        );
        assert!(
            p.has_picker("QUEUE MERCHANT 0000"),
            "the first visible row has a picker"
        );
        assert!(
            !p.has_picker("QUEUE MERCHANT 1999"),
            "the last group is off-screen, so it must not have a picker"
        );
    });
}

/// 3.19: categorizing still works with a long queue — drive the same
/// `categorize_merchant` call the row's button makes for the group at the
/// very bottom, and its rows leave the queue behind a rule.
#[gpui::test]
fn a_queue_group_at_the_very_bottom_can_still_be_categorized(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| queue_len(&shell, cx) == 2_000);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    let last = cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        p.groups
            .last()
            .expect("queue has a bottom")
            .merchant
            .clone()
    });
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    queue_pick(cx, handle, &shell, &last, child);
    queue_categorize(cx, handle, &shell, last.clone());
    let gone = last.clone();
    let shell_for_gone = shell.clone();
    // 3.84: the group leaves the list in the click's frame, so "gone" no
    // longer means "written" — wait for the write's report too.
    wait_until(cx, "bottom queue group categorized", move |cx| {
        let s = shell_for_gone.read(cx);
        s.cat_counts.get(&child).copied().unwrap_or(0) == 2
            && s.rules.iter().any(|r| r.pattern == gone)
            && s.queue_view
                .read(cx)
                .groups
                .iter()
                .all(|g| g.merchant != gone)
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.cat_counts.get(&child).copied().unwrap_or(0),
            2,
            "both rows of the bottom group are categorized"
        );
        assert!(
            s.rules.iter().any(|r| r.pattern == last),
            "a rule was created for the bottom merchant"
        );
        assert!(
            s.queue_view.read(cx).rows_built.get() < 100,
            "still windowed after categorizing"
        );
    });
}

/// 3.21: the To-categorize queue keeps its scroll offset across frames and
/// across a plain refresh, so the bottom of a thousands-group queue stays
/// reachable once scrolled to (the same bug as the tree: a per-render
/// scroll handle reset the offset to 0 every frame).
#[gpui::test]
fn the_queue_keeps_its_scroll_offset(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| queue_len(&shell, cx) == 2_000);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert_eq!(p.scroll.offset(), Point::default(), "starts unscrolled");
        assert_eq!(p.range.get().0, 0, "the first frame shows the top");
        // Jump straight to the bottom; layout clamps to the scroll max.
        p.scroll.set_offset(point(px(0.), px(-1_000_000.)));
    });
    draw_frame(cx, handle);
    let scrolled = cx.update(|cx| shell.read(cx).queue_view.read(cx).scroll.offset().y);
    cx.update(|cx| {
        let s = shell.read(cx);
        let p = s.queue_view.read(cx);
        assert!(scrolled < px(0.), "the queue is scrolled down");
        let (start, end) = p.range.get();
        assert!(
            start > 0,
            "the built window moved off the top (start {start})"
        );
        assert_eq!(
            end,
            p.groups.len(),
            "the built window reached the last group"
        );
        // Pickers come with the rows that rendered: the bottom group's
        // picker is there, a middle group that was never on screen isn't.
        assert!(
            p.has_picker("QUEUE MERCHANT 1999"),
            "the bottom group is on screen, so it has a picker"
        );
        assert!(
            !p.has_picker("QUEUE MERCHANT 1000"),
            "a middle group never scrolled into view, so it must not have a picker"
        );
        // 3.83: and pruning keeps the map the size of the window — the
        // picker the top row left behind is gone.
        assert!(
            !p.has_picker("QUEUE MERCHANT 0000"),
            "a picker scrolled a screenful past is pruned"
        );
        assert!(
            p.picker_count() < 100,
            "{} pickers after scrolling — pruning must bound the map",
            p.picker_count()
        );
    });
    // A refresh that doesn't replace the queue must not move the offset.
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        let p = s.queue_view.read(cx);
        assert_eq!(
            p.scroll.offset().y,
            scrolled,
            "the offset survived an unrelated notify"
        );
        assert_eq!(p.range.get().1, p.groups.len());
    });
}

/// 3.83: pickers track the rendered range. Scrolling a screenful past a row
/// retires its picker (and its subscription); scrolling back rebuilds one,
/// with the current category choices in it; and a picker holding a pick is
/// never dropped, however far its row scrolls.
#[gpui::test]
fn the_queue_prunes_pickers_as_its_window_moves(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| queue_len(&shell, cx) == 2_000);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    // Top of the list: the first row's picker exists and the map is small.
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert!(p.has_picker("QUEUE MERCHANT 0000"));
        assert!(p.picker_count() < 100);
    });
    // Scroll to the bottom and back to the top.
    cx.update(|cx| {
        shell
            .read(cx)
            .queue_view
            .read(cx)
            .scroll
            .set_offset(point(px(0.), px(-1_000_000.)));
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert!(
            !p.has_picker("QUEUE MERCHANT 0000"),
            "the top row's picker is pruned once its row stops being built"
        );
        assert!(
            p.has_picker("QUEUE MERCHANT 1999"),
            "the bottom row is on screen, so it has a picker"
        );
        assert!(
            p.picker_count() < 100,
            "{} pickers at the bottom — retention must stay bounded",
            p.picker_count()
        );
        // 3.34: subscriptions retire with their pickers, one per survivor.
        assert_eq!(p.picker_sub_count(), p.picker_count());
    });
    cx.update(|cx| {
        shell
            .read(cx)
            .queue_view
            .read(cx)
            .scroll
            .set_offset(Point::default());
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert!(
            p.has_picker("QUEUE MERCHANT 0000"),
            "scrolling back rebuilds the row's picker lazily"
        );
    });
    // The rebuilt picker offers the current categories: picking the queue's
    // only child category lands on its id value (3.59: the grouped items
    // carry the id, the label is the menu row).
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    queue_pick(cx, handle, &shell, "QUEUE MERCHANT 0000", child);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        let picker = p.picker("QUEUE MERCHANT 0000").expect("picker exists");
        assert_eq!(
            picker.read(cx).selected_value().copied(),
            Some(child),
            "the rebuilt picker holds the current category choices"
        );
    });
    // A pick is a pending action: far away as the row is, the picker with
    // an active selection survives the prune.
    cx.update(|cx| {
        shell
            .read(cx)
            .queue_view
            .read(cx)
            .scroll
            .set_offset(point(px(0.), px(-1_000_000.)));
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert!(
            p.has_picker("QUEUE MERCHANT 0000"),
            "a picker with an active selection is never pruned mid-interaction"
        );
        assert_eq!(
            p.picker("QUEUE MERCHANT 0000")
                .unwrap()
                .read(cx)
                .selected_value()
                .copied(),
            Some(child),
            "the pick itself survives the scroll"
        );
    });
}

/// H4: exact-page scrolls. Scrolling to the offset where the previous
/// pass ended makes the built ids continue with no break (PageDown at a
/// viewport height that's a multiple of the 40 px row); the old sequence
/// heuristic only reset the paint window on a break, so window and picker
/// map grew with every page. The exact pass range from
/// `DataList::on_pass` moves the window page to page instead, and the
/// map stays the size of the visible slice.
#[gpui::test]
fn exact_page_scrolls_move_the_window_instead_of_growing_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| queue_len(&shell, cx) == 2_000);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    // Walk down twenty exact pages: each offset puts the row the previous
    // pass ended on right at the viewport top, so the new pass starts
    // exactly at the old pass's end.
    for page in 0..20 {
        let end = cx.update(|cx| {
            let p = shell.read(cx).queue_view.read(cx);
            let end = p.range.get().1;
            p.scroll.set_offset(point(px(0.), px(-(end as f32 * 40.))));
            end
        });
        draw_frame(cx, handle);
        cx.update(|cx| {
            let p = shell.read(cx).queue_view.read(cx);
            let (start, next_end) = p.range.get();
            assert_eq!(
                start, end,
                "page {page}: the ids continued at {end}, no break"
            );
            assert!(
                next_end > start && p.rows_built.get() < 100,
                "page {page}: the window is one screenful, not the walk ({start}..{next_end})",
            );
            assert!(
                p.picker_count() < 100,
                "page {page}: {} pickers after exact-page scrolls — the map must stay \
                 the size of the window",
                p.picker_count()
            );
        });
    }
    // Nothing walked past survives: the first page left the window
    // twenty screens ago.
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        assert!(
            !p.has_picker("QUEUE MERCHANT 0000"),
            "a picker an exact-page walk left behind is pruned"
        );
    });
}

/// H4 (direct): passes whose ids continue exactly at the previous
/// pass's end — the case the 3.91 review flagged, where nothing breaks
/// the sequence for the old heuristic to see. The harness lays out
/// before it paints, and measuring re-builds row 0, so even an exact
/// scrolled frame breaks the sequence in a test; the no-break walk can
/// only be driven through the pass hook itself. `note_paint_pass` takes
/// the exact range `DataList::on_pass` reports, so every pass moves the
/// prune window and the map stops growing with the walk.
#[gpui::test]
fn a_pass_that_continues_the_ids_still_moves_the_window(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| queue_len(&shell, cx) == 2_000);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        let window = p.range.get().1;
        assert!(
            p.has_picker("QUEUE MERCHANT 0000"),
            "the top page has pickers"
        );
        // Walk down the queue one exact page at a time: every pass
        // starts where the last ended, the ids continuing with no break.
        for page in 1.. {
            let lo = page * window;
            if lo + window > p.groups.len() {
                break;
            }
            p.note_paint_pass(lo..lo + window);
            p.prune_visible(cx);
            assert_eq!(
                p.rows_built.get(),
                window,
                "page {page}: a pass is one window"
            );
            assert!(
                p.picker_count() < 100,
                "page {page}: {} pickers — the map must stop growing with the walk",
                p.picker_count()
            );
        }
        assert!(
            !p.has_picker("QUEUE MERCHANT 0000"),
            "the top page's pickers are pruned even though the ids never broke"
        );
    });
}

/// 3.27: categorizing a queue group answers with a toast, and the toast is an
/// overlay — the top of the page paints in exactly the same place with a card
/// showing, where the old inline status line used to shift everything down.
#[gpui::test]
fn categorizing_a_queue_group_toasts_without_moving_the_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 4));
    wait_until(cx, "4 queue groups", |cx| queue_len(&shell, cx) == 4);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    let top = bounds(cx, handle, "queue:merchant-col:QUEUE MERCHANT 0000");
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    let merchant = "QUEUE MERCHANT 0001".to_string();
    queue_pick(cx, handle, &shell, &merchant, child);
    queue_categorize(cx, handle, &shell, merchant.clone());
    wait_until(cx, "toast up and queue shrinks", |cx| {
        !shell.read(cx).toasts.read(cx).is_empty()
            && shell.read(cx).queue_view.read(cx).groups.len() == 3
    });
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some((format!("Categorized {merchant}"), false))
        );
    });
    draw_frame(cx, handle);
    let after = bounds(cx, handle, "queue:merchant-col:QUEUE MERCHANT 0000");
    assert_eq!(
        after.origin, top.origin,
        "the page did not move while the toast shows"
    );
    let card = bounds(cx, handle, format!("toast:Categorized {merchant}"));
    assert!(
        card.center().x > px(550.) && card.center().y > px(360.),
        "the toast sits in the bottom-right of the 1200 × 720 window, at {card:?}"
    );
}

/// 3.16: every queue column, the category picker included, has a visible
/// heading above the rows — the picker's placeholder alone is not a label.
#[gpui::test]
fn the_queue_page_labels_its_columns(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    import_and_top_merchant(cx, handle, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    let mut window = VisualTestContext::from_window(handle, cx);
    cx.run_until_parked();
    draw_frame(cx, handle);
    // 3.91: the heading row belongs to the shared `DataList`, whose
    // heading selectors are `queue:heading:<column>`.
    for sel in [
        "queue:heading:Merchant",
        "queue:heading:Transactions",
        "queue:heading:Total",
        "queue:heading:Dates",
        "queue:heading:Rule",
        "queue:heading:Category",
        "queue:heading:Categorize",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the queue paints the column heading {sel:?}"
        );
    }
}

/// The heading row reserves the same tail as the rows (picker + Categorize
/// button) and shares the QUEUE_*_W column widths, so every heading sits
/// over its own column — measured at the 1200 × 720 minimum window, where
/// a row just fits (3.16, extended for 3.22).
#[gpui::test]
fn the_queue_category_heading_sits_over_the_picker_column(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (merchant, _, _) = import_and_top_merchant(cx, handle, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    cx.run_until_parked();
    draw_frame(cx, handle);
    for (label, sel) in [
        ("Merchant", format!("queue:merchant-col:{merchant}")),
        ("Transactions", format!("queue:txns-col:{merchant}")),
        ("Total", format!("queue:total-col:{merchant}")),
        ("Dates", format!("queue:dates-col:{merchant}")),
        // 3.95: the Rule column, input and switch together.
        ("Rule", format!("queue:rule-col:{merchant}")),
        ("Category", format!("queue:picker-col:{merchant}")),
    ] {
        let heading = bounds(cx, handle, format!("queue:heading:{label}"));
        let column = bounds(cx, handle, sel);
        assert!(
            (heading.origin.x - column.origin.x).abs() < px(1.),
            "the {label:?} heading starts where its column starts: \
             heading x {}, column x {}",
            heading.origin.x,
            column.origin.x
        );
    }
    let heading = window
        .debug_bounds("queue:heading:Category")
        .expect("the queue paints a Category heading");
    let picker = bounds(cx, handle, format!("queue:picker-col:{merchant}"));
    assert!(
        (heading.size.width - picker.size.width).abs() < px(1.),
        "the Category heading is as wide as the picker column: \
         heading w {}, picker w {}",
        heading.size.width,
        picker.size.width
    );
    // 3.95: the Rule heading spans the rule column too, and the row's
    // pattern input and regex switch sit inside that column, not spilling
    // into the picker's.
    let heading = window
        .debug_bounds("queue:heading:Rule")
        .expect("the queue paints a Rule heading");
    let column = bounds(cx, handle, format!("queue:rule-col:{merchant}"));
    assert!(
        (heading.size.width - column.size.width).abs() < px(1.),
        "the Rule heading is as wide as the rule column: \
         heading w {}, column w {}",
        heading.size.width,
        column.size.width,
    );
    for (what, sel) in [
        ("pattern input", format!("queue:rule-input:{merchant}")),
        ("regex switch", format!("queue:regex-slot:{merchant}")),
    ] {
        let b = bounds(cx, handle, sel);
        assert!(
            b.origin.x >= column.origin.x - px(1.)
                && b.origin.x + b.size.width <= column.origin.x + column.size.width + px(1.),
            "the row's {what} sits inside the Rule column: {b:?} in {column:?}",
        );
    }
    assert!(
        heading.origin.y < picker.origin.y,
        "the heading is above the rows: heading y {}, picker y {}",
        heading.origin.y,
        picker.origin.y
    );
}

/// The queue rows for the 1200 × 720 fit test: the first merchant's name is
/// exactly 120 characters, the others are ordinary ones.
const QUEUE_FIT_MERCHANTS: [&str; 3] = [
    "LONG MERCHANT NAME THAT WILL NOT FIT THE QUEUE COLUMN AT MINIMUM WINDOW SIZE SO IT MUST TRUNCATE AND SHOW FULL TEXT IN A",
    "CORNER CAFE",
    "TRANSIT FARE",
];

/// One uncategorized group per name in [`QUEUE_FIT_MERCHANTS`].
fn seed_queue_fit(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    for (i, name) in QUEUE_FIT_MERCHANTS.iter().enumerate() {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -123456, ?5)",
            (
                acct,
                import,
                i as i64,
                format!("fit-hash-{i}"),
                name.to_string(),
            ),
        )
        .unwrap();
    }
}

/// 3.22: at the 1200 × 720 minimum window every queue row keeps its category
/// picker and Categorize button fully inside the page; a 120-character
/// merchant name truncates with its full text in a hover tooltip, and the
/// Transactions cell shows just the number.
#[gpui::test]
fn the_queue_fits_the_minimum_window(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_queue_fit);
    wait_until(cx, "3 queue groups", |cx| queue_len(&shell, cx) == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    cx.run_until_parked();
    draw_frame(cx, handle);

    // The row's right edge — its Categorize button — ends inside the page
    // for every row, and so does the picker next to it.
    let page_right = px(1200. - PAGE_PAD);
    for m in QUEUE_FIT_MERCHANTS {
        for (what, sel) in [
            // 3.95: the row's rule controls ride along in the row too.
            ("regex switch", format!("queue:regex-slot:{m}")),
            ("category picker", format!("queue:picker-col:{m}")),
            ("Categorize button", format!("queue:categorize-col:{m}")),
        ] {
            let b = bounds(cx, handle, sel);
            assert!(
                b.origin.x + b.size.width <= page_right + px(1.),
                "the {what} of {m:?} ends at x {:?}, past the page edge {page_right:?}",
                b.origin.x + b.size.width,
            );
        }
    }

    // The 120-character name truncates: the Merchant cell is far narrower
    // than the full text needs at 14 px.
    let long = QUEUE_FIT_MERCHANTS[0];
    let cell = bounds(cx, handle, format!("queue:merchant-col:{long}"));
    let full_w = TEXT_SM_CHAR_PX * long.chars().count() as f32;
    assert!(
        cell.size.width < px(full_w / 2.),
        "the merchant name truncates: the cell is {:?} wide, the full text \
         needs about {full_w:?}",
        cell.size.width,
    );

    // The Transactions cell shows the bare number, not "N transactions".
    assert!(
        paints(cx, handle, format!("queue:txns-text:{long}:1")),
        "the Transactions cell shows the bare count"
    );
    assert!(
        !paints(cx, handle, format!("queue:txns-text:{long}:1 transactions")),
        "the old \"N transactions\" text is gone"
    );

    // Hovering the truncated name shows the tooltip with the full text.
    assert!(
        !paints(cx, handle, format!("queue:merchant-tooltip:{long}")),
        "nothing hovers the row, so no tooltip is drawn yet"
    );
    let centre = point(
        cell.origin.x + cell.size.width / 2.,
        cell.origin.y + cell.size.height / 2.,
    );
    window.simulate_mouse_move(centre, None, Modifiers::default());
    // The tooltip appears after a 500 ms hover; the test clock is fake.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("queue:merchant-tooltip:{long}")),
        "hovering the truncated merchant name shows a tooltip with the full name",
    );
}

/// 3.34: the queue list lives in one bordered panel that fills the rest of
/// the page — the heading row and the rows both paint inside it, below the
/// header summary, and nothing sits loose on the page background.
#[gpui::test]
fn the_queue_list_paints_inside_one_panel(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 3));
    wait_until(cx, "3 queue groups", |cx| queue_len(&shell, cx) == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    let mut window = VisualTestContext::from_window(handle, cx);
    cx.run_until_parked();
    draw_frame(cx, handle);
    let panel = window
        .debug_bounds("queue:panel")
        .expect("the queue page paints one bordered panel");
    // The panel fits the default 1200 × 720 window.
    assert!(
        panel.origin.x >= px(0.)
            && panel.origin.y >= px(0.)
            && panel.origin.x + panel.size.width <= px(1200.) + px(1.)
            && panel.origin.y + panel.size.height <= px(720.) + px(1.),
        "the panel fits the window: {panel:?}",
    );
    // The heading row and the first queue row paint inside the panel, with
    // the heading above the rows.
    let heading = window
        .debug_bounds("queue:heading:Merchant")
        .expect("the panel's heading row paints");
    let row = bounds(cx, handle, "queue:row:QUEUE MERCHANT 0000");
    assert!(
        heading.origin.x >= panel.origin.x
            && heading.origin.y >= panel.origin.y
            && row.origin.x >= panel.origin.x
            && row.origin.y + row.size.height <= panel.origin.y + panel.size.height + px(1.),
        "the heading {heading:?} and the row {row:?} sit inside the panel {panel:?}",
    );
    assert!(
        heading.origin.y < row.origin.y,
        "the heading is above the rows: heading y {}, row y {}",
        heading.origin.y,
        row.origin.y,
    );
}

/// 3.34: the header carries a muted summary with both counts — transactions
/// and merchants — through `flow::count`.
#[gpui::test]
fn the_queue_header_summarizes_transactions_and_merchants(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 3));
    wait_until(cx, "3 queue groups", |cx| queue_len(&shell, cx) == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    let txns = cx.update(|cx| {
        shell
            .read(cx)
            .queue_view
            .read(cx)
            .groups
            .iter()
            .map(|g| g.count)
            .sum::<i64>()
    });
    assert_eq!(txns, 6, "3 groups of 2 transactions each");
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "queue:summary:6 transactions · 3 merchants"),
        "the header shows the muted \"6 transactions · 3 merchants\" summary",
    );
}

/// 3.34: a row's Categorize button mirrors its picker — disabled with no
/// pick, enabled once the row's picker holds a category — while rows without
/// a pick stay disabled.
#[gpui::test]
fn the_queue_categorize_button_waits_for_its_picker(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 3));
    wait_until(cx, "3 queue groups", |cx| queue_len(&shell, cx) == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    draw_frame(cx, handle);
    let merchant = "QUEUE MERCHANT 0000".to_string();
    assert!(
        paints(
            cx,
            handle,
            format!("queue:categorize-state:{merchant}:disabled")
        ),
        "with no pick the row's Categorize is disabled",
    );
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    queue_pick(cx, handle, &shell, &merchant, child);
    draw_frame(cx, handle);
    assert!(
        paints(
            cx,
            handle,
            format!("queue:categorize-state:{merchant}:enabled")
        ),
        "after picking a category the row's Categorize is enabled",
    );
    assert!(
        !paints(
            cx,
            handle,
            format!("queue:categorize-state:{merchant}:disabled")
        ),
        "the disabled variant is gone",
    );
    assert!(
        paints(
            cx,
            handle,
            "queue:categorize-state:QUEUE MERCHANT 0001:disabled"
        ),
        "a row whose picker has no pick stays disabled",
    );
}

/// 3.83: Escape on the To-categorize page is the page's own `Cancel`, and
/// the queue's one transient is its error banner: the first press clears
/// it, and the consumed keystroke must not reach the dashboard drill.
/// With nothing left the keystroke no longer means anything here — 3.82
/// moved the drill-clearing Escape onto the Dashboard page itself, so the
/// drill waits for an Escape *there*.
#[gpui::test]
fn escape_on_the_queue_page_clears_the_error_then_propagates(cx: &mut TestAppContext) {
    // `seed_chart_rows` leaves one uncategorized row ("Mystery Vendor"),
    // so the queue has a group to fail on, and the chart has rows to drill.
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "chart rows load", |cx| {
        dash(cx, &shell).read(cx).chart_rows.len() == 3
    });
    // Drill the dashboard chart first: the drill lives on the dashboard
    // page entity and survives the page switch, so it waits here for the
    // last Escape.
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled into Food", |cx| {
        dash(cx, &shell).read(cx).filter.chart_parent == Some(food)
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "queue populated", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    draw_frame(cx, handle);
    // The banner's real trigger: Categorize with no pick in the row.
    queue_categorize(cx, handle, &shell, "Mystery Vendor".to_string());
    draw_frame(cx, handle);
    assert!(
        paints(
            cx,
            handle,
            "queue:error:Choose a category for this merchant first."
        ),
        "the refused categorize raises the error banner"
    );

    // Actions dispatch along the focus path, so focus the page root —
    // what showing the page does for a user (see the 3.80 Imports test).
    let root = cx.update(|cx| shell.read(cx).queue_view.read(cx).focus.clone());
    with_window(cx, handle, |window, cx| window.focus(&root, cx));

    // 1. The first Escape closes the banner; the drill keeps its drill.
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            shell.read(cx).queue_view.read(cx).error.is_none(),
            "Escape cleared the error banner"
        );
        assert_eq!(
            dash(cx, &shell).read_with(cx, |d, _| d.filter.chart_parent),
            Some(food),
            "the consumed Escape did not clear the drill"
        );
    });
    draw_frame(cx, handle);
    assert!(
        !paints(
            cx,
            handle,
            "queue:error:Choose a category for this merchant first."
        ),
        "the banner is gone from the page"
    );

    // 2. With nothing open the keystroke means nothing here: the pages
    // keep their own Escapes now (3.82), so the drill survives one
    // pressed on the queue. Visit the dashboard and press it once —
    // Escape still means *clear* on the page that owns the drill.
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            dash(cx, &shell).read_with(cx, |d, _| d.filter.chart_parent),
            Some(food),
            "Escape on the queue page leaves the drill alone"
        );
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    let dash_root = cx.update(|cx| dash(cx, &shell).read(cx).focus.clone());
    with_window(cx, handle, |window, cx| window.focus(&dash_root, cx));
    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "drill and legend selection cleared", |cx| {
        dash(cx, &shell).read(cx).filter.chart_parent.is_none()
            && dash(cx, &shell).read(cx).filter.legend_selection.is_none()
    });
}
