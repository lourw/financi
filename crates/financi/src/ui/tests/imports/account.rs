//! Import preview account handling: inline new-account creation (3.16)
//! and the detected institution's tag and picker (3.61).

use super::*;

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

    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "import finished and table refreshed", |cx| {
        shell.read(cx).imports.read(cx).import.is_none()
            && dash(cx, &shell).read(cx).row_count == 11
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
        shell.read(cx).imports.read(cx).import.is_some()
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
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "Cobalt import", |cx| {
        dash(cx, &shell).read(cx).row_count == 67
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
    cx.update(|cx| {
        assert!(
            shell.read(cx).imports.read(cx).import.is_some(),
            "still open"
        )
    });

    with_window(cx, handle, |_, cx| {
        panel(&shell, cx).update(cx, |p, _| p.override_overlap = true); // the checkbox's handler
    });
    click_import(cx, handle, &shell);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "Gold import with override", |cx| {
        shell.read(cx).imports.read(cx).import.is_none()
            && dash(cx, &shell).read(cx).row_count == 67 + 100
    });
}
