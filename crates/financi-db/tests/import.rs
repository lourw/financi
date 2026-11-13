//! Import pipeline tests through the real Amex parser.

use financi_core::{AccountKind, CategoryKind, RawFile, StatementParser, read_raw_file};
use financi_db::categories::{
    between_accounts_category, card_payment_category, create_child, create_parent,
};
use financi_db::import::{
    CommitOptions, ImportError, ImportOutcome, commit_import, list_transactions, plan_import,
    undo_last_import,
};
use financi_db::master_csv::rewrite_years;
use financi_db::{add_rule, create_account, get_account, open_in_memory, rusqlite::Connection};
use financi_parsers::amex::Amex;
use financi_parsers::simplii::Simplii;

const HEADER: &str = "Date,Date Processed,Description,Amount";

fn csv(rows: &[&str]) -> RawFile {
    let text = std::iter::once(HEADER)
        .chain(rows.iter().copied())
        .collect::<Vec<_>>()
        .join("\n");
    read_raw_file(text.as_bytes()).unwrap()
}

/// Same rows under a different header — e.g. the bank added a column (3.68).
fn csv_with_header(header: &str, rows: &[&str]) -> RawFile {
    let text = std::iter::once(header)
        .chain(rows.iter().copied())
        .collect::<Vec<_>>()
        .join("\n");
    read_raw_file(text.as_bytes()).unwrap()
}

/// A Simplii chequing export: `Date, Transaction Details, Funds Out, Funds In `
/// with mm/dd/yyyy dates.
const SIMPLII_HEADER: &str = "Date, Transaction Details, Funds Out, Funds In ";

fn simplii_csv(rows: &[&str]) -> RawFile {
    let text = std::iter::once(SIMPLII_HEADER)
        .chain(rows.iter().copied())
        .collect::<Vec<_>>()
        .join("\n");
    read_raw_file(text.as_bytes()).unwrap()
}

fn fixture(name: &str) -> RawFile {
    let path = format!(
        "{}/../financi-parsers/tests/fixtures/amex/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    read_raw_file(&std::fs::read(path).unwrap()).unwrap()
}

/// A demo fixture (the realistic month files), read through the real parser.
fn demo_fixture(name: &str) -> RawFile {
    let path = format!("{}/../../fixtures/demo/{name}", env!("CARGO_MANIFEST_DIR"));
    read_raw_file(&std::fs::read(path).unwrap()).unwrap()
}

fn setup() -> (Connection, i64) {
    let conn = open_in_memory().unwrap();
    let cobalt = create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    (conn, cobalt)
}

fn import(conn: &mut Connection, account: i64, file: &RawFile) -> ImportOutcome {
    commit_import(
        conn,
        account,
        &Amex,
        file,
        "activity.csv",
        "2026-11-07T12:00:00",
        CommitOptions::default(),
    )
    .unwrap()
}

fn import_simplii(conn: &mut Connection, account: i64, file: &RawFile) -> ImportOutcome {
    commit_import(
        conn,
        account,
        &Simplii,
        file,
        "chequing.csv",
        "2026-11-07T12:00:00",
        CommitOptions::default(),
    )
    .unwrap()
}

fn count(conn: &Connection, account: i64) -> usize {
    list_transactions(conn, Some(account)).unwrap().len()
}

#[test]
fn fresh_import_stores_rows_raw_bytes_and_file_shape() {
    let (mut conn, acct) = setup();
    let file = fixture("basic.csv");

    let out = import(&mut conn, acct, &file);
    assert_eq!(out.inserted, 5, "both identical coffees are kept");
    assert_eq!(out.already_present, 0);

    let (header, trailing, line_ending): (Vec<u8>, bool, String) = conn
        .query_row(
            "SELECT header_raw, trailing_newline, line_ending FROM imports",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(header, HEADER.as_bytes());
    assert!(!trailing);
    assert_eq!(line_ending, "lf");

    let raw: Vec<u8> = conn
        .query_row(
            "SELECT raw_bytes FROM transactions WHERE source_line = 2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        raw, file.records[2].bytes,
        "quoted row stored byte for byte"
    );
    assert_eq!(raw.first(), Some(&b'0'));

    let rows = list_transactions(&conn, Some(acct)).unwrap();
    assert_eq!(rows[0].posted_date, "2026-11-06", "newest first");
    assert_eq!(rows[0].account, "American Express · Cobalt");
    let payment = rows
        .iter()
        .find(|r| r.description.starts_with("PAYMENT"))
        .unwrap();
    assert_eq!(payment.amount_cents, 150_000);
}

#[test]
fn reimporting_the_same_file_adds_nothing_and_records_no_import() {
    let (mut conn, acct) = setup();
    let file = fixture("basic.csv");
    import(&mut conn, acct, &file);

    let again = import(&mut conn, acct, &file);
    assert_eq!(
        again,
        ImportOutcome {
            import_id: None,
            inserted: 0,
            replaced: 0,
            already_present: 5,
            affected_years: Default::default(),
        }
    );
    assert_eq!(count(&conn, acct), 5);
    let imports: i64 = conn
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(imports, 1);
}

#[test]
fn overlapping_export_adds_only_new_rows() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &fixture("basic.csv"));

    // overlap.csv: one new noodle charge + 3 rows already imported
    let plan = plan_import(&conn, acct, &Amex, &fixture("overlap.csv"), "activity.csv").unwrap();
    assert_eq!(plan.total_rows, 4);
    assert_eq!(plan.already_present, 3);
    assert_eq!(plan.new_rows.len(), 1);
    assert!(plan.new_rows[0].txn.description_raw.starts_with("NOODLE"));
    assert_eq!(plan.date_range, ("2026-11-05".into(), "2026-11-09".into()));

    import(&mut conn, acct, &fixture("overlap.csv"));
    assert_eq!(count(&conn, acct), 6);
}

#[test]
fn identical_repeats_are_counted_not_collapsed() {
    let (mut conn, acct) = setup();
    let coffee = "06 Nov 2026,06 Nov 2026,COFFEE,5.75";
    import(&mut conn, acct, &csv(&[coffee, coffee]));

    // A later export shows three identical coffees that day: one is new.
    let out = import(&mut conn, acct, &csv(&[coffee, coffee, coffee]));
    assert_eq!((out.inserted, out.already_present), (1, 2));
    // And a file with just one of them adds nothing.
    assert_eq!(import(&mut conn, acct, &csv(&[coffee])).inserted, 0);
    assert_eq!(count(&conn, acct), 3);
}

#[test]
fn aliases_count_as_already_present() {
    let (mut conn, acct) = setup();
    import(
        &mut conn,
        acct,
        &csv(&["06 Nov 2026,06 Nov 2026,BISTRO,40.00"]),
    );

    // Simulate a confirmed near-match: the posted version (tip added) now
    // answers to the row's alias.
    let tipped = csv(&["06 Nov 2026,07 Nov 2026,BISTRO,46.00"]);
    let account = get_account(&conn, acct).unwrap();
    let alias = Amex.row_hash(&account.account_key, &tipped.records[0]);
    conn.execute(
        "INSERT INTO row_aliases (transaction_id, row_hash, import_id)
         SELECT id, ?1, import_id FROM transactions",
        [alias.0],
    )
    .unwrap();

    assert_eq!(import(&mut conn, acct, &tipped).inserted, 0);
}

#[test]
fn one_row_from_another_account_blocks_until_overridden() {
    let (mut conn, cobalt) = setup();
    let gold = create_account(&conn, "amex", "Gold", AccountKind::CreditCard).unwrap();
    let shared = "05 Nov 2026,06 Nov 2026,SHARED ROW,10.00";
    import(&mut conn, cobalt, &csv(&[shared]));

    let file = csv(&[shared, "06 Nov 2026,06 Nov 2026,ONLY ON GOLD,3.00"]);
    let plan = plan_import(&conn, gold, &Amex, &file, "activity.csv").unwrap();
    assert_eq!(
        plan.overlap.as_ref().map(|o| o.matching_rows),
        Some(1),
        "even one row is surfaced"
    );

    let err = commit_import(
        &mut conn,
        gold,
        &Amex,
        &file,
        "activity.csv",
        "t",
        CommitOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            ImportError::OtherAccountOverlap {
                matching_rows: 1,
                ..
            }
        ),
        "{err}"
    );
    assert!(
        err.to_string().contains("American Express · Cobalt"),
        "{err}"
    );
    assert_eq!(count(&conn, gold), 0, "nothing written without override");

    let forced = CommitOptions {
        allow_other_account_overlap: true,
        ..CommitOptions::default()
    };
    let out = commit_import(&mut conn, gold, &Amex, &file, "activity.csv", "t", forced).unwrap();
    assert_eq!(out.inserted, 2);
}

#[test]
fn reports_the_account_holding_the_most_rows() {
    let (mut conn, cobalt) = setup();
    let gold = create_account(&conn, "amex", "Gold", AccountKind::CreditCard).unwrap();
    import(&mut conn, cobalt, &fixture("basic.csv"));

    let plan = plan_import(&conn, gold, &Amex, &fixture("basic.csv"), "activity.csv").unwrap();
    let warning = plan.overlap.expect("wrong-account warning");
    assert_eq!(warning.account.name, "Cobalt");
    assert_eq!(warning.matching_rows, 5);

    let fresh = csv(&["09 Nov 2026,09 Nov 2026,SOMETHING NEW,1.00"]);
    assert!(
        plan_import(&conn, gold, &Amex, &fresh, "activity.csv")
            .unwrap()
            .overlap
            .is_none()
    );
}

#[test]
fn rejects_wrong_institution_and_account_kind() {
    let (conn, _) = setup();
    let chequing =
        create_account(&conn, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();
    let err = plan_import(&conn, chequing, &Amex, &fixture("basic.csv"), "x.csv").unwrap_err();
    assert!(matches!(err, ImportError::WrongInstitution { .. }), "{err}");

    let odd = create_account(&conn, "amex", "Odd", AccountKind::Chequing).unwrap();
    let err = plan_import(&conn, odd, &Amex, &fixture("basic.csv"), "x.csv").unwrap_err();
    assert!(matches!(err, ImportError::WrongAccountKind { .. }), "{err}");
    assert_eq!(
        err.to_string(),
        "this export comes from Credit card accounts, but the account is a Chequing account"
    );
}

#[test]
fn a_bad_row_rejects_the_whole_file() {
    let (mut conn, acct) = setup();
    let err = commit_import(
        &mut conn,
        acct,
        &Amex,
        &fixture("bad_date.csv"),
        "x.csv",
        "t",
        CommitOptions::default(),
    )
    .unwrap_err();
    match err {
        ImportError::Parse(errors) => assert_eq!(errors.len(), 1),
        other => panic!("{other}"),
    }
    assert_eq!(count(&conn, acct), 0, "nothing written");
}

#[test]
fn undo_removes_latest_import_first_and_only_for_that_account() {
    let (mut conn, cobalt) = setup();
    let gold = create_account(&conn, "amex", "Gold", AccountKind::CreditCard).unwrap();
    import(&mut conn, cobalt, &fixture("basic.csv"));
    import(&mut conn, cobalt, &fixture("overlap.csv"));
    import(
        &mut conn,
        gold,
        &csv(&["01 Nov 2026,01 Nov 2026,GOLD ONLY,9.99"]),
    );

    let undone = undo_last_import(&mut conn, cobalt).unwrap().unwrap();
    assert_eq!(undone.removed, 1, "overlap.csv added one row");
    assert_eq!(count(&conn, cobalt), 5);

    assert_eq!(
        undo_last_import(&mut conn, cobalt)
            .unwrap()
            .unwrap()
            .removed,
        5
    );
    assert!(undo_last_import(&mut conn, cobalt).unwrap().is_none());
    assert_eq!(count(&conn, gold), 1, "other account untouched");
}

#[test]
fn stale_preview_cannot_double_insert() {
    let (mut conn, acct) = setup();
    let file = fixture("basic.csv");
    let plan = plan_import(&conn, acct, &Amex, &file, "activity.csv").unwrap();
    assert_eq!(plan.new_rows.len(), 5);

    import(&mut conn, acct, &file);
    import(&mut conn, acct, &file); // user clicks Import twice on the same preview
    assert_eq!(count(&conn, acct), 5);
}

#[test]
fn two_years_of_rows_import_quickly() {
    let (mut conn, acct) = setup();
    let rows: Vec<String> = (0..2_000)
        .map(|i| {
            format!(
                "{:02} Nov 2026,{:02} Nov 2026,MERCHANT {i},{}.{:02}",
                1 + i % 28,
                1 + i % 28,
                i % 300,
                i % 100
            )
        })
        .collect();
    let file = csv(&rows.iter().map(String::as_str).collect::<Vec<_>>());

    let start = std::time::Instant::now();
    assert_eq!(import(&mut conn, acct, &file).inserted, 2_000);
    assert_eq!(import(&mut conn, acct, &file).inserted, 0);
    // Acceptance target is 2 s for release builds; allow slack in debug.
    assert!(
        start.elapsed().as_secs_f64() < 5.0,
        "took {:?}",
        start.elapsed()
    );
}

/// The stored `(category_id, category_locked)` of the row with this exact
/// description. Panics when there is none or more than one.
fn stored(conn: &Connection, description: &str) -> (Option<i64>, bool) {
    conn.query_row(
        "SELECT category_id, category_locked FROM transactions WHERE description_raw = ?1",
        [description],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

#[test]
fn import_categorizes_new_rows_with_rules() {
    let (mut conn, acct) = setup();
    let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
    let coffee = create_child(&conn, food, "Coffee").unwrap();
    add_rule(&conn, "COFFEE", false, coffee).unwrap();

    import(
        &mut conn,
        acct,
        &csv(&[
            "06 Nov 2026,06 Nov 2026,COFFEE VANCOUVER,5.75",
            "06 Nov 2026,06 Nov 2026,BISTRO VANCOUVER,40.00",
        ]),
    );

    let (cat, locked) = stored(&conn, "COFFEE VANCOUVER");
    assert_eq!(cat, Some(coffee), "the rule applies at import");
    assert!(!locked, "rule-categorized rows stay unlocked");
    assert_eq!(stored(&conn, "BISTRO VANCOUVER"), (None, false));
}

#[test]
fn card_payment_beats_conflicting_user_rules_and_locks_the_row() {
    let (mut conn, acct) = setup();
    let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
    let payments = create_child(&conn, food, "Payments").unwrap();
    add_rule(&conn, "PAYMENT", false, payments).unwrap();

    import(
        &mut conn,
        acct,
        &csv(&[
            "25 Oct 2026,25 Oct 2026,PAYMENT RECEIVED - THANK YOU,-1500.00",
            "06 Nov 2026,06 Nov 2026,COFFEE VANCOUVER,5.75",
        ]),
    );

    let (cat, locked) = stored(&conn, "PAYMENT RECEIVED - THANK YOU");
    assert_eq!(cat, Some(card_payment_category(&conn).unwrap()));
    assert_ne!(cat, Some(payments), "user rules lose to the built-in check");
    assert!(locked, "card payments are locked against rule re-runs");

    // A later rule change re-runs over unlocked rows and leaves it alone.
    let more = create_child(&conn, food, "More").unwrap();
    add_rule(&conn, "RECEIVED", false, more).unwrap();
    assert_eq!(stored(&conn, "PAYMENT RECEIVED - THANK YOU"), (cat, true));
}

#[test]
fn simplii_chequing_income_containing_payment_is_not_a_card_payment() {
    // 3.66: the Visa marker rule must not run on chequing imports — a
    // ` GST PAYMENT CANADA` deposit is income, not a locked transfer.
    let conn = open_in_memory().unwrap();
    let chequing =
        create_account(&conn, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();
    let mut conn = conn;

    import_simplii(
        &mut conn,
        chequing,
        &simplii_csv(&[
            "10/20/2026, GST PAYMENT CANADA,,100.00",
            "10/21/2026, PAYMENT REVERSAL,,75.00",
            "10/09/2026, BILL SEND SAMPLE CO LTD,25.00,",
        ]),
    );

    let card = card_payment_category(&conn).unwrap();
    for description in [" GST PAYMENT CANADA", " PAYMENT REVERSAL"] {
        let (cat, locked) = stored(&conn, description);
        assert_ne!(cat, Some(card), "{description} is not a card payment");
        assert!(!locked, "{description} stays open to rules");
    }
    // Money in, visible and uncategorized: income the totals include and
    // the To-categorize queue surfaces (locked transfers were invisible).
    let rows = list_transactions(&conn, Some(chequing)).unwrap();
    let income: i64 = rows
        .iter()
        .filter(|r| r.category_id != Some(card))
        .map(|r| r.amount_cents)
        .sum();
    assert_eq!(income, 10_000 + 7_500 - 2_500);
}

#[test]
fn simplii_visa_payment_shape_decides_card_payment() {
    // 3.66 on the Visa side: `PAYMENT RECEIVED THANK YOU` is still a card
    // payment, but a refund from a merchant named PAYMENT is income.
    let conn = open_in_memory().unwrap();
    let visa = create_account(&conn, "simplii", "Visa", AccountKind::CreditCard).unwrap();
    let mut conn = conn;

    import_simplii(
        &mut conn,
        visa,
        &simplii_csv(&[
            "10/12/2026, PAYMENT RECEIVED THANK YOU,,250.00",
            "10/22/2026, PAYPAL *PAYMENTS REFUND,,42.00",
        ]),
    );

    let (cat, locked) = stored(&conn, " PAYMENT RECEIVED THANK YOU");
    assert_eq!(cat, Some(card_payment_category(&conn).unwrap()));
    assert!(locked, "the real payment into the card keeps its lock");
    assert_eq!(stored(&conn, " PAYPAL *PAYMENTS REFUND"), (None, false));
}

#[test]
fn history_lists_imports_newest_first_with_their_rows() {
    use financi_db::import::list_imports;

    let (mut conn, acct) = setup();
    let first = csv(&["01 Nov 2026,01 Nov 2026,COFFEE ONE,-5.00"]);
    let second = csv(&[
        "05 Nov 2026,05 Nov 2026,COFFEE TWO,-4.00",
        "06 Nov 2026,06 Nov 2026,BISTRO,-40.00",
    ]);
    import(&mut conn, acct, &first);
    import(&mut conn, acct, &second);

    let history = list_imports(&conn).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].file_name, "activity.csv");
    assert_eq!(history[0].rows_added, 2);
    assert_eq!(history[0].account, "American Express · Cobalt");
    assert_eq!(history[0].first_date.as_deref(), Some("2026-11-05"));
    assert_eq!(history[0].last_date.as_deref(), Some("2026-11-06"));
    assert_eq!(history[1].rows_added, 1);

    let undone = undo_last_import(&mut conn, acct).unwrap();
    assert!(undone.is_some());
    let history = list_imports(&conn).unwrap();
    assert_eq!(
        history.len(),
        1,
        "the undone import is gone from the history"
    );
}

#[test]
fn importing_a_statement_pairs_the_card_payment_with_the_chequing_debit() {
    let (mut conn, cobalt) = setup();
    let chequing = create_account(&conn, "simplii", "Chequing", AccountKind::Chequing).unwrap();
    // The debit that funded the fixture's 25 Sep payment, one day later.
    conn.execute(
        "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                              encoding, has_bom, line_ending, trailing_newline, row_count)
         VALUES (?1, 'amex', 'prior.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
        rusqlite::params![chequing],
    )
    .unwrap();
    let prior_import = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                   posted_date, amount_cents, description_raw)
         VALUES (?1, ?2, 0, X'00', 'prior-debit', '2026-09-26', -150000,
                 'CREDIT CARD PAYMENT - AMEX')",
        rusqlite::params![chequing, prior_import],
    )
    .unwrap();

    let prior_debit = conn.last_insert_rowid();

    import(&mut conn, cobalt, &demo_fixture("amex-sep-nov.csv"));

    let rows = list_transactions(&conn, Some(cobalt)).unwrap();
    let payments: Vec<_> = rows
        .iter()
        .filter(|r| r.description.contains("PAYMENT RECEIVED"))
        .collect();
    assert_eq!(payments.len(), 2, "the fixture has two payments");
    let sep = payments
        .iter()
        .find(|r| r.posted_date == "2026-09-25")
        .unwrap();
    let oct = payments
        .iter()
        .find(|r| r.posted_date == "2026-10-25")
        .unwrap();
    let card_payment = card_payment_category(&conn).unwrap();
    assert_eq!(
        sep.transfer_pair_id,
        Some(prior_debit),
        "the September payment pairs with the chequing debit inside 5 days"
    );
    assert_eq!(
        sep.category_id,
        Some(card_payment),
        "the payment takes its partner's category"
    );
    assert_eq!(
        oct.transfer_pair_id, None,
        "the October payment has no debit near it and stays unpaired"
    );
    assert_eq!(
        oct.category_id,
        Some(card_payment),
        "the October payment is still Card Payment from import (2.5), just unpaired"
    );
    let debits = list_transactions(&conn, Some(chequing)).unwrap();
    assert_eq!(debits[0].transfer_pair_id, Some(sep.id));
    assert_eq!(
        debits[0].category_id,
        Some(card_payment),
        "the chequing debit takes the Card Payment category from pairing"
    );
}

#[test]
fn a_row_ambiguous_under_both_rules_leaves_one_review_and_imports_calmly() {
    let (mut conn, cobalt) = setup();
    let alpha = create_account(&conn, "simplii", "Alpha", AccountKind::Chequing).unwrap();
    let beta = create_account(&conn, "simplii", "Beta", AccountKind::Chequing).unwrap();

    // Two card payments on the card…
    import(
        &mut conn,
        cobalt,
        &csv(&[
            "25 Oct 2026,25 Oct 2026,PAYMENT RECEIVED,-1500.00",
            "28 Oct 2026,28 Oct 2026,PAYMENT RECEIVED,-1500.00",
        ]),
    );
    // …two transfer credits on Beta…
    import_simplii(
        &mut conn,
        beta,
        &simplii_csv(&[
            "10/26/2026,TRANSFER IN,,1500.00",
            "10/27/2026,TRANSFER IN,,1500.00",
        ]),
    );
    // …and one Alpha debit ambiguous under both rules at once. Committing
    // this import used to crash on the review table's UNIQUE constraint.
    import_simplii(
        &mut conn,
        alpha,
        &simplii_csv(&["10/26/2026,BILL SEND,1500.00,"]),
    );

    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM transfer_reviews", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "one review per row, not one per rule");
    let list: String = conn
        .query_row(
            "SELECT r.candidate_ids FROM transfer_reviews r
               JOIN transactions t ON t.id = r.transaction_id
              WHERE t.description_raw = 'BILL SEND'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        list.split(',').count(),
        4,
        "the review carries the union of both rules' candidates"
    );
}

#[test]
fn undoing_a_paired_import_releases_the_surviving_partner() {
    let (mut conn, cobalt) = setup();
    let alpha = create_account(&conn, "simplii", "Alpha", AccountKind::Chequing).unwrap();
    let bills = create_parent(&conn, "Bills", CategoryKind::Expense).unwrap();
    let card_bill = create_child(&conn, bills, "Card Bill").unwrap();
    add_rule(&conn, "BILL PAYMENT", false, card_bill).unwrap();

    // A pair formed at import: the statement payment and its debit.
    import(
        &mut conn,
        cobalt,
        &csv(&["25 Oct 2026,25 Oct 2026,PAYMENT RECEIVED,-1500.00"]),
    );
    import_simplii(
        &mut conn,
        alpha,
        &simplii_csv(&["10/26/2026,BILL PAYMENT AMEX,1500.00,"]),
    );
    let card_payment = card_payment_category(&conn).unwrap();
    assert_eq!(
        stored(&conn, "BILL PAYMENT AMEX"),
        (Some(card_payment), true),
        "pairing categorized the debit and locked it"
    );

    // Undo the statement: the surviving debit loses link, lock and transfer
    // category — rules re-apply, as after *Unpair*.
    undo_last_import(&mut conn, cobalt).unwrap().unwrap();
    assert_eq!(
        stored(&conn, "BILL PAYMENT AMEX"),
        (Some(card_bill), false),
        "the partner is unlocked and re-categorized by rules"
    );
    let pair: Option<i64> = conn
        .query_row(
            "SELECT transfer_pair_id FROM transactions
              WHERE description_raw = 'BILL PAYMENT AMEX'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pair, None, "the link is gone with its partner");

    // And it is available for future pairing: a fresh payment finds it.
    import(
        &mut conn,
        cobalt,
        &csv(&["25 Oct 2026,25 Oct 2026,PAYMENT RECEIVED AGAIN,-1500.00"]),
    );
    let pair: Option<i64> = conn
        .query_row(
            "SELECT transfer_pair_id FROM transactions
              WHERE description_raw = 'BILL PAYMENT AMEX'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        pair.is_some(),
        "the released debit pairs with the new payment"
    );
    assert_eq!(stored(&conn, "BILL PAYMENT AMEX").0, Some(card_payment));
}

#[test]
fn undo_re_runs_pairing_and_drops_stale_reviews() {
    let mut conn = open_in_memory().unwrap();
    let alpha = create_account(&conn, "simplii", "Alpha", AccountKind::Chequing).unwrap();
    let beta = create_account(&conn, "simplii", "Beta", AccountKind::Chequing).unwrap();
    let gamma = create_account(&conn, "simplii", "Gamma", AccountKind::Chequing).unwrap();

    // Two transfer credits…
    import_simplii(
        &mut conn,
        beta,
        &simplii_csv(&["10/27/2026,TRANSFER IN,,1500.00"]),
    );
    import_simplii(
        &mut conn,
        gamma,
        &simplii_csv(&["10/28/2026,TRANSFER IN,,1500.00"]),
    );
    // …and one debit ambiguous between them: a review, no pair.
    import_simplii(
        &mut conn,
        alpha,
        &simplii_csv(&["10/26/2026,TRANSFER OUT,1500.00,"]),
    );
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM transfer_reviews", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "the debit is ambiguous while both credits exist");

    // Undoing Beta's import deletes one credit. Pairing runs at the end of
    // the undo (3.69): the debit pairs with the surviving credit at once,
    // and no review is left pointing at the deleted row.
    undo_last_import(&mut conn, beta).unwrap().unwrap();
    let gamma_row: i64 = conn
        .query_row(
            "SELECT id FROM transactions WHERE account_id = ?1",
            [gamma],
            |r| r.get(0),
        )
        .unwrap();
    let pair: Option<i64> = conn
        .query_row(
            "SELECT transfer_pair_id FROM transactions
              WHERE description_raw = 'TRANSFER OUT'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        pair,
        Some(gamma_row),
        "the pair forms at undo, not next import"
    );
    let between = between_accounts_category(&conn).unwrap();
    assert_eq!(stored(&conn, "TRANSFER OUT").0, Some(between));
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM transfer_reviews", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "reviews are recomputed, none points at a deleted row");
}

// ---------------------------------------------------------------------------
// Near-match review (4.3)

/// Commit `file`, marking the given preview pairs as "Same charge".
fn import_marking(
    conn: &mut Connection,
    account: i64,
    file: &RawFile,
    same_charges: Vec<(usize, i64)>,
) -> ImportOutcome {
    commit_import(
        conn,
        account,
        &Amex,
        file,
        "activity.csv",
        "2026-11-07T12:00:00",
        CommitOptions {
            same_charges,
            ..CommitOptions::default()
        },
    )
    .unwrap()
}

fn plan(conn: &Connection, account: i64, file: &RawFile) -> financi_db::import::ImportPlan {
    plan_import(conn, account, &Amex, file, "activity.csv").unwrap()
}

fn txn_id(conn: &Connection, description: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM transactions WHERE description_raw = ?1",
        [description],
        |r| r.get(0),
    )
    .unwrap()
}

fn row_values(conn: &Connection, id: i64) -> (String, i64, String, Vec<u8>, String) {
    conn.query_row(
        "SELECT posted_date, amount_cents, description_raw, raw_bytes, row_hash
           FROM transactions WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .unwrap()
}

fn fx_values(conn: &Connection, id: i64) -> (Option<String>, Option<i64>) {
    conn.query_row(
        "SELECT original_currency, original_amount_cents FROM transactions WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

const COFFEE: &str = "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74";
const COFFEE_TIP: &str = "06 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,18.74";

#[test]
fn tip_added_charge_is_offered_as_a_near_match() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));

    let p = plan(&conn, acct, &csv(&[COFFEE_TIP]));
    assert_eq!(p.new_rows.len(), 1);
    assert_eq!(p.near_matches.len(), 1, "the tip is suspected");
    let m = &p.near_matches[0];
    assert_eq!(m.new_row, 0);
    assert_eq!(m.source_line, 0, "the file's first record");
    assert_eq!(m.existing_id, txn_id(&conn, "COFFEE SHOP    VANCOUVER"));
    assert_eq!(
        (m.new_date.as_str(), m.new_amount_cents),
        ("2026-11-06", -1874)
    );
    assert_eq!(
        (m.existing_date.as_str(), m.existing_amount_cents),
        ("2026-11-05", -1574)
    );
}

#[test]
fn near_match_windows_are_inclusive_and_merchant_sensitive() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));

    let offered = |row: &str| plan(&conn, acct, &csv(&[row])).near_matches.len() == 1;
    // Amount: 20% of 1574 is 314.8, so up to 18.88 is within, 18.89 is not.
    assert!(offered(
        "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,18.88"
    ));
    assert!(!offered(
        "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,18.89"
    ));
    // Date: ±3 days inclusive.
    assert!(offered(
        "08 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74"
    ));
    assert!(offered(
        "02 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74"
    ));
    assert!(!offered(
        "09 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74"
    ));
    // Merchant: case-insensitive on the cleaned merchant…
    assert!(offered(
        "05 Nov 2026,08 Nov 2026,Coffee Shop    vancouver,15.74"
    ));
    // …but a different merchant never pairs.
    assert!(!offered(
        "05 Nov 2026,08 Nov 2026,TEA SHOP    VANCOUVER,15.74"
    ));
    // A payment (opposite direction) never pairs with a charge.
    assert!(!offered(
        "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,-15.74"
    ));
}

#[test]
fn near_matches_take_the_nearest_existing_row_then_the_lowest_id() {
    let (mut conn, acct) = setup();
    import(
        &mut conn,
        acct,
        &csv(&[
            "04 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74",
            "08 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74",
        ]),
    );
    let ids: Vec<(i64, String)> = conn
        .prepare("SELECT id, posted_date FROM transactions ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids.len(), 2);
    let (early_id, early_date) = &ids[0];
    let (late_id, late_date) = (&ids[1].0, &ids[1].1);

    // Nearest posted date: 07 Nov is 1 day from 08 Nov, 3 from 04 Nov.
    let p = plan(
        &conn,
        acct,
        &csv(&["07 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,16.00"]),
    );
    assert_eq!(p.near_matches.len(), 1);
    assert_eq!(p.near_matches[0].existing_id, *late_id);
    assert_eq!(&p.near_matches[0].existing_date, late_date);

    // Equidistant candidates: the lower transaction id wins.
    let p = plan(
        &conn,
        acct,
        &csv(&["06 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,16.00"]),
    );
    assert_eq!(p.near_matches.len(), 1);
    assert_eq!(p.near_matches[0].existing_id, *early_id);
    assert_eq!(&p.near_matches[0].existing_date, early_date);
}

#[test]
fn an_existing_row_matches_only_one_new_row() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));

    // Two new rows both look like the one existing charge: only the first
    // (file order) pairs; the second imports as new.
    let p = plan(
        &conn,
        acct,
        &csv(&[
            "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,16.00",
            "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,16.10",
        ]),
    );
    assert_eq!(p.near_matches.len(), 1);
    assert_eq!(p.near_matches[0].new_row, 0);

    // With two existing rows, two new rows can each take one.
    import(
        &mut conn,
        acct,
        &csv(&["03 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,15.74"]),
    );
    let p = plan(
        &conn,
        acct,
        &csv(&[
            "05 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,16.00",
            "03 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,16.10",
        ]),
    );
    assert_eq!(p.near_matches.len(), 2);
    assert_ne!(
        p.near_matches[0].existing_id, p.near_matches[1].existing_id,
        "one-to-one"
    );
}

#[test]
fn repeat_purchase_still_in_the_file_is_not_a_near_match() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));

    // A re-export holding yesterday's coffee plus today's bigger one: the
    // earlier row is still in the file unchanged, so it is a repeat
    // purchase, not a changed charge — today's coffee must not be offered
    // against it as "Same charge?" (3.68).
    let reexport = csv(&[COFFEE_TIP, COFFEE]);
    let p = plan(&conn, acct, &reexport);
    assert_eq!(p.already_present, 1, "yesterday's coffee reconciles");
    assert_eq!(p.new_rows.len(), 1, "today's coffee is new");
    assert_eq!(
        p.near_matches.len(),
        0,
        "the repeat is not a changed charge"
    );

    // The exclusion follows the file: without yesterday's row, the very
    // same charge is offered against the existing coffee again.
    assert_eq!(plan(&conn, acct, &csv(&[COFFEE_TIP])).near_matches.len(), 1);
}

#[test]
fn an_aliased_repeat_purchase_still_in_the_file_is_not_a_near_match() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));
    let id = txn_id(&conn, "COFFEE SHOP    VANCOUVER");
    import_marking(&mut conn, acct, &csv(&[COFFEE_TIP]), vec![(0, id)]);

    // The stored row now carries the tip's hash, with the coffee's old hash
    // aliased to it. A re-export still holding the old coffee holds that
    // row unchanged — through the alias — so a further lookalike must not
    // pair with it (3.68).
    let lookalike = "07 Nov 2026,08 Nov 2026,COFFEE SHOP    VANCOUVER,16.00";
    assert_eq!(
        plan(&conn, acct, &csv(&[lookalike, COFFEE]))
            .near_matches
            .len(),
        0,
        "the aliased repeat is excluded"
    );
    assert_eq!(
        plan(&conn, acct, &csv(&[lookalike])).near_matches.len(),
        1,
        "without the repeat in the file it pairs again"
    );
}

#[test]
fn a_changed_header_or_bom_does_not_offer_near_matches() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));
    let id = txn_id(&conn, "COFFEE SHOP    VANCOUVER");

    // Control: same variant, the tip is offered…
    let tip = csv(&[COFFEE_TIP]);
    assert_eq!(plan(&conn, acct, &tip).near_matches.len(), 1);

    // …but under a different header variant it is not (3.68): a
    // replacement keeps the existing row's import, so its new bytes would
    // be written to the master CSV under the old import's header.
    let changed = csv_with_header(
        "Date,Date Processed,Description,Amount,FX Currency",
        &[COFFEE_TIP],
    );
    let p = plan(&conn, acct, &changed);
    assert_eq!(p.new_rows.len(), 1);
    assert_eq!(p.near_matches.len(), 0, "different header variant");

    // Marking the stale pair anyway changes nothing: the row imports new,
    // so the master CSV keeps writing it under its own header.
    let out = import_marking(&mut conn, acct, &changed, vec![(0, id)]);
    assert_eq!((out.inserted, out.replaced), (1, 0), "imports as new");
    assert_eq!(count(&conn, acct), 2, "the existing row survives");

    // A UTF-8 BOM changes the variant the same way.
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));
    let bom = read_raw_file(format!("\u{feff}{HEADER}\n{COFFEE_TIP}").as_bytes()).unwrap();
    assert!(bom.has_bom, "the core records the BOM");
    assert_eq!(
        plan(&conn, acct, &bom).near_matches.len(),
        0,
        "different encoding variant"
    );
}

#[test]
fn same_charge_replaces_and_aliases_then_overlapping_export_adds_zero() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));
    let food = create_parent(&conn, "Food", CategoryKind::Expense).unwrap();
    let id = txn_id(&conn, "COFFEE SHOP    VANCOUVER");
    conn.execute(
        "UPDATE transactions SET category_id = ?1 WHERE id = ?2",
        rusqlite::params![food, id],
    )
    .unwrap();
    let (old_date, old_amount, old_desc, old_raw, old_hash) = row_values(&conn, id);

    let tip = csv(&[COFFEE_TIP]);
    let out = import_marking(&mut conn, acct, &tip, vec![(0, id)]);
    assert_eq!((out.inserted, out.replaced), (0, 1));
    assert!(out.import_id.is_some());

    // The existing id survives with the new values…
    assert_eq!(count(&conn, acct), 1, "nothing new was inserted");
    let (date, amount, desc, raw, hash) = row_values(&conn, id);
    assert_eq!(date, "2026-11-06");
    assert_eq!(amount, -1874);
    assert_eq!(desc, "COFFEE SHOP    VANCOUVER");
    assert_eq!(raw, tip.records[0].bytes, "raw bytes replaced");
    assert_ne!(hash, old_hash);
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT category_id FROM transactions WHERE id = ?1",
            [id],
            |r| r.get(0)
        )
        .unwrap(),
        food,
        "id and category survive"
    );

    // …the old hash is aliased to it under the new import…
    let import_id: i64 = conn
        .query_row("SELECT id FROM imports ORDER BY id DESC LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let aliases: Vec<(i64, String, i64)> = conn
        .prepare("SELECT transaction_id, row_hash, import_id FROM row_aliases")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0], (id, old_hash.clone(), import_id));

    // …and the replacement is recorded with the old values.
    let recorded: (String, Option<String>, i64, String, Vec<u8>, Option<String>) = conn
        .query_row(
            "SELECT old_posted_date, old_processed_date, old_amount_cents,
                    old_description_raw, old_raw_bytes, old_merchant
               FROM replacements WHERE import_id = ?1 AND transaction_id = ?2",
            rusqlite::params![import_id, id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        recorded,
        (
            old_date,
            Some("2026-11-08".to_string()),
            old_amount,
            old_desc,
            old_raw,
            Some("COFFEE SHOP".to_string())
        )
    );

    // A later overlapping export of the same file adds nothing: it matches
    // the new hash directly.
    let out = import(&mut conn, acct, &tip);
    assert_eq!((out.inserted, out.replaced), (0, 0));
    assert_eq!(out.import_id, None, "nothing to record");

    // Even the old file reconciles silently through the alias.
    let old_file = csv(&[COFFEE]);
    let out = import(&mut conn, acct, &old_file);
    assert_eq!(out.inserted, 0, "the alias answers the old hash");
}

#[test]
fn undo_restores_the_replaced_transaction() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));
    let id = txn_id(&conn, "COFFEE SHOP    VANCOUVER");
    let before = row_values(&conn, id);

    let tip = csv(&[COFFEE_TIP]);
    import_marking(&mut conn, acct, &tip, vec![(0, id)]);
    assert_ne!(row_values(&conn, id), before);

    let undone = undo_last_import(&mut conn, acct).unwrap().unwrap();
    assert_eq!(
        undone.removed, 0,
        "the replacement belonged to the earlier import"
    );
    assert_eq!(row_values(&conn, id), before, "old values restored");
    let aliases: i64 = conn
        .query_row("SELECT COUNT(*) FROM row_aliases", [], |r| r.get(0))
        .unwrap();
    assert_eq!(aliases, 0, "the import's aliases cascade away with it");
    let reps: i64 = conn
        .query_row("SELECT COUNT(*) FROM replacements", [], |r| r.get(0))
        .unwrap();
    assert_eq!(reps, 0);

    // After undo the original file is still "already present"…
    let out = import(&mut conn, acct, &csv(&[COFFEE]));
    assert_eq!((out.inserted, out.already_present), (0, 1));
    // …and the tip file offers its near-match again.
    assert_eq!(plan(&conn, acct, &tip).near_matches.len(), 1);
}

#[test]
fn same_charge_replaces_and_restores_the_fx_columns() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));
    let id = txn_id(&conn, "COFFEE SHOP    VANCOUVER");
    // A foreign-currency charge: the Amex parser leaves the FX columns
    // NULL, so set them the way an FX-aware import would (3.68).
    conn.execute(
        "UPDATE transactions SET original_currency = 'USD', original_amount_cents = 1234
          WHERE id = ?1",
        [id],
    )
    .unwrap();

    let tip = csv(&[COFFEE_TIP]);
    let out = import_marking(&mut conn, acct, &tip, vec![(0, id)]);
    assert_eq!(out.replaced, 1);

    // The replacement carries the new row's FX values…
    assert_eq!(fx_values(&conn, id), (None, None));
    // …and the old ones are recorded for undo.
    let import_id: i64 = conn
        .query_row("SELECT id FROM imports ORDER BY id DESC LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let recorded: (Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT old_original_currency, old_original_amount_cents
               FROM replacements WHERE import_id = ?1 AND transaction_id = ?2",
            rusqlite::params![import_id, id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(recorded, (Some("USD".to_string()), Some(1234)));

    // Undo puts them back.
    undo_last_import(&mut conn, acct).unwrap().unwrap();
    assert_eq!(
        fx_values(&conn, id),
        (Some("USD".to_string()), Some(1234)),
        "undo restores the FX columns"
    );
}

#[test]
fn different_charge_imports_as_new() {
    let (mut conn, acct) = setup();
    import(&mut conn, acct, &csv(&[COFFEE]));
    let id = txn_id(&conn, "COFFEE SHOP    VANCOUVER");
    let before = row_values(&conn, id);

    // The pair is offered, but the default (Different charge) is not marked.
    let tip = csv(&[COFFEE_TIP]);
    assert_eq!(plan(&conn, acct, &tip).near_matches.len(), 1);
    let out = import(&mut conn, acct, &tip);
    assert_eq!((out.inserted, out.replaced), (1, 0));
    assert_eq!(count(&conn, acct), 2, "the tip imports as a new row");
    assert_eq!(
        row_values(&conn, id),
        before,
        "the existing row is untouched"
    );
}

#[test]
fn same_charge_moves_the_master_csv_row_across_the_year_boundary() {
    let (mut conn, acct) = setup();
    let root =
        std::env::temp_dir().join(format!("financi-near-match-csv-{}", uuid::Uuid::new_v4()));
    let account = get_account(&conn, acct).unwrap();
    let dir = root
        .join(&account.institution.key)
        .join(&account.account_key.0);

    // Old row in 2026, its replacement three days later, across the year
    // boundary: the two dates live in different master CSVs.
    let old = "30 Dec 2026,31 Dec 2026,LATE SNACKS    TORONTO,15.74";
    let new = "02 Jan 2027,03 Jan 2027,LATE SNACKS    TORONTO,18.74";
    import(&mut conn, acct, &csv(&[old]));
    rewrite_years(&conn, &root, acct, &[2026]).unwrap();
    assert_eq!(
        std::fs::read(dir.join("2026.csv")).unwrap(),
        format!("{HEADER}\n{old}").as_bytes()
    );

    // "Same charge" moves the row: 2026 empties out, 2027 holds the new
    // bytes exactly as exported, and both years are flagged for rewriting.
    let id = txn_id(&conn, "LATE SNACKS    TORONTO");
    let out = import_marking(&mut conn, acct, &csv(&[new]), vec![(0, id)]);
    assert_eq!(out.affected_years, [2026, 2027].into_iter().collect());
    rewrite_years(&conn, &root, acct, &[2026, 2027]).unwrap();
    assert!(!dir.join("2026.csv").exists(), "the old row is gone");
    assert_eq!(
        std::fs::read(dir.join("2027.csv")).unwrap(),
        format!("{HEADER}\n{new}").as_bytes(),
        "the new row appears exactly as exported"
    );

    // Undo puts the old row back, byte for byte, and empties 2027.
    let undone = undo_last_import(&mut conn, acct).unwrap().unwrap();
    assert_eq!(undone.affected_years, [2026, 2027].into_iter().collect());
    rewrite_years(&conn, &root, acct, &[2026, 2027]).unwrap();
    assert!(!dir.join("2027.csv").exists());
    assert_eq!(
        std::fs::read(dir.join("2026.csv")).unwrap(),
        format!("{HEADER}\n{old}").as_bytes()
    );
    std::fs::remove_dir_all(root).unwrap();
}
