//! Tests for `ui/categories/` (3.62 split from `ui_tests.rs`).
//!
//! S1 split this file by feature the way `tests/dashboard/` is split: one
//! sub-file per feature, with the click and seed helpers two or more of
//! them share living here. New tests go next to related ones in the right
//! sub-file.

use financi_db::rules::RuleMatchStatus;
use gpui_kit::component::input::InputEvent;

use super::*;

use super::dashboard::{click_chart_legend, dash, seed_chart_rows, top_category_id};

mod forms;
mod labels;
mod layout;
mod lifecycle;
mod long_names;
mod new_category;
mod rule_preview;
mod rules_list;
mod tree;
/// 3.59: pick `child` in the Delete form's reassign picker. The grouped
/// items carry the sub-category id as their value, so the pick is by
/// value (a sectioned menu's `IndexPath` is not a flat row number).
pub(super) fn s_select(shell: &Entity<Shell>, cx: &mut App, child: i64, window: &mut Window) {
    let target = shell
        .read(cx)
        .categories_view
        .read(cx)
        .delete_target
        .clone();
    target.update(cx, |sel, cx| sel.set_selected_value(&child, window, cx));
}
/// 3.41: a child category whose full label "Drinks › …" is exactly 40
/// characters — too wide for the 200 px trigger (it ellipsizes), but it
/// fits the 320 px open menu.
/// 3.41/3.53: a child category named exactly 40 characters — too wide
/// for the 200 px trigger (it ellipsizes, and the hover tooltip names
/// the selection), but it fits the 320 px open menu.
pub(super) const LONG_CHILD: &str = "international-subscriptions-limited-2026";
pub(super) fn long_label() -> String {
    format!("Drinks › {LONG_CHILD}")
}
/// 3.41: like `seed_rule_fixture`, but the rule targets the long-named
/// child, so the Categories picker's menu and the trigger's tooltip have
/// a 40-character name to show. Returns `(rule, long child, short child)`.
pub(super) fn seed_long_name_rule_fixture(
    conn: &financi_db::rusqlite::Connection,
) -> (i64, i64, i64) {
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
pub(super) fn open_long_name_rule_fixture(
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
pub(super) fn open_picker_menu(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    sel: impl Into<String>,
) {
    click(cx, handle, sel);
    draw_frame(cx, handle);
    draw_frame(cx, handle);
}
/// 3.53: click outside an open menu to dismiss it (the popup closes on
/// any press that misses it).
pub(super) fn click_past_anything(cx: &mut TestAppContext, window: &mut VisualTestContext) {
    window.simulate_click(point(px(2.), px(2.)), Modifiers::default());
    cx.run_until_parked();
}
/// 3.53: assert a menu row paints with its text unclipped — the natural
/// (nowrap, absolutely positioned) probe copy fits inside the drawn row
/// — and that the row sits inside the window.
pub(super) fn assert_menu_item_unclipped(
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
