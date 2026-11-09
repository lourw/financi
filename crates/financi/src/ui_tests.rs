//! Headless UI tests: real views in a test window and a real DbWorker over an
//! in-memory database, driven through the same view methods the buttons
//! call, using the synthetic demo exports.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use financi_core::dates::{DatePreset, preset_range};
use financi_core::parse::merchant;
use financi_core::{AccountKind, CategoryKind};
use financi_db::import::TransactionRow;
use financi_db::settings::{self, DateRange, UI_DATE_RANGE, UI_INCLUDE_TRANSFERS};
use financi_db::{DbWorker, ViewFilter, create_account, open_in_memory};
// No glob import: `gpui_kit::*` brings GPUI's `test` macro into scope, which
// shadows the built-in #[test] that #[gpui::test] expands to.
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Entity, Point, TestAppContext, Window, WindowBounds,
    WindowOptions, px, size,
};

use super::{ImportPanel, NEW_ACCOUNT, PlanState, RowMenuItem, Shell, row_menu_items};

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
        assert_eq!(p.read(cx).format, "Simplii Financial chequing");
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
            s.status.clone(),
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
            shell.read(cx).status.clone(),
            Some((
                "Undid import of amex-sep-oct.csv: removed 68 rows".into(),
                false
            ))
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
        let (msg, is_error) = s.status.clone().unwrap();
        assert!(is_error && msg.contains("not a recognized export"), "{msg}");
    });
    std::fs::remove_dir_all(dir).unwrap();
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
            let picker = s.queue_pickers.get(&merchant).expect("picker built");
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

// -- row context menu (2.9) -------------------------------------------------

/// A minimal row for the pure menu-model test.
fn browse_row(hidden: bool) -> TransactionRow {
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
    }
}

#[test]
fn menu_shows_hide_for_a_visible_row_and_unhide_only_for_a_hidden_one() {
    assert_eq!(
        row_menu_items(&browse_row(false)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Hide],
        "a visible row offers Hide, never Unhide"
    );
    assert_eq!(
        row_menu_items(&browse_row(true)),
        [RowMenuItem::ChangeCategory, RowMenuItem::Unhide],
        "a hidden row offers Unhide, never Hide"
    );
}

/// The dashboard table's rows.
fn table_rows(shell: &Entity<Shell>, cx: &App) -> Vec<TransactionRow> {
    shell.read(cx).table.read(cx).delegate().rows.clone()
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
