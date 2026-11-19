//! The detail panel's inline forms: open/close, add, rename, move,
//! delete, and the loading state writes show.

use super::*;

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
/// 3.96: adding a subcategory keeps the parent selected (user request,
/// 2026-11-15): the detail panel stays on the parent, the input is empty
/// and still focused for the next name, and the new child appears in the
/// tree under the parent and in the parent's rule-target picker.
#[gpui::test]
fn a_successful_subcategory_add_keeps_the_parent_selected(cx: &mut TestAppContext) {
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
    wait_until(cx, "the subcategory lands", |cx| {
        find_child(&shell, cx, "Groceries").is_some()
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let page = shell.read(cx).categories_view.read(cx);
        assert_eq!(
            page.selected_category,
            Some(food),
            "the parent stays selected"
        );
        assert!(
            page.new_child_name.read(cx).value().is_empty(),
            "the input is empty for the next name"
        );
    });
    let focused = with_window(cx, handle, |window, cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .new_child_name
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "focus stays in the input for the next name");
    // The detail panel still shows the parent: its title row carries the
    // parent-only Add subcategory button, not a child's Move. Had the
    // selection jumped to the new row, these two would swap.
    assert!(
        paints(cx, handle, "cat:title-add-sub"),
        "the detail panel still shows the parent"
    );
    assert!(
        !paints(cx, handle, "cat:title-move"),
        "the panel did not switch to the new child"
    );
    // The new child paints in the tree, and in the parent's add-rule
    // picker (3.53) — which didn't exist before: a childless parent shows
    // the add-a-subcategory hint in place of the add-rule row.
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    assert!(
        paints(cx, handle, format!("cat:tree-row:{child}")),
        "the new child is in the tree"
    );
    open_picker_menu(cx, handle, "cat:rule-subcategory-select:cat-add");
    assert!(
        paints(cx, handle, "cat:rule-choice:Groceries"),
        "the new child is in the parent's rule-target picker"
    );
}
/// 3.96: Enter adds the subcategory and focus stays in the cleared input,
/// so three names typed in a row go in as three children (ui-conventions
/// *Keyboard*: "Enter submits the focused form").
#[gpui::test]
fn enter_adds_subcategories_one_after_another(cx: &mut TestAppContext) {
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
    // The user clicks into the input; from there Enter does the adding.
    with_window(cx, handle, |window, cx| {
        let input = shell
            .read(cx)
            .categories_view
            .read(cx)
            .new_child_name
            .clone();
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    for name in ["Groceries", "Meals", "Coffee"] {
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, cx| {
                s.categories_view
                    .read(cx)
                    .new_child_name
                    .clone()
                    .update(cx, |i, cx| i.set_value(name, window, cx));
            });
        });
        cx.simulate_keystrokes(handle, "enter");
        let owned = name.to_string();
        let sh = shell.clone();
        wait_until(cx, &format!("{owned} lands"), move |cx| {
            find_child(&sh, cx, &owned).is_some()
        });
    }
    cx.update(|cx| {
        let s = shell.read(cx);
        let food_row = s.categories.iter().find(|p| p.name == "Food").unwrap();
        let names: Vec<&str> = food_row.children.iter().map(|c| c.name.as_str()).collect();
        for want in ["Groceries", "Meals", "Coffee"] {
            assert!(names.contains(&want), "{want} added under Food: {names:?}");
        }
        assert_eq!(names.len(), 3, "exactly the three typed names: {names:?}");
        assert_eq!(
            s.categories_view.read(cx).selected_category,
            Some(food),
            "the parent stayed selected through all three adds"
        );
    });
    draw_frame(cx, handle);
    for want in ["Groceries", "Meals", "Coffee"] {
        let id = cx.update(|cx| find_child(&shell, cx, want).unwrap());
        assert!(
            paints(cx, handle, format!("cat:tree-row:{id}")),
            "{want} is visible in the tree"
        );
    }
}
/// 3.96: a refused add — the database's duplicate-name refusal — keeps the
/// typed name in the input and shows the error. Nothing clears, the form
/// stays open and the parent stays selected, so the name can be fixed and
/// Enter pressed again.
#[gpui::test]
fn a_failed_subcategory_add_keeps_the_typed_name(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let food =
            financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .any(|p| p.name == "Food" && p.children.len() == 1)
    });
    let food = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food")
            .unwrap()
            .id
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
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
    wait_until(cx, "the duplicate refusal", |cx| {
        shell.read(cx).categories_view.read(cx).cat_error.is_some()
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let page = shell.read(cx).categories_view.read(cx);
        assert_eq!(
            page.cat_error.as_deref(),
            Some("a category named \"Groceries\" already exists under the same category"),
            "the refusal is shown"
        );
        assert_eq!(
            page.new_child_name.read(cx).value(),
            "Groceries",
            "the typed name survives the refusal for the retry"
        );
        assert_eq!(
            page.selected_category,
            Some(food),
            "the parent stays selected"
        );
    });
    assert!(
        paints(cx, handle, "cat:form-add-sub"),
        "the form stays open for the retry"
    );
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
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.delete_target
                    .update(cx, |sel, cx| sel.set_selected_value(&keep, window, cx))
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
/// 3.84: every button that starts a category write renders loading,
/// disabled, while the DB task runs (the Rules page's 3.40 pattern). The
/// test executor only advances inside `wait_until`, so reading
/// `cat_pending` right after the handler call observes the write in
/// flight; each step then waits for the write to land and the flag to
/// clear. All five writes: add parent, add child, rename, move, delete.
#[gpui::test]
fn every_category_write_shows_loading_until_the_task_ends(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);

    // -- Add category (a success selects the new parent).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.new_parent_name
                    .update(cx, |i, cx| i.set_value("Food", window, cx));
            });
            s.add_parent(window, cx);
            assert_eq!(
                s.categories_view.read(cx).cat_pending,
                Some(PendingCatWrite::AddParent),
                "Add category shows loading, disabled, while the write runs"
            );
        });
    });
    wait_until(cx, "Food lands, loading cleared", |cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .cat_pending
            .is_none()
            && shell.read(cx).categories.iter().any(|p| p.name == "Food")
    });

    // -- Add subcategory (the add above left "Food" selected).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.new_child_name
                    .update(cx, |i, cx| i.set_value("Restaurants", window, cx));
            });
            s.add_child(window, cx);
            assert_eq!(
                s.categories_view.read(cx).cat_pending,
                Some(PendingCatWrite::AddChild),
                "Add subcategory shows loading, disabled, while the write runs"
            );
        });
    });
    wait_until(cx, "Restaurants lands", |cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .cat_pending
            .is_none()
            && shell
                .read(cx)
                .categories
                .iter()
                .any(|p| p.name == "Food" && p.children.iter().any(|c| c.name == "Restaurants"))
    });

    // -- Rename. 3.96: the add kept "Food" selected, so select the new
    // child explicitly before renaming it.
    let restaurants = cx.update(|cx| find_child(&shell, cx, "Restaurants").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(restaurants, window, cx));
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view.update(cx, |p, cx| {
                p.renaming = true;
                p.rename_input
                    .update(cx, |i, cx| i.set_value("Dining", window, cx));
            });
            s.categories_view
                .update(cx, |p, cx| p.save_rename(window, cx));
            assert_eq!(
                s.categories_view.read(cx).cat_pending,
                Some(PendingCatWrite::Rename),
                "Save shows loading, disabled, while the write runs"
            );
        });
    });
    wait_until(cx, "the rename lands", |cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .cat_pending
            .is_none()
            && shell
                .read(cx)
                .categories
                .iter()
                .any(|p| p.name == "Food" && p.children.iter().any(|c| c.name == "Dining"))
    });

    // -- Move: "Dining" from Food under a second parent.
    add_parent(cx, handle, &shell, "Home");
    let dining = cx.update(|cx| find_child(&shell, cx, "Dining").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(dining, window, cx));
    });
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
            s.move_child_to_selected_parent(window, cx);
            assert_eq!(
                s.categories_view.read(cx).cat_pending,
                Some(PendingCatWrite::Move),
                "Move shows loading, disabled, while the write runs"
            );
        });
    });
    wait_until(cx, "Dining moves under Home", |cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .cat_pending
            .is_none()
            && shell
                .read(cx)
                .categories
                .iter()
                .any(|p| p.name == "Home" && p.children.iter().any(|c| c.name == "Dining"))
    });

    // -- Delete: a second child inherits the transactions; the first
    // Delete click only arms, so it must not set the pending flag, and
    // the second runs the write.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let home = s.categories.iter().find(|p| p.name == "Home").unwrap().id;
            s.select_category(home, window, cx);
        });
    });
    add_child(cx, handle, &shell, "Cafes");
    let cafes = cx.update(|cx| find_child(&shell, cx, "Cafes").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(dining, window, cx));
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            // 3.54: the first click arms and, through the delete action,
            // re-sets the pickers against the current list — the pick of
            // a reassign target then resolves against fresh items.
            s.arm_delete_category(window, cx);
            s.categories_view.update(cx, |p, cx| {
                p.delete_target
                    .update(cx, |sel, cx| sel.set_selected_value(&cafes, window, cx))
            });
            assert_eq!(
                s.categories_view.read(cx).cat_pending,
                None,
                "the first Delete click only asks, it writes nothing"
            );
            s.arm_delete_category(window, cx);
            assert_eq!(
                s.categories_view.read(cx).cat_pending,
                Some(PendingCatWrite::Delete),
                "Delete shows loading, disabled, while the write runs"
            );
        });
    });
    wait_until(cx, "Dining is deleted", |cx| {
        shell
            .read(cx)
            .categories_view
            .read(cx)
            .cat_pending
            .is_none()
            && shell
                .read(cx)
                .categories
                .iter()
                .all(|p| !p.children.iter().any(|c| c.name == "Dining"))
    });
}
/// 3.84: when the DB refuses the write, the loading state clears on the
/// error path too, with the refusal shown and the button clickable again.
#[gpui::test]
fn a_refused_category_write_clears_the_loading_state(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    add_parent(cx, handle, &shell, "Food");
    add_child(cx, handle, &shell, "Restaurants");
    let food = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food")
            .unwrap()
            .id
    });
    // The database's own refusal: a duplicate subcategory name.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(food, window, cx);
            s.categories_view.update(cx, |p, cx| {
                p.new_child_name
                    .update(cx, |i, cx| i.set_value("Restaurants", window, cx));
            });
            s.add_child(window, cx);
            assert_eq!(
                s.categories_view.read(cx).cat_pending,
                Some(PendingCatWrite::AddChild),
                "the add is in flight until the executor runs"
            );
        });
    });
    wait_until(cx, "the refusal clears the loading state", |cx| {
        let p = shell.read(cx).categories_view.read(cx);
        p.cat_pending.is_none() && p.cat_error.is_some()
    });
    cx.update(|cx| {
        let p = shell.read(cx).categories_view.read(cx);
        assert_eq!(p.cat_pending, None, "loading cleared on the error path too");
        assert!(p.cat_error.is_some(), "the refusal is shown");
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
