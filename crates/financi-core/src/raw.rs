//! Reads a CSV export into a [`RawFile`] while keeping every record's exact
//! bytes, so rows can be hashed as exported and written back byte for byte.
//!
//! - Encoding: a UTF-8 BOM is detected and stripped from the header; valid
//!   UTF-8 is decoded as UTF-8; anything else is decoded as Windows-1252.
//!   Files over [`MAX_FILE_BYTES`] or containing a NUL byte are rejected as
//!   not-CSV before parsing.
//! - Records end at an LF or CRLF outside quotes, so a quoted field may span
//!   lines. Record bytes exclude the terminator.
//! - Quoting follows RFC 4180 with one real-world allowance (3.67): a quote
//!   only *opens* a field at the field's start, so a stray `"` inside an
//!   unquoted field (`BEST BUY 55" TV`) is a literal character. A quoted
//!   field closes at `"` followed by `,`, end of line or end of input;
//!   `""` inside quotes is an escaped quote.
//! - Fields are otherwise left untouched: no trimming.
//! - Blank lines carry no data and are skipped.

use crate::{Encoding, LineEnding, RawFile, RawRecord};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReadError {
    #[error("the file is empty")]
    Empty,
    #[error("the file has a header row but no transactions")]
    NoRecords,
    #[error("line {line}: unclosed quote")]
    UnclosedQuote { line: usize },
    #[error("This isn't a CSV export.")]
    NotCsv,
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Largest file [`read_raw_file`] accepts. Bank exports are a few MB at
/// most; anything bigger is the wrong file, not a statement.
pub const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;

pub fn read_raw_file(bytes: &[u8]) -> Result<RawFile, ReadError> {
    reject_non_csv(bytes, MAX_FILE_BYTES)?;
    let has_bom = bytes.starts_with(UTF8_BOM);
    let body = if has_bom {
        &bytes[UTF8_BOM.len()..]
    } else {
        bytes
    };
    if body.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(ReadError::Empty);
    }

    let encoding = detect_encoding(body, has_bom);
    let line_ending = if body.windows(2).any(|w| w == b"\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    let trailing_newline = body.ends_with(b"\n");

    let mut slices = split_records(body)?.into_iter();
    let (header_raw, _header_line) = slices.next().ok_or(ReadError::Empty)?;
    let header = split_fields(header_raw, encoding);

    let records: Vec<RawRecord> = slices
        .enumerate()
        .map(|(source_line, (bytes, file_line))| RawRecord {
            fields: split_fields(bytes, encoding),
            bytes: bytes.to_vec(),
            source_line,
            file_line,
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

/// Files that can never be a CSV export fail fast: oversized or binary
/// (a NUL byte never appears in text a bank exports). Split out with an
/// explicit `max` so the guard is testable without a 50 MB allocation.
fn reject_non_csv(bytes: &[u8], max: usize) -> Result<(), ReadError> {
    if bytes.len() > max || bytes.contains(&0) {
        return Err(ReadError::NotCsv);
    }
    Ok(())
}

fn detect_encoding(body: &[u8], has_bom: bool) -> Encoding {
    if has_bom || std::str::from_utf8(body).is_ok() {
        Encoding::Utf8
    } else {
        Encoding::Windows1252
    }
}

/// Best-effort header for error messages when a file fails to parse (3.67):
/// the first physical line, decoded and split like a record. Never fails.
pub fn sniff_header(bytes: &[u8]) -> Vec<String> {
    let has_bom = bytes.starts_with(UTF8_BOM);
    let body = if has_bom {
        &bytes[UTF8_BOM.len()..]
    } else {
        bytes
    };
    let end = body.iter().position(|&b| b == b'\n').unwrap_or(body.len());
    let mut line = &body[..end];
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    split_fields(line, detect_encoding(body, has_bom))
}

/// True when the `"` at `i` closes a quoted field: the next byte is the
/// field separator, a terminator, or the end of input.
fn closes_quoted_field(bytes: &[u8], i: usize) -> bool {
    matches!(bytes.get(i + 1), None | Some(b',' | b'\n' | b'\r'))
}

/// Split into records at line terminators outside quotes. Blank lines are
/// dropped; each slice excludes its terminator and carries the 1-based
/// physical line it starts on.
fn split_records(body: &[u8]) -> Result<Vec<(&[u8], usize)>, ReadError> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut line_no = 1; // physical line containing the byte being read
    let mut start_line = 1; // physical line the current record starts on
    let mut in_quotes = false;
    let mut at_field_start = true;
    let mut i = 0;
    while i < body.len() {
        match body[i] {
            b'"' if in_quotes => {
                if body.get(i + 1) == Some(&b'"') {
                    i += 1; // escaped quote: both bytes belong to the field
                } else if closes_quoted_field(body, i) {
                    in_quotes = false;
                }
                // any other `"` stays literal content of the quoted field
            }
            b'"' if at_field_start => {
                in_quotes = true;
                at_field_start = false;
            }
            b',' if !in_quotes => at_field_start = true,
            b'\n' if in_quotes => line_no += 1, // a quoted field spans lines
            b'\n' => {
                let end = if i > start && body[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                push_record(&mut out, &body[start..end], start_line);
                start = i + 1;
                line_no += 1;
                start_line = line_no;
                at_field_start = true;
            }
            _ => at_field_start = false,
        }
        i += 1;
    }
    if in_quotes {
        return Err(ReadError::UnclosedQuote { line: start_line });
    }
    push_record(&mut out, &body[start..], start_line);
    Ok(out)
}

fn push_record<'a>(out: &mut Vec<(&'a [u8], usize)>, record: &'a [u8], file_line: usize) {
    if !record
        .iter()
        .all(|b| *b == b' ' || *b == b'\t' || *b == b'\r')
    {
        out.push((record, file_line));
    }
}

/// RFC 4180 field split of one record, decoding each field. A quote opens
/// a quoted field only at the field's start; elsewhere it is literal, so a
/// description like `BEST BUY 55" TV` survives intact.
fn split_fields(record: &[u8], encoding: Encoding) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = Vec::new();
    let mut in_quotes = false;
    let mut at_field_start = true;
    let mut i = 0;
    while i < record.len() {
        match record[i] {
            b'"' if in_quotes => {
                if record.get(i + 1) == Some(&b'"') {
                    field.push(b'"');
                    i += 1; // escaped quote
                } else if closes_quoted_field(record, i) {
                    in_quotes = false;
                } else {
                    field.push(b'"');
                }
            }
            b'"' if at_field_start => {
                in_quotes = true;
                at_field_start = false;
            }
            b',' if !in_quotes => {
                fields.push(decode(&std::mem::take(&mut field), encoding));
                at_field_start = true;
            }
            b => {
                field.push(b);
                at_field_start = false;
            }
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
        assert_eq!(
            read_raw_file(b"A,B\n\"open,1\n").unwrap_err(),
            ReadError::UnclosedQuote { line: 2 }
        );
    }

    #[test]
    fn quote_is_literal_inside_an_unquoted_field() {
        // An Amex description like `BEST BUY 55" TV` must not open a quote
        // and swallow the following lines (3.67).
        let f = read_raw_file(
            b"Date,Description,Amount\n05 Nov 2026,BEST BUY 55\" TV    TORONTO,549.99\n04 Nov 2026,BOOKS,12.00\n",
        )
        .unwrap();
        assert_eq!(f.records.len(), 2);
        assert_eq!(f.records[0].fields[1], "BEST BUY 55\" TV    TORONTO");
        assert_eq!(f.records[1].fields, ["04 Nov 2026", "BOOKS", "12.00"]);
    }

    #[test]
    fn quoted_field_closes_at_comma_line_end_or_eof() {
        let f = read_raw_file(b"A,B\n\"x,y\",2\n\"3,4\",5\n\"6,7\"").unwrap();
        assert_eq!(f.records.len(), 3);
        assert_eq!(f.records[0].fields, ["x,y", "2"]);
        assert_eq!(f.records[1].fields, ["3,4", "5"]);
        assert_eq!(f.records[2].fields, ["6,7"]);
    }

    #[test]
    fn quoted_amount_with_thousands_comma() {
        let f = read_raw_file(b"A,B\nSHOP,\"1,234.56\"\n").unwrap();
        assert_eq!(f.records[0].fields, ["SHOP", "1,234.56"]);
    }

    #[test]
    fn file_line_counts_physical_lines() {
        // Blank lines advance file_line but not source_line; a quoted field
        // spanning lines makes the two diverge further.
        let f = read_raw_file(b"A,B\n\n1,2\n\"x\ny\",3\n4,5").unwrap();
        assert_eq!(f.records.len(), 3);
        assert_eq!(
            f.records
                .iter()
                .map(|r| (r.source_line, r.file_line))
                .collect::<Vec<_>>(),
            vec![(0, 3), (1, 4), (2, 6)]
        );
    }

    #[test]
    fn unclosed_quote_reports_the_physical_line() {
        let err = read_raw_file(b"A,B\n1,2\n\n\"oops\n").unwrap_err();
        assert_eq!(err, ReadError::UnclosedQuote { line: 4 });
    }

    #[test]
    fn nul_byte_is_not_csv() {
        assert_eq!(
            read_raw_file(b"A,B\n1,\x002\n").unwrap_err(),
            ReadError::NotCsv
        );
    }

    #[test]
    fn oversized_file_is_not_csv() {
        // Guard is exercised with a tiny cap; read_raw_file passes 50 MB.
        assert_eq!(
            reject_non_csv(b"12345", 4).unwrap_err(),
            ReadError::NotCsv,
            "over the cap"
        );
        assert_eq!(reject_non_csv(b"1234", 4), Ok(()));
        assert_eq!(reject_non_csv(b"4\x00", 50).unwrap_err(), ReadError::NotCsv);
    }

    #[test]
    fn sniff_header_reads_the_first_line_even_when_broken() {
        assert_eq!(sniff_header(b"A,B\n1,2\n"), ["A", "B"]);
        assert_eq!(sniff_header(b"\xEF\xBB\xBFH1,H2\r\nx"), ["H1", "H2"]);
        // an unclosed quote in the header: the rest of the line stays one field
        assert_eq!(sniff_header(b"\"junk,no close\n1,2\n"), ["junk,no close"]);
        assert_eq!(sniff_header(b""), [""]);
    }
}
