//! 3.57: every page reloads only what it depends on. One test per write
//! row of the table on [`crate::ui::catalog::Domain`] — driven through the
//! query-count hook, so each asserts exactly the queries the write may
//! run and none it may not — plus the two scheduling halves: a hidden
//! dirty page reloads once when next shown, and the page switch itself
//! never queries (the switch test lives in `catalog.rs`). The rule-edit
//! row is already covered there (`a_rule_edit_reloads_only_the_sections_it_touches`).

use super::dashboard::dash;
use super::transfers::seed_transfer_tie;
use super::*;
use crate::ui::page::PageEntity;

/// The names the query-count hook recorded since the last `clear`.
fn logged(cx: &App, shell: &Entity<Shell>) -> Vec<String> {
    shell.read(cx).db.query_log().names()
}

/// The shared catalog sections every Transactions write reloads: the
/// category tree and its counts, the rules and their use counts, the
/// sidebar totals and the queue (a row moving can change all of them).
const SHARED_TX: [&str; 6] = [
    "list_categories",
    "transaction_counts",
    "list_rules",
    "rule_use_stats",
    "sidebar_counts",
    "uncategorized_by_merchant",
];

fn assert_logged(names: &[String], expected: &[&str], forbidden: &[&str]) {
    for e in expected {
        assert!(
            names.contains(&e.to_string()),
            "the write must reload {e}; ran {names:?}"
        );
    }
    for f in forbidden {
        assert!(
            !names.contains(&f.to_string()),
            "the write must not reload {f}; ran {names:?}"
        );
    }
}

/// The *set category* row: the dashboard stays visible, so its rows,
/// chart and summary reload alongside the shared sections Transactions
/// touches — but the Imports history and the open transfer reviews never
/// read a categorization.
#[gpui::test]
fn setting_a_row_category_reloads_only_transactions(cx: &mut TestAppContext) {
    let (handle, shell, _rule, _first, second) = open_rule_fixture(cx);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_idle(cx, &shell);
    assert_eq!(cx.update(|cx| dash(cx, &shell).read(cx).row_count), 3);
    shell.update(cx, |s, _| s.db.query_log().clear());
    let txn = cx.update(|cx| table_rows(&shell, cx)[0].id);
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.dashboard
                .update(cx, |d, cx| d.change_row_category(txn, second, window, cx))
        });
    });
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    let mut expected: Vec<&str> = SHARED_TX.to_vec();
    expected.extend(["list_transactions_filtered", "dashboard"]);
    assert_logged(
        &names,
        &expected,
        &["list_accounts", "list_imports", "list_transfer_reviews"],
    );
}

/// The *category add / rename / move / delete* row: a new parent touches
/// Categories, Rules and Transactions — the same shared sections plus the
/// visible dashboard's own load — and nothing else.
#[gpui::test]
fn a_category_write_reloads_categories_rules_and_transactions(cx: &mut TestAppContext) {
    let (handle, shell, _rule, _first, _second) = open_rule_fixture(cx);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_idle(cx, &shell);
    shell.update(cx, |s, _| s.db.query_log().clear());
    add_parent(cx, handle, &shell, "Groceries");
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    let mut expected: Vec<&str> = SHARED_TX.to_vec();
    expected.extend(["list_transactions_filtered", "dashboard"]);
    assert_logged(
        &names,
        &expected,
        &["list_accounts", "list_imports", "list_transfer_reviews"],
    );
}

/// The *pair / unpair / transfer review* row: on the Transfers page a
/// pairing reloads the reviews and the shared Transactions sections, but
/// never the accounts, the Imports history — and not the dashboard, which
/// is hidden behind the page and defers its one load to the visit.
#[gpui::test]
fn pairing_a_transfer_reloads_reviews_and_transactions_only(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_transfer_tie(c));
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Transfers, cx)));
    wait_until(cx, "the tie review", |cx| {
        shell.read(cx).transfers.read(cx).transfer_reviews.len() == 1
    });
    wait_idle(cx, &shell);
    shell.update(cx, |s, _| s.db.query_log().clear());
    let (debit, credit_a, _credit_b) = ids;
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.transfers
                .update(cx, |p, cx| p.pair_transfer(debit, credit_a, window, cx))
        });
    });
    wait_until(cx, "the review closes", |cx| {
        shell
            .read(cx)
            .transfers
            .read(cx)
            .transfer_reviews
            .is_empty()
    });
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    let mut expected: Vec<&str> = vec!["list_transfer_reviews"];
    expected.extend(SHARED_TX);
    assert_logged(
        &names,
        &expected,
        &[
            "list_accounts",
            "list_imports",
            // The hidden dashboard defers its load to the visit.
            "dashboard",
            "list_transactions_filtered",
        ],
    );
    cx.update(|cx| assert!(dash(cx, &shell).read(cx).dirty()));
}

/// The *new account (import panel)* row: creating the account writes only
/// an account, so only the account list and the sidebar counts reload —
/// the categories, rules, queue, reviews and history all stay put. The
/// panel open already switched to the Imports page (ui-conventions,
/// *Pages*), so the dashboard watches hidden and defers its own load to
/// the visit, dirty with the news.
#[gpui::test]
fn a_new_account_reloads_accounts_and_the_sidebar_only(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    open_and_choose(cx, handle, &shell, "amex-sep-oct.csv", NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        let p = panel(&shell, cx);
        p.update(cx, |p, cx| {
            p.new_name.update(cx, |input, cx| {
                input.set_value("Amex Gold", window, cx);
            });
        });
    });
    wait_idle(cx, &shell);
    shell.update(cx, |s, _| s.db.query_log().clear());
    with_window(cx, handle, |window, cx| {
        let p = panel(&shell, cx);
        p.update(cx, |p, cx| p.create_account(window, cx));
    });
    wait_until(cx, "plan ready after naming", |cx| plan_ready(&shell, cx));
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    assert_logged(
        &names,
        &["list_accounts", "sidebar_counts"],
        &[
            "list_categories",
            "transaction_counts",
            "list_rules",
            "rule_use_stats",
            "uncategorized_by_merchant",
            "list_imports",
            "list_transfer_reviews",
            // The hidden dashboard defers its load to the visit.
            "dashboard",
            "list_transactions_filtered",
        ],
    );
    cx.update(|cx| assert!(dash(cx, &shell).read(cx).dirty()));
}

/// The *import, undo import, restore* row: undoing the import is a full
/// reload — every catalog section runs. The dashboard is hidden behind
/// the Imports page, so its one load is the only thing deferred, and it
/// lands as a single query pair when the page is next shown.
#[gpui::test]
fn undoing_an_import_reloads_everything(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    import_demo_file(cx, handle, &shell, "amex-sep-oct.csv", "Amex Gold");
    wait_idle(cx, &shell);
    wait_until(cx, "the import in the history", |cx| {
        !shell.read(cx).imports.read(cx).import_history.is_empty()
    });
    // The import lands on the Imports page; the dashboard watches hidden.
    cx.update(|cx| assert!(!dash(cx, &shell).read(cx).visible()));
    let id = cx.update(|cx| shell.read(cx).imports.read(cx).import_history[0].id);
    shell.update(cx, |s, _| s.db.query_log().clear());
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.imports.update(cx, |p, cx| p.undo_import(id, window, cx))
        });
    });
    // Arm, then the second click really undoes.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.imports.update(cx, |p, cx| p.undo_import(id, window, cx))
        });
    });
    wait_until(cx, "the history empties", |cx| {
        shell.read(cx).imports.read(cx).import_history.is_empty()
    });
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    assert_logged(
        &names,
        &[
            "list_accounts",
            "list_categories",
            "transaction_counts",
            "list_rules",
            "rule_use_stats",
            "sidebar_counts",
            "uncategorized_by_merchant",
            "list_transfer_reviews",
            "list_imports",
        ],
        &["dashboard", "list_transactions_filtered"],
    );
    // Shown again, the dirty hidden dashboard reloads — once.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    assert_eq!(
        names.iter().filter(|n| n.as_str() == "dashboard").count(),
        1,
        "the visit must run the dashboard's one load exactly once; ran {names:?}"
    );
    assert_eq!(
        names
            .iter()
            .filter(|n| n.as_str() == "list_transactions_filtered")
            .count(),
        1,
        "the visit must reload the rows exactly once; ran {names:?}"
    );
}

/// The view settings are the write that never travels through a report
/// (3.56): a search change reloads only the dashboard's rows, chart and
/// summary — not one shared section.
#[gpui::test]
fn a_view_setting_reloads_the_dashboard_rows_only(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |c| {
        seed_rule_fixture(c);
    });
    wait_idle(cx, &shell);
    wait_until(cx, "the fixture rows", |cx| {
        dash(cx, &shell).read(cx).row_count == 3
    });
    shell.update(cx, |s, _| s.db.query_log().clear());
    with_window(cx, handle, |window, cx| {
        dash(cx, &shell).update(cx, |d, cx| d.apply_search("LOCKED", window, cx));
    });
    wait_until(cx, "one row left", |cx| {
        dash(cx, &shell).read(cx).row_count == 1
    });
    let names = cx.update(|cx| logged(cx, &shell));
    assert_logged(
        &names,
        &["list_transactions_filtered", "dashboard"],
        &[
            "list_accounts",
            "list_categories",
            "transaction_counts",
            "list_rules",
            "rule_use_stats",
            "sidebar_counts",
            "uncategorized_by_merchant",
            "list_imports",
            "list_transfer_reviews",
        ],
    );
}

/// A write that lands while the dashboard is hidden makes it dirty and
/// queries nothing of its; showing the page runs its one load exactly
/// once — no second reload rides along.
#[gpui::test]
fn a_dirty_hidden_dashboard_reloads_once_when_shown(cx: &mut TestAppContext) {
    let (handle, shell, _rule, _first, _second) = open_rule_fixture(cx);
    // The fixture ends on Categories; the dashboard watches from hiding.
    cx.update(|cx| assert!(!dash(cx, &shell).read(cx).visible()));
    shell.update(cx, |s, _| s.db.query_log().clear());
    add_parent(cx, handle, &shell, "Groceries");
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    assert!(
        !names.contains(&"dashboard".to_string())
            && !names.contains(&"list_transactions_filtered".to_string()),
        "a hidden dashboard must not query; ran {names:?}"
    );
    cx.update(|cx| assert!(dash(cx, &shell).read(cx).dirty()));
    shell.update(cx, |s, _| s.db.query_log().clear());
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    assert_eq!(
        names.iter().filter(|n| n.as_str() == "dashboard").count(),
        1,
        "the visit must reload the dashboard exactly once; ran {names:?}"
    );
    assert_eq!(
        names
            .iter()
            .filter(|n| n.as_str() == "list_transactions_filtered")
            .count(),
        1,
        "the visit must reload the rows exactly once; ran {names:?}"
    );
    cx.update(|cx| assert!(!dash(cx, &shell).read(cx).dirty()));
}

/// The same deferral on a snapshot page: the To-categorize list applies
/// the catalog instead of querying it, so a pairing that lands while the
/// page is hidden leaves it dirty with stale rows, and showing it applies
/// the current snapshot — without running one query.
#[gpui::test]
fn a_dirty_hidden_page_applies_the_snapshot_when_shown(cx: &mut TestAppContext) {
    let mut ids = (0i64, 0i64, 0i64);
    let (handle, shell) = open_shell(cx, |c| ids = seed_transfer_tie(c));
    wait_idle(cx, &shell);
    // Visit first: the page applies, so it leaves this visit clean.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_until(cx, "the queue populates", |cx| {
        !shell.read(cx).queue_view.read(cx).groups.is_empty()
    });
    cx.update(|cx| assert!(!shell.read(cx).queue_view.read(cx).dirty()));
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Transfers, cx)));
    wait_until(cx, "the tie review", |cx| {
        shell.read(cx).transfers.read(cx).transfer_reviews.len() == 1
    });
    wait_idle(cx, &shell);
    let (debit, credit_a, _credit_b) = ids;
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| {
            s.transfers
                .update(cx, |p, cx| p.pair_transfer(debit, credit_a, window, cx))
        });
    });
    wait_until(cx, "the review closes", |cx| {
        shell.read(cx).catalog.read(cx).snap.reviews.is_empty()
    });
    wait_idle(cx, &shell);
    // Hidden and dirty: the page did not apply while it was away.
    cx.update(|cx| {
        let s = shell.read(cx);
        let q = s.queue_view.read(cx);
        assert!(q.dirty());
    });
    shell.update(cx, |s, _| s.db.query_log().clear());
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::ToCategorize, cx)));
    wait_idle(cx, &shell);
    let names = cx.update(|cx| logged(cx, &shell));
    assert!(
        names.is_empty(),
        "applying the snapshot must issue no query; ran {names:?}"
    );
    cx.update(|cx| {
        let s = shell.read(cx);
        let q = s.queue_view.read(cx);
        assert!(!q.dirty(), "the visit must consume the dirty flag");
        assert_eq!(
            q.groups,
            s.catalog.read(cx).snap.queue,
            "the visit must show the current snapshot"
        );
    });
}
