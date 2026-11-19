//! The page's visible wording: control labels and the no-parent/child
//! rule.

use super::*;

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
    // 3.96: adding a subcategory keeps the parent selected; re-select it
    // anyway so the scan below reads the parent's panel whatever happens
    // to the selection on the way here.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(parent_id, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    for sel in [
        // 3.91: the tree's heading belongs to the shared list, so it
        // carries the list's `{prefix}:heading:{column}` selector.
        "cat:heading:Category tree",
        "label:New category name",
        "label:Category type",
        "label:Rules",
        "label:Pattern",
        // 3.103: the add-rule form's picker is label-less (3.48 line 2);
        // it reads by its placeholder.
        "label:Subcategory…",
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
        "cat:heading:Category tree",
        "label:New category name",
        "label:Category type",
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
        s_select(&shell, cx, child, window);
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
