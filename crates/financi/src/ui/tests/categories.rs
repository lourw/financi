//! Tests for `ui/categories.rs` (3.62 split from `ui_tests.rs`).

use super::*;

use super::dashboard::{click_chart_legend, dash, seed_chart_rows, top_category_id};

#[gpui::test]
fn creates_parent_and_child(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);

    add_parent(cx, handle, &shell, "Food & Dining");
    cx.update(|cx| {
        let parent = shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food & Dining")
            .unwrap();
        assert_eq!(
            parent.kind,
            CategoryKind::Expense,
            "kind defaults to Expense"
        );
        assert!(parent.children.is_empty());
    });

    add_child(cx, handle, &shell, "Restaurants");
    cx.update(|cx| {
        let parent = shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food & Dining")
            .unwrap();
        assert_eq!(parent.children.len(), 1);
        assert_eq!(parent.children[0].name, "Restaurants");
        assert_eq!(parent.children[0].kind, CategoryKind::Expense);
    });
}

/// 3.64: an add that lands late claims the selection only while nothing
/// newer has claimed it — and claims it through `select_category`, never by
/// writing the field. The bare write used to jump the selection to the new
/// row while the old row's transient UI stayed open on it: a rename editor
/// left open then renamed the *new* category on its next Save.
#[gpui::test]
fn a_late_category_add_never_steals_a_newer_selection(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| {
        ids = (
            financi_db::categories::create_parent(c, "Drinks", CategoryKind::Expense).unwrap(),
            financi_db::categories::create_parent(c, "Groceries", CategoryKind::Expense).unwrap(),
        );
    });
    let (_drinks, groceries) = ids;
    wait_until(cx, "the categories load", |cx| {
        shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");

    // One frame: the add goes out, then the user selects another row while
    // the write is still running.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .read(cx)
                .new_parent_name
                .clone()
                .update(cx, |i, cx| {
                    i.set_value("Late", window, cx);
                });
            s.add_parent(window, cx);
            s.select_category(groceries, window, cx);
        });
    });
    wait_until(cx, "the late add lands", |cx| {
        shell.read(cx).categories.iter().any(|p| p.name == "Late")
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_view.read(cx).selected_category,
            Some(groceries),
            "the click owns the selection: the late add must not take it back"
        );
    });

    // With the selection left alone the new row still takes it, and the
    // rename editor open on the old row closes with the move.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .read(cx)
                .new_parent_name
                .clone()
                .update(cx, |i, cx| {
                    i.set_value("Later", window, cx);
                });
            s.add_parent(window, cx);
            s.categories_view.update(cx, |p, _| p.renaming = true);
            s.categories_view
                .read(cx)
                .rename_input
                .clone()
                .update(cx, |i, cx| {
                    i.set_value("Groceries", window, cx);
                });
        });
    });
    wait_until(cx, "the second add lands", |cx| {
        shell.read(cx).categories.iter().any(|p| p.name == "Later")
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let s = shell.read(cx);
        let later = s.categories.iter().find(|p| p.name == "Later").unwrap().id;
        assert_eq!(
            s.categories_view.read(cx).selected_category,
            Some(later),
            "an untouched selection still hands over to the new row"
        );
        assert!(
            !s.categories_view.read(cx).renaming,
            "the rename editor closed with the selection it belonged to, so \
             its Save can no longer rename the new category"
        );
    });
}

#[gpui::test]
fn delete_category_reassigns_transactions_and_rules(cx: &mut TestAppContext) {
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
    add_parent(cx, handle, &shell, "Everything Else"); // second parent to reassign onto
    add_child(cx, handle, &shell, "Keep");
    let keep = cx.update(|cx| find_child(&shell, cx, "Keep").unwrap());
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let (merchant, _) = cx.update(|cx| top_group(&shell, cx));

    // Categorize the top merchant's rows onto Online, so a delete has work to move.
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
                .position(|(id, _)| *id == online)
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
    wait_until(cx, "rows on Online", |cx| {
        shell.read(cx).cat_counts.get(&online).copied().unwrap_or(0) > 0
    });

    // Select Online, arm the delete, then confirm reassigning to Keep.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(online, window, cx);
        });
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
        assert_eq!(
            shell
                .read(cx)
                .categories_view
                .read(cx)
                .confirm_delete_category,
            Some(online),
            "first click arms"
        );
        let keep_ix = s_ix(&shell, cx, keep);
        s_select(&shell, cx, keep_ix, window);
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
    });
    wait_until(cx, "Online deleted", |cx| {
        find_child(&shell, cx, "Online").is_none()
    });
    let keep_count = cx.update(|cx| *shell.read(cx).cat_counts.get(&keep).unwrap());
    assert!(keep_count > 0, "transactions moved to Keep");
    cx.update(|cx| {
        assert!(
            shell.read(cx).rules.iter().all(|r| r.category_id == keep),
            "the rule followed the reassignment"
        );
    });
}

/// The index of `child` in the shell's child_choices.
fn s_ix(shell: &Entity<Shell>, cx: &App, child: i64) -> usize {
    shell
        .read(cx)
        .child_choices
        .iter()
        .position(|(id, _)| *id == child)
        .unwrap()
}

fn s_select(shell: &Entity<Shell>, cx: &mut App, ix: usize, window: &mut Window) {
    let target = shell
        .read(cx)
        .categories_view
        .read(cx)
        .delete_target
        .clone();
    target.update(cx, |sel, cx| {
        sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx)
    });
}

/// 3.12: with thousands of categories and rules the Categories page must
/// still render, and the virtualized lists must build only the visible
/// rows, not all 4,000 / 2,000.
#[gpui::test]
fn the_categories_page_builds_only_the_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    // +1: the system `Transfers` parent from migration 0002.
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: the rule list paints only for the selected category, and all
    // 2,000 seeded rules sit under "Cat 0000".
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.rules.len(), 2_000, "the rule list is full too");
        assert!(
            s.categories_page.tree_rows_built.get() > 0
                && s.categories_page.tree_rows_built.get() < 100,
            "the tree built {} rows of 4000 — it must render a window",
            s.categories_page.tree_rows_built.get()
        );
        assert!(
            s.categories_page.rule_rows_built.get() > 0
                && s.categories_page.rule_rows_built.get() < 100,
            "the rule list built {} rows of 2000 — it must render a window",
            s.categories_page.rule_rows_built.get()
        );
    });
}

/// 3.12: selection and the row click handler keep working with a long
/// tree: clicking (the same `select_category` call `tree_row` makes) a
/// category at the very bottom selects it, and the page re-renders.
#[gpui::test]
fn clicking_a_category_still_works_with_a_long_tree(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    draw_frame(cx, handle);
    let deep = cx.update(|cx| {
        let s = shell.read(cx);
        s.categories[1_999].children[0].id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(deep, window, cx));
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_view.read(cx).selected_category,
            Some(deep),
            "the last child of the last parent selected"
        );
        assert!(
            s.categories_page.tree_rows_built.get() < 100,
            "still windowed after selecting"
        );
    });
}

/// 3.21: wheel-scrolling the category tree must stick. `v_virtual_list`
/// created a fresh `VirtualListScrollHandle` per render and `track_scroll`ed
/// it, and GPUI prefers a tracked handle over element state — so the offset
/// snapped back to 0 every frame and rows past the first screen were
/// unreachable. With the handle a `Shell` field, the built window of rows
/// must follow the wheel to the bottom and survive an unrelated refresh.
#[gpui::test]
fn the_category_tree_keeps_its_scroll_offset(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_view.read(cx).tree_scroll.offset(),
            Point::default(),
            "starts unscrolled"
        );
        assert_eq!(
            s.categories_page.tree_range.get().0,
            0,
            "the first frame shows the top"
        );
    });
    // Wheel down to the bottom (content is ~36 px a row for thousands of
    // rows); the offset clamps at the last row.
    for _ in 0..120 {
        let centre = cx.update(|cx| {
            viewport_centre(shell.read(cx).categories_view.read(cx).tree_scroll.bounds())
        });
        wheel(cx, handle, centre, 0., -2_000.);
        draw_frame(cx, handle);
    }
    cx.update(|cx| {
        let s = shell.read(cx);
        let rows: usize = s.categories.iter().map(|p| 1 + p.children.len()).sum();
        assert!(
            s.categories_view.read(cx).tree_scroll.offset().y < px(0.),
            "the tree is scrolled down"
        );
        let (start, end) = s.categories_page.tree_range.get();
        assert!(
            start > 0,
            "the built window moved off the top (start {start})"
        );
        assert_eq!(end, rows, "the built window reached the last row");
        assert!(
            s.categories_page.tree_rows_built.get() < 100,
            "still windowed while scrolled to the bottom"
        );
    });
    // An unrelated refresh must not scroll the tree back to the top.
    let scrolled = cx.update(|cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .tree_scroll
            .offset()
            .y
    });
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_view.read(cx).tree_scroll.offset().y,
            scrolled,
            "the offset survived an unrelated notify"
        );
        assert!(
            s.categories_page.tree_range.get().0 > 0,
            "still showing rows past the top"
        );
    });
}

/// 3.21: the Categories-page rule list keeps its scroll offset across
/// frames and across a plain refresh — only a content replacement (3.23:
/// the selected category changes) may reset it.
#[gpui::test]
fn the_rule_list_keeps_its_scroll_offset(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2000 rules", |cx| {
        shell.read(cx).rules.len() == 2_000 && shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: the list shows the selected category's rules; all 2,000 of
    // them sit under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_view.read(cx).rule_scroll.offset(),
            Point::default(),
            "starts unscrolled"
        );
        assert_eq!(
            s.categories_page.rule_range.get().0,
            0,
            "the first frame shows the top"
        );
        // Jump straight to the bottom; layout clamps to the scroll max.
        s.categories_view
            .read(cx)
            .rule_scroll
            .set_offset(point(px(0.), px(-1_000_000.)));
    });
    draw_frame(cx, handle);
    let scrolled = cx.update(|cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .rule_scroll
            .offset()
            .y
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(scrolled < px(0.), "the rule list is scrolled down");
        let (start, end) = s.categories_page.rule_range.get();
        assert!(
            start > 0,
            "the built window moved off the top (start {start})"
        );
        assert_eq!(end, s.rules.len(), "the built window reached the last rule");
    });
    // A refresh that doesn't replace the rules must not move the offset.
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_view.read(cx).rule_scroll.offset().y,
            scrolled,
            "the offset survived an unrelated notify"
        );
        assert_eq!(s.categories_page.rule_range.get().1, s.rules.len());
    });
    // 3.23: changing the selection replaces the list's content, and the
    // offset resets to the top — the one reset 3.21 allowed.
    select_parent(cx, handle, &shell, "Cat 0001");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_view.read(cx).rule_scroll.offset(),
            Point::default(),
            "the selection change reset the rule list's scroll"
        );
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("cat:rules-empty").is_some(),
        "a category without rules shows the empty note"
    );
}

// -- Rules page (3.28) -------------------------------------------------------

/// 3.18 regression: opening Categories while the first refresh is still in
/// flight used to leave the page blank — the list columns collapsed to 0px
/// inside the page's `h_flex` row (which centers its children), so the
/// virtual lists resolved an empty window and no row appeared when the data
/// finally arrived. Opening the page first, with the data arriving after,
/// must still fill both lists without any further action.
#[gpui::test]
fn categories_page_shows_rows_when_data_arrives_after_it_opens(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // One draw while the refresh is still in flight: the page is open empty.
    draw_frame(cx, handle);
    wait_until(cx, "categories and rules in the shell", |cx| {
        let s = shell.read(cx);
        s.categories.len() == 2_001 && s.rules.len() == 2_000
    });
    // 3.23: the rule list paints with the selection; all 2,000 rules sit
    // under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    // The first draw that has the data must show rows — no extra action.
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.categories_page.tree_rows_built.get() >= 8,
            "tree rows on first open, before any other action: {}",
            s.categories_page.tree_rows_built.get()
        );
        assert!(
            s.categories_page.rule_rows_built.get() > 0,
            "rule rows on first open, before any other action: {}",
            s.categories_page.rule_rows_built.get()
        );
    });
}

/// 3.18 regression: switching to Categories from another page after the
/// data has already loaded must fill both lists on the first draw there.
#[gpui::test]
fn switching_to_categories_after_load_shows_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    wait_until(cx, "2000 rules", |cx| shell.read(cx).rules.len() == 2_000);
    // 3.23: the rule list paints with the selection; all 2,000 rules sit
    // under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.categories_page.tree_rows_built.get() >= 8,
            "tree rows after switching to the page: {}",
            s.categories_page.tree_rows_built.get()
        );
        assert!(
            s.categories_page.rule_rows_built.get() > 0,
            "rule rows after switching to the page: {}",
            s.categories_page.rule_rows_built.get()
        );
    });
}

/// 3.16: the Categories page labels every control — the tree heading, the
/// new-parent fields, the detail pickers (parent and child variants) and
/// the add-rule row. 3.50: the detail pane dropped the Organize heading
/// and its two field labels — they are asserted *absent* below; the
/// controls they used to caption carry their accessibility names in code
/// (`aria_label`/`accessibility_label`), which the bounds dump can't see.
/// 3.51: the pickers and buttons live in the inline forms behind the
/// title-row buttons — opened, switched and cancelled here, and asserted
/// absent while no form is open.
#[gpui::test]
fn the_categories_page_labels_its_controls(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    add_parent(cx, handle, &shell, "Food & Dining");
    let parent_id = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food & Dining")
            .unwrap()
            .id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(parent_id, window, cx);
            // 3.23: the new-category form only paints once opened.
            s.categories_view
                .update(cx, |p, _| p.new_category_open = true);
        });
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    // 3.53: a parent without subcategories can't hold a rule, so the hint
    // stands in for the add-rule row — the scan below needs a child first.
    assert!(
        window.debug_bounds("cat:add-rule-need-child").is_some(),
        "a childless parent offers the add-a-subcategory hint"
    );
    add_child(cx, handle, &shell, "Restaurants");
    let child_id = cx.update(|cx| find_child(&shell, cx, "Restaurants").unwrap());
    // Adding a child selects it, so the scan goes back to the parent.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(parent_id, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    for sel in [
        "label:Category tree",
        "label:New category name",
        "label:Kind",
        "label:Rules",
        "label:Pattern",
        "label:Subcategory",
    ] {
        assert!(
            paints(cx, handle, sel),
            "the Categories page paints the visible label {sel:?}"
        );
    }
    // 3.50: the detail pane's captions are gone — the Organize heading
    // and both field labels. `debug_bounds` matches exactly, so the
    // placeholder selectors (`label:Move to category…`) don't shadow them.
    for sel in [
        "label:Organize",
        "label:New subcategory name",
        "label:Move to category",
    ] {
        assert!(
            !paints(cx, handle, sel),
            "the Categories page no longer paints the caption {sel:?}"
        );
    }
    // 3.51: with no form open none of the form controls paint — the
    // Organize and Delete sections are gone, their controls appear only
    // inside the form their title-row button opened.
    for sel in [
        "label:Add subcategory",
        "label:Move its transactions to…",
        "cat:form-add-sub",
        "cat:form-move",
        "cat:form-delete",
    ] {
        assert!(
            !paints(cx, handle, sel),
            "no detail form is open, so {sel:?} does not paint"
        );
    }
    // The title row offers the actions: a parent gets Add subcategory
    // and Delete; Move belongs to a subcategory, not to it.
    assert!(
        paints(cx, handle, "cat:title-add-sub") && paints(cx, handle, "cat:title-delete"),
        "the parent's title row paints its action buttons"
    );
    assert!(
        !paints(cx, handle, "cat:title-move"),
        "a parent has no Move button"
    );
    // Each button opens its form; opening another closes the previous one.
    click(cx, handle, "cat:title-add-sub");
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:form-add-sub") && paints(cx, handle, "label:Add subcategory"),
        "Add subcategory opens its form, whose button names itself"
    );
    assert!(
        !paints(cx, handle, "cat:form-delete")
            && !paints(cx, handle, "label:Move its transactions to…"),
        "the Delete form stays closed while another is open"
    );
    click(cx, handle, "cat:title-delete");
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:form-delete")
            && paints(cx, handle, "label:Move its transactions to…"),
        "Delete opens its form with the reassign picker"
    );
    assert!(
        !paints(cx, handle, "cat:form-add-sub") && !paints(cx, handle, "label:Add subcategory"),
        "opening Delete closed the Add subcategory form"
    );
    // Cancel closes the form.
    click(cx, handle, "cat:form-delete-cancel");
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "cat:form-delete"),
        "Cancel closes the Delete form"
    );
    // A selected child swaps the buttons: Move in, Add subcategory out,
    // and its form carries the move picker (3.50: named by its
    // placeholder, no field label).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(child_id, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:title-move") && !paints(cx, handle, "cat:title-add-sub"),
        "a subcategory gets the Move button and no Add subcategory one"
    );
    click(cx, handle, "cat:title-move");
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:form-move") && paints(cx, handle, "label:Move to category…"),
        "Move opens its form with the picker's placeholder"
    );
    click(cx, handle, "cat:form-move-cancel");
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "cat:form-move"),
        "Cancel closes the Move form"
    );
    // 3.53: under a subcategory the add-rule row drops its picker — the
    // selection itself names where the rule goes.
    assert!(
        paints(cx, handle, "label:Pattern"),
        "the add-rule row keeps its pattern label under a subcategory"
    );
    assert!(
        !paints(cx, handle, "label:Subcategory"),
        "and no subcategory picker"
    );
}

/// 3.33: nothing the Categories page shows says parent, child or children.
/// With a long-named tree and a subcategory selected, the page paints the
/// new labels and none of the old ones (debug selectors), the placeholders
/// it renders are clean, and every validation error — the typed ones and
/// the ones the database refuses with — reaches the page in the new words.
/// The renamed buttons still add a category and a subcategory.
#[gpui::test]
fn the_categories_page_never_says_parent_or_child(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let food = financi_db::categories::create_parent(
            conn,
            "Food, Groceries & Dining Out",
            CategoryKind::Expense,
        )
        .unwrap();
        financi_db::categories::create_child(conn, food, "Restaurants, Cafés & Takeaway").unwrap();
        financi_db::categories::create_parent(
            conn,
            "Home & Garden Supplies",
            CategoryKind::Expense,
        )
        .unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.run_until_parked();

    fn retired_word(s: &str) -> String {
        const FORBIDDEN: [&str; 3] = ["parent", "child", "children"];
        let low = s.to_lowercase();
        FORBIDDEN
            .into_iter()
            .find(|w| low.contains(*w))
            .unwrap_or("")
            .to_string()
    }

    // Trigger one validation error the way its button does, and check the
    // message the page then renders: exact new copy, no retired word.
    fn trigger_error(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        shell: &Entity<Shell>,
        what: &str,
        trigger: impl Fn(&mut Window, &mut App),
        want: &str,
    ) {
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, cx| {
                s.categories_view.update(cx, |p, _| p.cat_error = None)
            });
            trigger(window, cx);
        });
        wait_until(cx, &format!("the {what:?} error"), |cx| {
            shell.read(cx).categories_view.read(cx).cat_error.is_some()
        });
        let e = cx
            .update(|cx| shell.read(cx).categories_view.read(cx).cat_error.clone())
            .unwrap();
        assert_eq!(e, want, "the {what:?} error reads");
        assert!(
            retired_word(&e).is_empty(),
            "{e:?} still says something retired"
        );
        draw_frame(cx, handle);
    }

    let food_id = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food, Groceries & Dining Out")
            .unwrap()
            .id
    });
    let child = cx.update(|cx| find_child(&shell, cx, "Restaurants, Cafés & Takeaway").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(food_id, window, cx);
            // 3.23: the new-category form only paints once its button
            // opened it; open it so the scan covers its labels too.
            s.categories_view
                .update(cx, |p, _| p.new_category_open = true);
        });
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // The painted labels carry the new words… buttons and Select
    // placeholders included: their text is reachable only through the
    // `label:<text>` debug selectors (3.33).
    let n_cats = cx.update(|cx| shell.read(cx).categories.len());
    let n_subs = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .map(|p| p.children.len())
            .sum::<usize>()
    });
    for sel in [
        "label:Category tree",
        "label:New category name",
        "label:Kind",
        "label:Add category",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the Categories page paints {sel:?}"
        );
    }
    // 3.51: the subcategory and delete controls hide behind the
    // title-row forms; with none open their labels are absent, and
    // opening each form shows them in the new words.
    for sel in ["label:Add subcategory", "label:Move its transactions to…"] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "no form is open, so the Categories page does not paint {sel:?}"
        );
    }
    for (button, painted_sel) in [
        ("cat:title-add-sub", "label:Add subcategory"),
        ("cat:title-delete", "label:Move its transactions to…"),
    ] {
        click(cx, handle, button);
        draw_frame(cx, handle);
        assert!(
            paints(cx, handle, painted_sel),
            "the form opened by {button} paints {painted_sel:?}"
        );
        for sel in [
            "label:New parent name",
            "label:New child name",
            "label:Add parent",
            "label:Add child",
        ] {
            assert!(
                !paints(cx, handle, sel),
                "the form opened by {button} never paints {sel:?}"
            );
        }
    }
    click(cx, handle, "cat:form-delete-cancel");
    draw_frame(cx, handle);
    let count_sel = format!("label:{n_cats} categories · {n_subs} subcategories");
    assert!(
        paints(cx, handle, count_sel.clone()),
        "the header paints the count {count_sel:?}"
    );
    // …and none of the retired ones.
    for sel in [
        "label:New parent name".to_string(),
        "label:New child name".to_string(),
        "label:Add parent".to_string(),
        "label:Add child".to_string(),
        format!("label:{n_cats} parents"),
    ] {
        assert!(
            !paints(cx, handle, sel.clone()),
            "the Categories page never paints {sel:?}"
        );
    }

    // A selected subcategory adds the move picker, again in the new words.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(child, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    // 3.50: the move picker has no field label any more; its placeholder
    // is the visible name, in the new words. 3.51: it paints with the
    // Move form its title-row button opened.
    click(cx, handle, "cat:title-move");
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("label:Move to category…").is_some(),
        "the move picker paints its placeholder in the new words"
    );
    for sel in ["label:Move to parent", "label:Move to parent…"] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "the Categories page never paints {sel:?}"
        );
    }

    // The placeholders are the exact strings the inputs render.
    cx.update(|cx| {
        let s = shell.read(cx);
        for (what, input) in [
            ("add-category", &s.categories_view.read(cx).new_parent_name),
            (
                "add-subcategory",
                &s.categories_view.read(cx).new_child_name,
            ),
            ("rename", &s.categories_view.read(cx).rename_input),
        ] {
            let p = input.read(cx).presentation().placeholder().to_string();
            assert!(
                retired_word(&p).is_empty(),
                "the {what} input's placeholder {p:?} still says {:?}",
                retired_word(&p)
            );
        }
    });

    trigger_error(
        cx,
        handle,
        &shell,
        "add category",
        |window, cx| shell.update(cx, |s, cx| s.add_parent(window, cx)),
        "Give the category a name.",
    );
    trigger_error(
        cx,
        handle,
        &shell,
        "add subcategory",
        |window, cx| {
            shell.update(cx, |s, cx| {
                s.select_category(food_id, window, cx);
                s.add_child(window, cx);
            });
        },
        "Give the subcategory a name.",
    );
    trigger_error(
        cx,
        handle,
        &shell,
        "move",
        |window, cx| {
            shell.update(cx, |s, cx| {
                s.select_category(child, window, cx);
                s.move_child_to_selected_parent(window, cx);
            });
        },
        "Choose the category to move it under.",
    );
    trigger_error(
        cx,
        handle,
        &shell,
        "delete",
        |window, cx| {
            shell.update(cx, |s, cx| {
                s.select_category(child, window, cx);
                s.arm_delete_category(window, cx); // first click arms
                s.arm_delete_category(window, cx);
            });
        },
        "Choose where its transactions should go.",
    );

    // A duplicate subcategory name: the database's own refusal.
    let duplicate = {
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, cx| {
                s.categories_view.update(cx, |p, _| p.cat_error = None);
                s.select_category(food_id, window, cx);
                s.categories_view
                    .read(cx)
                    .new_child_name
                    .clone()
                    .update(cx, |i, cx| {
                        i.set_value("Restaurants, Cafés & Takeaway", window, cx);
                    });
                s.add_child(window, cx);
            });
        });
        wait_until(cx, "the duplicate refusal", |cx| {
            shell.read(cx).categories_view.read(cx).cat_error.is_some()
        });
        let e = cx
            .update(|cx| shell.read(cx).categories_view.read(cx).cat_error.clone())
            .unwrap();
        assert_eq!(
            e,
            "a category named \"Restaurants, Cafés & Takeaway\" already exists under the same category"
        );
        e
    };
    assert!(retired_word(&duplicate).is_empty());

    // Deleting the category that still has subcategories: refused, in the
    // new words.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, _| p.cat_error = None);
            s.select_category(food_id, window, cx);
        });
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
        let ix = s_ix(&shell, cx, child);
        s_select(&shell, cx, ix, window);
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
    });
    wait_until(cx, "the has-subcategories refusal", |cx| {
        shell.read(cx).categories_view.read(cx).cat_error.is_some()
    });
    let has_sub = cx
        .update(|cx| shell.read(cx).categories_view.read(cx).cat_error.clone())
        .unwrap();
    assert_eq!(
        has_sub,
        "Food, Groceries & Dining Out still has subcategories; delete or move them first"
    );
    assert!(retired_word(&has_sub).is_empty());

    // The renamed buttons still work: a new category and, under it, a new
    // subcategory.
    add_parent(cx, handle, &shell, "Travel & Commuting");
    add_child(cx, handle, &shell, "Airfare");
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .categories
                .iter()
                .any(|p| p.name == "Travel & Commuting"
                    && p.children.iter().any(|c| c.name == "Airfare")),
            "the renamed buttons added the category and its subcategory"
        );
    });
    draw_frame(cx, handle);
}

/// 3.51: the inline forms live under the button that opened them, not
/// under the selection: selecting another category closes the open form.
#[gpui::test]
fn selecting_another_category_closes_an_open_detail_form(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let food =
            financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, food, "Restaurants").unwrap();
        financi_db::categories::create_parent(conn, "Home", CategoryKind::Expense).unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Food");
    click(cx, handle, "cat:title-add-sub");
    draw_frame(cx, handle);
    assert!(paints(cx, handle, "cat:form-add-sub"), "the form is open");
    select_parent(cx, handle, &shell, "Home");
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "cat:form-add-sub"),
        "selecting another category closes the open form"
    );
    // Escape closes it too (3.81's one-layer-per-press chain grew the
    // detail form after the rename editor).
    select_parent(cx, handle, &shell, "Food");
    click(cx, handle, "cat:title-add-sub");
    draw_frame(cx, handle);
    let root = cx.update(|cx| shell.read(cx).categories_view.read(cx).focus.clone());
    with_window(cx, handle, |window, cx| window.focus(&root, cx));
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, "cat:form-add-sub"),
        "Escape closes the open detail form"
    );
}

/// 3.51: the Add subcategory form keeps its promise (2026-11-15): a
/// successful add clears the field, keeps the form open and focused on
/// the parent it was opened for, so several subcategories go in a row —
/// and the selection never jumps to the new row.
#[gpui::test]
fn the_add_subcategory_form_stays_open_after_a_successful_add(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    add_parent(cx, handle, &shell, "Food");
    let food = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food")
            .unwrap()
            .id
    });
    select_parent(cx, handle, &shell, "Food");
    click(cx, handle, "cat:title-add-sub");
    draw_frame(cx, handle);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .read(cx)
                .new_child_name
                .clone()
                .update(cx, |i, cx| i.set_value("Groceries", window, cx));
        });
    });
    click(cx, handle, "label:Add subcategory");
    wait_until(cx, "the first subcategory lands", |cx| {
        find_child(&shell, cx, "Groceries").is_some()
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:form-add-sub"),
        "a successful add keeps the form open"
    );
    cx.update(|cx| {
        let s = shell.read(cx);
        let page = s.categories_view.read(cx);
        assert_eq!(
            page.selected_category,
            Some(food),
            "the parent stays selected"
        );
        assert!(
            page.new_child_name.read(cx).value().is_empty(),
            "the field clears for the next name"
        );
    });
    // A second name goes straight in — no reopen, and no leftover text
    // glued in front of it (which is why the field must have cleared).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .read(cx)
                .new_child_name
                .clone()
                .update(cx, |i, cx| i.set_value("Meals", window, cx));
        });
    });
    click(cx, handle, "label:Add subcategory");
    wait_until(cx, "the second subcategory lands", |cx| {
        find_child(&shell, cx, "Meals").is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        let food = s.categories.iter().find(|p| p.name == "Food").unwrap();
        let names: Vec<&str> = food.children.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"Groceries") && names.contains(&"Meals"),
            "both subcategories landed, none joined: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("GroceriesMeals")),
            "the cleared field never concatenates"
        );
    });
}

/// 3.51: Move and Delete still do their work through the forms — and a
/// successful one closes its form. The Delete form carries the two-click
/// question; Cancel closes the form and the question together.
#[gpui::test]
fn the_move_and_delete_forms_run_their_actions_and_close(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let food =
            financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, food, "Restaurants").unwrap();
        financi_db::categories::create_child(conn, food, "Keep").unwrap();
        financi_db::categories::create_parent(conn, "Home", CategoryKind::Expense).unwrap();
        financi_db::categories::create_parent(conn, "Work", CategoryKind::Expense).unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell.read(cx).categories.len() == 4
    });
    let keep = cx.update(|cx| find_child(&shell, cx, "Keep").unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));

    // -- Move: pick the new parent in the form, Move, done.
    let restaurants = cx.update(|cx| find_child(&shell, cx, "Restaurants").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(restaurants, window, cx));
    });
    cx.run_until_parked();
    click(cx, handle, "cat:title-move");
    draw_frame(cx, handle);
    let home_ix = cx.update(|cx| {
        let s = shell.read(cx);
        let food = s.categories.iter().find(|p| p.name == "Food").unwrap().id;
        s.categories
            .iter()
            .filter(|p| p.kind == CategoryKind::Expense && p.id != food)
            .position(|p| p.name == "Home")
            .unwrap()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.move_target.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        Some(gpui_kit::component::IndexPath::new(home_ix)),
                        window,
                        cx,
                    );
                })
            });
        });
    });
    click(cx, handle, "cat:form-move-go");
    wait_until(cx, "Restaurants moves under Home", |cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .any(|p| p.name == "Home" && p.children.iter().any(|c| c.name == "Restaurants"))
    });
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .categories_view
                .read(cx)
                .detail_form
                .is_none(),
            "a successful move closes the Move form"
        );
    });

    // -- Delete (a childless, empty category): the first click arms,
    // Cancel closes form and question together; then the real two-click
    // delete reassigns and closes the form too.
    select_parent(cx, handle, &shell, "Work");
    let work = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Work")
            .unwrap()
            .id
    });
    click(cx, handle, "cat:title-delete");
    draw_frame(cx, handle);
    click(cx, handle, "cat:form-delete-go");
    cx.update(|cx| {
        assert_eq!(
            shell
                .read(cx)
                .categories_view
                .read(cx)
                .confirm_delete_category,
            Some(work),
            "the first click asks instead of deleting"
        );
    });
    click(cx, handle, "cat:form-delete-cancel");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let page = shell.read(cx).categories_view.read(cx);
        assert!(page.detail_form.is_none(), "Cancel closes the form");
        assert!(
            page.confirm_delete_category.is_none(),
            "Cancel disarms the two-click question with it"
        );
    });
    // Reopen, choose where the transactions go, and delete for real.
    click(cx, handle, "cat:title-delete");
    let keep_ix = cx.update(|cx| s_ix(&shell, cx, keep));
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.delete_target.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        Some(gpui_kit::component::IndexPath::new(keep_ix)),
                        window,
                        cx,
                    );
                })
            });
        });
    });
    click(cx, handle, "cat:form-delete-go"); // arm
    click(cx, handle, "cat:form-delete-go"); // run
    wait_until(cx, "Work is deleted", |cx| {
        shell.read(cx).categories.iter().all(|p| p.name != "Work")
    });
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .categories_view
                .read(cx)
                .detail_form
                .is_none(),
            "a successful delete closes the Delete form"
        );
    });
}

/// 3.51: system categories can't be deleted, so their title row has no
/// Delete button at all (and no Rename either, as before).
#[gpui::test]
fn a_system_category_offers_no_delete_button(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    let transfers = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.is_system)
            .unwrap_or_else(|| panic!("the seed has the system Transfers tree"))
            .id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(transfers, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:detail-title"),
        "the system category's detail paints"
    );
    assert!(
        !paints(cx, handle, "cat:title-delete"),
        "a system category has no Delete button"
    );
}

/// 3.51: with Organize and Delete gone, the Rules section starts right
/// under the title divider and takes the panel's remaining height; an
/// open form pushes it down and shortens it.
#[gpui::test]
fn the_rules_section_holds_the_detail_panels_remaining_height(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let food =
            financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, food, "Restaurants").unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell.read(cx).categories.len() == 2
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Food");
    draw_frame(cx, handle);
    let panel = bounds(cx, handle, "cat:detail-panel");
    let title = bounds(cx, handle, "cat:detail-title");
    let rules = bounds(cx, handle, "cat:rules-section");
    // Title, its divider, then Rules: no section stands between them.
    let gap = rules.origin.y - (title.origin.y + title.size.height);
    assert!(
        gap < px(40.),
        "Rules starts right under the title divider: gap {gap:?}"
    );
    // Rules owns the rest of the panel (border 1 px + `p_4`).
    let bottom = panel.origin.y + panel.size.height - px(17.);
    assert!(
        (rules.origin.y + rules.size.height - bottom).abs() < px(1.),
        "Rules reaches the panel's content edge: {:?}",
        rules.origin.y + rules.size.height
    );
    // Opening a form inserts it between: Rules moves down and shrinks.
    click(cx, handle, "cat:title-add-sub");
    let rules_open = bounds(cx, handle, "cat:rules-section");
    assert!(
        rules_open.origin.y > rules.origin.y + px(30.),
        "the open form sits between the title divider and Rules"
    );
    assert!(
        rules_open.size.height < rules.size.height,
        "the rule list gives the form its height"
    );
}

/// 3.51: the title row must fit the 1200 × 720 window with a 60-character
/// name — the name truncates first, the buttons stay painted and inside
/// the panel.
#[gpui::test]
fn the_detail_title_row_fits_a_sixty_character_name(cx: &mut TestAppContext) {
    let long = "c".repeat(60);
    let (handle, shell) = open_shell(cx, |conn| {
        let parent =
            financi_db::categories::create_parent(conn, &long, CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, parent, "A child").unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell.read(cx).categories.len() == 2
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, &long);
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    let panel = bounds(cx, handle, "cat:detail-panel");
    let right = panel.origin.x + panel.size.width;
    for sel in ["cat:title-add-sub", "cat:title-delete"] {
        let b = bounds(cx, handle, sel);
        assert!(
            b.origin.x + b.size.width <= right + px(1.),
            "{sel} stays inside the panel: ends at {:?}, panel {right:?}",
            b.origin.x + b.size.width,
        );
    }
    // The name ellipsizes rather than pushing the buttons out.
    let title = bounds(cx, handle, "cat:detail-title");
    assert!(
        title.size.width <= panel.size.width,
        "the 60-character title truncates inside the panel: title {:?}, panel {:?}",
        title.size.width,
        panel.size.width,
    );
    // And the Move form's row fits for a subcategory too.
    let child = cx.update(|cx| find_child(&shell, cx, "A child").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(child, window, cx));
    });
    cx.run_until_parked();
    click(cx, handle, "cat:title-move");
    let form = bounds(cx, handle, "cat:form-move");
    assert!(
        form.origin.x + form.size.width <= right + px(1.),
        "the Move form fits the panel: ends at {:?}",
        form.origin.x + form.size.width,
    );
}

/// Two parents, each with one subcategory whose rules target that
/// subcategory (3.23: the rule list filters on the selection). Returns
/// `(drinks, transport, coffee_rule, fuel_rule)`.
fn seed_split_rules(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64, i64) {
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let coffee = financi_db::categories::create_child(conn, drinks, "Coffee").unwrap();
    let transport =
        financi_db::categories::create_parent(conn, "Transport", CategoryKind::Expense).unwrap();
    let fuel = financi_db::categories::create_child(conn, transport, "Fuel").unwrap();
    let coffee_rule = financi_db::rules::add_rule(conn, "COFFEE", false, coffee).unwrap();
    let fuel_rule = financi_db::rules::add_rule(conn, "SHELL", false, fuel).unwrap();
    (drinks, transport, coffee_rule, fuel_rule)
}

/// 3.23: the rule list holds only the selected category's rules — a parent
/// shows its subcategories', other categories' rules disappear on a new
/// selection, and a category without rules shows the empty hint instead.
/// Without a selection the detail panel prompts for one.
#[gpui::test]
fn selecting_a_category_lists_only_its_own_rules(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_split_rules(c));
    let (_drinks, _transport, coffee_rule, fuel_rule) = ids;
    // +1: the system `Transfers` parent from migration 0002.
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2 && shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:detail-empty").is_some(),
        "without a selection the detail panel prompts for one"
    );

    select_parent(cx, handle, &shell, "Drinks");
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("cat:rule-row:{coffee_rule}")),
        "a parent's list holds its subcategories' rules"
    );
    assert!(
        !paints(cx, handle, format!("cat:rule-row:{fuel_rule}")),
        "another category's rule is not listed"
    );

    select_parent(cx, handle, &shell, "Transport");
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("cat:rule-row:{fuel_rule}")),
        "the other parent's rule lists after switching"
    );
    assert!(
        !paints(cx, handle, format!("cat:rule-row:{coffee_rule}")),
        "the first parent's rule is gone after switching"
    );

    // A category without rules shows the hint and no rows.
    select_parent(cx, handle, &shell, "Transfers");
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:rules-empty").is_some(),
        "a ruleless category shows the empty hint"
    );
    assert!(
        !paints(cx, handle, format!("cat:rule-row:{coffee_rule}"))
            && !paints(cx, handle, format!("cat:rule-row:{fuel_rule}")),
        "no rule rows paint for the ruleless category"
    );
}

/// 3.53: with a subcategory selected the page already says where the rule
/// goes — the add-rule row has no category picker, and Add rule targets
/// the selection itself.
#[gpui::test]
fn a_selected_subcategory_adds_a_rule_without_a_picker(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_split_rules(c));
    let (_drinks, _transport, _coffee_rule, _fuel_rule) = ids;
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2 && shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    let fuel = cx.update(|cx| find_child(&shell, cx, "Fuel").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(fuel, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("cat:add-rule-row").is_some(),
        "the add-rule row still shows under a subcategory"
    );
    assert!(
        window
            .debug_bounds("cat:rule-subcategory-select:cat-add")
            .is_none(),
        "the row carries no category picker — the selection names the target"
    );
    assert!(
        !paints(cx, handle, format!("label:{SUBCATEGORY_PH}")),
        "no \"Subcategory…\" placeholder paints either"
    );
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern
                    .update(cx, |i, cx| i.set_value("PRET", window, cx))
            });
        });
    });
    let btn = window
        .debug_bounds("cat:add-rule-btn:add")
        .expect("the row's Add rule button");
    window.simulate_click(btn.center(), Modifiers::default());
    wait_until(cx, "the rule added on the selected subcategory", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.pattern == "PRET" && r.category_id == fuel)
    });
}

/// 3.53: under a parent the row picks among the parent's own children by
/// their own names — no default pick, no other category's children, no
/// "Parent › Child" labels — and Add rule without a pick explains itself
/// instead of saving.
#[gpui::test]
fn the_add_rule_picker_lists_the_selected_parents_children(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_split_rules(c));
    let (_drinks, _transport, _coffee_rule, _fuel_rule) = ids;
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2 && shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:rule-subcategory-select:cat-add"),
        "under a parent the row paints its subcategory picker"
    );
    assert!(
        paints(cx, handle, format!("label:{SUBCATEGORY_PH}")),
        "the picker reads {SUBCATEGORY_PH:?} while nothing is picked"
    );
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.categories_page
                .cat_rule_target
                .read(cx)
                .selected_index(cx)
                .is_none(),
            "a fresh parent selection pre-picks nothing"
        );
    });
    // The open menu holds this parent's children, by their own names.
    open_picker_menu(cx, handle, "cat:rule-subcategory-select:cat-add");
    assert!(
        paints(cx, handle, "cat:rule-choice:Coffee"),
        "the parent's own child is listed by its own name"
    );
    assert!(
        !paints(cx, handle, "cat:rule-choice:Fuel"),
        "another parent's child is not in the list"
    );
    assert!(
        !paints(cx, handle, "cat:rule-choice:Drinks › Coffee"),
        "the names are bare, not \"Parent › Child\""
    );
    // Add rule with a pattern but no pick: the error explains, nothing
    // saves. Close the menu first — the click must land on the button.
    let mut window = VisualTestContext::from_window(handle, cx);
    click_past_anything(cx, &mut window);
    draw_frame(cx, handle);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern
                    .update(cx, |i, cx| i.set_value("UBER", window, cx))
            });
        });
    });
    click(cx, handle, "cat:add-rule-btn:add");
    wait_until(cx, "the no-pick error", |cx| {
        shell.read(cx).rules_page.read(cx).rule_error.is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rules_page.read(cx).rule_error.as_deref(),
            Some("Choose the subcategory the rule assigns."),
            "the error names the missing pick"
        );
        assert_eq!(s.rules.len(), 2, "no rule was added");
    });
}

/// 3.53: a parent with no subcategories cannot hold a rule (rules target
/// a child), so a muted hint stands in for the add-rule row; a parent
/// with children keeps the row.
#[gpui::test]
fn a_childless_parent_offers_no_rule_row(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let drinks =
            financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, drinks, "Coffee").unwrap();
        financi_db::categories::create_parent(conn, "Books", CategoryKind::Expense).unwrap();
    });
    // +1: the system `Transfers` parent from migration 0002.
    wait_until(cx, "both parents loaded", |cx| {
        shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Books");
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("cat:add-rule-need-child").is_some(),
        "the childless parent shows the add-a-subcategory hint"
    );
    assert!(
        window.debug_bounds("cat:add-rule-row").is_none(),
        "and no add-rule row"
    );
    select_parent(cx, handle, &shell, "Drinks");
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:add-rule-need-child").is_none(),
        "the hint is gone once the selection has children"
    );
    assert!(
        window.debug_bounds("cat:add-rule-row").is_some(),
        "the parent with children keeps the add-rule row"
    );
}

/// 3.53: the rule list ends with a muted pointer to the Rules page — the
/// editor no longer moves rules — and only while there are rules there
/// to move.
#[gpui::test]
fn the_rule_list_points_at_the_rules_page(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_split_rules(c));
    let (_drinks, _transport, _coffee_rule, _fuel_rule) = ids;
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2 && shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("cat:rules-move-hint").is_some(),
        "a category with rules shows the move hint"
    );
    select_parent(cx, handle, &shell, "Transfers");
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:rules-move-hint").is_none(),
        "a ruleless category shows no move hint"
    );
}

/// 3.23: the header's "New category…" button opens the add-category form
/// in place; Cancel hides it again, a successful add closes it, and so does
/// leaving the page.
#[gpui::test]
fn the_new_category_button_opens_and_closes_the_form(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "the form starts hidden"
    );
    // Click the header button.
    let b = bounds(cx, handle, format!("label:{NEW_CATEGORY}"));
    window.simulate_click(b.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_some(),
        "the click opens the form"
    );
    // Cancel closes it again.
    let cancel = window
        .debug_bounds("cat:new-form-cancel")
        .expect("the form paints its Cancel button");
    window.simulate_click(cancel.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "Cancel closes the form"
    );
    // Reopen, then add a category: success closes the form too.
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .update(cx, |p, _| p.new_category_open = true)
        })
    });
    draw_frame(cx, handle);
    add_parent(cx, handle, &shell, "Books");
    cx.update(|cx| {
        assert!(
            !shell.read(cx).categories_view.read(cx).new_category_open,
            "adding a category closes the form"
        );
    });
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "the closed form paints nothing after the add"
    );
    // And it never survives a page switch.
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .update(cx, |p, _| p.new_category_open = true)
        })
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "leaving the page closes the form"
    );
}

/// 3.81: Escape on the Categories page is the page's own `Cancel`, and it
/// closes one layer per press, innermost first: the rename editor, then
/// the new-category form, then an armed category delete. 3.82 moved the
/// dashboard's Escape binding onto the Dashboard page itself, so with
/// nothing open the keystroke no longer reaches the drill from here —
/// while a layer is open the consumed keystroke must not reach it either.
#[gpui::test]
fn escape_on_the_categories_page_closes_one_layer_per_press(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "chart rows load", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
    });
    // Drill the dashboard chart first: the drill lives on the shell and
    // survives the page switch, so it waits here for the last Escape.
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled into Food", |cx| {
        dash(cx, &shell).read(cx).filter.chart_parent == Some(food)
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    wait_idle(cx, &shell);
    select_parent(cx, handle, &shell, "Food");
    // Open the new-category form through the header button, as a user
    // does, and the rename editor the way the tests here do it (the
    // page's own flag; the Rename… button has no selector).
    click(cx, handle, format!("label:{NEW_CATEGORY}"));
    with_window(cx, handle, |_window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, _| p.renaming = true)
        });
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_some(),
        "the form is open"
    );
    assert!(
        window.debug_bounds("cat:detail-title").is_none(),
        "the rename editor replaces the detail title while open"
    );

    // Actions dispatch along the focus path, so focus the page root —
    // what showing the page does for a user (see the 3.79 Imports test).
    let root = cx.update(|cx| shell.read(cx).categories_view.read(cx).focus.clone());
    with_window(cx, handle, |window, cx| window.focus(&root, cx));

    // 1. The rename editor is innermost: the first Escape closes it and
    // the consumed keystroke reaches neither the form nor the drill.
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            !s.categories_view.read(cx).renaming,
            "Escape closed the rename editor"
        );
        assert_eq!(
            dash(cx, &shell).read(cx).filter.chart_parent,
            Some(food),
            "the consumed Escape did not clear the drill"
        );
    });
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:detail-title").is_some(),
        "the detail title paints again once the editor closes"
    );
    assert!(
        window.debug_bounds("cat:new-form").is_some(),
        "the form is still open after the first Escape"
    );

    // 2. The second Escape closes the new-category form.
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "Escape closed the form"
    );

    // 3. Arm the category delete (the first click only asks), and the
    // third Escape disarms it.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx))
    });
    cx.update(|cx| {
        assert_eq!(
            shell
                .read(cx)
                .categories_view
                .read(cx)
                .confirm_delete_category,
            Some(food),
            "first click arms"
        );
    });
    cx.simulate_keystrokes(handle, "escape");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shell
                .read(cx)
                .categories_view
                .read(cx)
                .confirm_delete_category,
            None,
            "Escape disarms the category delete"
        );
    });

    // 4. With nothing open the keystroke is no longer consumed here: 3.82
    // moved the Escape binding onto the Dashboard page, so the drill waits
    // for an Escape *there*. Visit the dashboard and press it once —
    // Escape still means *clear* on the page that owns the drill.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    let dash_root = cx.update(|cx| dash(cx, &shell).read(cx).focus.clone());
    with_window(cx, handle, |window, cx| window.focus(&dash_root, cx));
    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "drill and legend selection cleared", |cx| {
        dash(cx, &shell).read(cx).filter.chart_parent.is_none()
            && dash(cx, &shell).read(cx).filter.legend_selection.is_none()
    });
}

/// 3.23: at the 1200 × 720 minimum window both panels, the new-category
/// form and a rule row with long texts stay fully inside the page; the
/// long rule pattern truncates to its column.
#[gpui::test]
fn the_categories_page_fits_the_minimum_window(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_categories_fit(c));
    let (child, rule) = ids;
    wait_until(cx, "the rule loaded", |cx| shell.read(cx).rules.len() == 1);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // The widest state of every part: form open, long subcategory selected.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .update(cx, |p, _| p.new_category_open = true);
            s.select_category(child, window, cx);
        });
    });
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    let page_right = px(1200. - PAGE_PAD);
    for (what, sel) in [
        ("the new-category form", "cat:new-form".to_string()),
        ("the tree panel", "cat:tree-panel".to_string()),
        ("the detail panel", "cat:detail-panel".to_string()),
        ("the selected tree row", format!("cat:tree-row:{child}")),
        ("the rule row", format!("cat:rule-row:{rule}")),
    ] {
        let b = bounds(cx, handle, sel);
        assert!(
            b.origin.x + b.size.width <= page_right + px(1.),
            "{what} ends at x {:?}, past the page edge {page_right:?}",
            b.origin.x + b.size.width,
        );
    }

    // The rule pattern truncates: its column is far narrower than the
    // full text needs at 14 px.
    let col = bounds(cx, handle, format!("cat:rule-pattern-col:{rule}"));
    let full_w = TEXT_SM_CHAR_PX * CAT_FIT_PATTERN.chars().count() as f32;
    assert!(
        col.size.width < px(full_w / 2.),
        "the pattern truncates: the column is {:?} wide, the full text \
         needs about {full_w:?}",
        col.size.width,
    );
    // The detail title box cannot outgrow the panel beside it.
    let title = window
        .debug_bounds("cat:detail-title")
        .expect("the detail panel paints the category title");
    let panel = window
        .debug_bounds("cat:detail-panel")
        .expect("the detail panel");
    assert!(
        title.size.width <= panel.size.width,
        "the detail title truncates inside the panel: title {:?}, panel {:?}",
        title.size.width,
        panel.size.width,
    );

    // A tree row fills its panel: a content-width row would leave the
    // counts floating mid-panel and a long name would never truncate, so
    // the row's right edge is the tree panel's content edge (border 1 px
    // + `p_2`).
    let tree = window
        .debug_bounds("cat:tree-panel")
        .expect("the tree panel");
    let row = bounds(cx, handle, format!("cat:tree-row:{child}"));
    let tree_content_right = tree.origin.x + tree.size.width - px(9.);
    assert!(
        (row.origin.x + row.size.width - tree_content_right).abs() < px(1.),
        "the tree row fills the panel to its content edge: row right {:?}, \
         content edge {tree_content_right:?}",
        row.origin.x + row.size.width,
    );
    // A fixed control pinned to the detail panel: the add-rule row's last
    // child is the primary button, so the row must end exactly at the
    // panel's content edge (border 1 px + `p_4`).
    let add_row = window
        .debug_bounds("cat:add-rule-row")
        .expect("the detail panel paints the add-rule row");
    let detail_content_right = panel.origin.x + panel.size.width - px(17.);
    assert!(
        (add_row.origin.x + add_row.size.width - detail_content_right).abs() < px(1.),
        "the add-rule row fills the panel to its content edge: row right \
         {:?}, content edge {detail_content_right:?}",
        add_row.origin.x + add_row.size.width,
    );
}

/// 3.41: a child category whose full label "Drinks › …" is exactly 40
/// characters — too wide for the 200 px trigger (it ellipsizes), but it
/// fits the 320 px open menu.
/// 3.41/3.53: a child category named exactly 40 characters — too wide
/// for the 200 px trigger (it ellipsizes, and the hover tooltip names
/// the selection), but it fits the 320 px open menu.
const LONG_CHILD: &str = "international-subscriptions-limited-2026";

fn long_label() -> String {
    format!("Drinks › {LONG_CHILD}")
}

/// 3.41: like `seed_rule_fixture`, but the rule targets the long-named
/// child, so the Categories picker's menu and the trigger's tooltip have
/// a 40-character name to show. Returns `(rule, long child, short child)`.
fn seed_long_name_rule_fixture(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let long = financi_db::categories::create_child(conn, drinks, LONG_CHILD).unwrap();
    let short = financi_db::categories::create_child(conn, drinks, "Second").unwrap();
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents, description_raw)
         VALUES (?1, ?2, 0, X'00', 'long-name-hash', '2026-11-01', -1000, 'COFFEE MART')",
        (acct, import),
    )
    .unwrap();
    let rule = financi_db::rules::add_rule(conn, "COFFEE", false, long).unwrap();
    (rule, long, short)
}

/// 3.41/3.53: open the shell over the long-name fixture and select the
/// parent, so the Categories detail panel paints its add-rule row and
/// picker. Returns `(handle, shell, rule, long child, short child)`.
fn open_long_name_rule_fixture(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Shell>, i64, i64, i64) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_long_name_rule_fixture(c));
    let (rule, long, short) = ids;
    wait_until(cx, "the long-name fixture loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 1 && s.child_choices.iter().any(|(_, n)| n == &long_label())
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");
    (handle, shell, rule, long, short)
}

/// 3.53: open a rule picker the way a user does — a click on the middle
/// of the trigger — and paint until the (deferred) menu is up.
fn open_picker_menu(cx: &mut TestAppContext, handle: AnyWindowHandle, sel: impl Into<String>) {
    click(cx, handle, sel);
    draw_frame(cx, handle);
    draw_frame(cx, handle);
}

/// 3.53: click outside an open menu to dismiss it (the popup closes on
/// any press that misses it).
fn click_past_anything(cx: &mut TestAppContext, window: &mut VisualTestContext) {
    window.simulate_click(point(px(2.), px(2.)), Modifiers::default());
    cx.run_until_parked();
}

/// 3.53: assert a menu row paints with its text unclipped — the natural
/// (nowrap, absolutely positioned) probe copy fits inside the drawn row
/// — and that the row sits inside the window.
fn assert_menu_item_unclipped(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    label: &str,
    win_w: f32,
) {
    let item = bounds(cx, handle, format!("cat:rule-choice:{label}"));
    let natural = bounds(cx, handle, format!("cat:rule-choice-natural:{label}"));
    assert!(
        f32::from(natural.size.width) <= f32::from(item.size.width) + 1.,
        "{label:?} is clipped in the menu: the text wants {:?}, the row is {:?}",
        natural.size.width,
        item.size.width,
    );
    // The 320 px menu minus the list and row padding; a menu as narrow
    // as the trigger (200 px) fails this.
    assert!(
        f32::from(item.size.width) >= 250.,
        "the menu row is {:?} wide, not the wide (320 px) menu",
        item.size.width,
    );
    assert!(
        f32::from(item.origin.x) + f32::from(item.size.width) <= win_w,
        "the open menu runs past the window's right edge: row ends at {:?}, window is {win_w} px wide",
        f32::from(item.origin.x) + f32::from(item.size.width),
    );
}

/// 3.53: a 40-character child name is too wide for the 200 px trigger —
/// the trigger caps there and its ellipsized pick carries a hover tooltip
/// — while the open menu grows to 320 px and paints the full name
/// unclipped, inside the window.
#[gpui::test]
fn the_subcategory_menu_shows_a_forty_character_name(cx: &mut TestAppContext) {
    let (handle, shell, _rule, _long, _short) = open_long_name_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let name = LONG_CHILD;

    let trigger = window
        .debug_bounds("cat:rule-subcategory-select:cat-add")
        .expect("the Categories page paints the add-rule picker");
    assert!(
        (f32::from(trigger.size.width) - 200.).abs() <= 1.,
        "the picker's trigger is {:?} wide, not the 200 px ceiling",
        trigger.size.width,
    );

    open_picker_menu(cx, handle, "cat:rule-subcategory-select:cat-add");
    assert_menu_item_unclipped(cx, handle, name, 1200.);

    // Pick the long child; the trigger then holds the ellipsized name…
    click(cx, handle, format!("cat:rule-choice:{name}"));
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.categories_page
                .cat_rule_target
                .read(cx)
                .selected_value()
                .map(|v| v.as_str()),
            Some(name),
            "the long child is the picker's selection"
        );
    });

    // …and hovering it shows the full name in a tooltip.
    let trigger = window
        .debug_bounds("cat:rule-subcategory-select:cat-add")
        .expect("the picker trigger");
    window.simulate_mouse_move(trigger.center(), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("cat:category-tooltip:{name}")),
        "hovering the ellipsized trigger shows the full name {name:?} in a tooltip",
    );
}

/// 3.42: in every rule row — the add-rule row on each page and the rule
/// editor — the regex switch sits on the controls' centre line: the
/// switch itself (its shrink-wrap probe, not the 32 px slot around it)
/// centres within 1 px of the Pattern input, the primary button and the
/// slot. 3.48: the editor's controls moved to their own label-less line
/// (no slot, no input beside them), so there the switch centres on Save
/// and Cancel instead.
#[gpui::test]
fn the_regex_switch_lines_up_with_the_rule_fields(cx: &mut TestAppContext) {
    let (handle, shell, rule, _long, _short) = open_long_name_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);

    let centre = |window: &mut VisualTestContext, sel: &'static str, where_: &str| -> f32 {
        let b = window
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} paints in the {where_} row"));
        f32::from(b.origin.y) + f32::from(b.size.height) / 2.
    };
    let aligned = |window: &mut VisualTestContext,
                   where_: &str,
                   switch: &'static str,
                   slot: &'static str,
                   input: &'static str,
                   button: &'static str| {
        let sw = centre(window, switch, where_);
        for (what, sel) in [
            ("the slot around it", slot),
            ("the Pattern input", input),
            ("the primary button", button),
        ] {
            let c = centre(window, sel, where_);
            assert!(
                (c - sw).abs() <= 1.,
                "in the {where_} row {what} centres at y {c}, the regex switch at {sw}",
            );
        }
    };

    // The Categories page's add-rule row.
    aligned(
        &mut window,
        "add-rule",
        "cat:switch:add",
        "cat:regex-slot:add",
        "cat:pattern-input:add",
        "cat:add-rule-btn:add",
    );

    // The rule editor row (3.48): the switch, Save and Cancel share the
    // controls line, so they share its centre — within 1 px of each other.
    start_rule_edit(cx, handle, &shell, rule);
    draw_frame(cx, handle);
    let sw = centre(&mut window, "rules:switch:edit", "rule editor");
    for (what, sel) in [
        ("Save", "rules:edit-save-btn"),
        ("Cancel", "rules:edit-cancel-btn"),
    ] {
        let c = centre(&mut window, sel, "rule editor");
        assert!(
            (c - sw).abs() <= 1.,
            "in the rule editor {what} centres at y {c}, the regex switch at {sw}",
        );
    }

    // The Rules page's add-rule row.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, _cx| p.new_rule_open = true)
        });
    });
    draw_frame(cx, handle);
    aligned(
        &mut window,
        "Rules page add-rule",
        "rules:switch:add",
        "rules:regex-slot:add",
        "rules:pattern-input:add",
        "rules:add-rule-btn:add",
    );
}

/// 3.23: the rule-list headings use the rows' widths, and the Actions
/// label sits on the same line as the others, right-aligned over the
/// rows' buttons (3.24 MUST-KEEP). A stacked label-plus-buttons column
/// measured 83.5 px tall, slid the Actions label 64 px up, and pushed
/// the Type/Category headings 62 px right of their columns. 3.49: the
/// third heading is the clickable Uses one over the Uses column.
#[gpui::test]
fn the_rule_list_headings_sit_over_their_columns(cx: &mut TestAppContext) {
    let (handle, _shell, rule, _first, _second) = open_rule_fixture(cx);
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for (label, heading_sel, column_sel) in [
        (
            "Pattern",
            "cat:rule-heading-pattern",
            format!("cat:rule-pattern-col:{rule}"),
        ),
        (
            "Type",
            "cat:rule-heading-type",
            format!("cat:rule-type-col:{rule}"),
        ),
        (
            "Uses",
            "cat:rule-heading-uses",
            format!("cat:rule-uses-col:{rule}"),
        ),
    ] {
        let heading = window
            .debug_bounds(heading_sel)
            .unwrap_or_else(|| panic!("the rules list paints the {label:?} heading"));
        let column = bounds(cx, handle, column_sel);
        assert!(
            (heading.origin.x - column.origin.x).abs() < px(1.),
            "the {label:?} heading starts where its column starts: \
             heading x {:?}, column x {:?}",
            heading.origin.x,
            column.origin.x,
        );
    }
    // The Actions label is right-aligned over the buttons: its right
    // edge is the row's right edge (the Delete… button ends there).
    let actions = window
        .debug_bounds("cat:rule-heading-actions")
        .expect("the rules list paints the Actions heading");
    let row = bounds(cx, handle, format!("cat:rule-row:{rule}"));
    assert!(
        (actions.origin.x + actions.size.width - (row.origin.x + row.size.width)).abs() < px(1.),
        "the Actions heading ends where the row's buttons end: \
         heading right {:?}, row right {:?}",
        actions.origin.x + actions.size.width,
        row.origin.x + row.size.width,
    );
    // A single line: every heading label shares one line, and the whole
    // heading is no taller than a line of labels.
    let type_heading = window
        .debug_bounds("cat:rule-heading-type")
        .expect("the rules list paints the Type heading");
    assert!(
        (actions.origin.y - type_heading.origin.y).abs() < px(1.),
        "the Actions heading sits on the same line as the others: \
         Actions y {:?}, Type y {:?}",
        actions.origin.y,
        type_heading.origin.y,
    );
    let heading = window
        .debug_bounds("cat:rule-heading-row")
        .expect("the rules list paints its heading row");
    assert!(
        heading.size.height < px(24.),
        "the heading is a single line tall: it measures {:?}",
        heading.size.height,
    );
    assert!(
        heading.origin.y < row.origin.y,
        "the heading sits above the rows: heading y {:?}, row y {:?}",
        heading.origin.y,
        row.origin.y,
    );
}

// -- Uses column (3.49) ------------------------------------------------------

/// 3.49: the rule list's third column is Uses — the transactions a rule
/// categorizes, the same number the Rules page shows — and nothing of the
/// old Category column paints. The COFFEE rule wins the fixture's two
/// unlocked rows, so its cell reads 2.
#[gpui::test]
fn the_rule_list_shows_uses_instead_of_category(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    assert!(
        paints(cx, handle, format!("cat:rule-uses:{rule}:2")),
        "the Uses cell paints the two rows the rule categorizes"
    );
    assert!(
        paints(cx, handle, "cat:rule-heading-uses"),
        "the list paints the Uses heading"
    );
    assert!(
        !paints(cx, handle, "cat:rule-heading-category")
            && !paints(cx, handle, format!("cat:rule-category-col:{rule}")),
        "nothing of the old Category column paints"
    );
    // The Rules page shows the same number for the same rule: both pages
    // read the one `rule_use_stats` mirror off the shared snapshot.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shell
                .read(cx)
                .rules_page
                .read(cx)
                .rule_uses
                .get(&rule)
                .copied(),
            Some(2),
            "the Rules page's Uses for the rule matches the Categories list"
        );
    });
}

/// 3.49: Uses counts only the rows a rule wins. `COFFEE SHOP` (added on
/// top of the `seed_rule_fixture` rule) is the longer pattern, so it wins
/// both unlocked rows; `COFFEE` wins nothing and reads 0 (muted), and the
/// locked row counts for neither.
fn seed_overlapping_rules(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64) {
    let (coffee_rule, _first, second) = seed_rule_fixture(conn);
    let shop_rule = financi_db::rules::add_rule(conn, "COFFEE SHOP", false, second).unwrap();
    (coffee_rule, shop_rule, second)
}

#[gpui::test]
fn the_uses_cell_counts_only_the_rows_the_rule_wins(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_overlapping_rules(c));
    let (coffee_rule, shop_rule, second) = ids;
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2
            && shell.read(cx).cat_counts.get(&second).copied().unwrap_or(0) == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    let drinks = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Drinks")
            .unwrap()
            .id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(drinks, window, cx));
    });
    cx.run_until_parked();
    resize_and_draw(cx, handle, 1200., 720.);
    assert!(
        paints(cx, handle, format!("cat:rule-uses:{shop_rule}:2")),
        "the winning rule counts the two unlocked rows it won"
    );
    assert!(
        paints(cx, handle, format!("cat:rule-uses:{coffee_rule}:0")),
        "the rule that wins no rows reads 0 (and paints muted)"
    );
}

/// 3.49: the count follows the rule list's own edits — narrowing the
/// pattern drops the rows the rule no longer wins, and deleting the rule
/// takes its row out of the list.
#[gpui::test]
fn the_uses_count_follows_rule_edits_and_deletes(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    assert!(paints(cx, handle, format!("cat:rule-uses:{rule}:2")));

    // Edit "COFFEE" down to "SHOP B": one row, and the cell follows.
    start_rule_edit(cx, handle, &shell, rule);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_edit_pattern
                    .update(cx, |i, cx| i.set_value("SHOP B", window, cx))
            });
            s.rules_page
                .update(cx, |p, cx| p.save_rule_edit(window, cx));
        });
    });
    wait_until(cx, "the count drops to one", |cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .uses
            .get(&rule)
            .copied()
            == Some(1)
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("cat:rule-uses:{rule}:1")),
        "the edited rule's Uses cell repaints with the new count"
    );

    // Delete (two clicks): the row leaves the list.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.arm_delete_rule(rule, window, cx))
        });
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page
                .update(cx, |p, cx| p.arm_delete_rule(rule, window, cx))
        });
    });
    wait_until(cx, "the rule deleted", |cx| shell.read(cx).rules.is_empty());
    wait_until(cx, "the page applies the delete", |cx| {
        !shell
            .read(cx)
            .categories_view
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule)
    });
    draw_frame(cx, handle);
    assert!(
        !paints(cx, handle, format!("cat:rule-row:{rule}")),
        "the deleted rule's row is gone"
    );
}

/// 3.49: with a parent selected, each rule from a subcategory carries a
/// muted tag of that subcategory's name after its pattern; selecting the
/// subcategory itself paints no tags.
#[gpui::test]
fn a_parent_view_tags_each_rule_with_its_subcategory(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    // The fixture selected Drinks, the rules' parent; the rule targets First.
    assert!(
        paints(cx, handle, format!("cat:rule-subcat:{rule}:First")),
        "a parent's rule rows carry the subcategory tag"
    );
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, format!("cat:rule-row:{rule}")),
        "the rule still lists under its own subcategory"
    );
    assert!(
        !paints(cx, handle, format!("cat:rule-subcat:{rule}:First")),
        "a subcategory selection paints no tag"
    );
}

/// 3.49 fixture: one parent, two subcategories, and two rules whose Uses
/// run against the pattern order — `UBER` categorizes two rows but sorts
/// after `AMAZON` (one row) alphabetically. Returns `(drinks, uber, amazon)`.
fn seed_uses_rules(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let coffee = financi_db::categories::create_child(conn, drinks, "Coffee").unwrap();
    let taxi = financi_db::categories::create_child(conn, drinks, "Taxi").unwrap();
    for (n, desc) in ["AMAZON ORDER", "UBER TRIP A", "UBER TRIP B"]
        .iter()
        .enumerate()
    {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
            (acct, import, n as i64, format!("uses-hash-{n}"), *desc),
        )
        .unwrap();
    }
    let amazon = financi_db::rules::add_rule(conn, "AMAZON", false, coffee).unwrap();
    let uber = financi_db::rules::add_rule(conn, "UBER", false, taxi).unwrap();
    (drinks, uber, amazon)
}

/// 3.49: clicking the Uses heading sorts the list by Uses descending; the
/// Pattern heading sorts back, like the Rules page's pair.
#[gpui::test]
fn the_uses_heading_sorts_the_rule_list(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_uses_rules(c));
    let (drinks, uber, amazon) = ids;
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(drinks, window, cx));
    });
    cx.run_until_parked();
    resize_and_draw(cx, handle, 1200., 720.);
    let y = |cx: &mut TestAppContext, id| {
        f32::from(bounds(cx, handle, format!("cat:rule-row:{id}")).origin.y)
    };
    assert!(
        y(cx, amazon) < y(cx, uber),
        "the default order is pattern ascending"
    );
    assert!(paints(cx, handle, format!("cat:rule-uses:{uber}:2")));

    click(cx, handle, "cat:rule-heading-uses");
    draw_frame(cx, handle);
    assert!(
        y(cx, uber) < y(cx, amazon),
        "the Uses heading sorts by Uses descending"
    );
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .categories_view
                .read(cx)
                .cat_rules_sort_by_uses
        )
    });

    click(cx, handle, "cat:rule-heading-pattern");
    draw_frame(cx, handle);
    assert!(
        y(cx, amazon) < y(cx, uber),
        "the Pattern heading sorts back to pattern order"
    );
    cx.update(|cx| {
        assert!(
            !shell
                .read(cx)
                .categories_view
                .read(cx)
                .cat_rules_sort_by_uses
        )
    });
}

/// 3.49: hovering the Uses heading explains the column.
#[gpui::test]
fn the_uses_heading_tells_what_it_counts(cx: &mut TestAppContext) {
    let (handle, _shell, _rule, _first, _second) = open_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let heading = bounds(cx, handle, "cat:rule-heading-uses");
    window.simulate_mouse_move(heading.center(), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "cat:rule-heading-uses-tip"),
        "hovering the Uses heading shows the tooltip"
    );
}

// -- row context menu (2.9) -------------------------------------------------
