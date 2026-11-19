//! Geometry: panel heights, title-row sizing, the minimum window,
//! column headings and icon buttons.

use super::*;

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
/// 3.104: every title-row action is `small`, so all buttons in the row
/// paint at the same height — Rename used to sit at the default size and
/// towered over Add subcategory, Move and Delete. Checked on both shapes
/// of the row: a category (Rename, Add subcategory, Delete) and a
/// subcategory (Rename, Move, Delete). The group also reads in the order
/// ui-conventions.md lists it: Rename first, Delete last.
#[gpui::test]
fn every_title_row_button_is_the_same_size(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let parent =
            financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
        financi_db::categories::create_child(conn, parent, "Coffee").unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell.read(cx).categories.len() == 2
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");
    resize_and_draw(cx, handle, 1200., 720.);

    let rename_sel = "cat:title-rename:icons/pencil.svg";

    // The category row: Rename, Add subcategory, Delete.
    let mut sels = [rename_sel, "cat:title-add-sub", "cat:title-delete"];
    let mut last_height = f32::NAN;
    let mut last_x = f32::NAN;
    for (n, sel) in sels.iter().enumerate() {
        let b = bounds(cx, handle, *sel);
        let (h, x) = (f32::from(b.size.height), f32::from(b.origin.x));
        if n == 0 {
            last_height = h;
        } else {
            assert_eq!(
                h, last_height,
                "{sel} is the same height as the row's other actions"
            );
            assert!(
                x > last_x,
                "the actions read Rename, Add subcategory, Delete: \
                 {sel} at x {x} after x {last_x}"
            );
        }
        last_x = x;
    }

    // And the subcategory row: Rename, Move, Delete.
    let child = cx.update(|cx| find_child(&shell, cx, "Coffee").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(child, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    sels[1] = "cat:title-move";
    last_height = f32::NAN;
    last_x = f32::NAN;
    for (n, sel) in sels.iter().enumerate() {
        let b = bounds(cx, handle, *sel);
        let (h, x) = (f32::from(b.size.height), f32::from(b.origin.x));
        if n == 0 {
            last_height = h;
        } else {
            assert_eq!(
                h, last_height,
                "{sel} is the same height as the row's other actions on a subcategory"
            );
            assert!(
                x > last_x,
                "the actions read Rename, Move, Delete: {sel} at x {x} \
                 after x {last_x}"
            );
        }
        last_x = x;
    }
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
    // the row's right edge is the tree panel's content edge — 3.91: the
    // shared list runs edge to edge, so only the panel's 1 px border
    // stands between the row and the panel's outer edge.
    let tree = window
        .debug_bounds("cat:tree-panel")
        .expect("the tree panel");
    let row = bounds(cx, handle, format!("cat:tree-row:{child}"));
    let tree_content_right = tree.origin.x + tree.size.width - px(1.);
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
/// 3.42: in every rule row — the add-rule row on each page and the rule
/// editor — the regex switch sits on the controls' centre line: the
/// switch itself (its shrink-wrap probe, not the 32 px slot around it)
/// centres within 1 px of the Pattern input, the primary button and the
/// slot. 3.48: the editor's controls moved to their own label-less line
/// (no slot, no input beside them), so there the switch centres on Save
/// and Cancel instead. 3.103: so did the Categories add-rule form's — its
/// switch centres on the picker and Add rule, with the input above.
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

    // 3.103: the Categories add-rule form is the 3.48 two-line shape: the
    // switch, the subcategory picker and the primary button share line 2's
    // centre (`items_center`, no slot), and the Pattern input sits above
    // them at the form's full width.
    let sw = centre(&mut window, "cat:switch:add", "add-rule");
    for (what, sel) in [
        (
            "the subcategory picker",
            "cat:rule-subcategory-select:cat-add",
        ),
        ("the primary button", "cat:add-rule-btn:add"),
    ] {
        let c = centre(&mut window, sel, "add-rule");
        assert!(
            (c - sw).abs() <= 1.,
            "in the add-rule form {what} centres at y {c}, the regex switch at {sw}",
        );
    }
    let input = window
        .debug_bounds("cat:pattern-input:add")
        .expect("the add-rule form paints the Pattern input");
    assert!(
        f32::from(input.bottom()) <= sw,
        "the Pattern input (bottom {:?}) sits above the controls line (centre {sw})",
        input.bottom(),
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
    // edge is the row's right edge (the Delete button ends there).
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
/// 3.43: the Categories page's buttons carry their Lucide icon left of the
/// label and no label ends in an ellipsis: New category (plus), Rename
/// (pencil), the delete form's confirm (trash) and the rule row's Edit
/// (pencil) and Delete (trash). Each button's selector names the icon
/// asset it draws; the old "…" labels draw nothing, and the drawn widths
/// show the icons actually paint.
#[gpui::test]
fn the_category_buttons_paint_icons_instead_of_ellipses(cx: &mut TestAppContext) {
    let (handle, _shell, rule, _first, _second) = open_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    for old in ["label:New category…", "label:Rename…", "label:Delete…"] {
        assert!(!paints(cx, handle, old), "{old} no longer paints");
    }
    let new_btn = bounds(cx, handle, format!("label:{NEW_CATEGORY}"));
    let rename = bounds(cx, handle, "cat:title-rename:icons/pencil.svg");
    let edit = bounds(cx, handle, format!("cat:rule-edit:{rule}:icons/pencil.svg"));
    let delete = bounds(
        cx,
        handle,
        format!("cat:rule-delete:{rule}:icons/trash.svg"),
    );
    // The delete form's confirm button: open the form, then measure it
    // idle — armed it reads "Click again to delete" (unchanged).
    click(cx, handle, "cat:title-delete");
    draw_frame(cx, handle);
    let del_go = bounds(cx, handle, "cat:form-delete-go");

    let mut refs = open_icon_text_refs(
        cx,
        vec![
            ("ref:cat-new", NEW_CATEGORY),
            ("ref:cat-rename", "Rename"),
            ("ref:cat-delete", "Delete"),
            ("ref:cat-edit", "Edit"),
        ],
    );
    // 3.104: Rename is `small` like the title row's other actions, so it
    // is measured with the small icon block and padding; the rest are
    // still medium.
    for (what, btn, text_sel, icon_w, pad_x) in [
        (
            "the New category button",
            new_btn,
            "ref:cat-new",
            MEDIUM_ICON_BLOCK_W,
            MEDIUM_BUTTON_PAD_X,
        ),
        (
            "the Rename button",
            rename,
            "ref:cat-rename",
            SMALL_ICON_BLOCK_W,
            SMALL_BUTTON_PAD_X,
        ),
        (
            "the delete form's confirm",
            del_go,
            "ref:cat-delete",
            MEDIUM_ICON_BLOCK_W,
            MEDIUM_BUTTON_PAD_X,
        ),
        (
            "the rule row's Edit",
            edit,
            "ref:cat-edit",
            MEDIUM_ICON_BLOCK_W,
            MEDIUM_BUTTON_PAD_X,
        ),
        (
            "the rule row's Delete",
            delete,
            "ref:cat-delete",
            MEDIUM_ICON_BLOCK_W,
            MEDIUM_BUTTON_PAD_X,
        ),
    ] {
        assert_button_paints_icon(&mut refs, what, btn, text_sel, icon_w, pad_x);
    }

    // 3.43: 3.23's fit rule with the icons in (≈ 24 px per button): at
    // the minimum window and at 1600 × 900 the row's Edit and Delete
    // still paint fully inside the detail panel.
    for (w, h) in [(1200., 720.), (1600., 900.)] {
        resize_and_draw(cx, handle, w, h);
        let panel = bounds(cx, handle, "cat:detail-panel");
        let edit = bounds(cx, handle, format!("cat:rule-edit:{rule}:icons/pencil.svg"));
        let delete = bounds(
            cx,
            handle,
            format!("cat:rule-delete:{rule}:icons/trash.svg"),
        );
        for (what, b) in [("Edit", edit), ("Delete", delete)] {
            assert!(
                b.origin.x >= panel.origin.x
                    && b.origin.x + b.size.width <= panel.origin.x + panel.size.width,
                "{what} paints inside the detail panel at {w} × {h}: button \
                 right {:?}, panel right {:?}",
                b.origin.x + b.size.width,
                panel.origin.x + panel.size.width,
            );
        }
    }
}
