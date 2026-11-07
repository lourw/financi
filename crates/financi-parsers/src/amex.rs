//! American Express (Canada) activity export, `activity.csv`.
//!
//! `Date,Date Processed,Description,Amount`, newest first. Charges are
//! positive and payments negative, the reverse of financi's convention.
//! Identity fields for the row hash: Date, Description, Amount, exactly as
//! exported. Date Processed is left out because it can change once a charge
//! settles.

use financi_core::{
    AccountKey, AccountKind, ParseError, RawRecord, RawTransaction, RowHash, StatementParser,
    parse, row_hash,
};

const HEADER: [&str; 4] = ["Date", "Date Processed", "Description", "Amount"];
const DATE: usize = 0;
const PROCESSED: usize = 1;
const DESCRIPTION: usize = 2;
const AMOUNT: usize = 3;

pub struct Amex;

impl StatementParser for Amex {
    fn id(&self) -> &'static str {
        "amex"
    }

    fn institution(&self) -> &'static str {
        "amex"
    }

    fn account_kinds(&self) -> &'static [AccountKind] {
        &[AccountKind::CreditCard]
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

        let posted_date = parse::date_dd_mon_yyyy(&f[DATE]).ok_or_else(|| bad("Date", &f[DATE]))?;
        let processed_date = if f[PROCESSED].trim().is_empty() {
            None
        } else {
            Some(
                parse::date_dd_mon_yyyy(&f[PROCESSED])
                    .ok_or_else(|| bad("Date Processed", &f[PROCESSED]))?,
            )
        };
        let charge = parse::cents(&f[AMOUNT]).ok_or_else(|| bad("Amount", &f[AMOUNT]))?;

        Ok(RawTransaction {
            posted_date,
            processed_date,
            amount_cents: -charge,
            original_currency: None,
            original_amount_cents: None,
            description_raw: f[DESCRIPTION].clone(),
            merchant: Some(parse::merchant(&f[DESCRIPTION])),
        })
    }

    fn row_hash(&self, account_key: &AccountKey, record: &RawRecord) -> RowHash {
        let field = |ix: usize| record.fields.get(ix).map_or(&b""[..], |s| s.as_bytes());
        row_hash(
            account_key,
            &[field(DATE), field(DESCRIPTION), field(AMOUNT)],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use financi_core::read_raw_file;

    fn record(line: &str) -> RawRecord {
        let file = format!("{}\n{line}", HEADER.join(","));
        read_raw_file(file.as_bytes()).unwrap().records.remove(0)
    }

    fn key() -> AccountKey {
        AccountKey("00000000-0000-4000-8000-0000000000aa".into())
    }

    #[test]
    fn detects_header_and_rejects_others() {
        let h = |s: &str| s.split(',').map(String::from).collect::<Vec<_>>();
        assert!(Amex.detect(&h("Date,Date Processed,Description,Amount")));
        assert!(
            Amex.detect(&h("Date, Date Processed ,Description,Amount")),
            "trims names"
        );
        assert!(!Amex.detect(&h("Date, Transaction Details, Funds Out, Funds In ")));
        assert!(!Amex.detect(&h("Date,Description,Amount")));
    }

    #[test]
    fn maps_a_charge_with_sign_flip_and_merchant() {
        let t = Amex
            .map(&record(
                "05 Nov 2026,06 Nov 2026,COFFEE          VANCOUVER,15.74",
            ))
            .unwrap();
        assert_eq!(t.posted_date, "2026-11-05");
        assert_eq!(t.processed_date.as_deref(), Some("2026-11-06"));
        assert_eq!(t.amount_cents, -1574, "a charge is money out");
        assert_eq!(t.description_raw, "COFFEE          VANCOUVER");
        assert_eq!(t.merchant.as_deref(), Some("COFFEE"));
    }

    #[test]
    fn payment_becomes_money_in() {
        let t = Amex
            .map(&record(
                "04 Nov 2026,04 Nov 2026,PAYMENT RECEIVED - THANK YOU,-2042.40",
            ))
            .unwrap();
        assert_eq!(t.amount_cents, 204_240);
    }

    #[test]
    fn quoted_description_with_comma() {
        let t = Amex
            .map(&record(
                "05 Nov 2026,06 Nov 2026,\"SHOP, INC   TORONTO\",42.10",
            ))
            .unwrap();
        assert_eq!(t.description_raw, "SHOP, INC   TORONTO");
        assert_eq!(t.merchant.as_deref(), Some("SHOP, INC"));
    }

    #[test]
    fn reports_bad_values_with_line() {
        let err = Amex
            .map(&record("31 Feb 2026,01 Mar 2026,X,1.00"))
            .unwrap_err();
        assert!(
            matches!(
                err,
                ParseError::BadValue {
                    line: 0,
                    field: "Date",
                    ..
                }
            ),
            "{err}"
        );
        let err = Amex
            .map(&record("05 Nov 2026,06 Nov 2026,X,abc"))
            .unwrap_err();
        assert!(
            matches!(
                err,
                ParseError::BadValue {
                    field: "Amount",
                    ..
                }
            ),
            "{err}"
        );
        let err = Amex.map(&record("05 Nov 2026,X,1.00")).unwrap_err();
        assert!(
            matches!(
                err,
                ParseError::FieldCount {
                    expected: 4,
                    found: 3,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn hash_ignores_date_processed_but_not_identity_fields() {
        let a = Amex.row_hash(&key(), &record("05 Nov 2026,06 Nov 2026,COFFEE,4.50"));
        let b = Amex.row_hash(&key(), &record("05 Nov 2026,09 Nov 2026,COFFEE,4.50"));
        let c = Amex.row_hash(&key(), &record("05 Nov 2026,06 Nov 2026,COFFEE,4.51"));
        assert_eq!(a, b, "Date Processed is not an identity field");
        assert_ne!(a, c);
    }
}
