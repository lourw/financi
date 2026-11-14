//! Tests for `ui/widgets.rs` (3.62 split from `ui_tests.rs`).

use super::*;
/// 3.27: success and info toasts dismiss themselves after ~4 s. The manager
/// runs on a 100 ms ticker, so the whole life of a toast is entrance (400 ms)
/// + timeout (4 s) + exit (200 ms) of virtual time.
#[gpui::test]
fn info_toasts_dismiss_themselves_after_a_few_seconds(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    // 5.3 took the Export… placeholder toast away; a finished import
    // pushes the same plain info toast.
    import_demo_file(cx, handle, &shell, "simplii-chequing.csv", "Simplii");
    wait_until(cx, "the toast is up", |cx| !toasts(&shell, cx).is_empty());
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
    let card = bounds(cx, handle, format!("toast:{msg}"));
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

// 3.92: every UI job reaches the worker through `on_db`. A panicking job
// (an undo or a category edit that blows up) must report the *job panic*
// — the toast text the user sees — not the old worker-gone message a
// dropped result sender produced, and the worker must keep serving the
// next job. Worker-layer twin: `panicking_typed_job_reports_job_panicked`.

#[test]
fn a_panicking_on_db_job_reports_the_job_panic() {
    use crate::ui::widgets::on_db;
    let db = DbWorker::spawn(open_in_memory().unwrap());
    let err = block_on(on_db(&db, |c| -> Result<i64, std::io::Error> {
        c.execute_batch("BEGIN; INSERT INTO settings (key, value) VALUES ('x', '1');")
            .unwrap();
        panic!("boom")
    }))
    .expect_err("a panicking job fails");
    assert!(err.contains("panicked"), "{err}");
    assert!(!err.contains("gone"), "{err}");

    // The worker survives the panic and still answers reads.
    let rows = block_on(on_db(&db, |c| {
        c.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
    }))
    .expect("the worker still serves jobs after a panic");
    assert_eq!(rows, 0, "the panicking job's transaction was rolled back");
}

/// 3.92: only panics are re-classified — a job that fails normally keeps
/// passing its own message through, unchanged from the untyped path.
#[test]
fn on_db_passes_ordinary_job_errors_through_unchanged() {
    use crate::ui::widgets::on_db;
    let db = DbWorker::spawn(open_in_memory().unwrap());
    let err = block_on(on_db(&db, |_c| -> Result<i64, std::io::Error> {
        Err(std::io::Error::other("nope"))
    }))
    .expect_err("the job fails");
    assert_eq!(err, "nope");

    let value = block_on(on_db(&db, |c| {
        c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
    }))
    .expect("a job that succeeds still returns its value");
    assert_eq!(value, 1);
}
