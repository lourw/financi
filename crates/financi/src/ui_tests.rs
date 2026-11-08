//! Headless UI tests: real views in a test window and a real DbWorker over an
//! in-memory database, driven through the same view methods the buttons
//! call, using the synthetic demo exports.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use financi_core::AccountKind;
use financi_db::{DbWorker, create_account, open_in_memory};
// No glob import: `gpui_kit::*` brings GPUI's `test` macro into scope, which
// shadows the built-in #[test] that #[gpui::test] expands to.
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Entity, Point, TestAppContext, Window, WindowBounds,
    WindowOptions, px, size,
};

use super::{ImportPanel, NEW_ACCOUNT, PlanState, Shell};

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
    wait_until(cx, "Cobalt import", |cx| shell.read(cx).row_count == 68);

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
        shell.read(cx).import.is_none() && shell.read(cx).row_count == 68 + 102
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
    wait_until(cx, "import", |cx| shell.read(cx).row_count == 68);

    with_window(cx, handle, |window, cx| {
        let id = shell.read(cx).accounts[0].id;
        shell.update(cx, |s, cx| {
            s.selected_account = Some(id);
            s.undo(window, cx);
        });
        assert!(
            shell.read(cx).confirm_undo,
            "first click only asks to confirm"
        );
        assert_eq!(shell.read(cx).row_count, 68);
        shell.update(cx, |s, cx| s.undo(window, cx));
    });

    wait_until(cx, "undo", |cx| shell.read(cx).row_count == 0);
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
