//! Tests for `ui/transfers.rs` (3.62 split from `ui_tests.rs`).

use super::*;
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
        shell.read(cx).transfers.transfer_reviews.len() == 1
    });
    let review = cx.update(|cx| shell.read(cx).transfers.transfer_reviews[0].clone());
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
        shell.read(cx).transfers.transfer_reviews.is_empty()
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
        shell.read(cx).transfers.transfer_reviews.len() == 1
    });

    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.mark_not_transfer(debit, window, cx));
    });
    wait_until(cx, "review gone", |cx| {
        shell.read(cx).transfers.transfer_reviews.is_empty()
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
        assert!(shell.read(cx).transfers.transfer_reviews.is_empty());
        let rows = table_rows(&shell, cx);
        let find = |id: i64| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(find(out).category, None);
        assert_eq!(find(incoming).category, None);
    });

    // …and the engine's re-run leaves them apart: no new review, no pair.
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(shell.read(cx).transfers.transfer_reviews.is_empty());
        let rows = table_rows(&shell, cx);
        assert!(rows.iter().all(|r| r.transfer_pair_id.is_none()));
    });
}
