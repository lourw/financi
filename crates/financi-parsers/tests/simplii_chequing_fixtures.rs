//! Parses whole synthetic Simplii chequing exports end to end.

use financi_core::{AccountKey, read_raw_file};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/simplii_chequing/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn basic_export_parses_completely() {
    let file = read_raw_file(&fixture("basic.csv")).unwrap();
    assert!(file.trailing_newline, "Simplii exports end with a newline");

    let parser = financi_parsers::detect(&file.header).expect("detected");
    assert_eq!(parser.id(), "simplii-chequing");
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
fn identical_rows_share_a_hash() {
    let file = read_raw_file(&fixture("basic.csv")).unwrap();
    let parser = financi_parsers::by_id("simplii-chequing").unwrap();
    let key = AccountKey("00000000-0000-4000-8000-0000000000bb".into());
    let h: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.row_hash(&key, r))
        .collect();
    assert_eq!(h[3], h[4], "two identical e-transfers on one day");
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
    assert_eq!(ids, ["amex", "simplii-chequing"]);
    let amex =
        read_raw_file(b"Date,Date Processed,Description,Amount\n05 Nov 2026,05 Nov 2026,X,1.00")
            .unwrap();
    assert_eq!(financi_parsers::detect(&amex.header).unwrap().id(), "amex");
}
