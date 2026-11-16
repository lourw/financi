//! End to end (3.56): a committed import reloads every page, and every
//! page shows the expected values.

use super::*;

/// 3.56: an import commit still reloads *everything* — the nine catalog
/// queries plus the dashboard's own load alongside them. The keyed
/// dashboard job is an addition to the full reload, not a replacement:
/// the history, tree, rules, queue and reviews all follow an import.
#[gpui::test]
fn an_import_commit_still_reloads_every_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    open_and_choose(cx, handle, &shell, "simplii-chequing.csv", NEW_ACCOUNT);
    with_window(cx, handle, |window, cx| {
        let p = panel(&shell, cx);
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
    cx.update(|cx| shell.read(cx).db.query_log().clear());
    click_import(cx, handle, &shell);
    // 3.82: the import switched to the Imports page; go back, where the
    // hidden dashboard reloads.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "import finished and table refreshed", |cx| {
        shell.read(cx).imports.read(cx).import.is_none()
            && dash(cx, &shell).read(cx).row_count == 11
    });
    // The dashboard job is the fastest of the reload and the row check
    // above can pass before the commit's full reload is even dispatched
    // (3.82: the dashboard reloads on its own visit). Wait for the last
    // of the catalog queries, then for everything to settle, before
    // reading the log.
    wait_until(cx, "full reload dispatched", |cx| {
        shell
            .read(cx)
            .db
            .query_log()
            .names()
            .contains(&"list_imports".to_string())
    });
    wait_idle(cx, &shell);
    let names = cx.update(|cx| shell.read(cx).db.query_log().names());
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
        "dashboard",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "an import must reload {expected}; ran {names:?}"
        );
    }
}

/// 3.82: the hidden dashboard picks up a commit made on the Imports page
/// through the full reload's `DataChanged` alone — one visit shows the new
/// totals, with no manual refresh and no second navigation round trip.
#[gpui::test]
fn a_committed_import_shows_on_the_dashboard_without_a_manual_refresh(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, |_| {});
    import_demo_file(
        cx,
        handle,
        &shell,
        "simplii-chequing.csv",
        "No Fee Chequing",
    );
    wait_idle(cx, &shell);
    // 3.82: the import left the shell on the Imports page; visit the
    // dashboard once to see the first file's rows.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    wait_until(cx, "first import on the dashboard", |cx| {
        dash(cx, &shell).read(cx).row_count == 11
    });

    // Second file, committed from the Imports page into a second account:
    // the dashboard is hidden while it lands.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    import_demo_file(cx, handle, &shell, "amex-sep-oct.csv", "Cobalt");
    cx.update(|cx| {
        assert_eq!(shell.read(cx).page, Page::Imports, "still on Imports");
    });

    // The one visit shows both files: the hidden reload already ran.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Dashboard, cx)));
    // 11 + 67, minus the card payment the second account finally matches:
    // 77 rows with transfers hidden (the same arithmetic the Cobalt-only
    // import test makes).
    wait_until(cx, "both imports on the dashboard", |cx| {
        dash(cx, &shell).read(cx).row_count == 77
    });
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
        dash(cx, &shell).update(cx, |d, cx| {
            d.change_date_preset(DatePreset::AllTime, window, cx)
        });
    });
    // amex-sep-nov: 102 rows (100 unique + 2 genuine repeats), of which the
    // 2 card payments are auto-categorized as transfers and drop out of the
    // default view; simplii-chequing: 11 rows, of which the 2 "INTERNET BILL
    // PAY DEMO AMEX" debits pair with those payments (4.2) and are
    // categorized as Card Payment too.
    wait_until(cx, "all-time view over both imports", |cx| {
        dash(cx, &shell).read(cx).summary.months == ["2026-09", "2026-10", "2026-11"]
            && dash(cx, &shell).read(cx).row_count == 109
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
        let d = dash(cx, &shell).read(cx);
        let row = |name: &str| {
            d.summary
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
        let chart: Vec<(&str, i64)> = d
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
        let q = s.queue_view.read(cx);
        assert!(!q.groups.is_empty(), "the queue has groups");
        assert!(
            q.groups.windows(2).all(|w| w[0].count >= w[1].count),
            "sorted by count descending"
        );
        let rows: i64 = q.groups.iter().map(|g| g.count).sum();
        assert_eq!(
            rows, s.uncategorized_visible,
            "the queue covers every uncategorized row"
        );
        let cents: i64 = q.groups.iter().map(|g| g.total_cents).sum();
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
            .read(cx)
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
            s.imports.read(cx).import_history[0].first_date.as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(
            s.imports.read(cx).import_history[0].last_date.as_deref(),
            Some("2026-11-01")
        );
        assert_eq!(
            s.imports.read(cx).import_history[1].first_date.as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(
            s.imports.read(cx).import_history[1].last_date.as_deref(),
            Some("2026-11-06")
        );
    });
}
