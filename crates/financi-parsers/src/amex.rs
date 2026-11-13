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

/// Card payments read `PAYMENT RECEIVED - THANK YOU` in the export.
const CARD_PAYMENT_PREFIX: &str = "PAYMENT RECEIVED";

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
        let line = record.file_line;
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
        // 3.71: hash the undecoded bytes, so the file's encoding guess
        // cannot move a row's identity.
        let field = |ix: usize| record.raw_fields.get(ix).map_or(&b""[..], Vec::as_slice);
        row_hash(
            account_key,
            &[field(DATE), field(DESCRIPTION), field(AMOUNT)],
        )
    }

    fn newest_first(&self) -> bool {
        true
    }

    fn is_card_payment(&self, txn: &RawTransaction, _kind: AccountKind) -> bool {
        // Amex only exports credit-card activity, so `kind` is always
        // CreditCard and can be ignored. The marker appears in the export
        // in upper case; fold like the rule engine does, and match only as
        // a prefix of the description.
        txn.description_raw
            .get(..CARD_PAYMENT_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(CARD_PAYMENT_PREFIX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use financi_core::{Encoding, read_raw_file};

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

    fn mapped(description: &str) -> RawTransaction {
        Amex.map(&record(&format!(
            "04 Nov 2026,04 Nov 2026,{description},-2042.40"
        )))
        .unwrap()
    }

    #[test]
    fn card_payment_is_a_case_insensitive_prefix_match() {
        assert!(Amex.is_card_payment(
            &mapped("PAYMENT RECEIVED - THANK YOU"),
            AccountKind::CreditCard
        ));
        assert!(
            Amex.is_card_payment(
                &mapped("payment received - thank you"),
                AccountKind::CreditCard
            ),
            "case-insensitive like the rule engine"
        );
        assert!(!Amex.is_card_payment(&mapped("COFFEE VANCOUVER"), AccountKind::CreditCard));
        assert!(
            !Amex.is_card_payment(&mapped("CREDIT PAYMENT RECEIVED"), AccountKind::CreditCard),
            "only a prefix counts"
        );
        assert!(!Amex.is_card_payment(&mapped("PAYMENT"), AccountKind::CreditCard));
    }

    #[test]
    fn payment_marker_is_anchored_to_the_start() {
        // The Simplii parser matches PAYMENT anywhere; Amex only as a
        // prefix, so look-alikes stay ordinary spending.
        assert!(!Amex.is_card_payment(
            &mapped("THANK YOU - CARD PAYMENT 1234"),
            AccountKind::CreditCard
        ));
    }

    #[test]
    fn amex_ignores_the_account_kind() {
        // Amex only exports credit-card activity, so the rule has no
        // per-kind branch (3.66): the marker decides on its own.
        assert!(Amex.is_card_payment(
            &mapped("PAYMENT RECEIVED - THANK YOU"),
            AccountKind::Chequing
        ));
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
                    line: 2,
                    field: "Date",
                    ..
                }
            ),
            "{err}"
        );
        assert!(err.to_string().starts_with("line 2: bad Date"), "{err}");
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

    #[test]
    fn ascii_row_hash_is_pinned() {
        // 3.71: an all-ASCII row's hash is unchanged by byte hashing. This
        // hex digest pins the scheme (key, `|`-free identity fields,
        // length-prefixed SHA-256) so any change trips here first.
        let rec = record("05 Nov 2026,06 Nov 2026,COFFEE,4.50");
        assert_eq!(
            Amex.row_hash(&key(), &rec).0,
            "ab548ab4d7b0a777d4787259740a57ebcb6a550352594216e4d6c6afb6dcd5f0"
        );
    }

    #[test]
    fn hash_survives_the_encoding_guess() {
        // 3.71: the same row, byte for byte, in a clean UTF-8 file and in
        // a file a stray Windows-1252 byte pushes to that encoding, must
        // hash the same even though the two files decode it differently.
        let row = b"05 Nov 2026,06 Nov 2026,CAF\xC3\xA9 MARKET,4.50"; // é as UTF-8
        let header = b"Date,Date Processed,Description,Amount\n";
        let clean = read_raw_file(&[header.as_slice(), row, b"\n"].concat()).unwrap();
        let mixed = read_raw_file(
            &[
                header.as_slice(),
                row,
                b"\n07 Nov 2026,07 Nov 2026,STRAY\xC9,3.00\n",
            ]
            .concat(),
        )
        .unwrap();
        assert_eq!(clean.encoding, Encoding::Utf8);
        assert_eq!(
            mixed.encoding,
            Encoding::Windows1252,
            "the stray byte moves the file"
        );
        let a = Amex.row_hash(&key(), &clean.records[0]);
        let b = Amex.row_hash(&key(), &mixed.records[0]);
        assert_eq!(a, b, "same bytes, same hash");
        assert_ne!(
            clean.records[0].fields[2], mixed.records[0].fields[2],
            "the decoded text differs, which is exactly why it must not be hashed"
        );
    }

    #[test]
    fn rows_differing_only_in_invalid_bytes_hash_differently() {
        // 3.71: a BOM pins the file to UTF-8 with lossy decoding, so two
        // rows carrying different invalid bytes look identical once
        // decoded — hashing the bytes keeps them distinct rows.
        let mut file = vec![0xEF, 0xBB, 0xBF];
        file.extend(b"Date,Date Processed,Description,Amount\n");
        file.extend(b"05 Nov 2026,06 Nov 2026,CAF\x81,4.50\n");
        file.extend(b"05 Nov 2026,06 Nov 2026,CAF\x8D,4.50\n");
        let raw = read_raw_file(&file).unwrap();
        assert_eq!(raw.encoding, Encoding::Utf8, "the BOM decides the guess");
        assert_eq!(
            raw.records[0].fields[2], raw.records[1].fields[2],
            "both invalid bytes decode to the replacement character"
        );
        let a = Amex.row_hash(&key(), &raw.records[0]);
        let b = Amex.row_hash(&key(), &raw.records[1]);
        assert_ne!(a, b, "the bytes differ, so the rows differ");
    }
}
