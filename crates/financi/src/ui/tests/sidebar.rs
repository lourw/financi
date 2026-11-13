//! Tests for `ui/sidebar.rs` (3.62 split from `ui_tests.rs`).

use super::*;
/// 3.13: sidebar nav items must not sit right of the section labels. The
/// gpui-kit nav row carries its own 8px left padding and the section label
/// had its own `px_2`, so both floated right of the single column the header
/// and footer form (`docs/ui-conventions.md`, *Sidebar*); the nav item and
/// the section label now render with one shared inset, pinned flush here.
/// Element padding lives inside the component's own row and is invisible to
/// the headless test surface, so this pins the shared constant both render
/// with rather than the rendered pixels.
#[test]
fn sidebar_nav_items_and_section_labels_share_one_flush_inset() {
    assert_eq!(
        NAV_ROW_INSET, 0.,
        "nav items and section labels must share one inset, flush with the \
         header and footer (`docs/ui-conventions.md`, *Sidebar*)"
    );
}

/// 3.38: the sidebar footer's database path ellipsizes at the sidebar's
/// width, and hovering it shows the full path prefixed "Database: " in a
/// tooltip (`docs/ui-conventions.md`, *Sidebar*).
#[gpui::test]
fn sidebar_db_path_shows_the_full_path_on_hover(cx: &mut TestAppContext) {
    // A path well over 100 characters: even at a conservative 4.5 px per
    // `text_xs` glyph it needs far more than the 210 px sidebar, so the
    // footer must ellipsize it.
    let db_path = std::env::temp_dir().join(format!(
        "financi-ui-db-{}/databases/a-scratch-directory-with-a-very-long-name-for-ellipsis/financi.sqlite",
        std::process::id()
    ));
    let (handle, _shell) = open_shell_at(cx, |_| {}, db_path.clone());
    let path = db_path.to_string_lossy().to_string();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    let cell = bounds(cx, handle, format!("sidebar:db-path:{path}"));
    assert!(
        cell.size.width < px(path.chars().count() as f32 * 4.5),
        "the path truncates: the cell is {:?} wide, the full text needs at \
         least {:?}",
        cell.size.width,
        px(path.chars().count() as f32 * 4.5),
    );

    // The tooltip carries the full path, prefixed "Database: "; it only
    // exists while the path is hovered.
    let tip = format!("sidebar:db-path-tooltip:Database: {path}");
    assert!(
        !paints(cx, handle, tip.clone()),
        "nothing hovers the path, so no tooltip is drawn yet"
    );
    window.simulate_mouse_move(
        point(
            cell.origin.x + cell.size.width / 2.,
            cell.origin.y + cell.size.height / 2.,
        ),
        None,
        Modifiers::default(),
    );
    // The tooltip appears after a 500 ms hover; the test clock is fake.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, tip.clone()),
        "hovering the path shows a tooltip with the full path",
    );
}

/// 3.39: the sidebar footer's last line is the build version — it paints
/// the text `version::VERSION` carries, `v`-prefixed once — with the same
/// kind of hover tooltip as the path (3.38).
#[gpui::test]
fn sidebar_footer_shows_the_app_version(cx: &mut TestAppContext) {
    let (handle, _shell) = open_shell(cx, |_| {});
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // The selector embeds the label, so a match proves the footer paints
    // exactly the const's text.
    let label = crate::version::version_label();
    let text_sel = format!("sidebar:version-text:{label}");
    let cell = bounds(cx, handle, text_sel.clone());
    assert!(
        label.starts_with('v'),
        "the version label is \"v\"-prefixed: {label:?}"
    );
    assert_eq!(
        label.trim_start_matches('v'),
        crate::version::VERSION.trim_start_matches('v'),
        "the shown label is the const, with one \"v\" prefix"
    );

    // Hovering shows the full version, as with the path (3.38).
    let tip = format!("sidebar:version-tooltip:{label}");
    assert!(
        !paints(cx, handle, tip.clone()),
        "nothing hovers the version, so no tooltip is drawn yet"
    );
    window.simulate_mouse_move(
        point(
            cell.origin.x + cell.size.width / 2.,
            cell.origin.y + cell.size.height / 2.,
        ),
        None,
        Modifiers::default(),
    );
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, tip.clone()),
        "hovering the version shows a tooltip with the full version",
    );
}
