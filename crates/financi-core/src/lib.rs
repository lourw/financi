//! Shared model for financi.
//!
//! Core reads a CSV export once into a [`RawFile`], keeping every record's
//! exact bytes so they survive for hashing and byte-exact export. Each
//! institution's [`StatementParser`] then works one [`RawRecord`] at a time.

use sha2::{Digest, Sha256};

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
    pub source_line: usize,
    /// Fields exactly as exported, decoded to text but not trimmed or parsed.
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryKind {
    Spend,
    Income,
    Transfer,
}

impl CategoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CategoryKind::Spend => "spend",
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
    #[error("record {line}: expected {expected} fields, found {found}")]
    FieldCount {
        line: usize,
        expected: usize,
        found: usize,
    },
    #[error("record {line}: bad {field} {value:?}")]
    BadValue {
        line: usize,
        field: &'static str,
        value: String,
    },
}

/// One implementation per institution export format.
pub trait StatementParser {
    /// Stable id, e.g. `"amex"`, `"simplii-chequing"`.
    fn id(&self) -> &'static str;
    /// True if this header row looks like ours (auto-detect).
    fn detect(&self, header: &[String]) -> bool;
    /// Map one record into the shared model (sign flip, date parsing, ...).
    fn map(&self, record: &RawRecord) -> Result<RawTransaction, ParseError>;
    /// This institution's own dedup hash over its identity columns,
    /// normally a call to [`row_hash`] with the right fields.
    fn row_hash(&self, account_key: &AccountKey, record: &RawRecord) -> RowHash;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> AccountKey {
        AccountKey("6f1c2a5e-0000-4000-8000-000000000001".into())
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
