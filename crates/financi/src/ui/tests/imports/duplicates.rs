//! Near-match duplicate review on the import preview (4.3): the offered
//! pair, the "Same charge" confirmation and the replace it commits.

use super::*;

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
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "base import", |cx| {
        shell.read(cx).imports.read(cx).import.is_none() && dash(cx, &shell).read(cx).row_count == 1
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
        shell.read(cx).imports.read(cx).import.is_none()
    });
    cx.update(|cx| {
        let d = dash(cx, &shell).read(cx);
        assert_eq!(d.row_count, 1, "the existing row was replaced, not added");
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
