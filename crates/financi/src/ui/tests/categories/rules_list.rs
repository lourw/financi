//! The detail panel's rule list: per-category filtering, the shared
//! add-rule row, uses counts and the uses-heading sort.

use super::*;

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
        paints(
            cx,
            handle,
            "cat:rule-heading-uses-tip:Transactions this rule categorizes"
        ),
        "hovering the Uses heading shows the tip, its exact words pinned (H5)"
    );
}

// -- row context menu (2.9) -------------------------------------------------
