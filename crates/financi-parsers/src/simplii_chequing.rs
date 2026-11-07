//! Simplii Financial chequing export.
//!
//! `Date, Transaction Details, Funds Out, Funds In ` (note the spaces), month
//! first dates, oldest first. Funds Out and Funds In are unsigned and exactly
//! one is filled per row. There is no balance column, so all four fields are
//! identity fields, hashed exactly as exported (leading spaces included).

use financi_core::{
    AccountKey, AccountKind, ParseError, RawRecord, RawTransaction, RowHash, StatementParser,
    parse, row_hash,
};

const HEADER: [&str; 4] = ["Date", "Transaction Details", "Funds Out", "Funds In"];
const DATE: usize = 0;
const DETAILS: usize = 1;
const OUT: usize = 2;
const IN: usize = 3;

pub struct SimpliiChequing;

impl StatementParser for SimpliiChequing {
    fn id(&self) -> &'static str {
        "simplii-chequing"
    }

    fn institution(&self) -> &'static str {
        "simplii"
    }

    fn account_kinds(&self) -> &'static [AccountKind] {
        &[AccountKind::Chequing]
    }

    fn detect(&self, header: &[String]) -> bool {
        header.len() == HEADER.len() && header.iter().zip(HEADER).all(|(h, e)| h.trim() == e)
    }

    fn map(&self, record: &RawRecord) -> Result<RawTransaction, ParseError> {
        let line = record.source_line;
        let f = &record.fields;
        if f.len() != HEADER.len() {
            return Err(ParseError::FieldCount {
                line,
                expected: HEADER.len(),
                found: f.len(),
            });
        }
        let bad = |field: &'static str, value: &str| ParseError::BadValue {
            line,
            field,
            value: value.to_string(),
        };

        let posted_date = parse::date_mm_dd_yyyy(&f[DATE]).ok_or_else(|| bad("Date", &f[DATE]))?;
        let money = |ix: usize, name: &'static str| -> Result<Option<i64>, ParseError> {
            if f[ix].trim().is_empty() {
                Ok(None)
            } else {
                match parse::cents(&f[ix]) {
                    Some(c) if c >= 0 => Ok(Some(c)),
                    _ => Err(bad(name, &f[ix])),
                }
            }
        };
        let amount_cents = match (money(OUT, "Funds Out")?, money(IN, "Funds In")?) {
            (Some(out), None) => -out,
            (None, Some(inn)) => inn,
            _ => {
                return Err(bad(
                    "Funds Out/Funds In",
                    &format!("{:?} / {:?}", f[OUT], f[IN]),
                ));
            }
        };

        Ok(RawTransaction {
            posted_date,
            processed_date: None,
            amount_cents,
            original_currency: None,
            original_amount_cents: None,
            description_raw: f[DETAILS].clone(),
            merchant: Some(parse::merchant(&f[DETAILS])),
        })
    }

    fn row_hash(&self, account_key: &AccountKey, record: &RawRecord) -> RowHash {
        let field = |ix: usize| record.fields.get(ix).map_or(&b""[..], |s| s.as_bytes());
        row_hash(
            account_key,
            &[field(DATE), field(DETAILS), field(OUT), field(IN)],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use financi_core::read_raw_file;

    const RAW_HEADER: &str = "Date, Transaction Details, Funds Out, Funds In ";

    fn record(line: &str) -> RawRecord {
        read_raw_file(format!("{RAW_HEADER}\n{line}\n").as_bytes())
            .unwrap()
            .records
            .remove(0)
    }

    #[test]
    fn detects_spaced_header_only() {
        let h = |s: &str| s.split(',').map(String::from).collect::<Vec<_>>();
        assert!(SimpliiChequing.detect(&h(RAW_HEADER)));
        assert!(!SimpliiChequing.detect(&h("Date,Date Processed,Description,Amount")));
    }

    #[test]
    fn funds_in_is_positive_and_funds_out_negative() {
        let t = SimpliiChequing
            .map(&record("10/07/2026, E-TRANSFER RECEIVE,,101.19"))
            .unwrap();
        assert_eq!(t.posted_date, "2026-10-07");
        assert_eq!(t.amount_cents, 10_119);
        assert_eq!(
            t.description_raw, " E-TRANSFER RECEIVE",
            "raw keeps the leading space"
        );
        assert_eq!(t.merchant.as_deref(), Some("E-TRANSFER RECEIVE"));

        let t = SimpliiChequing
            .map(&record("10/09/2026, E-TRANSFER SEND,1450.00,"))
            .unwrap();
        assert_eq!(t.amount_cents, -145_000);
    }

    #[test]
    fn exactly_one_amount_column_must_be_filled() {
        for line in [
            "10/07/2026, X,5.00,5.00",
            "10/07/2026, X,,",
            "10/07/2026, X,-5.00,",
        ] {
            let err = SimpliiChequing.map(&record(line)).unwrap_err();
            assert!(matches!(err, ParseError::BadValue { .. }), "{line}: {err}");
        }
    }

    #[test]
    fn rejects_day_first_dates() {
        let err = SimpliiChequing
            .map(&record("13/07/2026, X,1.00,"))
            .unwrap_err();
        assert!(
            matches!(err, ParseError::BadValue { field: "Date", .. }),
            "{err}"
        );
    }

    #[test]
    fn hash_uses_all_four_raw_fields() {
        let key = AccountKey("00000000-0000-4000-8000-0000000000bb".into());
        let a = SimpliiChequing.row_hash(&key, &record("10/07/2026, X,,1.00"));
        let b = SimpliiChequing.row_hash(&key, &record("10/07/2026,X,,1.00"));
        let c = SimpliiChequing.row_hash(&key, &record("10/07/2026, X,1.00,"));
        assert_ne!(a, b, "leading space is part of the raw value");
        assert_ne!(a, c, "in vs out matters");
    }
}
