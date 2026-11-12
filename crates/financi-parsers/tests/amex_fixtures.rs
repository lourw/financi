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
        errors[0].to_string().starts_with("line 3:"),
        "{}",
        errors[0]
    );
}

#[test]
fn crlf_bom_fixture_parses() {
    let bytes = fixture("crlf_bom.csv");
    let file = read_raw_file(&bytes).unwrap();
    assert!(file.has_bom);
    assert_eq!(file.line_ending, financi_core::LineEnding::CrLf);
    assert!(!file.trailing_newline, "Amex exports end without a newline");
    assert_eq!(
        file.header,
        ["Date", "Date Processed", "Description", "Amount"]
    );

    let parser = financi_parsers::detect(&file.header).expect("detected as amex");
    let txns: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.map(r).unwrap())
        .collect();
    assert_eq!(txns.len(), 3);
    assert_eq!(txns[2].description_raw, "SHOP, INC   TORONTO");
    assert_eq!(txns[1].amount_cents, 150_000, "payment is money in");
    assert_eq!(
        file.records.iter().map(|r| r.file_line).collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
}

#[test]
fn stray_quote_row_does_not_swallow_the_file() {
    // `BEST BUY 55" TV` used to open a quote and eat the rest of the file.
    let file = read_raw_file(&fixture("stray_quotes.csv")).unwrap();
    let parser = financi_parsers::detect(&file.header).expect("detected as amex");
    assert_eq!(file.records.len(), 3, "the blank line is skipped");
    assert_eq!(file.records[0].fields[2], "BEST BUY 55\" TV    TORONTO");

    let txns: Vec<_> = file
        .records
        .iter()
        .map(|r| parser.map(r).unwrap())
        .collect();
    assert_eq!(txns[0].merchant.as_deref(), Some("BEST BUY 55\" TV"));
    assert_eq!(txns[1].amount_cents, -123_456, "quoted \"1,234.56\"");

    // file_line counts the blank line; source_line does not.
    assert_eq!(
        file.records
            .iter()
            .map(|r| (r.source_line, r.file_line))
            .collect::<Vec<_>>(),
        vec![(0, 2), (1, 4), (2, 5)]
    );
}

#[test]
fn same_rows_hash_identically_across_line_ending_and_bom_variants() {
    let base = "Date,Date Processed,Description,Amount\n\
                06 Nov 2026,06 Nov 2026,BEANERY COFFEE          VANCOUVER,5.75\n\
                05 Nov 2026,05 Nov 2026,\"SHOP, INC   TORONTO\",42.10\n";
    let bom: &[u8] = b"\xEF\xBB\xBF";
    let variants: Vec<Vec<u8>> = [
        base.as_bytes().to_vec(),
        base.replace("\n", "\r\n").into_bytes(),
        [bom, base.as_bytes()].concat(),
        [bom, base.replace("\n", "\r\n").as_bytes()].concat(),
    ]
    .into();
    let parser = financi_parsers::by_id("amex").unwrap();
    let key = AccountKey("00000000-0000-4000-8000-0000000000aa".into());
    let files: Vec<_> = variants
        .iter()
        .map(|v| read_raw_file(v).expect("variant parses"))
        .collect();
    for file in &files[1..] {
        assert_eq!(file.records.len(), files[0].records.len());
        for (a, b) in file.records.iter().zip(&files[0].records) {
            assert_eq!(a.bytes, b.bytes, "record bytes ignore the terminator");
            assert_eq!(a.fields, b.fields);
            assert_eq!(
                parser.row_hash(&key, a),
                parser.row_hash(&key, b),
                "line endings and BOM are not part of the row"
            );
        }
    }
}

#[test]
fn unknown_header_is_not_detected() {
    let file = read_raw_file(b"Posted,Payee,Debit,Credit\n2026-11-05,X,1.00,").unwrap();
    assert!(financi_parsers::detect(&file.header).is_none());
}
