//! Parses whole synthetic Amex exports end to end: read raw file, detect,
//! map every row, hash.

use financi_core::{AccountKey, read_raw_file};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/amex/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn basic_export_parses_completely() {
    let file = read_raw_file(&fixture("basic.csv")).unwrap();
    assert!(!file.trailing_newline, "Amex exports end without a newline");

    let parser = financi_parsers::detect(&file.header).expect("detected as amex");
    assert_eq!(parser.id(), "amex");

    let txns: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.map(r).unwrap())
        .collect();
    assert_eq!(txns.len(), 5);

    let total: i64 = txns.iter().map(|t| t.amount_cents).sum();
    // charges 5.75 + 5.75 + 42.10 + 84.20 = 137.80 out; payment 1500.00 in
    assert_eq!(total, 150_000 - 13_780);
    assert_eq!(txns[2].merchant.as_deref(), Some("SHOP, INC"));
}

#[test]
fn identical_charges_share_a_hash() {
    let file = read_raw_file(&fixture("basic.csv")).unwrap();
    let parser = financi_parsers::by_id("amex").unwrap();
    let key = AccountKey("00000000-0000-4000-8000-0000000000aa".into());
    let hashes: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.row_hash(&key, r))
        .collect();
    assert_eq!(
        hashes[0], hashes[1],
        "two identical coffees, different processed dates"
    );
    let unique: std::collections::HashSet<_> = hashes.iter().collect();
    assert_eq!(unique.len(), 4);
}

#[test]
fn bad_row_is_reported_by_line() {
    let file = read_raw_file(&fixture("bad_date.csv")).unwrap();
    let parser = financi_parsers::detect(&file.header).unwrap();
    let errors: Vec<_> = file
        .records
        .iter()
        .filter_map(|r| parser.map(r).err())
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].to_string().starts_with("record 1:"),
        "{}",
        errors[0]
    );
}

#[test]
fn unknown_header_is_not_detected() {
    let file = read_raw_file(b"Posted,Payee,Debit,Credit\n2026-11-05,X,1.00,").unwrap();
    assert!(financi_parsers::detect(&file.header).is_none());
}
