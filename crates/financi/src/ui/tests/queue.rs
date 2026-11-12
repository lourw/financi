//! Tests for `ui/queue.rs` (3.62 split from `ui_tests.rs`).

use super::*;
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
    wait_until(cx, "import", |cx| shell.read(cx).row_count == 67);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());

    wait_until(cx, "queue populated", |cx| !shell.read(cx).queue.is_empty());
    let (merchant, before) = cx.update(|cx| top_group(&shell, cx));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            // 3.19: pickers are built for visible rows; `queue_picker` is
            // the same get-or-create the row's render makes.
            let picker = s.queue_picker(&merchant, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.categorize_merchant(merchant.clone(), window, cx);
        });
    });
    let gone = merchant.clone();
    let shell_for_gone = shell.clone();
    wait_until(cx, "queue group gone", move |cx| {
        !shell_for_gone
            .read(cx)
            .queue
            .iter()
            .any(|g| g.merchant == gone)
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

/// 3.19: with thousands of merchant groups the To-categorize page must still
/// render, building only the visible rows — and only pickers for those.
#[gpui::test]
fn the_queue_page_builds_only_the_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| {
        shell.read(cx).queue.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.queue_rows_built.get() > 0 && s.queue_rows_built.get() < 100,
            "the queue built {} rows of 2000 — it must render a window",
            s.queue_rows_built.get()
        );
        // 3.19: pickers come with the rows that rendered, not one per group.
        assert!(
            s.queue_pickers.len() < 100,
            "{} pickers for a {}-row window — pickers must be built per visible row",
            s.queue_pickers.len(),
            s.queue_rows_built.get()
        );
        assert!(
            s.queue_pickers.contains_key("QUEUE MERCHANT 0000"),
            "the first visible row has a picker"
        );
        assert!(
            !s.queue_pickers.contains_key("QUEUE MERCHANT 1999"),
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
    wait_until(cx, "2000 queue groups", |cx| {
        shell.read(cx).queue.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    let last = cx.update(|cx| {
        let s = shell.read(cx);
        s.queue.last().expect("queue has a bottom").merchant.clone()
    });
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let picker = s.queue_picker(&last, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.categorize_merchant(last.clone(), window, cx);
        });
    });
    let gone = last.clone();
    let shell_for_gone = shell.clone();
    wait_until(cx, "bottom queue group gone", move |cx| {
        !shell_for_gone
            .read(cx)
            .queue
            .iter()
            .any(|g| g.merchant == gone)
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
            s.queue_rows_built.get() < 100,
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
    wait_until(cx, "2000 queue groups", |cx| {
        shell.read(cx).queue.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.queue_scroll.offset(),
            Point::default(),
            "starts unscrolled"
        );
        assert_eq!(s.queue_range.get().0, 0, "the first frame shows the top");
        // Jump straight to the bottom; layout clamps to the scroll max.
        s.queue_scroll.set_offset(point(px(0.), px(-1_000_000.)));
    });
    draw_frame(cx, handle);
    let scrolled = cx.update(|cx| shell.read(cx).queue_scroll.offset().y);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(scrolled < px(0.), "the queue is scrolled down");
        let (start, end) = s.queue_range.get();
        assert!(
            start > 0,
            "the built window moved off the top (start {start})"
        );
        assert_eq!(
            end,
            s.queue.len(),
            "the built window reached the last group"
        );
        // Pickers come with the rows that rendered: the bottom group's
        // picker is there, a middle group that was never on screen isn't.
        assert!(
            s.queue_pickers.contains_key("QUEUE MERCHANT 1999"),
            "the bottom group is on screen, so it has a picker"
        );
        assert!(
            !s.queue_pickers.contains_key("QUEUE MERCHANT 1000"),
            "a middle group never scrolled into view, so it must not have a picker"
        );
    });
    // A refresh that doesn't replace the queue must not move the offset.
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.queue_scroll.offset().y,
            scrolled,
            "the offset survived an unrelated notify"
        );
        assert_eq!(s.queue_range.get().1, s.queue.len());
    });
}

/// 3.27: categorizing a queue group answers with a toast, and the toast is an
/// overlay — the top of the page paints in exactly the same place with a card
/// showing, where the old inline status line used to shift everything down.
#[gpui::test]
fn categorizing_a_queue_group_toasts_without_moving_the_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 4));
    wait_until(cx, "4 queue groups", |cx| shell.read(cx).queue.len() == 4);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    let mut vtc = VisualTestContext::from_window(handle, cx);
    let top = vtc
        .debug_bounds(leaked("queue:merchant-col:QUEUE MERCHANT 0000".to_string()))
        .expect("the first queue row is painted");
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let merchant = "QUEUE MERCHANT 0001".to_string();
            let picker = s.queue_picker(&merchant, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.categorize_merchant(merchant, window, cx);
        });
    });
    wait_until(cx, "toast up and queue shrinks", |cx| {
        !shell.read(cx).toasts.is_empty() && shell.read(cx).queue.len() == 3
    });
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Categorized QUEUE MERCHANT 0001".into(), false))
        );
    });
    draw_frame(cx, handle);
    let after = vtc
        .debug_bounds(leaked("queue:merchant-col:QUEUE MERCHANT 0000".to_string()))
        .expect("the first queue row is still painted");
    assert_eq!(
        after.origin, top.origin,
        "the page did not move while the toast shows"
    );
    let card = vtc
        .debug_bounds(leaked("toast:Categorized QUEUE MERCHANT 0001".to_string()))
        .expect("the toast card is painted");
    assert!(
        card.center().x > px(550.) && card.center().y > px(360.),
        "the toast sits in the bottom-right of the 1100 × 720 window, at {card:?}"
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
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for sel in [
        "label:Merchant",
        "label:Transactions",
        "label:Total",
        "label:Dates",
        "label:Category",
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
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for (label, sel) in [
        ("Merchant", format!("queue:merchant-col:{merchant}")),
        ("Transactions", format!("queue:txns-col:{merchant}")),
        ("Total", format!("queue:total-col:{merchant}")),
        ("Dates", format!("queue:dates-col:{merchant}")),
        ("Category", format!("queue:picker-col:{merchant}")),
    ] {
        let heading = window
            .debug_bounds(leaked(format!("label:{label}")))
            .unwrap_or_else(|| panic!("the queue paints the {label:?} heading"));
        let column = window
            .debug_bounds(leaked(sel))
            .unwrap_or_else(|| panic!("the queue paints the {label:?} column"));
        assert!(
            (heading.origin.x - column.origin.x).abs() < px(1.),
            "the {label:?} heading starts where its column starts: \
             heading x {}, column x {}",
            heading.origin.x,
            column.origin.x
        );
    }
    let heading = window
        .debug_bounds("label:Category")
        .expect("the queue paints a Category heading");
    let picker = window
        .debug_bounds(leaked(format!("queue:picker-col:{merchant}")))
        .expect("the queue paints a picker column");
    assert!(
        (heading.size.width - picker.size.width).abs() < px(1.),
        "the Category heading is as wide as the picker column: \
         heading w {}, picker w {}",
        heading.size.width,
        picker.size.width
    );
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
    wait_until(cx, "3 queue groups", |cx| shell.read(cx).queue.len() == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // The row's right edge — its Categorize button — ends inside the page
    // for every row, and so does the picker next to it.
    let page_right = px(1200. - PAGE_PAD);
    for m in QUEUE_FIT_MERCHANTS {
        for (what, sel) in [
            ("category picker", format!("queue:picker-col:{m}")),
            ("Categorize button", format!("queue:categorize-col:{m}")),
        ] {
            let b = window
                .debug_bounds(leaked(sel))
                .unwrap_or_else(|| panic!("the row for {m:?} draws its {what}"));
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
    let cell = window
        .debug_bounds(leaked(format!("queue:merchant-col:{long}")))
        .expect("the long merchant's row draws its cell");
    let full_w = TEXT_SM_CHAR_PX * long.chars().count() as f32;
    assert!(
        cell.size.width < px(full_w / 2.),
        "the merchant name truncates: the cell is {:?} wide, the full text \
         needs about {full_w:?}",
        cell.size.width,
    );

    // The Transactions cell shows the bare number, not "N transactions".
    assert!(
        window
            .debug_bounds(leaked(format!("queue:txns-text:{long}=1")))
            .is_some(),
        "the Transactions cell shows the bare count"
    );
    assert!(
        window
            .debug_bounds(leaked(format!("queue:txns-text:{long}=1 transactions")))
            .is_none(),
        "the old \"N transactions\" text is gone"
    );

    // Hovering the truncated name shows the tooltip with the full text.
    assert!(
        window
            .debug_bounds(leaked(format!("queue:merchant-tooltip:{long}")))
            .is_none(),
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
        window
            .debug_bounds(leaked(format!("queue:merchant-tooltip:{long}")))
            .is_some(),
        "hovering the truncated merchant name shows a tooltip with the full name",
    );
}

/// 3.34: the queue list lives in one bordered panel that fills the rest of
/// the page — the heading row and the rows both paint inside it, below the
/// header summary, and nothing sits loose on the page background.
#[gpui::test]
fn the_queue_list_paints_inside_one_panel(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 3));
    wait_until(cx, "3 queue groups", |cx| shell.read(cx).queue.len() == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    let panel = window
        .debug_bounds("queue:panel")
        .expect("the queue page paints one bordered panel");
    // The panel fits the default 1100 × 720 window.
    assert!(
        panel.origin.x >= px(0.)
            && panel.origin.y >= px(0.)
            && panel.origin.x + panel.size.width <= px(1100.) + px(1.)
            && panel.origin.y + panel.size.height <= px(720.) + px(1.),
        "the panel fits the window: {panel:?}",
    );
    // The heading row and the first queue row paint inside the panel, with
    // the heading above the rows.
    let heading = window
        .debug_bounds("label:Merchant")
        .expect("the panel's heading row paints");
    let row = window
        .debug_bounds(leaked("queue:row:QUEUE MERCHANT 0000".to_string()))
        .expect("the panel paints the queue's rows");
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
    wait_until(cx, "3 queue groups", |cx| shell.read(cx).queue.len() == 3);
    let txns = cx.update(|cx| shell.read(cx).queue.iter().map(|g| g.count).sum::<i64>());
    assert_eq!(txns, 6, "3 groups of 2 transactions each");
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds(leaked(
                "queue:summary:6 transactions · 3 merchants".to_string()
            ))
            .is_some(),
        "the header shows the muted \"6 transactions · 3 merchants\" summary",
    );
}

/// 3.34: a row's Categorize button mirrors its picker — disabled with no
/// pick, enabled once the row's picker holds a category — while rows without
/// a pick stay disabled.
#[gpui::test]
fn the_queue_categorize_button_waits_for_its_picker(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 3));
    wait_until(cx, "3 queue groups", |cx| shell.read(cx).queue.len() == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    let merchant = "QUEUE MERCHANT 0000".to_string();
    assert!(
        window
            .debug_bounds(leaked(format!(
                "queue:categorize-state:{merchant}=disabled"
            )))
            .is_some(),
        "with no pick the row's Categorize is disabled",
    );
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let picker = s.queue_picker(&merchant, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            // The popup's pick emits `Confirm`, which re-renders the rows;
            // `set_selected_index` drives the state directly, so the test
            // notifies on the subscription's behalf.
            cx.notify();
        });
    });
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds(leaked(format!("queue:categorize-state:{merchant}=enabled")))
            .is_some(),
        "after picking a category the row's Categorize is enabled",
    );
    assert!(
        window
            .debug_bounds(leaked(format!(
                "queue:categorize-state:{merchant}=disabled"
            )))
            .is_none(),
        "the disabled variant is gone",
    );
    assert!(
        window
            .debug_bounds(leaked(
                "queue:categorize-state:QUEUE MERCHANT 0001=disabled".to_string()
            ))
            .is_some(),
        "a row whose picker has no pick stays disabled",
    );
}
