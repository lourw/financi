//! Tests for `ui/imports.rs` (3.62 split from `ui_tests.rs`).
//!
//! H2 split this file by feature: each sub-module below holds one feature's
//! tests, and the seed and dialog helpers used by two or more of them (plus
//! the `pub(super)` seed `tests/page.rs` imports) live here. New tests go
//! next to related ones in the right sub-file.

use super::*;

mod account;
mod conflicts;
mod duplicates;
mod end_to_end;
mod export;
mod file_menu;
mod layout;
mod panel_lifecycle;
mod restore;
mod undo;

/// Seed three imports, oldest first, so the history table has several rows
/// and the newest one — the only row with the in-row Undo button — stands
/// out (3.32). The rows differ only in name and time; `imports.id` ascends,
/// and the history lists `id DESC`.
pub(super) fn seed_three_imports(conn: &financi_db::rusqlite::Connection) {
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
        !shell.read(cx).imports.read(cx).exporting
            && last_toast(shell, cx) == Some((format!("Exported to {label}"), false))
    });
}
