//! Restore…: replacing the database from an export, refusing files that
//! are not exports, and the empty-state entry point (5.3).

use super::*;

/// Dispatch Restore… and answer its file dialog with `zip` (or cancel
/// with `None`); returns once the dialog has been answered.
fn answer_restore(cx: &mut TestAppContext, handle: AnyWindowHandle, zip: Option<PathBuf>) {
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RestoreBackup), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "Restore opened the file dialog");
    cx.simulate_path_prompt_response(move |_| zip.map(|z| vec![z]));
    cx.run_until_parked();
}

/// Restore… replaces the whole database: the rows come back to the
/// export's, the second import's account is gone, and the armed Undo —
/// an id into the replaced data — is dropped (5.3).
#[gpui::test]
fn restore_replaces_the_data_and_resets_transient_state(cx: &mut TestAppContext) {
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

    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Simplii");
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "the second import lands", |cx| {
        dash(cx, &shell).read(cx).row_count > 67 && shell.read(cx).accounts.len() == 2
    });
    // Arm the history Undo: a transient id naming a row of the *old*
    // database, which must not survive the restore.
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    cx.run_until_parked();
    assert!(
        cx.update(|cx| shell
            .read(cx)
            .imports
            .read(cx)
            .confirm_undo_import
            .is_some()),
        "the Undo is armed before the restore"
    );

    answer_restore(cx, handle, Some(zip.clone()));
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "the restored rows land", |cx| {
        dash(cx, &shell).read(cx).row_count == 67 && shell.read(cx).accounts.len() == 1
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .imports
                .read(cx)
                .confirm_undo_import
                .is_none(),
            "the armed Undo did not survive the restore"
        );
        assert!(!shell.read(cx).imports.read(cx).restoring, "not stuck busy");
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Restored from snapshot.zip".into(), false))
        );
    });
}

/// A file that is not an export is refused: the live data is untouched,
/// and the error toast stays until closed (5.3).
#[gpui::test]
fn restore_of_a_file_that_is_not_an_export_stays_until_closed(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_on_file(cx, |_| {});
    wait_idle(cx, &shell);
    import_demo_file(cx, handle, &shell, "amex-sep-oct.csv", "Amex");
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "67 rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 67
    });
    let not_zip = scratch_file("not-an-export.zip");
    std::fs::write(&not_zip, b"not a zip at all").unwrap();

    answer_restore(cx, handle, Some(not_zip));
    wait_until(cx, "the error toast", |cx| {
        last_toast(&shell, cx).is_some_and(|(_, err)| err)
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert_eq!(dash(cx, &shell).read(cx).row_count, 67, "data untouched");
        assert!(!shell.read(cx).imports.read(cx).restoring, "not stuck busy");
    });
    // Errors stay until closed: ten seconds of ticker and it is up.
    tick_toasts(cx, 100);
    cx.update(|cx| {
        let toast = last_toast(&shell, cx).expect("the error toast is still mounted");
        assert!(toast.1);
        assert!(toast.0.starts_with("Restore failed:"), "{}", toast.0);
    });
}

/// The empty-state screen offers "Restore from export…" (5.3): the link
/// opens the file dialog, and a chosen zip fills the empty database.
#[gpui::test]
fn the_empty_state_offers_restore_from_export(cx: &mut TestAppContext) {
    // A zip exported from a seeded database …
    let seed_path = temp_db_path();
    let seed = financi_db::open(&seed_path).unwrap();
    seed_long_history(&seed);
    let zip = seed_path.with_file_name("seed.zip");
    financi_db::export::export(
        &seed,
        &financi_db::master_csv::root_for_db(&seed_path),
        &zip,
        "test",
    )
    .unwrap();
    drop(seed);

    // … restored into an empty one from the dashboard's empty state.
    let (handle, shell) = open_shell_on_file(cx, |_| {});
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert_eq!(
            dash(cx, &shell).read(cx).dashboard_body(),
            DashboardBody::NoData
        )
    });
    click(cx, handle, "dashboard:restore-link");
    assert!(cx.did_prompt_for_paths(), "the link opened the file dialog");
    cx.simulate_path_prompt_response(move |_| Some(vec![zip]));
    wait_until(cx, "the restored account", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        // The seed has an account and an import but no transactions, so
        // the dashboard may still be the empty state — what must have
        // changed is everything the restore brought back.
        // The Imports page was never shown (3.79): read the Catalog
        // snapshot the restore refreshed, not the hidden page's mirror.
        assert_eq!(shell.read(cx).catalog.read(cx).snap.imports.len(), 1);
        assert!(
            last_toast(&shell, cx)
                .is_some_and(|(msg, err)| !err && msg.starts_with("Restored from"))
        );
    });
}

/// 5.3 review update: a dashboard load in flight when the restore lands
/// is superseded — its late result never overwrites the restored view.
#[gpui::test]
fn restore_supersedes_an_in_flight_dashboard_load(cx: &mut TestAppContext) {
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
    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Simplii");
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "the second import lands", |cx| {
        dash(cx, &shell).read(cx).row_count > 67
    });
    let mid = cx.update(|cx| dash(cx, &shell).read(cx).row_count);

    // Hold the worker, start a dashboard load, and restore while its
    // query waits in the queue behind the blocker.
    let block = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.refresh(window, cx));
    });
    cx.run_until_parked();
    let (started, applied) = cx.update(|cx| {
        let s = shell.read(cx);
        (s.refresh_started.get(), s.refresh_applied.get())
    });
    assert!(started > applied, "the dashboard load is in flight");

    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RestoreBackup), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "Restore opened the file dialog");
    cx.simulate_path_prompt_response(move |_| Some(vec![zip]));
    cx.run_until_parked();

    // Release the worker: the queued load answers from the pre-restore
    // database, the restore follows, and its refresh supersedes the
    // load's task — the load's result is dropped on the floor.
    drop(block);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "the restored rows land", |cx| {
        dash(cx, &shell).read(cx).row_count == 67
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let s = shell.read(cx);
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.row_count, 67, "the restored data stands");
        assert_ne!(d.row_count, mid, "the late load never landed");
        assert_eq!(
            s.refresh_started.get(),
            s.refresh_applied.get(),
            "the restore's own refresh was the last to apply"
        );
    });
}
