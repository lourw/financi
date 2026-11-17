//! One write at a time: while an export or restore runs, imports,
//! queue applies and the other archive operation are refused (3.64, 5.3).

use super::*;

/// One archive operation at a time (3.64): while Restore… has its dialog
/// open, Export… is refused — and works again once the restore ends.
#[gpui::test]
fn export_and_restore_refuse_each_other_while_one_runs(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RestoreBackup), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "Restore opened the file dialog");
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    assert!(
        !cx.did_prompt_for_new_path(),
        "no save dialog starts while a restore runs"
    );
    // Cancel the restore; Export answers now.
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| assert!(!shell.read(cx).imports.read(cx).restoring));
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "Export works once idle");
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!shell.read(cx).imports.read(cx).exporting);
        assert!(
            toasts(&shell, cx).is_empty(),
            "cancelled dialogs say nothing"
        );
    });
}

// -- Writes wait for the archive (5.3 review fix) ------------------------------

/// 5.3 review fix: while a Restore… job runs every other write dispatch
/// is refused — an import queued behind the restore would commit its
/// pre-restore account id into the replaced database, and its panel is
/// destroyed by the restore's reset anyway. The Import button is
/// disabled, even a direct call does nothing, and once the restore
/// lands the data is the snapshot's exactly.
#[gpui::test]
fn restore_in_flight_refuses_an_import(cx: &mut TestAppContext) {
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
    export_to(cx, handle, &shell, &zip);

    // A second file's panel, planned and ready to commit.
    open_and_choose(cx, handle, &shell, "simplii-chequing.csv", NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        let p = panel(&shell, cx);
        p.update(cx, |p, cx| {
            p.new_name.update(cx, |input, cx| {
                input.set_value("Simplii", window, cx);
            });
            p.create_account(window, cx);
        });
    });
    wait_until(cx, "plan ready", |cx| plan_ready(&shell, cx));
    wait_idle(cx, &shell);

    // Hold the writer, start a restore, answer its dialog: the job
    // waits on the blocker, so the restore is in flight.
    let block = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RestoreBackup), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "Restore opened the file dialog");
    cx.simulate_path_prompt_response(move |_| Some(vec![zip]));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).restoring,
            "the restore is in flight"
        );
        let p = panel(&shell, cx);
        assert!(
            !p.read(cx).can_import(),
            "Import is refused while a restore runs"
        );
    });
    // The button is disabled, but even the direct call must be refused.
    with_window(cx, handle, |_, cx| {
        let p = panel(&shell, cx);
        p.update(cx, |p, cx| p.import(cx));
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "imports:import-state:ready"),
        "the refused import leaves the panel idle"
    );
    assert!(
        !paints(cx, handle, "imports:import-state:loading"),
        "no commit started"
    );

    // Release the writer: the restore lands and the import never happened.
    drop(block);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "the restored rows land", |cx| {
        !shell.read(cx).imports.read(cx).restoring
            && dash(cx, &shell).read(cx).row_count == 67
            && shell.read(cx).accounts.len() == 1
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).import.is_none(),
            "the restore reset closed the panel"
        );
        assert_eq!(
            shell.read(cx).imports.read(cx).import_history.len(),
            1,
            "only the snapshot's import is in the history"
        );
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Restored from snapshot.zip".into(), false))
        );
        assert!(
            !toasts(&shell, cx)
                .iter()
                .any(|(msg, _)| msg.to_lowercase().contains("simplii")),
            "the refused import never ran (no Imported toast for its file)"
        );
    });
}

/// 5.3 review fix: a queue apply holds transaction ids of the database
/// being replaced — dense ids name different rows after the restore —
/// so `categorize_merchant` is refused while a restore runs too: no rule
/// is created and the restored queue still holds the group whole.
#[gpui::test]
fn restore_in_flight_refuses_a_queue_apply(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_on_file(cx, |_| {});
    wait_idle(cx, &shell);
    import_demo_file(cx, handle, &shell, "amex-sep-oct.csv", "Amex");
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "67 rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 67
    });
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    wait_until(cx, "queue populated", |cx| {
        // 3.83: the queue page defers applying while hidden; the snapshot
        // is what's always current.
        !shell.read(cx).catalog.read(cx).snap.queue.is_empty()
    });
    let (merchant, before) = cx.update(|cx| top_group(&shell, cx));
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let zip = scratch_file("queue-snapshot.zip");
    export_to(cx, handle, &shell, &zip);
    wait_idle(cx, &shell);

    let block = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RestoreBackup), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "Restore opened the file dialog");
    cx.simulate_path_prompt_response(move |_| Some(vec![zip]));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).restoring,
            "the restore is in flight"
        )
    });

    // The very call the row's Apply button makes, picker and all —
    // refused mid-restore. 3.83: the page applies its snapshot on show
    // (no DB round-trip, so the blocked worker doesn't matter), which is
    // what gives the picker its choices.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    let refused = merchant.clone();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let picker = s
                .queue_view
                .update(cx, |p, cx| p.queue_picker(&refused, window, cx));
            // 3.59: the grouped picker items carry the sub-category id
            // as their value, so the pick resolves by value (a sectioned
            // menu's `IndexPath` is not a flat row number).
            picker.update(cx, |sel, cx| {
                sel.set_selected_value(&child, window, cx);
            });
            s.queue_view.update(cx, |p, cx| {
                p.categorize_merchant(refused.clone(), window, cx)
            });
        });
    });

    drop(block);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "the restored rows land", |cx| {
        !shell.read(cx).imports.read(cx).restoring && dash(cx, &shell).read(cx).row_count == 67
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert!(
            !shell.read(cx).rules.iter().any(|r| r.pattern == merchant),
            "the refused categorize created no rule"
        );
        let (top, count) = top_group(&shell, cx);
        assert_eq!(top, merchant, "the same group tops the restored queue");
        assert_eq!(count, before, "and it still holds every row");
    });
}

/// 5.3 review fix: the export flag covers the open save dialog too, so
/// an import planned while the dialog waits is refused for its duration
/// — and works the moment the dialog is cancelled.
#[gpui::test]
fn a_cancelled_export_dialog_leaves_import_working(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    open_and_choose(cx, handle, &shell, "amex-sep-oct.csv", NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        let p = panel(&shell, cx);
        p.update(cx, |p, cx| {
            p.new_name.update(cx, |input, cx| {
                input.set_value("Cobalt", window, cx);
            });
            p.create_account(window, cx);
        });
    });
    wait_until(cx, "plan ready", |cx| plan_ready(&shell, cx));

    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    assert!(
        cx.did_prompt_for_new_path(),
        "Export opened the save dialog"
    );
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).exporting,
            "the dialog is in flight"
        );
        let p = panel(&shell, cx);
        assert!(!p.read(cx).can_import(), "Import waits for the dialog");
    });
    with_window(cx, handle, |_, cx| {
        let p = panel(&shell, cx);
        p.update(cx, |p, cx| p.import(cx));
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "imports:import-state:ready"),
        "the refused import leaves the panel idle"
    );
    assert!(
        !paints(cx, handle, "imports:import-state:loading"),
        "the refused import started no commit"
    );

    // Cancel the dialog: the panel is live again and imports for real.
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!shell.read(cx).imports.read(cx).exporting, "not stuck busy");
        let p = panel(&shell, cx);
        assert!(p.read(cx).can_import(), "Import is live after the cancel");
    });
    click_import(cx, handle, &shell);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "the import lands", |cx| {
        dash(cx, &shell).read(cx).row_count == 67
    });
    wait_idle(cx, &shell);
}
