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
            p.read(cx).format,
            "Simplii Financial chequing / credit card"
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
    // and padding and the cell's, so neither it nor the icon-only rest
    // state can clip (3.32).
    assert!(
        history_undo_col_w()
            >= UNDO_LABEL_MAX.chars().count() as f32 * TEXT_SM_CHAR_PX
                + BUTTON_PAD_X
                + UNDO_ICON_W
                + CELL_PAD_X
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
    let btn = |window: &mut VisualTestContext| {
        window
            .debug_bounds("history:undo-btn")
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

    // Idle: the icon-only button is centred with clearance to spare.
    let (above, below) = clearance(btn(&mut window), cells[0]);
    assert!(
        above >= 4. && below >= 4.,
        "idle: {above} above, {below} below"
    );

    // Hovering the button shows the tooltip naming the action.
    let b = btn(&mut window);
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

    // Arm the confirmation (first click): the button grows its label —
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
    let (above, below) = clearance(btn(&mut window), cell(cx, handle, 0));
    assert!(
        above >= 4. && below >= 4.,
        "armed: {above} above, {below} below"
    );
    let cells: Vec<_> = (0..3).map(|r| cell(cx, handle, r)).collect();
    assert_eq!(
        cells[0].size.height, cells[1].size.height,
        "the armed newest row is no taller than the others"
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
        btn(&mut window).size.width + px(CELL_PAD_X) <= col_w,
        "the armed label overflows the Undo column: button {:?} + {CELL_PAD_X} > {col_w:?}",
        btn(&mut window).size.width,
    );
    // The header Undo shares the confirmation state: same moment, armed text.
    assert!(
        window
            .debug_bounds("imports:header:Click again to undo:icons/undo-2.svg")
            .is_some(),
        "the header Undo arms with the in-row one"
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
    // in-row button draws the prompt together with its icon (width as
    // above; at rest the icon-only button is a fixed square, so only the
    // armed state shows whether the icon paints).
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
        .debug_bounds("history:undo-btn")
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

/// Export… and Restore… are wired in M5: the actions (menu items, ⌘E, the
/// page buttons) already exist and say plainly that they are not available.
#[gpui::test]
fn export_and_restore_say_not_available_yet(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            toasts(&shell, cx),
            vec![("Export is not available yet.".into(), false)]
        );
    });
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(RestoreBackup), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            toasts(&shell, cx),
            vec![
                ("Export is not available yet.".into(), false),
                ("Restore is not available yet.".into(), false),
            ],
            "a burst of messages stacks, it does not replace"
        );
    });
}

/// The ⌘E binding reaches the same not-yet-available state as the button.
#[gpui::test]
fn cmd_e_dispatches_export(cx: &mut TestAppContext) {
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
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Export is not available yet.".into(), false))
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
