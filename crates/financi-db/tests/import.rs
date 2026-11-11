//! Import pipeline tests through the real Amex parser.

use financi_core::{AccountKind, CategoryKind, RawFile, StatementParser, read_raw_file};
use financi_db::categories::{card_payment_category, create_child, create_parent};
use financi_db::import::{
    CommitOptions, ImportError, ImportOutcome, commit_import, list_transactions, plan_import,
    undo_last_import,
};
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
