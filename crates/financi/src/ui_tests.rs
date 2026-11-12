//! Headless UI tests: real views in a test window and a real DbWorker over an
//! in-memory database, driven through the same view methods the buttons
//! call, using the synthetic demo exports.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDate};
use financi_core::dates::{DatePreset, preset_range};
use financi_core::parse::merchant;
use financi_core::rules::Rule;
use financi_core::{AccountKind, CategoryKind};
use financi_db::import::TransactionRow;
use financi_db::settings::{self, DateRange, UI_DATE_RANGE, UI_INCLUDE_TRANSFERS};
use financi_db::{
    CategorySpend, DbWorker, TxSort, TxSortColumn, ViewFilter, create_account, open_in_memory,
};
use gpui_kit::component::table::{ColumnSort, TableDelegate};
// No glob import: `gpui_kit::*` brings GPUI's `test` macro into scope, which
// shadows the built-in #[test] that #[gpui::test] expands to.
use gpui_kit::assets::IconName;
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Entity, Focusable as _, Modifiers, OwnedMenuItem,
    Pixels, Point, Render, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, div, point, px, size,
};

use super::{
    BAR_LABEL_MAX_CHARS, BUTTON_PAD_X, CELL_PAD_X, CHART_CHAR_PX, CHART_LABEL_GAP,
    CHART_LABEL_MAX_CHARS, CHART_LABEL_MIN_CHARS, DashboardBody, ExportData, HISTORY_DATES_W,
    HISTORY_FLEX_MIN_W, HISTORY_ROWS_W, HISTORY_TIME_W, ImportPanel, LEGEND_W, LegendSelection,
    NAV_ROW_INSET, NEW_ACCOUNT, NEW_CATEGORY, NEW_RULE, PAGE_PAD, Page, PendingRuleAction,
    PlanState, RestoreBackup, RowMenuItem, RowShowRule, SIDEBAR_W, Shell, SummaryRowKind,
    TABLE_BORDER_X, TEXT_SM_CHAR_PX, TX_FIXED_W, UNDO_ICON_W, UNDO_LABEL_MAX, UndoLastImport,
    band_label_budget, bar_label_plan, bar_label_slots, bar_share_label, chart_tooltip_value,
    history_flex_widths, history_undo_col_w, row_menu_items, row_show_rule, short_chart_labels,
    tx_flex_widths,
};

fn demo(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/demo")
        .join(name)
}

/// Open the main window over a fresh in-memory database prepared by `setup`.
fn open_shell(
    cx: &mut TestAppContext,
    setup: impl FnOnce(&financi_db::rusqlite::Connection),
) -> (AnyWindowHandle, Entity<Shell>) {
    // The DB worker answers from its own OS thread.
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let conn = open_in_memory().unwrap();
    setup(&conn);
    let db = DbWorker::spawn(conn);
    // A real path beside a scratch directory so master CSV writes during
    // import/undo never land in the repository. The database itself stays
    // in memory.
    let db_path = std::env::temp_dir().join(format!(
        "financi-ui-db-{}/financi.sqlite",
        std::process::id()
    ));
    cx.update(|cx| {
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1100.), px(720.)),
        };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Shell::new(db_path, db, window, cx)),
        )
        .unwrap();
        (window, shell)
    })
}

/// Drive the executor until `done` holds, up to 10 s of real time (the DB
/// worker runs on a real thread).
fn wait_until(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if cx.update(|cx| done(cx)) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn with_window<R>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    cx.update_window(handle, |_, window, cx| f(window, cx))
        .unwrap()
}

fn panel(shell: &Entity<Shell>, cx: &App) -> Entity<ImportPanel> {
    shell.read(cx).import.clone().expect("import panel open")
}

fn plan_ready(shell: &Entity<Shell>, cx: &App) -> bool {
    shell
        .read(cx)
        .import
        .as_ref()
        .is_some_and(|p| matches!(p.read(cx).plan, PlanState::Ready(_)))
}

/// Open a file's import panel, wait until its account list offers
/// `account` (it loads from the DB worker), then choose it.
fn open_and_choose(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    file: &str,
    account: &str,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.open_file(demo(file), window, cx));
    });
    let offered = format!("{account} offered in the account picker");
    wait_until(cx, &offered, |cx| {
        panel(shell, cx).read(cx).names.iter().any(|n| n == account)
    });
    with_window(cx, handle, |_, cx| {
        panel(shell, cx).update(cx, |p, cx| p.choose(account.into(), cx));
    });
}

fn click_import(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>) {
    with_window(cx, handle, |_, cx| {
        let p = panel(shell, cx);
        // The Import button is disabled exactly when can_import() is false.
        if p.read(cx).can_import() {
            p.update(cx, |p, cx| p.import(cx));
        }
    });
}

/// 3.27: the toasts mounted in the bottom-right overlay, oldest first, as
/// `(text, is_error)` — including any mid-exit. The feedback assertion of
/// choice now that `Shell::status` is gone. Toast timers only run when the
/// test advances the executor clock, so a toast pushed earlier in a test
/// stays mounted until then.
fn toasts(shell: &Entity<Shell>, cx: &App) -> Vec<(String, bool)> {
    shell
        .read(cx)
        .toasts
        .iter()
        .map(|(_, toast, _)| (toast.message.to_string(), toast.is_error))
        .collect()
}

/// The newest mounted toast, if any.
fn last_toast(shell: &Entity<Shell>, cx: &App) -> Option<(String, bool)> {
    toasts(shell, cx).pop()
}

/// Advance the toast ticker by `ticks` intervals of 100 ms. `advance_clock`
/// readies a self-re-arming timer once per call, so tests drive the ticker
/// tick by tick instead of advancing seconds at once.
fn tick_toasts(cx: &mut TestAppContext, ticks: u32) {
    for _ in 0..ticks {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
}

#[gpui::test]
fn drop_create_account_inline_and_import(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });

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
        shell.read(cx).import.is_none() && shell.read(cx).row_count == 11
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
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
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
    wait_until(cx, "Cobalt import", |cx| shell.read(cx).row_count == 67);

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
    cx.run_until_parked();
    cx.update(|cx| assert!(shell.read(cx).import.is_some(), "still open"));

    with_window(cx, handle, |_, cx| {
        panel(&shell, cx).update(cx, |p, _| p.override_overlap = true); // the checkbox's handler
    });
    click_import(cx, handle, &shell);
    wait_until(cx, "Gold import with override", |cx| {
        shell.read(cx).import.is_none() && shell.read(cx).row_count == 67 + 100
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
        shell.read(cx).import.is_none() && shell.read(cx).row_count == 1
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
        shell.read(cx).import.is_none()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.row_count, 1, "the existing row was replaced, not added");
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
    wait_until(cx, "import", |cx| shell.read(cx).row_count == 67);

    wait_until(cx, "import history", |cx| {
        shell.read(cx).import_history.len() == 1
    });

    with_window(cx, handle, |window, cx| {
        let id = shell.read(cx).import_history[0].id;
        shell.update(cx, |s, cx| s.undo_import(id, window, cx));
        assert_eq!(
            shell.read(cx).confirm_undo_import,
            Some(id),
            "first click only asks to confirm"
        );
        assert_eq!(shell.read(cx).row_count, 67);
        shell.update(cx, |s, cx| s.undo_import(id, window, cx));
    });

    wait_until(cx, "undo", |cx| {
        shell.read(cx).row_count == 0 && shell.read(cx).import_history.is_empty()
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

/// Seed one import whose file name and account name are both far longer
/// than any column, the worst case for the history table.
fn seed_long_history(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(
        conn,
        "simplii",
        "Simplii Chequing at Home and Abroad Forever",
        AccountKind::Chequing,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'simplii-chequing',
                 'simplii-chequing-statement-2026-10-to-2026-11-final-v2.csv',
                 '2026-11-06T10:00:00', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
}

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

/// 3.25: the dashboard table's width plan — the flexed `Account`,
/// `Description` and `Category` columns plus the fixed ones always sum to
/// exactly the page width, so the table reaches the page's right edge
/// without overhanging it at any window size (the wrapper alone cannot
/// pin this, and the old fixed 300/360/260 columns overflowed 1200 px).
#[test]
fn dashboard_table_columns_fit_every_page_width() {
    for win in [1200., 1600., 1920., 2560.] {
        let page_w = win - SIDEBAR_W - 2. * PAGE_PAD;
        let (account, description, category) = tx_flex_widths(page_w);
        let total = TX_FIXED_W + TABLE_BORDER_X + account + description + category;
        assert!(
            (total - page_w).abs() <= 0.01,
            "window {win}: columns sum to {total}, page is {page_w}"
        );
        assert!(
            account >= HISTORY_FLEX_MIN_W
                && description >= HISTORY_FLEX_MIN_W
                && category >= HISTORY_FLEX_MIN_W,
            "window {win}: flexed columns below the floor"
        );
    }
    // The columns grow with the window...
    let small = tx_flex_widths(1200. - SIDEBAR_W - 2. * PAGE_PAD).1;
    let big = tx_flex_widths(2560. - SIDEBAR_W - 2. * PAGE_PAD).1;
    assert!(big > small, "{big} vs {small}");
    // ...and below any real window the flexed total stops at its floor.
    let (a, d, c) = tx_flex_widths(100.);
    assert!(
        (a + d + c - 3. * HISTORY_FLEX_MIN_W).abs() <= 0.01,
        "floored total {} is not 3 × {HISTORY_FLEX_MIN_W}",
        a + d + c,
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
        shell.read(cx).import_history.len() == 1
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
        let table = shell.read(cx).history_table.read(cx);
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
        shell.read(cx).import_history.len() == 3
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
    let cell = |window: &mut VisualTestContext, row: usize| {
        window
            .debug_bounds(leaked(format!("history:undo-cell:{row}")))
            .unwrap_or_else(|| panic!("row {row} draws its Undo cell"))
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
    let cells: Vec<_> = (0..3).map(|r| cell(&mut window, r)).collect();
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
    let newest = cx.update(|cx| shell.read(cx).import_history[0].id);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.undo_import(newest, window, cx));
    });
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).confirm_undo_import,
            Some(newest),
            "the first click only arms the confirmation"
        );
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    let (above, below) = clearance(btn(&mut window), cell(&mut window, 0));
    assert!(
        above >= 4. && below >= 4.,
        "armed: {above} above, {below} below"
    );
    let cells: Vec<_> = (0..3).map(|r| cell(&mut window, r)).collect();
    assert_eq!(
        cells[0].size.height, cells[1].size.height,
        "the armed newest row is no taller than the others"
    );
    let col_w = cx.update(|cx| shell.read(cx).history_table.read(cx).col_width(5).unwrap());
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
        shell.read(cx).import_history.len() == 3
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
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
    // Actions dispatch along the focus path, so focus the search input
    // first — like a user who clicked into the window.
    with_window(cx, handle, |window, cx| {
        let input = shell.read(cx).search_input.read(cx).focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "cmd-o");
    assert!(cx.did_prompt_for_paths(), "⌘O opened the file picker");

    let file = demo("simplii-chequing.csv");
    cx.simulate_path_prompt_response(move |_| Some(vec![file]));
    wait_until(cx, "preview open on the Imports page", |cx| {
        shell.read(cx).import.is_some() && shell.read(cx).page == Page::Imports
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
        shell.read(cx).import_history.len() == 1 && shell.read(cx).row_count == 67
    });
    let id = cx.update(|cx| shell.read(cx).import_history[0].id);

    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).confirm_undo_import,
            Some(id),
            "first activation only asks to confirm"
        );
    });
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    wait_until(cx, "undo", |cx| {
        shell.read(cx).row_count == 0 && shell.read(cx).import_history.is_empty()
    });
}

/// With no imports there is nothing to undo: the action is a no-op.
#[gpui::test]
fn the_undo_action_is_a_no_op_without_history(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(UndoLastImport), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(shell.read(cx).confirm_undo_import, None);
        assert!(toasts(&shell, cx).is_empty());
    });
}

/// Export… and Restore… are wired in M5: the actions (menu items, ⌘E, the
/// page buttons) already exist and say plainly that they are not available.
#[gpui::test]
fn export_and_restore_say_not_available_yet(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
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
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
    with_window(cx, handle, |window, cx| {
        let input = shell.read(cx).search_input.read(cx).focus_handle(cx);
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
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(s.import.is_none());
        let (msg, is_error) = last_toast(&shell, cx).expect("error toast");
        assert!(is_error && msg.contains("not a recognized export"), "{msg}");
    });
    std::fs::remove_dir_all(dir).unwrap();
}

/// 3.27: success and info toasts dismiss themselves after ~4 s. The manager
/// runs on a 100 ms ticker, so the whole life of a toast is entrance (400 ms)
/// + timeout (4 s) + exit (200 ms) of virtual time.
#[gpui::test]
fn info_toasts_dismiss_themselves_after_a_few_seconds(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
    with_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ExportData), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(toasts(&shell, cx).len(), 1, "the toast is up");
    });
    // 3 s in: still mounted, the timeout is 4 s.
    tick_toasts(cx, 30);
    cx.update(|cx| {
        assert_eq!(
            toasts(&shell, cx).len(),
            1,
            "3 s in the toast is still mounted"
        );
    });
    // 2.5 s more: the timeout fires and the exit animation runs out.
    tick_toasts(cx, 25);
    cx.update(|cx| {
        assert!(toasts(&shell, cx).is_empty(), "the toast went away");
    });
}

/// 3.27: an error toast outlives the auto-dismiss and closes when clicked.
#[gpui::test]
fn an_error_toast_stays_until_the_user_closes_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    let dir = std::env::temp_dir().join(format!("financi-ui-err-toast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let odd = dir.join("odd.csv");
    std::fs::write(&odd, "Posted,Payee,Debit\n2026-11-01,X,1.00\n").unwrap();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.open_file(odd.clone(), window, cx));
    });
    cx.run_until_parked();
    let msg = cx.update(|cx| {
        let toasts = toasts(&shell, cx);
        assert_eq!(toasts.len(), 1);
        assert!(toasts[0].1, "the toast is an error");
        toasts[0].0.clone()
    });
    // Ten seconds of ticker: well past the 4 s auto-dismiss, and still there.
    tick_toasts(cx, 100);
    cx.update(|cx| {
        let toasts = toasts(&shell, cx);
        assert_eq!(toasts.len(), 1, "the error toast outlived the timeout");
        assert!(toasts[0].1);
    });
    // A click on the card closes it, once the exit animation runs out.
    draw_frame(cx, handle);
    let mut vtc = VisualTestContext::from_window(handle, cx);
    let card = vtc
        .debug_bounds(leaked(format!("toast:{msg}")))
        .expect("the toast card is painted");
    vtc.simulate_click(card.center(), Modifiers::default());
    cx.run_until_parked();
    tick_toasts(cx, 5);
    cx.update(|cx| {
        assert!(toasts(&shell, cx).is_empty(), "the click dismissed it");
    });
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------------------
// Import end-to-end (task 3.9): run both demo exports through the real
// import panel, then visit every page and check the values shown against
// `fixtures/demo/expected.json`. Runs on the "All time" preset so the
// result never depends on the machine's date.

/// `fixtures/demo/expected.json` — the reference totals, computed outside
/// financi by a Python script.
fn expected_totals() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo/expected.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Drive one demo file through the whole flow: open it, create a new
/// account named `account`, and click Import.
fn import_demo_file(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    file: &str,
    account: &str,
) {
    open_and_choose(cx, handle, shell, file, NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        let p = panel(shell, cx);
        p.update(cx, |p, cx| {
            p.new_name.update(cx, |input, cx| {
                input.set_value(account, window, cx);
            });
            p.create_account(window, cx);
        });
    });
    wait_until(cx, "plan ready after naming the account", |cx| {
        plan_ready(shell, cx)
    });
    click_import(cx, handle, shell);
    wait_until(cx, "import finished", |cx| shell.read(cx).import.is_none());
}

#[gpui::test]
fn import_end_to_end_shows_expected_values_on_every_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });

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
        shell.read(cx).summary.months == ["2026-09", "2026-10", "2026-11"]
            && shell.read(cx).row_count == 109
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
            s.summary
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
        assert!(!s.queue.is_empty(), "the queue has groups");
        assert!(
            s.queue.windows(2).all(|w| w[0].count >= w[1].count),
            "sorted by count descending"
        );
        let rows: i64 = s.queue.iter().map(|g| g.count).sum();
        assert_eq!(
            rows, s.uncategorized_visible,
            "the queue covers every uncategorized row"
        );
        let cents: i64 = s.queue.iter().map(|g| g.total_cents).sum();
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
            s.import_history[0].first_date.as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(s.import_history[0].last_date.as_deref(), Some("2026-11-01"));
        assert_eq!(
            s.import_history[1].first_date.as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(s.import_history[1].last_date.as_deref(), Some("2026-11-06"));
    });
}

/// The import itself must put the data on the Dashboard: the shell
/// refreshes when an import finishes, with no filter change from the user.
/// A missing refresh leaves the Dashboard on its "no data" state while the
/// sidebar already counts the imported rows (task 3.10). The range is
/// pinned to "All time" through the persisted setting — the fixture dates
/// are fixed 2026 dates, so the test passes whenever it runs.
#[gpui::test]
fn the_dashboard_shows_the_imported_totals_without_touching_the_range(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        shell.read(cx).date_range == DateRange::Preset(DatePreset::AllTime)
    });

    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");

    // Nothing else touches the shell: these numbers can only come from the
    // refresh the finished import triggers.
    wait_until(cx, "dashboard shows the import", |cx| {
        shell.read(cx).row_count == 100 && shell.read(cx).total_visible == 102
    });

    // Spend per month from expected.json: the two money-in rows are the
    // card payments, auto-categorized as transfers, so they leave the
    // default view entirely instead of offsetting the spend.
    let expected = expected_totals();
    let want: Vec<i64> = ["2026-09", "2026-10", "2026-11"]
        .iter()
        .map(|m| {
            expected["amex-sep-nov.csv"]["months"][m]["money_out_cents"]
                .as_i64()
                .unwrap()
        })
        .collect();
    let total: i64 = want.iter().sum();
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.summary.months, ["2026-09", "2026-10", "2026-11"]);
        let row = |name: &str| {
            s.summary
                .rows
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("no summary row {name:?}"))
        };
        assert_eq!(row("Spend").months, want);
        assert_eq!(row("Spend").total, total);
        assert_eq!(row("Income").total, 0);
        assert_eq!(row("Net").total, -total);
        assert_eq!(
            s.chart_rows
                .iter()
                .map(|r| (r.name.as_str(), r.spend_cents))
                .collect::<Vec<_>>(),
            [("Uncategorized", total)],
            "the chart shows the imported spend with no further interaction"
        );
    });
}

/// The blank-dashboard bug (3.10): a date range that selects none of the
/// imported rows must show the range/filter empty states — never the
/// whole-window "Drop a CSV" prompt, which belongs to an empty database.
#[gpui::test]
fn a_range_that_excludes_everything_shows_no_matches_not_the_import_prompt(
    cx: &mut TestAppContext,
) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "custom:2020-01-01..2020-12-31").unwrap();
    });
    wait_until(cx, "custom range restored", |cx| {
        matches!(shell.read(cx).date_range, DateRange::Custom { .. })
    });

    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");

    // The 2020 range keeps every 2026 row out of the table, while the
    // unfiltered count still sees them.
    wait_until(cx, "table empty over a full database", |cx| {
        shell.read(cx).total_visible == 102 && shell.read(cx).row_count == 0
    });

    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.dashboard_body(),
            DashboardBody::NoMatches,
            "data exists but the range selects none: the table's empty \
             state shows, not the import prompt"
        );
        assert!(s.chart_rows.is_empty());
        assert!(
            !s.summary_has_data(),
            "the padded all-zero grid must read as empty"
        );
    });
}

/// A fresh database has nothing to show, so the dashboard is the
/// whole-window import prompt (ui-conventions, *States*).
#[gpui::test]
fn an_empty_database_shows_the_import_prompt(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |_c| {});
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(shell.read(cx).dashboard_body(), DashboardBody::NoData);
    });
}

/// "Clear filters" drops the table-narrowing filters — search, the toggles
/// and the category/month selection — and the rows come back (3.10).
#[gpui::test]
fn clear_filters_restores_a_table_emptied_by_search(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        shell.read(cx).date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(cx, "import shows", |cx| shell.read(cx).row_count == 100);

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.search_input
                .update(cx, |i, cx| i.set_value("zzz-no-match", window, cx));
            s.apply_search("zzz-no-match", window, cx);
        });
    });
    wait_until(cx, "search empties the table", |cx| {
        shell.read(cx).row_count == 0 && shell.read(cx).total_visible == 102
    });
    cx.update(|cx| assert_eq!(shell.read(cx).dashboard_body(), DashboardBody::NoMatches));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.clear_filters(window, cx));
    });
    wait_until(cx, "clear filters restores the rows", |cx| {
        shell.read(cx).row_count == 100
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.dashboard_body(), DashboardBody::Rows);
        assert!(s.search.is_empty());
        assert!(
            s.search_input.read(cx).value().is_empty(),
            "the search box clears with the filter"
        );
    });
}

/// 3.35: the dashboard's filter region is one row — no "Filters" title, no
/// label lines (this replaces 3.16's visible-label rule for the dashboard
/// filters). The row never grows past one 32 px control line, with the
/// custom range closed or open.
#[gpui::test]
fn the_dashboard_filter_region_is_one_untitled_row(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    let region = window
        .debug_bounds("filters:region")
        .expect("the filter region paints");
    assert!(
        f32::from(region.size.height) <= 40.,
        "the filter region is {} tall, not a single row",
        f32::from(region.size.height)
    );
    // The region title and every label line are gone.
    for sel in [
        "label:Filters",
        "label:Date range",
        "label:Accounts",
        "label:Search",
    ] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "{sel:?} must not paint any more (3.35 removed the region title \
             and the label lines)"
        );
    }
    // Custom range: the pickers join the row and carry no labels either.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::Custom, window, cx)
        });
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    for sel in ["label:From", "label:To"] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "{sel:?} must not paint (3.35: the pickers name themselves in \
             tooltips, not labels)"
        );
    }
    let region = window
        .debug_bounds("filters:region")
        .expect("the filter region paints with the custom range open");
    assert!(
        f32::from(region.size.height) <= 40.,
        "with the custom range the filter region is {} tall, not a single row",
        f32::from(region.size.height)
    );
}

/// 3.35: every dashboard filter control that lost its visible label names
/// itself in a hover tooltip — Date range, Accounts, Options.
#[gpui::test]
fn the_unlabelled_filter_controls_show_their_names_on_hover(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for (sel, tip) in [
        ("filters:date-range", "filters:tooltip:Date range"),
        ("accounts:filter", "filters:tooltip:Accounts"),
        ("filters:options", "filters:tooltip:Options"),
    ] {
        let b = window
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
        window.simulate_mouse_move(b.center(), None, Modifiers::default());
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        draw_frame(cx, handle);
        assert!(
            window.debug_bounds(tip).is_some(),
            "hovering {sel} shows the tooltip {tip:?}"
        );
        // Move off so no stale tooltip lingers into the next hover.
        window.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::default());
        cx.run_until_parked();
        draw_frame(cx, handle);
    }
}

/// 3.16: the chart and the monthly summary panels name themselves, as the
/// ui-conventions mockups show (with data, so both draw their bodies).
#[gpui::test]
fn the_dashboard_panels_name_themselves(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        shell.read(cx).date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(cx, "dashboard shows the import", |cx| {
        shell.read(cx).row_count == 100 && shell.read(cx).total_visible == 102
    });
    // Opening the file switched to the Imports page; go back.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for sel in ["label:Spending by category", "label:Monthly summary"] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the dashboard panel shows the title {sel:?}"
        );
    }
}

/// 3.25: resize to `w` × `h` and paint, so `debug_bounds` reports the
/// geometry of that size (the alignment pass is tested at the minimum and
/// at a large window).
fn resize_and_draw(cx: &mut TestAppContext, handle: AnyWindowHandle, w: f32, h: f32) {
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(w), px(h)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
}

/// 3.25: the centre-line y of a selector, panicking with its name when the
/// element did not paint at all.
fn centre_y(window: &mut VisualTestContext, sel: &'static str) -> f32 {
    let b = window
        .debug_bounds(sel)
        .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
    f32::from(b.origin.y) + f32::from(b.size.height) / 2.
}

/// 3.25: the dashboard with the demo import, measured at `w` × `h`.
fn open_dashboard_for_geometry(
    cx: &mut TestAppContext,
    w: f32,
    h: f32,
) -> (AnyWindowHandle, Entity<Shell>) {
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "all-time range restored", |cx| {
        shell.read(cx).date_range == DateRange::Preset(DatePreset::AllTime)
    });
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(cx, "dashboard shows the import", |cx| {
        shell.read(cx).row_count == 100 && shell.read(cx).total_visible == 102
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    resize_and_draw(cx, handle, w, h);
    (handle, shell)
}

/// 3.25, 3.35 (dashboard alignment): every control in the single filter
/// row — dropdowns, the search box, Options, the transaction count — sits
/// on one centre line, vertically centred in the 32 px row. The row stays
/// one control line tall (≤ 40 px). At both window sizes.
#[gpui::test]
fn the_dashboard_filter_row_controls_share_one_centre_line(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, _shell) = open_dashboard_for_geometry(cx, w, h);
        let mut window = VisualTestContext::from_window(handle, cx);
        let centre = centre_y(&mut window, "filters:date-range");
        for sel in [
            "accounts:filter",
            "filters:search",
            "filters:options",
            "filters:count",
        ] {
            let cy = centre_y(&mut window, sel);
            assert!(
                (cy - centre).abs() <= 1.,
                "{sel} centres at y {cy}, not on the filter row's centre line {centre} ({w}x{h})"
            );
        }
        let region = window
            .debug_bounds("filters:region")
            .expect("the filter region paints");
        assert!(
            f32::from(region.size.height) <= 40.,
            "the filter region is {} tall, not one 32 px row ({w}x{h})",
            f32::from(region.size.height)
        );
    }
}

/// 3.25 (dashboard alignment): the filter region, the chart panel, the
/// monthly summary and the table share the page's left and right edges
/// exactly; the two middle panels are the same height and their titles sit
/// on one line. At both window sizes.
#[gpui::test]
fn the_dashboard_regions_share_the_page_edges(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, _shell) = open_dashboard_for_geometry(cx, w, h);
        let mut window = VisualTestContext::from_window(handle, cx);
        let left = SIDEBAR_W + PAGE_PAD;
        let right = w - PAGE_PAD;
        for sel in ["filters:region", "panel:chart", "panel:table"] {
            let b = window
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
            assert!(
                (f32::from(b.origin.x) - left).abs() <= 1.,
                "{sel} starts at x {}, not the page's left edge {left} ({w}x{h})",
                f32::from(b.origin.x),
            );
        }
        for sel in ["filters:region", "panel:summary", "panel:table"] {
            let b = window
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} paints on the dashboard"));
            let r = f32::from(b.origin.x) + f32::from(b.size.width);
            assert!(
                (r - right).abs() <= 1.,
                "{sel} ends at x {r}, not the page's right edge {right} ({w}x{h})",
            );
        }
        let chart = window
            .debug_bounds("panel:chart")
            .expect("the chart panel paints");
        let summary = window
            .debug_bounds("panel:summary")
            .expect("the summary panel paints");
        assert!(
            (f32::from(chart.size.height) - f32::from(summary.size.height)).abs() <= 1.,
            "the chart panel is {} tall, the summary {} ({w}x{h})",
            f32::from(chart.size.height),
            f32::from(summary.size.height),
        );
        let chart_title = window
            .debug_bounds("label:Spending by category")
            .expect("the chart title paints");
        let summary_title = window
            .debug_bounds("label:Monthly summary")
            .expect("the summary title paints");
        assert!(
            (f32::from(chart_title.origin.y) - f32::from(summary_title.origin.y)).abs() <= 1.,
            "the panel titles sit at y {} and y {}, not on one line ({w}x{h})",
            f32::from(chart_title.origin.y),
            f32::from(summary_title.origin.y),
        );
    }
}

/// 3.25, 3.35 (dashboard alignment): opening the custom date range adds the
/// From and To pickers without breaking anything — they join the filter
/// row's centre line, every control and the count still paint inside the
/// page (at 1200 × 720 nothing is clipped and nothing wraps), and the
/// region still spans the page exactly. At both window sizes.
#[gpui::test]
fn the_dashboard_stays_aligned_with_a_custom_date_range(cx: &mut TestAppContext) {
    for (w, h) in [(1200., 720.), (1600., 1000.)] {
        let (handle, shell) = open_dashboard_for_geometry(cx, w, h);
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, cx| {
                s.change_date_preset(DatePreset::Custom, window, cx)
            });
        });
        wait_until(cx, "custom range open", |cx| {
            matches!(shell.read(cx).date_range, DateRange::Custom { .. })
        });
        draw_frame(cx, handle);
        let mut window = VisualTestContext::from_window(handle, cx);
        const CONTROLS: [&str; 7] = [
            "filters:date-range",
            "filters:from",
            "filters:to",
            "accounts:filter",
            "filters:search",
            "filters:options",
            "filters:count",
        ];
        let centre = centre_y(&mut window, "filters:date-range");
        for sel in CONTROLS {
            let cy = centre_y(&mut window, sel);
            assert!(
                (cy - centre).abs() <= 1.,
                "{sel} centres at y {cy}, not on the filter row's centre line {centre} (custom range, {w}x{h})"
            );
        }
        // Every control and the count paint inside the page: no control
        // hangs off either edge, and the row does not wrap to a second
        // line (its height stays one control).
        let left = SIDEBAR_W + PAGE_PAD;
        let right = w - PAGE_PAD;
        for sel in CONTROLS {
            let b = window
                .debug_bounds(sel)
                .unwrap_or_else(|| panic!("{sel} paints on the dashboard (custom range, {w}x{h})"));
            assert!(
                f32::from(b.origin.x) >= left - 0.01
                    && f32::from(b.origin.x) + f32::from(b.size.width) <= right + 0.01,
                "{sel} spans x {}–{}, outside the page's {left}–{right} (custom range, {w}x{h})",
                f32::from(b.origin.x),
                f32::from(b.origin.x) + f32::from(b.size.width),
            );
        }
        let b = window
            .debug_bounds("filters:region")
            .expect("the filter region paints");
        assert!(
            f32::from(b.size.height) <= 40.,
            "with the custom range the filter region wraps to {} px, not one row ({w}x{h})",
            f32::from(b.size.height)
        );
        assert!(
            (f32::from(b.origin.x) - left).abs() <= 1.,
            "with the custom range the filter region starts at x {}, not the page's left edge ({w}x{h})",
            f32::from(b.origin.x),
        );
        let r = f32::from(b.origin.x) + f32::from(b.size.width);
        assert!(
            (r - right).abs() <= 1.,
            "with the custom range the filter region ends at x {r}, not the page's right edge {right} ({w}x{h})",
        );
    }
}

/// 3.35: with the filters collapsed to one row, the chart panel starts
/// where the filter rows used to end — at least 80 px higher than the
/// 177.5 px its top measured at 1200 × 720 on the pre-3.35 layout.
#[gpui::test]
fn the_chart_panel_starts_higher_now_the_filters_are_one_row(cx: &mut TestAppContext) {
    const CHART_TOP_BEFORE: f32 = 177.5; // measured at HEAD~ on the 3.25 two-row layout
    let (handle, _shell) = open_dashboard_for_geometry(cx, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let chart = window
        .debug_bounds("panel:chart")
        .expect("the chart panel paints");
    assert!(
        f32::from(chart.origin.y) <= CHART_TOP_BEFORE - 80.,
        "the chart panel's top is at y {}, not at least 80 px above the \
         pre-3.35 y {CHART_TOP_BEFORE}",
        f32::from(chart.origin.y)
    );
}

// -- categories page -------------------------------------------------------

/// Type a parent name, pick the Expense kind (index 0) and add it.
fn add_parent(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>, name: &str) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.new_parent_name.update(cx, |i, cx| {
                i.set_value(name, window, cx);
            });
            s.add_parent(window, cx);
        });
    });
    let what = format!("parent {name}");
    wait_until(cx, &what, |cx| {
        shell.read(cx).categories.iter().any(|p| p.name == name)
    });
}

/// Add a child under the currently selected parent.
fn add_child(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>, name: &str) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.new_child_name.update(cx, |i, cx| {
                i.set_value(name, window, cx);
            });
            s.add_child(window, cx);
        });
    });
    let what = format!("child {name}");
    wait_until(cx, &what, |cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .any(|p| p.children.iter().any(|c| c.name == name))
    });
}

fn find_child(shell: &Entity<Shell>, cx: &App, name: &str) -> Option<i64> {
    shell
        .read(cx)
        .categories
        .iter()
        .find_map(|p| p.children.iter().find(|c| c.name == name).map(|c| c.id))
}

#[gpui::test]
fn creates_parent_and_child(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });

    add_parent(cx, handle, &shell, "Food & Dining");
    cx.update(|cx| {
        let parent = shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food & Dining")
            .unwrap();
        assert_eq!(
            parent.kind,
            CategoryKind::Expense,
            "kind defaults to Expense"
        );
        assert!(parent.children.is_empty());
    });

    add_child(cx, handle, &shell, "Restaurants");
    cx.update(|cx| {
        let parent = shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food & Dining")
            .unwrap();
        assert_eq!(parent.children.len(), 1);
        assert_eq!(parent.children[0].name, "Restaurants");
        assert_eq!(parent.children[0].kind, CategoryKind::Expense);
    });
}

#[gpui::test]
fn creating_a_rule_categorizes_matching_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts", |cx| shell.read(cx).accounts.len() == 1);
    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    wait_until(cx, "import", |cx| shell.read(cx).row_count == 67);

    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let (merchant, rows) = cx.update(|cx| top_group(&shell, cx));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_pattern.update(cx, |i, cx| {
                i.set_value(merchant.clone(), window, cx);
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            s.rule_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.add_rule(window, cx);
        });
    });
    wait_until(cx, "rule added", |cx| !shell.read(cx).rules.is_empty());
    wait_until(cx, "rows categorized", |cx| {
        shell.read(cx).cat_counts.get(&child).copied().unwrap_or(0) >= rows
    });
}

/// The busiest queue group as `(merchant, count)`.
fn top_group(shell: &Entity<Shell>, cx: &App) -> (String, i64) {
    shell
        .read(cx)
        .queue
        .first()
        .map(|g| (g.merchant.clone(), g.count))
        .expect("queue has a group")
}

// -- rule edit & delete (3.24) -------------------------------------------------

/// 3.24 fixture: one account, two child categories and three transactions,
/// plus a rule that puts `COFFEE` rows on `first` (added through
/// `add_rule`, so the two unlocked rows are categorized by it). "COFFEE
/// SHOP LOCKED" is then manually overridden to `second` and locked — a
/// rule re-run must never touch it (2.6). Returns `(rule, first, second)`.
fn seed_rule_fixture(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let first = financi_db::categories::create_child(conn, drinks, "First").unwrap();
    let second = financi_db::categories::create_child(conn, drinks, "Second").unwrap();
    for (n, desc) in ["COFFEE SHOP A", "COFFEE SHOP B", "COFFEE SHOP LOCKED"]
        .iter()
        .enumerate()
    {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
            (acct, import, n as i64, format!("rule-edit-hash-{n}"), *desc),
        )
        .unwrap();
    }
    let rule = financi_db::rules::add_rule(conn, "COFFEE", false, first).unwrap();
    conn.execute(
        "UPDATE transactions SET category_id = ?1, category_locked = 1 \
         WHERE description_raw = 'COFFEE SHOP LOCKED'",
        [second],
    )
    .unwrap();
    (rule, first, second)
}

/// Open the shell over `seed_rule_fixture`, wait until the rule, the two
/// child categories and the counts have loaded, and switch to Categories.
fn open_rule_fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Shell>, i64, i64, i64) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_rule_fixture(c));
    let (rule, first, second) = ids;
    wait_until(cx, "rule fixture loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 1
            && s.child_choices.iter().any(|(id, _)| *id == first)
            && s.child_choices.iter().any(|(id, _)| *id == second)
            && s.cat_counts.get(&first) == Some(&2)
            && s.cat_counts.get(&second) == Some(&1)
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: the page lists only the selected category's rules, so the
    // fixture selects the rules' parent to put them on screen.
    let drinks = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.children.iter().any(|c| c.id == first))
            .map(|p| p.id)
            .unwrap()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(drinks, window, cx));
    });
    cx.run_until_parked();
    (handle, shell, rule, first, second)
}

/// Click a rule row's Edit — the same `start_edit_rule` the button calls.
fn start_rule_edit(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    rule: i64,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.start_edit_rule(rule, window, cx));
    });
}

/// Type into the open editor's Pattern input (the same `set_value` a user's
/// keystrokes end up as; it drives the live preview like a keystroke).
fn set_edit_pattern(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    value: &str,
) {
    let value = value.to_string();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_edit_pattern
                .update(cx, |i, cx| i.set_value(value, window, cx));
            s.preview_rule_match(cx);
        });
    });
}

/// Press the editor's Save — the same `save_rule_edit` the button calls.
fn save_rule_edit(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.save_rule_edit(window, cx));
    });
}

/// 3.24: Edit swaps the row for a pre-filled editor — the rule's pattern,
/// regex kind and category are in the controls, and the editor is open on
/// that rule only.
#[gpui::test]
fn the_rule_editor_is_prefilled_from_the_rule(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, Some(rule), "the editor is open on the rule");
        assert_eq!(s.rule_edit_pattern.read(cx).value().to_string(), "COFFEE");
        assert!(!s.rule_edit_regex, "substring kind pre-filled");
        let sel = s.rule_edit_target.read(cx).selected_index(cx);
        assert_eq!(
            sel.map(|ix| s.child_choices[ix.row].0),
            Some(s.rules[0].category_id),
            "the rule's category is selected in the picker"
        );
    });
    wait_until(cx, "live match count for the edit", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
}

/// 3.24: editing a rule's pattern re-runs every rule over the unlocked
/// rows — the rows the new pattern drops go back to Uncategorized, the
/// ones it keeps stay on the rule's category, and the locked row keeps its
/// manual category even though the new pattern matches it.
#[gpui::test]
fn editing_a_rules_pattern_recategorizes_unlocked_rows(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "SHOP B");
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the edit saved", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule && r.pattern == "SHOP B")
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, None, "the editor closed on save");
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            1,
            "only SHOP B is still on the rule's category"
        );
        assert_eq!(
            s.cat_counts.get(&second).copied().unwrap_or(0),
            1,
            "the locked row keeps its manual category — the new pattern matches it, but locked rows are never re-run"
        );
        assert_eq!(
            s.uncategorized_visible, 1,
            "the dropped row went back to Uncategorized"
        );
    });
}

/// 3.24: changing a rule's category moves the rows it matches onto the new
/// one.
#[gpui::test]
fn changing_a_rules_category_moves_its_rows(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == second)
                .unwrap();
            s.rule_edit_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
        });
    });
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the category change saved", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule && r.category_id == second)
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            0,
            "`first` no longer holds the rule's rows"
        );
        assert_eq!(
            s.cat_counts.get(&second).copied().unwrap_or(0),
            3,
            "both moved rows joined `second` (with the locked row)"
        );
    });
}

/// 3.24: a bad regex keeps the editor open with the error in `danger`, and
/// nothing is saved — the rule and the categorized rows are unchanged.
#[gpui::test]
fn an_invalid_regex_shows_the_error_and_saves_nothing(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "[");
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.rule_edit_regex = true);
    });
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).rule_error.is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, Some(rule), "the editor stays open");
        assert_eq!(s.rules[0].pattern, "COFFEE", "the rule kept its pattern");
        assert!(!s.rules[0].is_regex, "and its kind");
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            2,
            "no rows moved"
        );
    });
}

/// 3.40: pressing Save puts the editor's Save button into its loading,
/// disabled state (`pending_rule_action`) for as long as the DB task runs,
/// and the editor closes once the task ends — the button never looks dead
/// mid-save and can't be double-submitted.
#[gpui::test]
fn saving_a_rule_shows_loading_until_the_task_ends(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "SHOP B");
    // The task hasn't run yet (the test executor only moves inside
    // `wait_until`), so pending is observed while the save is in flight.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.save_rule_edit(window, cx);
            assert_eq!(
                s.pending_rule_action,
                Some(PendingRuleAction::Save(rule)),
                "Save shows loading, disabled, while the save runs"
            );
        });
    });
    wait_until(cx, "the save task to finish", |cx| {
        shell.read(cx).pending_rule_action.is_none()
    });
    wait_until(cx, "the refreshed rule list", |cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .any(|r| r.id == rule && r.pattern == "SHOP B")
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.pending_rule_action, None, "loading cleared");
        assert_eq!(s.editing_rule, None, "the editor closed after the task");
        assert_eq!(s.rule_error, None, "the save succeeded");
    });
}

/// 3.40: when the DB refuses the save (bad regex), the loading state
/// clears on the error path too — the editor stays open with its error,
/// and the button is clickable again.
#[gpui::test]
fn a_refused_rule_save_clears_the_loading_state(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "[");
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.rule_edit_regex = true);
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.save_rule_edit(window, cx);
            assert_eq!(s.pending_rule_action, Some(PendingRuleAction::Save(rule)));
        });
    });
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).pending_rule_action.is_none() && shell.read(cx).rule_error.is_some()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.pending_rule_action, None,
            "loading cleared on the error path too"
        );
        assert_eq!(s.editing_rule, Some(rule), "the editor stays open");
    });
}

/// 3.24 review: a validation error open under the editor must stay inside
/// its row — the virtual list does not clip per item, so an error taller
/// than the reserved slot would paint over the next rule row. The error is
/// clamped to one ellipsized line and the slot grows while it is shown.
#[gpui::test]
fn the_editor_error_stays_inside_its_row(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, second) = open_rule_fixture(cx);
    // A second rule below the one being edited: its row is what an
    // overflowing error would overlap.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_pattern
                .update(cx, |i, cx| i.set_value("TOAST".to_string(), window, cx));
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == second)
                .unwrap();
            s.rule_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.add_rule(window, cx);
        });
    });
    wait_until(cx, "the second rule", |cx| shell.read(cx).rules.len() == 2);
    let toast = cx.update(|cx| {
        shell
            .read(cx)
            .rules
            .iter()
            .find(|r| r.pattern == "TOAST")
            .unwrap()
            .id
    });

    // Edit the first rule and leave an invalid regex open — the multi-line
    // regex-crate message is the one that used to break the layout.
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "[");
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.rule_edit_regex = true);
    });
    save_rule_edit(cx, handle, &shell);
    wait_until(cx, "the editor error", |cx| {
        shell.read(cx).rule_error.is_some()
    });
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    let err = window
        .debug_bounds("rule-edit-error")
        .expect("the error paints under the editor");
    let next = window
        .debug_bounds(format!("rule-row-{toast}").leak())
        .expect("the row below still paints");
    assert!(
        err.bottom() <= next.top(),
        "the error (bottom {:?}) must not overlap the next rule row (top {:?})",
        err.bottom(),
        next.top()
    );
    assert!(
        err.size.height <= px(24.),
        "the error is one line (text_sm measures 22.5 px), not {:?}",
        err.size.height
    );
}

/// 3.24 review: while a row's editor is open the live count belongs to the
/// editor — the add-rule row must not show the editor's count (the same
/// filter `rule_error` has).
#[gpui::test]
fn the_add_rule_preview_stays_hidden_while_editing(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    wait_until(cx, "the editor preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("rule-edit-preview").is_some(),
        "the editor shows the live count"
    );
    assert!(
        window.debug_bounds("add-rule-preview").is_none(),
        "the add-rule row shows none of it"
    );
    // Not merely never painted: with the editor closed and the add-rule
    // row's own pattern, the row shows its own count.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.cancel_edit_rule(cx);
            s.rule_pattern
                .update(cx, |i, cx| i.set_value("COFFEE".to_string(), window, cx));
            s.preview_rule_match(cx);
        });
    });
    wait_until(cx, "the add-rule preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("add-rule-preview").is_some(),
        "the add-rule row shows its own count once not editing"
    );
}

/// 3.24: Cancel and Escape both close the editor without touching the
/// rule.
#[gpui::test]
fn cancel_and_escape_leave_the_rule_unchanged(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);

    // Cancel button.
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "ZZZ");
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.cancel_edit_rule(cx);
            let _ = window;
        });
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, None, "Cancel closed the editor");
        assert_eq!(s.rules[0].pattern, "COFFEE", "Cancel saved nothing");
    });

    // Escape: the window-level binding cancels the innermost thing first.
    start_rule_edit(cx, handle, &shell, rule);
    set_edit_pattern(cx, handle, &shell, "ZZZ");
    with_window(cx, handle, |window, cx| {
        let input = shell.read(cx).rule_edit_pattern.read(cx).focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "escape");
    wait_until(cx, "Escape closed the editor", |cx| {
        shell.read(cx).editing_rule.is_none()
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.rules[0].pattern, "COFFEE", "Escape saved nothing");
    });
}

/// 3.24: selecting another category cancels the open editor — and clears
/// the shared preview count with it, so the add-rule row never inherits a
/// stale editor count.
#[gpui::test]
fn selecting_another_category_cancels_the_rule_editor(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    wait_until(cx, "the editor preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.selected_category, Some(first));
        assert_eq!(
            s.editing_rule, None,
            "selecting a category cancels the edit"
        );
        assert!(
            s.rule_match_count.is_none(),
            "closing the editor clears the shared preview count"
        );
        assert_eq!(s.rules[0].pattern, "COFFEE");
    });
}

/// 3.24: Delete… is the two-click confirm — the first click arms the row,
/// the second deletes; the rule is gone and its rows fall back to
/// Uncategorized (the locked row keeps its manual category).
#[gpui::test]
fn delete_rule_needs_two_clicks_and_uncategorizes_its_rows(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, second) = open_rule_fixture(cx);

    // First click: arms, nothing is deleted.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
        assert_eq!(
            shell.read(cx).armed_delete_rule,
            Some(rule),
            "the first click arms the row"
        );
        assert_eq!(shell.read(cx).rules.len(), 1, "and deletes nothing yet");
    });

    // Second click: deletes.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
    });
    wait_until(cx, "the rule deleted", |cx| shell.read(cx).rules.is_empty());
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.armed_delete_rule, None, "the armed state reset");
        assert_eq!(
            s.cat_counts.get(&first).copied().unwrap_or(0),
            0,
            "the rule's rows fell back to Uncategorized"
        );
        assert_eq!(s.uncategorized_visible, 2);
        assert_eq!(
            s.cat_counts.get(&second).copied().unwrap_or(0),
            1,
            "the locked row kept its manual category"
        );
    });
}

/// 3.24: the armed delete is transient like the page's other two-click
/// confirmations — selecting a category disarms it, and so does switching
/// pages (which also closes an open rule editor).
#[gpui::test]
fn the_armed_rule_delete_is_transient(cx: &mut TestAppContext) {
    let (handle, shell, rule, first, _second) = open_rule_fixture(cx);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
        assert_eq!(shell.read(cx).armed_delete_rule, Some(rule));
        shell.update(cx, |s, cx| s.select_category(first, window, cx));
        assert_eq!(
            shell.read(cx).armed_delete_rule,
            None,
            "selecting a category disarms the delete"
        );
    });

    // A page switch closes the editor (and its preview count) and disarms
    // in one go.
    start_rule_edit(cx, handle, &shell, rule);
    wait_until(cx, "the editor preview", |cx| {
        shell.read(cx).rule_match_count.is_some()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_rule(rule, window, cx));
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, None, "the page switch closed the editor");
        assert!(
            s.rule_match_count.is_none(),
            "and cleared the shared preview count"
        );
        assert_eq!(s.armed_delete_rule, None, "and disarmed the delete");
    });
}

/// 3.24: with thousands of rules the row actions (Edit, Delete…) are built
/// only for the rows the virtualized list renders — buttons come into the
/// world inside `rule_row`, so the built-row window proves it. Opening an
/// editor on a visible row must not change that.
#[gpui::test]
fn rule_row_actions_are_built_only_for_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2000 rules", |cx| {
        shell.read(cx).rules.len() == 2_000 && shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: rules paint only under their selected category; all 2,000 of
    // them sit under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    let first_rule = cx.update(|cx| shell.read(cx).rules[0].id);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.rule_rows_built.get() > 0 && s.rule_rows_built.get() < 100,
            "the rule list built {} rows of 2000 — buttons come with each row, so it must render a window",
            s.rule_rows_built.get()
        );
    });
    // The editor swaps one row for a taller block; the window stays small.
    start_rule_edit(cx, handle, &shell, first_rule);
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.editing_rule, Some(first_rule));
        assert!(
            s.rule_rows_built.get() < 100,
            "still a window of rows with the editor open: {}",
            s.rule_rows_built.get()
        );
    });
}

#[gpui::test]
fn categorizing_a_queue_group_locks_in_a_rule(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts", |cx| shell.read(cx).accounts.len() == 1);
    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    wait_until(cx, "import", |cx| shell.read(cx).row_count == 67);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let child = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());

    wait_until(cx, "queue populated", |cx| !shell.read(cx).queue.is_empty());
    let (merchant, before) = cx.update(|cx| top_group(&shell, cx));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            // 3.19: pickers are built for visible rows; `queue_picker` is
            // the same get-or-create the row's render makes.
            let picker = s.queue_picker(&merchant, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.categorize_merchant(merchant.clone(), window, cx);
        });
    });
    let gone = merchant.clone();
    let shell_for_gone = shell.clone();
    wait_until(cx, "queue group gone", move |cx| {
        !shell_for_gone
            .read(cx)
            .queue
            .iter()
            .any(|g| g.merchant == gone)
    });
    let online = cx.update(|cx| *shell.read(cx).cat_counts.get(&child).unwrap());
    assert!(online >= before, "{online} >= {before} categorized");
    cx.update(|cx| {
        assert!(
            shell.read(cx).rules.iter().any(|r| r.pattern == merchant),
            "a rule was created for the merchant"
        );
    });
}

/// 3.19: synthetic queue data — `groups` merchant groups of two uncategorized
/// transactions each, plus one parent/child pair to categorize onto.
fn seed_queue_thousands(conn: &financi_db::rusqlite::Connection, groups: usize) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    for i in 0..groups {
        for n in 0..2 {
            conn.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents, description_raw)
                 VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
                (
                    acct,
                    import,
                    (i * 2 + n) as i64,
                    format!("queue-hash-{i}-{n}"),
                    format!("QUEUE MERCHANT {i:04}"),
                ),
            )
            .unwrap();
        }
    }
}

/// 3.19: with thousands of merchant groups the To-categorize page must still
/// render, building only the visible rows — and only pickers for those.
#[gpui::test]
fn the_queue_page_builds_only_the_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| {
        shell.read(cx).queue.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.queue_rows_built.get() > 0 && s.queue_rows_built.get() < 100,
            "the queue built {} rows of 2000 — it must render a window",
            s.queue_rows_built.get()
        );
        // 3.19: pickers come with the rows that rendered, not one per group.
        assert!(
            s.queue_pickers.len() < 100,
            "{} pickers for a {}-row window — pickers must be built per visible row",
            s.queue_pickers.len(),
            s.queue_rows_built.get()
        );
        assert!(
            s.queue_pickers.contains_key("QUEUE MERCHANT 0000"),
            "the first visible row has a picker"
        );
        assert!(
            !s.queue_pickers.contains_key("QUEUE MERCHANT 1999"),
            "the last group is off-screen, so it must not have a picker"
        );
    });
}

/// 3.19: categorizing still works with a long queue — drive the same
/// `categorize_merchant` call the row's button makes for the group at the
/// very bottom, and its rows leave the queue behind a rule.
#[gpui::test]
fn a_queue_group_at_the_very_bottom_can_still_be_categorized(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| {
        shell.read(cx).queue.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    let last = cx.update(|cx| {
        let s = shell.read(cx);
        s.queue.last().expect("queue has a bottom").merchant.clone()
    });
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let picker = s.queue_picker(&last, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.categorize_merchant(last.clone(), window, cx);
        });
    });
    let gone = last.clone();
    let shell_for_gone = shell.clone();
    wait_until(cx, "bottom queue group gone", move |cx| {
        !shell_for_gone
            .read(cx)
            .queue
            .iter()
            .any(|g| g.merchant == gone)
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.cat_counts.get(&child).copied().unwrap_or(0),
            2,
            "both rows of the bottom group are categorized"
        );
        assert!(
            s.rules.iter().any(|r| r.pattern == last),
            "a rule was created for the bottom merchant"
        );
        assert!(
            s.queue_rows_built.get() < 100,
            "still windowed after categorizing"
        );
    });
}

/// 3.21: the To-categorize queue keeps its scroll offset across frames and
/// across a plain refresh, so the bottom of a thousands-group queue stays
/// reachable once scrolled to (the same bug as the tree: a per-render
/// scroll handle reset the offset to 0 every frame).
#[gpui::test]
fn the_queue_keeps_its_scroll_offset(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 2_000));
    wait_until(cx, "2000 queue groups", |cx| {
        shell.read(cx).queue.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.queue_scroll.offset(),
            Point::default(),
            "starts unscrolled"
        );
        assert_eq!(s.queue_range.get().0, 0, "the first frame shows the top");
        // Jump straight to the bottom; layout clamps to the scroll max.
        s.queue_scroll.set_offset(point(px(0.), px(-1_000_000.)));
    });
    draw_frame(cx, handle);
    let scrolled = cx.update(|cx| shell.read(cx).queue_scroll.offset().y);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(scrolled < px(0.), "the queue is scrolled down");
        let (start, end) = s.queue_range.get();
        assert!(
            start > 0,
            "the built window moved off the top (start {start})"
        );
        assert_eq!(
            end,
            s.queue.len(),
            "the built window reached the last group"
        );
        // Pickers come with the rows that rendered: the bottom group's
        // picker is there, a middle group that was never on screen isn't.
        assert!(
            s.queue_pickers.contains_key("QUEUE MERCHANT 1999"),
            "the bottom group is on screen, so it has a picker"
        );
        assert!(
            !s.queue_pickers.contains_key("QUEUE MERCHANT 1000"),
            "a middle group never scrolled into view, so it must not have a picker"
        );
    });
    // A refresh that doesn't replace the queue must not move the offset.
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.queue_scroll.offset().y,
            scrolled,
            "the offset survived an unrelated notify"
        );
        assert_eq!(s.queue_range.get().1, s.queue.len());
    });
}

/// 3.27: categorizing a queue group answers with a toast, and the toast is an
/// overlay — the top of the page paints in exactly the same place with a card
/// showing, where the old inline status line used to shift everything down.
#[gpui::test]
fn categorizing_a_queue_group_toasts_without_moving_the_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_queue_thousands(conn, 4));
    wait_until(cx, "4 queue groups", |cx| shell.read(cx).queue.len() == 4);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    draw_frame(cx, handle);
    let mut vtc = VisualTestContext::from_window(handle, cx);
    let top = vtc
        .debug_bounds(leaked("queue:merchant-col:QUEUE MERCHANT 0000".to_string()))
        .expect("the first queue row is painted");
    let child = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            let merchant = "QUEUE MERCHANT 0001".to_string();
            let picker = s.queue_picker(&merchant, window, cx);
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == child)
                .unwrap();
            picker.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.categorize_merchant(merchant, window, cx);
        });
    });
    wait_until(cx, "toast up and queue shrinks", |cx| {
        !shell.read(cx).toasts.is_empty() && shell.read(cx).queue.len() == 3
    });
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Categorized QUEUE MERCHANT 0001".into(), false))
        );
    });
    draw_frame(cx, handle);
    let after = vtc
        .debug_bounds(leaked("queue:merchant-col:QUEUE MERCHANT 0000".to_string()))
        .expect("the first queue row is still painted");
    assert_eq!(
        after.origin, top.origin,
        "the page did not move while the toast shows"
    );
    let card = vtc
        .debug_bounds(leaked("toast:Categorized QUEUE MERCHANT 0001".to_string()))
        .expect("the toast card is painted");
    assert!(
        card.center().x > px(550.) && card.center().y > px(360.),
        "the toast sits in the bottom-right of the 1100 × 720 window, at {card:?}"
    );
}

/// 3.16: every queue column, the category picker included, has a visible
/// heading above the rows — the picker's placeholder alone is not a label.
#[gpui::test]
fn the_queue_page_labels_its_columns(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    import_and_top_merchant(cx, handle, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for sel in [
        "label:Merchant",
        "label:Transactions",
        "label:Total",
        "label:Dates",
        "label:Category",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the queue paints the column heading {sel:?}"
        );
    }
}

/// The heading row reserves the same tail as the rows (picker + Categorize
/// button) and shares the QUEUE_*_W column widths, so every heading sits
/// over its own column — measured at the 1200 × 720 minimum window, where
/// a row just fits (3.16, extended for 3.22).
#[gpui::test]
fn the_queue_category_heading_sits_over_the_picker_column(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (merchant, _, _) = import_and_top_merchant(cx, handle, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for (label, sel) in [
        ("Merchant", format!("queue:merchant-col:{merchant}")),
        ("Transactions", format!("queue:txns-col:{merchant}")),
        ("Total", format!("queue:total-col:{merchant}")),
        ("Dates", format!("queue:dates-col:{merchant}")),
        ("Category", format!("queue:picker-col:{merchant}")),
    ] {
        let heading = window
            .debug_bounds(leaked(format!("label:{label}")))
            .unwrap_or_else(|| panic!("the queue paints the {label:?} heading"));
        let column = window
            .debug_bounds(leaked(sel))
            .unwrap_or_else(|| panic!("the queue paints the {label:?} column"));
        assert!(
            (heading.origin.x - column.origin.x).abs() < px(1.),
            "the {label:?} heading starts where its column starts: \
             heading x {}, column x {}",
            heading.origin.x,
            column.origin.x
        );
    }
    let heading = window
        .debug_bounds("label:Category")
        .expect("the queue paints a Category heading");
    let picker = window
        .debug_bounds(leaked(format!("queue:picker-col:{merchant}")))
        .expect("the queue paints a picker column");
    assert!(
        (heading.size.width - picker.size.width).abs() < px(1.),
        "the Category heading is as wide as the picker column: \
         heading w {}, picker w {}",
        heading.size.width,
        picker.size.width
    );
    assert!(
        heading.origin.y < picker.origin.y,
        "the heading is above the rows: heading y {}, picker y {}",
        heading.origin.y,
        picker.origin.y
    );
}

/// The queue rows for the 1200 × 720 fit test: the first merchant's name is
/// exactly 120 characters, the others are ordinary ones.
const QUEUE_FIT_MERCHANTS: [&str; 3] = [
    "LONG MERCHANT NAME THAT WILL NOT FIT THE QUEUE COLUMN AT MINIMUM WINDOW SIZE SO IT MUST TRUNCATE AND SHOW FULL TEXT IN A",
    "CORNER CAFE",
    "TRANSIT FARE",
];

/// One uncategorized group per name in [`QUEUE_FIT_MERCHANTS`].
fn seed_queue_fit(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    for (i, name) in QUEUE_FIT_MERCHANTS.iter().enumerate() {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -123456, ?5)",
            (
                acct,
                import,
                i as i64,
                format!("fit-hash-{i}"),
                name.to_string(),
            ),
        )
        .unwrap();
    }
}

/// 3.22: at the 1200 × 720 minimum window every queue row keeps its category
/// picker and Categorize button fully inside the page; a 120-character
/// merchant name truncates with its full text in a hover tooltip, and the
/// Transactions cell shows just the number.
#[gpui::test]
fn the_queue_fits_the_minimum_window(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_queue_fit);
    wait_until(cx, "3 queue groups", |cx| shell.read(cx).queue.len() == 3);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    cx.run_until_parked();
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // The row's right edge — its Categorize button — ends inside the page
    // for every row, and so does the picker next to it.
    let page_right = px(1200. - PAGE_PAD);
    for m in QUEUE_FIT_MERCHANTS {
        for (what, sel) in [
            ("category picker", format!("queue:picker-col:{m}")),
            ("Categorize button", format!("queue:categorize-col:{m}")),
        ] {
            let b = window
                .debug_bounds(leaked(sel))
                .unwrap_or_else(|| panic!("the row for {m:?} draws its {what}"));
            assert!(
                b.origin.x + b.size.width <= page_right + px(1.),
                "the {what} of {m:?} ends at x {:?}, past the page edge {page_right:?}",
                b.origin.x + b.size.width,
            );
        }
    }

    // The 120-character name truncates: the Merchant cell is far narrower
    // than the full text needs at 14 px.
    let long = QUEUE_FIT_MERCHANTS[0];
    let cell = window
        .debug_bounds(leaked(format!("queue:merchant-col:{long}")))
        .expect("the long merchant's row draws its cell");
    let full_w = TEXT_SM_CHAR_PX * long.chars().count() as f32;
    assert!(
        cell.size.width < px(full_w / 2.),
        "the merchant name truncates: the cell is {:?} wide, the full text \
         needs about {full_w:?}",
        cell.size.width,
    );

    // The Transactions cell shows the bare number, not "N transactions".
    assert!(
        window
            .debug_bounds(leaked(format!("queue:txns-text:{long}=1")))
            .is_some(),
        "the Transactions cell shows the bare count"
    );
    assert!(
        window
            .debug_bounds(leaked(format!("queue:txns-text:{long}=1 transactions")))
            .is_none(),
        "the old \"N transactions\" text is gone"
    );

    // Hovering the truncated name shows the tooltip with the full text.
    assert!(
        window
            .debug_bounds(leaked(format!("queue:merchant-tooltip:{long}")))
            .is_none(),
        "nothing hovers the row, so no tooltip is drawn yet"
    );
    let centre = point(
        cell.origin.x + cell.size.width / 2.,
        cell.origin.y + cell.size.height / 2.,
    );
    window.simulate_mouse_move(centre, None, Modifiers::default());
    // The tooltip appears after a 500 ms hover; the test clock is fake.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds(leaked(format!("queue:merchant-tooltip:{long}")))
            .is_some(),
        "hovering the truncated merchant name shows a tooltip with the full name",
    );
}

#[gpui::test]
fn delete_category_reassigns_transactions_and_rules(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "accounts", |cx| shell.read(cx).accounts.len() == 1);
    open_and_choose(
        cx,
        handle,
        &shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(&shell, cx));
    click_import(cx, handle, &shell);
    wait_until(cx, "import", |cx| shell.read(cx).row_count == 67);
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    add_parent(cx, handle, &shell, "Everything Else"); // second parent to reassign onto
    add_child(cx, handle, &shell, "Keep");
    let keep = cx.update(|cx| find_child(&shell, cx, "Keep").unwrap());
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let (merchant, _) = cx.update(|cx| top_group(&shell, cx));

    // Categorize the top merchant's rows onto Online, so a delete has work to move.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_pattern.update(cx, |i, cx| {
                i.set_value(merchant.clone(), window, cx);
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == online)
                .unwrap();
            s.rule_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.add_rule(window, cx);
        });
    });
    wait_until(cx, "rows on Online", |cx| {
        shell.read(cx).cat_counts.get(&online).copied().unwrap_or(0) > 0
    });

    // Select Online, arm the delete, then confirm reassigning to Keep.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(online, window, cx);
        });
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
        assert_eq!(
            shell.read(cx).confirm_delete_category,
            Some(online),
            "first click arms"
        );
        let keep_ix = s_ix(&shell, cx, keep);
        s_select(&shell, cx, keep_ix, window);
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
    });
    wait_until(cx, "Online deleted", |cx| {
        find_child(&shell, cx, "Online").is_none()
    });
    let keep_count = cx.update(|cx| *shell.read(cx).cat_counts.get(&keep).unwrap());
    assert!(keep_count > 0, "transactions moved to Keep");
    cx.update(|cx| {
        assert!(
            shell.read(cx).rules.iter().all(|r| r.category_id == keep),
            "the rule followed the reassignment"
        );
    });
}

/// The index of `child` in the shell's child_choices.
fn s_ix(shell: &Entity<Shell>, cx: &App, child: i64) -> usize {
    shell
        .read(cx)
        .child_choices
        .iter()
        .position(|(id, _)| *id == child)
        .unwrap()
}

fn s_select(shell: &Entity<Shell>, cx: &mut App, ix: usize, window: &mut Window) {
    let target = shell.read(cx).delete_target.clone();
    target.update(cx, |sel, cx| {
        sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx)
    });
}

/// 3.12: synthetic data for the long-list tests — 2,000 parents with one
/// child each (4,000 tree rows) and 2,000 rules, inserted directly.
fn seed_thousands(conn: &financi_db::rusqlite::Connection) {
    // 3.23: the page lists only the selected category's rules, so the
    // long-list tests need them all under one: every rule targets the
    // first subcategory, under "Cat 0000".
    let mut first_child = 0i64;
    for i in 0..2_000 {
        conn.execute(
            "INSERT INTO categories (name, kind) VALUES (?1, 'expense')",
            financi_db::rusqlite::params![format!("Cat {i:04}")],
        )
        .unwrap();
        let parent = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO categories (name, parent_id, kind) VALUES (?1, ?2, 'expense')",
            financi_db::rusqlite::params![format!("Kid {i:04}"), parent],
        )
        .unwrap();
        let child = conn.last_insert_rowid();
        if i == 0 {
            first_child = child;
        }
        conn.execute(
            "INSERT INTO category_rules (pattern, category_id) VALUES (?1, ?2)",
            financi_db::rusqlite::params![format!("PAT{i:04}"), first_child],
        )
        .unwrap();
    }
}

/// 3.23: select a top-level category by name — the same `select_category`
/// call `tree_row` makes. The page's rule list only paints with a
/// selection, so most Categories tests start here.
fn select_parent(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    name: &str,
) {
    let id = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no parent category {name:?}"))
            .id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(id, window, cx));
    });
    cx.run_until_parked();
}

/// Draw one real frame so the virtualized lists lay out and build rows.
fn draw_frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    with_window(cx, handle, |window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// 3.21: wheel `dy` pixels over a point — `v_virtual_list` scrolls under
/// the mouse, so tests aim at the centre of the list handle's viewport.
fn wheel_over(cx: &mut TestAppContext, handle: AnyWindowHandle, at: Point<Pixels>, dy: f32) {
    let mut window = VisualTestContext::from_window(handle, cx);
    window.simulate_event(ScrollEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        ..Default::default()
    });
}

/// The centre of a window-space rectangle's viewport, to aim `wheel_over`.
fn viewport_centre(b: Bounds<Pixels>) -> Point<Pixels> {
    point(
        b.origin.x + b.size.width / 2.0,
        b.origin.y + b.size.height / 2.0,
    )
}

/// 3.12: with thousands of categories and rules the Categories page must
/// still render, and the virtualized lists must build only the visible
/// rows, not all 4,000 / 2,000.
#[gpui::test]
fn the_categories_page_builds_only_the_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    // +1: the system `Transfers` parent from migration 0002.
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: the rule list paints only for the selected category, and all
    // 2,000 seeded rules sit under "Cat 0000".
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.rules.len(), 2_000, "the rule list is full too");
        assert!(
            s.tree_rows_built.get() > 0 && s.tree_rows_built.get() < 100,
            "the tree built {} rows of 4000 — it must render a window",
            s.tree_rows_built.get()
        );
        assert!(
            s.rule_rows_built.get() > 0 && s.rule_rows_built.get() < 100,
            "the rule list built {} rows of 2000 — it must render a window",
            s.rule_rows_built.get()
        );
    });
}

/// 3.12: selection and the row click handler keep working with a long
/// tree: clicking (the same `select_category` call `tree_row` makes) a
/// category at the very bottom selects it, and the page re-renders.
#[gpui::test]
fn clicking_a_category_still_works_with_a_long_tree(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    draw_frame(cx, handle);
    let deep = cx.update(|cx| {
        let s = shell.read(cx);
        s.categories[1_999].children[0].id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(deep, window, cx));
    });
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.selected_category,
            Some(deep),
            "the last child of the last parent selected"
        );
        assert!(
            s.tree_rows_built.get() < 100,
            "still windowed after selecting"
        );
    });
}

/// 3.21: wheel-scrolling the category tree must stick. `v_virtual_list`
/// created a fresh `VirtualListScrollHandle` per render and `track_scroll`ed
/// it, and GPUI prefers a tracked handle over element state — so the offset
/// snapped back to 0 every frame and rows past the first screen were
/// unreachable. With the handle a `Shell` field, the built window of rows
/// must follow the wheel to the bottom and survive an unrelated refresh.
#[gpui::test]
fn the_category_tree_keeps_its_scroll_offset(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.tree_scroll.offset(),
            Point::default(),
            "starts unscrolled"
        );
        assert_eq!(s.tree_range.get().0, 0, "the first frame shows the top");
    });
    // Wheel down to the bottom (content is ~36 px a row for thousands of
    // rows); the offset clamps at the last row.
    for _ in 0..120 {
        let centre = cx.update(|cx| viewport_centre(shell.read(cx).tree_scroll.bounds()));
        wheel_over(cx, handle, centre, -2_000.);
        draw_frame(cx, handle);
    }
    cx.update(|cx| {
        let s = shell.read(cx);
        let rows: usize = s.categories.iter().map(|p| 1 + p.children.len()).sum();
        assert!(
            s.tree_scroll.offset().y < px(0.),
            "the tree is scrolled down"
        );
        let (start, end) = s.tree_range.get();
        assert!(
            start > 0,
            "the built window moved off the top (start {start})"
        );
        assert_eq!(end, rows, "the built window reached the last row");
        assert!(
            s.tree_rows_built.get() < 100,
            "still windowed while scrolled to the bottom"
        );
    });
    // An unrelated refresh must not scroll the tree back to the top.
    let scrolled = cx.update(|cx| shell.read(cx).tree_scroll.offset().y);
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.tree_scroll.offset().y,
            scrolled,
            "the offset survived an unrelated notify"
        );
        assert!(s.tree_range.get().0 > 0, "still showing rows past the top");
    });
}

/// 3.21: the Categories-page rule list keeps its scroll offset across
/// frames and across a plain refresh — only a content replacement (3.23:
/// the selected category changes) may reset it.
#[gpui::test]
fn the_rule_list_keeps_its_scroll_offset(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2000 rules", |cx| {
        shell.read(cx).rules.len() == 2_000 && shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // 3.23: the list shows the selected category's rules; all 2,000 of
    // them sit under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rule_scroll.offset(),
            Point::default(),
            "starts unscrolled"
        );
        assert_eq!(s.rule_range.get().0, 0, "the first frame shows the top");
        // Jump straight to the bottom; layout clamps to the scroll max.
        s.rule_scroll.set_offset(point(px(0.), px(-1_000_000.)));
    });
    draw_frame(cx, handle);
    let scrolled = cx.update(|cx| shell.read(cx).rule_scroll.offset().y);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(scrolled < px(0.), "the rule list is scrolled down");
        let (start, end) = s.rule_range.get();
        assert!(
            start > 0,
            "the built window moved off the top (start {start})"
        );
        assert_eq!(end, s.rules.len(), "the built window reached the last rule");
    });
    // A refresh that doesn't replace the rules must not move the offset.
    cx.update(|cx| shell.update(cx, |_, cx| cx.notify()));
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rule_scroll.offset().y,
            scrolled,
            "the offset survived an unrelated notify"
        );
        assert_eq!(s.rule_range.get().1, s.rules.len());
    });
    // 3.23: changing the selection replaces the list's content, and the
    // offset resets to the top — the one reset 3.21 allowed.
    select_parent(cx, handle, &shell, "Cat 0001");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rule_scroll.offset(),
            Point::default(),
            "the selection change reset the rule list's scroll"
        );
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    assert!(
        window.debug_bounds("cat:rules-empty").is_some(),
        "a category without rules shows the empty note"
    );
}

// -- Rules page (3.28) -------------------------------------------------------

/// 3.28 fixture: rules and rows for the Rules page tests. Categories
/// Food › Coffee, Food › Snacks, Fun › Games. Rules:
/// - r1 "COFFEE" (substring) → Coffee: wins the two roast rows (Uses 2);
/// - r2 "COFFEE SHOP" (substring) → Coffee: wins the shop row (Uses 1);
/// - r3 "GAME" (substring) → Games: nothing matches (unused);
/// - r4 "^SNACK" (regex) → Snacks: unused.
///   "COFFEE SHOP LOCKED" is overridden to Games and locked — no rule
///   re-run may touch it — and "PARKING FEE" stays uncategorised. Returns
///   `(coffee, snacks, games, [r1, r2, r3, r4])`.
fn seed_rules_page(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64, [i64; 4]) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let coffee = financi_db::categories::create_child(conn, food, "Coffee").unwrap();
    let snacks = financi_db::categories::create_child(conn, food, "Snacks").unwrap();
    let fun = financi_db::categories::create_parent(conn, "Fun", CategoryKind::Expense).unwrap();
    let games = financi_db::categories::create_child(conn, fun, "Games").unwrap();
    for (n, desc) in [
        "COFFEE BEAN ROAST 1",
        "COFFEE BEAN ROAST 2",
        "COFFEE SHOP A",
        "COFFEE SHOP LOCKED",
        "PARKING FEE",
    ]
    .iter()
    .enumerate()
    {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-01', -1000, ?5)",
            (
                acct,
                import,
                n as i64,
                format!("rules-page-hash-{n}"),
                *desc,
            ),
        )
        .unwrap();
    }
    let r1 = financi_db::rules::add_rule(conn, "COFFEE", false, coffee).unwrap();
    let r2 = financi_db::rules::add_rule(conn, "COFFEE SHOP", false, coffee).unwrap();
    let r3 = financi_db::rules::add_rule(conn, "GAME", false, games).unwrap();
    let r4 = financi_db::rules::add_rule(conn, "^SNACK", true, snacks).unwrap();
    conn.execute(
        "UPDATE transactions SET category_id = ?1, category_locked = 1 \
         WHERE description_raw = 'COFFEE SHOP LOCKED'",
        [games],
    )
    .unwrap();
    (coffee, snacks, games, [r1, r2, r3, r4])
}

/// Open the shell over `seed_rules_page`, wait for the rules and their Uses
/// counts to load, and switch to the Rules page.
fn open_rules_page(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Shell>, i64, i64, i64, [i64; 4]) {
    let mut ids = (0i64, 0i64, 0i64, [0i64; 4]);
    let (handle, shell) = open_shell(cx, |c| ids = seed_rules_page(c));
    let (coffee, snacks, games, rules) = ids;
    wait_until(cx, "rules page fixture loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 4
            && s.rule_uses.get(&rules[0]) == Some(&2)
            && s.rule_uses.get(&rules[1]) == Some(&1)
            && s.cat_counts.get(&coffee) == Some(&3)
            && s.cat_counts.get(&games) == Some(&1)
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.run_until_parked();
    (handle, shell, coffee, snacks, games, rules)
}

/// Type into the Rules page search box (the same `set_value` a user's
/// keystrokes end up as).
fn set_rules_search(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    value: &str,
) {
    let value = value.to_string();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rules_search
                .update(cx, |i, cx| i.set_value(value, window, cx));
        });
    });
    cx.run_until_parked();
}

/// Does a widget with this debug selector paint in the last frame?
fn paints(window: &mut VisualTestContext, sel: String) -> bool {
    window.debug_bounds(leaked(sel)).is_some()
}

/// The y of a painted selector's top edge, panicking with a Rules-page
/// message when it is not painted.
fn rules_sel_y(window: &mut VisualTestContext, sel: &str) -> f32 {
    let b = window
        .debug_bounds(leaked(sel.to_string()))
        .unwrap_or_else(|| panic!("{sel} paints on the Rules page"));
    f32::from(b.origin.y)
}

/// 3.28: the list groups rules by their subcategory with a header row per
/// group ("Food › Coffee · 3 rules"), groups in label order, and rows
/// within a group by Pattern (the default).
#[gpui::test]
fn rules_page_groups_rows_by_target_category(cx: &mut TestAppContext) {
    let (handle, _shell, coffee, snacks, games, rules) = open_rules_page(cx);
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    assert!(
        paints(&mut window, "label:4 rules".into()),
        "the header counts the rules"
    );
    for id in [coffee, snacks, games] {
        assert!(
            paints(&mut window, format!("rules:group-{id}")),
            "a group header for category {id}"
        );
    }
    for id in rules {
        assert!(
            paints(&mut window, format!("rules:row-{id}")),
            "a row for rule {id}"
        );
    }
    // Groups sorted by label: Food › Coffee before Food › Snacks before
    // Fun › Games.
    assert!(
        rules_sel_y(&mut window, &format!("rules:group-{coffee}")) < {
            draw_frame(cx, handle);
            rules_sel_y(&mut window, &format!("rules:group-{snacks}"))
        }
    );
    assert!(
        rules_sel_y(&mut window, &format!("rules:group-{snacks}")) < {
            draw_frame(cx, handle);
            rules_sel_y(&mut window, &format!("rules:group-{games}"))
        }
    );
    // Within the Coffee group the rows are by Pattern: "COFFEE" above
    // "COFFEE SHOP".
    assert!(
        rules_sel_y(&mut window, &format!("rules:row-{}", rules[0])) < {
            draw_frame(cx, handle);
            rules_sel_y(&mut window, &format!("rules:row-{}", rules[1]))
        },
        "rows sort by Pattern within their group"
    );
}

/// 3.28: Search filters live over the pattern and the target category's
/// child and parent names, case-insensitively.
#[gpui::test]
fn rules_page_search_covers_patterns_and_category_names(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let mut window = VisualTestContext::from_window(handle, cx);

    // A pattern substring: only r2 ("COFFEE SHOP").
    set_rules_search(cx, handle, &shell, "shop");
    draw_frame(cx, handle);
    assert!(paints(&mut window, format!("rules:row-{}", rules[1])));
    for id in [rules[0], rules[2], rules[3]] {
        assert!(
            !paints(&mut window, format!("rules:row-{id}")),
            "r{id} does not match \"shop\""
        );
    }

    // A child-category name in another case: only r3 (targets Games).
    set_rules_search(cx, handle, &shell, "games");
    draw_frame(cx, handle);
    assert!(paints(&mut window, format!("rules:row-{}", rules[2])));
    for id in [rules[0], rules[1], rules[3]] {
        assert!(
            !paints(&mut window, format!("rules:row-{id}")),
            "rule {id} does not match \"games\""
        );
    }

    // A parent-category name: "fun" is r3's parent.
    set_rules_search(cx, handle, &shell, "fun");
    draw_frame(cx, handle);
    assert!(paints(&mut window, format!("rules:row-{}", rules[2])));
    assert!(!paints(&mut window, format!("rules:row-{}", rules[0])));

    // Nothing matches: the empty state, no rows.
    set_rules_search(cx, handle, &shell, "zzz");
    draw_frame(cx, handle);
    assert!(paints(&mut window, "rules:empty".into()));
    for id in rules {
        assert!(!paints(&mut window, format!("rules:row-{id}")));
    }
    // Clearing the box shows everything again.
    set_rules_search(cx, handle, &shell, "");
    draw_frame(cx, handle);
    assert!(!paints(&mut window, "rules:empty".into()));
    for id in rules {
        assert!(paints(&mut window, format!("rules:row-{id}")));
    }
}

/// 3.28: "Test a description" shows only the rules matching the text, in
/// the order the categoriser tries them, and marks the rule that would
/// actually win — the same one `winning_rule` picks.
#[gpui::test]
fn rules_page_test_mode_marks_the_winner_first(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_test_mode = true;
            cx.notify();
        });
    });
    set_rules_search(cx, handle, &shell, "COFFEE SHOP A");
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // Both substring rules match the description; the unused ones don't.
    assert!(paints(&mut window, format!("rules:row-{}", rules[0])));
    assert!(paints(&mut window, format!("rules:row-{}", rules[1])));
    for id in &rules[2..] {
        assert!(
            !paints(&mut window, format!("rules:row-{id}")),
            "r{id} does not match the description"
        );
    }
    // "COFFEE SHOP" is the longer substring, so it wins and is the first
    // row, marked "wins"; "COFFEE" follows unmarked.
    assert!(paints(&mut window, format!("rules:wins-{}", rules[1])));
    assert!(!paints(&mut window, format!("rules:wins-{}", rules[0])));
    assert!(
        rules_sel_y(&mut window, &format!("rules:row-{}", rules[1])) < {
            draw_frame(cx, handle);
            rules_sel_y(&mut window, &format!("rules:row-{}", rules[0]))
        },
        "the winner is the first row"
    );

    // Leaving test mode restores the grouped list.
    set_rules_search(cx, handle, &shell, "");
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_test_mode = false;
            cx.notify();
        });
    });
    draw_frame(cx, handle);
    assert!(!paints(&mut window, format!("rules:wins-{}", rules[1])));
    assert!(paints(&mut window, format!("rules:row-{}", rules[2])));
}

/// 3.28: the Unused checkbox keeps only the rules deciding zero
/// transactions; their Uses column reads 0.
#[gpui::test]
fn rules_page_unused_shows_only_zero_use_rules(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.rules_unused_only = true;
            cx.notify();
        });
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    // r1 (2 uses) and r2 (1 use) hide; the two rules nothing matches stay.
    for id in &rules[..2] {
        assert!(
            !paints(&mut window, format!("rules:row-{id}")),
            "a used rule hides behind Unused"
        );
    }
    for id in &rules[2..] {
        assert!(
            paints(&mut window, format!("rules:row-{id}")),
            "the unused rule {id} stays listed"
        );
    }
}

/// 3.28: the Type picker keeps only rows of the chosen kind.
#[gpui::test]
fn rules_page_type_picker_filters_by_kind(cx: &mut TestAppContext) {
    let (handle, shell, _coffee, _snacks, _games, rules) = open_rules_page(cx);
    let pick = |cx: &mut TestAppContext, row: Option<usize>| {
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, cx| {
                s.rules_type_select.update(cx, |sel, cx| {
                    sel.set_selected_index(
                        row.map(gpui_kit::component::IndexPath::new),
                        window,
                        cx,
                    );
                });
            });
        });
        cx.run_until_parked();
    };
    let mut window = VisualTestContext::from_window(handle, cx);

    pick(cx, Some(2)); // Regex
    draw_frame(cx, handle);
    assert!(paints(&mut window, format!("rules:row-{}", rules[3])));
    for id in &rules[..3] {
        assert!(
            !paints(&mut window, format!("rules:row-{id}")),
            "substring rules hide under Regex"
        );
    }

    pick(cx, Some(1)); // Substring
    draw_frame(cx, handle);
    for id in &rules[..3] {
        assert!(paints(&mut window, format!("rules:row-{id}")));
    }
    assert!(!paints(&mut window, format!("rules:row-{}", rules[3])));

    pick(cx, None); // All types
    draw_frame(cx, handle);
    for id in rules {
        assert!(paints(&mut window, format!("rules:row-{id}")));
    }
}

/// 3.28: clicking a group header collapses the group — its rows are gone,
/// the header stays — and clicking it again expands.
#[gpui::test]
fn rules_page_collapse_hides_a_groups_rows(cx: &mut TestAppContext) {
    let (handle, _shell, coffee, _snacks, _games, rules) = open_rules_page(cx);
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    let header = window
        .debug_bounds(leaked(format!("rules:group-{coffee}")))
        .expect("the Coffee group header paints");
    window.simulate_click(header.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        paints(&mut window, format!("rules:group-{coffee}")),
        "the collapsed group keeps its header"
    );
    for id in [rules[0], rules[1]] {
        assert!(
            !paints(&mut window, format!("rules:row-{id}")),
            "rule {id} hides with its collapsed group"
        );
    }
    assert!(
        paints(&mut window, format!("rules:row-{}", rules[2])),
        "other groups stay open"
    );

    let header = window
        .debug_bounds(leaked(format!("rules:group-{coffee}")))
        .expect("the header still paints");
    window.simulate_click(header.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(paints(&mut window, format!("rules:row-{}", rules[0])));
    assert!(paints(&mut window, format!("rules:row-{}", rules[1])));
}

/// 3.28: checking rules opens the Move bar; Move points them at another
/// subcategory in one call, which re-runs the rules over the unlocked rows
/// (the locked row keeps its override) and clears the selection.
#[gpui::test]
fn rules_page_move_recategorizes_rows_and_clears_selection(cx: &mut TestAppContext) {
    let (handle, shell, coffee, snacks, games, rules) = open_rules_page(cx);
    let (r1, r2) = (rules[0], rules[1]);
    with_window(cx, handle, |_, cx| {
        shell.update(cx, |s, cx| {
            s.set_rule_selected(r1, true, cx);
            s.set_rule_selected(r2, true, cx);
        });
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        paints(&mut window, "rules:move-bar".into()),
        "checked rows open the Move bar"
    );

    // Target: Food › Snacks, the same child list the rule pickers use.
    let ix = cx.update(|cx| {
        shell
            .read(cx)
            .child_choices
            .iter()
            .position(|(id, _)| *id == snacks)
            .unwrap()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.move_rules_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.move_selected_rules(window, cx);
        });
    });
    wait_until(cx, "the move lands", |cx| {
        let s = shell.read(cx);
        s.rules.iter().all(|r| r.category_id != coffee) && s.cat_counts.get(&snacks) == Some(&3)
    });
    // Both moved rules now target Snacks; the other two are untouched.
    shell.read_with(cx, |s, _| {
        assert_eq!(
            s.rules.iter().filter(|r| r.category_id == snacks).count(),
            3
        );
    });
    // The unlocked rows followed the rules; the locked row stayed in Games.
    // A category with no rows drops out of `cat_counts` entirely.
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            !s.cat_counts.contains_key(&coffee),
            "Coffee has no rows left after the move"
        );
        assert_eq!(s.cat_counts.get(&snacks), Some(&3));
        assert_eq!(s.cat_counts.get(&games), Some(&1));
        assert!(s.selected_rules.is_empty(), "the selection clears");
    });
    draw_frame(cx, handle);
    assert!(
        !paints(&mut window, "rules:move-bar".into()),
        "the Move bar closes with the cleared selection"
    );
}

/// 3.28: thousands of rules — the virtualized list builds only the rows
/// that can be on screen, headers included.
#[gpui::test]
fn rules_page_builds_only_the_visible_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "two thousand rules", |cx| {
        shell.read(cx).rules.len() == 2_000
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    let built = shell.read_with(cx, |s, _| s.rules_page_rows_built.get());
    assert!(
        built > 0 && built < 200,
        "the list built {built} of 2000 rules, only the visible slice"
    );
}

/// 3.28: at the 1200 × 720 minimum window the Rules page fits: header,
/// filter row, group header and a rule row with long names stay inside the
/// page, and the long pattern truncates to its column.
#[gpui::test]
fn the_rules_page_fits_the_minimum_window(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_categories_fit(c));
    let (child, rule) = ids;
    wait_until(cx, "the rule loaded", |cx| shell.read(cx).rules.len() == 1);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    let page_right = px(1200. - PAGE_PAD);
    for (what, sel) in [
        ("the filter row", "rules:filter-row".to_string()),
        ("the group header", format!("rules:group-{child}")),
        ("the rule row", format!("rules:row-{rule}")),
        ("the New rule… button", format!("label:{NEW_RULE}")),
    ] {
        let b = window
            .debug_bounds(leaked(sel))
            .unwrap_or_else(|| panic!("the page paints {what}"));
        assert!(
            b.origin.x + b.size.width <= page_right + px(1.),
            "{what} ends at x {:?}, past the page edge {page_right:?}",
            b.origin.x + b.size.width,
        );
    }
    // The pattern truncates inside its column, like on Categories.
    let col = window
        .debug_bounds(leaked(format!("rules:pattern-col:{rule}")))
        .expect("the rule row paints its pattern column");
    let full_w = TEXT_SM_CHAR_PX * CAT_FIT_PATTERN.chars().count() as f32;
    assert!(
        col.size.width < px(full_w),
        "the pattern truncates: the column is {:?} wide, the full text \
         needs about {full_w:?}",
        col.size.width,
    );
}

/// 3.28: opening the Rules page focuses the search box (D5), so the user
/// can type straight away.
#[gpui::test]
fn rules_page_opens_with_the_search_focused(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        seed_categories_fit(conn);
    });
    wait_until(cx, "the rule loaded", |cx| shell.read(cx).rules.len() == 1);
    // The focus request lands on the first frame of the Rules page.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    let focused = with_window(cx, handle, |window, cx| {
        shell
            .read(cx)
            .rules_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "opening the Rules page focuses Search");
    // Leaving and returning focuses it again.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    draw_frame(cx, handle);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    draw_frame(cx, handle);
    let focused = with_window(cx, handle, |window, cx| {
        shell
            .read(cx)
            .rules_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "returning to the Rules page focuses Search again");
}

/// 3.29: *Show rule* from a dashboard row's menu opens the Rules page on
/// the rule that decided that row: the search box takes the rule's pattern,
/// the rule's row is scrolled into view and carries the reveal highlight,
/// and a new search or a page switch drops the highlight. The fixture adds
/// 30 filler rules in their own "Fill ›" groups (all matching "COFFEE", all
/// sorted before "Food › Coffee") so the revealing really has to scroll.
#[gpui::test]
fn show_rule_from_a_row_lands_on_the_deciding_rule(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, [0i64; 4]);
    let (handle, shell) = open_shell(cx, |conn| {
        ids = seed_rules_page(conn);
        let fill =
            financi_db::categories::create_parent(conn, "Fill", CategoryKind::Expense).unwrap();
        for n in 1..=30 {
            let child =
                financi_db::categories::create_child(conn, fill, &format!("A{n:02}")).unwrap();
            financi_db::rules::add_rule(conn, &format!("COFFEE A{n:02}"), false, child).unwrap();
        }
    });
    let (coffee, _snacks, _games, rules) = ids;
    let (r1, r3) = (rules[0], rules[2]);
    wait_until(cx, "the rules and dashboard rows loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 34
            && s.rule_uses.get(&r1) == Some(&2)
            && s.cat_counts.get(&coffee) == Some(&3)
            && s.table.read(cx).delegate().rows.len() == 5
    });
    let mut window = VisualTestContext::from_window(handle, cx);

    // The menu model on the dashboard row: r1 decided it, so *Show rule*
    // is live.
    let row = cx
        .update(|cx| {
            table_rows(&shell, cx)
                .into_iter()
                .find(|r| r.description.contains("BEAN ROAST 1"))
        })
        .expect("the roast row is on the dashboard");
    assert_eq!(
        cx.update(|cx| shell.update(cx, |s, _| row_show_rule(
            &row,
            &s.rules,
            s.card_payment_category
        ))),
        RowShowRule::Found(r1)
    );
    assert_eq!(
        cx.update(|cx| shell.read(cx).page),
        Page::Dashboard,
        "the menu is opened on the dashboard"
    );

    // Clicking *Show rule* navigates, narrows, scrolls and highlights.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_rule(r1, window, cx));
    });
    let active = format!("rules:row-{r1}:active");
    for _ in 0..8 {
        cx.run_until_parked();
        draw_frame(cx, handle);
        if paints(&mut window, active.clone()) {
            break;
        }
    }
    assert_eq!(
        cx.update(|cx| shell.read(cx).page),
        Page::Rules,
        "*Show rule* lands on the Rules page"
    );
    assert_eq!(
        with_window(cx, handle, |_, cx| {
            shell.read(cx).rules_search.read(cx).value().to_string()
        }),
        "COFFEE",
        "the search box takes the rule's pattern, find-in-page style"
    );
    assert_eq!(cx.update(|cx| shell.read(cx).rules_reveal), Some(r1));
    assert!(
        paints(&mut window, active.clone()),
        "the deciding rule paints highlighted"
    );
    assert!(
        !paints(&mut window, format!("rules:row-{r1}")),
        "the highlighted row carries only the reveal selector"
    );
    assert!(
        !paints(&mut window, format!("rules:row-{r3}")),
        "r3 (\"GAME\") does not match the pattern and stays hidden"
    );
    let y = rules_sel_y(&mut window, &active);
    assert!(
        y > 0. && y < 720.,
        "the revealed row was scrolled into view (y {y})"
    );

    // Typing a fresh search is the user's own search: the highlight is
    // gone. The search box has the focus, so a real Backspace edits it
    // (`set_value` would be programmatic and emit no Change event).
    cx.simulate_keystrokes(handle, "backspace");
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert_eq!(cx.update(|cx| shell.read(cx).rules_reveal), None);
    assert!(
        !paints(&mut window, active.clone()),
        "the highlight did not survive the user's own edit"
    );

    // Revealing again and then leaving the page drops it too.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_rule(r1, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(paints(&mut window, active.clone()), "revealed again");
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    draw_frame(cx, handle);
    assert_eq!(
        cx.update(|cx| shell.read(cx).rules_reveal),
        None,
        "the reveal belongs to the Rules page"
    );
}

// -- page switching (3.11) ---------------------------------------------------

/// 3.11: after a page switch only the current page paints. Walks every page,
/// then opens the accounts popover with a real click and switches away: the
/// popover must not stay painted, nor come back open.
#[gpui::test]
fn page_switches_paint_only_the_current_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    let mut window = VisualTestContext::from_window(handle, cx);

    // Everything a painted frame may reveal: the page-area selector and
    // two page-specific widgets. Exactly one page root may appear per frame.
    const PAINTED: [&str; 5] = [
        "page:page-dashboard",
        "page:page-queue",
        "page:page-categories",
        "page:page-rules",
        "checkbox:account-1",
    ];
    let painted = |window: &mut VisualTestContext| -> Vec<&'static str> {
        PAINTED
            .iter()
            .copied()
            .filter(|s| window.debug_bounds(s).is_some())
            .collect()
    };
    let expect = |page: Page| -> Vec<&'static str> {
        match page {
            Page::Dashboard => vec!["page:page-dashboard"],
            Page::ToCategorize => vec!["page:page-queue"],
            Page::Categories => vec!["page:page-categories"],
            Page::Rules => vec!["page:page-rules"],
            Page::Transfers | Page::Imports => vec![],
        }
    };

    for page in [
        Page::Dashboard,
        Page::ToCategorize,
        Page::Categories,
        Page::Rules,
        Page::Transfers,
        Page::Imports,
        Page::Dashboard,
    ] {
        cx.update(|cx| shell.update(cx, |s, cx| s.go_page(page, cx)));
        draw_frame(cx, handle);
        assert_eq!(
            painted(&mut window),
            expect(page),
            "painted after switching to {page:?}"
        );
    }

    // The accounts popover (Dashboard content) opened with a real click:
    // switching away must close it, and returning must not show it open.
    let trigger = window
        .debug_bounds("accounts:filter")
        .expect("accounts filter trigger");
    window.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        painted(&mut window).contains(&"checkbox:account-1"),
        "the popover opened on the Dashboard"
    );
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    draw_frame(cx, handle);
    assert_eq!(
        painted(&mut window),
        expect(Page::Categories),
        "the popover is gone after switching pages"
    );
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    draw_frame(cx, handle);
    assert_eq!(
        painted(&mut window),
        expect(Page::Dashboard),
        "the popover does not come back open"
    );
}

/// 3.11: `go_page` closes the transient UI it owns — the two-click
/// confirmations, the rule offer and the inline rename editor.
#[gpui::test]
fn go_page_closes_transient_ui(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |conn| {
        create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    wait_until(cx, "the account list loads", |cx| {
        shell.read(cx).accounts.len() == 1
    });
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.confirm_undo_import = Some(1);
            s.rule_offer = Some((1, 2));
            s.renaming = true;
            s.confirm_delete_category = Some(1);
            s.go_page(Page::Categories, cx);
        });
        let s = shell.read(cx);
        assert!(s.confirm_undo_import.is_none(), "undo confirmation cleared");
        assert!(s.rule_offer.is_none(), "rule offer cleared");
        assert!(!s.renaming, "rename editor closed");
        assert!(
            s.confirm_delete_category.is_none(),
            "delete confirmation cleared"
        );
    });
}

/// 3.18 regression: opening Categories while the first refresh is still in
/// flight used to leave the page blank — the list columns collapsed to 0px
/// inside the page's `h_flex` row (which centers its children), so the
/// virtual lists resolved an empty window and no row appeared when the data
/// finally arrived. Opening the page first, with the data arriving after,
/// must still fill both lists without any further action.
#[gpui::test]
fn categories_page_shows_rows_when_data_arrives_after_it_opens(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // One draw while the refresh is still in flight: the page is open empty.
    draw_frame(cx, handle);
    wait_until(cx, "categories and rules in the shell", |cx| {
        let s = shell.read(cx);
        s.categories.len() == 2_001 && s.rules.len() == 2_000
    });
    // 3.23: the rule list paints with the selection; all 2,000 rules sit
    // under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    // The first draw that has the data must show rows — no extra action.
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.tree_rows_built.get() >= 8,
            "tree rows on first open, before any other action: {}",
            s.tree_rows_built.get()
        );
        assert!(
            s.rule_rows_built.get() > 0,
            "rule rows on first open, before any other action: {}",
            s.rule_rows_built.get()
        );
    });
}

/// 3.18 regression: switching to Categories from another page after the
/// data has already loaded must fill both lists on the first draw there.
#[gpui::test]
fn switching_to_categories_after_load_shows_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_thousands);
    wait_until(cx, "2001 category parents", |cx| {
        shell.read(cx).categories.len() == 2_001
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    wait_until(cx, "2000 rules", |cx| shell.read(cx).rules.len() == 2_000);
    // 3.23: the rule list paints with the selection; all 2,000 rules sit
    // under "Cat 0000" (see `seed_thousands`).
    select_parent(cx, handle, &shell, "Cat 0000");
    draw_frame(cx, handle);
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            s.tree_rows_built.get() >= 8,
            "tree rows after switching to the page: {}",
            s.tree_rows_built.get()
        );
        assert!(
            s.rule_rows_built.get() > 0,
            "rule rows after switching to the page: {}",
            s.rule_rows_built.get()
        );
    });
}

/// 3.16: the Categories page labels every control — the tree heading, the
/// new-parent fields, the detail pickers (parent and child variants) and
/// the add-rule row.
#[gpui::test]
fn the_categories_page_labels_its_controls(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    add_parent(cx, handle, &shell, "Food & Dining");
    let parent_id = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food & Dining")
            .unwrap()
            .id
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(parent_id, window, cx);
            // 3.23: the new-category form only paints once opened.
            s.new_category_open = true;
        });
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for sel in [
        "label:Category tree",
        "label:New category name",
        "label:Kind",
        "label:Organize",
        "label:New subcategory name",
        "label:Rules",
        "label:Delete",
        "label:Move its transactions to",
        "label:Pattern",
        "label:Category",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the Categories page paints the visible label {sel:?}"
        );
    }
    // A selected child adds the move-to-category picker's label.
    add_child(cx, handle, &shell, "Restaurants");
    let child_id = cx.update(|cx| find_child(&shell, cx, "Restaurants").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(child_id, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("label:Move to category").is_some(),
        "the child detail paints the move picker's label"
    );
}

/// 3.33: nothing the Categories page shows says parent, child or children.
/// With a long-named tree and a subcategory selected, the page paints the
/// new labels and none of the old ones (debug selectors), the placeholders
/// it renders are clean, and every validation error — the typed ones and
/// the ones the database refuses with — reaches the page in the new words.
/// The renamed buttons still add a category and a subcategory.
#[gpui::test]
fn the_categories_page_never_says_parent_or_child(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| {
        let food = financi_db::categories::create_parent(
            conn,
            "Food, Groceries & Dining Out",
            CategoryKind::Expense,
        )
        .unwrap();
        financi_db::categories::create_child(conn, food, "Restaurants, Cafés & Takeaway").unwrap();
        financi_db::categories::create_parent(
            conn,
            "Home & Garden Supplies",
            CategoryKind::Expense,
        )
        .unwrap();
    });
    wait_until(cx, "the tree loads", |cx| {
        shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.run_until_parked();

    fn retired_word(s: &str) -> String {
        const FORBIDDEN: [&str; 3] = ["parent", "child", "children"];
        let low = s.to_lowercase();
        FORBIDDEN
            .into_iter()
            .find(|w| low.contains(*w))
            .unwrap_or("")
            .to_string()
    }

    // Trigger one validation error the way its button does, and check the
    // message the page then renders: exact new copy, no retired word.
    fn trigger_error(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        shell: &Entity<Shell>,
        what: &str,
        trigger: impl Fn(&mut Window, &mut App),
        want: &str,
    ) {
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, _| s.cat_error = None);
            trigger(window, cx);
        });
        wait_until(cx, &format!("the {what:?} error"), |cx| {
            shell.read(cx).cat_error.is_some()
        });
        let e = cx.update(|cx| shell.read(cx).cat_error.clone()).unwrap();
        assert_eq!(e, want, "the {what:?} error reads");
        assert!(
            retired_word(&e).is_empty(),
            "{e:?} still says something retired"
        );
        draw_frame(cx, handle);
    }

    let food_id = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .find(|p| p.name == "Food, Groceries & Dining Out")
            .unwrap()
            .id
    });
    let child = cx.update(|cx| find_child(&shell, cx, "Restaurants, Cafés & Takeaway").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.select_category(food_id, window, cx);
            // 3.23: the new-category form only paints once its button
            // opened it; open it so the scan covers its labels too.
            s.new_category_open = true;
        });
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    // The painted labels carry the new words… buttons and Select
    // placeholders included: their text is reachable only through the
    // `label:<text>` debug selectors (3.33).
    let n_cats = cx.update(|cx| shell.read(cx).categories.len());
    let n_subs = cx.update(|cx| {
        shell
            .read(cx)
            .categories
            .iter()
            .map(|p| p.children.len())
            .sum::<usize>()
    });
    for sel in [
        "label:Category tree",
        "label:New category name",
        "label:Kind",
        "label:Add category",
        "label:New subcategory name",
        "label:Add subcategory",
        "label:Move its transactions to…",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the Categories page paints {sel:?}"
        );
    }
    let count_sel = leaked(format!(
        "label:{n_cats} categories · {n_subs} subcategories"
    ));
    assert!(
        window.debug_bounds(count_sel).is_some(),
        "the header paints the count {count_sel:?}"
    );
    // …and none of the retired ones.
    for sel in [
        "label:New parent name",
        "label:New child name",
        "label:Add parent",
        "label:Add child",
        leaked(format!("label:{n_cats} parents")),
    ] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "the Categories page never paints {sel:?}"
        );
    }

    // A selected subcategory adds the move picker, again in the new words.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(child, window, cx));
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("label:Move to category").is_some(),
        "the Categories page paints the move picker's label"
    );
    assert!(
        window.debug_bounds("label:Move to category…").is_some(),
        "the move picker paints its placeholder in the new words"
    );
    for sel in ["label:Move to parent", "label:Move to parent…"] {
        assert!(
            window.debug_bounds(sel).is_none(),
            "the Categories page never paints {sel:?}"
        );
    }

    // The placeholders are the exact strings the inputs render.
    cx.update(|cx| {
        let s = shell.read(cx);
        for (what, input) in [
            ("add-category", &s.new_parent_name),
            ("add-subcategory", &s.new_child_name),
            ("rename", &s.rename_input),
        ] {
            let p = input.read(cx).presentation().placeholder().to_string();
            assert!(
                retired_word(&p).is_empty(),
                "the {what} input's placeholder {p:?} still says {:?}",
                retired_word(&p)
            );
        }
    });

    trigger_error(
        cx,
        handle,
        &shell,
        "add category",
        |window, cx| shell.update(cx, |s, cx| s.add_parent(window, cx)),
        "Give the category a name.",
    );
    trigger_error(
        cx,
        handle,
        &shell,
        "add subcategory",
        |window, cx| {
            shell.update(cx, |s, cx| {
                s.select_category(food_id, window, cx);
                s.add_child(window, cx);
            });
        },
        "Give the subcategory a name.",
    );
    trigger_error(
        cx,
        handle,
        &shell,
        "move",
        |window, cx| {
            shell.update(cx, |s, cx| {
                s.select_category(child, window, cx);
                s.move_child_to_selected_parent(window, cx);
            });
        },
        "Choose the category to move it under.",
    );
    trigger_error(
        cx,
        handle,
        &shell,
        "delete",
        |window, cx| {
            shell.update(cx, |s, cx| {
                s.select_category(child, window, cx);
                s.arm_delete_category(window, cx); // first click arms
                s.arm_delete_category(window, cx);
            });
        },
        "Choose where its transactions should go.",
    );

    // A duplicate subcategory name: the database's own refusal.
    let duplicate = {
        with_window(cx, handle, |window, cx| {
            shell.update(cx, |s, cx| {
                s.cat_error = None;
                s.select_category(food_id, window, cx);
                s.new_child_name.update(cx, |i, cx| {
                    i.set_value("Restaurants, Cafés & Takeaway", window, cx);
                });
                s.add_child(window, cx);
            });
        });
        wait_until(cx, "the duplicate refusal", |cx| {
            shell.read(cx).cat_error.is_some()
        });
        let e = cx.update(|cx| shell.read(cx).cat_error.clone()).unwrap();
        assert_eq!(
            e,
            "a category named \"Restaurants, Cafés & Takeaway\" already exists under the same category"
        );
        e
    };
    assert!(retired_word(&duplicate).is_empty());

    // Deleting the category that still has subcategories: refused, in the
    // new words.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.cat_error = None;
            s.select_category(food_id, window, cx);
        });
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
        let ix = s_ix(&shell, cx, child);
        s_select(&shell, cx, ix, window);
        shell.update(cx, |s, cx| s.arm_delete_category(window, cx));
    });
    wait_until(cx, "the has-subcategories refusal", |cx| {
        shell.read(cx).cat_error.is_some()
    });
    let has_sub = cx.update(|cx| shell.read(cx).cat_error.clone()).unwrap();
    assert_eq!(
        has_sub,
        "Food, Groceries & Dining Out still has subcategories; delete or move them first"
    );
    assert!(retired_word(&has_sub).is_empty());

    // The renamed buttons still work: a new category and, under it, a new
    // subcategory.
    add_parent(cx, handle, &shell, "Travel & Commuting");
    add_child(cx, handle, &shell, "Airfare");
    cx.update(|cx| {
        assert!(
            shell
                .read(cx)
                .categories
                .iter()
                .any(|p| p.name == "Travel & Commuting"
                    && p.children.iter().any(|c| c.name == "Airfare")),
            "the renamed buttons added the category and its subcategory"
        );
    });
    draw_frame(cx, handle);
}

/// Two parents, each with one subcategory whose rules target that
/// subcategory (3.23: the rule list filters on the selection). Returns
/// `(drinks, transport, coffee_rule, fuel_rule)`.
fn seed_split_rules(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64, i64) {
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let coffee = financi_db::categories::create_child(conn, drinks, "Coffee").unwrap();
    let transport =
        financi_db::categories::create_parent(conn, "Transport", CategoryKind::Expense).unwrap();
    let fuel = financi_db::categories::create_child(conn, transport, "Fuel").unwrap();
    let coffee_rule = financi_db::rules::add_rule(conn, "COFFEE", false, coffee).unwrap();
    let fuel_rule = financi_db::rules::add_rule(conn, "SHELL", false, fuel).unwrap();
    (drinks, transport, coffee_rule, fuel_rule)
}

/// 3.23: the rule list holds only the selected category's rules — a parent
/// shows its subcategories', other categories' rules disappear on a new
/// selection, and a category without rules shows the empty hint instead.
/// Without a selection the detail panel prompts for one.
#[gpui::test]
fn selecting_a_category_lists_only_its_own_rules(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_split_rules(c));
    let (_drinks, _transport, coffee_rule, fuel_rule) = ids;
    // +1: the system `Transfers` parent from migration 0002.
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2 && shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:detail-empty").is_some(),
        "without a selection the detail panel prompts for one"
    );

    select_parent(cx, handle, &shell, "Drinks");
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds(leaked(format!("rule-row-{coffee_rule}")))
            .is_some(),
        "a parent's list holds its subcategories' rules"
    );
    assert!(
        window
            .debug_bounds(leaked(format!("rule-row-{fuel_rule}")))
            .is_none(),
        "another category's rule is not listed"
    );

    select_parent(cx, handle, &shell, "Transport");
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds(leaked(format!("rule-row-{fuel_rule}")))
            .is_some(),
        "the other parent's rule lists after switching"
    );
    assert!(
        window
            .debug_bounds(leaked(format!("rule-row-{coffee_rule}")))
            .is_none(),
        "the first parent's rule is gone after switching"
    );

    // A category without rules shows the hint and no rows.
    select_parent(cx, handle, &shell, "Transfers");
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:rules-empty").is_some(),
        "a ruleless category shows the empty hint"
    );
    assert!(
        window
            .debug_bounds(leaked(format!("rule-row-{coffee_rule}")))
            .is_none()
            && window
                .debug_bounds(leaked(format!("rule-row-{fuel_rule}")))
                .is_none(),
        "no rule rows paint for the ruleless category"
    );
}

/// 3.23: the add-rule Category picker defaults to the selected category
/// when it lists it (the picker lists subcategories), and clears when the
/// selection is one it cannot show.
#[gpui::test]
fn the_add_rule_picker_defaults_to_the_selected_subcategory(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_split_rules(c));
    let (_drinks, _transport, _coffee_rule, _fuel_rule) = ids;
    wait_until(cx, "both rules loaded", |cx| {
        shell.read(cx).rules.len() == 2 && shell.read(cx).categories.len() == 3
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    let fuel = cx.update(|cx| find_child(&shell, cx, "Fuel").unwrap());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.select_category(fuel, window, cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let s = shell.read(cx);
        let want = s
            .child_choices
            .iter()
            .position(|(id, _)| *id == fuel)
            .expect("Fuel is a subcategory choice");
        assert_eq!(
            s.rule_target.read(cx).selected_index(cx).map(|p| p.row),
            Some(want),
            "the picker defaults to the selected subcategory"
        );
        assert_eq!(s.child_choices[want].1, "Transport › Fuel");
    });
    // A parent is not in the picker, so selecting one clears the pick.
    select_parent(cx, handle, &shell, "Drinks");
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(
            s.rule_target.read(cx).selected_index(cx).map(|p| p.row),
            None,
            "selecting a parent clears the picker"
        );
    });
}

/// 3.23: the header's "New category…" button opens the add-category form
/// in place; Cancel hides it again, a successful add closes it, and so does
/// leaving the page.
#[gpui::test]
fn the_new_category_button_opens_and_closes_the_form(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "initial load", |cx| {
        shell.read(cx).refresh_task.is_some()
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "the form starts hidden"
    );
    // Click the header button.
    let b = window
        .debug_bounds(leaked(format!("label:{NEW_CATEGORY}")))
        .expect("the header paints the New category… button");
    window.simulate_click(b.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_some(),
        "the click opens the form"
    );
    // Cancel closes it again.
    let cancel = window
        .debug_bounds("cat:new-form-cancel")
        .expect("the form paints its Cancel button");
    window.simulate_click(cancel.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "Cancel closes the form"
    );
    // Reopen, then add a category: success closes the form too.
    cx.update(|cx| shell.update(cx, |s, _cx| s.new_category_open = true));
    draw_frame(cx, handle);
    add_parent(cx, handle, &shell, "Books");
    cx.update(|cx| {
        assert!(
            !shell.read(cx).new_category_open,
            "adding a category closes the form"
        );
    });
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "the closed form paints nothing after the add"
    );
    // And it never survives a page switch.
    cx.update(|cx| shell.update(cx, |s, _cx| s.new_category_open = true));
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("cat:new-form").is_none(),
        "leaving the page closes the form"
    );
}

/// 3.23 fit-test names: long enough that nothing but truncation fits.
const CAT_FIT_PARENT: &str = "Food, Groceries & Dining Out On Both Sides Of The Boulevard Downtown";
const CAT_FIT_CHILD: &str = "Restaurants, Cafés, Takeaway & Late-Night Snack Stands Far Uptown";
const CAT_FIT_PATTERN: &str = "LONG RULE PATTERN THAT WILL NOT FIT THE RULE ROW AT THE MINIMUM WINDOW SIZE SO IT MUST TRUNCATE";

/// One long-named parent and subcategory with one long-pattern rule.
fn seed_categories_fit(conn: &financi_db::rusqlite::Connection) -> (i64, i64) {
    let parent =
        financi_db::categories::create_parent(conn, CAT_FIT_PARENT, CategoryKind::Expense).unwrap();
    let child = financi_db::categories::create_child(conn, parent, CAT_FIT_CHILD).unwrap();
    let rule = financi_db::rules::add_rule(conn, CAT_FIT_PATTERN, false, child).unwrap();
    (child, rule)
}

/// 3.23: at the 1200 × 720 minimum window both panels, the new-category
/// form and a rule row with long texts stay fully inside the page; the
/// long rule pattern truncates to its column.
#[gpui::test]
fn the_categories_page_fits_the_minimum_window(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_categories_fit(c));
    let (child, rule) = ids;
    wait_until(cx, "the rule loaded", |cx| shell.read(cx).rules.len() == 1);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    // The widest state of every part: form open, long subcategory selected.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.new_category_open = true;
            s.select_category(child, window, cx);
        });
    });
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);

    let page_right = px(1200. - PAGE_PAD);
    for (what, sel) in [
        ("the new-category form", "cat:new-form".to_string()),
        ("the tree panel", "cat:tree-panel".to_string()),
        ("the detail panel", "cat:detail-panel".to_string()),
        ("the selected tree row", format!("cat-tree-row-{child}")),
        ("the rule row", format!("rule-row-{rule}")),
    ] {
        let b = window
            .debug_bounds(leaked(sel))
            .unwrap_or_else(|| panic!("the page paints {what}"));
        assert!(
            b.origin.x + b.size.width <= page_right + px(1.),
            "{what} ends at x {:?}, past the page edge {page_right:?}",
            b.origin.x + b.size.width,
        );
    }

    // The rule pattern truncates: its column is far narrower than the
    // full text needs at 14 px.
    let col = window
        .debug_bounds(leaked(format!("rule:pattern-col:{rule}")))
        .expect("the rule row paints its pattern column");
    let full_w = TEXT_SM_CHAR_PX * CAT_FIT_PATTERN.chars().count() as f32;
    assert!(
        col.size.width < px(full_w / 2.),
        "the pattern truncates: the column is {:?} wide, the full text \
         needs about {full_w:?}",
        col.size.width,
    );
    // The detail title box cannot outgrow the panel beside it.
    let title = window
        .debug_bounds("cat:detail-title")
        .expect("the detail panel paints the category title");
    let panel = window
        .debug_bounds("cat:detail-panel")
        .expect("the detail panel");
    assert!(
        title.size.width <= panel.size.width,
        "the detail title truncates inside the panel: title {:?}, panel {:?}",
        title.size.width,
        panel.size.width,
    );

    // A tree row fills its panel: a content-width row would leave the
    // counts floating mid-panel and a long name would never truncate, so
    // the row's right edge is the tree panel's content edge (border 1 px
    // + `p_2`).
    let tree = window
        .debug_bounds("cat:tree-panel")
        .expect("the tree panel");
    let row = window
        .debug_bounds(leaked(format!("cat-tree-row-{child}")))
        .expect("the tree paints the selected row");
    let tree_content_right = tree.origin.x + tree.size.width - px(9.);
    assert!(
        (row.origin.x + row.size.width - tree_content_right).abs() < px(1.),
        "the tree row fills the panel to its content edge: row right {:?}, \
         content edge {tree_content_right:?}",
        row.origin.x + row.size.width,
    );
    // A fixed control pinned to the detail panel: the add-rule row's last
    // child is the primary button, so the row must end exactly at the
    // panel's content edge (border 1 px + `p_4`).
    let add_row = window
        .debug_bounds("cat:add-rule-row")
        .expect("the detail panel paints the add-rule row");
    let detail_content_right = panel.origin.x + panel.size.width - px(17.);
    assert!(
        (add_row.origin.x + add_row.size.width - detail_content_right).abs() < px(1.),
        "the add-rule row fills the panel to its content edge: row right \
         {:?}, content edge {detail_content_right:?}",
        add_row.origin.x + add_row.size.width,
    );
}

/// 3.41: a child category whose full label "Drinks › …" is exactly 40
/// characters — too wide for the 200 px trigger (it ellipsizes), but it
/// fits the 320 px open menu.
const LONG_CHILD: &str = "international subscriptions ltd";

fn long_label() -> String {
    format!("Drinks › {LONG_CHILD}")
}

/// 3.41: like `seed_rule_fixture`, but the rule targets the long-named
/// child, so the editor's picker opens with the long selection and the
/// trigger needs its tooltip. Returns `(rule, long child, short child)`.
fn seed_long_name_rule_fixture(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let drinks =
        financi_db::categories::create_parent(conn, "Drinks", CategoryKind::Expense).unwrap();
    let long = financi_db::categories::create_child(conn, drinks, LONG_CHILD).unwrap();
    let short = financi_db::categories::create_child(conn, drinks, "Second").unwrap();
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents, description_raw)
         VALUES (?1, ?2, 0, X'00', 'long-name-hash', '2026-11-01', -1000, 'COFFEE MART')",
        (acct, import),
    )
    .unwrap();
    let rule = financi_db::rules::add_rule(conn, "COFFEE", false, long).unwrap();
    (rule, long, short)
}

/// 3.41: open the shell over the long-name fixture and select the parent,
/// so the Categories detail panel paints its add-rule row. Returns
/// `(handle, shell, rule, long child, short child)`.
fn open_long_name_rule_fixture(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Shell>, i64, i64, i64) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_long_name_rule_fixture(c));
    let (rule, long, short) = ids;
    wait_until(cx, "the long-name fixture loaded", |cx| {
        let s = shell.read(cx);
        s.rules.len() == 1 && s.child_choices.iter().any(|(_, n)| n == &long_label())
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Categories, cx)));
    select_parent(cx, handle, &shell, "Drinks");
    (handle, shell, rule, long, short)
}

/// 3.41: open a rule picker the way a user does — a click on the middle
/// of the trigger — and paint until the (deferred) menu is up.
fn open_picker_menu(
    cx: &mut TestAppContext,
    window: &mut VisualTestContext,
    handle: AnyWindowHandle,
    sel: &'static str,
) {
    let b = window
        .debug_bounds(sel)
        .unwrap_or_else(|| panic!("{sel} paints"));
    window.simulate_click(b.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    draw_frame(cx, handle);
}

/// 3.41: click outside an open menu to dismiss it (the popup closes on
/// any press that misses it).
fn click_past_anything(cx: &mut TestAppContext, window: &mut VisualTestContext) {
    window.simulate_click(point(px(2.), px(2.)), Modifiers::default());
    cx.run_until_parked();
}

/// 3.41: assert a menu row paints with its text unclipped — the natural
/// (nowrap, absolutely positioned) probe copy fits inside the drawn row
/// — and that the row sits inside the window.
fn assert_menu_item_unclipped(window: &mut VisualTestContext, label: &str, win_w: f32) {
    let item = window
        .debug_bounds(leaked(format!("rule-choice:{label}")))
        .unwrap_or_else(|| panic!("{label:?} paints in the open menu"));
    let natural = window
        .debug_bounds(leaked(format!("rule-choice-natural:{label}")))
        .expect("the natural-width probe paints beside the menu row");
    assert!(
        f32::from(natural.size.width) <= f32::from(item.size.width) + 1.,
        "{label:?} is clipped in the menu: the text wants {:?}, the row is {:?}",
        natural.size.width,
        item.size.width,
    );
    // The 320 px menu minus the list and row padding; a menu as narrow
    // as the old trigger (160 px) fails this.
    assert!(
        f32::from(item.size.width) >= 250.,
        "the menu row is {:?} wide, not the wide (320 px) menu",
        item.size.width,
    );
    assert!(
        f32::from(item.origin.x) + f32::from(item.size.width) <= win_w,
        "the open menu runs past the window's right edge: row ends at {:?}, window is {win_w} px wide",
        f32::from(item.origin.x) + f32::from(item.size.width),
    );
}

/// 3.41: with a 40-character "Parent › Child" name, the open menu of both
/// rule pickers — the add-rule row's on each page — paints the name
/// unclipped inside a menu that stays in the window, and typing in the
/// search box narrows the list to the matches.
#[gpui::test]
fn rule_category_picker_menus_show_the_full_name(cx: &mut TestAppContext) {
    let (handle, shell, _rule, _long, _short) = open_long_name_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let label = long_label();

    // The Categories page's add-rule picker: the trigger is the rule
    // list's Category column (200 px), not the old 160 px.
    let trigger = window
        .debug_bounds("rule:category-select:cat-add")
        .expect("the Categories page paints the add-rule picker");
    assert!(
        (f32::from(trigger.size.width) - 200.).abs() <= 1.,
        "the add-rule picker's trigger is {:?} wide, not 200 px",
        trigger.size.width,
    );

    open_picker_menu(cx, &mut window, handle, "rule:category-select:cat-add");
    assert_menu_item_unclipped(&mut window, &label, 1200.);

    // Typing narrows the list (the state is `searchable`): the long name
    // drops out, the match stays.
    window.simulate_input("second");
    draw_frame(cx, handle);
    assert!(
        !paints(&mut window, format!("rule-choice:{label}")),
        "the search narrowed the list: only matches paint",
    );
    assert!(
        paints(&mut window, "rule-choice:Drinks › Second".into()),
        "the matching entry stays in the list"
    );
    click_past_anything(cx, &mut window);
    draw_frame(cx, handle);

    // The Rules page's add-rule picker: same row, same menu.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.new_rule_open = true);
    });
    draw_frame(cx, handle);
    open_picker_menu(cx, &mut window, handle, "rule:category-select:add");
    assert_menu_item_unclipped(&mut window, &label, 1200.);
}

/// 3.41: the rule editor's picker is the same 200 px column with the wide
/// searchable menu, and the trigger — which ellipsizes the selected
/// long name — shows the full "Parent › Child" on hover.
#[gpui::test]
fn the_rule_editor_picker_shows_the_full_name(cx: &mut TestAppContext) {
    let (handle, shell, rule, _long, _short) = open_long_name_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let label = long_label();

    let trigger = window
        .debug_bounds("rule:category-select:edit")
        .expect("the rule editor paints its category picker");
    assert!(
        (f32::from(trigger.size.width) - 200.).abs() <= 1.,
        "the editor picker's trigger is {:?} wide, not 200 px",
        trigger.size.width,
    );

    // The ellipsized selection shows in full on hover.
    window.simulate_mouse_move(trigger.center(), None, Modifiers::default());
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds(leaked(format!("rule:category-tooltip:{label}")))
            .is_some(),
        "hovering the editor picker shows the full name {label:?} in a tooltip",
    );
    window.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);

    // And the open menu paints the full name unclipped, with the search
    // box narrowing the list.
    open_picker_menu(cx, &mut window, handle, "rule:category-select:edit");
    assert_menu_item_unclipped(&mut window, &label, 1200.);
    window.simulate_input("second");
    draw_frame(cx, handle);
    assert!(
        !paints(&mut window, format!("rule-choice:{label}")),
        "the editor picker's search narrowed the list",
    );
}

/// 3.42: in every rule row — the add-rule row on each page and the rule
/// editor — the regex switch sits on the controls' centre line: the
/// switch itself (its shrink-wrap probe, not the 32 px slot around it)
/// centres within 1 px of the Pattern input, the primary button and the
/// slot.
#[gpui::test]
fn the_regex_switch_lines_up_with_the_rule_fields(cx: &mut TestAppContext) {
    let (handle, shell, rule, _long, _short) = open_long_name_rule_fixture(cx);
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);

    let centre = |window: &mut VisualTestContext, sel: &'static str, where_: &str| -> f32 {
        let b = window
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} paints in the {where_} row"));
        f32::from(b.origin.y) + f32::from(b.size.height) / 2.
    };
    let aligned = |window: &mut VisualTestContext,
                   where_: &str,
                   switch: &'static str,
                   slot: &'static str,
                   input: &'static str,
                   button: &'static str| {
        let sw = centre(window, switch, where_);
        for (what, sel) in [
            ("the slot around it", slot),
            ("the Pattern input", input),
            ("the primary button", button),
        ] {
            let c = centre(window, sel, where_);
            assert!(
                (c - sw).abs() <= 1.,
                "in the {where_} row {what} centres at y {c}, the regex switch at {sw}",
            );
        }
    };

    // The Categories page's add-rule row.
    aligned(
        &mut window,
        "add-rule",
        "rule:switch:cat-add",
        "rule:regex-slot:cat-add",
        "rule:pattern-input:cat-add",
        "rule:add-rule-btn:cat-add",
    );

    // The rule editor row.
    start_rule_edit(cx, handle, &shell, rule);
    draw_frame(cx, handle);
    aligned(
        &mut window,
        "rule editor",
        "rule:switch:edit",
        "rule:regex-slot:edit",
        "rule:edit-pattern-input",
        "rule:edit-save-btn",
    );

    // The Rules page's add-rule row.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Rules, cx)));
    cx.update(|cx| {
        shell.update(cx, |s, _cx| s.new_rule_open = true);
    });
    draw_frame(cx, handle);
    aligned(
        &mut window,
        "Rules page add-rule",
        "rule:switch:add",
        "rule:regex-slot:add",
        "rule:pattern-input:add",
        "rule:add-rule-btn:add",
    );
}

/// 3.23: the rule-list headings use the rows' widths, and the Actions
/// label sits on the same line as the others, right-aligned over the
/// rows' buttons (3.24 MUST-KEEP). A stacked label-plus-buttons column
/// measured 83.5 px tall, slid the Actions label 64 px up, and pushed
/// the Type/Category headings 62 px right of their columns.
#[gpui::test]
fn the_rule_list_headings_sit_over_their_columns(cx: &mut TestAppContext) {
    let (handle, _shell, rule, _first, _second) = open_rule_fixture(cx);
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    for (label, heading_sel, column_sel) in [
        (
            "Pattern",
            "rule:heading-pattern",
            format!("rule:pattern-col:{rule}"),
        ),
        ("Type", "rule:heading-type", format!("rule:type-col:{rule}")),
        (
            "Category",
            "rule:heading-category",
            format!("rule:category-col:{rule}"),
        ),
    ] {
        let heading = window
            .debug_bounds(heading_sel)
            .unwrap_or_else(|| panic!("the rules list paints the {label:?} heading"));
        let column = window
            .debug_bounds(leaked(column_sel))
            .unwrap_or_else(|| panic!("the rule row paints the {label:?} column"));
        assert!(
            (heading.origin.x - column.origin.x).abs() < px(1.),
            "the {label:?} heading starts where its column starts: \
             heading x {:?}, column x {:?}",
            heading.origin.x,
            column.origin.x,
        );
    }
    // The Actions label is right-aligned over the buttons: its right
    // edge is the row's right edge (the Delete… button ends there).
    let actions = window
        .debug_bounds("rule:heading-actions")
        .expect("the rules list paints the Actions heading");
    let row = window
        .debug_bounds(leaked(format!("rule-row-{rule}")))
        .expect("the rules list paints the fixture's row");
    assert!(
        (actions.origin.x + actions.size.width - (row.origin.x + row.size.width)).abs() < px(1.),
        "the Actions heading ends where the row's buttons end: \
         heading right {:?}, row right {:?}",
        actions.origin.x + actions.size.width,
        row.origin.x + row.size.width,
    );
    // A single line: every heading label shares one line, and the whole
    // heading is no taller than a line of labels.
    let type_heading = window
        .debug_bounds("rule:heading-type")
        .expect("the rules list paints the Type heading");
    assert!(
        (actions.origin.y - type_heading.origin.y).abs() < px(1.),
        "the Actions heading sits on the same line as the others: \
         Actions y {:?}, Type y {:?}",
        actions.origin.y,
        type_heading.origin.y,
    );
    let heading = window
        .debug_bounds("rule:heading-row")
        .expect("the rules list paints its heading row");
    assert!(
        heading.size.height < px(24.),
        "the heading is a single line tall: it measures {:?}",
        heading.size.height,
    );
    assert!(
        heading.origin.y < row.origin.y,
        "the heading sits above the rows: heading y {:?}, row y {:?}",
        heading.origin.y,
        row.origin.y,
    );
}

// -- row context menu (2.9) -------------------------------------------------

/// A minimal row for the pure menu-model test.
fn browse_row(hidden: bool, paired: bool) -> TransactionRow {
    TransactionRow {
        id: 1,
        account: "American Express · Cobalt".into(),
        posted_date: "2026-10-31".into(),
        description: "COFFEE    VANCOUVER".into(),
        merchant: Some("COFFEE".into()),
        category: None,
        category_id: None,
        amount_cents: -450,
        hidden,
        transfer_pair_id: paired.then_some(2),
        category_locked: false,
    }
}

#[test]
fn menu_shows_hide_for_a_visible_row_and_unhide_only_for_a_hidden_one() {
    assert_eq!(
        row_menu_items(&browse_row(false, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Hide],
        "a visible row offers Hide, never Unhide"
    );
    assert_eq!(
        row_menu_items(&browse_row(true, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Unhide],
        "a hidden row offers Unhide, never Hide"
    );
    assert_eq!(
        row_menu_items(&browse_row(false, true)),
        [
            RowMenuItem::ChangeCategory,
            RowMenuItem::Hide,
            RowMenuItem::Unpair
        ],
        "a paired row also offers Unpair"
    );
    assert_eq!(
        row_menu_items(&browse_row(true, false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Unhide],
        "an unpaired row never offers Unpair"
    );
}

/// 3.29: the *Show rule* item finds the rule that decided the row, or says
/// why it cannot: a hand-set (locked) row, the built-in card payment, or no
/// rule matching the description. The reason rides in the item's label.
#[test]
fn show_rule_finds_the_deciding_rule_or_gives_the_disabled_reason() {
    let coffee = Rule {
        id: 7,
        pattern: "COFFEE".into(),
        is_regex: false,
        category_id: 3,
    };
    let rules = [coffee];
    // An unlocked row that a rule matches: the item works, plain label.
    let row = browse_row(false, false);
    assert_eq!(row_show_rule(&row, &rules, None), RowShowRule::Found(7));
    assert_eq!(row_show_rule(&row, &rules, None).menu_label(), "Show rule");
    // A locked row was set by hand, whatever the rules would say.
    let mut locked = browse_row(false, false);
    locked.category_locked = true;
    locked.category_id = Some(9);
    assert_eq!(row_show_rule(&locked, &rules, None), RowShowRule::SetByHand);
    assert_eq!(
        row_show_rule(&locked, &rules, Some(4)),
        RowShowRule::SetByHand
    );
    // Locked to the built-in card-payment category: its own reason, even
    // though a rule would have matched (card payments beat the rules).
    assert_eq!(
        row_show_rule(&locked, &rules, Some(9)),
        RowShowRule::CardPayment
    );
    // A locked row with no category at all still reads as hand-set.
    let mut locked_none = browse_row(false, false);
    locked_none.category_locked = true;
    assert_eq!(
        row_show_rule(&locked_none, &rules, Some(9)),
        RowShowRule::SetByHand
    );
    // Nothing matches: there is no rule to show.
    let mut unmatched = browse_row(false, false);
    unmatched.description = "PARKING FEE METRO".into();
    assert_eq!(row_show_rule(&unmatched, &rules, None), RowShowRule::NoRule);
    // The disabled labels say why, in the menu.
    assert_eq!(
        RowShowRule::SetByHand.menu_label(),
        "Show rule — Set by hand"
    );
    assert_eq!(
        RowShowRule::CardPayment.menu_label(),
        "Show rule — Card payment (built in)"
    );
    assert_eq!(
        RowShowRule::NoRule.menu_label(),
        "Show rule — No rule matches"
    );
}

/// The dashboard table's rows.
fn table_rows(shell: &Entity<Shell>, cx: &App) -> Vec<TransactionRow> {
    shell.read(cx).table.read(cx).delegate().rows.clone()
}

// ---------------------------------------------------------------------------
// Transfer pairing (task 4.2)

/// `Window::debug_bounds` takes a `&'static str`; dynamic selectors are
/// leaked (test processes are short-lived).
fn leaked(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Insert one row the way the importer's final insert does, returning its id.
fn seed_txn(
    conn: &financi_db::rusqlite::Connection,
    import: i64,
    account: i64,
    line: i64,
    date: &str,
    cents: i64,
    description: &str,
) -> i64 {
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents, description_raw)
         VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7)",
        (
            account,
            import,
            line,
            format!("hash-{account}-{line}"),
            date,
            cents,
            description,
        ),
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A minimal import row. The parser id must be a real one: unpairing rewrites
/// the account's master CSV, which resolves the parser of the account's
/// earliest import.
fn seed_import(conn: &financi_db::rusqlite::Connection, account: i64) -> i64 {
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-08', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [account],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// Chequing → savings tie: one debit on Alpha could pair with either of two
/// equal credits on Beta, so the engine opens exactly one review (on the
/// debit) and pairs nothing. Returns `(debit, credit_a, credit_b)`.
fn seed_transfer_tie(conn: &financi_db::rusqlite::Connection) -> (i64, i64, i64) {
    let alpha = create_account(conn, "simplii", "Alpha Chequing", AccountKind::Chequing).unwrap();
    let beta = create_account(conn, "simplii", "Beta Savings", AccountKind::Savings).unwrap();
    let ai = seed_import(conn, alpha);
    let bi = seed_import(conn, beta);
    let debit = seed_txn(
        conn,
        ai,
        alpha,
        0,
        "2026-11-05",
        -50_000,
        "INTERAC TRANSFER SEND",
    );
    let credit_a = seed_txn(
        conn,
        bi,
        beta,
        0,
        "2026-11-05",
        50_000,
        "INTERAC TRANSFER RECEIVE",
    );
    let credit_b = seed_txn(
        conn,
        bi,
        beta,
        1,
        "2026-11-06",
        50_000,
        "INTERAC TRANSFER RECEIVE",
    );
    financi_db::run_transfer_pairing(conn).unwrap();
    (debit, credit_a, credit_b)
}

/// Two accounts, one clean mutual transfer, paired by the engine at seed
/// time. Returns `(outgoing, incoming)`.
fn seed_paired_transfer(conn: &financi_db::rusqlite::Connection) -> (i64, i64) {
    let alpha = create_account(conn, "simplii", "Alpha Chequing", AccountKind::Chequing).unwrap();
    let beta = create_account(conn, "simplii", "Beta Savings", AccountKind::Savings).unwrap();
    let ai = seed_import(conn, alpha);
    let bi = seed_import(conn, beta);
    let out = seed_txn(
        conn,
        ai,
        alpha,
        0,
        "2026-11-05",
        -75_000,
        "INTERAC TRANSFER OUT",
    );
    let incoming = seed_txn(
        conn,
        bi,
        beta,
        0,
        "2026-11-06",
        75_000,
        "INTERAC TRANSFER IN",
    );
    financi_db::run_transfer_pairing(conn).unwrap();
    (out, incoming)
}

#[gpui::test]
fn the_transfers_page_offers_every_candidate_for_a_tie_and_pairs_the_chosen_one(
    cx: &mut TestAppContext,
) {
    let mut seeded = None;
    let (handle, shell) = open_shell(cx, |c| seeded = Some(seed_transfer_tie(c)));
    let Some((debit, credit_a, credit_b)) = seeded else {
        unreachable!()
    };

    // The tie is not guessed: one review, carried by the debit, listing both
    // credits in (posted_date, id) order; the pill counts it.
    wait_until(cx, "review appears", |cx| {
        shell.read(cx).transfer_reviews.len() == 1
    });
    let review = cx.update(|cx| shell.read(cx).transfer_reviews[0].clone());
    assert_eq!(review.transaction_id, debit);
    assert_eq!(
        review.candidates.iter().map(|c| c.id).collect::<Vec<_>>(),
        [credit_a, credit_b]
    );

    // The page opens with the review panel and one row per candidate.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Transfers, cx)));
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(window.debug_bounds("page:page-transfers").is_some());
    assert!(
        window
            .debug_bounds(leaked(format!("transfer-review:{debit}")))
            .is_some()
    );
    assert!(
        window
            .debug_bounds(leaked(format!("transfer-candidate:{debit}:{credit_a}")))
            .is_some()
    );
    assert!(
        window
            .debug_bounds(leaked(format!("transfer-candidate:{debit}:{credit_b}")))
            .is_some()
    );

    // Choosing a candidate pairs exactly that couple, categorizes both sides
    // *Between Accounts*, and clears the review (and the pill).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.pair_transfer(debit, credit_b, window, cx));
    });
    wait_until(cx, "review cleared", |cx| {
        shell.read(cx).transfer_reviews.is_empty()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_include_transfers(true, window, cx));
    });
    wait_until(cx, "all three rows in the table", |cx| {
        table_rows(&shell, cx).len() == 3
    });
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Paired transfer.".into(), false))
        );
        let rows = table_rows(&shell, cx);
        let find = |id: i64| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(find(debit).transfer_pair_id, Some(credit_b));
        assert_eq!(find(credit_b).transfer_pair_id, Some(debit));
        assert_eq!(find(debit).category.as_deref(), Some("Between Accounts"));
        assert_eq!(find(credit_b).category.as_deref(), Some("Between Accounts"));
        // The rejected candidate stays alone and uncategorized.
        assert_eq!(find(credit_a).transfer_pair_id, None);
        assert_eq!(find(credit_a).category, None);
    });
}

#[gpui::test]
fn marking_a_review_not_a_transfer_clears_it_and_blocks_re_pairing(cx: &mut TestAppContext) {
    let mut seeded = None;
    let (handle, shell) = open_shell(cx, |c| seeded = Some(seed_transfer_tie(c)));
    let Some((debit, _, _)) = seeded else {
        unreachable!()
    };
    wait_until(cx, "review appears", |cx| {
        shell.read(cx).transfer_reviews.len() == 1
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.mark_not_transfer(debit, window, cx));
    });
    wait_until(cx, "review gone", |cx| {
        shell.read(cx).transfer_reviews.is_empty()
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_include_transfers(true, window, cx));
    });
    wait_until(cx, "all three rows in the table", |cx| {
        table_rows(&shell, cx).len() == 3
    });
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Marked as not a transfer.".into(), false))
        );
        let rows = table_rows(&shell, cx);
        assert!(
            rows.iter().all(|r| r.transfer_pair_id.is_none()),
            "a marked-as-not-a-transfer row never pairs again"
        );
        assert!(rows.iter().all(|r| r.category.is_none()));
    });
}

#[gpui::test]
fn unpairing_a_row_clears_both_sides_and_they_stay_apart(cx: &mut TestAppContext) {
    let mut seeded = None;
    let (handle, shell) = open_shell(cx, |c| seeded = Some(seed_paired_transfer(c)));
    let Some((out, incoming)) = seeded else {
        unreachable!()
    };

    // The pair is linked (the table would hide it without the toggle).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_include_transfers(true, window, cx));
    });
    wait_until(cx, "paired rows in the table", |cx| {
        let rows = table_rows(&shell, cx);
        rows.len() == 2 && rows.iter().all(|r| r.transfer_pair_id.is_some())
    });

    // Unpair from one side: both partners are cleared…
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.unpair_row(out, window, cx));
    });
    wait_until(cx, "unpaired", |cx| {
        let rows = table_rows(&shell, cx);
        rows.len() == 2 && rows.iter().all(|r| r.transfer_pair_id.is_none())
    });
    cx.update(|cx| {
        assert_eq!(
            last_toast(&shell, cx),
            Some(("Unpaired transaction.".into(), false))
        );
        assert!(shell.read(cx).transfer_reviews.is_empty());
        let rows = table_rows(&shell, cx);
        let find = |id: i64| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(find(out).category, None);
        assert_eq!(find(incoming).category, None);
    });

    // …and the engine's re-run leaves them apart: no new review, no pair.
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(shell.read(cx).transfer_reviews.is_empty());
        let rows = table_rows(&shell, cx);
        assert!(rows.iter().all(|r| r.transfer_pair_id.is_none()));
    });
}

/// Import October onto a fresh Cobalt and return the busiest merchant with
/// a target row and every row id sharing that merchant.
fn import_and_top_merchant(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
) -> (String, i64, Vec<i64>) {
    wait_until(cx, "accounts", |cx| shell.read(cx).accounts.len() == 1);
    open_and_choose(
        cx,
        handle,
        shell,
        "amex-sep-oct.csv",
        "American Express · Cobalt",
    );
    wait_until(cx, "plan", |cx| plan_ready(shell, cx));
    click_import(cx, handle, shell);
    wait_until(cx, "import", |cx| shell.read(cx).row_count == 67);
    wait_until(cx, "queue populated", |cx| !shell.read(cx).queue.is_empty());
    cx.update(|cx| {
        let (m, _) = top_group(shell, cx);
        let group: Vec<i64> = table_rows(shell, cx)
            .iter()
            .filter(|r| merchant(&r.description) == m)
            .map(|r| r.id)
            .collect();
        let target = *group.first().unwrap();
        (m, target, group)
    })
}

#[gpui::test]
fn changing_a_rows_category_locks_it_and_offers_the_rule(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (m, target, group) = import_and_top_merchant(cx, handle, &shell);
    assert!(group.len() >= 2, "the merchant has several rows");
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    add_parent(cx, handle, &shell, "Home");
    add_child(cx, handle, &shell, "Groceries");
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_row_category(target, online, window, cx)
        });
    });
    wait_until(cx, "target categorized and offer up", |cx| {
        shell.read(cx).rule_offer == Some((target, online))
            && table_rows(&shell, cx)
                .iter()
                .any(|r| r.id == target && r.category.as_deref() == Some("Online"))
    });

    // A later rule for the same merchant moves the siblings but never the
    // manually-set (locked) row.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_pattern.update(cx, |i, cx| {
                i.set_value(m.clone(), window, cx);
            });
            let ix = s
                .child_choices
                .iter()
                .position(|(id, _)| *id == groceries)
                .unwrap();
            s.rule_target.update(cx, |sel, cx| {
                sel.set_selected_index(Some(gpui_kit::component::IndexPath::new(ix)), window, cx);
            });
            s.add_rule(window, cx);
        });
    });
    wait_until(cx, "siblings moved, locked target held", |cx| {
        let rows = table_rows(&shell, cx);
        let cat = |id: i64| {
            rows.iter()
                .find(|r| r.id == id)
                .and_then(|r| r.category.clone())
        };
        cat(target).as_deref() == Some("Online")
            && group
                .iter()
                .filter(|id| **id != target)
                .any(|id| cat(*id).as_deref() == Some("Groceries"))
    });
}

#[gpui::test]
fn applying_the_row_offer_categorizes_the_whole_merchant(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (m, target, group) = import_and_top_merchant(cx, handle, &shell);
    assert!(group.len() >= 2, "the merchant has several rows");
    add_parent(cx, handle, &shell, "Shopping");
    add_child(cx, handle, &shell, "Online");
    let online = cx.update(|cx| find_child(&shell, cx, "Online").unwrap());
    let all_uncategorized = cx.update(|cx| {
        table_rows(&shell, cx)
            .iter()
            .all(|r| !group.contains(&r.id) || r.category.is_none())
    });
    assert!(all_uncategorized, "the group starts uncategorized");

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_row_category(target, online, window, cx)
        });
    });
    wait_until(cx, "offer up", |cx| {
        shell.read(cx).rule_offer == Some((target, online))
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_row_rule(target, window, cx));
    });
    wait_until(cx, "rule created and offer cleared", |cx| {
        shell.read(cx).rule_offer.is_none() && shell.read(cx).rules.iter().any(|r| r.pattern == m)
    });
    let rows = cx.update(|cx| table_rows(&shell, cx));
    for id in &group {
        assert_eq!(
            rows.iter()
                .find(|r| r.id == *id)
                .and_then(|r| r.category.as_deref()),
            Some("Online"),
            "every row of {m} is now Online"
        );
    }
}

#[gpui::test]
fn hiding_and_unhiding_a_row_updates_the_table_and_sidebar_counts(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    });
    let (_merchant, target, _group) = import_and_top_merchant(cx, handle, &shell);
    let (before_total, before_uncategorized) = cx.update(|cx| {
        (
            shell.read(cx).total_visible,
            shell.read(cx).uncategorized_visible,
        )
    });
    let target_visible = cx.update(|cx| table_rows(&shell, cx).iter().any(|r| r.id == target));
    assert!(target_visible, "the target row is visible before hiding it");

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_row_hidden(target, true, window, cx));
    });
    wait_until(cx, "hidden row leaves table and counts", |cx| {
        !table_rows(&shell, cx).iter().any(|r| r.id == target)
            && shell.read(cx).total_visible == before_total - 1
            && shell.read(cx).uncategorized_visible == before_uncategorized - 1
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_row_hidden(target, false, window, cx));
    });
    wait_until(cx, "unhidden row returns to table and counts", |cx| {
        table_rows(&shell, cx).iter().any(|r| r.id == target)
            && shell.read(cx).total_visible == before_total
            && shell.read(cx).uncategorized_visible == before_uncategorized
    });
}

// ---------------------------------------------------------------------------
// Date range control (task 3.2)

/// Read one raw `settings` row through the shell's own DB worker. Jobs run
/// on the worker thread in order, so a write already queued by the shell
/// has landed by the time this returns.
/// Block this thread until `future` completes. The `DbWorker` answers from
/// its own OS thread, so no test-executor pumping is needed.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::sync::mpsc;
    use std::task::{Context, Poll, Wake, Waker};
    struct ThreadWaker(mpsc::Sender<()>);
    impl Wake for ThreadWaker {
        fn wake(self: std::sync::Arc<Self>) {
            let _ = self.0.send(());
        }
    }
    let (tx, rx) = mpsc::channel();
    let waker = Waker::from(std::sync::Arc::new(ThreadWaker(tx)));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        rx.recv().expect("waker vanished");
    }
}

fn db_setting(cx: &mut TestAppContext, shell: &Entity<Shell>, key: &str) -> Option<String> {
    let db = shell.update(cx, |s, _| s.db.clone());
    let key = key.to_string();
    block_on(db.call(move |c| {
        c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    }))
    .ok()
    .flatten()
}

#[gpui::test]
fn shell_starts_on_the_default_last_three_months(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |_| {});
    // Before the restore lands `view_filter` is still the open default;
    // narrower bounds prove the saved setting (here: none) was applied.
    wait_until(cx, "default date range restored", |cx| {
        shell.read(cx).view_filter.from > ViewFilter::default().from
    });
    let (from, to) = preset_range(DatePreset::Last3Months, Shell::today());
    let state = cx.update(|cx| {
        let s = shell.read(cx);
        (s.date_range, s.view_filter.from, s.view_filter.to)
    });
    assert_eq!(
        state,
        (DateRange::Preset(DatePreset::Last3Months), from, to)
    );
    // The default is not persisted; the setting only appears on change.
    assert_eq!(db_setting(cx, &shell, UI_DATE_RANGE), None);
}

#[gpui::test]
fn choosing_a_preset_updates_the_filter_and_persists(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "date range restored", |cx| {
        shell.read(cx).view_filter.from > ViewFilter::default().from
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::LastMonth, window, cx)
        });
    });

    let (from, to) = preset_range(DatePreset::LastMonth, Shell::today());
    let state = cx.update(|cx| {
        let s = shell.read(cx);
        (s.date_range, s.view_filter.from, s.view_filter.to)
    });
    assert_eq!(state, (DateRange::Preset(DatePreset::LastMonth), from, to));
    // Let the persist task run until it awaits the worker, so its write
    // job is queued ahead of the read below (the worker is FIFO).
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("last1"),
        "ui.date_range was written"
    );
}

/// One spend and one income row in the current month and one pair three
/// calendar months back (just outside "Last 3 months"), so the default
/// preset and "All time" show different totals. Dates are derived from
/// `Shell::today()`, so the test passes whenever it runs.
fn seed_preset_totals_rows(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    let salary =
        financi_db::categories::create_parent(conn, "Salary", CategoryKind::Income).unwrap();

    // Last day of the month three before the current one: always outside
    // "Last 3 months" (which opens on the first of two months back) and
    // exactly three month columns away, so the All-time grid is four
    // months wide whatever day the test runs.
    let today = Shell::today();
    let old = today
        .with_day(1)
        .expect("first of the month")
        .checked_sub_months(chrono::Months::new(2))
        .expect("two months back")
        .checked_sub_days(chrono::Days::new(1))
        .expect("last day of the month before that");
    let mut line = 0i64;
    let mut txn = |description: &str, date: NaiveDate, cents: i64, category: i64| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("preset-totals-hash-{line}"),
                date.format("%Y-%m-%d").to_string(),
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn("GROCER STORE OLD", old, -5000, groceries);
    txn("PAYROLL OLD", old, 100000, salary);
    txn("GROCER STORE CUR", today, -1500, groceries);
    txn("PAYROLL CUR", today, 210500, salary);
}

/// The month columns plus the `Total`, for the three total rows, and the
/// Food bar of the chart — everything the dashboard shows as a number.
fn dashboard_totals(
    shell: &Entity<Shell>,
    cx: &App,
) -> (Vec<String>, Vec<i64>, Vec<i64>, Vec<i64>, i64) {
    let s = shell.read(cx);
    let cells = |name: &str| {
        s.summary_rows()
            .into_iter()
            .find(|r| r.name == name)
            .expect("a total row")
            .cells
    };
    let food = s
        .chart_rows
        .iter()
        .find(|r| r.name == "Food")
        .expect("Food legend row")
        .spend_cents;
    (
        s.summary.months.clone(),
        cells("Income"),
        cells("Spend"),
        cells("Net"),
        food,
    )
}

#[gpui::test]
fn changing_the_preset_updates_the_dashboard_totals(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_preset_totals_rows);
    let today = Shell::today();
    let month_label = |n: u32| {
        today
            .checked_sub_months(chrono::Months::new(n))
            .expect("month back")
            .format("%Y-%m")
            .to_string()
    };
    let last3: Vec<String> = [2, 1, 0].iter().map(|n| month_label(*n)).collect();
    // All time clamps the grid to the data span: the old row's month plus
    // the three columns of the default range.
    let all_months: Vec<String> = [3, 2, 1, 0].iter().map(|n| month_label(*n)).collect();

    // Default "Last 3 months": only the current month's rows count; the
    // grid shows the three months of the range.
    wait_until(cx, "last-3-months totals", |cx| {
        shell.read(cx).row_count == 2 && shell.read(cx).summary.months == last3
    });
    let totals = cx.update(|cx| dashboard_totals(&shell, cx));
    assert_eq!(
        totals,
        (
            last3.clone(),
            vec![0, 0, 210500, 210500],
            vec![0, 0, 1500, 1500],
            vec![0, 0, 209000, 209000],
            1500,
        ),
        "only the current month is inside the default range"
    );

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });

    // "All time" pulls in the old pair: the grid clamps to the data span
    // and every number grows.
    wait_until(cx, "all-time totals", |cx| {
        shell.read(cx).row_count == 4 && shell.read(cx).summary.months == all_months
    });
    assert_eq!(
        cx.update(|cx| dashboard_totals(&shell, cx)),
        (
            all_months,
            vec![100000, 0, 0, 210500, 310500],
            vec![5000, 0, 0, 1500, 6500],
            vec![95000, 0, 0, 209000, 304000],
            6500,
        ),
        "the preset change moved the visible totals"
    );
}

#[gpui::test]
fn a_saved_custom_range_is_restored_at_launch(cx: &mut TestAppContext) {
    let from = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let (_handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "custom:2026-03-01..2026-03-31").unwrap();
    });
    wait_until(cx, "custom range restored", |cx| {
        shell.read(cx).date_range == DateRange::Custom { from, to }
    });
    let state = cx.update(|cx| {
        let s = shell.read(cx);
        (s.view_filter.from, s.view_filter.to)
    });
    assert_eq!(state, (from, to));
}

#[gpui::test]
fn a_garbage_setting_falls_back_to_the_default(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "last13").unwrap();
    });
    wait_until(cx, "garbage replaced by default", |cx| {
        shell.read(cx).view_filter.from > ViewFilter::default().from
    });
    let range = cx.update(|cx| shell.read(cx).date_range);
    assert_eq!(range, DateRange::Preset(DatePreset::Last3Months));
}

#[gpui::test]
fn picking_a_custom_range_persists_and_normalizes(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_until(cx, "date range restored", |cx| {
        shell.read(cx).view_filter.from > ViewFilter::default().from
    });

    // Switch to "Custom…", then pick both dates with the pickers.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::Custom, window, cx)
        });
    });
    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.date_picked(false, start, window, cx);
            s.date_picked(true, end, window, cx);
        });
    });
    // Let the persist task queue its write before reading (the worker is FIFO).
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("custom:2026-03-01..2026-03-31"),
        "the picker writes the custom range"
    );
    let state = cx.update(|cx| {
        let s = shell.read(cx);
        (s.date_range, s.view_filter.from, s.view_filter.to)
    });
    assert_eq!(
        state,
        (
            DateRange::Custom {
                from: start,
                to: end
            },
            start,
            end
        )
    );

    // Picking the end before the start persists the swapped pair, and the
    // pickers show the corrected order rather than the reversed pick.
    let early = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.date_picked(true, early, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("custom:2026-01-15..2026-03-01"),
        "a reversed pick is stored swapped"
    );
    let shown = cx.update(|cx| {
        let s = shell.read(cx);
        (
            s.view_filter.from,
            s.view_filter.to,
            s.date_from_picker.read(cx).date(),
            s.date_to_picker.read(cx).date(),
        )
    });
    assert_eq!(
        shown,
        (
            early,
            start,
            CalendarDate::Single(Some(early)),
            CalendarDate::Single(Some(start)),
        ),
        "the pickers show the swapped range"
    );

    // Picking the start after the end normalizes the same way.
    let late = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.date_picked(false, late, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_DATE_RANGE).as_deref(),
        Some("custom:2026-03-01..2026-09-01"),
        "the start handler normalizes too"
    );
}

// ---------------------------------------------------------------------------
// Accounts filter + transfers toggle (task 3.3)

/// Two Amex accounts: Cobalt gets a normal spend row and a
/// transfer-category row, Gold one normal spend row. With transfers off
/// the table starts at two rows, one per account.
fn seed_two_accounts_and_a_transfer(conn: &financi_db::rusqlite::Connection) {
    let cobalt = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let gold = create_account(conn, "amex", "Gold", AccountKind::CreditCard).unwrap();
    let import = |account: i64| -> i64 {
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
            [account],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    let txn =
        |account: i64, import: i64, line: i64, date: &str, cents: i64, category: Option<i64>| {
            conn.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, category_id)
                 VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, 'ROW', ?7)",
                (
                    account,
                    import,
                    line,
                    format!("hash-{account}-{line}"),
                    date,
                    cents,
                    category,
                ),
            )
            .unwrap();
        };
    let transfer = financi_db::categories::card_payment_category(conn).unwrap();
    let ci = import(cobalt);
    let gi = import(gold);
    txn(cobalt, ci, 0, "2026-11-03", -150, None);
    txn(cobalt, ci, 1, "2026-11-04", -5000, Some(transfer));
    txn(gold, gi, 0, "2026-11-05", -250, None);
}

#[gpui::test]
fn toggling_include_transfers_filters_rows_and_persists(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        shell.read(cx).row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(!s.include_transfers, "default off");
        assert!(!s.view_filter.include_transfers);
    });
    // The default is not persisted; the setting only appears on change.
    assert_eq!(db_setting(cx, &shell, UI_INCLUDE_TRANSFERS), None);

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_include_transfers(true, window, cx));
    });
    wait_until(cx, "transfer row appears", |cx| {
        shell.read(cx).row_count == 3 && shell.read(cx).view_filter.include_transfers
    });
    // Let the persist task queue its write before reading (the worker is FIFO).
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_INCLUDE_TRANSFERS).as_deref(),
        Some("1"),
        "ui.include_transfers was written"
    );

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_include_transfers(false, window, cx));
    });
    wait_until(cx, "transfer row hidden again", |cx| {
        shell.read(cx).row_count == 2
    });
    cx.run_until_parked();
    assert_eq!(
        db_setting(cx, &shell, UI_INCLUDE_TRANSFERS).as_deref(),
        Some("0")
    );
}

/// 3.14: a checked box draws its check from the `icons/check.svg` asset,
/// loaded through GPUI's application asset source. The app used to run with
/// GPUI's default source, which loads nothing, so the box changed background
/// but showed no check; it must register the bundled gpui-kit icons.
#[test]
fn a_checked_box_finds_its_check_glyph() {
    use gpui_kit::AssetSource as _;
    let check = IconName::Check.path();
    assert_eq!(check, "icons/check.svg");
    assert!(
        ().load(&check).ok().flatten().is_none(),
        "GPUI's default asset source — the bug's baseline — loads nothing"
    );
    let svg = crate::icon_assets()
        .load(&check)
        .ok()
        .flatten()
        .expect("the app registers the icon a checked Checkbox draws for its mark");
    assert!(std::str::from_utf8(&svg).is_ok_and(|s| s.contains("<svg")));
}

/// 3.13: sidebar nav items must not sit right of the section labels. The
/// gpui-kit nav row carries its own 8px left padding and the section label
/// had its own `px_2`, so both floated right of the single column the header
/// and footer form (`docs/ui-conventions.md`, *Sidebar*); the nav item and
/// the section label now render with one shared inset, pinned flush here.
/// Element padding lives inside the component's own row and is invisible to
/// the headless test surface, so this pins the shared constant both render
/// with rather than the rendered pixels.
#[test]
fn sidebar_nav_items_and_section_labels_share_one_flush_inset() {
    assert_eq!(
        NAV_ROW_INSET, 0.,
        "nav items and section labels must share one inset, flush with the \
         header and footer (`docs/ui-conventions.md`, *Sidebar*)"
    );
}

/// 3.35: the Options popover carries the old filter-row toggles — same ids,
/// same state and handlers. A real click opens it, all three checkboxes
/// paint, and Include transfers toggles the rows exactly as before.
#[gpui::test]
fn clicking_the_include_transfers_checkbox_toggles_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        shell.read(cx).row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    let trigger = window
        .debug_bounds("filters:options")
        .expect("the filter row renders the Options button");
    window.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    draw_frame(cx, handle);
    for sel in [
        "checkbox:include-transfers",
        "checkbox:uncategorized-only",
        "checkbox:show-hidden",
    ] {
        assert!(
            window.debug_bounds(sel).is_some(),
            "the Options popover renders {sel}"
        );
    }
    let bounds = window
        .debug_bounds("checkbox:include-transfers")
        .expect("the Options popover renders the include-transfers checkbox");
    let at = point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    );
    window.simulate_click(at, Modifiers::default());
    wait_until(cx, "click checks the box and shows the transfer", |cx| {
        shell.read(cx).include_transfers && shell.read(cx).row_count == 3
    });
    window.simulate_click(at, Modifiers::default());
    wait_until(cx, "clicking again unchecks", |cx| {
        !shell.read(cx).include_transfers && shell.read(cx).row_count == 2
    });
}

/// 3.35: the Options trigger counts its toggles — plain "Options" while
/// none are on, "Options · 2" when two are.
#[gpui::test]
fn the_options_button_counts_the_toggles_that_are_on(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "initial rows, transfers off", |cx| {
        shell.read(cx).row_count == 2 && shell.read(cx).accounts.len() == 2
    });
    let mut window = VisualTestContext::from_window(handle, cx);
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds("filters:options-btn:Options").is_some(),
        "with no toggle on, the button reads just \"Options\""
    );
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.set_include_transfers(true, window, cx);
            s.set_uncategorized_only(true, window, cx);
        });
    });
    wait_until(cx, "both toggles on", |cx| {
        shell.read(cx).include_transfers && shell.read(cx).uncategorized_only
    });
    draw_frame(cx, handle);
    assert!(
        window
            .debug_bounds("filters:options-btn:Options \u{b7} 2")
            .is_some(),
        "with two toggles on, the button reads \"Options \u{b7} 2\""
    );
}

#[gpui::test]
fn a_saved_include_transfers_setting_is_restored_at_launch(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        seed_two_accounts_and_a_transfer(c);
        settings::set(c, UI_INCLUDE_TRANSFERS, "1").unwrap();
    });
    // A fresh shell over the stored "1" re-opens with transfers included,
    // and the table query follows the restored filter.
    wait_until(cx, "saved transfers flag restored", |cx| {
        shell.read(cx).include_transfers && shell.read(cx).row_count == 3
    });
    cx.update(|cx| assert!(shell.read(cx).view_filter.include_transfers));
}

#[gpui::test]
fn unchecking_an_account_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_two_accounts_and_a_transfer);
    wait_until(cx, "accounts and rows loaded", |cx| {
        shell.read(cx).accounts.len() == 2 && shell.read(cx).row_count == 2
    });
    let (cobalt, gold) = cx.update(|cx| {
        let s = shell.read(cx);
        assert!(s.excluded_accounts.is_empty(), "default: all checked");
        assert!(s.view_filter.account_ids.is_empty(), "empty means all");
        let id = |name: &str| s.accounts.iter().find(|a| a.name == name).unwrap().id;
        (id("Cobalt"), id("Gold"))
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_account_checked(cobalt, false, window, cx));
    });
    wait_until(cx, "only Gold's row shows", |cx| {
        shell.read(cx).row_count == 1
            && table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Gold"))
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert_eq!(s.view_filter.account_ids, vec![gold]);
    });

    // Unchecking every account shows no rows (empty in `ViewFilter`
    // means all, so the table must not fall back to everything).
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_account_checked(gold, false, window, cx));
    });
    wait_until(cx, "nothing selected: empty table", |cx| {
        shell.read(cx).row_count == 0
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_account_checked(cobalt, true, window, cx));
    });
    wait_until(cx, "Cobalt's row back", |cx| {
        shell.read(cx).row_count == 1
            && table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Cobalt"))
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).view_filter.account_ids, vec![cobalt]);
    });
}

/// Task 3.17: `none_selected` must resolve against the accounts loaded *by
/// this* refresh, not the previous refresh's list. An import that adds an
/// account while every existing checkbox is unchecked leaves that new
/// account checked, so every view must show its rows as soon as the import
/// finishes — not stay blank until some other filter changes.
#[gpui::test]
fn an_import_that_adds_an_account_shows_it_with_every_box_unchecked(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
    });
    wait_until(cx, "Gold loaded and the all-time range restored", |cx| {
        shell.read(cx).accounts.len() == 1
            && shell.read(cx).date_range == DateRange::Preset(DatePreset::AllTime)
    });

    // Uncheck the only account: nothing is selected, so the views are
    // legitimately blank.
    let gold = cx.update(|cx| shell.read(cx).accounts[0].id);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_account_checked(gold, false, window, cx));
    });
    wait_until(cx, "no account checked", |cx| {
        shell.read(cx).excluded_accounts == [gold] && shell.read(cx).row_count == 0
    });

    // Import into a brand-new account: its checkbox is on, so the finished
    // import must fill the views with its rows on its own.
    import_demo_file(cx, handle, &shell, "amex-sep-nov.csv", "Amex");
    wait_until(
        cx,
        "the imported rows show without a further filter change",
        |cx| shell.read(cx).row_count == 100,
    );

    cx.update(|cx| {
        let s = shell.read(cx);
        let amex = s
            .accounts
            .iter()
            .find(|a| a.name == "Amex")
            .expect("the import created Amex");
        assert!(
            !s.excluded_accounts.contains(&amex.id),
            "a new account starts checked"
        );
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Amex")),
            "only the new account is selected, so only its rows show"
        );
        assert_eq!(s.summary.months, ["2026-09", "2026-10", "2026-11"]);
        assert!(
            !s.chart_rows.is_empty(),
            "the chart shows the imported spend"
        );
    });
}

/// Task 3.17 (regression): the refresh triggered when an import finishes
/// starts with the *previous* refresh's `self.accounts` — the database
/// already has the new account, the shell does not. Judging "no accounts
/// checked" from that stale list skipped the queries, so every view stayed
/// blank until some other filter changed; `none_selected` must resolve
/// against the accounts the refresh itself loads.
#[gpui::test]
fn a_refresh_resolves_the_account_filter_against_the_accounts_it_loads(cx: &mut TestAppContext) {
    // Gold's rows were imported long ago; Amex's import is the one that has
    // just landed, after the shell's previous refresh.
    let (handle, shell) = open_shell(cx, |c| {
        settings::set(c, UI_DATE_RANGE, "all").unwrap();
        let gold = create_account(c, "amex", "Gold", AccountKind::CreditCard).unwrap();
        let amex = create_account(c, "amex", "Amex", AccountKind::CreditCard).unwrap();
        for (account, line, date) in [
            (gold, 0, "2026-11-05"),
            (amex, 0, "2026-10-01"),
            (amex, 1, "2026-10-02"),
        ] {
            c.execute(
                "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                      encoding, has_bom, line_ending, trailing_newline, row_count)
                 VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
                [account],
            )
            .unwrap();
            let import = c.last_insert_rowid();
            c.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, category_id)
                 VALUES (?1, ?2, ?3, X'00', ?4, ?5, -100, 'ROW', NULL)",
                (
                    account,
                    import,
                    line,
                    format!("hash-{account}-{line}"),
                    date,
                ),
            )
            .unwrap();
        }
    });
    wait_until(cx, "both accounts and three rows loaded", |cx| {
        shell.read(cx).accounts.len() == 2 && shell.read(cx).row_count == 3
    });
    let gold = cx.update(|cx| {
        shell
            .read(cx)
            .accounts
            .iter()
            .find(|a| a.name == "Gold")
            .unwrap()
            .id
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            // The state the import's refresh starts from: the user has
            // unchecked every account the previous refresh showed — which
            // knew only Gold, not the account the import added.
            s.accounts.retain(|a| a.id == gold);
            s.excluded_accounts = vec![gold];
            s.sync_account_filter();
            s.refresh(window, cx);
        });
    });

    // The accounts loaded by this refresh include Amex, whose checkbox is on,
    // so its two rows show: the filter must not be judged from the stale list.
    wait_until(cx, "the checked new account's rows show", |cx| {
        shell.read(cx).row_count == 2
    });
    cx.update(|cx| {
        let s = shell.read(cx);
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.account.ends_with("Amex")),
            "only the new account is checked, so only its rows show"
        );
        let amex = s.accounts.iter().find(|a| a.name == "Amex").unwrap().id;
        assert_eq!(s.view_filter.account_ids, vec![amex]);
        assert!(!s.chart_rows.is_empty(), "the chart is not blank");
        assert!(!s.summary.months.is_empty(), "the summary is not blank");
    });
}

// Dashboard table search and toggles (task 3.4)

/// One account with a categorized spend, an uncategorized spend, a money-in
/// row and a hidden spend, so the search box and the two table-only toggles
/// each change which rows the table shows.
fn seed_table_filter_rows(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();

    let mut line = 0i64;
    let mut txn =
        |description: &str, merchant: Option<&str>, cents: i64, category: Option<i64>| -> i64 {
            conn.execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                           row_hash, posted_date, amount_cents,
                                           description_raw, merchant, category_id)
                 VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-05', ?5, ?6, ?7, ?8)",
                (
                    acct,
                    import,
                    line,
                    format!("hash-{line}"),
                    cents,
                    description,
                    merchant,
                    category,
                ),
            )
            .unwrap();
            line += 1;
            conn.last_insert_rowid()
        };
    txn("CORNER CAFE", None, -500, None);
    txn("GROCER STORE", Some("Fresh Mart"), -1200, Some(groceries));
    txn("PAYROLL DEPOSIT", None, 3000, None);
    let hidden = txn("HIDDEN SPEND", None, -700, None);
    financi_db::set_hidden(conn, hidden, true).unwrap();
}

#[gpui::test]
fn search_filters_the_table_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| shell.read(cx).row_count == 3);
    // A case-insensitive substring of the description.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_search("cafe", window, cx));
    });
    wait_until(cx, "one cafe row", |cx| shell.read(cx).row_count == 1);
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "CORNER CAFE");
    });
    // A match on the merchant column.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_search("fresh", window, cx));
    });
    wait_until(cx, "merchant match", |cx| {
        shell.read(cx).row_count == 1
            && table_rows(&shell, cx)[0].merchant.as_deref() == Some("Fresh Mart")
    });
    // Clearing the box brings every row back.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_search("", window, cx));
    });
    wait_until(cx, "all rows back", |cx| shell.read(cx).row_count == 3);
}

#[gpui::test]
fn uncategorized_only_toggle_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three rows", |cx| shell.read(cx).row_count == 3);
    cx.update(|cx| assert!(!shell.read(cx).uncategorized_only));

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_uncategorized_only(true, window, cx));
    });
    wait_until(cx, "two uncategorized rows", |cx| {
        shell.read(cx).row_count == 2
    });
    cx.update(|cx| {
        assert!(shell.read(cx).uncategorized_only);
        assert!(
            table_rows(&shell, cx)
                .iter()
                .all(|r| r.category_id.is_none()),
            "only uncategorized rows remain"
        );
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_uncategorized_only(false, window, cx));
    });
    wait_until(cx, "three rows back", |cx| shell.read(cx).row_count == 3);
}

#[gpui::test]
fn show_hidden_toggle_reveals_hidden_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "hidden row not shown by default", |cx| {
        shell.read(cx).row_count == 3
    });
    cx.update(|cx| {
        assert!(!shell.read(cx).show_hidden);
        assert!(table_rows(&shell, cx).iter().all(|r| !r.hidden));
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_show_hidden(true, window, cx));
    });
    wait_until(cx, "hidden row appears", |cx| shell.read(cx).row_count == 4);
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        let hidden = rows
            .iter()
            .find(|r| r.hidden)
            .expect("showing hidden reveals the hidden row");
        assert_eq!(hidden.description, "HIDDEN SPEND");
    });
}

#[gpui::test]
fn sorting_the_amount_header_reorders_rows_without_a_reentrant_panic(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_table_filter_rows);
    wait_until(cx, "three visible rows", |cx| shell.read(cx).row_count == 3);

    let descriptions = |cx: &App| -> Vec<String> {
        table_rows(&shell, cx)
            .iter()
            .map(|r| r.description.clone())
            .collect()
    };
    // Newest-first default; every row shares a date, so they fall back to
    // source order.
    assert_eq!(
        cx.read(descriptions),
        ["CORNER CAFE", "GROCER STORE", "PAYROLL DEPOSIT"]
    );

    // Drive the header sort exactly as the table's click listener does:
    // `perform_sort` runs while the table entity is leased and hands the new
    // sort to the shell, which must not update that same entity in response
    // (that double-lease is the crash). The click test-support gpui-kit ships
    // is disabled in this crate, so we enter the sort through the same
    // delegate call the header's `on_click` makes, under the table's lease.
    let table = cx.update(|cx| shell.read(cx).table.clone());
    with_window(cx, handle, |window, cx| {
        table.update(cx, |t, cx| {
            t.delegate_mut()
                .perform_sort(4, ColumnSort::Descending, window, cx);
        });
    });

    // The shell adopted the sort and the async re-query reordered by amount
    // descending: +3000, -500, -1200.
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).tx_sort,
            TxSort {
                column: TxSortColumn::Amount,
                descending: true,
            }
        );
    });
    wait_until(cx, "rows reordered by amount", |cx| {
        descriptions(cx) == ["PAYROLL DEPOSIT", "CORNER CAFE", "GROCER STORE"]
    });
}

// ------------------------------------------------------------------ 3.5 chart

/// Food (Groceries -1000, Restaurants -500), Transport (Bus -300) and one
/// uncategorized -200, all inside the default date range: the parent chart
/// rows are Food 1500, Transport 300, Uncategorized 200.
fn seed_chart_rows(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    let restaurants = financi_db::categories::create_child(conn, food, "Restaurants").unwrap();
    let transport =
        financi_db::categories::create_parent(conn, "Transport", CategoryKind::Expense).unwrap();
    let bus = financi_db::categories::create_child(conn, transport, "Bus").unwrap();

    let mut line = 0i64;
    let mut txn = |description: &str, cents: i64, category: Option<i64>| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, '2026-11-05', ?5, ?6, ?7)",
            (
                acct,
                import,
                line,
                format!("chart-hash-{line}"),
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn("GROCER STORE", -1000, Some(groceries));
    txn("CORNER CAFE", -500, Some(restaurants));
    txn("CITY BUS", -300, Some(bus));
    txn("Mystery Vendor", -200, None);
}

fn click_chart_legend(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    id: Option<i64>,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.click_legend_row(id, window, cx));
    });
}

fn top_category_id(shell: &Entity<Shell>, cx: &App, name: &str) -> i64 {
    shell
        .read(cx)
        .chart_rows
        .iter()
        .find(|r| r.name == name)
        .expect("chart row")
        .id
        .expect("a real category")
}

/// The legend draws no numbers since 3.31, but the model rows still
/// carry each parent's amount and share — that is what the bar and
/// legend-row tooltips show.
#[gpui::test]
fn chart_rows_carry_amounts_and_shares_for_the_tooltips(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).chart_rows.len() == 3
    });
    cx.update(|cx| {
        let rows = &shell.read(cx).chart_rows;
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Food", "Transport", "Uncategorized"]);
        let cents: Vec<i64> = rows.iter().map(|r| r.spend_cents).collect();
        assert_eq!(cents, [1500, 300, 200]);
        for (row, share) in rows.iter().zip([0.75, 0.15, 0.10]) {
            assert!(
                (row.share - share).abs() < 1e-9,
                "{} share {}",
                row.name,
                row.share
            );
        }
    });
}

#[gpui::test]
fn clicking_a_parent_drills_the_chart_and_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 4);
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));

    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "chart drilled to Food's children", |cx| {
        shell.read(cx).chart_parent == Some(food) && shell.read(cx).row_count == 2
    });
    cx.update(|cx| {
        let names: Vec<&str> = shell
            .read(cx)
            .chart_rows
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["Groceries", "Restaurants"]);
        assert_eq!(
            shell.read(cx).legend_selection,
            Some(LegendSelection::Category(food))
        );
        let rows = table_rows(&shell, cx);
        let mut descs: Vec<&str> = rows.iter().map(|r| r.description.as_str()).collect();
        descs.sort_unstable();
        assert_eq!(descs, ["CORNER CAFE", "GROCER STORE"]);
    });
}

#[gpui::test]
fn all_categories_row_clears_drill_and_filter(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 4);
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled", |cx| {
        shell.read(cx).chart_parent == Some(food)
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_all_categories(window, cx));
    });
    wait_until(cx, "back to parents and all rows", |cx| {
        shell.read(cx).chart_parent.is_none() && shell.read(cx).row_count == 4
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).legend_selection, None);
        assert_eq!(shell.read(cx).chart_rows.len(), 3);
    });
}

#[gpui::test]
fn child_click_filters_further_and_a_second_click_clears(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 4);
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled into Food", |cx| {
        shell.read(cx).chart_parent == Some(food)
    });
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "only the Groceries row", |cx| {
        shell.read(cx).row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "GROCER STORE");
        assert_eq!(
            shell.read(cx).legend_selection,
            Some(LegendSelection::Category(groceries))
        );
        // The chart stays on the children.
        assert_eq!(shell.read(cx).chart_parent, Some(food));
    });

    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "filter cleared, children still shown", |cx| {
        shell.read(cx).legend_selection.is_none() && shell.read(cx).row_count == 4
    });
    cx.update(|cx| assert_eq!(shell.read(cx).chart_parent, Some(food)));
}

#[gpui::test]
fn uncategorized_legend_row_filters_the_table(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 4);

    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "only the uncategorized row", |cx| {
        shell.read(cx).row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "Mystery Vendor");
        assert_eq!(
            shell.read(cx).legend_selection,
            Some(LegendSelection::Uncategorized)
        );
        // The pseudo-row never drills the chart.
        assert!(shell.read(cx).chart_parent.is_none());
        assert_eq!(shell.read(cx).chart_rows.len(), 3);
    });

    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "all rows back", |cx| shell.read(cx).row_count == 4);
    cx.update(|cx| assert!(shell.read(cx).legend_selection.is_none()));
}

#[gpui::test]
fn escape_clears_the_drill_and_the_selection(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 4);
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "drilled with a selection", |cx| {
        shell.read(cx).chart_parent == Some(food)
            && shell.read(cx).legend_selection == Some(LegendSelection::Category(food))
    });

    // Actions dispatch along the focus path, so focus the search input
    // first — like a user who clicked into the window. The real Esc
    // keybinding (registered in `Shell::new`) must map the keystroke to
    // `ClearCategorySelection`; dispatching the action directly would not
    // test the binding.
    with_window(cx, handle, |window, cx| {
        let input = shell.read(cx).search_input.read(cx).focus_handle(cx);
        window.focus(&input, cx);
    });
    cx.simulate_keystrokes(handle, "escape");
    // The state clears synchronously in the action handler; the rows
    // follow when the refresh it started lands, so wait for both.
    wait_until(cx, "drill, selection and rows cleared", |cx| {
        shell.read(cx).chart_parent.is_none()
            && shell.read(cx).legend_selection.is_none()
            && shell.read(cx).row_count == 4
    });
}

// -- Responsive charts (3.15) -------------------------------------------

#[test]
fn band_labels_truncate_to_the_character_budget() {
    let names = vec![
        "Food".to_string(),
        "Public Transport Costs".to_string(),
        "Café du Marché".to_string(),
    ];
    let shorts = short_chart_labels(&names, 8);
    assert_eq!(shorts[0].as_str(), "Food", "a short name is left alone");
    assert_eq!(shorts[1].as_str(), "Public…");
    assert_eq!(shorts[2].as_str(), "Café du…", "truncates by characters");

    // No budget means no truncation.
    let all = short_chart_labels(&names, 0);
    assert_eq!(all[1].as_str(), "Public Transport Costs");
}

#[test]
fn colliding_short_labels_fall_back_to_their_full_names() {
    // The band text is also `BarChart`'s band key, so two rows must never
    // be shortened to the same string.
    let names = vec![
        "Travel International".to_string(),
        "Travel Domestic".to_string(),
    ];
    let shorts = short_chart_labels(&names, 8);
    assert_eq!(shorts[0].as_str(), "Travel International");
    assert_eq!(shorts[1].as_str(), "Travel Domestic");

    // Without a collision the shortening stands.
    let names = vec!["Travel International".to_string(), "Food".to_string()];
    assert_eq!(short_chart_labels(&names, 8)[0].as_str(), "Travel…");
}

#[test]
fn band_label_budget_keeps_labels_inside_their_spacing() {
    // A wide plot with few bands: label every band with room to spare.
    let (chars, labels) = band_label_budget(779., 7);
    assert_eq!(labels, 7);
    assert!(chars >= 12, "{chars} chars at 779px across 7 bands");

    // The minimum-window plot (~167px) with 7 bands: too tight for all
    // seven, so labels are thinned out but still legible.
    let (chars, labels) = band_label_budget(167., 7);
    assert!((1..7).contains(&labels), "{labels} labels at 167px");
    assert!(chars >= CHART_LABEL_MIN_CHARS, "{chars}");

    // The invariant for every window size and category count: each drawn
    // label owns `chart_width / labels` pixels, enough for its characters
    // plus the gap, so centred labels cannot overlap or run past the
    // plot's edges.
    for width in [100., 167., 400., 779., 1600., 3000.] {
        for count in 1..=30usize {
            let (chars, labels) = band_label_budget(width, count);
            assert!(
                labels >= 1 && labels <= count,
                "width {width}, count {count}"
            );
            assert!((1..=CHART_LABEL_MAX_CHARS).contains(&chars));
            let per = width / labels as f32;
            assert!(
                chars as f32 * CHART_CHAR_PX + CHART_LABEL_GAP <= per + 0.001,
                "width {width}, count {count}: {chars} chars need more than {per:.1}px"
            );
        }
    }
    assert_eq!(band_label_budget(779., 0), (0, 0));
}

// -- Bar share labels and the slim legend (3.31) ------------------------

#[test]
fn bar_share_labels_read_as_whole_percents() {
    assert_eq!(bar_share_label(0.75), "75%");
    assert_eq!(bar_share_label(0.182), "18%");
    assert_eq!(bar_share_label(1.0), "100%");
    assert_eq!(
        bar_share_label(0.999),
        "100%",
        "rounds up to a whole percent"
    );
    assert_eq!(
        bar_share_label(0.01),
        "1%",
        "one percent is a whole percent"
    );
    assert_eq!(
        bar_share_label(0.009),
        "0.9%",
        "below one percent, one decimal"
    );
    assert_eq!(bar_share_label(0.004), "0.4%");
    assert_eq!(bar_share_label(0.0), "0%");
    assert_eq!(
        bar_share_label(0.0095),
        "0.9%",
        "0.95–1% still reads with its one decimal"
    );
    assert_eq!(bar_share_label(0.0094), "0.9%");
    assert_eq!(
        bar_share_label(0.0099),
        "1.0%",
        "0.99% rounds to one percent with its decimal, not a bare 1%"
    );
    assert_eq!(
        bar_share_label(0.0004),
        "0.1%",
        "a sliver below 0.05% ($1 of $2,500) rounds up, never to 0.0%"
    );
    assert_eq!(bar_share_label(0.00004), "0.1%");

    // The tooltip body is the bar's and the legend row's one string.
    assert_eq!(
        chart_tooltip_value(123456, 0.182, " of Food"),
        "$1,234.56 · 18.2% of Food"
    );
    assert_eq!(chart_tooltip_value(1500, 0.75, ""), "$15.00 · 75.0%");
}

#[test]
fn bar_label_plan_thins_share_labels_without_collisions() {
    // A wide plot with few bands: every bar keeps its share label.
    assert_eq!(bar_label_plan(779., 7), 7);
    // The 1200 × 720 plot (~247 px) with 15 bars: too narrow for all
    // fifteen, so the labels are thinned out.
    let labels = bar_label_plan(247., 15);
    assert!((1..15).contains(&labels), "{labels} labels at 247px");
    assert_eq!(bar_label_plan(0., 5), 1, "a degenerate plot labels one bar");
    assert_eq!(bar_label_plan(100., 0), 0);
    assert_eq!(bar_label_plan(100., 1), 1);

    // The invariant for every window size and category count: drawn
    // labels are spread first-to-last and their band centres sit at
    // least a full label apart, so centred labels cannot overlap.
    let need = BAR_LABEL_MAX_CHARS as f32 * CHART_CHAR_PX;
    for width in [100., 247., 400., 779., 1600., 3000.] {
        for count in 1..=30usize {
            let labels = bar_label_plan(width, count);
            assert!(
                (1..=count).contains(&labels),
                "width {width}, count {count}: {labels} labels"
            );
            let slots = bar_label_slots(count, labels);
            let drawn: Vec<usize> = slots
                .iter()
                .enumerate()
                .filter(|(_, on)| **on)
                .map(|(ix, _)| ix)
                .collect();
            assert_eq!(drawn.len(), labels, "width {width}, count {count}");
            assert_eq!(drawn.first(), Some(&0), "the first bar is labelled");
            assert_eq!(drawn.last(), Some(&(count - 1)), "the last bar is labelled");
            let band_w = width / count as f32;
            for pair in drawn.windows(2) {
                let spacing = (pair[1] - pair[0]) as f32 * band_w;
                assert!(
                    spacing >= need - 0.001,
                    "width {width}, count {count}: labels on bars {} and {} \
                     are {spacing:.1}px apart, need {need:.1}",
                    pair[0],
                    pair[1],
                );
            }
        }
    }
}

#[gpui::test]
fn chart_labels_adapt_to_the_window_size(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).chart_rows.len() == 3
    });

    // `open_shell` opens 1100 wide: with the legend beside it the plot is
    // narrow, so the labels are truncated to a small character budget.
    let (small_chars, small_labels) = cx.update(|cx| shell.read(cx).chart_label_plan());
    assert_eq!(small_labels, 3);
    assert!(
        small_chars < CHART_LABEL_MAX_CHARS,
        "a narrow window truncates, got {small_chars} chars"
    );

    // A large window: every band gets its label again, and the labels
    // grow. The window-bounds observer must have updated the plan.
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(2000.), px(900.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| shell.read(cx).window_width), 2000.);
    let (big_chars, big_labels) = cx.update(|cx| shell.read(cx).chart_label_plan());
    assert_eq!(big_labels, 3, "a wide window labels every band");
    assert!(big_chars > small_chars, "{big_chars} vs {small_chars}");

    // And the chart draws at both the minimum and a large window.
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(1200.), px(720.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
    with_window(cx, handle, |window, cx| {
        window.resize(size(px(2560.), px(1440.)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();
    draw_frame(cx, handle);
}

#[gpui::test]
fn the_chart_renders_long_names_and_many_categories(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).chart_rows.len() == 3
    });
    // 25 categories with long names — the worst case for the band axis.
    shell.update(cx, |s, cx| {
        s.chart_rows = (0..25)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Category number {i} with a very long descriptive name"),
                spend_cents: (i + 1) * 12_345,
                share: 0.04,
            })
            .collect();
        cx.notify();
    });
    // The plan stays bounded, and the chart draws without panic at the
    // test window's width and at the minimum size. 25 bands across the
    // narrow plot must thin the labels out, not squeeze them flat.
    let (chars, labels) = cx.update(|cx| shell.read(cx).chart_label_plan());
    assert!((1..25).contains(&labels), "{labels} labels for 25 bands");
    assert!((1..=CHART_LABEL_MAX_CHARS).contains(&chars));
    draw_frame(cx, _handle);
}

/// The legend column is just the key (3.31), so it must fit 140 px.
const _: () = assert!(LEGEND_W <= 140.);

/// 3.31: the legend is just swatch and name. The amount (76 px) and share
/// (52 px) columns are gone, the column is 140 px wide, and the plot gets
/// the width back — at 1200 × 720 it is far wider than the ~167 px the
/// 220 px legend left it.
#[gpui::test]
fn the_legend_is_just_a_key(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let rows: Vec<(String, i64, f64)> = cx.update(|cx| {
        shell
            .read(cx)
            .chart_rows
            .iter()
            .map(|r| (r.name.to_string(), r.spend_cents, r.share))
            .collect()
    });
    assert_eq!(rows.len(), 3);

    let legend = window
        .debug_bounds("chart:legend")
        .expect("the legend paints");
    assert!(
        f32::from(legend.size.width) <= LEGEND_W + 0.5,
        "the legend column is {:?} wide, expected ≤ {LEGEND_W}",
        legend.size.width,
    );
    let plot = window.debug_bounds("chart:plot").expect("the plot paints");
    assert!(
        f32::from(plot.size.width) >= 230.,
        "the plot is {:?} wide at 1200 × 720 — the 220 px legend left it ~167 px",
        plot.size.width,
    );

    // Each row draws its name and nothing else: no amount, no percentage.
    for (ix, (name, cents, share)) in rows.iter().enumerate() {
        assert!(
            window
                .debug_bounds(leaked(format!("chart:legend-text:{ix}={name}")))
                .is_some(),
            "legend row {ix} draws its name"
        );
        for gone in [crate::flow::money(*cents), format!("{:.1}%", share * 100.)] {
            assert!(
                window
                    .debug_bounds(leaked(format!("chart:legend-text:{ix}={gone}")))
                    .is_none(),
                "legend row {ix} no longer draws {gone:?}"
            );
        }
    }
}

/// 3.31: hovering a legend row shows the bar's own tooltip text — full
/// name, exact amount and share.
#[gpui::test]
fn hovering_a_legend_row_shows_amount_and_share(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).chart_rows.len() == 3
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);
    let value = cx.update(|cx| {
        let r = &shell.read(cx).chart_rows[0];
        assert_eq!(r.name.as_str(), "Food");
        chart_tooltip_value(r.spend_cents, r.share, "")
    });
    assert_eq!(value, "$15.00 · 75.0%");
    let tip_sel = leaked(format!("chart:legend-tooltip:Food={value}"));
    assert!(
        window.debug_bounds(tip_sel).is_none(),
        "nothing hovers a row, so no tooltip is drawn yet"
    );

    let row = window
        .debug_bounds("chart:legend-row:0")
        .expect("the first legend row paints");
    let centre = point(
        row.origin.x + row.size.width / 2.,
        row.origin.y + row.size.height / 2.,
    );
    window.simulate_mouse_move(centre, None, Modifiers::default());
    // The tooltip appears after a 500 ms hover; the test clock is fake.
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    draw_frame(cx, handle);
    assert!(
        window.debug_bounds(tip_sel).is_some(),
        "hovering the legend row shows a tooltip with the amount and share",
    );
}

/// 3.31: at the minimum window with 15 categories the bars are too narrow
/// for every share label, so the plan thins them the way band labels are
/// thinned (3.15) — and every drawn label, centred over its bar, stays
/// inside the chart panel and clear of its neighbours.
#[gpui::test]
fn fifteen_category_bars_keep_their_share_labels_clear(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "three chart rows", |cx| {
        shell.read(cx).chart_rows.len() == 3
    });
    shell.update(cx, |s, cx| {
        s.chart_rows = (0..15)
            .map(|i| CategorySpend {
                id: Some(i),
                name: format!("Cat {i:02}"),
                spend_cents: (i + 1) * 10_000,
                // The share of the total this row's spend represents, so
                // every bar's share label is a different string.
                share: (i + 1) as f64 / 120.,
            })
            .collect();
        cx.notify();
    });
    resize_and_draw(cx, handle, 1200., 720.);
    let mut window = VisualTestContext::from_window(handle, cx);

    let plot = window.debug_bounds("chart:plot").expect("the plot paints");
    let panel = window
        .debug_bounds("panel:chart")
        .expect("the chart panel paints");
    let plan_w = cx.update(|cx| shell.read(cx).chart_plot_width());
    assert!(
        (f32::from(plot.size.width) - plan_w).abs() <= 3.,
        "the plan measured the plot at {plan_w:.1}px, it painted {:?} \
         (the panel's 1 px borders are the only expected slack)",
        plot.size.width,
    );

    // The bars are too narrow for fifteen labels: the plan thins them.
    let plot_w = f32::from(plot.size.width);
    let labels = bar_label_plan(plot_w, 15);
    assert!((1..15).contains(&labels), "{labels} labels for 15 bars");

    // Drawn labels: first-to-last, centres at least a whole label apart,
    // and the first and last ones inside the panel's borders even when
    // centred over the edge bars.
    let slots = bar_label_slots(15, labels);
    let drawn: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .map(|(ix, _)| ix)
        .collect();
    assert_eq!(drawn.first(), Some(&0));
    assert_eq!(drawn.last(), Some(&14));

    // The wiring (3.31): the rows `chart_panel` hands the chart carry
    // their own share's label on the planned bars and nothing on the
    // thinned-out ones — drop the label call in `chart_bars` or misalign
    // it and this fails.
    let bars = cx.update(|cx| {
        shell
            .read(cx)
            .chart_bars(plan_w, &Default::default(), gpui::black())
    });
    assert_eq!(bars.len(), 15);
    let wired_slots = bar_label_slots(15, bar_label_plan(plan_w, 15));
    for (ix, bar) in bars.iter().enumerate() {
        let want = if wired_slots[ix] {
            bar_share_label(bar.share)
        } else {
            String::new()
        };
        assert_eq!(bar.bar_label.to_string(), want, "bar {ix}");
    }
    assert!(
        !bars[0].bar_label.is_empty() && !bars[14].bar_label.is_empty(),
        "the first and last bars carry their share"
    );

    let band_w = plot_w / 15.;
    let half = BAR_LABEL_MAX_CHARS as f32 * CHART_CHAR_PX / 2.;
    for pair in drawn.windows(2) {
        assert!(
            (pair[1] - pair[0]) as f32 * band_w >= 2. * half,
            "the labels on bars {} and {} overlap: {:.1}px apart",
            pair[0],
            pair[1],
            (pair[1] - pair[0]) as f32 * band_w,
        );
    }
    let first_left = plot.origin.x + px(0.5 * band_w - half);
    let last_right = plot.origin.x + px(14.5 * band_w + half);
    assert!(
        f32::from(first_left) >= f32::from(panel.origin.x),
        "the first bar's label spills past the panel's left border",
    );
    assert!(
        f32::from(last_right) <= f32::from(panel.origin.x + panel.size.width),
        "the last bar's label spills past the panel's right border",
    );
}

/// `seed_chart_rows` plus a childless Gifts parent whose only transaction
/// (2026-10-05) falls in the default range but not in "this month", so a
/// preset change can empty a selection out from under the legend.
fn seed_chart_rows_split(conn: &financi_db::rusqlite::Connection) {
    seed_chart_rows(conn);
    let gifts =
        financi_db::categories::create_parent(conn, "Gifts", CategoryKind::Expense).unwrap();
    let acct = conn
        .query_row("SELECT id FROM accounts WHERE name = 'Cobalt'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    let import = conn
        .query_row("SELECT id FROM imports LIMIT 1", [], |r| r.get::<_, i64>(0))
        .unwrap();
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents,
                                   description_raw, category_id)
         VALUES (?1, ?2, 9, X'00', 'chart-hash-gifts', '2026-10-05', -400, 'GIFT SHOP', ?3)",
        (acct, import, gifts),
    )
    .unwrap();
}

fn set_uncategorized_only(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    on: bool,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.set_uncategorized_only(on, window, cx));
    });
}

#[gpui::test]
fn a_filter_change_drops_the_stale_legend_selection(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows_split);
    wait_until(cx, "four chart rows", |cx| {
        shell.read(cx).chart_rows.len() == 4
    });
    let gifts = cx.update(|cx| top_category_id(&shell, cx, "Gifts"));

    // Gifts has no children: the drill snaps back to the top level while
    // the selection (and the table filter) stay on Gifts.
    click_chart_legend(cx, handle, &shell, Some(gifts));
    wait_until(cx, "Gifts selected, table filtered", |cx| {
        shell.read(cx).legend_selection == Some(LegendSelection::Category(gifts))
            && shell.read(cx).chart_parent.is_none()
            && shell.read(cx).row_count == 1
    });

    // "This month" drops the October Gifts spend: the stale selection must
    // go with it, and the table must widen back to the remaining rows.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::ThisMonth, window, cx)
        });
    });
    wait_until(cx, "selection and drill cleared, rows back", |cx| {
        shell.read(cx).legend_selection.is_none()
            && shell.read(cx).chart_parent.is_none()
            // Four rows remain this month: two Food, one Transport, one
            // uncategorized (the October Gifts row is out of range).
            && shell.read(cx).row_count == 4
    });
    cx.update(|cx| {
        let names: Vec<&str> = shell
            .read(cx)
            .chart_rows
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["Food", "Transport", "Uncategorized"]);
    });
}

#[gpui::test]
fn the_uncategorized_toggle_and_a_category_selection_are_exclusive(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 4);
    let food = cx.update(|cx| top_category_id(&shell, cx, "Food"));

    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "only the uncategorized row", |cx| {
        shell.read(cx).row_count == 1
    });

    // A category legend row wins: the checkbox goes off, the table shows
    // the category again.
    click_chart_legend(cx, handle, &shell, Some(food));
    wait_until(cx, "Food filters the table", |cx| {
        !shell.read(cx).uncategorized_only
            && shell.read(cx).legend_selection == Some(LegendSelection::Category(food))
            && shell.read(cx).row_count == 2
    });

    // Toggling the checkbox back on clears the category selection (the
    // chart stays drilled) instead of stacking both clauses.
    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "uncategorized only again", |cx| {
        shell.read(cx).uncategorized_only
            && shell.read(cx).legend_selection.is_none()
            && shell.read(cx).row_count == 1
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).chart_parent, Some(food));
        let rows = table_rows(&shell, cx);
        assert!(rows.iter().all(|r| r.category_id.is_none()));
    });
}

// ------------------------------------------------------- 3.6 monthly summary

/// Two fixed months (March and April 2026): Food (Groceries -1000 -600,
/// Restaurants -500), Transport (Bus -300), one uncategorized -200, and a
/// $2,105.00 salary each month. The tests switch to "All time", so the
/// grid covers exactly the seeded months whatever today's date is.
fn seed_summary_rows(conn: &financi_db::rusqlite::Connection) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-01-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let food = financi_db::categories::create_parent(conn, "Food", CategoryKind::Expense).unwrap();
    let groceries = financi_db::categories::create_child(conn, food, "Groceries").unwrap();
    let restaurants = financi_db::categories::create_child(conn, food, "Restaurants").unwrap();
    let transport =
        financi_db::categories::create_parent(conn, "Transport", CategoryKind::Expense).unwrap();
    let bus = financi_db::categories::create_child(conn, transport, "Bus").unwrap();
    let salary =
        financi_db::categories::create_parent(conn, "Salary", CategoryKind::Income).unwrap();

    let mut line = 0i64;
    let mut txn = |description: &str, date: &str, cents: i64, category: Option<i64>| {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("summary-hash-{line}"),
                date,
                cents,
                description,
                category,
            ),
        )
        .unwrap();
        line += 1;
    };
    txn("GROCER STORE MAR", "2026-03-10", -1000, Some(groceries));
    txn("CORNER CAFE MAR", "2026-03-10", -500, Some(restaurants));
    txn("Mystery Vendor", "2026-03-10", -200, None);
    txn("PAYROLL DEPOSIT", "2026-03-15", 210500, Some(salary));
    txn("GROCER STORE APR", "2026-04-10", -600, Some(groceries));
    txn("CITY BUS", "2026-04-10", -300, Some(bus));
    txn("PAYROLL DEPOSIT", "2026-04-15", 210500, Some(salary));
}

/// [`seed_summary_rows`] plus a large uncategorized March purchase, so the
/// uncategorized total beats every category's and the query — which orders
/// rows by amount — puts `Uncategorized` first. The view must still render
/// it as the last category row.
fn seed_summary_rows_uncategorized_largest(conn: &financi_db::rusqlite::Connection) {
    seed_summary_rows(conn);
    let (acct, import) = conn
        .query_row(
            "SELECT account_id, import_id FROM transactions LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                   row_hash, posted_date, amount_cents,
                                   description_raw, category_id)
         VALUES (?1, ?2, 7, X'00', 'summary-hash-largest', '2026-03-20', -5000,
                 'MYSTERY MEGA PURCHASE', NULL)",
        (acct, import),
    )
    .unwrap();
}

/// Switch the dashboard to "All time" and wait until the summary grid
/// covers exactly the two seeded months.
fn set_all_time(cx: &mut TestAppContext, handle: AnyWindowHandle, shell: &Entity<Shell>) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::AllTime, window, cx);
        });
    });
    wait_until(cx, "summary covers the two seeded months", |cx| {
        shell.read(cx).summary.months == ["2026-03", "2026-04"]
    });
}

fn click_summary_cell(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    id: Option<i64>,
    month: Option<&str>,
) {
    let month = month.map(str::to_string);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.click_summary_cell(id, month.clone(), window, cx)
        });
    });
}

fn summary_row_names(shell: &Entity<Shell>, cx: &App) -> Vec<String> {
    shell
        .read(cx)
        .summary_rows()
        .into_iter()
        .map(|r| r.name.to_string())
        .collect()
}

fn summary_parent_id(shell: &Entity<Shell>, cx: &App, name: &str) -> i64 {
    shell
        .read(cx)
        .summary_rows()
        .into_iter()
        .find(|r| r.name == name)
        .expect("summary row")
        .id
        .expect("a real category")
}

#[gpui::test]
fn summary_shows_months_total_and_the_three_total_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 7);
    cx.update(|cx| {
        let rows = shell.read(cx).summary_rows();
        let names: Vec<String> = rows.iter().map(|r| r.name.to_string()).collect();
        // Children are collapsed by default; the three total rows close
        // the grid.
        assert_eq!(
            names,
            [
                "Food",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
        let row = |name: &str| rows.iter().find(|r| r.name == name).unwrap();
        assert_eq!(row("Food").cells, [1500, 600, 2100]);
        assert_eq!(row("Uncategorized").cells, [200, 0, 200]);
        assert_eq!(row("Income").cells, [210500, 210500, 421000]);
        assert_eq!(row("Spend").cells, [1700, 900, 2600]);
        assert_eq!(row("Net").cells, [208800, 209600, 418400]);
        assert_eq!(
            row("Food").kind,
            SummaryRowKind::Parent {
                expandable: true,
                expanded: false
            }
        );
        assert_eq!(row("Uncategorized").kind, SummaryRowKind::Uncategorized);
        assert_eq!(row("Income").kind, SummaryRowKind::Income);
        assert_eq!(row("Spend").kind, SummaryRowKind::Spend);
        assert_eq!(row("Net").kind, SummaryRowKind::Net);
    });
}

#[gpui::test]
fn clicking_a_summary_cell_filters_the_table_to_category_and_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 7);
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));

    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "only March's Food rows", |cx| {
        shell.read(cx).row_count == 2
    });
    cx.update(|cx| {
        let rows = table_rows(&shell, cx);
        let mut descs: Vec<&str> = rows.iter().map(|r| r.description.as_str()).collect();
        descs.sort_unstable();
        assert_eq!(descs, ["CORNER CAFE MAR", "GROCER STORE MAR"]);
        assert_eq!(shell.read(cx).summary_month.as_deref(), Some("2026-03"));
        assert_eq!(
            shell.read(cx).legend_selection,
            Some(LegendSelection::Category(food))
        );
        // The parent is visible at the top level, so the chart stays put.
        assert!(shell.read(cx).chart_parent.is_none());
    });

    // Clicking the same cell again clears both halves of the filter.
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "all rows back", |cx| {
        shell.read(cx).row_count == 7
            && shell.read(cx).summary_month.is_none()
            && shell.read(cx).legend_selection.is_none()
    });
}

#[gpui::test]
fn clicking_a_child_summary_cell_drills_the_chart_to_its_parent(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 7);
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    click_summary_cell(cx, handle, &shell, Some(groceries), Some("2026-04"));
    wait_until(cx, "only April's Groceries row", |cx| {
        shell.read(cx).row_count == 1
    });
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    cx.update(|cx| {
        assert_eq!(table_rows(&shell, cx)[0].description, "GROCER STORE APR");
        assert_eq!(
            shell.read(cx).legend_selection,
            Some(LegendSelection::Category(groceries))
        );
        assert_eq!(shell.read(cx).summary_month.as_deref(), Some("2026-04"));
        // The selection must be one of the legend's rows, so the chart
        // drills to the child's parent and stays there across refreshes.
        assert_eq!(shell.read(cx).chart_parent, Some(food));
    });
}

#[gpui::test]
fn expanding_a_summary_parent_shows_its_child_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));

    cx.update(|cx| shell.update(cx, |s, _| s.toggle_summary_parent(food)));
    cx.update(|cx| {
        let rows = shell.read(cx).summary_rows();
        let names: Vec<String> = rows.iter().map(|r| r.name.to_string()).collect();
        assert_eq!(
            names,
            [
                "Food",
                "Groceries",
                "Restaurants",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
        assert_eq!(
            rows[0].kind,
            SummaryRowKind::Parent {
                expandable: true,
                expanded: true
            }
        );
        assert_eq!(rows[1].kind, SummaryRowKind::Child);
        assert_eq!(rows[1].cells, [1000, 600, 1600]);
        assert_eq!(rows[2].kind, SummaryRowKind::Child);
    });

    // Collapse again: the children disappear.
    cx.update(|cx| shell.update(cx, |s, _| s.toggle_summary_parent(food)));
    cx.update(|cx| {
        assert_eq!(
            summary_row_names(&shell, cx),
            [
                "Food",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
    });
}

/// The month is only half of a summary-cell selection: every handler that
/// clears `legend_selection` must clear `summary_month` too, or the table
/// stays silently month-filtered — no cell highlight, and the refresh
/// revalidation only drops a month missing from the grid.
#[gpui::test]
fn clearing_the_legend_selection_clears_the_summary_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 7);
    let food = cx.update(|cx| summary_parent_id(&shell, cx, "Food"));
    let groceries = cx.update(|cx| find_child(&shell, cx, "Groceries").unwrap());

    // Re-clicking the selected legend row: the pair clears together.
    click_summary_cell(cx, handle, &shell, Some(groceries), Some("2026-04"));
    wait_until(cx, "only April's Groceries row", |cx| {
        shell.read(cx).row_count == 1
    });
    click_chart_legend(cx, handle, &shell, Some(groceries));
    wait_until(cx, "the month goes with the selection", |cx| {
        shell.read(cx).row_count == 7
            && shell.read(cx).legend_selection.is_none()
            && shell.read(cx).summary_month.is_none()
    });

    // Re-clicking the `Uncategorized` legend row: the same.
    click_summary_cell(cx, handle, &shell, None, Some("2026-03"));
    wait_until(cx, "only March's uncategorized row", |cx| {
        shell.read(cx).row_count == 1
    });
    click_chart_legend(cx, handle, &shell, None);
    wait_until(cx, "the month goes with the uncategorized row", |cx| {
        shell.read(cx).row_count == 7
            && shell.read(cx).legend_selection.is_none()
            && shell.read(cx).summary_month.is_none()
    });

    // "← All categories".
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-03"));
    wait_until(cx, "only March's Food rows", |cx| {
        shell.read(cx).row_count == 2
    });
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.show_all_categories(window, cx));
    });
    wait_until(cx, "the month goes with all-categories", |cx| {
        shell.read(cx).row_count == 7
            && shell.read(cx).legend_selection.is_none()
            && shell.read(cx).summary_month.is_none()
    });

    // Turning on "Uncategorized only" over a category selection. Without
    // the month clearing this would show the empty intersection of
    // uncategorized April rows instead of the one uncategorized row.
    click_summary_cell(cx, handle, &shell, Some(food), Some("2026-04"));
    wait_until(cx, "only April's Food row", |cx| {
        shell.read(cx).row_count == 1
    });
    set_uncategorized_only(cx, handle, &shell, true);
    wait_until(cx, "uncategorized rows without the month", |cx| {
        shell.read(cx).legend_selection.is_none()
            && shell.read(cx).summary_month.is_none()
            && shell.read(cx).uncategorized_only
            && table_rows(&shell, cx)
                .iter()
                .map(|r| r.description.as_str())
                .eq(["Mystery Vendor"])
    });
    cx.update(|cx| {
        assert_eq!(shell.read(cx).row_count, 1);
    });
}

/// The grid comes from the query in amount order; the view still renders
/// `Uncategorized` as the last category row, before Income/Spend/Net,
/// whatever its amount (ui-conventions, *Numbers, dates and labels*).
#[gpui::test]
fn uncategorized_is_the_last_summary_category_row_whatever_its_amount(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_summary_rows_uncategorized_largest);
    set_all_time(cx, handle, &shell);
    wait_until(cx, "table loaded", |cx| shell.read(cx).row_count == 8);
    cx.update(|cx| {
        // The query really does put the biggest row — Uncategorized —
        // first, so the ordering below comes from the view's sort.
        assert_eq!(shell.read(cx).summary.rows[0].id, None);
        let rows = shell.read(cx).summary_rows();
        let names: Vec<String> = rows.iter().map(|r| r.name.to_string()).collect();
        assert_eq!(
            names,
            [
                "Food",
                "Transport",
                "Uncategorized",
                "Income",
                "Spend",
                "Net"
            ]
        );
        assert_eq!(rows[2].kind, SummaryRowKind::Uncategorized);
        assert_eq!(rows[2].cells, [5200, 0, 5200]);
    });
}

/// 3.30: one spend per month for `months` consecutive months ending
/// March 2026, plus eight spend parents (so the rows overflow the panel
/// and vertical scrolling has somewhere to go). The grid gets exactly
/// `months` columns.
fn seed_summary_months(conn: &financi_db::rusqlite::Connection, months: usize) {
    let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'seed.csv', '2026-01-01', X'00', 'utf-8', 0, 'lf', 0, 0)",
        [acct],
    )
    .unwrap();
    let import = conn.last_insert_rowid();
    let mut parents = Vec::new();
    for ix in 0..8 {
        parents.push(
            financi_db::categories::create_parent(
                conn,
                &format!("Cat {ix}"),
                CategoryKind::Expense,
            )
            .unwrap(),
        );
    }
    // The last `months` months, oldest first, ending 2026-03.
    let mut dates: Vec<String> = Vec::with_capacity(months);
    let (mut year, mut month) = (2026usize, 3usize);
    for _ in 0..months {
        dates.push(format!("{year:04}-{month:02}-10"));
        if month == 1 {
            year -= 1;
            month = 12;
        } else {
            month -= 1;
        }
    }
    dates.reverse();
    let mut line = 0i64;
    for (ix, date) in dates.iter().enumerate() {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("hscroll-hash-{line}"),
                date,
                -1000 - ix as i64,
                "MONTHLY SPEND",
                parents[0],
            ),
        )
        .unwrap();
        line += 1;
    }
    // One transaction per parent so every row appears in the grid.
    let last = dates.last().unwrap();
    for parent in &parents[1..] {
        conn.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes,
                                       row_hash, posted_date, amount_cents,
                                       description_raw, category_id)
             VALUES (?1, ?2, ?3, X'00', ?4, ?5, ?6, ?7, ?8)",
            (
                acct,
                import,
                line,
                format!("hscroll-hash-{line}"),
                last,
                -500 - line,
                "ONE-OFF SPEND",
                *parent,
            ),
        )
        .unwrap();
        line += 1;
    }
}

/// 3.30: like [`set_all_time`], but waits for the grid to cover exactly
/// `months` months (the seeds of these tests span more than two).
fn set_all_time_covering(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    shell: &Entity<Shell>,
    months: usize,
) {
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });
    wait_until(cx, "summary covers all the seeded months", |cx| {
        shell.read(cx).summary.months.len() == months
    });
}

/// 3.30: wheel `dx` pixels sideways over a point. On Linux Shift+wheel
/// arrives as a horizontal delta, which is what the month strip scrolls on.
fn wheel_over_x(cx: &mut TestAppContext, handle: AnyWindowHandle, at: Point<Pixels>, dx: f32) {
    let mut window = VisualTestContext::from_window(handle, cx);
    window.simulate_event(ScrollEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(dx), px(0.))),
        ..Default::default()
    });
}

/// 3.30: the painted bounds of a selector, panicking with its name when
/// the element did not paint at all.
fn bounds_of(window: &mut VisualTestContext, sel: &'static str) -> Bounds<Pixels> {
    window
        .debug_bounds(sel)
        .unwrap_or_else(|| panic!("{sel} paints on the dashboard"))
}

/// 3.30, measured at 1200 × 720: the pinned name column and `Total` paint
/// inside the panel, every visible month cell lies between them, and the
/// header and body months scroll by the same amount while the pinned
/// columns stay put.
#[gpui::test]
fn summary_months_scroll_under_pinned_name_and_total(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 24));
    resize_and_draw(cx, handle, 1200., 720.);
    set_all_time_covering(cx, handle, &shell, 24);
    wait_until(cx, "24 months in the grid", |cx| {
        shell.read(cx).summary.months.len() == 24
    });
    draw_frame(cx, handle);

    let mut window = VisualTestContext::from_window(handle, cx);
    let panel = bounds_of(&mut window, "panel:summary");
    let name = bounds_of(&mut window, "summary:name-0");
    let total = bounds_of(&mut window, "summary:cell-0-24");
    let zone = bounds_of(&mut window, "summary:months-0");
    // Both pinned columns paint inside the panel…
    assert!(
        panel.origin.x <= name.origin.x && name.right() <= panel.right(),
        "the name column paints inside the panel"
    );
    assert!(
        panel.origin.x <= total.origin.x && total.right() <= panel.right(),
        "the Total column paints inside the panel"
    );
    // …and the scrolling month zone sits between them.
    assert!(
        zone.origin.x >= name.right() - px(1.) && zone.right() <= total.origin.x + px(1.),
        "the month zone lies between the pinned columns"
    );
    // The month cells paint between the two pinned columns: the zone shows
    // whole columns, and a column scrolled past an edge is clipped there
    // (the viewport hides the overflow rather than painting over the names
    // or the Total), so no cell's visible part can leave the zone.
    let mut whole = 0;
    for col in 0..24 {
        let cell = bounds_of(&mut window, leaked(format!("summary:cell-0-{col}")));
        if cell.origin.x >= zone.origin.x - px(1.) && cell.right() <= zone.right() + px(1.) {
            whole += 1;
        } else {
            // A partially visible column hangs over exactly one edge only.
            let hangs_left = cell.origin.x < zone.origin.x - px(1.);
            let hangs_right = cell.right() > zone.right() + px(1.);
            assert!(
                !(hangs_left && hangs_right),
                "month column {col} is wider than the month zone"
            );
        }
    }
    assert!(whole >= 1, "at least one whole month cell is visible");
    // A range change (the All-time switch above) lands on the most recent
    // month: the newest column is on screen beside `Total`, the oldest is
    // scrolled away to the left.
    let off0 = cx.update(|cx| shell.read(cx).summary_scroll.offset().x);
    assert!(off0 < px(0.), "the 24-month strip is scrolled sideways");
    let newest = bounds_of(&mut window, "summary:cell-0-23");
    assert!(
        newest.right() > zone.origin.x && newest.origin.x < zone.right(),
        "the latest month is on screen"
    );
    let oldest = bounds_of(&mut window, "summary:cell-0-0");
    assert!(
        oldest.right() <= zone.origin.x + px(1.),
        "the oldest month is scrolled out to the left"
    );

    // Shift+wheel over the month zone scrolls it, and the header moves by
    // exactly the same amount as the body cells.
    let before_label = bounds_of(&mut window, "summary:month-label-12").origin.x;
    let before_cell = bounds_of(&mut window, "summary:cell-0-12").origin.x;
    let before_name = bounds_of(&mut window, "summary:name-0").origin.x;
    let before_total = bounds_of(&mut window, "summary:cell-0-24").origin.x;
    wheel_over_x(cx, handle, viewport_centre(zone), 240.);
    draw_frame(cx, handle);
    let off1 = cx.update(|cx| shell.read(cx).summary_scroll.offset().x);
    assert!(off1 > off0, "the strip scrolled back towards its left end");
    let moved = f32::from(off1 - off0);
    assert!(moved > 0.);
    let after_label = f32::from(bounds_of(&mut window, "summary:month-label-12").origin.x);
    let after_cell = f32::from(bounds_of(&mut window, "summary:cell-0-12").origin.x);
    assert!(
        (after_label - f32::from(before_label) - moved).abs() < 1.,
        "the header months moved with the handle"
    );
    assert!(
        (after_cell - f32::from(before_cell) - moved).abs() < 1.,
        "the body months moved by the same amount as the header"
    );
    // The pinned columns did not move.
    assert_eq!(
        bounds_of(&mut window, "summary:name-0").origin.x,
        before_name,
        "the name column stays pinned"
    );
    assert_eq!(
        bounds_of(&mut window, "summary:cell-0-24").origin.x,
        before_total,
        "the Total column stays pinned"
    );

    // Vertical scrolling of the body still works over the month zone,
    // and the name column moves with the rows (it is part of the body).
    let y_before = bounds_of(&mut window, "summary:name-0").origin.y;
    wheel_over(cx, handle, viewport_centre(zone), -40.);
    draw_frame(cx, handle);
    let y_after = bounds_of(&mut window, "summary:name-0").origin.y;
    assert!(
        y_after < y_before,
        "the rows scroll vertically under the month zone, names included"
    );
}

/// 3.30: a horizontal swipe over the months scrolls the month strip only.
/// A y-only scroller remaps a horizontal wheel delta onto its own axis,
/// so `summary-rows` must restrict itself to the vertical axis — otherwise
/// the same swipe that moves the months also jumps the rows.
#[gpui::test]
fn a_horizontal_swipe_does_not_jump_the_summary_rows_vertically(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 24));
    resize_and_draw(cx, handle, 1200., 720.);
    set_all_time_covering(cx, handle, &shell, 24);
    draw_frame(cx, handle);
    let mut window = VisualTestContext::from_window(handle, cx);
    let zone = bounds_of(&mut window, "summary:months-0");

    // Wheel the body down first, so there is somewhere to jump back to.
    let y_top = bounds_of(&mut window, "summary:name-0").origin.y;
    wheel_over(cx, handle, viewport_centre(zone), -40.);
    draw_frame(cx, handle);
    let y_scrolled = bounds_of(&mut window, "summary:name-0").origin.y;
    assert!(y_scrolled < y_top, "the body is scrolled down");

    // A horizontal swipe over the month zone moves the months…
    let x0 = cx.update(|cx| shell.read(cx).summary_scroll.offset().x);
    wheel_over_x(cx, handle, viewport_centre(zone), 200.);
    draw_frame(cx, handle);
    let x1 = cx.update(|cx| shell.read(cx).summary_scroll.offset().x);
    assert_ne!(x1, x0, "the month strip scrolled sideways");

    // …and leaves the body exactly where it was.
    let y_after = bounds_of(&mut window, "summary:name-0").origin.y;
    assert_eq!(
        y_after, y_scrolled,
        "the horizontal swipe left the rows' vertical position alone"
    );
}

/// 3.30: changing the range re-lands the month strip on its most recent
/// month — even after the user had scrolled away.
#[gpui::test]
fn changing_the_summary_range_lands_on_the_latest_month(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 24));
    resize_and_draw(cx, handle, 1200., 720.);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.change_date_preset(DatePreset::Last12Months, window, cx)
        })
    });
    wait_until(cx, "12 months in the grid", |cx| {
        shell.read(cx).summary.months.len() == 12
    });
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let s = shell.read(cx);
        (s.summary_scroll.offset().x, s.summary_scroll.max_offset().x)
    });
    assert!(max > px(0.), "twelve months overflow the month zone");
    assert!(
        (offset + max).abs() < px(1.),
        "the strip starts at its right end (offset {offset}, max {max})"
    );

    // Scroll away from the right end…
    let mut window = VisualTestContext::from_window(handle, cx);
    let zone = bounds_of(&mut window, "summary:months-0");
    wheel_over_x(cx, handle, viewport_centre(zone), 200.);
    draw_frame(cx, handle);
    let scrolled = cx.update(|cx| shell.read(cx).summary_scroll.offset().x);
    assert!(scrolled < max, "the strip is no longer at its right end");

    // …and change the range again: the latest month is beside `Total`.
    set_all_time_covering(cx, handle, &shell, 24);
    wait_until(cx, "24 months in the grid", |cx| {
        shell.read(cx).summary.months.len() == 24
    });
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let s = shell.read(cx);
        (s.summary_scroll.offset().x, s.summary_scroll.max_offset().x)
    });
    assert!(
        (offset + max).abs() < px(1.),
        "the new range lands scrolled to the last month"
    );
    let zone = bounds_of(&mut window, "summary:months-0");
    let newest = bounds_of(&mut window, "summary:cell-0-23");
    assert!(
        newest.right() > zone.origin.x && newest.origin.x < zone.right(),
        "the latest month is on screen"
    );
}

/// 3.30: when every month fits there is nothing to scroll and the
/// horizontal scrollbar does not show.
///
/// This runs at 1600 × 900 rather than 1200 × 720: with the name pinned at
/// 160 px and Total at 96 px, the 1200-wide panel leaves only a ~200 px
/// month zone, so even three 88 px columns overflow it by construction.
/// "Everything fits" needs a window wide enough for three months plus the
/// two pinned columns.
#[gpui::test]
fn a_short_summary_range_shows_no_horizontal_scrollbar(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |conn| seed_summary_months(conn, 3));
    resize_and_draw(cx, handle, 1600., 900.);
    set_all_time_covering(cx, handle, &shell, 3);
    draw_frame(cx, handle);
    let (offset, max) = cx.update(|cx| {
        let s = shell.read(cx);
        (s.summary_scroll.offset().x, s.summary_scroll.max_offset().x)
    });
    assert_eq!(
        max,
        px(0.),
        "three months must fit the month zone without scrolling"
    );
    assert_eq!(offset, px(0.), "and nothing is scrolled");
    // The strip still reads as today: the newest month sits flush against
    // `Total`.
    let mut window = VisualTestContext::from_window(handle, cx);
    let total = bounds_of(&mut window, "summary:cell-0-3");
    let newest = bounds_of(&mut window, "summary:cell-0-2");
    assert!(
        (total.origin.x - newest.right()).abs() < px(9.),
        "the months sit right-aligned beside the Total column"
    );
}
