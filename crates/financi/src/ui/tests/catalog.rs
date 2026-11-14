//! 3.78: the shared data model. A write reloads exactly the sections its
//! domains touch — asserted through the query-count hook (`query_log`),
//! which every catalog reload names as it runs its queries — and a page
//! switch reloads nothing at all.

use super::*;

/// The names recorded since the last `clear`.
fn logged(cx: &App, shell: &Entity<Shell>) -> Vec<String> {
    shell.read(cx).db.query_log().names()
}

/// A real rule edit — Edit on the fixture rule, a new pattern, Save —
/// reloads the rules and their use counts, the dashboard's rows, chart
/// and summary, the category counts and the queue (a re-run of the rules
/// can move unlocked rows), but never the Imports history or the open
/// transfer reviews: no rule edit touches either.
#[gpui::test]
fn a_rule_edit_reloads_only_the_sections_it_touches(cx: &mut TestAppContext) {
    let (handle, shell, rule, _first, _second) = open_rule_fixture(cx);
    start_rule_edit(cx, handle, &shell, rule);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.rule_edit_pattern.update(cx, |i, cx| {
                i.set_value("COFFEE EDITED", window, cx);
            });
        });
    });
    // Forget the launch load and the editor's opening preview; count
    // only what the save itself reloads.
    shell.update(cx, |s, _| s.db.query_log().clear());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.save_rule_edit(window, cx));
    });
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    for expected in [
        "list_rules",
        "rule_use_stats",
        "list_transactions_filtered",
        "spend_by_category",
        "monthly_summary",
        "sidebar_counts",
        "uncategorized_by_merchant",
        "list_categories",
        "transaction_counts",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "a rule edit must reload {expected}; ran {names:?}"
        );
    }
    for forbidden in ["list_imports", "list_transfer_reviews", "list_accounts"] {
        assert!(
            !names.contains(&forbidden.to_string()),
            "a rule edit must not reload {forbidden}; ran {names:?}"
        );
    }
}

/// The full reload — what launch and the import / undo / restore flows
/// run — still loads every section: all twelve queries, including the
/// history and the reviews the partial reloads leave alone.
#[gpui::test]
fn a_full_reload_loads_every_section(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        seed_rule_fixture(c);
    });
    wait_idle(cx, &shell);
    shell.update(cx, |s, _| s.db.query_log().clear());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.refresh(window, cx));
    });
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    for expected in [
        "list_accounts",
        "list_categories",
        "transaction_counts",
        "list_rules",
        "rule_use_stats",
        "sidebar_counts",
        "uncategorized_by_merchant",
        "list_transfer_reviews",
        "list_imports",
        "list_transactions_filtered",
        "spend_by_category",
        "monthly_summary",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "the full reload must run {expected}; ran {names:?}"
        );
    }
}

/// 3.78 review fix, reproduced: the full reload runs on the writer queue
/// and the keyed partial reloads on the reader queue, with no ordering
/// between the two (worker.rs). A full reload dispatched first can land
/// *after* a newer keyed reload — and must then keep its rows, queried
/// with filters the user has since changed, off the screen. The in-memory
/// test databases (one FIFO queue) can never invert, so this drives the
/// inversion over a file DB — real reader thread — with the writer held
/// by `block_worker` to delay the full reload's landing.
#[gpui::test]
fn a_full_reload_superseded_mid_flight_keeps_the_newer_rows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell_on_file(cx, |conn| {
        let acct = create_account(conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
        conn.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'seed.csv', '2026-11-08', X'00', 'utf-8', 0, 'lf', 0, 3)",
            [acct],
        )
        .unwrap();
        let import = conn.last_insert_rowid();
        for line in 0..3 {
            seed_txn(
                conn,
                import,
                acct,
                line,
                "2026-11-05",
                -1_000,
                "COFFEE STAND",
            );
        }
    });
    wait_idle(cx, &shell);
    cx.update(|cx| assert_eq!(shell.read(cx).dashboard.row_count, 3));
    // Hold the writer thread, then start the full reload: it waits behind
    // the blocker, its filters snapshotted at dispatch (no search yet).
    let blocker = block_worker(cx, &shell);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.refresh(window, cx));
    });
    // The user searches while the writer is held: the keyed reload runs on
    // the idle reader and the table shows the user's (empty) result.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.apply_search("no such merchant", window, cx));
    });
    wait_until(cx, "the search's reload lands", |cx| {
        shell.read(cx).dashboard.row_count == 0
    });
    // Release the writer: the full reload runs now and lands late with the
    // three rows its stale filter returned.
    drop(blocker);
    wait_idle(cx, &shell);
    cx.update(|cx| {
        assert_eq!(
            shell.read(cx).dashboard.row_count,
            0,
            "a superseded full reload resurrected rows the user's search removed"
        );
    });
}

/// Switching pages reads only what is already loaded: not one query
/// leaves the shell (3.78's page-switch invariant).
#[gpui::test]
fn switching_pages_issues_no_queries(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |c| {
        seed_rule_fixture(c);
    });
    wait_idle(cx, &shell);
    shell.update(cx, |s, _| s.db.query_log().clear());
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.go_page(Page::Categories, cx);
            s.go_page(Page::Imports, cx);
            s.go_page(Page::Dashboard, cx);
        });
    });
    cx.run_until_parked();
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    assert!(names.is_empty(), "a page switch issued queries: {names:?}");
}
