//! Smoke test against real exports on this machine. Ignored by default and
//! never committed with data: point env vars at local files, e.g.
//!
//! FINANCI_SAMPLE_AMEX=~/Downloads/activity.csv \
//! FINANCI_SAMPLE_SIMPLII_CHEQUING=~/Downloads/SIMPLII.csv \
//!   cargo test -p financi-parsers --test real_samples -- --ignored --nocapture
//!
//! Prints only counts, never transaction contents.

use financi_core::{AccountKind, read_raw_file};

fn check(env: &str, expected_parser: &str, kind: AccountKind) {
    let Some(path) = std::env::var_os(env) else {
        eprintln!("{env} not set, skipping");
        return;
    };
    let file = read_raw_file(&std::fs::read(&path).unwrap()).unwrap();
    let parser = financi_parsers::detect(&file.header).expect("format detected");
    assert_eq!(parser.id(), expected_parser);
    let failures: Vec<_> = file
        .records
        .iter()
        .filter_map(|r| parser.map(r).err())
        .collect();
    for f in &failures {
        // Line numbers and field names only; the value may be personal.
        match f {
            financi_core::ParseError::BadValue { line, field, .. } => {
                eprintln!("  record {line}: bad {field}")
            }
            other => eprintln!("  {other}"),
        }
    }
    // How many rows the built-in check treats as card payments on an
    // account of `kind` (3.66) — a count only, never contents.
    let card_payments = file
        .records
        .iter()
        .filter_map(|r| parser.map(r).ok())
        .filter(|t| parser.is_card_payment(t, kind))
        .count();
    eprintln!(
        "{env}: {} rows, {} failed, parser {}, {} card payments",
        file.records.len(),
        failures.len(),
        parser.id(),
        card_payments
    );
    assert!(failures.is_empty());
}

#[test]
#[ignore = "needs local real exports"]
fn real_amex_export_parses() {
    check("FINANCI_SAMPLE_AMEX", "amex", AccountKind::CreditCard);
}

#[test]
#[ignore = "needs local real exports"]
fn real_simplii_chequing_export_parses() {
    check(
        "FINANCI_SAMPLE_SIMPLII_CHEQUING",
        "simplii",
        AccountKind::Chequing,
    );
}
