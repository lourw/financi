//! Long names: the subcategory menu shows a forty-character name
//! without clipping.

use super::*;

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
