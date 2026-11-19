//! Showing and hiding the page: rows and pickers when data arrives
//! late or the page is re-entered.

use super::*;

/// 3.54: a hidden page keeps its pickers as they are across a refresh —
/// the refresh only flags them — and re-sets them on its next draw. The
/// `cat_picker_sets` hook counts `set_items` calls, which need a window
/// and so run in `render` and the picking actions only.
#[gpui::test]
fn a_hidden_page_re_sets_its_pickers_on_its_next_draw(cx: &mut TestAppContext) {
    let (handle, shell, _rule, _first, _second) = open_rule_fixture(cx);
    let base = cx.update(|cx| shell.read(cx).rules_page.read(cx).cat_picker_sets.get());
    // A write while the Rules page is hidden refreshes the shared
    // snapshot; its `apply` only flags the page.
    add_parent(cx, handle, &shell, "Home");
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).rules_page.read(cx).cat_picker_sets.get(),
            base,
            "a hidden page re-sets nothing during a refresh"
        );
    });
    // Its next draw picks the change up — both pickers at once (+2).
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).rules_page.read(cx).cat_picker_sets.get(),
            base + 2
        );
    });
}
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
