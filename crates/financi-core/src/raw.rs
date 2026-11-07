//! Reads a CSV export into a [`RawFile`] while keeping every record's exact
//! bytes, so rows can be hashed as exported and written back byte for byte.
//!
//! - Encoding: a UTF-8 BOM is detected and stripped from the header; valid
//!   UTF-8 is decoded as UTF-8; anything else is decoded as Windows-1252.
//! - Records end at an LF or CRLF outside quotes, so a quoted field may span
//!   lines. Record bytes exclude the terminator.
//! - Fields follow RFC 4180 quoting (`""` inside quotes is a literal quote)
//!   and are otherwise left untouched: no trimming.
//! - Blank lines carry no data and are skipped.

use crate::{Encoding, LineEnding, RawFile, RawRecord};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReadError {
    #[error("the file is empty")]
    Empty,
    #[error("the file has a header row but no transactions")]
    NoRecords,
    #[error("record {line} has an unclosed quote")]
    UnclosedQuote { line: usize },
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

pub fn read_raw_file(bytes: &[u8]) -> Result<RawFile, ReadError> {
    let has_bom = bytes.starts_with(UTF8_BOM);
    let body = if has_bom {
        &bytes[UTF8_BOM.len()..]
    } else {
        bytes
    };
    if body.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(ReadError::Empty);
    }

    let encoding = if has_bom || std::str::from_utf8(body).is_ok() {
        Encoding::Utf8
    } else {
        Encoding::Windows1252
    };
    let line_ending = if body.windows(2).any(|w| w == b"\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    let trailing_newline = body.ends_with(b"\n");

    let mut slices = split_records(body)?.into_iter();
    let header_raw = slices.next().ok_or(ReadError::Empty)?;
    let header = split_fields(header_raw, encoding);

    let records: Vec<RawRecord> = slices
        .enumerate()
        .map(|(source_line, bytes)| RawRecord {
            fields: split_fields(bytes, encoding),
            bytes: bytes.to_vec(),
            source_line,
        })
        .collect();
    if records.is_empty() {
        return Err(ReadError::NoRecords);
    }

    Ok(RawFile {
        header_raw: header_raw.to_vec(),
        header,
        encoding,
        has_bom,
        line_ending,
        trailing_newline,
        records,
    })
}

/// Split into records at line terminators outside quotes. Blank lines are
/// dropped; each slice excludes its terminator.
fn split_records(body: &[u8]) -> Result<Vec<&[u8]>, ReadError> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (i, &b) in body.iter().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b'\n' if !in_quotes => {
                let end = if i > start && body[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                push_record(&mut out, &body[start..end]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if in_quotes {
        return Err(ReadError::UnclosedQuote {
            line: out.len().saturating_sub(1),
        });
    }
    push_record(&mut out, &body[start..]);
    Ok(out)
}

fn push_record<'a>(out: &mut Vec<&'a [u8]>, record: &'a [u8]) {
    if !record
        .iter()
        .all(|b| *b == b' ' || *b == b'\t' || *b == b'\r')
    {
        out.push(record);
    }
}

/// RFC 4180 field split of one record, decoding each field.
fn split_fields(record: &[u8], encoding: Encoding) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = Vec::new();
    let mut in_quotes = false;
    let mut i = 0;
    while i < record.len() {
        let b = record[i];
        match (in_quotes, b) {
            (true, b'"') if record.get(i + 1) == Some(&b'"') => {
                field.push(b'"');
                i += 1;
            }
            (_, b'"') => in_quotes = !in_quotes,
            (false, b',') => fields.push(decode(&std::mem::take(&mut field), encoding)),
            _ => field.push(b),
        }
        i += 1;
    }
    fields.push(decode(&field, encoding));
    fields
}

pub(crate) fn decode(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        Encoding::Windows1252 => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

/// Windows-1252 differs from Latin-1 only in 0x80..=0x9F.
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amex_shape_no_trailing_newline() {
        let f = read_raw_file(b"Date,Description,Amount\n05 Nov 2026,COFFEE    VANCOUVER,4.50\n04 Nov 2026,BOOKS,12.00").unwrap();
        assert_eq!(f.header, ["Date", "Description", "Amount"]);
        assert_eq!(f.header_raw, b"Date,Description,Amount");
        assert_eq!(f.encoding, Encoding::Utf8);
        assert_eq!(f.line_ending, LineEnding::Lf);
        assert!(!f.trailing_newline);
        assert_eq!(f.records.len(), 2);
        assert_eq!(f.records[0].bytes, b"05 Nov 2026,COFFEE    VANCOUVER,4.50");
        assert_eq!(
            f.records[0].fields[1], "COFFEE    VANCOUVER",
            "spaces are not trimmed"
        );
        assert_eq!(f.records[1].source_line, 1);
    }

    #[test]
    fn simplii_shape_with_spaces_and_trailing_newline() {
        let f = read_raw_file(
            b"Date, Transaction Details, Funds Out, Funds In \n10/07/2026, E-TRANSFER,,101.19\n",
        )
        .unwrap();
        assert_eq!(
            f.header,
            ["Date", " Transaction Details", " Funds Out", " Funds In "]
        );
        assert!(f.trailing_newline);
        assert_eq!(f.records.len(), 1);
        assert_eq!(
            f.records[0].fields,
            ["10/07/2026", " E-TRANSFER", "", "101.19"]
        );
    }

    #[test]
    fn quoted_comma_and_escaped_quote_keep_raw_bytes() {
        let f = read_raw_file(b"A,B\n\"SHOP, INC\",\"say \"\"hi\"\"\"\n").unwrap();
        let r = &f.records[0];
        assert_eq!(r.fields, ["SHOP, INC", "say \"hi\""]);
        assert_eq!(
            r.bytes, b"\"SHOP, INC\",\"say \"\"hi\"\"\"",
            "raw bytes keep the quotes"
        );
    }

    #[test]
    fn quoted_field_may_span_lines() {
        let f = read_raw_file(b"A,B\n\"line one\nline two\",2\nx,3").unwrap();
        assert_eq!(f.records.len(), 2);
        assert_eq!(f.records[0].fields[0], "line one\nline two");
        assert_eq!(f.records[1].fields, ["x", "3"]);
    }

    #[test]
    fn crlf_is_detected_and_excluded_from_record_bytes() {
        let f = read_raw_file(b"A,B\r\n1,2\r\n3,4\r\n").unwrap();
        assert_eq!(f.line_ending, LineEnding::CrLf);
        assert!(f.trailing_newline);
        assert_eq!(f.header_raw, b"A,B");
        assert_eq!(f.records[1].bytes, b"3,4");
    }

    #[test]
    fn bom_is_detected_and_stripped_from_header() {
        let f = read_raw_file(b"\xEF\xBB\xBFDate,Amount\n05 Nov 2026,1.00").unwrap();
        assert!(f.has_bom);
        assert_eq!(f.header[0], "Date");
        assert_eq!(f.header_raw, b"Date,Amount");
    }

    #[test]
    fn invalid_utf8_falls_back_to_windows_1252() {
        let f = read_raw_file(b"Desc,Amount\nCAF\xC9 \x80,1.00").unwrap();
        assert_eq!(f.encoding, Encoding::Windows1252);
        assert_eq!(f.records[0].fields[0], "CAFÉ €");
        assert_eq!(
            f.records[0].bytes, b"CAF\xC9 \x80,1.00",
            "bytes stay undecoded"
        );
    }

    #[test]
    fn blank_lines_are_skipped() {
        let f = read_raw_file(b"A,B\n\n1,2\n   \n3,4\n\n").unwrap();
        assert_eq!(f.records.len(), 2);
        assert_eq!(f.records[1].source_line, 1);
    }

    #[test]
    fn errors() {
        assert_eq!(read_raw_file(b"").unwrap_err(), ReadError::Empty);
        assert_eq!(read_raw_file(b"\n \n").unwrap_err(), ReadError::Empty);
        assert_eq!(read_raw_file(b"A,B\n").unwrap_err(), ReadError::NoRecords);
        assert!(matches!(
            read_raw_file(b"A,B\n\"open,1\n").unwrap_err(),
            ReadError::UnclosedQuote { .. }
        ));
    }
}
