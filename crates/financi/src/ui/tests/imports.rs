//! Tests for `ui/imports.rs` (3.62 split from `ui_tests.rs`).

use super::*;
use crate::ui::imports::ImportEvent;
#[gpui::test]
fn drop_create_account_inline_and_import(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);

    open_and_choose(cx, handle, &shell, "simplii-chequing.csv", NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        let p = panel(&shell, cx);
        assert!(p.read(cx).creating_new, "new-account form shown");
        assert!(!p.read(cx).can_import(), "no account yet");
        assert_eq!(
            p.read(cx).institution,
            "Simplii Financial",
            "the preview names the institution"
        );
        p.update(cx, |p, cx| {
            p.new_name.update(cx, |input, cx| {
                input.set_value("No Fee Chequing", window, cx)
            });
            p.create_account(window, cx);
        });
    });

    wait_until(cx, "plan after creating account", |cx| {
        plan_ready(&shell, cx)
    });
    cx.update(|cx| {
        let p = panel(&shell, cx);
        let PlanState::Ready(s) = &p.read(cx).plan else {
            unreachable!()
        };
        assert_eq!((s.total, s.new_rows, s.overlap.clone()), (11, 11, None));
    });
    click_import(cx, handle, &shell);

    wait_until(cx, "import finished and table refreshed", |cx| {
        shell.read(cx).imports.import.is_none() && shell.read(cx).dashboard.row_count == 11
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.accounts.len(), 1);
        assert_eq!(
            s.accounts[0].kind,
            AccountKind::Chequing,
            "type follows the detected format"
        );
        assert_eq!(
            s.accounts[0].institution.key, "simplii",
            "the new account lands under the detected institution"
        );
        assert_eq!(
            last_toast(&shell, cx),
            Some((
                "Imported simplii-chequing.csv: 11 new, 0 already imported".into(),
                false
            ))
        );
    });
}

#[gpui::test]
fn empty_account_name_is_refused(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    open_and_choose(cx, handle, &shell, "amex-sep-oct.csv", NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        panel(&shell, cx).update(cx, |p, cx| p.create_account(window, cx));
    });
    cx.update(|cx| {
        assert_eq!(
            panel(&shell, cx).read(cx).error.as_deref(),
            Some("Give the account a name.")
        );
    });
}

/// 3.16: the import preview's inline new-account field carries a visible
/// label, not just its placeholder.
#[gpui::test]
fn the_new_account_field_has_a_visible_label(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    open_and_choose(cx, handle, &shell, "amex-sep-oct.csv", NEW_ACCOUNT);
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("label:New account name").is_some(),
        "the new-account input is labelled"
    );
}

/// 3.61: line one of the preview is the file name, then the institution
/// as an outline tag — the picker's "Institution · Account" name, not the
/// parser id. The muted "Detected format: …" it replaced is gone: nothing
/// paints between the file name and the tag but the row's 8 px gap.
#[gpui::test]
fn the_preview_tags_the_institution(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    with_window(cx, handle, |window, cx| {
        let path = demo("amex-sep-oct.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    wait_until(cx, "the panel opens", |cx| {
        shell.read(cx).imports.import.is_some()
    });
    draw_frame(cx, handle);
    assert!(
        paints(cx, handle, "imports:institution:American Express"),
        "the preview paints the institution name"
    );
    let name = bounds(cx, handle, "imports:file-name");
    let tag = bounds(cx, handle, "imports:institution:American Express");
    let gap = f32::from(tag.origin.x) - f32::from(name.origin.x + name.size.width);
    assert!(
        gap > 0. && gap < 16.,
        "the tag follows the file name directly ({gap} px): no \
         \"Detected format\" text paints between them"
    );
}

/// 3.61: the "Into" picker lists only the detected institution's eligible
/// accounts, each item reading "Institution · Account", plus the
/// new-account entry.
#[gpui::test]
fn the_account_picker_items_carry_the_institution(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
        create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
        create_account(c, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();
    });
    wait_until(cx, "accounts loaded", |cx| {
        shell.read(cx).accounts.len() == 3
    });
    with_window(cx, handle, |window, cx| {
        let path = demo("amex-sep-oct.csv");
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    wait_until(cx, "the panel lists the Amex accounts", |cx| {
        panel(&shell, cx)
            .read(cx)
            .names
            .iter()
            .any(|n| n == "American Express · Cobalt")
    });
    cx.update(|cx| {
        let names: Vec<String> = panel(&shell, cx)
            .read(cx)
            .names
            .iter()
            .map(|n| n.to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "American Express · Cobalt".to_string(),
                "American Express · Gold".to_string(),
                NEW_ACCOUNT.to_string(),
            ],
            "items read Institution · Account, and only the detected \
             institution's eligible accounts are offered"
        );
    });
}

/// 3.61: "+ New account…" creates the account under the detected
/// institution, so it carries the same name the tag and the picker show.
#[gpui::test]
fn a_new_account_lands_under_the_detected_institution(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    open_and_choose(cx, handle, &shell, "amex-sep-oct.csv", NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        panel(&shell, cx).update(cx, |p, cx| {
            p.new_name
                .update(cx, |input, cx| input.set_value("Cobalt", window, cx));
            p.create_account(window, cx);
        });
    });
    wait_until(cx, "the account created", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    cx.update(|cx| {
        let a = &shell.read(cx).accounts[0];
        assert_eq!(
            a.institution.key, "amex",
            "created under the detected institution"
        );
        assert_eq!(a.display_name(), "American Express · Cobalt");
    });
}

#[gpui::test]
fn overlap_with_another_account_blocks_until_overridden(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
        create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts loaded", |cx| {
        shell.read(cx).accounts.len() == 2
    });

    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "Cobalt plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    // 68 imported rows minus the card payment: transfers are hidden by
    // default (task 3.3).
    wait_until(cx, "Cobalt import", |cx| {
        shell.read(cx).dashboard.row_count == 67
    });

    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-nov.csv",
        "American Express · Gold",
    );
    wait_until(cx, "Gold plan", |cx| plan_ready(&shell, cx));
    cx.update(|cx| {
        let p = panel(&shell, cx);
        let PlanState::Ready(s) = &p.read(cx).plan else {
            unreachable!()
        };
        assert_eq!(s.overlap, Some(("American Express · Cobalt".into(), 68)));
        assert!(!p.read(cx).can_import(), "blocked without override");
    });
    click_import(cx, handle, &shell); // disabled: does nothing
    wait_idle(cx, &shell);
    cx.update(|cx| assert!(shell.read(cx).imports.import.is_some(), "still open"));

    with_window(cx, handle, |_, cx| {
        panel(&shell, cx).update(cx, |p, _| p.override_overlap = true); // the checkbox's handler
    });
    click_import(cx, handle, &shell);
    wait_until(cx, "Gold import with override", |cx| {
        shell.read(cx).imports.import.is_none() && shell.read(cx).dashboard.row_count == 67 + 100
    });
}

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
        assert!(shell.read(cx).imports.import.is_some(), "the panel stays");
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
        shell.read(cx).imports.import.is_none()
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
        shell.read(cx).imports.import.is_none()
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
            shell.read(cx).imports.import.is_some(),
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
            shell.read(cx).imports.import.is_some(),
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
            shell.read(cx).imports.import.is_none(),
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

/// `open_and_choose` for an arbitrary path instead of a demo fixture.
fn open_path_and_choose(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    path: &std::path::Path,
    account: &str,
) {
    let path = path.to_path_buf();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.open_file(path, window, cx));
    });
    let offered = format!("{account} offered in the account picker");
    wait_until(cx, &offered, |cx| {
        panel(shell, cx).read(cx).names.iter().any(|n| n == account)
    });
    with_window(cx, handle, |_, cx| {
        panel(shell, cx).update(cx, |p, cx| p.choose(account.into(), cx));
    });
}

/// Near-match review (4.3): the preview offers the suspected pair, the
/// default is "Different charge", and a confirmed "Same charge" replaces
/// the existing row instead of adding a new one.
#[gpui::test]
fn near_match_is_offered_and_a_confirmed_same_charge_replaces(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("financi-ui-near-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, row: &str| {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("Date,Date Processed,Description,Amount\n{row}\n"),
        )
        .unwrap();
        path
    };
    let base = write(
        "base.csv",
        "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74",
    );
    let tip = write(
        "tip.csv",
        "06 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,18.74",
    );

    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts loaded", |cx| {
        shell.read(cx).accounts.len() == 1
    });

    open_path_and_choose(cx, handle, &shell, &base, "American Express · Cobalt");
    wait_until(cx, "base plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    wait_until(cx, "base import", |cx| {
        shell.read(cx).imports.import.is_none() && shell.read(cx).dashboard.row_count == 1
    });

    open_path_and_choose(cx, handle, &shell, &tip, "American Express · Cobalt");
    wait_until(cx, "tip plan", |cx| plan_ready(&shell, cx));
    let key = cx.update(|cx| {
        let p = panel(&shell, cx);
        let PlanState::Ready(s) = &p.read(cx).plan else {
            unreachable!()
        };
        assert_eq!(s.near_matches.len(), 1, "the tip is suspected");
        let m = &s.near_matches[0];
        assert_eq!((m.source_line, m.new_amount_cents), (0, -1874));
        assert!(p.read(cx).same_charges.is_empty(), "default is Different");
        (m.source_line, m.existing_id)
    });
    with_window(cx, handle, |_, cx| {
        panel(&shell, cx).update(cx, |p, cx| p.set_same_charge(key, true, cx));
    });
    cx.update(|cx| {
        assert!(
            panel(&shell, cx).read(cx).same_charges.contains(&key),
            "the toggle marks the pair"
        );
    });
    click_import(cx, handle, &shell);
    wait_until(cx, "replacement import", |cx| {
        shell.read(cx).imports.import.is_none()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.dashboard.row_count, 1,
            "the existing row was replaced, not added"
        );
        assert_eq!(
            last_toast(&shell, cx),
            Some((
                "Imported tip.csv: 0 new, 1 replaced, 0 already imported".into(),
                false
            ))
        );
    });
    std::fs::remove_dir_all(dir).unwrap();
}

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
        shell.read(cx).imports.import_history.len() == 1
    });

    with_window(cx, handle, |window, cx| {
        let id = shell.read(cx).imports.import_history[0].id;
        shell.update(cx, |s, cx| s.undo_import(id, window, cx));
        assert_eq!(
            shell.read(cx).imports.confirm_undo_import,
            Some(id),
            "first click only asks to confirm"
        );
        assert_eq!(shell.read(cx).dashboard.row_count, 67);
        shell.update(cx, |s, cx| s.undo_import(id, window, cx));
    });

    wait_until(cx, "undo", |cx| {
        shell.read(cx).dashboard.row_count == 0 && shell.read(cx).imports.import_history.is_empty()
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

// --------------------------------------------------- 3.20 history table width

/// Seed three imports, oldest first, so the history table has several rows
/// and the newest one — the only row with the in-row Undo button — stands
/// out (3.32). The rows differ only in name and time; `imports.id` ascends,
/// and the history lists `id DESC`.
fn seed_three_imports(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "simplii", "Simplii Chequing", AccountKind::Chequing).unwrap();
    for n in 0..3 {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'simplii-chequing', ?2, ?3, X'00', 'utf-8', 0, 'lf', 0, 0)",
            financi_db::rusqlite::params![
                acct,
                format!("import-{n}.csv"),
                format!("2026-11-0{n}T09:00:00")
            ],
        )
        .unwrap();
    }
}

/// The width plan itself: the Undo column fits both button labels, and
/// from the minimum window's page width up every column fits the page —
/// File and Account shrink and grow with the window instead of the old
/// fixed 240 px each.
#[test]
fn history_table_columns_fit_every_page_width() {
    // The Undo column budgets the longest label ("Click again to undo")
    // at the generous per-character width, plus the xsmall button's icon
    // and padding and the cell's, so neither it nor the icon-plus-"Undo"
    // rest state (3.37) can clip.
    assert!(
        history_undo_col_w()
            >= UNDO_LABEL_MAX.chars().count() as f32 * TEXT_SM_CHAR_PX
                + BUTTON_PAD_X
                + UNDO_ICON_W
                + CELL_PAD_X
    );
    // 3.37: the idle label fits too, and the column is exactly as wide as
    // the widest label needs — no wider, so it steals nothing from `File`
    // and `Account`.
    assert!(
        history_undo_col_w()
            >= "Undo".chars().count() as f32 * TEXT_SM_CHAR_PX
                + BUTTON_PAD_X
                + UNDO_ICON_W
                + CELL_PAD_X
    );
    assert_eq!(
        history_undo_col_w(),
        (UNDO_LABEL_MAX.chars().count() as f32 * TEXT_SM_CHAR_PX
            + BUTTON_PAD_X
            + UNDO_ICON_W
            + CELL_PAD_X)
            .ceil()
    );
    for win in [1200., 1440., 1920., 2560., 4000.] {
        let page_w = win - SIDEBAR_W - 2. * PAGE_PAD;
        let (file, account) = history_flex_widths(page_w);
        let total = HISTORY_TIME_W
            + file
            + account
            + HISTORY_ROWS_W
            + HISTORY_DATES_W
            + history_undo_col_w()
            + TABLE_BORDER_X;
        assert!(
            total <= page_w + 0.01,
            "window {win}: columns sum to {total}, page is {page_w}"
        );
        assert!(
            file >= HISTORY_FLEX_MIN_W && account >= HISTORY_FLEX_MIN_W,
            "window {win}"
        );
    }
    // File and Account grow with the window, and at the minimum size
    // they are below the old fixed 240 px that overflowed the page.
    let small = history_flex_widths(1200. - SIDEBAR_W - 2. * PAGE_PAD).0;
    let big = history_flex_widths(2560. - SIDEBAR_W - 2. * PAGE_PAD).0;
    assert!(small < 240., "{small} at the minimum window");
    assert!(big > small, "{big} vs {small}");
    // Below the minimum window the flexible columns stop at their floor.
    assert_eq!(
        history_flex_widths(100.),
        (HISTORY_FLEX_MIN_W, HISTORY_FLEX_MIN_W)
    );
}

/// 3.20: with long file and account names the history table draws at the
/// minimum window size and at a large one, its columns — the widths the
/// table actually renders at — never summing past the page beside the
/// sidebar, so it never scrolls sideways.
#[gpui::test]
fn the_imports_history_table_fits_the_available_width(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_long_history);
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.import_history.len() == 1
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));

    let resize = |cx: &mut TestAppContext, w: f32, h: f32| {
        with_window(cx, handle, |window, cx| {
            window.resize(size(px(w), px(h)));
            window.bounds_changed(cx);
        });
        cx.run_until_parked();
        draw_frame(cx, handle);
    };
    let widths = |cx: &App| -> Vec<f32> {
        let table = shell.read(cx).imports.history_table.read(cx);
        (0..6)
            .map(|i| table.col_width(i).unwrap().as_f32())
            .collect()
    };

    // Minimum window: the drawn columns fit the page beside the sidebar.
    resize(cx, 1200., 720.);
    let small = cx.update(|cx| widths(cx));
    let page_w = 1200. - SIDEBAR_W - 2. * PAGE_PAD;
    assert!(
        small.iter().sum::<f32>() <= page_w + 0.01,
        "columns sum to {}, page is {page_w}",
        small.iter().sum::<f32>()
    );
    // Undo is sized for both labels; File and Account shrank below the
    // old fixed 240 px and truncate their long names with an ellipsis.
    assert!(
        small[5] >= history_undo_col_w(),
        "Undo column {} cannot fit both labels",
        small[5]
    );
    assert!(small[1] < 240. && small[2] < 240., "{small:?}");
    assert!((small[1] - small[2]).abs() < 0.01, "{small:?}");

    // A large window: still fitting, and the flexible columns grew.
    resize(cx, 2560., 1440.);
    let big = cx.update(|cx| widths(cx));
    assert!(
        big.iter().sum::<f32>() <= 2560. - SIDEBAR_W - 2. * PAGE_PAD + 0.01,
        "columns sum to {}",
        big.iter().sum::<f32>()
    );
    assert!(big[1] > small[1], "File column grows with the window");
}

/// Horizontal space a medium `Button` draws around its label when it has
/// an icon: the medium icon (`size_4`, 16 px) plus the `gap_2` (8 px)
/// between icon and label, and the button's own `px_2p5` per side (3.32).
const HEADER_ICON_BLOCK_W: f32 = 24.;
const HEADER_BUTTON_PAD_X: f32 = 20.;
/// Slack for comparing a drawn button width against label + icon +
/// padding: layout rounds to physical pixels. Tight enough that a
/// dropped `.icon()` call — 24 px on a header button, 16 on the armed
/// in-row one — fails the comparison.
const WIDTH_TOL: f32 = 3.;

/// 3.32: a window of plain labels — each at the text size the button
/// with that label renders (medium buttons `text_sm`, xsmall `text_xs`)
/// — so a test can pin that a button's drawn width is its label plus
/// exactly the icon block and padding. A debug selector records how a
/// button was built; only its width shows the icon actually paints.
struct IconWidthRefs {
    /// `(selector, text, xsmall?)` per label.
    labels: Vec<(&'static str, &'static str, bool)>,
}

impl Render for IconWidthRefs {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex().children(self.labels.iter().map(|(sel, text, xs)| {
            // `self_start` so the label's width is its text, not the
            // full window (columns stretch their children by default).
            let label = div().self_start().child(*text);
            if *xs {
                label.text_xs()
            } else {
                label.text_sm()
            }
            .debug_selector(move || (*sel).to_string())
        }))
    }
}

/// Open the reference-label window (after `open_shell`, so gpui-kit is
/// initialized) and draw one frame; the returned context reads the
/// labels' drawn widths.
fn open_icon_width_refs(
    cx: &mut TestAppContext,
    labels: Vec<(&'static str, &'static str, bool)>,
) -> VisualTestContext {
    let handle: AnyWindowHandle = cx.add_window(|_, _| IconWidthRefs { labels }).into();
    draw_frame(cx, handle);
    VisualTestContext::from_window(handle, cx)
}

/// Assert a drawn button's width is its label's drawn width plus the
/// icon block and horizontal padding (3.32) — the icon must paint.
fn assert_button_paints_icon(
    refs: &mut VisualTestContext,
    what: &str,
    btn: Bounds<Pixels>,
    text_sel: &'static str,
    icon_block_w: f32,
    pad_x: f32,
) {
    let text_w = refs
        .debug_bounds(text_sel)
        .unwrap_or_else(|| panic!("the reference window draws {text_sel:?}"))
        .size
        .width
        .as_f32();
    let got = btn.size.width.as_f32();
    let want = text_w + icon_block_w + pad_x;
    assert!(
        (got - want).abs() <= WIDTH_TOL,
        "{what}: drawn width {got} is not its label {text_w} + icon block \
         {icon_block_w} + padding {pad_x} (± {WIDTH_TOL}) — is the icon drawn?",
    );
}

/// 3.32: the history table's in-row Undo button is an xsmall ghost button.
/// Idle and armed it stays fully inside the fixed 32 px row with at least
/// 4 px above and below, arming never makes the newest row taller than the
/// others, and the armed label fits the Undo column. Hovering it shows the
/// "Undo this import" tooltip.
#[gpui::test]
fn the_in_row_undo_button_stays_inside_its_row(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.import_history.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // Each Undo cell carries a `h_full` marker spanning the cell's content
    // box, so the button's centring and the row heights are measurable.
    // The table's 32 px row height and 4 px-per-side cell padding are the
    // gpui-kit `Size::Medium` defaults the width constants already assume.
    const ROW_H: f32 = 32.;
    const CELL_PAD_Y: f32 = 4.;
    let cell = |cx: &mut TestAppContext, handle: AnyWindowHandle, row: usize| {
        bounds(cx, handle, format!("history:undo-cell:{row}"))
    };
    let btn = |window: &mut VisualTestContext, label: &str| {
        window
            .debug_bounds(leaked(format!("history:undo-btn:{label}")))
            .expect("the newest row draws its Undo button")
    };
    // Distance from the button's edges to the row's, the cell padding
    // included; the spec's floor is 4 px above and below.
    let clearance = |b: Bounds<Pixels>, c: Bounds<Pixels>| {
        (
            b.origin.y.as_f32() - c.origin.y.as_f32() + CELL_PAD_Y,
            c.origin.y.as_f32() + c.size.height.as_f32()
                - b.origin.y.as_f32()
                - b.size.height.as_f32()
                + CELL_PAD_Y,
        )
    };

    // Every row is one row height and every Undo cell the same height.
    let cells: Vec<_> = (0..3).map(|r| cell(cx, handle, r)).collect();
    for (above, below) in cells.iter().zip(cells[1..].iter()) {
        assert_eq!(
            above.size.height, below.size.height,
            "Undo cells are the same height in every row"
        );
        let pitch = below.origin.y.as_f32() - above.origin.y.as_f32();
        assert!(
            (pitch - ROW_H).abs() <= 1.,
            "rows sit one row height apart, pitch {pitch}"
        );
    }

    // Idle: the icon-and-"Undo"-label button (3.37) is centred with
    // clearance to spare, and its label fits the Undo column.
    let (above, below) = clearance(btn(&mut window, "Undo"), cells[0]);
    assert!(
        above >= 4. && below >= 4.,
        "idle: {above} above, {below} below"
    );
    let col_w = cx.update(|cx| {
        shell
            .read(cx)
            .imports
            .history_table
            .read(cx)
            .col_width(5)
            .unwrap()
    });
    assert!(
        btn(&mut window, "Undo").size.width + px(CELL_PAD_X) <= col_w,
        "the idle label overflows the Undo column: button {:?} + {CELL_PAD_X} > {col_w:?}",
        btn(&mut window, "Undo").size.width,
    );

    // Hovering the button shows the tooltip naming the action.
    let b = btn(&mut window, "Undo");
    let centre = point(b.center().x, b.center().y);
    window.simulate_mouse_move(centre, None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("history:undo-tooltip").is_some(),
        "hovering the Undo button shows its tooltip"
    );
    // Move off so the tooltip cannot hold the hover while the button is armed.
    window.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);

    // Arm the confirmation (first click): the button grows its prompt —
    // still with clearance, the row no taller than its neighbours, and the
    // armed label fitting the Undo column.
    let newest = cx.update(|cx| shell.read(cx).imports.import_history[0].id);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.undo_import(newest, window, cx));
    });
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).imports.confirm_undo_import,
            Some(newest),
            "the first click only arms the confirmation"
        );
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    let (above, below) = clearance(btn(&mut window, UNDO_LABEL_MAX), cell(cx, handle, 0));
    assert!(
        above >= 4. && below >= 4.,
        "armed: {above} above, {below} below"
    );
    let cells: Vec<_> = (0..3).map(|r| cell(cx, handle, r)).collect();
    assert_eq!(
        cells[0].size.height, cells[1].size.height,
        "the armed newest row is no taller than the others"
    );
    assert!(
        btn(&mut window, UNDO_LABEL_MAX).size.width + px(CELL_PAD_X) <= col_w,
        "the armed label overflows the Undo column: button {:?} + {CELL_PAD_X} > {col_w:?}",
        btn(&mut window, UNDO_LABEL_MAX).size.width,
    );
    // The header Undo shares the confirmation state: same moment, armed text.
    assert!(
        window
            .debug_bounds("imports:header:Click again to undo:icons/undo-2.svg")
            .is_some(),
        "the header Undo arms with the in-row one"
    );
}

/// 3.37: the in-row Undo button shows the `undo-2` icon plus the label
/// "Undo" — 3.32's icon-only button read as unclear. The selector records
/// the label it was built with; the drawn width proves the icon paints
/// beside it; and the labelled button still fits the Undo column at rest.
#[gpui::test]
fn the_in_row_undo_button_shows_the_undo_label(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.import_history.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // At rest the button reads "Undo".
    let idle = window
        .debug_bounds(leaked("history:undo-btn:Undo".into()))
        .expect("the newest row's Undo button reads \"Undo\"");
    // And it paints the icon beside the label: the drawn width is the
    // label plus the xsmall icon block and the button's padding, so a
    // dropped `.icon()` call — 16 px narrower — fails here.
    let mut refs = open_icon_width_refs(cx, vec![("ref:undo-xs", "Undo", true)]);
    assert_button_paints_icon(
        &mut refs,
        "the idle in-row Undo",
        idle,
        "ref:undo-xs",
        UNDO_ICON_W,
        BUTTON_PAD_X,
    );
    // With its label the idle button still fits the Undo column.
    let col_w = cx.update(|cx| {
        shell
            .read(cx)
            .imports
            .history_table
            .read(cx)
            .col_width(5)
            .unwrap()
    });
    assert!(
        idle.size.width + px(CELL_PAD_X) <= col_w,
        "the idle label overflows the Undo column: {} + {CELL_PAD_X} > {col_w:?}",
        idle.size.width
    );
}

/// 3.32: the Imports header buttons carry their Lucide icon left of the
/// label, and no label ends in an ellipsis — the action runs straight away,
/// and any confirmation is the button's own armed state. Each button's
/// selector records the exact label it was built with and the icon asset it
/// draws; the old "…" labels draw nothing.
#[gpui::test]
fn the_imports_header_buttons_show_icons_without_ellipses(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    wait_until(cx, "import history", |cx| {
        shell.read(cx).imports.import_history.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for sel in [
        "imports:header:Import CSV:icons/file-up.svg",
        "imports:header:Undo:icons/undo-2.svg",
        "imports:header:Export:icons/download.svg",
        "imports:header:Restore:icons/archive-restore.svg",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the header draws {sel:?}"
        );
    }
    for sel in [
        "imports:header:Import CSV…:icons/file-up.svg",
        "imports:header:Undo…:icons/undo-2.svg",
        "imports:header:Export…:icons/download.svg",
        "imports:header:Restore…:icons/archive-restore.svg",
    ] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "no header label ends in an ellipsis ({sel:?})"
        );
    }

    // Pin the icons for real (the selectors above only record how each
    // button was built): a button's drawn width must be its label plus
    // the medium icon block and the button's padding, so dropping an
    // `.icon()` call — 24 px narrower — fails here.
    let mut refs = open_icon_width_refs(
        cx,
        vec![
            ("ref:import-csv", "Import CSV", false),
            ("ref:undo", "Undo", false),
            ("ref:export", "Export", false),
            ("ref:restore", "Restore", false),
            ("ref:click-again", "Click again to undo", true),
        ],
    );
    for (sel, text_sel) in [
        (
            "imports:header:Import CSV:icons/file-up.svg",
            "ref:import-csv",
        ),
        ("imports:header:Undo:icons/undo-2.svg", "ref:undo"),
        ("imports:header:Export:icons/download.svg", "ref:export"),
        (
            "imports:header:Restore:icons/archive-restore.svg",
            "ref:restore",
        ),
    ] {
        let b = window
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("the header draws {sel:?}"));
        assert_button_paints_icon(
            &mut refs,
            sel,
            b,
            text_sel,
            HEADER_ICON_BLOCK_W,
            HEADER_BUTTON_PAD_X,
        );
    }

    // The shared confirm, seen from the header: the header button's own
    // first click arms it, and the in-row Undo reflects it — the armed
    // in-row button swaps its idle "Undo" label for the prompt together
    // with its icon (width as above).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.undo_last_import(window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds("imports:header:Click again to undo:icons/undo-2.svg")
            .is_some(),
        "the header button arms the confirm"
    );
    let armed = window
        .debug_bounds(leaked(format!("history:undo-btn:{UNDO_LABEL_MAX}")))
        .expect("the newest row draws its Undo button");
    assert_button_paints_icon(
        &mut refs,
        "the armed in-row Undo",
        armed,
        "ref:click-again",
        UNDO_ICON_W,
        BUTTON_PAD_X,
    );
}

/// 3.32: the four icons the Imports page draws are bundled with the app and
/// resolve through its asset source, the way the check glyph (3.14) must.
#[test]
fn the_imports_icons_load_from_the_bundled_assets() {
    use gpui_kit::AssetSource as _;
    let icons = [
        IconName::FileUp,
        IconName::Undo2,
        IconName::Download,
        IconName::ArchiveRestore,
    ];
    for icon in icons {
        let path = icon.path();
        let svg = crate::icon_assets()
            .load(&path)
            .ok()
            .flatten()
            .unwrap_or_else(|| panic!("the app bundles the {path} icon"));
        assert!(
            std::str::from_utf8(&svg).is_ok_and(|s| s.contains("<svg")),
            "{path} is an SVG"
        );
    }
}

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
        let input = shell
            .read(cx)
            .dashboard
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
        shell.read(cx).imports.import.is_some() && shell.read(cx).page == Page::Imports
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
        shell_b.read(cx).imports.import.is_some()
    });
    // The inactive shell never acted: once B's work has settled, it has
    // no panel of its own.
    wait_idle(cx, &shell_a);
    cx.update(|cx| {
        assert!(
            shell_a.read(cx).imports.import.is_none(),
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
        shell.read(cx).imports.import_history.len() == 1 && shell.read(cx).dashboard.row_count == 67
    });
    let id = cx.update(|cx| shell.read(cx).imports.import_history[0].id);

    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).imports.confirm_undo_import,
            Some(id),
            "first activation only asks to confirm"
        );
    });
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    wait_until(cx, "undo", |cx| {
        shell.read(cx).dashboard.row_count == 0 && shell.read(cx).imports.import_history.is_empty()
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
        assert_eq!(shell.read(cx).imports.confirm_undo_import, None);
        assert!(toasts(&shell, cx).is_empty());
    });
}

// -- Export / restore (5.3) ---------------------------------------------------

/// A scratch path beside a fresh per-test directory (the zip files the
/// export tests write land here, never in the repository).
fn scratch_file(name: &str) -> PathBuf {
    let dir = temp_db_path()
        .parent()
        .expect("scratch db path has a parent")
        .to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Answer the save dialog an Export… just opened with `dest`, and wait
/// for the success toast.
fn export_to(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    dest: &std::path::Path,
) {
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    assert!(
        cx.did_prompt_for_new_path(),
        "Export opened the save dialog"
    );
    let label = dest.file_name().unwrap().to_string_lossy().into_owned();
    let dest = dest.to_path_buf();
    cx.simulate_new_path_selection(move |_| Some(dest));
    wait_until(cx, "exported toast", |cx| {
        !shell.read(cx).exporting
            && last_toast(shell, cx) == Some((format!("Exported to {label}"), false))
    });
}

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

/// The save dialog's default (5.3): today's zip name, in the Desktop
/// directory (or home / `.` where the platform has no Desktop).
#[test]
fn export_defaults_to_todays_zip_on_the_desktop() {
    let (dir, name) = super::super::imports::export_default_path();
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
    wait_until(cx, "67 rows", |cx| shell.read(cx).dashboard.row_count == 67);
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
        !shell.read(cx).exporting
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
        assert!(!shell.read(cx).exporting, "not stuck busy");
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
        let input = shell
            .read(cx)
            .dashboard
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
    cx.update(|cx| assert!(!shell.read(cx).exporting));
}

/// Restore… replaces the whole database: the rows come back to the
/// export's, the second import's account is gone, and the armed Undo —
/// an id into the replaced data — is dropped (5.3).
#[gpui::test]
fn restore_replaces_the_data_and_resets_transient_state(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_on_file(cx, |_| {});
    wait_idle(cx, &shell);
    import_demo_file(cx, handle, &shell, "amex-sep-oct.csv", "Amex");
    wait_until(cx, "67 rows", |cx| shell.read(cx).dashboard.row_count == 67);
    let zip = scratch_file("snapshot.zip");
    export_to(cx, handle, &shell, &zip);

    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Simplii");
    wait_until(cx, "the second import lands", |cx| {
        shell.read(cx).dashboard.row_count > 67 && shell.read(cx).accounts.len() == 2
    });
    // Arm the history Undo: a transient id naming a row of the *old*
    // database, which must not survive the restore.
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    cx.run_until_parked();
    assert!(
        cx.update(|cx| shell.read(cx).imports.confirm_undo_import.is_some()),
        "the Undo is armed before the restore"
    );

    answer_restore(cx, handle, Some(zip.clone()));
    wait_until(cx, "the restored rows land", |cx| {
        shell.read(cx).dashboard.row_count == 67 && shell.read(cx).accounts.len() == 1
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.confirm_undo_import.is_none(),
            "the armed Undo did not survive the restore"
        );
        assert!(!shell.read(cx).restoring, "not stuck busy");
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
    wait_until(cx, "67 rows", |cx| shell.read(cx).dashboard.row_count == 67);
    let not_zip = scratch_file("not-an-export.zip");
    std::fs::write(&not_zip, b"not a zip at all").unwrap();

    answer_restore(cx, handle, Some(not_zip));
    wait_until(cx, "the error toast", |cx| {
        last_toast(&shell, cx).is_some_and(|(_, err)| err)
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert_eq!(shell.read(cx).dashboard.row_count, 67, "data untouched");
        assert!(!shell.read(cx).restoring, "not stuck busy");
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
    cx.update(|cx| assert_eq!(shell.read(cx).dashboard_body(), DashboardBody::NoData));
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
        assert_eq!(shell.read(cx).imports.import_history.len(), 1);
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
    wait_until(cx, "67 rows", |cx| shell.read(cx).dashboard.row_count == 67);
    let zip = scratch_file("snapshot.zip");
    export_to(cx, handle, &shell, &zip);
    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Simplii");
    wait_until(cx, "the second import lands", |cx| {
        shell.read(cx).dashboard.row_count > 67
    });
    let mid = cx.update(|cx| shell.read(cx).dashboard.row_count);

    // Hold the worker, start a dashboard load, and restore while its
    // query waits in the queue behind the blocker.
    let block = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.refresh(window, cx));
    });
    cx.run_until_parked();
    let (started, applied) = cx.update(|cx| {
        let s = shell.read(cx);
        (s.refresh_started, s.refresh_applied)
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
    wait_until(cx, "the restored rows land", |cx| {
        shell.read(cx).dashboard.row_count == 67
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.dashboard.row_count, 67, "the restored data stands");
        assert_ne!(s.dashboard.row_count, mid, "the late load never landed");
        assert_eq!(
            s.refresh_started, s.refresh_applied,
            "the restore's own refresh was the last to apply"
        );
    });
}

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
    cx.update(|cx| assert!(!shell.read(cx).restoring));
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "Export works once idle");
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!shell.read(cx).exporting);
        assert!(
            toasts(&shell, cx).is_empty(),
            "cancelled dialogs say nothing"
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
        assert!(s.imports.import.is_none());
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
            shell.read(cx).imports.import.is_none(),
            "a huge drop never opens a panel"
        );
        assert_eq!(shell.read(cx).page, Page::Dashboard, "the page stays put");
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn import_end_to_end_shows_expected_values_on_every_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);

    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Chequing");
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some((
                "Imported simplii-chequing.csv: 11 new, 0 already imported".into(),
                false
            ))
        );
    });

    // The fixtures carry fixed 2026 dates, so pin the range to All time:
    // the test must pass whenever it runs.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });
    // amex-sep-nov: 102 rows (100 unique + 2 genuine repeats), of which the
    // 2 card payments are auto-categorized as transfers and drop out of the
    // default view; simplii-chequing: 11 rows, of which the 2 "INTERNET BILL
    // PAY DEMO AMEX" debits pair with those payments (4.2) and are
    // categorized as Card Payment too.
    wait_until(cx, "all-time view over both imports", |cx| {
        shell.read(cx).dashboard.summary.months == ["2026-09", "2026-10", "2026-11"]
            && shell.read(cx).dashboard.row_count == 109
    });

    // Spend per month from expected.json with transfers excluded: the Amex
    // money-in rows are card payments (they leave the view entirely), and so
    // do the two paired chequing debits since 4.2; the Simplii money-in rows
    // stay uncategorized and offset its spend.
    let expected = expected_totals();
    let cents =
        |file: &str, month: &str, key: &str| expected[file]["months"][month][key].as_i64().unwrap();
    let paired_debit = |month: &str| match month {
        "2026-09" | "2026-10" => 150_000,
        _ => 0,
    };
    let want_spend: Vec<i64> = ["2026-09", "2026-10", "2026-11"]
        .iter()
        .map(|m| {
            cents("amex-sep-nov.csv", m, "money_out_cents")
                + cents("simplii-chequing.csv", m, "money_out_cents")
                - cents("simplii-chequing.csv", m, "money_in_cents")
                - paired_debit(m)
        })
        .collect();
    let spend_total: i64 = want_spend.iter().sum();

    // -- Dashboard: summary grid, chart + legend, sidebar counts ----------
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    cx.update(|cx| {
        let s = shell.read(cx);
        let row = |name: &str| {
            s.dashboard
                .summary
                .rows
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("no summary row {name:?}"))
        };
        assert_eq!(row("Spend").months, want_spend);
        assert_eq!(row("Spend").total, spend_total);
        assert_eq!(row("Income").total, 0, "nothing is categorized yet");
        assert_eq!(row("Net").total, -spend_total);
        // No user categories exist, so every spend row is Uncategorized.
        assert_eq!(row("Uncategorized").months, want_spend);
        assert_eq!(row("Uncategorized").total, spend_total);
        assert_eq!(row("Uncategorized").total, row("Spend").total);
        // Chart + legend (3.5): the single Uncategorized row.
        let chart: Vec<(&str, i64)> = s
            .dashboard
            .chart_rows
            .iter()
            .map(|r| (r.name.as_str(), r.spend_cents))
            .collect();
        assert_eq!(chart, [("Uncategorized", spend_total)]);
        // Sidebar count pills (task 2.x): every visible row, uncategorized
        // ones apart. The 2 card payments and their 2 paired chequing debits
        // are categorized, not hidden.
        assert_eq!((s.total_visible, s.uncategorized_visible), (113, 109));
    });

    // -- To categorize: the queue covers every uncategorized row ----------
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(!s.queue.groups.is_empty(), "the queue has groups");
        assert!(
            s.queue.groups.windows(2).all(|w| w[0].count >= w[1].count),
            "sorted by count descending"
        );
        let rows: i64 = s.queue.groups.iter().map(|g| g.count).sum();
        assert_eq!(
            rows, s.uncategorized_visible,
            "the queue covers every uncategorized row"
        );
        let cents: i64 = s.queue.groups.iter().map(|g| g.total_cents).sum();
        assert_eq!(cents, -spend_total, "queue amounts match the dashboard");
    });

    // -- Categories: only the system tree, with the card payments -------
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.update(|cx| {
        let s = shell.read(cx);
        let parents: Vec<&str> = s.categories.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(parents, ["Transfers"], "imports create no categories");
        let children: Vec<&str> = s.categories[0]
            .children
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(children, ["Between Accounts", "Card Payment"]);
        let card = find_child(&shell, cx, "Card Payment").expect("Card Payment child");
        assert_eq!(
            s.cat_counts.get(&card).copied(),
            Some(4),
            "the import auto-categorized both Amex payments and, since 4.2, \
             their two paired chequing debits"
        );
    });

    // -- Imports: history rows for both files ------------------------------
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    cx.update(|cx| {
        let s = shell.read(cx);
        let history: Vec<(&str, &str, i64)> = s
            .imports
            .import_history
            .iter()
            .map(|e| (e.file_name.as_str(), e.account.as_str(), e.rows_added))
            .collect();
        assert_eq!(
            history,
            [
                ("simplii-chequing.csv", "Simplii Financial · Chequing", 11),
                ("amex-sep-nov.csv", "American Express · Amex", 102),
            ],
            "newest import first"
        );
        assert_eq!(
            s.imports.import_history[0].first_date.as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(
            s.imports.import_history[0].last_date.as_deref(),
            Some("2026-11-01")
        );
        assert_eq!(
            s.imports.import_history[1].first_date.as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(
            s.imports.import_history[1].last_date.as_deref(),
            Some("2026-11-06")
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
    wait_until(cx, "67 rows", |cx| shell.read(cx).dashboard.row_count == 67);
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
        assert!(shell.read(cx).restoring, "the restore is in flight");
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
    wait_until(cx, "the restored rows land", |cx| {
        !shell.read(cx).restoring
            && shell.read(cx).dashboard.row_count == 67
            && shell.read(cx).accounts.len() == 1
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.import.is_none(),
            "the restore reset closed the panel"
        );
        assert_eq!(
            shell.read(cx).imports.import_history.len(),
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
    wait_until(cx, "67 rows", |cx| shell.read(cx).dashboard.row_count == 67);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    wait_until(cx, "queue populated", |cx| {
        !shell.read(cx).queue.groups.is_empty()
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
    cx.update(|cx| assert!(shell.read(cx).restoring, "the restore is in flight"));

    // The very call the row's Apply button makes, picker and all —
    // refused mid-restore.
    let refused = merchant.clone();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let picker = s.queue_picker(&refused, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.categorize_merchant(refused.clone(), window, cx);
        });
    });

    drop(block);
    wait_until(cx, "the restored rows land", |cx| {
        !shell.read(cx).restoring && shell.read(cx).dashboard.row_count == 67
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
        assert!(shell.read(cx).exporting, "the dialog is in flight");
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
        assert!(!shell.read(cx).exporting, "not stuck busy");
        let p = panel(&shell, cx);
        assert!(p.read(cx).can_import(), "Import is live after the cancel");
    });
    click_import(cx, handle, &shell);
    wait_until(cx, "the import lands", |cx| {
        shell.read(cx).dashboard.row_count == 67
    });
    wait_idle(cx, &shell);
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
