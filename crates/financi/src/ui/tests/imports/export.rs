//! Export…: the save dialog's default, the zip it writes, cancelling,
//! and the ⌘E binding (5.3).

use super::*;

/// The save dialog's default (5.3): today's zip name, in the Desktop
/// directory (or home / `.` where the platform has no Desktop).
#[test]
fn export_defaults_to_todays_zip_on_the_desktop() {
    let (dir, name) = crate::ui::imports::export_default_path();
    assert_eq!(
        name,
        format!(
            "financi-export-{}.zip",
            chrono::Local::now().format("%Y-%m-%d")
        )
    );
    assert!(
        dir.is_absolute() || dir == std::path::Path::new("."),
        "a real directory: {}",
        dir.display()
    );
}

/// The Imports page header's **Export** button opens the save dialog; a
/// chosen path gets the zip and a toast names it (5.3).
#[gpui::test]
fn export_writes_the_chosen_zip_and_toasts(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_on_file(cx, |_| {});
    wait_idle(cx, &shell);
    import_demo_file(cx, handle, &shell, "amex-sep-oct.csv", "Amex");
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "67 rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 67
    });
    let zip = scratch_file("snapshot.zip");

    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    click(cx, handle, "imports:header:Export:icons/download.svg");
    assert!(
        cx.did_prompt_for_new_path(),
        "the Export button opened the save dialog"
    );
    let dest = zip.clone();
    cx.simulate_new_path_selection(move |_| Some(dest));
    wait_until(cx, "exported toast", |cx| {
        !shell.read(cx).imports.read(cx).exporting
            && last_toast(&shell, cx) == Some(("Exported to snapshot.zip".into(), false))
    });
    let written = zip.metadata().expect("the zip was written");
    assert!(written.len() > 0, "and it is not empty");
    wait_idle(cx, &shell);
}

/// Cancelling the save dialog writes nothing and says nothing; the
/// button is live again (5.3).
#[gpui::test]
fn export_cancel_writes_nothing_and_says_nothing(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    assert!(
        cx.did_prompt_for_new_path(),
        "Export opened the save dialog"
    );
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert!(!shell.read(cx).imports.read(cx).exporting, "not stuck busy");
        assert!(
            toasts(&shell, cx).is_empty(),
            "a cancelled Export is silent"
        );
    });
}

/// The ⌘E binding opens the same save dialog the button does (5.3).
#[gpui::test]
fn cmd_e_opens_the_export_save_dialog(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    with_window(cx, handle, |window, cx| {
        let input = dash(cx, &shell)
            .read(cx)
            .search_input
            .read(cx)
            .focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "cmd-e");
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "⌘E opened the save dialog");
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert!(!shell.read(cx).imports.read(cx).exporting));
}
