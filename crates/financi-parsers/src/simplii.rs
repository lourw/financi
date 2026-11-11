//! Simplii Financial export: chequing and Visa credit card.
//!
//! `Date, Transaction Details, Funds Out, Funds In ` (note the spaces), month
//! first dates, oldest first. Funds Out and Funds In are unsigned and exactly
//! one is filled per row. There is no balance column, so all four fields are
//! identity fields, hashed exactly as exported (leading spaces included).
//!
//! The Visa statement is *assumed* to use the same format (task 4.1; no real
//! sample exists yet), so one parser covers both account kinds and the
//! account chosen at import decides which. Funds Out is money out and Funds
//! In is money in for both. Revisit when a real Visa export arrives.

use financi_core::{
    AccountKey, AccountKind, ParseError, RawRecord, RawTransaction, RowHash, StatementParser,
    parse, row_hash,
};

const HEADER: [&str; 4] = ["Date", "Transaction Details", "Funds Out", "Funds In"];
const DATE: usize = 0;
const DETAILS: usize = 1;
const OUT: usize = 2;
const IN: usize = 3;

/// A Visa payment reads `... PAYMENT ...` in the description and arrives as
/// Funds In (money towards the card); the marker matches anywhere in the
/// description, case-insensitively.
const CARD_PAYMENT_MARKER: &[u8] = b"PAYMENT";

pub struct Simplii;

impl StatementParser for Simplii {
    fn id(&self) -> &'static str {
        "simplii"
    }

    fn institution(&self) -> &'static str {
        "simplii"
    }

    fn account_kinds(&self) -> &'static [AccountKind] {
        &[AccountKind::Chequing, AccountKind::CreditCard]
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

    fn newest_first(&self) -> bool {
        false
    }

    fn is_card_payment(&self, txn: &RawTransaction) -> bool {
        // A Funds In row (money in, from the account's own perspective)
        // whose description contains the marker. Folding case matches how
        // the rule engine treats descriptions; unlike Amex the marker is
        // not anchored to the start.
        txn.amount_cents > 0
            && txn
                .description_raw
                .as_bytes()
                .windows(CARD_PAYMENT_MARKER.len())
                .any(|w| w.eq_ignore_ascii_case(CARD_PAYMENT_MARKER))
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

    fn mapped(line: &str) -> RawTransaction {
        Simplii.map(&record(line)).unwrap()
    }

    #[test]
    fn detects_spaced_header_only() {
        let h = |s: &str| s.split(',').map(String::from).collect::<Vec<_>>();
        assert!(Simplii.detect(&h(RAW_HEADER)));
        assert!(!Simplii.detect(&h("Date,Date Processed,Description,Amount")));
    }

    #[test]
    fn one_parser_covers_both_account_kinds() {
        assert_eq!(
            Simplii.account_kinds(),
            [AccountKind::Chequing, AccountKind::CreditCard],
            "the account picked at import decides which; chequing first \
             so it stays the new-account default"
        );
    }

    #[test]
    fn funds_in_is_positive_and_funds_out_negative() {
        let t = Simplii
            .map(&record("10/07/2026, E-TRANSFER RECEIVE,,101.19"))
            .unwrap();
        assert_eq!(t.posted_date, "2026-10-07");
        assert_eq!(t.amount_cents, 10_119);
        assert_eq!(
            t.description_raw, " E-TRANSFER RECEIVE",
            "raw keeps the leading space"
        );
        assert_eq!(t.merchant.as_deref(), Some("E-TRANSFER RECEIVE"));

        let t = Simplii
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
            let err = Simplii.map(&record(line)).unwrap_err();
            assert!(matches!(err, ParseError::BadValue { .. }), "{line}: {err}");
        }
    }

    #[test]
    fn rejects_day_first_dates() {
        let err = Simplii.map(&record("13/07/2026, X,1.00,")).unwrap_err();
        assert!(
            matches!(err, ParseError::BadValue { field: "Date", .. }),
            "{err}"
        );
    }

    #[test]
    fn hash_uses_all_four_raw_fields() {
        let key = AccountKey("00000000-0000-4000-8000-0000000000bb".into());
        let a = Simplii.row_hash(&key, &record("10/07/2026, X,,1.00"));
        let b = Simplii.row_hash(&key, &record("10/07/2026,X,,1.00"));
        let c = Simplii.row_hash(&key, &record("10/07/2026, X,1.00,"));
        assert_ne!(a, b, "leading space is part of the raw value");
        assert_ne!(a, c, "in vs out matters");
    }

    #[test]
    fn card_payment_is_funds_in_containing_payment() {
        assert!(Simplii.is_card_payment(&mapped("10/12/2026, PAYMENT RECEIVED THANK YOU,,250.00")));
        assert!(
            Simplii.is_card_payment(&mapped("10/12/2026, online payment received,,50.00")),
            "case-insensitive like the rule engine"
        );
        assert!(
            Simplii.is_card_payment(&mapped("10/12/2026, CREDIT CARD PAYMENT 1234,,50.00")),
            "unlike Amex, the marker counts anywhere in the description"
        );
    }

    #[test]
    fn other_rows_are_not_card_payments() {
        assert!(
            !Simplii.is_card_payment(&mapped("10/07/2026, BILL PAYMENT SAMPLE CARD,25.00,")),
            "money out is the transfer's other side, not a payment into the card"
        );
        assert!(!Simplii.is_card_payment(&mapped("10/07/2026, E-TRANSFER RECEIVE,,40.00")));
    }
}
