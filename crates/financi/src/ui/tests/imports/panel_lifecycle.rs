//! The import panel's life and death: opens, commits, replaced and
//! abandoned panels, refused files, and results that land after the
//! window is gone (3.64, 3.94, 5.3).

use crate::ui::imports::ImportEvent;

use super::*;

/// 3.64: while a commit runs, the panel is the record of it — Cancel is
/// disabled, Import shows its spinner, and a file dropped mid-commit is
/// ignored instead of replacing the panel and stranding the result. The
/// commit itself finishes on its own whatever the user clicks.
#[gpui::test]
fn a_committing_import_disables_cancel_and_ignores_new_files(cx: &mut TestAppContext) {
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

    // Nothing has pumped the executor since `click_import` started the
    // commit, so this frame is the busy panel: Cancel disabled, Import a
    // spinner, and a second Import click impossible (`can_import` false).
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "imports:cancel-state:disabled"),
        "Cancel is disabled while the commit runs"
    );
    assert!(
        !paints(cx, handle, "imports:cancel-state:enabled"),
        "the enabled Cancel is gone"
    );
    assert!(
        paints(cx, handle, "imports:import-state:loading"),
        "Import shows loading while it commits"
    );
    let first_panel = cx.update(|cx| {
        assert!(!panel(&shell, cx).read(cx).can_import(), "no second commit");
        panel(&shell, cx).entity_id()
    });

    // A second file dropped mid-commit is ignored: same panel, same plan.
    with_window(cx, handle, |window, cx| {
        let path = demo("simplii-chequing.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).import.is_some(),
            "the panel stays"
        );
        assert_eq!(
            panel(&shell, cx).entity_id(),
            first_panel,
            "the drop while busy must not replace the committing panel"
        );
    });

    // A real click on the disabled Cancel must not abort the commit. The
    // input dispatch is the first pump the commit sees, so this also
    // proves the ordering: the click lands while the button is still
    // disabled (busy only clears in the commit's own continuation) and
    // does nothing; had Cancel fired, the panel would close with no
    // success toast at all — the detached commit's result has nowhere
    // to emit to.
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    let b = window
        .debug_bounds(leaked("imports:cancel-state:disabled".into()))
        .expect("the Cancel paints disabled while the commit is in flight");
    window.simulate_click(b.center(), Modifiers::default());

    // The commit lands on its own: panel gone, toast of the first file
    // only, and the second file was never imported.
    wait_until(cx, "commit finished", |cx| {
        shell.read(cx).imports.read(cx).import.is_none()
    });
    cx.update(|cx| {
        let (msg, is_error) = last_toast(&shell, cx).expect("import toast");
        assert!(!is_error, "the commit succeeded: {msg}");
        assert!(
            msg.starts_with("Imported amex-sep-oct.csv:"),
            "the toast names the committing file: {msg}"
        );
        assert!(
            !msg.contains("simplii"),
            "the ignored file left no trace: {msg}"
        );
    });
    wait_idle(cx, &shell);
    assert_eq!(shell.update(cx, |s, _| s.dashboard.row_count), 67);
}

/// 3.94 regression: the reverse order of the test above — a file is
/// dropped while the open panel is idle, then Import is clicked on that
/// panel while the load is still in flight. The load's update hop passes
/// the generation check (it *is* the newest open) and must re-apply the
/// 3.64 busy guard before building a panel: replacing the committing
/// panel would strand the commit's result on the dropped entity — the DB
/// import would finish with no toast and no refresh.
#[gpui::test]
fn a_load_landing_mid_commit_never_replaces_the_committing_panel(cx: &mut TestAppContext) {
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
    let first_panel = cx.update(|cx| panel(&shell, cx).entity_id());

    // Drop a second file while the panel is idle: the synchronous guard
    // passes and the load runs on the background executor.
    with_window(cx, handle, |window, cx| {
        let path = demo("simplii-chequing.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });

    // Import is clicked on the open panel while that load is in flight:
    // `busy` is set synchronously, and nothing has pumped since.
    click_import(cx, handle, &shell);
    cx.update(|cx| {
        assert_eq!(
            panel(&shell, cx).entity_id(),
            first_panel,
            "the in-flight load has built no panel yet"
        );
    });

    // Pumping lets the load return: its hop must see the busy panel and
    // ignore it, and the commit must land on the panel it started from.
    // Without the guard the hop replaces the tuple here, the commit's
    // result lands on the dropped entity, and this never resolves.
    wait_until(cx, "commit finished", |cx| {
        shell.read(cx).imports.read(cx).import.is_none()
    });
    cx.update(|cx| {
        let (msg, is_error) = last_toast(&shell, cx).expect("import toast");
        assert!(!is_error, "the commit succeeded: {msg}");
        assert!(
            msg.starts_with("Imported amex-sep-oct.csv:"),
            "the toast names the committing file: {msg}"
        );
        assert!(
            !msg.contains("simplii"),
            "the ignored file left no trace: {msg}"
        );
    });
    wait_idle(cx, &shell);
    assert_eq!(shell.update(cx, |s, _| s.dashboard.row_count), 67);
}

/// 3.64: every opened panel subscribes the shell to it, and the
/// subscription lives and dies with the panel. Ten files dropped in a row
/// replace the panel ten times — the shell's subscription list must not
/// grow by ten (it used to: each open leaked one).
#[gpui::test]
fn opening_ten_files_leaves_one_import_subscription(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    let before = cx.update(|cx| shell.read(cx)._subs.len());

    let mut seen = Vec::new();
    for _ in 0..10 {
        with_window(cx, handle, |window, cx| {
            let path = demo("amex-sep-oct.csv");
            shell.update(cx, |s, cx| s.open_file(path, window, cx));
        });
        // 3.94: the load runs on the background executor, so pump once
        // before the panel it builds exists.
        cx.run_until_parked();
        let id = cx.update(|cx| panel(&shell, cx).entity_id());
        assert!(
            !seen.contains(&id),
            "each open_file replaces the panel — a stuck panel would empty this test"
        );
        seen.push(id);
    }
    // Let the panels' account loads finish, then count.
    cx.run_until_parked();
    let after = cx.update(|cx| shell.read(cx)._subs.len());
    assert_eq!(
        after, before,
        "ten replaced panels must leave exactly one import subscription"
    );
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).import.is_some(),
            "the tenth panel is the open one"
        );
    });
}

/// 3.64: the live half of the previous test — gpui cancels a `Subscription`
/// when its handle *drops*, so replacing the panel really does unsubscribe
/// it. A panel the shell has moved on from must stop reaching the shell:
/// the abandoned panel cancelling itself used to close the current one.
#[gpui::test]
fn a_replaced_import_panel_stops_reaching_the_shell(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);

    with_window(cx, handle, |window, cx| {
        let path = demo("amex-sep-oct.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    cx.run_until_parked();
    // Held here so the old entity outlives its replacement: what must go
    // away is its subscription, not the panel.
    let old = cx.update(|cx| panel(&shell, cx));

    with_window(cx, handle, |window, cx| {
        let path = demo("simplii-chequing.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    cx.run_until_parked();
    let current = cx.update(|cx| panel(&shell, cx).entity_id());
    assert_ne!(
        current,
        old.entity_id(),
        "the second file replaced the panel"
    );

    // The abandoned panel cancels itself; the shell ignores it now.
    old.update(cx, |_, cx| cx.emit(ImportEvent::Cancelled));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).import.is_some(),
            "an event from the replaced panel must not close the open one"
        );
        assert_eq!(
            panel(&shell, cx).entity_id(),
            current,
            "and must not replace it either"
        );
    });
}

/// 3.94: loads run on the background executor, so a second drop while
/// the first file is still loading must not interleave panels: the first
/// load's result is discarded and only the newest open previews.
#[gpui::test]
fn opening_two_files_quickly_previews_only_the_second(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);

    with_window(cx, handle, |window, cx| {
        let path = demo("amex-sep-oct.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    // The load is in flight — `open_file` returned with no panel, so
    // nothing read or parsed on the UI thread.
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).import.is_none(),
            "the panel arrives when the background load returns"
        );
    });
    with_window(cx, handle, |window, cx| {
        let path = demo("simplii-chequing.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    cx.run_until_parked();
    let open = cx.update(|cx| {
        let p = panel(&shell, cx);
        assert_eq!(
            p.read(cx).file.file_name,
            "simplii-chequing.csv",
            "the newest open owns the panel"
        );
        assert!(
            toasts(&shell, cx).is_empty(),
            "the superseded load stays silent"
        );
        p.entity_id()
    });
    // Nothing is still pending: both loads have reported, one panel stands.
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            panel(&shell, cx).entity_id(),
            open,
            "the first load's result never lands"
        );
    });
}

#[gpui::test]
fn unrecognized_file_shows_an_error_and_no_panel(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    let dir = std::env::temp_dir().join(format!("financi-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let odd = dir.join("odd.csv");
    std::fs::write(&odd, "Posted,Payee,Debit\n2026-11-01,X,1.00\n").unwrap();

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.open_file(odd.clone(), window, cx));
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(s.imports.read(cx).import.is_none());
        let (msg, is_error) = last_toast(&shell, cx).expect("error toast");
        assert!(is_error && msg.contains("not a recognized export"), "{msg}");
    });
    std::fs::remove_dir_all(dir).unwrap();
}

/// 3.94: a file over the 50 MB limit is refused from its metadata, not
/// read or parsed — the same error toast the in-memory guard produces,
/// and no panel. (The fixture is sparse: metadata sees the size, disk
/// does not hold 50 MB.)
#[gpui::test]
fn oversize_file_is_rejected_without_a_panel(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    let dir = std::env::temp_dir().join(format!("financi-ui-oversize-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.csv");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(50 * 1024 * 1024 + 1).unwrap();
    drop(f);

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.open_file(big.clone(), window, cx));
    });
    wait_until(cx, "the oversize refusal toast", |cx| {
        !toasts(&shell, cx).is_empty()
    });
    cx.update(|cx| {
        let (msg, is_error) = last_toast(&shell, cx).expect("error toast");
        assert!(is_error, "{msg}");
        assert!(
            msg.contains("This isn't a CSV export."),
            "same message as the in-memory guard: {msg}"
        );
        assert!(
            shell.read(cx).imports.read(cx).import.is_none(),
            "a huge drop never opens a panel"
        );
        assert_eq!(shell.read(cx).page, Page::Dashboard, "the page stays put");
    });
    std::fs::remove_dir_all(dir).unwrap();
}

/// 5.3 review fix: an archive result that lands after the window is
/// gone hits the weak `update_in` path — it is dropped, never panicked.
/// Phase 1 releases a restore job to a closed window (its continuation
/// touches the window for the page reset); phase 2 answers an export's
/// save dialog after that window closed.
#[gpui::test]
fn archive_results_landing_after_the_window_closes_are_dropped(cx: &mut TestAppContext) {
    // Phase 1: Restore… answered, window closed, job released.
    let (handle, shell) = open_shell_on_file(cx, |_| {});
    wait_idle(cx, &shell);
    import_demo_file(cx, handle, &shell, "amex-sep-oct.csv", "Amex");
    wait_until(cx, "67 rows", |cx| shell.read(cx).dashboard.row_count == 67);
    let snapshot = scratch_file("snapshot.zip");
    export_to(cx, handle, &shell, &snapshot);
    let db = shell.update(cx, |s, _| s.db.clone());
    let raw_count = |db: &financi_db::DbWorker| -> i64 {
        block_on(db.call(|c| {
            c.query_row("SELECT COUNT(*) FROM transactions", [], |r| {
                r.get::<_, i64>(0)
            })
        }))
        .unwrap()
        .unwrap()
    };
    // The snapshot's raw rows: the fixture's 68, one of them a hidden
    // transfer the dashboard does not count.
    let snapshot_rows = raw_count(&db);
    assert_eq!(snapshot_rows, 68, "raw rows the snapshot carries");
    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Simplii");
    wait_until(cx, "the second import lands", |cx| {
        shell.read(cx).dashboard.row_count > 67
    });
    assert!(raw_count(&db) > snapshot_rows, "the second import landed");

    let block = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RestoreBackup), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "Restore opened the file dialog");
    cx.simulate_path_prompt_response(move |_| Some(vec![snapshot]));
    cx.run_until_parked();
    // The job waits on the blocker; close the window under it.
    with_window(cx, handle, |window, _cx| window.remove_window());
    // Release: the restore replaces the database and its result tries
    // to land on the closed window — dropped, not panicked.
    drop(block);
    cx.run_until_parked();
    let rows = raw_count(&db);
    assert_eq!(
        rows, snapshot_rows,
        "the restore ran after the window closed and the result was dropped"
    );

    // Phase 2: Export… dialog answered after its window closed.
    let (handle2, shell2) = open_shell_on_file(cx, |_| {});
    wait_idle(cx, &shell2);
    import_demo_file(cx, handle2, &shell2, "amex-sep-oct.csv", "Amex");
    wait_until(cx, "67 rows again", |cx| {
        shell2.read(cx).dashboard.row_count == 67
    });
    let block2 = block_worker(cx, &shell2);
    with_window(cx, handle2, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    assert!(
        cx.did_prompt_for_new_path(),
        "Export opened the save dialog"
    );
    with_window(cx, handle2, |window, _cx| window.remove_window());
    let late_zip = scratch_file("late-export.zip");
    let dest = late_zip.clone();
    cx.simulate_new_path_selection(move |_| Some(dest));
    drop(block2);
    cx.run_until_parked();
    let db2 = shell2.update(cx, |s, _| s.db.clone());
    block_on(db2.call(|_| ())).unwrap();
    let written = late_zip.metadata().expect("the export still wrote its zip");
    assert!(written.len() > 0, "and it is not empty");
}
