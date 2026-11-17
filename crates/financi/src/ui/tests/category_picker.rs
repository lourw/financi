//! 3.59: the shared searchable category pickers, driven through the
//! To-categorize row picker (the same `SubChoices` delegate every
//! converted picker uses) over a deliberately large tree: 200 categories
//! with 10 sub-categories each, plus two same-named sub-categories.

use super::*;

const PICKER_MERCHANT: &str = "PICKER MERCHANT";

/// 3.59: synthetic world for the picker tests — 2,000 sub-categories
/// across 200 categories ("Cat 000"…"Cat 199", "Sub 000-00"…), a
/// "Duplicate" under Cat 000 and Cat 001, and one merchant group of two
/// uncategorized rows to pick a category for.
fn seed_picker_world(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    for c in 0..200 {
        let parent = financi_db::categories::create_parent(
            conn,
            &format!("Cat {c:03}"),
            CategoryKind::Expense,
        )
        .unwrap();
        for n in 0..10 {
            financi_db::categories::create_child(conn, parent, &format!("Sub {c:03}-{n:02}"))
                .unwrap();
        }
        if c < 2 {
            financi_db::categories::create_child(conn, parent, "Duplicate").unwrap();
        }
    }
    for n in 0..2 {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
            (acct, import, n, format!("picker-hash-{n}"), PICKER_MERCHANT),
        )
        .unwrap();
    }
}

/// Open the shell over `seed_picker_world` and show the queue, rows built.
fn open_picker_queue(cx: &mut TestAppContext) -> (gpui::AnyWindowHandle, Entity<Shell>) {
    let (handle, shell) = open_shell(cx, seed_picker_world);
    wait_until(cx, "the queue loads", |cx| {
        shell.read(cx).catalog.read(cx).snap.queue.len() == 1
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "the queue page applies", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    draw_frame(cx, handle);
    (handle, shell)
}

/// Open the row's picker menu with a real click (the 3.53 pattern: two
/// frames so the deferred popover lays out and paints its rows).
fn open_picker_menu(cx: &mut TestAppContext, handle: gpui::AnyWindowHandle) {
    click(cx, handle, format!("queue:picker-col:{PICKER_MERCHANT}"));
    draw_frame(cx, handle);
    draw_frame(cx, handle);
}

/// Type `text` into whatever holds focus — the menu's search box once the
/// picker is open — and paint.
fn type_into_menu(cx: &mut TestAppContext, handle: gpui::AnyWindowHandle, text: &str) {
    {
        let mut window = VisualTestContext::from_window(handle, cx);
        window.simulate_input(text);
    }
    draw_frame(cx, handle);
}

/// 3.59: with 2,002 sub-categories the open menu must build only the
/// rows that paint — the virtualized list (the task's gate) keeps the
/// first rows up and the tail unbuilt.
#[gpui::test]
fn the_grouped_menu_builds_only_its_visible_rows(cx: &mut TestAppContext) {
    let (handle, _shell) = open_picker_queue(cx);
    open_picker_menu(cx, handle);
    assert!(
        paints(cx, handle, "cat:sub-choice:Cat 000 › Sub 000-00"),
        "the first sub-category paints, under its category"
    );
    assert!(
        paints(cx, handle, "cat:sub-choice:Cat 000 › Sub 000-01"),
        "the visible rows below it paint too"
    );
    assert!(
        !paints(cx, handle, "cat:sub-choice:Cat 199 › Sub 199-09"),
        "the menu built every one of the 2,002 rows — only the visible \
         window may be built"
    );
}

/// 3.59: typing a sub-category name (any case) leaves its row in the
/// menu — labelled with its category — and Enter picks it: the picker's
/// value is the sub-category id and the row's Categorize button enables.
#[gpui::test]
fn typing_a_subcategory_name_finds_it_and_enter_picks_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_picker_queue(cx);
    let id = cx.update(|cx| find_child(&shell, cx, "Sub 195-07").unwrap());
    open_picker_menu(cx, handle);
    type_into_menu(cx, handle, "sub 195-07");
    assert!(
        paints(cx, handle, "cat:sub-choice:Cat 195 › Sub 195-07"),
        "the typed sub-category shows, with its category in front"
    );
    assert!(
        !paints(cx, handle, "cat:sub-choice:Cat 000 › Sub 000-00"),
        "rows that do not match the query are gone from the menu"
    );
    {
        let mut window = VisualTestContext::from_window(handle, cx);
        window.simulate_keystrokes("enter");
    }
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "cat:sub-choice:Cat 195 › Sub 195-07"),
        "Enter closes the menu"
    );
    cx.update(|cx| {
        let p = shell.read(cx).queue_view.read(cx);
        let picker = p.picker(PICKER_MERCHANT).expect("the row's picker");
        assert_eq!(
            picker.read(cx).selected_value().copied(),
            Some(id),
            "Enter picked exactly the searched sub-category, by id"
        );
    });
    assert!(
        paints(
            cx,
            handle,
            format!("queue:categorize-state:{PICKER_MERCHANT}:enabled")
        ),
        "the row's Categorize button mirrors the pick"
    );
}

/// 3.59: typing a *category* name lists its sub-categories — every child
/// of "Cat 007" matches, another category's children do not.
#[gpui::test]
fn typing_a_category_name_lists_its_subcategories(cx: &mut TestAppContext) {
    let (handle, _shell) = open_picker_queue(cx);
    open_picker_menu(cx, handle);
    type_into_menu(cx, handle, "cat 007");
    for n in 0..3 {
        assert!(
            paints(
                cx,
                handle,
                format!("cat:sub-choice:Cat 007 › Sub 007-{n:02}")
            ),
            "the parent's own sub-category {n:02} stays listed"
        );
    }
    assert!(
        !paints(cx, handle, "cat:sub-choice:Cat 006 › Sub 006-00"),
        "the neighbouring category's children are filtered out"
    );
    assert!(
        !paints(cx, handle, "cat:sub-choice:Cat 008 › Sub 008-00"),
        "and so are the other side's"
    );
}

/// 3.59: two same-named sub-categories under different parents are both
/// listed while searching, each row carrying its category, so they can
/// be told apart and each picked.
#[gpui::test]
fn two_same_named_subcategories_both_appear_with_their_categories(cx: &mut TestAppContext) {
    let (handle, _shell) = open_picker_queue(cx);
    open_picker_menu(cx, handle);
    type_into_menu(cx, handle, "duplicate");
    assert!(
        paints(cx, handle, "cat:sub-choice:Cat 000 › Duplicate"),
        "the first same-named sub-category shows under its category"
    );
    assert!(
        paints(cx, handle, "cat:sub-choice:Cat 001 › Duplicate"),
        "the second one shows too — same name, different parent"
    );
}
