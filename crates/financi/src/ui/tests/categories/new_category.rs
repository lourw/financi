//! The new-category form: open and close, its picker label, and the
//! one-layer-per-press Escape behaviour.

use super::*;

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
/// 3.60: the new-category form's kind picker is captioned "Category type",
/// and no control is captioned "Kind" any more (the detail panel's muted
/// value line, "Expense · N transactions", is unaffected). The longer
/// caption must still fit the form's row at the 1200 × 720 minimum window
/// the shell opens at: the form ends inside the page, as in
/// [`the_categories_page_fits_the_minimum_window`].
#[gpui::test]
fn the_new_category_form_labels_its_picker_category_type(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.categories_view
                .update(cx, |p, _| p.new_category_open = true)
        })
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "label:Category type"),
        "the new-category form paints the \"Category type\" label"
    );
    assert!(
        !paints(cx, handle, "label:Kind"),
        "no control on the Categories page is captioned \"Kind\" any more"
    );
    let page_right = px(1200. - PAGE_PAD);
    let b = bounds(cx, handle, "cat:new-form");
    assert!(
        b.origin.x + b.size.width <= page_right + px(1.),
        "with the longer caption the new-category form ends at x {:?}, \
         past the page edge {page_right:?}",
        b.origin.x + b.size.width,
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
