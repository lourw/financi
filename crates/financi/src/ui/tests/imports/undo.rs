//! Undoing the newest import: the in-row two-click button and the
//! `UndoLastImport` action.

use super::*;

#[gpui::test]
fn undo_needs_a_second_click_and_removes_the_latest_import(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts loaded", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    wait_until(cx, "import", |cx| shell.read(cx).dashboard.row_count == 67);

    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.read(cx).import_history.len() == 1
    });

    with_window(cx, handle, |window, cx| {
        let id = shell.read(cx).imports.read(cx).import_history[0].id;
        shell.update(cx, |s, cx| {
            s.imports.update(cx, |p, cx| p.undo_import(id, window, cx))
        });
        assert_eq!(
            shell.read(cx).imports.read(cx).confirm_undo_import,
            Some(id),
            "first click only asks to confirm"
        );
        assert_eq!(shell.read(cx).dashboard.row_count, 67);
        shell.update(cx, |s, cx| {
            s.imports.update(cx, |p, cx| p.undo_import(id, window, cx))
        });
    });

    wait_until(cx, "undo", |cx| {
        shell.read(cx).dashboard.row_count == 0
            && shell.read(cx).imports.read(cx).import_history.is_empty()
    });
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some((
                "Undid import of amex-sep-oct.csv: removed 68 rows".into(),
                false
            ))
        );
    });
}

/// The `UndoLastImport` action (File menu item and Imports header button)
/// runs the newest row's undo path: arm, then undo.
#[gpui::test]
fn the_undo_action_arms_then_undoes_the_newest_import(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts loaded", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.read(cx).import_history.len() == 1
            && shell.read(cx).dashboard.row_count == 67
    });
    let id = cx.update(|cx| shell.read(cx).imports.read(cx).import_history[0].id);

    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).imports.read(cx).confirm_undo_import,
            Some(id),
            "first activation only asks to confirm"
        );
    });
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    wait_until(cx, "undo", |cx| {
        shell.read(cx).dashboard.row_count == 0
            && shell.read(cx).imports.read(cx).import_history.is_empty()
    });
}

/// With no imports there is nothing to undo: the action is a no-op.
#[gpui::test]
fn the_undo_action_is_a_no_op_without_history(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert_eq!(shell.read(cx).imports.read(cx).confirm_undo_import, None);
        assert!(toasts(&shell, cx).is_empty());
    });
}
