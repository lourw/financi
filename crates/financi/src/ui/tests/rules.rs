//! Tests for `ui/rules.rs` (3.62 split from `ui_tests.rs`).

use super::*;
#[gpui::test]
fn creating_a_rule_categorizes_matching_rows(cx: &mut TestAppContext) {
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
    wait_until(cx, "import", |cx| shell.read(cx).dashboard.row_count == 67);

    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let (merchant, rows) = cx.update(|cx| top_group(&shell, cx));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_pattern.update(cx, |i, cx| {
                i.set_value(merchant.clone(), window, cx);
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            s.rule_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.add_rule(window, cx);
        });
    });
    wait_until(cx, "rule added", |cx| !shell.read(cx).rules.is_empty());
    wait_until(cx, "rows categorized", |cx| {
        shell.read(cx).cat_counts.get(&child).copied().unwrap_or(0) >= rows
    });
}

/// Type into the open editor's Pattern input (the same `set_value` a user's
/// keystrokes end up as; it drives the live preview like a keystroke).
fn set_edit_pattern(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    value: &str,
) {
    let value = value.to_string();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_edit_pattern
                .update(cx, |i, cx| i.set_value(value, window, cx));
            s.preview_rule_match(cx);
        });
    });
}

/// Press the editor's Save — the same `save_rule_edit` the button calls.
fn save_rule_edit(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.save_rule_edit(window, cx));
    });
}

/// 3.24: Edit swaps the row for a pre-filled editor — the rule's pattern
/// and regex kind are in the controls, and the editor is open on that
/// rule only. 3.53: the editor carries no category picker — the rule's
/// category is where it lives, not a field to edit.
#[gpui::test]
fn the_rule_editor_is_prefilled_from_the_rule(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, Some(rule), "the editor is open on the rule");
        assert_eq!(s.rule_edit_pattern.read(cx).value().to_string(), "COFFEE");
        assert!(!s.rule_edit_regex, "substring kind pre-filled");
    });
    // 3.53: the editor draws its own controls and nothing that picks a
    // category — saving keeps the rule's category (pinned by the next
    let mut window = VisualTestContext::from_window(handle, cx);
    // test), so there is no picker here.
    assert!(
        window.debug_bounds("rules:edit-pattern-input").is_some()
            && window.debug_bounds("rules:edit-save-btn").is_some(),
        "the editor paints its pattern field and Save"
    );
    wait_until(cx, "live match count for the edit", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
}

/// 3.24: editing a rule's pattern re-runs every rule over the unlocked
/// rows — the rows the new pattern drops go back to Uncategorized, the
/// ones it keeps stay on the rule's category, and the locked row keeps its
/// manual category even though the new pattern matches it.
#[gpui::test]
fn editing_a_rules_pattern_recategorizes_unlocked_rows(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "SHOP B");
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the edit saved", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule && r.pattern == "SHOP B")
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, None, "the editor closed on save");
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            1,
            "only SHOP B is still on the rule's category"
        );
        assert_eq!(
            s.cat_counts.get(&second).copied().unwrap_or(0),
            1,
            "the locked row keeps its manual category — the new pattern matches it, but locked rows are never re-run"
        );
        assert_eq!(
            s.uncategorized_visible, 1,
            "the dropped row went back to Uncategorized"
        );
    });
}

/// 3.53: the editor has no picker, so saving an edit — even with a new
/// pattern — keeps the rule on its own category, and the rows follow the
/// rule there. Moving a rule to another category is the Rules page's
/// Move bar (3.28), not the editor.
#[gpui::test]
fn saving_a_rule_edit_keeps_its_category(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "SHOP B");
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the edit saved", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule && r.pattern == "SHOP B")
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        let saved = s.rules.iter().find(|r| r.id == rule).unwrap();
        assert_eq!(saved.category_id, first, "the rule stayed on its category");
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            1,
            "the kept row is on `first`, where the rule still lives"
        );
        assert_eq!(
            s.cat_counts.get(&second).copied().unwrap_or(0),
            1,
            "`second` gained nothing — only its locked row is there"
        );
    });
}

/// 3.24: a bad regex keeps the editor open with the error in `danger`, and
/// nothing is saved — the rule and the categorized rows are unchanged.
#[gpui::test]
fn an_invalid_regex_shows_the_error_and_saves_nothing(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "[");
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.rule_edit_regex = true);
    });
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).rule_error.is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, Some(rule), "the editor stays open");
        assert_eq!(s.rules[0].pattern, "COFFEE", "the rule kept its pattern");
        assert!(!s.rules[0].is_regex, "and its kind");
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            2,
            "no rows moved"
        );
    });
}

/// 3.40: pressing Save puts the editor's Save button into its loading,
/// disabled state (`pending_rule_action`) for as long as the DB task runs,
/// and the editor closes once the task ends — the button never looks dead
/// mid-save and can't be double-submitted.
#[gpui::test]
fn saving_a_rule_shows_loading_until_the_task_ends(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "SHOP B");
    // The task hasn't run yet (the test executor only moves inside
    // `wait_until`), so pending is observed while the save is in flight.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.save_rule_edit(window, cx);
            assert_eq!(
                s.pending_rule_action,
                Some(PendingRuleAction::Save(rule)),
                "Save shows loading, disabled, while the save runs"
            );
        });
    });
    wait_until(cx, "the save task to finish", |cx| {
        shell.read(cx).pending_rule_action.is_none()
    });
    wait_until(cx, "the refreshed rule list", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule && r.pattern == "SHOP B")
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.pending_rule_action, None, "loading cleared");
        assert_eq!(s.editing_rule, None, "the editor closed after the task");
        assert_eq!(s.rule_error, None, "the save succeeded");
    });
}

/// 3.40: when the DB refuses the save (bad regex), the loading state
/// clears on the error path too — the editor stays open with its error,
/// and the button is clickable again.
#[gpui::test]
fn a_refused_rule_save_clears_the_loading_state(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "[");
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.rule_edit_regex = true);
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.save_rule_edit(window, cx);
            assert_eq!(s.pending_rule_action, Some(PendingRuleAction::Save(rule)));
        });
    });
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).pending_rule_action.is_none() && shell.read(cx).rule_error.is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.pending_rule_action, None,
            "loading cleared on the error path too"
        );
        assert_eq!(s.editing_rule, Some(rule), "the editor stays open");
    });
}

/// 3.24 review: a validation error open under the editor must stay inside
/// its row — the virtual list does not clip per item, so an error taller
/// than the reserved slot would paint over the next rule row. The error is
/// clamped to one ellipsized line and the slot grows while it is shown.
#[gpui::test]
fn the_editor_error_stays_inside_its_row(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, second) = open_rule_fixture(cx);
    // A second rule below the one being edited: its row is what an
    // overflowing error would overlap.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_pattern
                .update(cx, |i, cx| i.set_value("TOAST".to_string(), window, cx));
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == second)
                .unwrap();
            s.rule_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.add_rule(window, cx);
        });
    });
    wait_until(cx, "the second rule", |cx| shell.read(cx).rules.len() == 2);
    let toast = cx.update(|cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .find(|r| r.pattern == "TOAST")
            .unwrap()
            .id
    });

    // Edit the first rule and leave an invalid regex open — the multi-line
    // regex-crate message is the one that used to break the layout.
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "[");
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.rule_edit_regex = true);
    });
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).rule_error.is_some()
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    let err = window
        .debug_bounds("rules:edit-error")
        .expect("the error paints under the editor");
    let next = window
        .debug_bounds(format!("cat:rule-row:{toast}").leak())
        .expect("the row below still paints");
    assert!(
        err.bottom() <= next.top(),
        "the error (bottom {:?}) must not overlap the next rule row (top {:?})",
        err.bottom(),
        next.top()
    );
    assert!(
        err.size.height <= px(24.),
        "the error is one line (text_sm measures 22.5 px), not {:?}",
        err.size.height
    );
}

/// 3.24 review: while a row's editor is open the live count belongs to the
/// editor — the add-rule row must not show the editor's count (the same
/// filter `rule_error` has).
#[gpui::test]
fn the_add_rule_preview_stays_hidden_while_editing(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    wait_until(cx, "the editor preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("rules:edit-preview").is_some(),
        "the editor shows the live count"
    );
    assert!(
        window.debug_bounds("cat:add-rule-preview").is_none(),
        "the add-rule row shows none of it"
    );
    // Not merely never painted: with the editor closed and the add-rule
    // row's own pattern, the row shows its own count.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.cancel_edit_rule(cx);
            s.rule_pattern
                .update(cx, |i, cx| i.set_value("COFFEE".to_string(), window, cx));
            s.preview_rule_match(cx);
        });
    });
    wait_until(cx, "the add-rule preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:add-rule-preview").is_some(),
        "the add-rule row shows its own count once not editing"
    );
}

/// 3.24: Cancel and Escape both close the editor without touching the
/// rule.
#[gpui::test]
fn cancel_and_escape_leave_the_rule_unchanged(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);

    // Cancel button.
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "ZZZ");
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.cancel_edit_rule(cx);
            let _ = window;
        });
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, None, "Cancel closed the editor");
        assert_eq!(s.rules[0].pattern, "COFFEE", "Cancel saved nothing");
    });

    // Escape: the window-level binding cancels the innermost thing first.
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "ZZZ");
    with_window(cx, handle, |window, cx| {
        let input = shell.read(cx).rule_edit_pattern.read(cx).focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "Escape closed the editor", |cx| {
        shell.read(cx).editing_rule.is_none()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.rules[0].pattern, "COFFEE", "Escape saved nothing");
    });
}

/// 3.24: selecting another category cancels the open editor — and clears
/// the shared preview count with it, so the add-rule row never inherits a
/// stale editor count.
#[gpui::test]
fn selecting_another_category_cancels_the_rule_editor(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    wait_until(cx, "the editor preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.categories_page.selected_category, Some(first));
        assert_eq!(
            s.editing_rule, None,
            "selecting a category cancels the edit"
        );
        assert!(
            s.rule_match_count.is_none(),
            "closing the editor clears the shared preview count"
        );
        assert_eq!(s.rules[0].pattern, "COFFEE");
    });
}

/// 3.24: Delete… is the two-click confirm — the first click arms the row,
/// the second deletes; the rule is gone and its rows fall back to
/// Uncategorized (the locked row keeps its manual category).
#[gpui::test]
fn delete_rule_needs_two_clicks_and_uncategorizes_its_rows(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, second) = open_rule_fixture(cx);

    // First click: arms, nothing is deleted.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
        assert_eq!(
            shell.read(cx).armed_delete_rule,
            Some(rule),
            "the first click arms the row"
        );
        assert_eq!(shell.read(cx).rules.len(), 1, "and deletes nothing yet");
    });

    // Second click: deletes.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
    });
    wait_until(cx, "the rule deleted", |cx| shell.read(cx).rules.is_empty());
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.armed_delete_rule, None, "the armed state reset");
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            0,
            "the rule's rows fell back to Uncategorized"
        );
        assert_eq!(s.uncategorized_visible, 2);
        assert_eq!(
            s.cat_counts.get(&second).copied().unwrap_or(0),
            1,
            "the locked row kept its manual category"
        );
    });
}

/// 3.24: the armed delete is transient like the page's other two-click
/// confirmations — selecting a category disarms it, and so does switching
/// pages (which also closes an open rule editor).
#[gpui::test]
fn the_armed_rule_delete_is_transient(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
        assert_eq!(shell.read(cx).armed_delete_rule, Some(rule));
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
        assert_eq!(
            shell.read(cx).armed_delete_rule,
            None,
            "selecting a category disarms the delete"
        );
    });

    // A page switch closes the editor (and its preview count) and disarms
    // in one go.
    start_rule_edit(cx, handle, &shell, rule);
    wait_until(cx, "the editor preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, None, "the page switch closed the editor");
        assert!(
            s.rule_match_count.is_none(),
            "and cleared the shared preview count"
        );
        assert_eq!(s.armed_delete_rule, None, "and disarmed the delete");
    });
}

/// 3.24: with thousands of rules the row actions (Edit, Delete…) are built
/// only for the rows the virtualized list renders — buttons come into the
/// world inside `rule_row`, so the built-row window proves it. Opening an
/// editor on a visible row must not change that.
#[gpui::test]
fn rule_row_actions_are_built_only_for_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2000 rules", |cx| {
        shell.read(cx).rules.len() == 2_000 && shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: rules paint only under their selected category; all 2,000 of
    // them sit under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    let first_rule = cx.update(|cx| shell.read(cx).rules[0].id);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.categories_page.rule_rows_built.get() > 0 && s.categories_page.rule_rows_built.get() < 100,
            "the rule list built {} rows of 2000 — buttons come with each row, so it must render a window",
            s.categories_page.rule_rows_built.get()
        );
    });
    // The editor swaps one row for a taller block; the window stays small.
    start_rule_edit(cx, handle, &shell, first_rule);
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, Some(first_rule));
        assert!(
            s.categories_page.rule_rows_built.get() < 100,
            "still a window of rows with the editor open: {}",
            s.categories_page.rule_rows_built.get()
        );
    });
}

/// 3.28 fixture: rules and rows for the Rules page tests. Categories
/// Food › Coffee, Food › Snacks, Fun › Games. Rules:
/// - r1 "COFFEE" (substring) → Coffee: wins the two roast rows (Uses 2);
/// - r2 "COFFEE SHOP" (substring) → Coffee: wins the shop row (Uses 1);
/// - r3 "GAME" (substring) → Games: nothing matches (unused);
/// - r4 "^SNACK" (regex) → Snacks: unused.
///   "COFFEE SHOP LOCKED" is overridden to Games and locked — no rule
///   re-run may touch it — and "PARKING FEE" stays uncategorised. Returns
///   `(coffee, snacks, games, [r1, r2, r3, r4])`.
fn seed_rules_page(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64, [i64; 4]) {
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
    let coffee = financi_db::categories::create_child(conn, food, "Coffee").unwrap();
    let snacks = financi_db::categories::create_child(conn, food, "Snacks").unwrap();
    let fun = financi_db::categories::create_parent(conn, "Fun", CategoryKind::Expense).unwrap();
    let games = financi_db::categories::create_child(conn, fun, "Games").unwrap();
    for (n, desc) in [
        "COFFEE BEAN ROAST 1",
        "COFFEE BEAN ROAST 2",
        "COFFEE SHOP A",
        "COFFEE SHOP LOCKED",
        "PARKING FEE",
    ]
    .iter()
    .enumerate()
    {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
            (
                acct,
                import,
                n as i64,
                format!("rules-page-hash-{n}"),
                *desc,
            ),
        )
        .unwrap();
    }
    let r1 = financi_db::rules::add_rule(conn, "COFFEE", false, coffee).unwrap();
    let r2 = financi_db::rules::add_rule(conn, "COFFEE SHOP", false, coffee).unwrap();
    let r3 = financi_db::rules::add_rule(conn, "GAME", false, games).unwrap();
    let r4 = financi_db::rules::add_rule(conn, "^SNACK", true, snacks).unwrap();
    conn.execute(
        "UPDATE transactions SET category_id = ?1, category_locked = 1 \
         WHERE description_raw = 'COFFEE SHOP LOCKED'",
        [games],
    )
    .unwrap();
    (coffee, snacks, games, [r1, r2, r3, r4])
}

/// Open the shell over `seed_rules_page`, wait for the rules and their Uses
/// counts to load, and switch to the Rules page.
fn open_rules_page(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Shell>, i64, i64, i64, [i64; 4]) {
    let mut ids = (0i64, 0i64, 0i64, [0i64; 4]);
    let (handle, shell) = open_shell(cx, |c| ids = seed_rules_page(c));
    let (coffee, snacks, games, rules) = ids;
    wait_until(cx, "rules page fixture loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 4
            && s.rules_page.rule_uses.get(&rules[0]) == Some(&2)
            && s.rules_page.rule_uses.get(&rules[1]) == Some(&1)
            && s.cat_counts.get(&coffee) == Some(&3)
            && s.cat_counts.get(&games) == Some(&1)
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.run_until_parked();
    (handle, shell, coffee, snacks, games, rules)
}

/// Type into the Rules page search box (the same `set_value` a user's
/// keystrokes end up as).
fn set_rules_search(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    value: &str,
) {
    let value = value.to_string();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .rules_search
                .update(cx, |i, cx| i.set_value(value, window, cx));
        });
    });
    cx.run_until_parked();
}

/// 3.28: the list groups rules by their subcategory with a header row per
/// group ("Food › Coffee · 3 rules"), groups in label order, and rows
/// within a group by Pattern (the default).
#[gpui::test]
fn rules_page_groups_rows_by_target_category(cx: &mut TestAppContext) {
    let (handle, _shell, coffee, snacks, games, rules) = open_rules_page(cx);
    draw_frame(cx, handle);

    assert!(
        paints(cx, handle, "label:4 rules"),
        "the header counts the rules"
    );
    for id in [coffee, snacks, games] {
        assert!(
            paints(cx, handle, format!("rules:group:{id}")),
            "a group header for category {id}"
        );
    }
    for id in rules {
        assert!(
            paints(cx, handle, format!("rules:row-{id}")),
            "a row for rule {id}"
        );
    }
    // Groups sorted by label: Food › Coffee before Food › Snacks before
    // Fun › Games.
    assert!(
        centre_y(cx, handle, format!("rules:group:{coffee}")) < {
            draw_frame(cx, handle);
            centre_y(cx, handle, format!("rules:group:{snacks}"))
        }
    );
    assert!(
        centre_y(cx, handle, format!("rules:group:{snacks}")) < {
            draw_frame(cx, handle);
            centre_y(cx, handle, format!("rules:group:{games}"))
        }
    );
    // Within the Coffee group the rows are by Pattern: "COFFEE" above
    // "COFFEE SHOP".
    assert!(
        centre_y(cx, handle, format!("rules:row-{}", rules[0])) < {
            draw_frame(cx, handle);
            centre_y(cx, handle, format!("rules:row-{}", rules[1]))
        },
        "rows sort by Pattern within their group"
    );
}

/// 3.28: Search filters live over the pattern and the target category's
/// child and parent names, case-insensitively.
#[gpui::test]
fn rules_page_search_covers_patterns_and_category_names(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);

    // A pattern substring: only r2 ("COFFEE SHOP").
    set_rules_search(cx, handle, &shell, "shop");
    draw_frame(cx, handle);
    assert!(paints(cx, handle, format!("rules:row-{}", rules[1])));
    for id in [rules[0], rules[2], rules[3]] {
        assert!(
            !paints(cx, handle, format!("rules:row-{id}")),
            "r{id} does not match \"shop\""
        );
    }

    // A child-category name in another case: only r3 (targets Games).
    set_rules_search(cx, handle, &shell, "games");
    draw_frame(cx, handle);
    assert!(paints(cx, handle, format!("rules:row-{}", rules[2])));
    for id in [rules[0], rules[1], rules[3]] {
        assert!(
            !paints(cx, handle, format!("rules:row-{id}")),
            "rule {id} does not match \"games\""
        );
    }

    // A parent-category name: "fun" is r3's parent.
    set_rules_search(cx, handle, &shell, "fun");
    draw_frame(cx, handle);
    assert!(paints(cx, handle, format!("rules:row-{}", rules[2])));
    assert!(!paints(cx, handle, format!("rules:row-{}", rules[0])));

    // Nothing matches: the empty state, no rows.
    set_rules_search(cx, handle, &shell, "zzz");
    draw_frame(cx, handle);
    assert!(paints(cx, handle, "rules:empty"));
    for id in rules {
        assert!(!paints(cx, handle, format!("rules:row-{id}")));
    }
    // Clearing the box shows everything again.
    set_rules_search(cx, handle, &shell, "");
    draw_frame(cx, handle);
    assert!(!paints(cx, handle, "rules:empty"));
    for id in rules {
        assert!(paints(cx, handle, format!("rules:row-{id}")));
    }
}

/// 3.28: "Test a description" shows only the rules matching the text, in
/// the order the categoriser tries them, and marks the rule that would
/// actually win — the same one `winning_rule` picks.
#[gpui::test]
fn rules_page_test_mode_marks_the_winner_first(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.rules_test_mode = true;
            cx.notify();
        });
    });
    set_rules_search(cx, handle, &shell, "COFFEE SHOP A");
    draw_frame(cx, handle);

    // Both substring rules match the description; the unused ones don't.
    assert!(paints(cx, handle, format!("rules:row-{}", rules[0])));
    assert!(paints(cx, handle, format!("rules:row-{}", rules[1])));
    for id in &rules[2..] {
        assert!(
            !paints(cx, handle, format!("rules:row-{id}")),
            "r{id} does not match the description"
        );
    }
    // "COFFEE SHOP" is the longer substring, so it wins and is the first
    // row, marked "wins"; "COFFEE" follows unmarked.
    assert!(paints(cx, handle, format!("rules:wins-{}", rules[1])));
    assert!(!paints(cx, handle, format!("rules:wins-{}", rules[0])));
    assert!(
        centre_y(cx, handle, format!("rules:row-{}", rules[1])) < {
            draw_frame(cx, handle);
            centre_y(cx, handle, format!("rules:row-{}", rules[0]))
        },
        "the winner is the first row"
    );

    // Leaving test mode restores the grouped list.
    set_rules_search(cx, handle, &shell, "");
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.rules_test_mode = false;
            cx.notify();
        });
    });
    draw_frame(cx, handle);
    assert!(!paints(cx, handle, format!("rules:wins-{}", rules[1])));
    assert!(paints(cx, handle, format!("rules:row-{}", rules[2])));
}

/// 3.28: the Unused checkbox keeps only the rules deciding zero
/// transactions; their Uses column reads 0.
#[gpui::test]
fn rules_page_unused_shows_only_zero_use_rules(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.rules_unused_only = true;
            cx.notify();
        });
    });
    draw_frame(cx, handle);
    // r1 (2 uses) and r2 (1 use) hide; the two rules nothing matches stay.
    for id in &rules[..2] {
        assert!(
            !paints(cx, handle, format!("rules:row-{id}")),
            "a used rule hides behind Unused"
        );
    }
    for id in &rules[2..] {
        assert!(
            paints(cx, handle, format!("rules:row-{id}")),
            "the unused rule {id} stays listed"
        );
    }
}

/// 3.28: the Type picker keeps only rows of the chosen kind.
#[gpui::test]
fn rules_page_type_picker_filters_by_kind(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let pick = |cx: &mut TestAppContext, row: Option<usize>| {
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, cx| {
                s.rules_page.rules_type_select.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        row.map(gpui_kit::component::IndexPath::new),
                        window,
                        cx,
                    );
                });
            });
        });
        cx.run_until_parked();
    };

    pick(cx, Some(2)); // Regex
    draw_frame(cx, handle);
    assert!(paints(cx, handle, format!("rules:row-{}", rules[3])));
    for id in &rules[..3] {
        assert!(
            !paints(cx, handle, format!("rules:row-{id}")),
            "substring rules hide under Regex"
        );
    }

    pick(cx, Some(1)); // Substring
    draw_frame(cx, handle);
    for id in &rules[..3] {
        assert!(paints(cx, handle, format!("rules:row-{id}")));
    }
    assert!(!paints(cx, handle, format!("rules:row-{}", rules[3])));

    pick(cx, None); // All types
    draw_frame(cx, handle);
    for id in rules {
        assert!(paints(cx, handle, format!("rules:row-{id}")));
    }
}

/// 3.28: clicking a group header collapses the group — its rows are gone,
/// the header stays — and clicking it again expands.
#[gpui::test]
fn rules_page_collapse_hides_a_groups_rows(cx: &mut TestAppContext) {
    let (handle, _shell, coffee, _snacks, _games, rules) = open_rules_page(cx);
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    let header = bounds(cx, handle, format!("rules:group:{coffee}"));
    window.simulate_click(header.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("rules:group:{coffee}")),
        "the collapsed group keeps its header"
    );
    for id in [rules[0], rules[1]] {
        assert!(
            !paints(cx, handle, format!("rules:row-{id}")),
            "rule {id} hides with its collapsed group"
        );
    }
    assert!(
        paints(cx, handle, format!("rules:row-{}", rules[2])),
        "other groups stay open"
    );

    let header = bounds(cx, handle, format!("rules:group:{coffee}"));
    window.simulate_click(header.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(paints(cx, handle, format!("rules:row-{}", rules[0])));
    assert!(paints(cx, handle, format!("rules:row-{}", rules[1])));
}

/// 3.28: checking rules opens the Move bar; Move points them at another
/// subcategory in one call, which re-runs the rules over the unlocked rows
/// (the locked row keeps its override) and clears the selection.
#[gpui::test]
fn rules_page_move_recategorizes_rows_and_clears_selection(cx: &mut TestAppContext) {
    let (handle, shell, coffee, snacks, games, rules) = open_rules_page(cx);
    let (r1, r2) = (rules[0], rules[1]);
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.set_rule_selected(r1, true, cx);
            s.set_rule_selected(r2, true, cx);
        });
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "rules:move-bar"),
        "checked rows open the Move bar"
    );

    // Target: Food › Snacks, the same child list the rule pickers use.
    let ix = cx.update(|cx| {
        shell
            .read(cx)
            .child_choices
            .iter()
            .position(|(id, _)| *id == snacks)
            .unwrap()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.move_rules_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.move_selected_rules(window, cx);
        });
    });
    wait_until(cx, "the move lands", |cx| {
        let s = shell.read(cx);
        s.rules.iter().all(|r| r.category_id != coffee) && s.cat_counts.get(&snacks) == Some(&3)
    });
    // Both moved rules now target Snacks; the other two are untouched.
    shell.read_with(cx, |s, _| {
        assert_eq!(
            s.rules.iter().filter(|r| r.category_id == snacks).count(),
            3
        );
    });
    // The unlocked rows followed the rules; the locked row stayed in Games.
    // A category with no rows drops out of `cat_counts` entirely.
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            !s.cat_counts.contains_key(&coffee),
            "Coffee has no rows left after the move"
        );
        assert_eq!(s.cat_counts.get(&snacks), Some(&3));
        assert_eq!(s.cat_counts.get(&games), Some(&1));
        assert!(
            s.rules_page.selected_rules.is_empty(),
            "the selection clears"
        );
    });
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "rules:move-bar"),
        "the Move bar closes with the cleared selection"
    );
}

/// 3.28: thousands of rules — the virtualized list builds only the rows
/// that can be on screen, headers included.
#[gpui::test]
fn rules_page_builds_only_the_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "two thousand rules", |cx| {
        shell.read(cx).rules.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    let built = shell.read_with(cx, |s, _| s.rules_page.rules_page_rows_built.get());
    assert!(
        built > 0 && built < 200,
        "the list built {built} of 2000 rules, only the visible slice"
    );
}

/// 3.28: at the 1200 × 720 minimum window the Rules page fits: header,
/// filter row, group header and a rule row with long names stay inside the
/// page, and the long pattern truncates to its column.
#[gpui::test]
fn the_rules_page_fits_the_minimum_window(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_categories_fit(c));
    let (child, rule) = ids;
    wait_until(cx, "the rule loaded", |cx| shell.read(cx).rules.len() == 1);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);

    let page_right = px(1200. - PAGE_PAD);
    for (what, sel) in [
        ("the filter row", "rules:filter-row".to_string()),
        ("the group header", format!("rules:group:{child}")),
        ("the rule row", format!("rules:row-{rule}")),
        ("the New rule… button", format!("label:{NEW_RULE}")),
    ] {
        let b = bounds(cx, handle, sel);
        assert!(
            b.origin.x + b.size.width <= page_right + px(1.),
            "{what} ends at x {:?}, past the page edge {page_right:?}",
            b.origin.x + b.size.width,
        );
    }
    // The pattern truncates inside its column, like on Categories.
    let col = bounds(cx, handle, format!("rules:pattern-col:{rule}"));
    let full_w = TEXT_SM_CHAR_PX * CAT_FIT_PATTERN.chars().count() as f32;
    assert!(
        col.size.width < px(full_w),
        "the pattern truncates: the column is {:?} wide, the full text \
         needs about {full_w:?}",
        col.size.width,
    );
}

/// 3.28: opening the Rules page focuses the search box (D5), so the user
/// can type straight away.
#[gpui::test]
fn rules_page_opens_with_the_search_focused(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        seed_categories_fit(conn);
    });
    wait_until(cx, "the rule loaded", |cx| shell.read(cx).rules.len() == 1);
    // The focus request lands on the first frame of the Rules page.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    let focused = with_window(cx, handle, |window, cx| {
        shell
            .read(cx)
            .rules_page
            .rules_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "opening the Rules page focuses Search");
    // Leaving and returning focuses it again.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    let focused = with_window(cx, handle, |window, cx| {
        shell
            .read(cx)
            .rules_page
            .rules_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "returning to the Rules page focuses Search again");
}

/// 3.29: *Show rule* from a dashboard row's menu opens the Rules page on
/// the rule that decided that row: the search box takes the rule's pattern,
/// the rule's row is scrolled into view and carries the reveal highlight,
/// and a new search or a page switch drops the highlight. The fixture adds
/// 30 filler rules in their own "Fill ›" groups (all matching "COFFEE", all
/// sorted before "Food › Coffee") so the revealing really has to scroll.
#[gpui::test]
fn show_rule_from_a_row_lands_on_the_deciding_rule(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, [0i64; 4]);
    let (handle, shell) = open_shell(cx, |conn| {
        ids = seed_rules_page(conn);
        let fill =
            financi_db::categories::create_parent(conn, "Fill", CategoryKind::Expense).unwrap();
        for n in 1..=30 {
            let child =
                financi_db::categories::create_child(conn, fill, &format!("A{n:02}")).unwrap();
            financi_db::rules::add_rule(conn, &format!("COFFEE A{n:02}"), false, child).unwrap();
        }
    });
    let (coffee, _snacks, _games, rules) = ids;
    let (r1, r3) = (rules[0], rules[2]);
    wait_until(cx, "the rules and dashboard rows loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 34
            && s.rules_page.rule_uses.get(&r1) == Some(&2)
            && s.cat_counts.get(&coffee) == Some(&3)
            && s.dashboard.table.read(cx).delegate().rows.len() == 5
    });

    // The menu model on the dashboard row: r1 decided it, so *Show rule*
    // is live.
    let row = cx
        .update(|cx| {
            table_rows(&shell, cx)
                .into_iter()
                .find(|r| r.description.contains("BEAN ROAST 1"))
        })
        .expect("the roast row is on the dashboard");
    assert_eq!(
        cx.update(|cx| shell.update(cx, |s, _| row_show_rule(
            &row,
            &s.rules,
            s.card_payment_category
        ))),
        RowShowRule::Found(r1)
    );
    assert_eq!(
        cx.update(|cx| shell.read(cx).page),
        Page::Dashboard,
        "the menu is opened on the dashboard"
    );

    // Clicking *Show rule* navigates, narrows, scrolls and highlights.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_rule(r1, window, cx));
    });
    let active = format!("rules:row-{r1}:active");
    for _ in 0..8 {
        cx.run_until_parked();
        draw_frame(cx, handle);
        if paints(cx, handle, active.clone()) {
            break;
        }
    }
    assert_eq!(
        cx.update(|cx| shell.read(cx).page),
        Page::Rules,
        "*Show rule* lands on the Rules page"
    );
    assert_eq!(
        with_window(cx, handle, |_, cx| {
            shell
                .read(cx)
                .rules_page
                .rules_search
                .read(cx)
                .value()
                .to_string()
        }),
        "COFFEE",
        "the search box takes the rule's pattern, find-in-page style"
    );
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.rules_reveal),
        Some(r1)
    );
    assert!(
        paints(cx, handle, active.clone()),
        "the deciding rule paints highlighted"
    );
    assert!(
        !paints(cx, handle, format!("rules:row-{r1}")),
        "the highlighted row carries only the reveal selector"
    );
    assert!(
        !paints(cx, handle, format!("rules:row-{r3}")),
        "r3 (\"GAME\") does not match the pattern and stays hidden"
    );
    let y = centre_y(cx, handle, &active);
    assert!(
        y > 0. && y < 720.,
        "the revealed row was scrolled into view (y {y})"
    );

    // Typing a fresh search is the user's own search: the highlight is
    // gone. The search box has the focus, so a real Backspace edits it
    // (`set_value` would be programmatic and emit no Change event).
    cx.simulate_keystrokes(handle, "backspace");
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert_eq!(cx.update(|cx| shell.read(cx).rules_page.rules_reveal), None);
    assert!(
        !paints(cx, handle, active.clone()),
        "the highlight did not survive the user's own edit"
    );

    // Revealing again and then leaving the page drops it too.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_rule(r1, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(paints(cx, handle, active.clone()), "revealed again");
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.rules_reveal),
        None,
        "the reveal belongs to the Rules page"
    );
}

// -- page switching (3.11) ---------------------------------------------------
