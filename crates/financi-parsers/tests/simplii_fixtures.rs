//! Parses whole synthetic Simplii exports (chequing and Visa) end to end.

use financi_core::{AccountKey, read_raw_file};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/simplii/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn basic_export_parses_completely() {
    let file = read_raw_file(&fixture("basic.csv")).unwrap();
    assert!(file.trailing_newline, "Simplii exports end with a newline");

    let parser = financi_parsers::detect(&file.header).expect("detected");
    assert_eq!(parser.id(), "simplii");
    assert_eq!(parser.institution(), "simplii");

    let txns: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.map(r).unwrap())
        .collect();
    assert_eq!(txns.len(), 5);
    let total: i64 = txns.iter().map(|t| t.amount_cents).sum();
    assert_eq!(total, 10_119 - 145_000 - 25_000 - 2_500 - 2_500);
    assert_eq!(txns[0].posted_date, "2026-10-07", "oldest first");
    assert_eq!(
        txns[2].merchant.as_deref(),
        Some("INTERNET BILL PAY SAMPLE CARD"),
        "row without leading space"
    );
}

#[test]
fn visa_export_parses_with_the_same_format() {
    let file = read_raw_file(&fixture("visa.csv")).unwrap();
    assert!(file.trailing_newline);

    // Task 4.1: the Visa statement is assumed to use the chequing format,
    // so one parser detects it and lists both account kinds.
    let parser = financi_parsers::detect(&file.header).expect("detected");
    assert_eq!(parser.id(), "simplii");
    assert_eq!(
        parser.account_kinds(),
        [
            financi_core::AccountKind::Chequing,
            financi_core::AccountKind::CreditCard
        ]
    );

    let txns: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.map(r).unwrap())
        .collect();
    assert_eq!(txns.len(), 6);
    // Funds Out is money out, Funds In money in — same signs as chequing.
    let amounts: Vec<_> = txns.iter().map(|t| t.amount_cents).collect();
    assert_eq!(
        amounts,
        [-575, -8_641, -1_200, 25_000, 4_875, -10_000],
        "charges negative, payments and rebates positive"
    );
    assert_eq!(txns[0].posted_date, "2026-10-02", "oldest first");

    let payments: Vec<_> = txns
        .iter()
        .enumerate()
        .filter(|(_, t)| parser.is_card_payment(t))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        payments,
        [3],
        "only the Funds In row containing PAYMENT; the money-out row with \
         PAYMENT in its name and the marker-free rebate stay spending"
    );
}

#[test]
fn identical_rows_share_a_hash() {
    let file = read_raw_file(&fixture("basic.csv")).unwrap();
    let parser = financi_parsers::by_id("simplii").unwrap();
    let key = AccountKey("00000000-0000-4000-8000-0000000000bb".into());
    let h: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.row_hash(&key, r))
        .collect();
    assert_eq!(h[3], h[4], "two identical e-transfers on one day");
}

#[test]
fn old_parser_id_still_resolves() {
    // `imports.parser_id` rows written before task 4.1 say
    // `simplii-chequing`; they must keep working.
    let legacy = financi_parsers::by_id("simplii-chequing").expect("legacy id resolves");
    assert_eq!(legacy.id(), "simplii");
    assert_eq!(legacy.institution(), "simplii");

    // Same behaviour: same hash for the same raw row, same mapping.
    let current = financi_parsers::by_id("simplii").unwrap();
    let file = read_raw_file(&fixture("basic.csv")).unwrap();
    let key = AccountKey("00000000-0000-4000-8000-0000000000bb".into());
    for record in &file.records {
        assert_eq!(
            legacy.row_hash(&key, record),
            current.row_hash(&key, record)
        );
        assert_eq!(
            format!("{:?}", legacy.map(record).unwrap()),
            format!("{:?}", current.map(record).unwrap())
        );
    }
    assert!(financi_parsers::by_id("nope").is_none());
}

#[test]
fn row_with_both_amounts_is_rejected() {
    let file = read_raw_file(&fixture("both_amounts.csv")).unwrap();
    let parser = financi_parsers::detect(&file.header).unwrap();
    assert!(parser.map(&file.records[0]).is_err());
}

#[test]
fn each_format_is_detected_as_its_own_parser() {
    let ids: Vec<_> = financi_parsers::all().iter().map(|p| p.id()).collect();
    assert_eq!(ids, ["amex", "simplii"]);
    let amex =
        read_raw_file(b"Date,Date Processed,Description,Amount\n05 Nov 2026,05 Nov 2026,X,1.00")
            .unwrap();
    assert_eq!(financi_parsers::detect(&amex.header).unwrap().id(), "amex");
}
