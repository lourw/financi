//! Dashboard row menu: hide/unhide, category lock and the rule offer.

use financi_core::rules::CompiledRules;
use gpui_kit::component::menu::PopupMenu;

use super::*;

/// A minimal row for the pure menu-model test.
fn browse_row(hidden: bool, paired: bool) -> TransactionRow {
    TransactionRow {
        id: 1,
        account: "American Express · Cobalt".into(),
        posted_date: "2026-10-31".into(),
        description: "COFFEE    VANCOUVER".into(),
        merchant: Some("COFFEE".into()),
        category: None,
        category_id: None,
        amount_cents: -450,
        hidden,
        transfer_pair_id: paired.then_some(2),
        category_locked: false,
    }
}

#[test]
fn menu_shows_hide_for_a_visible_row_and_unhide_only_for_a_hidden_one() {
    assert_eq!(
        row_menu_items(&browse_row(false, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Hide],
        "a visible row offers Hide, never Unhide"
    );
    assert_eq!(
        row_menu_items(&browse_row(true, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Unhide],
        "a hidden row offers Unhide, never Hide"
    );
    assert_eq!(
        row_menu_items(&browse_row(false, true)),
        [
            RowMenuItem::ChangeCategory,
            RowMenuItem::Hide,
            RowMenuItem::Unpair
        ],
        "a paired row also offers Unpair"
    );
    assert_eq!(
        row_menu_items(&browse_row(true, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Unhide],
        "an unpaired row never offers Unpair"
    );
}

/// 3.76: a menu or a cell asked for a row index the table no longer has
/// — a detached write can empty `rows` between the click and the menu
/// opening — yields nothing rather than a panic.
#[gpui::test]
fn row_menu_and_cells_past_the_end_are_empty(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    with_window(cx, handle, |window, cx| {
        let table = dash(cx, &shell).read(cx).table.clone();
        assert!(
            table.read(cx).delegate().rows.is_empty(),
            "fresh table, no rows"
        );
        // `context_menu` takes an owned menu, and only `PopupMenu::build`
        // hands one out — pass it to the delegate the way the table's own
        // right-click wiring does.
        let _menu = PopupMenu::build(window, cx, |menu, window, cx| {
            table.update(cx, |table, cx| {
                table.delegate_mut().context_menu(0, menu, window, cx)
            })
        });
        table.update(cx, |table, cx| {
            let _cell = table.delegate_mut().render_td(0, 0, window, cx);
        });
    });
}

/// 3.29: the *Show rule* item finds the rule that decided the row, or says
/// why it cannot: a hand-set (locked) row, the built-in card payment, or no
/// rule matching the description. The reason rides in the item's label.
#[test]
fn show_rule_finds_the_deciding_rule_or_gives_the_disabled_reason() {
    let coffee = Rule {
        id: 7,
        pattern: "COFFEE".into(),
        is_regex: false,
        category_id: 3,
    };
    let rules = CompiledRules::new(&[coffee]);
    // An unlocked row that a rule matches: the item works, plain label.
    let row = browse_row(false, false);
    assert_eq!(row_show_rule(&row, &rules, None), RowShowRule::Found(7));
    assert_eq!(row_show_rule(&row, &rules, None).menu_label(), "Show rule");
    // A locked row was set by hand, whatever the rules would say.
    let mut locked = browse_row(false, false);
    locked.category_locked = true;
    locked.category_id = Some(9);
    assert_eq!(row_show_rule(&locked, &rules, None), RowShowRule::SetByHand);
    assert_eq!(
        row_show_rule(&locked, &rules, Some(4)),
        RowShowRule::SetByHand
    );
    // Locked to the built-in card-payment category: its own reason, even
    // though a rule would have matched (card payments beat the rules).
    assert_eq!(
        row_show_rule(&locked, &rules, Some(9)),
        RowShowRule::CardPayment
    );
    // A locked row with no category at all still reads as hand-set.
    let mut locked_none = browse_row(false, false);
    locked_none.category_locked = true;
    assert_eq!(
        row_show_rule(&locked_none, &rules, Some(9)),
        RowShowRule::SetByHand
    );
    // Nothing matches: there is no rule to show.
    let mut unmatched = browse_row(false, false);
    unmatched.description = "PARKING FEE METRO".into();
    assert_eq!(row_show_rule(&unmatched, &rules, None), RowShowRule::NoRule);
    // The disabled labels say why, in the menu.
    assert_eq!(
        RowShowRule::SetByHand.menu_label(),
        "Show rule — Set by hand"
    );
    assert_eq!(
        RowShowRule::CardPayment.menu_label(),
        "Show rule — Card payment (built in)"
    );
    assert_eq!(
        RowShowRule::NoRule.menu_label(),
        "Show rule — No rule matches"
    );
}

#[gpui::test]
fn changing_a_rows_category_locks_it_and_offers_the_rule(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (m, target, group) = import_and_top_merchant(cx, handle, &shell);
    assert!(group.len() >= 2, "the merchant has several rows");
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    add_parent(cx, handle, &shell, "Home");
    add_child(cx, handle, &shell, "Groceries");
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_row_category(target, online, window, cx)
        });
    });
    wait_until(cx, "target categorized and offer up", |cx| {
        dash(cx, &shell).read(cx).rule_offer == Some((target, online))
            && table_rows(&shell, cx)
                .iter()
                .any(|r| r.id == target && r.category.as_deref() == Some("Online"))
    });

    // A later rule for the same merchant moves the siblings but never the
    // manually-set (locked) row.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_page.update(cx, |p, cx| {
                p.rule_pattern.update(cx, |i, cx| {
                    i.set_value(m.clone(), window, cx);
                })
            });
            // 3.59: the grouped picker items carry the sub-category id
            // as their value, so the pick resolves by value (a sectioned
            // menu's `IndexPath` is not a flat row number).
            s.rules_page.update(cx, |p, cx| {
                p.rule_target
                    .update(cx, |sel, cx| sel.set_selected_value(&groceries, window, cx));
            });
            s.rules_page.update(cx, |p, cx| p.add_rule(window, cx));
        });
    });
    wait_until(cx, "siblings moved, locked target held", |cx| {
        let rows = table_rows(&shell, cx);
        let cat = |id: i64| {
            rows.iter()
                .find(|r| r.id == id)
                .and_then(|r| r.category.clone())
        };
        cat(target).as_deref() == Some("Online")
            && group
                .iter()
                .filter(|id| **id != target)
                .any(|id| cat(*id).as_deref() == Some("Groceries"))
    });
}

#[gpui::test]
fn applying_the_row_offer_categorizes_the_whole_merchant(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (m, target, group) = import_and_top_merchant(cx, handle, &shell);
    assert!(group.len() >= 2, "the merchant has several rows");
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let all_uncategorized = cx.update(|cx| {
        table_rows(&shell, cx)
            .iter()
            .all(|r| !group.contains(&r.id) || r.category.is_none())
    });
    assert!(all_uncategorized, "the group starts uncategorized");

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_row_category(target, online, window, cx)
        });
    });
    wait_until(cx, "offer up", |cx| {
        dash(cx, &shell).read(cx).rule_offer == Some((target, online))
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.apply_row_rule(target, window, cx));
    });
    wait_until(cx, "rule created and offer cleared", |cx| {
        dash(cx, &shell).read(cx).rule_offer.is_none()
            && shell.read(cx).rules.iter().any(|r| r.pattern == m)
    });
    // 3.56: the rules list lands with the rule write, the table rows with
    // the dashboard job it triggers, so wait for the rows themselves.
    wait_until(cx, "every row of the merchant is Online", |cx| {
        let rows = table_rows(&shell, cx);
        group.iter().all(|id| {
            rows.iter()
                .find(|r| r.id == *id)
                .and_then(|r| r.category.as_deref())
                == Some("Online")
        })
    });
}

#[gpui::test]
fn hiding_and_unhiding_a_row_updates_the_table_and_sidebar_counts(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (_merchant, target, _group) = import_and_top_merchant(cx, handle, &shell);
    let (before_total, before_uncategorized) = cx.update(|cx| {
        (
            shell.read(cx).total_visible,
            shell.read(cx).uncategorized_visible,
        )
    });
    let target_visible = cx.update(|cx| table_rows(&shell, cx).iter().any(|r| r.id == target));
    assert!(target_visible, "the target row is visible before hiding it");

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_row_hidden(target, true, window, cx));
    });
    wait_until(cx, "hidden row leaves table and counts", |cx| {
        !table_rows(&shell, cx).iter().any(|r| r.id == target)
            && shell.read(cx).total_visible == before_total - 1
            && shell.read(cx).uncategorized_visible == before_uncategorized - 1
    });

    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_row_hidden(target, false, window, cx));
    });
    wait_until(cx, "unhidden row returns to table and counts", |cx| {
        table_rows(&shell, cx).iter().any(|r| r.id == target)
            && shell.read(cx).total_visible == before_total
            && shell.read(cx).uncategorized_visible == before_uncategorized
    });
}

/// 3.84: the row category change answers in the click's frame. With the
/// worker held on the write the click starts, the table row already shows
/// the new category (locked, with the child's own label from the page's
/// category mirror) while the DB still holds the old one. The write is
/// then made to fail — the category is deleted behind the page's back,
/// so the DB refuses it as unknown — and the rollback puts the saved row
/// back exactly. The page is hidden before the failure lands so the
/// write's reload can't overwrite the rollback before it is observed: a
/// hidden page defers its apply until it shows.
#[gpui::test]
fn row_category_change_paints_in_the_click_frame_and_rolls_back(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (_m, target, _group) = import_and_top_merchant(cx, handle, &shell);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let before = cx.update(|cx| {
        table_rows(&shell, cx)
            .into_iter()
            .find(|r| r.id == target)
            .expect("target row is in the table")
    });
    assert_eq!(before.category, None, "the target starts uncategorized");

    // Make the write fail: delete the category behind the page's back.
    // Its mirror still lists "Online", so the click paints optimistically
    // and `set_category` then refuses the unknown id.
    let db = cx.update(|cx| shell.read(cx).db.clone());
    block_on(db.call(move |c| {
        c.execute("DELETE FROM categories WHERE id = ?1", [online])
            .map(|_| ())
    }))
    .expect("the delete job ran")
    .expect("the category delete succeeds");

    let done = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_row_category(target, online, window, cx)
        });
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let row = table_rows(&shell, cx)
            .into_iter()
            .find(|r| r.id == target)
            .expect("the row stays in the table");
        assert_eq!(
            row.category.as_deref(),
            Some("Online"),
            "the row paints the new category before the write has run"
        );
        assert_eq!(row.category_id, Some(online));
        assert!(row.category_locked, "the painted row reads as hand-set");
    });

    // Hide the page, then let the write fail.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    drop(done);
    wait_until(cx, "the failed write reports an error", |cx| {
        last_toast(&shell, cx).is_some_and(|(_, is_err)| is_err)
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        let row = rows
            .iter()
            .find(|r| r.id == target)
            .expect("the rolled-back row is still in the table");
        assert_eq!(row.category, before.category, "the label is back");
        assert_eq!(row.category_id, before.category_id, "and the id");
        assert_eq!(row.category_locked, before.category_locked, "and the lock");
    });
}

/// 3.84: hiding a row with *Show hidden* off takes it out of the table in
/// the click's frame; a failed write (the transaction is deleted behind
/// the page's back first) puts it back at its exact index.
#[gpui::test]
fn row_hide_leaves_in_the_click_frame_and_rolls_back_to_its_slot(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (_m, target, _group) = import_and_top_merchant(cx, handle, &shell);
    let index = cx.update(|cx| {
        table_rows(&shell, cx)
            .iter()
            .position(|r| r.id == target)
            .expect("target row is in the table")
    });

    // Make the write fail: an unknown transaction is refused by `set_hidden`.
    let db = cx.update(|cx| shell.read(cx).db.clone());
    block_on(db.call(move |c| {
        c.execute("DELETE FROM transactions WHERE id = ?1", [target])
            .map(|_| ())
    }))
    .expect("the delete job ran")
    .expect("the transaction delete succeeds");

    let done = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_row_hidden(target, true, window, cx));
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert!(
            !table_rows(&shell, cx).iter().any(|r| r.id == target),
            "the row leaves the table before the write has run"
        );
    });

    // Hide the page, then let the write fail.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    drop(done);
    wait_until(cx, "the failed write reports an error", |cx| {
        last_toast(&shell, cx).is_some_and(|(_, is_err)| is_err)
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        assert_eq!(
            rows.iter().position(|r| r.id == target),
            Some(index),
            "the rolled-back row returns to its exact slot"
        );
        assert!(!rows[index].hidden, "with the hidden flag back off");
    });
}

/// 3.84: with *Show hidden* on the row stays visible, so the hide is an
/// in-place flip of its `hidden` flag — painted in the click's frame,
/// before the write's answer.
#[gpui::test]
fn hiding_a_row_under_show_hidden_flips_its_flag_in_the_click_frame(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (_m, target, _group) = import_and_top_merchant(cx, handle, &shell);
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_show_hidden(true, window, cx));
    });
    wait_idle(cx, &shell);

    let done = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.set_row_hidden(target, true, window, cx));
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let row = table_rows(&shell, cx)
            .into_iter()
            .find(|r| r.id == target)
            .expect("the row stays in the table with Show hidden on");
        assert!(row.hidden, "the hidden flag flips before the write has run");
    });
    drop(done);
    wait_until(cx, "the write lands and the reload keeps it hidden", |cx| {
        table_rows(&shell, cx)
            .iter()
            .any(|r| r.id == target && r.hidden)
    });
}

// ---------------------------------------------------------------------------
// Date range control (task 3.2)
