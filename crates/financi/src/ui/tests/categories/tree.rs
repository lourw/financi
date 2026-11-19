//! The category tree: add, delete with reassignment, move rollback,
//! selection, virtualization and the two lists' scroll offsets.

use super::*;

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

    // Categorize the top merchant's rows onto Online, so a delete has work
    // to move. 3.54: `rule_target` is fed on the Rules page's next draw,
    // so show it and settle before picking.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    wait_idle(cx, &shell);
    draw_frame(cx, handle);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern.update(cx, |i, cx| {
                    i.set_value(merchant.clone(), window, cx);
                })
            });
            s.rules_page.update(cx, |p, cx| {
                p.rule_target.update(cx, |sel, cx| {
                    sel.set_selected_value(&online, window, cx);
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
        s_select(&shell, cx, keep, window);
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
/// 3.54: the confirming Delete click drops the node and moves its counts
/// to the reassign target in its own frame; the DB round-trip only
/// reconciles.
#[gpui::test]
fn a_confirmed_delete_leaves_the_tree_in_its_own_frame(cx: &mut TestAppContext) {
    let (handle, shell, _rule, first, second) = open_rule_fixture(cx);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(first, window, cx);
            s.arm_delete_category(window, cx); // first click arms
        });
        s_select(&shell, cx, second, window);
        shell.update(cx, |s, cx| {
            s.arm_delete_category(window, cx); // second click runs it
            // Same frame, before the executor runs anything: the mirror
            // already shows the outcome — the node gone, its two
            // transactions with it, and the rule re-pointed.
            let view = s.categories_view.read(cx);
            assert_eq!(view.cat_pending, Some(PendingCatWrite::Delete));
            let drinks = view.tree.iter().find(|p| p.name == "Drinks").unwrap();
            assert!(drinks.children.iter().all(|c| c.id != first));
            assert_eq!(view.counts.get(&first), None);
            assert_eq!(view.counts.get(&second), Some(&3));
            assert!(view.rules.iter().all(|r| r.category_id != first));
        });
    });
    wait_idle(cx, &shell);
    // And the database agrees.
    cx.update(|cx| {
        assert!(find_child(&shell, cx, "First").is_none());
        let s = shell.read(cx);
        assert_eq!(s.cat_counts.get(&second), Some(&3));
        assert!(
            s.categories_view
                .read(cx)
                .tree
                .iter()
                .all(|p| p.id != first)
        );
    });
}
/// 3.54: a move the database refuses — the target was pulled out from
/// under the app — rolls the tree back to its snapshot and shows an
/// error. The refusal is injected as a `BEFORE UPDATE` trigger so the row
/// itself stays put and the reconciling refresh reads a stable database.
#[gpui::test]
fn a_refused_move_rolls_the_node_back_and_shows_an_error(cx: &mut TestAppContext) {
    let (handle, shell, _rule, first, _second) = open_rule_fixture(cx);
    add_parent(cx, handle, &shell, "Home");
    let db = cx.update(|cx| shell.read(cx).db.clone());
    block_on(db.call(move |c| {
        c.execute(
            "CREATE TRIGGER test_no_parent_change BEFORE UPDATE ON categories \
             BEGIN SELECT RAISE(ABORT, 'test: parent change refused'); END",
            [],
        )
        .map(|_| ())
    }))
    .expect("the trigger job ran")
    .expect("the trigger is created");

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(first, window, cx);
        });
        let home_ix = {
            let s = shell.read(cx);
            let drinks = s.categories.iter().find(|p| p.name == "Drinks").unwrap().id;
            s.categories
                .iter()
                .filter(|p| p.kind == CategoryKind::Expense && p.id != drinks)
                .position(|p| p.name == "Home")
                .unwrap()
        };
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
            s.move_child_to_selected_parent(window, cx);
            // Same frame: the node hangs under Home, off Drinks.
            let view = s.categories_view.read(cx);
            assert_eq!(view.cat_pending, Some(PendingCatWrite::Move));
            assert!(
                view.tree
                    .iter()
                    .find(|p| p.name == "Home")
                    .unwrap()
                    .children
                    .iter()
                    .any(|c| c.id == first)
            );
            assert!(
                view.tree
                    .iter()
                    .find(|p| p.name == "Drinks")
                    .unwrap()
                    .children
                    .iter()
                    .all(|c| c.id != first)
            );
        });
    });
    wait_until(cx, "the refused move to land", |cx| {
        let view = shell.read(cx).categories_view.read(cx);
        view.cat_pending.is_none() && view.cat_error.is_some()
    });
    // The snapshot is back: First under Drinks again, Home empty.
    cx.update(|cx| {
        let view = shell.read(cx).categories_view.read(cx);
        assert!(
            view.tree
                .iter()
                .find(|p| p.name == "Drinks")
                .unwrap()
                .children
                .iter()
                .any(|c| c.id == first)
        );
        assert!(
            view.tree
                .iter()
                .find(|p| p.name == "Home")
                .unwrap()
                .children
                .iter()
                .all(|c| c.id != first)
        );
    });
    block_on(db.call(|c| {
        c.execute("DROP TRIGGER test_no_parent_change", [])
            .map(|_| ())
    }))
    .expect("the trigger job ran")
    .expect("the trigger is dropped");
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
