//! The File menu definition, the ⌘O import shortcut and per-window
//! action wiring (3.7, 3.76).

use super::*;

/// The File menu the native menu bar is built from (3.7). The macOS menu
/// itself can't be observed headlessly; this checks the menu definition the
/// platform is handed.
#[gpui::test]
fn the_file_menu_offers_all_five_actions(cx: &mut TestAppContext) {
    open_shell(cx, |_| {});
    let menus = cx.update(|cx| cx.get_menus()).expect("menus set");
    assert_eq!(menus.len(), 1, "one top-level menu");
    let file = &menus[0];
    assert_eq!(file.name.as_ref(), "File");
    let items: Vec<String> = file
        .items
        .iter()
        .map(|item| match item {
            OwnedMenuItem::Separator => "—".into(),
            OwnedMenuItem::Action {
                name,
                action,
                disabled,
                ..
            } => {
                assert!(!disabled, "{name} must not be disabled");
                format!("{name} [{}]", action.name())
            }
            OwnedMenuItem::Submenu(_) | OwnedMenuItem::SystemMenu(_) => "?".into(),
        })
        .collect();
    assert_eq!(
        items,
        [
            "Import CSV… [financi::ImportCsv]",
            "Undo last import [financi::UndoLastImport]",
            "—",
            "Export… [financi::ExportData]",
            "Restore… [financi::RestoreBackup]",
            "—",
            "Quit [financi::QuitApp]",
        ]
    );
}

/// ⌘O (the menu's Import CSV… shortcut) opens the file picker, and a chosen
/// file lands in the import preview on the Imports page.
#[gpui::test]
fn cmd_o_opens_the_file_picker_and_the_preview(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    // Actions dispatch along the focus path, so focus the search input
    // first — like a user who clicked into the window.
    with_window(cx, handle, |window, cx| {
        let input = dash(cx, &shell)
            .read(cx)
            .search_input
            .read(cx)
            .focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "cmd-o");
    assert!(cx.did_prompt_for_paths(), "⌘O opened the file picker");

    let file = demo("simplii-chequing.csv");
    cx.simulate_path_prompt_response(move |_| Some(vec![file]));
    wait_until(cx, "preview open on the Imports page", |cx| {
        shell.read(cx).imports.read(cx).import.is_some() && shell.read(cx).page == Page::Imports
    });
}

/// 3.76: the global wiring is installed once per app, not once per
/// window. With two shells open, ⌘O opens exactly one dialog and drives
/// the *active* window's shell — the other shell never sees the action.
/// (Per-shell wiring let both shells answer every ⌘ action, and kept both
/// shells alive forever.)
#[gpui::test]
fn cmd_o_fires_once_and_drives_the_active_shell(cx: &mut TestAppContext) {
    let (handle_a, shell_a) = open_shell(cx, |_| {});
    let (handle_b, shell_b) = open_shell(cx, |_| {});
    wait_idle(cx, &shell_a);
    wait_idle(cx, &shell_b);
    // Like a click: window B becomes the platform's active window.
    with_window(cx, handle_b, |window, _cx| window.activate_window());
    cx.run_until_parked();

    // ⌘O dispatched on window A — the active window still wins, and only
    // one dialog opens.
    with_window(cx, handle_a, |window, cx| {
        window.dispatch_action(Box::new(ImportCsv), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "⌘O opened a file picker");
    let file = demo("simplii-chequing.csv");
    cx.simulate_path_prompt_response(move |_| Some(vec![file]));
    assert!(
        !cx.did_prompt_for_paths(),
        "⌘O opened exactly one file picker"
    );

    wait_until(cx, "the active shell shows the preview", |cx| {
        shell_b.read(cx).imports.read(cx).import.is_some()
    });
    // The inactive shell never acted: once B's work has settled, it has
    // no panel of its own.
    wait_idle(cx, &shell_a);
    cx.update(|cx| {
        assert!(
            shell_a.read(cx).imports.read(cx).import.is_none(),
            "the inactive shell did not act on ⌘O"
        );
    });
}

/// 3.76: nothing outlives a window. The wiring holds no strong handle to
/// the `Shell`, so removing the window drops the shell with it — a weak
/// handle to it stops upgrading.
#[gpui::test]
fn closing_the_window_drops_the_shell(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    let weak = shell.downgrade();
    drop(shell);
    with_window(cx, handle, |window, _cx| window.remove_window());
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "the Shell outlived its window");
}
