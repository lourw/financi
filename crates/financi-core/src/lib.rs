//! Shared model for financi.
//!
//! Core reads a CSV export once into a [`RawFile`], keeping every record's
//! exact bytes so they survive for hashing and byte-exact export. Each
//! institution's [`StatementParser`] then works one [`RawRecord`] at a time.

use sha2::{Digest, Sha256};

pub mod dates;
pub mod parse;
mod raw;
pub mod rules;
pub use raw::{ReadError, raw_record_from_bytes, read_raw_file, sniff_header};

/// Money in integer cents. Negative = money out.
pub type Cents = i64;

/// Stable per-account identifier (a UUID string). Used in row hashes and the
/// export manifest instead of the database row id, so a restored or fresh
/// database produces the same hashes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountKey(pub String);

/// Hex SHA-256 identifying a transaction within an account. Not unique: two
/// identical charges on one day hash the same (see count reconciliation).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowHash(pub String);

/// Hash an account key plus a source's identity fields, exactly as exported.
///
/// Each part is length-prefixed (u64 little-endian) so a separator character
/// inside a field can never make two different rows collide.
///
/// The parts must be the field's raw exported bytes — a [`RawRecord`]'s
/// `raw_fields`, not the decoded `fields` — so a file's encoding guess can
/// never change a row's identity (3.71).
pub fn row_hash(account_key: &AccountKey, identity_fields: &[&[u8]]) -> RowHash {
    let mut hasher = Sha256::new();
    let mut part = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    };
    part(account_key.0.as_bytes());
    for field in identity_fields {
        part(field);
    }
    let digest = hasher.finalize();
    RowHash(digest.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Windows1252,
}

impl Encoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf-8",
            Encoding::Windows1252 => "windows-1252",
        }
    }

    /// Inverse of [`Self::as_str`], for the value stored in `imports.encoding`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "utf-8" => Some(Encoding::Utf8),
            "windows-1252" => Some(Encoding::Windows1252),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "lf",
            LineEnding::CrLf => "crlf",
        }
    }
}

/// A whole CSV export as read from disk, split by core (not by the parser).
#[derive(Debug, Clone)]
pub struct RawFile {
    pub header_raw: Vec<u8>,
    pub header: Vec<String>,
    pub encoding: Encoding,
    pub has_bom: bool,
    pub line_ending: LineEnding,
    pub trailing_newline: bool,
    pub records: Vec<RawRecord>,
}

/// One CSV record: its exact bytes (quotes included) plus decoded fields.
#[derive(Debug, Clone)]
pub struct RawRecord {
    pub bytes: Vec<u8>,
    /// Record index within its file, starting at 0 for the first data row.
    /// Stored in the database and used to order rows.
    pub source_line: usize,
    /// The 1-based physical line in the source file where the record starts
    /// (3.67). Unlike `source_line` this counts blank lines and multi-line
    /// quoted fields, so error messages can point the user at their file.
    pub file_line: usize,
    /// Fields exactly as exported, decoded to text but not trimmed or parsed.
    /// Decoding depends on the file's encoding guess, so this is display
    /// text only — never hash it (3.71).
    pub fields: Vec<String>,
    /// The same fields undecoded, quotes removed (3.71). The row hash is
    /// computed over these bytes, so the same row hashes the same whatever
    /// the file's encoding was or how it was guessed.
    pub raw_fields: Vec<Vec<u8>>,
}

/// The type of an account within its institution. Stored as
/// `accounts.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountKind {
    Chequing,
    Savings,
    CreditCard,
}

impl AccountKind {
    pub const ALL: [AccountKind; 3] = [
        AccountKind::Chequing,
        AccountKind::Savings,
        AccountKind::CreditCard,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::Chequing => "chequing",
            AccountKind::Savings => "savings",
            AccountKind::CreditCard => "credit_card",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            AccountKind::Chequing => "Chequing",
            AccountKind::Savings => "Savings",
            AccountKind::CreditCard => "Credit card",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryKind {
    Expense,
    Income,
    Transfer,
}

impl CategoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CategoryKind::Expense => "expense",
            CategoryKind::Income => "income",
            CategoryKind::Transfer => "transfer",
        }
    }
}

/// What a parser produces from one record, before categorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTransaction {
    /// ISO `yyyy-mm-dd`.
    pub posted_date: String,
    pub processed_date: Option<String>,
    /// financi's sign convention: negative = money out.
    pub amount_cents: Cents,
    pub original_currency: Option<String>,
    pub original_amount_cents: Option<Cents>,
    pub description_raw: String,
    pub merchant: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("line {line}: expected {expected} fields, found {found}")]
    FieldCount {
        /// The record's 1-based `file_line`, so users can find it.
        line: usize,
        expected: usize,
        found: usize,
    },
    #[error("line {line}: bad {field} {value:?}")]
    BadValue {
        /// The record's 1-based `file_line`, so users can find it.
        line: usize,
        field: &'static str,
        value: String,
    },
}

/// One implementation per institution export format.
pub trait StatementParser {
    /// Stable id, e.g. `"amex"`, `"simplii"`.
    fn id(&self) -> &'static str;
    /// The institution an account must belong to for this format, matching
    /// `institutions.key`, e.g. `"amex"`, `"simplii"`.
    fn institution(&self) -> &'static str;
    /// The account types this export format comes from. Simplii chequing and
    /// Simplii Visa export the same format, so one parser lists both kinds
    /// and the account chosen at import decides which.
    fn account_kinds(&self) -> &'static [AccountKind];
    /// True if this header row looks like ours (auto-detect).
    fn detect(&self, header: &[String]) -> bool;
    /// Map one record into the shared model (sign flip, date parsing, ...).
    fn map(&self, record: &RawRecord) -> Result<RawTransaction, ParseError>;
    /// This institution's own dedup hash over its identity columns,
    /// normally a call to [`row_hash`] with the right fields.
    fn row_hash(&self, account_key: &AccountKey, record: &RawRecord) -> RowHash;
    /// Provider order in the export: `true` when the newest rows come first
    /// (Amex), `false` when the oldest come first (Simplii).
    fn newest_first(&self) -> bool {
        false
    }
    /// Built-in transfer detection, checked before user rules at import:
    /// `true` when this row moves the owner's own money instead of spending
    /// it (Amex's `PAYMENT RECEIVED`). Matching rows become
    /// `Transfers > Card Payment` and are locked against rules.
    /// `kind` is the account the row is importing into: a payment marker
    /// can mean different things per account kind, e.g. Simplii only treats
    /// `PAYMENT RECEIVED` rows as card payments on the Visa account, never
    /// on chequing (3.66). Parsers whose rule can't vary by kind ignore it.
    fn is_card_payment(&self, _txn: &RawTransaction, _kind: AccountKind) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> AccountKey {
        AccountKey("6f1c2a5e-0000-4000-8000-000000000001".into())
    }

    #[test]
    fn account_kind_round_trips() {
        for k in AccountKind::ALL {
            assert_eq!(AccountKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(AccountKind::parse("credit"), None);
    }

    #[test]
    fn encoding_parses_its_own_stored_name() {
        for e in [Encoding::Utf8, Encoding::Windows1252] {
            assert_eq!(Encoding::parse(e.as_str()), Some(e));
        }
        assert_eq!(Encoding::parse("latin-1"), None);
    }

    #[test]
    fn same_fields_same_hash() {
        let a = row_hash(&key(), &[b"05 Nov 2026", b"COFFEE", b"4.50"]);
        let b = row_hash(&key(), &[b"05 Nov 2026", b"COFFEE", b"4.50"]);
        assert_eq!(a, b);
        assert_eq!(a.0.len(), 64);
    }

    #[test]
    fn length_prefix_prevents_separator_collisions() {
        let a = row_hash(&key(), &[b"A|B", b"C"]);
        let b = row_hash(&key(), &[b"A", b"B|C"]);
        assert_ne!(a, b);
    }

    #[test]
    fn account_key_scopes_the_hash() {
        let other = AccountKey("6f1c2a5e-0000-4000-8000-000000000002".into());
        let fields: [&[u8]; 3] = [b"05 Nov 2026", b"COFFEE", b"4.50"];
        assert_ne!(row_hash(&key(), &fields), row_hash(&other, &fields));
    }

    #[test]
    fn raw_strings_are_not_normalized() {
        let a = row_hash(&key(), &[b"COFFEE  VANCOUVER"]);
        let b = row_hash(&key(), &[b"COFFEE VANCOUVER"]);
        assert_ne!(a, b, "whitespace is part of the raw value");
    }
}
