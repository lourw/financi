//! Tests for `ui/rules.rs` (3.62 split from `ui_tests.rs`).

use gpui_kit::component::input::InputEvent;

use crate::ui::widgets::SortState;

use super::dashboard::dash;
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
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "import", |cx| dash(cx, &shell).read(cx).row_count == 67);

    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let (merchant, rows) = cx.update(|cx| top_group(&shell, cx));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern.update(cx, |i, cx| {
                    i.set_value(merchant.clone(), window, cx);
                })
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            s.rules_page.update(cx, |p, cx| {
                p.rule_target.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        Some(gpui_kit::component::IndexPath::new(ix)),
                        window,
                        cx,
                    );
                })
            });
            s.rules_page.update(cx, |p, cx| p.add_rule(window, cx));
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
            s.rules_page.update(cx, |p, cx| {
                p.rule_edit_pattern
                    .update(cx, |i, cx| i.set_value(value, window, cx))
            });
            s.rules_page.update(cx, |p, cx| p.preview_rule_match(cx));
        });
    });
}

/// Press the editor's Save — the same `save_rule_edit` the button calls.
fn save_rule_edit(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.save_rule_edit(window, cx))
        });
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
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            Some(rule),
            "the editor is open on the rule"
        );
        assert_eq!(
            s.rules_page
                .read(cx)
                .rule_edit_pattern
                .read(cx)
                .value()
                .to_string(),
            "COFFEE"
        );
        assert!(
            !s.rules_page.read(cx).rule_edit_regex,
            "substring kind pre-filled"
        );
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
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_match_count
            .is_some()
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
        assert_eq!(s.rules_page.read(cx).editing_rule, None, "the editor closed on save");
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

/// 3.64: two actions issued in the same frame never cancel each other.
/// The shared task slot used to make the later one kill the earlier's DB
/// write mid-flight. Here a rule edit's Save and a row Hide go out
/// together, and *both* complete: the rule carries its new pattern (the
/// save's recategorize sent the dropped row back to Uncategorized), the
/// locked row left the counts (hidden), the editor closed, and the pending
/// badge cleared — with a refresh per action.
#[gpui::test]
fn a_rule_save_and_a_row_hide_never_cancel_each_other(cx: &mut TestAppContext) {
    // The shared fixture's rows, plus the id of the locked row to hide.
    let mut ids = (0i64, 0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| {
        let (rule, first, second) = seed_rule_fixture(c);
        let locked: i64 = c
            .query_row(
                "SELECT id FROM transactions WHERE description_raw = 'COFFEE SHOP LOCKED'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        ids = (rule, first, second, locked);
    });
    let (rule, _first, second, locked) = ids;
    wait_until(cx, "rule fixture loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 1 && s.cat_counts.get(&second) == Some(&1)
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");

    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "SHOP B");
    let started = cx.update(|cx| shell.read(cx).refresh_started.get());

    // One frame, two actions: Save the edited rule, Hide the locked row.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.save_rule_edit(window, cx));
        });
        dash(cx, &shell).update(cx, |d, cx| d.set_row_hidden(locked, true, window, cx));
    });
    // Both are in flight — the Hide left the Save's badge alone (the badge
    // names rule actions only), and nothing has run yet.
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rules_page.read(cx).pending_rule_action,
            Some(PendingRuleAction::Save(rule)),
            "the save is in flight"
        );
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            Some(rule),
            "the editor waits for its save"
        );
    });

    wait_until(cx, "the save and the hide both landed", |cx| {
        let s = shell.read(cx);
        s.rules_page.read(cx).pending_rule_action.is_none()
            && s.rules_page.read(cx).editing_rule.is_none()
            && s.rules
                .iter()
                .any(|r| r.id == rule && r.pattern == "SHOP B")
            && s.cat_counts.get(&second).copied().unwrap_or(0) == 0
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.rules_page.read(cx).rule_error.is_none(),
            "the save succeeded"
        );
        assert_eq!(
            s.uncategorized_visible, 1,
            "the row the new pattern dropped is back in Uncategorized — the save's recategorize ran"
        );
        assert!(
            s.refresh_started.get() >= started + 2,
            "each action refreshed: {started} → {}",
            s.refresh_started.get()
        );
    });
}

/// 3.64: `rule_error` is one line shared by the add row and the open
/// editor, so a late add result must never repaint it for someone else.
/// Here a failing add (an invalid regex) lands while the editor is open on
/// another rule: the editor keeps its own state, the failure surfaces as a
/// toast instead of under the editor's fields, and the editor's *own*
/// failure still paints where it belongs.
#[gpui::test]
fn a_late_add_failure_never_paints_under_another_rules_editor(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    // Select the rule's own subcategory: the add row targets it directly.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
    });
    let toasts_before = cx.update(|cx| toasts(&shell, cx).len());

    // One frame: the add goes out and the editor opens on the other rule
    // before the write has run, so the add is late by definition.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern.update(cx, |i, cx| {
                    i.set_value("(", window, cx);
                })
            });
            s.rules_page.update(cx, |p, _cx| p.rule_regex = true);
            s.add_rule_to_selected(window, cx);
            s.rules_page
                .update(cx, |p, cx| p.start_edit_rule(rule, window, cx));
        });
    });
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).rules_page.read(cx).pending_rule_action,
            Some(PendingRuleAction::Add),
            "the add is in flight"
        );
    });
    wait_until(cx, "the failed add lands", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pending_rule_action
            .is_none()
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.rules_page.read(cx).rule_error.is_none(),
            "the add's failure never paints under the other rule's editor: {:?}",
            s.rules_page.read(cx).rule_error
        );
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            Some(rule),
            "the editor is left alone"
        );
        let (msg, is_error) = last_toast(&shell, cx).expect("the failure is reported");
        assert!(is_error, "the add failed: {msg}");
        assert!(
            msg.contains("invalid regex"),
            "the toast is the add's: {msg}"
        );
        assert_eq!(toasts(&shell, cx).len(), toasts_before + 1);
    });
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "rules:edit-error"),
        "no error line paints under the editor"
    );

    // The line is not swallowed for its owner: the editor's *own* failure
    // still shows under it (and no second toast appears).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, _cx| p.rule_edit_regex = true);
            s.rules_page.update(cx, |p, cx| {
                p.rule_edit_pattern.update(cx, |i, cx| {
                    i.set_value("[", window, cx);
                })
            });
            s.rules_page
                .update(cx, |p, cx| p.save_rule_edit(window, cx));
        });
    });
    wait_until(cx, "the editor's own save fails", |cx| {
        let s = shell.read(cx);
        s.rules_page.read(cx).pending_rule_action.is_none()
            && s.rules_page.read(cx).rule_error.is_some()
    });
    wait_idle(cx, &shell);
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.rules_page
                .read(cx)
                .rule_error
                .as_deref()
                .is_some_and(|e| e.contains("invalid regex")),
            "the editor reports its own failure: {:?}",
            s.rules_page.read(cx).rule_error
        );
        assert_eq!(
            toasts(&shell, cx).len(),
            toasts_before + 1,
            "the editor's failure paints, it does not toast"
        );
    });
    assert!(
        paints(cx, handle, "rules:edit-error"),
        "the editor's own error line paints"
    );
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
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, _cx| p.rule_edit_regex = true)
        });
    });
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).rules_page.read(cx).rule_error.is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            Some(rule),
            "the editor stays open"
        );
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
            s.rules_page
                .update(cx, |p, cx| p.save_rule_edit(window, cx));
            assert_eq!(
                s.rules_page.read(cx).pending_rule_action,
                Some(PendingRuleAction::Save(rule)),
                "Save shows loading, disabled, while the save runs"
            );
        });
    });
    wait_until(cx, "the save task to finish", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pending_rule_action
            .is_none()
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
        assert_eq!(
            s.rules_page.read(cx).pending_rule_action,
            None,
            "loading cleared"
        );
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            None,
            "the editor closed after the task"
        );
        assert_eq!(s.rules_page.read(cx).rule_error, None, "the save succeeded");
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
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, _cx| p.rule_edit_regex = true)
        });
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.save_rule_edit(window, cx));
            assert_eq!(
                s.rules_page.read(cx).pending_rule_action,
                Some(PendingRuleAction::Save(rule))
            );
        });
    });
    wait_until(cx, "the editor error", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .pending_rule_action
            .is_none()
            && shell.read(cx).rules_page.read(cx).rule_error.is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rules_page.read(cx).pending_rule_action,
            None,
            "loading cleared on the error path too"
        );
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            Some(rule),
            "the editor stays open"
        );
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
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern
                    .update(cx, |i, cx| i.set_value("TOAST".to_string(), window, cx))
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == second)
                .unwrap();
            s.rules_page.update(cx, |p, cx| {
                p.rule_target.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        Some(gpui_kit::component::IndexPath::new(ix)),
                        window,
                        cx,
                    );
                })
            });
            s.rules_page.update(cx, |p, cx| p.add_rule(window, cx));
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
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, _cx| p.rule_edit_regex = true)
        });
    });
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).rules_page.read(cx).rule_error.is_some()
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

/// 3.48: the editor is 3.89's `InlineForm` — a bordered two-line form
/// opened in place of the row, whose Pattern input gets the editor's full
/// width (the user report: the one-row editor shrank it past editing).
/// At the minimum window the input is at least 90 % of the detail panel's
/// inner width, the controls line shares one centre (the switch beside
/// Save and Cancel — pinned by the Categories alignment test), and the
/// row below the editor starts under it: the reserved `RULE_EDITOR_H`
/// keeps the virtual list from painting the next row over the form.
/// Both add-rule rows measure ≥ 240 px wide for their Pattern input at
/// 1200 × 720 after 3.53 removed pickers, so they stay on one line (the
/// two-line shape is only for the editor).
#[gpui::test]
fn the_rule_editor_gives_the_pattern_room(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, second) = open_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    // 3.48: the add-rule rows stay on one line — after 3.53 their
    // Pattern input is wide enough to edit there (spec threshold 240 px).
    let cat_add = bounds(cx, handle, "cat:pattern-input:add");
    assert!(
        f32::from(cat_add.size.width) >= 240.,
        "the Categories add-rule row's Pattern input is {} px wide, keeping one line",
        cat_add.size.width
    );
    // A second rule below the one being edited: its row is what an
    // under-reserved editor would overlap.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern
                    .update(cx, |i, cx| i.set_value("TOAST".to_string(), window, cx))
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == second)
                .unwrap();
            s.rules_page.update(cx, |p, cx| {
                p.rule_target.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        Some(gpui_kit::component::IndexPath::new(ix)),
                        window,
                        cx,
                    );
                })
            });
            s.rules_page.update(cx, |p, cx| p.add_rule(window, cx));
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

    resize_and_draw(cx, handle, 1200., 720.);
    start_rule_edit(cx, handle, &shell, rule);
    // The steady editor: preview count in, error none — the tallest the
    // no-error form grows.
    wait_until(cx, "the editor preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_match_count
            .is_some()
    });
    draw_frame(cx, handle);

    let detail = bounds(cx, handle, "cat:detail-panel");
    // The panel's inner width: `p_4` padding and the 1 px border off the
    // painted bounds.
    let inner = f32::from(detail.size.width) - 32. - 2.;
    let input = bounds(cx, handle, "rules:edit-pattern-input");
    assert!(
        f32::from(input.size.width) >= inner * 0.9,
        "the pattern input is {} px wide, at least 90 % of the detail \
         panel's inner {inner} px",
        input.size.width
    );

    // 3.48: focus lands in the pattern input, caret at the end of the
    // rule's pattern, so the user can edit straight away.
    let (focused, caret) = with_window(cx, handle, |window, cx| {
        let p = shell.read(cx).rules_page.read(cx);
        let state = p.rule_edit_pattern.read(cx);
        (state.focus_handle(cx).is_focused(window), state.cursor())
    });
    assert!(focused, "Edit focuses the pattern input");
    assert_eq!(caret, "COFFEE".len(), "the caret sits at the end");

    // The editor's whole form ends above the next rule row's top.
    let form = bounds(cx, handle, "rules:edit-form");
    let next = bounds(cx, handle, format!("cat:rule-row:{toast}"));
    assert!(
        form.bottom() <= next.top(),
        "the editor (bottom {:?}) must not overlap the next rule row (top {:?})",
        form.bottom(),
        next.top(),
    );

    // The Rules page's add-rule row keeps one line too (same threshold).
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, _cx| p.new_rule_open = true)
        });
    });
    draw_frame(cx, handle);
    let rules_add = bounds(cx, handle, "rules:pattern-input:add");
    assert!(
        f32::from(rules_add.size.width) >= 240.,
        "the Rules page add-rule row's Pattern input is {} px wide, keeping one line",
        rules_add.size.width
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
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_match_count
            .is_some()
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
            s.rules_page.update(cx, |p, cx| p.cancel_edit_rule(cx));
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern
                    .update(cx, |i, cx| i.set_value("COFFEE".to_string(), window, cx))
            });
            s.rules_page.update(cx, |p, cx| p.preview_rule_match(cx));
        });
    });
    wait_until(cx, "the add-rule preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_match_count
            .is_some()
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
            s.rules_page.update(cx, |p, cx| p.cancel_edit_rule(cx));
            let _ = window;
        });
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            None,
            "Cancel closed the editor"
        );
        assert_eq!(s.rules[0].pattern, "COFFEE", "Cancel saved nothing");
    });

    // Escape: the window-level binding cancels the innermost thing first.
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "ZZZ");
    with_window(cx, handle, |window, cx| {
        let input = shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_edit_pattern
            .read(cx)
            .focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "Escape closed the editor", |cx| {
        shell.read(cx).rules_page.read(cx).editing_rule.is_none()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.rules[0].pattern, "COFFEE", "Escape saved nothing");
    });
}

/// 3.48: Enter in the editor's Pattern input saves. Since 3.48 Edit
/// focuses that input, so the keystroke needs no click first — the user
/// opens Edit, retypes, and presses Enter.
#[gpui::test]
fn enter_saves_the_rule_edit(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    draw_frame(cx, handle);
    let focused = with_window(cx, handle, |window, cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_edit_pattern
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "Edit focused the pattern input (3.48)");
    set_edit_pattern(cx, handle, &shell, "BREW");
    cx.simulate_keystrokes(handle, "enter");
    wait_until(cx, "Enter saved the edit", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule && r.pattern == "BREW")
    });
    wait_until(cx, "the editor closed", |cx| {
        shell.read(cx).rules_page.read(cx).editing_rule.is_none()
    });
}

/// 3.80: on the Rules page Escape is the page's own `Cancel` — it unwinds
/// one level per press (the open editor, then the armed delete, then the
/// ticked rows) and does nothing once all three are clear.
#[gpui::test]
fn escape_unwinds_the_rules_page_one_level_at_a_time(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));

    // Stage all three levels: a ticked row, an open editor, an armed delete.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.selected_rules.insert(rule);
                p.start_edit_rule(rule, window, cx);
                p.arm_delete_rule(rule, window, cx);
            });
        });
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let p = shell.read(cx).rules_page.read(cx);
        assert_eq!(p.editing_rule, Some(rule));
        assert_eq!(p.armed_delete_rule, Some(rule));
        assert_eq!(p.selected_rules.len(), 1);
    });

    // Focus the search field: it stays rendered while the levels unwind,
    // and any focus inside the page activates its `Rules` key context.
    with_window(cx, handle, |window, cx| {
        let input = shell
            .read(cx)
            .rules_page
            .read(cx)
            .rules_search
            .read(cx)
            .focus_handle(cx);
        window.focus(&input, cx);
    });
    draw_frame(cx, handle);

    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "the first Escape closed the editor", |cx| {
        let p = shell.read(cx).rules_page.read(cx);
        p.editing_rule.is_none() && p.armed_delete_rule.is_some()
    });
    cx.update(|cx| {
        let p = shell.read(cx).rules_page.read(cx);
        assert_eq!(
            p.selected_rules.len(),
            1,
            "the first Escape closes the editor and nothing else"
        );
    });

    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "the second Escape released the delete", |cx| {
        let p = shell.read(cx).rules_page.read(cx);
        p.armed_delete_rule.is_none() && !p.selected_rules.is_empty()
    });

    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "the third Escape cleared the ticked rows", |cx| {
        shell.read(cx).rules_page.read(cx).selected_rules.is_empty()
    });

    // Nothing left: a fourth press changes nothing.
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        let p = s.rules_page.read(cx);
        assert!(p.editing_rule.is_none() && p.armed_delete_rule.is_none());
        assert!(p.selected_rules.is_empty());
        assert_eq!(
            s.catalog.read(cx).snap.rules.len(),
            1,
            "the whole unwind deletes nothing"
        );
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
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_match_count
            .is_some()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.categories_page.selected_category, Some(first));
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            None,
            "selecting a category cancels the edit"
        );
        assert!(
            s.rules_page.read(cx).rule_match_count.is_none(),
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
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.arm_delete_rule(rule, window, cx))
        });
        assert_eq!(
            shell.read(cx).rules_page.read(cx).armed_delete_rule,
            Some(rule),
            "the first click arms the row"
        );
        assert_eq!(shell.read(cx).rules.len(), 1, "and deletes nothing yet");
    });

    // Second click: deletes.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.arm_delete_rule(rule, window, cx))
        });
    });
    wait_until(cx, "the rule deleted", |cx| shell.read(cx).rules.is_empty());
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rules_page.read(cx).armed_delete_rule,
            None,
            "the armed state reset"
        );
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
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.arm_delete_rule(rule, window, cx))
        });
        assert_eq!(
            shell.read(cx).rules_page.read(cx).armed_delete_rule,
            Some(rule)
        );
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
        assert_eq!(
            shell.read(cx).rules_page.read(cx).armed_delete_rule,
            None,
            "selecting a category disarms the delete"
        );
    });

    // A page switch closes the editor (and its preview count) and disarms
    // in one go.
    start_rule_edit(cx, handle, &shell, rule);
    wait_until(cx, "the editor preview", |cx| {
        shell
            .read(cx)
            .rules_page
            .read(cx)
            .rule_match_count
            .is_some()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.arm_delete_rule(rule, window, cx))
        });
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rules_page.read(cx).editing_rule,
            None,
            "the page switch closed the editor"
        );
        assert!(
            s.rules_page.read(cx).rule_match_count.is_none(),
            "and cleared the shared preview count"
        );
        assert_eq!(
            s.rules_page.read(cx).armed_delete_rule,
            None,
            "and disarmed the delete"
        );
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
        assert_eq!(s.rules_page.read(cx).editing_rule, Some(first_rule));
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
            && s.rules_page.read(cx).rule_uses.get(&rules[0]) == Some(&2)
            && s.rules_page.read(cx).rule_uses.get(&rules[1]) == Some(&1)
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
            s.rules_page.update(cx, |p, cx| {
                p.rules_search.update(cx, |i, cx| {
                    // 3.58: `set_value` alone does not emit (like paste);
                    // the emit stands for the keystroke's Change event the
                    // debounce rides on, as the dashboard's search tests.
                    i.set_value(value.clone(), window, cx);
                    cx.emit(InputEvent::Change);
                })
            });
        });
    });
    // 3.58: filtering is debounced by 150 ms — run the fake clock past
    // the pending timer so the query applies like it would for a user.
    cx.executor().advance_clock(Duration::from_millis(160));
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
        paints(cx, handle, "rules:summary:4 rules · 2 unused"),
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

/// 3.58: each row's Total column is the signed sum of the transactions
/// the rule decided — the same source as Uses, so the fixture's two
/// −10.00 roasts give r1 −2000 cents, the shop row gives r2 −1000, and
/// the unused rules read 0. (The value is baked into the selector, so
/// this pins the rendered number to the SQL sums `rule_use_stats`
/// produces over the same seed.)
#[gpui::test]
fn rules_page_shows_each_rules_total(cx: &mut TestAppContext) {
    let (handle, _shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    draw_frame(cx, handle);
    let [r1, r2, r3, r4] = rules;
    for (id, cents) in [(r1, -2000), (r2, -1000), (r3, 0), (r4, 0)] {
        assert!(
            paints(cx, handle, format!("rules:total:{id}:{cents}")),
            "rule {id}'s Total shows {cents} cents"
        );
        assert!(
            paints(cx, handle, format!("rules:total-col:{id}")),
            "rule {id} has a Total cell"
        );
    }
}

/// 3.58: the group header sums its rules' Uses and Total — Food › Coffee
/// (r1 + r2) reads 3 uses and −3000 cents; the unused groups read 0.
#[gpui::test]
fn rules_page_group_headers_sum_uses_and_total(cx: &mut TestAppContext) {
    let (handle, _shell, coffee, snacks, games, _rules) = open_rules_page(cx);
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("rules:group-uses:{coffee}:3")),
        "the Coffee group sums 3 uses"
    );
    assert!(
        paints(cx, handle, format!("rules:group-total:{coffee}:-3000")),
        "the Coffee group sums its decided cents"
    );
    for id in [snacks, games] {
        assert!(
            paints(cx, handle, format!("rules:group-uses:{id}:0")),
            "the unused group {id} sums 0 uses"
        );
        assert!(
            paints(cx, handle, format!("rules:group-total:{id}:0")),
            "the unused group {id} sums 0 cents"
        );
    }
}

/// The direction the heading arrow paints for `col`
/// (`rules:heading:{col}:arrow:{asc|desc}`), if it paints at all.
fn rules_arrow(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    col: &str,
) -> Option<&'static str> {
    ["asc", "desc"]
        .into_iter()
        .find(|d| paints(cx, handle, format!("rules:heading:{col}:arrow:{d}")))
}

/// 3.58: clicking the Total heading sorts each group by the Total's
/// absolute amount — first click descending (biggest flow first, like
/// the table's Amount), the repeat ascending; the Pattern heading
/// restores the default. The arrow paints only on the sorted column,
/// pointing the sorted way.
#[gpui::test]
fn rules_page_sort_by_total_orders_by_magnitude(cx: &mut TestAppContext) {
    let (handle, _shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let (r1, r2) = (rules[0], rules[1]);
    draw_frame(cx, handle);
    // At rest no sortable heading paints an arrow.
    for col in ["Pattern", "Uses", "Total"] {
        assert_eq!(
            rules_arrow(cx, handle, col),
            None,
            "{col} shows no arrow at rest"
        );
    }
    // Default: Pattern ascending, "COFFEE" above "COFFEE SHOP".
    assert!(
        centre_y(cx, handle, format!("rules:row-{r1}"))
            < centre_y(cx, handle, format!("rules:row-{r2}"))
    );

    click(cx, handle, "rules:heading:Total");
    draw_frame(cx, handle);
    // Descending by magnitude: r1 (−20.00) above r2 (−10.00).
    assert!(
        centre_y(cx, handle, format!("rules:row-{r1}"))
            < centre_y(cx, handle, format!("rules:row-{r2}")),
        "Total's first click is descending, biggest flow first"
    );
    assert_eq!(rules_arrow(cx, handle, "Total"), Some("desc"));
    assert_eq!(rules_arrow(cx, handle, "Pattern"), None);
    assert_eq!(rules_arrow(cx, handle, "Uses"), None);

    click(cx, handle, "rules:heading:Total");
    draw_frame(cx, handle);
    assert!(
        centre_y(cx, handle, format!("rules:row-{r2}"))
            < centre_y(cx, handle, format!("rules:row-{r1}")),
        "the repeat click flips to ascending"
    );
    assert_eq!(rules_arrow(cx, handle, "Total"), Some("asc"));

    // The Pattern heading sorts by pattern again.
    click(cx, handle, "rules:heading:Pattern");
    draw_frame(cx, handle);
    assert!(
        centre_y(cx, handle, format!("rules:row-{r1}"))
            < centre_y(cx, handle, format!("rules:row-{r2}")),
        "Pattern ascending is the default order"
    );
    assert_eq!(rules_arrow(cx, handle, "Pattern"), Some("asc"));
    assert_eq!(rules_arrow(cx, handle, "Total"), None);
}

/// 3.58: only Pattern, Uses and Total sort (ui-conventions *Rules*), so
/// the Type and checkbox headings are plain labels — a click there
/// leaves the sort state alone.
#[gpui::test]
fn rules_page_only_the_sortable_headings_sort(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, _rules) = open_rules_page(cx);
    let sorted =
        |cx: &mut TestAppContext| shell.read_with(cx, |s, app| s.rules_page.read(app).rules_sort);
    draw_frame(cx, handle);
    click(cx, handle, "rules:heading:Total");
    draw_frame(cx, handle);
    assert_eq!(
        sorted(cx),
        Some(("Total", SortState::Descending)),
        "Total's first click sorts descending"
    );

    for col in ["Type", ""] {
        click(cx, handle, format!("rules:heading:{col}"));
        draw_frame(cx, handle);
        assert_eq!(
            sorted(cx),
            Some(("Total", SortState::Descending)),
            "clicking the {col:?} heading must not change the sort"
        );
    }
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

/// 3.58: typing debounces by 150 ms — the list only re-filters after the
/// timer settles — while Enter applies the current text at once.
#[gpui::test]
fn rules_page_search_debounces_and_enter_applies_at_once(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rules_search.update(cx, |i, cx| {
                    i.set_value("shop", window, cx);
                    cx.emit(InputEvent::Change);
                })
            });
        });
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("rules:row-{}", rules[0])),
        "the list is still whole while the debounce runs"
    );
    cx.executor().advance_clock(Duration::from_millis(160));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, format!("rules:row-{}", rules[0])),
        "the filter applies once the debounce settles"
    );

    // Enter means now: clear the box and press Enter — no clock needed.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rules_search.update(cx, |i, cx| {
                    i.set_value("", window, cx);
                    cx.emit(InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    });
                })
            });
        });
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    for id in rules {
        assert!(
            paints(cx, handle, format!("rules:row-{id}")),
            "Enter applied the cleared search at once"
        );
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
            s.rules_page.update(cx, |p, _cx| p.rules_test_mode = true);
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
            s.rules_page.update(cx, |p, _cx| p.rules_test_mode = false);
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
            s.rules_page.update(cx, |p, _cx| p.rules_unused_only = true);
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
                s.rules_page.update(cx, |p, cx| {
                    p.rules_type_select.update(cx, |sel, cx| {
                        sel.set_selected_index(
                            row.map(gpui_kit::component::IndexPath::new),
                            window,
                            cx,
                        );
                    });
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

/// 3.28: clicking a group header's chevron collapses the group — its rows
/// are gone, the header stays — and clicking it again expands. (3.58: the
/// shared list binds the toggle to the chevron alone.)
#[gpui::test]
fn rules_page_collapse_hides_a_groups_rows(cx: &mut TestAppContext) {
    let (handle, _shell, coffee, _snacks, _games, rules) = open_rules_page(cx);
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    let chevron = bounds(cx, handle, format!("rules:group:{coffee}:toggle"));
    window.simulate_click(chevron.center(), Modifiers::default());
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

    let chevron = bounds(cx, handle, format!("rules:group:{coffee}:toggle"));
    window.simulate_click(chevron.center(), Modifiers::default());
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
            s.rules_page
                .update(cx, |p, cx| p.set_rule_selected(r1, true, cx));
            s.rules_page
                .update(cx, |p, cx| p.set_rule_selected(r2, true, cx));
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
            s.rules_page.update(cx, |p, cx| {
                p.move_rules_target.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        Some(gpui_kit::component::IndexPath::new(ix)),
                        window,
                        cx,
                    );
                });
            });
            s.rules_page
                .update(cx, |p, cx| p.move_selected_rules(window, cx));
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
            s.rules_page.read(cx).selected_rules.is_empty(),
            "the selection clears"
        );
    });
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "rules:move-bar"),
        "the Move bar closes with the cleared selection"
    );
}

/// 3.58: the list lives in one bordered panel that fills the rest of the
/// page, with the sortable heading row at its top — like the queue's
/// panel (3.34): the heading cells and the first rows sit inside the
/// panel's bounds.
#[gpui::test]
fn the_rules_list_paints_inside_one_panel(cx: &mut TestAppContext) {
    let (handle, _shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    let panel = bounds(cx, handle, "rules:panel");
    assert!(
        panel.origin.x >= px(0.)
            && panel.origin.y >= px(0.)
            && panel.origin.x + panel.size.width <= px(1200.) + px(1.)
            && panel.origin.y + panel.size.height <= px(720.) + px(1.),
        "the panel fits the window: {panel:?}",
    );
    for col in ["Pattern", "Type", "Uses", "Total"] {
        let head = bounds(cx, handle, format!("rules:heading:{col}"));
        assert!(
            head.origin.y >= panel.origin.y && head.origin.y < panel.origin.y + panel.size.height,
            "the {col} heading sits inside the panel"
        );
    }
    let first = bounds(cx, handle, format!("rules:row-{}", rules[0]));
    let heading = bounds(cx, handle, "rules:heading:Pattern");
    assert!(
        first.origin.y > heading.origin.y + heading.size.height,
        "the rows start under the heading"
    );
    assert!(
        first.origin.y + first.size.height <= panel.origin.y + panel.size.height + px(1.),
        "the first row sits inside the panel"
    );
}

/// 3.58: with rows checked the Move bar replaces the heading row inside
/// the panel — the heading is gone, the bar is there, and the list's
/// first row does not move down by a pixel.
#[gpui::test]
fn the_move_bar_replaces_the_heading_row(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let (r1, _r2) = (rules[0], rules[1]);
    draw_frame(cx, handle);
    let before = bounds(cx, handle, format!("rules:row-{r1}"));
    assert!(paints(cx, handle, "rules:heading:Pattern"));

    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.set_rule_selected(r1, true, cx));
        });
    });
    draw_frame(cx, handle);
    assert!(paints(cx, handle, "rules:move-bar"), "the Move bar shows");
    assert!(
        !paints(cx, handle, "rules:heading:Pattern"),
        "the heading row is replaced, not pushed down"
    );
    let after = bounds(cx, handle, format!("rules:row-{r1}"));
    assert!(
        (f32::from(after.origin.y) - f32::from(before.origin.y)).abs() <= 1.,
        "the list's top does not move: {} -> {}",
        f32::from(before.origin.y),
        f32::from(after.origin.y),
    );
    // Clearing the selection brings the heading back.
    click(cx, handle, "rules:clear-btn");
    draw_frame(cx, handle);
    assert!(!paints(cx, handle, "rules:move-bar"));
    assert!(paints(cx, handle, "rules:heading:Pattern"));
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
    let built = shell.read_with(cx, |s, app| {
        s.rules_page.read(app).rules_page_rows_built.get()
    });
    assert!(
        built > 0 && built < 200,
        "the list built {built} of 2000 rules, only the visible slice"
    );
}

/// 3.85: the grouped item list is cached — an unrelated notify re-renders
/// the page without rebuilding it, and a real input change (the Unused
/// checkbox) does.
#[gpui::test]
fn rules_page_rebuilds_its_items_only_when_inputs_change(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "two thousand rules", |cx| {
        shell.read(cx).rules.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    let built = shell.read_with(cx, |s, app| s.rules_page.read(app).rules_items_built.get());
    assert!(built >= 1, "opening the page built the list");
    // An unrelated notify: the page re-renders, the inputs do not move.
    cx.update(|cx| shell.update(cx, |s, cx| s.rules_page.update(cx, |_, cx| cx.notify())));
    draw_frame(cx, handle);
    assert_eq!(
        shell.read_with(cx, |s, app| s.rules_page.read(app).rules_items_built.get()),
        built,
        "an unrelated notify did not rebuild the item list"
    );
    // A real input: the Unused checkbox.
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, _cx| p.rules_unused_only = true);
            cx.notify();
        });
    });
    draw_frame(cx, handle);
    assert!(
        shell.read_with(cx, |s, app| s.rules_page.read(app).rules_items_built.get()) > built,
        "the Unused toggle rebuilt the item list"
    );
}

/// 3.43: the Rules page's buttons carry their Lucide icon left of the
/// label and no label ends in an ellipsis: the header's New rule (plus)
/// and each rule row's Edit (pencil) and Delete (trash). The buttons'
/// selectors name the icon assets they draw, the old "…" labels draw
/// nothing, and the drawn widths show the icons actually paint.
/// 3.58: the row actions are icon-only `row_action_button`s revealed on
/// row hover, so only New rule needs the label-width check.
#[gpui::test]
fn the_rules_buttons_paint_icons_instead_of_ellipses(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_categories_fit(c));
    let (_child, rule) = ids;
    wait_until(cx, "the rule loaded", |cx| shell.read(cx).rules.len() == 1);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    resize_and_draw(cx, handle, 1200., 720.);
    for old in ["label:New rule…", "label:Delete…"] {
        assert!(!paints(cx, handle, old), "{old} no longer paints");
    }
    let new_btn = bounds(cx, handle, format!("label:{NEW_RULE}"));

    // 3.58: nothing hovers the row, so its actions stay invisible.
    let edit_sel = format!("rules:row-edit:{rule}:icons/pencil.svg");
    let delete_sel = format!("rules:row-delete:{rule}:icons/trash.svg");
    assert!(
        !paints(cx, handle, edit_sel.clone()),
        "Edit reveals on row hover only"
    );
    assert!(
        !paints(cx, handle, delete_sel.clone()),
        "Delete reveals on row hover only"
    );
    let mut window = VisualTestContext::from_window(handle, cx);
    let row = bounds(cx, handle, format!("rules:row-{rule}"));
    window.simulate_mouse_move(row.center(), None, Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    let edit = bounds(cx, handle, edit_sel);
    let delete = bounds(cx, handle, delete_sel);
    // Icon-only: no "Edit"/"Delete" text anywhere, and the buttons stay
    // well under the width a labelled ghost button had.
    for old in ["label:Edit", "label:Delete"] {
        assert!(!paints(cx, handle, old), "{old} no longer paints");
    }
    for (what, btn) in [("Edit", edit), ("Delete", delete)] {
        assert!(
            btn.size.width.as_f32() < 48.,
            "{what} is icon-only, not a labelled button: {btn:?}"
        );
    }

    let mut refs = open_icon_text_refs(cx, vec![("ref:rules-new", NEW_RULE)]);
    assert_button_paints_icon(
        &mut refs,
        "the New rule button",
        new_btn,
        "ref:rules-new",
        MEDIUM_ICON_BLOCK_W,
        MEDIUM_BUTTON_PAD_X,
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
            .read(cx)
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
            .read(cx)
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
            && s.rules_page.read(cx).rule_uses.get(&r1) == Some(&2)
            && s.cat_counts.get(&coffee) == Some(&3)
            && dash(cx, &shell)
                .read(cx)
                .table
                .read(cx)
                .delegate()
                .rows
                .len()
                == 5
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
            &financi_core::rules::CompiledRules::new(&s.rules),
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
        dash(cx, &shell).update(cx, |d, cx| d.show_rule(r1, window, cx));
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
                .read(cx)
                .rules_search
                .read(cx)
                .value()
                .to_string()
        }),
        "COFFEE",
        "the search box takes the rule's pattern, find-in-page style"
    );
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).rules_reveal),
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
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).rules_reveal),
        None
    );
    assert!(
        !paints(cx, handle, active.clone()),
        "the highlight did not survive the user's own edit"
    );

    // Revealing again and then leaving the page drops it too.
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.show_rule(r1, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(paints(cx, handle, active.clone()), "revealed again");
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).rules_reveal),
        None,
        "the reveal belongs to the Rules page"
    );
}

// -- page switching (3.11) ---------------------------------------------------

// -- a rule's transactions under the rule list (3.52) ------------------------

/// 3.52: clicking a rule row focuses it — the row takes the active
/// background and the panel under the list shows the rule's matches: the
/// "Transactions matching" header, the `4 · 2 decided by this rule`
/// summary, and the non-deciding rows tagged "won by …" / "locked".
#[gpui::test]
fn clicking_a_rule_row_opens_its_matches_panel(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let r1 = rules[0];
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "rules:matches-panel"),
        "no focus, no panel"
    );

    click(cx, handle, format!("rules:row-{r1}"));
    wait_until(cx, "r1's matches loaded", |cx| {
        let p = shell.read(cx).rules_page.read(cx);
        !p.rule_matches_loading && !p.rule_matches.is_empty()
    });
    draw_frame(cx, handle);

    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).focused_rule),
        Some(r1)
    );
    assert!(
        paints(cx, handle, format!("rules:row-{r1}:focused")),
        "the focused row is marked"
    );
    assert!(paints(cx, handle, "rules:matches-panel"));
    assert!(paints(cx, handle, "rules:matches-pattern:COFFEE"));
    assert!(
        paints(cx, handle, "rules:matches-summary:4:2"),
        "4 matches, 2 decided by r1"
    );
    assert!(paints(cx, handle, "rules:match-tag:won by COFFEE SHOP"));
    assert!(paints(cx, handle, "rules:match-tag:locked"));
    assert!(
        centre_y(cx, handle, "rules:list") < centre_y(cx, handle, "rules:matches-panel"),
        "the matches panel sits below the rule list"
    );
}

/// 3.52: a rule nothing matches keeps the panel open with the empty
/// message — focus is about the rule, not about there being rows.
#[gpui::test]
fn a_rule_with_no_matches_shows_the_empty_state(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let r3 = rules[2];
    click(cx, handle, format!("rules:row-{r3}"));
    wait_until(cx, "r3's load finished", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    draw_frame(cx, handle);
    assert!(paints(cx, handle, "rules:matches-panel"));
    assert!(paints(cx, handle, "rules:matches-summary:0:0"));
    assert!(paints(cx, handle, "rules:matches-empty"));
}

/// 3.52: a result that arrives after the focus moved on is dropped — the
/// panel never paints the previous rule's rows.
#[gpui::test]
fn a_late_matches_result_is_dropped(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let (r1, r3) = (rules[0], rules[2]);
    // Focus r1 and move the focus to r3 before r1's job can resolve: both
    // keyed jobs are queued in one update, r1's lands after the move.
    with_window(cx, handle, |_window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.toggle_rule_focus(r1, cx);
                p.toggle_rule_focus(r3, cx);
            });
        });
    });
    wait_until(cx, "r3's load finished", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).rule_matches.len()),
        0,
        "r1's rows must not land under r3"
    );
    assert!(paints(cx, handle, "rules:matches-empty"));
    assert!(
        !paints(cx, handle, "rules:match-tag:won by COFFEE SHOP"),
        "r1's rows never painted"
    );
}

/// 3.52: Escape unwinds the editor first and the matches panel second;
/// clicking the focused row again closes the panel like Escape's second
/// press.
#[gpui::test]
fn escape_and_a_second_click_clear_the_focus(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let r1 = rules[0];
    click(cx, handle, format!("rules:row-{r1}"));
    wait_until(cx, "r1's matches loaded", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    draw_frame(cx, handle);
    assert!(paints(cx, handle, "rules:matches-panel"));

    // Editing the rule keeps the focus: the panel stays while the editor
    // is open.
    let mut window = VisualTestContext::from_window(handle, cx);
    let row = bounds(cx, handle, format!("rules:row-{r1}:focused"));
    window.simulate_mouse_move(row.center(), None, Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    click(cx, handle, format!("rules:row-edit:{r1}:icons/pencil.svg"));
    draw_frame(cx, handle);
    let st = cx.update(|cx| {
        let p = shell.read(cx).rules_page.read(cx);
        (p.focused_rule, p.editing_rule)
    });
    assert_eq!(st, (Some(r1), Some(r1)), "the editor keeps the focus");
    assert!(paints(cx, handle, "rules:matches-panel"));

    // Escape #1 closes the editor and keeps the panel; Escape #2 closes
    // the panel.
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).editing_rule),
        None
    );
    assert!(
        paints(cx, handle, "rules:matches-panel"),
        "Escape 1 kept the focus"
    );
    // The editor held the key focus; put it back inside the page (the
    // search box) so Escape #2 reaches the page's `Rules` context.
    with_window(cx, handle, |window, cx| {
        let input = shell
            .read(cx)
            .rules_page
            .read(cx)
            .rules_search
            .read(cx)
            .focus_handle(cx);
        window.focus(&input, cx);
    });
    draw_frame(cx, handle);
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).focused_rule),
        None
    );
    assert!(
        !paints(cx, handle, "rules:matches-panel"),
        "Escape 2 closed it"
    );

    // Re-click focuses again; clicking the focused row closes it.
    click(cx, handle, format!("rules:row-{r1}"));
    wait_until(cx, "r1 loaded again", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    draw_frame(cx, handle);
    assert!(paints(cx, handle, "rules:matches-panel"));
    click(cx, handle, format!("rules:row-{r1}:focused"));
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).focused_rule),
        None,
        "the second click closed the panel"
    );
    assert!(!paints(cx, handle, "rules:matches-panel"));
}

/// 3.52: the row's checkbox and its Edit / Delete buttons take their
/// clicks — they never move or close the focus.
#[gpui::test]
fn checkboxes_and_row_buttons_do_not_move_the_focus(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let (r1, r2) = (rules[0], rules[1]);
    click(cx, handle, format!("rules:row-{r1}"));
    wait_until(cx, "r1's matches loaded", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    draw_frame(cx, handle);

    // Tick r2's checkbox: it selects, but the focus stays on r1.
    click(cx, handle, format!("checkbox:rule-sel-{r2}"));
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).focused_rule),
        Some(r1),
        "the checkbox must not move the focus"
    );
    assert!(paints(cx, handle, "rules:matches-panel"));
    assert!(paints(cx, handle, format!("rules:row-{r1}:focused")));

    // r2's Edit button (revealed by hover) opens r2's editor — again
    // without moving the focus.
    let mut window = VisualTestContext::from_window(handle, cx);
    let row = bounds(cx, handle, format!("rules:row-{r2}"));
    window.simulate_mouse_move(row.center(), None, Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    click(cx, handle, format!("rules:row-edit:{r2}:icons/pencil.svg"));
    draw_frame(cx, handle);
    let st = cx.update(|cx| {
        let p = shell.read(cx).rules_page.read(cx);
        (p.focused_rule, p.editing_rule)
    });
    assert_eq!(st, (Some(r1), Some(r2)));
    assert!(paints(cx, handle, "rules:matches-panel"));
}

/// 3.52: at 1200 × 720 the split keeps both areas usable — the list at
/// least 120 px, the matches panel at least 160 px, the list the larger
/// share.
#[gpui::test]
fn both_areas_keep_their_minimum_heights(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let r1 = rules[0];
    click(cx, handle, format!("rules:row-{r1}"));
    wait_until(cx, "r1's matches loaded", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let list = bounds(cx, handle, "rules:list-area");
    let panel = bounds(cx, handle, "rules:matches-area");
    let (lh, ph) = (f32::from(list.size.height), f32::from(panel.size.height));
    assert!(lh >= 120., "the list area is only {lh} px tall");
    assert!(ph >= 160., "the matches area is only {ph} px tall");
    assert!(lh > ph, "the list keeps the larger share ({lh} vs {ph})");
}

/// 3.52: selecting another category (Categories page) or leaving the page
/// clears the focus; coming back shows no panel.
#[gpui::test]
fn leaving_the_page_or_picking_a_category_clears_the_focus(cx: &mut TestAppContext) {
    let (handle, shell, coffee, _snacks, _games, rules) = open_rules_page(cx);
    let r1 = rules[0];
    click(cx, handle, format!("rules:row-{r1}"));
    wait_until(cx, "r1's matches loaded", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).focused_rule),
        Some(r1)
    );

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(coffee, window, cx))
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).focused_rule),
        None,
        "a category selection closes the matches panel"
    );

    // Re-focus, then leave the page: the focus is transient like the rest.
    click(cx, handle, format!("rules:row-{r1}"));
    wait_until(cx, "r1 loaded again", |cx| {
        !shell.read(cx).rules_page.read(cx).rule_matches_loading
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_page.read(cx).focused_rule),
        None
    );
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "rules:matches-panel"),
        "no panel on return"
    );
}
