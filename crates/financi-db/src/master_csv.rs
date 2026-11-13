//! Master CSVs: one provider-format file per account and calendar year,
//! kept beside the database and rebuilt from the stored raw rows.
//!
//! The database is the source of truth; these files are always regenerable
//! from it. A row's bytes are never edited: they are exactly the
//! `transactions.raw_bytes` from the provider's export. Rows are only added
//! (import) or removed (undo).
//!
//! Layout: `<db dir>/csv/<institution-key>/<account_key>/<YYYY>.csv`, plus
//! an `account.json` with the readable name, institution and kind. Rows
//! from imports whose header, encoding or BOM differs from the earliest
//! import for that year go to `<YYYY>-v2.csv`, `-v3`, ...

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use financi_core::{Encoding, LineEnding};
use rusqlite::{Connection, ToSql, params_from_iter};

use crate::{Account, DbError, Result, get_account, list_accounts};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// The master CSV root beside a database file: `<db dir>/csv`.
pub fn root_for_db(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("csv")
}

/// What a startup self-check did. `rewritten_years` holds `(account_id, year)`
/// pairs that were rebuilt because the files were missing, extra or did not
/// match the database byte for byte.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SelfCheckReport {
    pub accounts: usize,
    pub rewritten_years: Vec<(i64, i32)>,
    pub account_json_updated: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    import_id: i64,
    source_line: i64,
    raw_bytes: Vec<u8>,
    posted_date: String,
}

impl Row {
    fn year(&self) -> Option<i32> {
        self.posted_date.get(0..4)?.parse().ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportMeta {
    id: i64,
    parser_id: String,
    header_raw: Vec<u8>,
    encoding: Encoding,
    has_bom: bool,
    line_ending: LineEnding,
    trailing_newline: bool,
}

/// One file to write: its name and exact bytes. The self-check compares
/// against these same bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct YearFile {
    name: String,
    contents: Vec<u8>,
}

/// Regenerate the given calendar-year files for one account from the
/// database, and refresh its `account.json`. An empty year deletes that
/// year's files. Writes are atomic (temp file, fsync, rename).
pub fn rewrite_years(conn: &Connection, root: &Path, account_id: i64, years: &[i32]) -> Result<()> {
    let account = get_account(conn, account_id)?;
    let dir = account_dir(root, &account);
    ensure_account_json(&dir, &account)?;

    let wanted: BTreeSet<i32> = years.iter().copied().collect();
    // Load only the years being written (3.73): an import or undo touches
    // one or two years, not the account's whole history.
    let rows = if wanted.is_empty() {
        Vec::new()
    } else {
        account_rows(conn, account_id, Some(&wanted))?
    };
    let mut by_year: BTreeMap<i32, Vec<(&Row, &ImportMeta)>> = BTreeMap::new();
    for (row, meta) in &rows {
        if let Some(year) = row.year()
            && wanted.contains(&year)
        {
            by_year.entry(year).or_default().push((row, meta));
        }
    }
    for year in wanted {
        let year_rows = by_year.get(&year).map(Vec::as_slice).unwrap_or(&[]);
        rewrite_year(&dir, year, year_rows)?;
    }
    Ok(())
}

/// Rebuild any account's year files that are missing, extra, or whose
/// contents differ from the bytes the database implies. Also writes missing
/// `account.json` files. Notices are logged to stderr.
pub fn self_check(conn: &Connection, root: &Path) -> Result<SelfCheckReport> {
    let accounts = list_accounts(conn)?;
    let mut report = SelfCheckReport {
        accounts: accounts.len(),
        ..Default::default()
    };

    for account in &accounts {
        let dir = account_dir(root, account);
        if ensure_account_json(&dir, account)? {
            report.account_json_updated += 1;
        }

        let rows = account_rows(conn, account.id, None)?;
        let mut by_year: BTreeMap<i32, Vec<(&Row, &ImportMeta)>> = BTreeMap::new();
        for (row, meta) in &rows {
            if let Some(year) = row.year() {
                by_year.entry(year).or_default().push((row, meta));
            }
        }

        let existing = existing_year_files(&dir)?;
        let years: BTreeSet<i32> = by_year
            .keys()
            .copied()
            .chain(existing.keys().copied())
            .collect();
        for year in years {
            let year_rows = by_year.get(&year).map(Vec::as_slice).unwrap_or(&[]);
            let expected = year_files(year, year_rows)?;
            let actual = existing.get(&year);
            if year_is_current(expected.as_slice(), actual) {
                continue;
            }
            rewrite_year(&dir, year, year_rows)?;
            report.rewritten_years.push((account.id, year));
            eprintln!(
                "financi: rebuilt master CSV for {} ({year})",
                account.display_name()
            );
        }
    }
    Ok(report)
}

/// True when the files on disk match the expected set: same names and the
/// exact bytes `year_files` builds (3.70: a count-only check never noticed
/// an edited or half-written file whose rows still counted right). The
/// comparison is against the rendered bytes themselves, so encoding, BOM,
/// line endings and trailing newline are all covered.
fn year_is_current(expected: &[YearFile], actual: Option<&BTreeMap<String, PathBuf>>) -> bool {
    match (expected.is_empty(), actual) {
        (true, None) => true,
        (true, Some(_)) => false,
        (false, None) => false,
        (false, Some(files)) => {
            if files.len() != expected.len() {
                return false;
            }
            expected.iter().all(|file| {
                files
                    .get(&file.name)
                    .and_then(|path| fs::read(path).ok())
                    .is_some_and(|bytes| bytes == file.contents)
            })
        }
    }
}

fn account_dir(root: &Path, account: &Account) -> PathBuf {
    root.join(&account.institution.key)
        .join(&account.account_key.0)
}

/// All transactions for an account, hidden rows included, with the metadata
/// of the import they came from. Row aliases are not rows of their own.
/// `years`, when given, loads only rows whose `posted_date` starts with one
/// of those years — `rewrite_years` rewrites only the years it touched, so
/// it never needs the other years' raw bytes (3.73). A row whose date has
/// no readable year can never match a wanted year, so it is not loaded
/// either; the caller's own `Row::year` check stays authoritative.
fn account_rows(
    conn: &Connection,
    account_id: i64,
    years: Option<&BTreeSet<i32>>,
) -> Result<Vec<(Row, ImportMeta)>> {
    let mut sql = String::from(
        "SELECT t.import_id, t.source_line, t.raw_bytes, t.posted_date,
                i.id, i.parser_id, i.header_raw, i.encoding, i.has_bom,
                i.line_ending, i.trailing_newline
           FROM transactions t JOIN imports i ON i.id = t.import_id
          WHERE t.account_id = ?1",
    );
    let mut args: Vec<Box<dyn ToSql>> = vec![Box::new(account_id)];
    if let Some(years) = years.filter(|ys| !ys.is_empty()) {
        let marks = (0..years.len())
            .map(|i| format!("?{}", args.len() + 1 + i))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(
            " AND CAST(substr(t.posted_date, 1, 4) AS INTEGER) IN ({marks})"
        ));
        args.extend(years.iter().map(|y| Box::new(*y) as Box<dyn ToSql>));
    }
    sql.push_str(" ORDER BY t.import_id, t.source_line");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args), |r| {
        let encoding: String = r.get(7)?;
        let line_ending: String = r.get(9)?;
        Ok((
            Row {
                import_id: r.get(0)?,
                source_line: r.get(1)?,
                raw_bytes: r.get(2)?,
                posted_date: r.get(3)?,
            },
            ImportMeta {
                id: r.get(4)?,
                parser_id: r.get(5)?,
                header_raw: r.get(6)?,
                encoding: encoding_from_str(&encoding),
                has_bom: r.get(8)?,
                line_ending: line_ending_from_str(&line_ending),
                trailing_newline: r.get(10)?,
            },
        ))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn encoding_from_str(s: &str) -> Encoding {
    if s == Encoding::Windows1252.as_str() {
        Encoding::Windows1252
    } else {
        Encoding::Utf8
    }
}

fn line_ending_from_str(s: &str) -> LineEnding {
    if s == LineEnding::CrLf.as_str() {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

type VariantKey = (Vec<u8>, &'static str, bool);

fn variant_key(meta: &ImportMeta) -> VariantKey {
    (
        meta.header_raw.clone(),
        meta.encoding.as_str(),
        meta.has_bom,
    )
}

/// Build the variant files for one year. The first (v1) variant is the one
/// from the earliest import in the year; later distinct variants are `-v2`,
/// `-v3`, ... in earliest-import order. Within a variant, rows are ordered
/// by posted date (provider direction), then import order, then source line.
/// Encoding, BOM, line ending and trailing newline come from the earliest
/// import of that variant.
fn year_files(year: i32, rows: &[(&Row, &ImportMeta)]) -> Result<Vec<YearFile>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let newest_first = newest_first(rows)?;

    struct Group<'a> {
        key: VariantKey,
        earliest: &'a ImportMeta,
        rows: Vec<&'a Row>,
    }

    let mut groups: Vec<Group<'_>> = Vec::new();
    for (row, meta) in rows {
        let key = variant_key(meta);
        match groups.iter_mut().find(|g| g.key == key) {
            Some(group) => {
                if meta.id < group.earliest.id {
                    group.earliest = meta;
                }
                group.rows.push(row);
            }
            None => groups.push(Group {
                key,
                earliest: meta,
                rows: vec![row],
            }),
        }
    }
    groups.sort_by_key(|g| g.earliest.id);

    Ok(groups
        .into_iter()
        .enumerate()
        .map(|(ix, mut group)| {
            group.rows.sort_by(|a, b| {
                let by_date = a.posted_date.cmp(&b.posted_date);
                let by_date = if newest_first {
                    by_date.reverse()
                } else {
                    by_date
                };
                by_date
                    .then_with(|| a.import_id.cmp(&b.import_id))
                    .then_with(|| a.source_line.cmp(&b.source_line))
            });
            let name = if ix == 0 {
                format!("{year}.csv")
            } else {
                format!("{year}-v{}.csv", ix + 1)
            };
            YearFile {
                name,
                contents: render(group.earliest, &group.rows),
            }
        })
        .collect())
}

fn newest_first(rows: &[(&Row, &ImportMeta)]) -> Result<bool> {
    let earliest = rows
        .iter()
        .map(|(_, meta)| *meta)
        .min_by_key(|meta| meta.id)
        .expect("non-empty rows");
    let parser = financi_parsers::by_id(&earliest.parser_id)
        .ok_or_else(|| DbError::UnknownParser(earliest.parser_id.clone()))?;
    Ok(parser.newest_first())
}

/// Header + untouched record bytes, using the import's encoding markers.
fn render(meta: &ImportMeta, rows: &[&Row]) -> Vec<u8> {
    let line_ending: &[u8] = match meta.line_ending {
        LineEnding::Lf => b"\n",
        LineEnding::CrLf => b"\r\n",
    };
    let mut out = Vec::new();
    if meta.has_bom {
        out.extend_from_slice(UTF8_BOM);
    }
    out.extend_from_slice(&meta.header_raw);
    for row in rows {
        out.extend_from_slice(line_ending);
        out.extend_from_slice(&row.raw_bytes);
    }
    if meta.trailing_newline {
        out.extend_from_slice(line_ending);
    }
    out
}

/// Rewrite one year: write every expected variant, then delete stale files
/// for that year (including all files when the year is now empty).
fn rewrite_year(dir: &Path, year: i32, rows: &[(&Row, &ImportMeta)]) -> Result<()> {
    fs::create_dir_all(dir).map_err(|source| io_error(dir, source))?;
    let existing = year_files_on_disk(dir, year)?;

    if rows.is_empty() {
        for path in existing.into_values() {
            remove_file(&path)?;
        }
        return Ok(());
    }

    let files = year_files(year, rows)?;
    let expected: BTreeSet<&str> = files.iter().map(|f| f.name.as_str()).collect();
    for file in &files {
        write_atomic(&dir.join(&file.name), &file.contents)?;
    }
    for (name, path) in existing {
        if !expected.contains(name.as_str()) {
            remove_file(&path)?;
        }
    }
    Ok(())
}

/// Year files present in `dir`, keyed by year then file name.
fn existing_year_files(dir: &Path) -> Result<BTreeMap<i32, BTreeMap<String, PathBuf>>> {
    let mut out: BTreeMap<i32, BTreeMap<String, PathBuf>> = BTreeMap::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(source) => return Err(io_error(dir, source)),
    };
    for entry in entries {
        let entry = entry.map_err(|source| io_error(dir, source))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some((year, _variant)) = parse_year_file(name) {
            out.entry(year).or_default().insert(name.to_string(), path);
        }
    }
    Ok(out)
}

fn year_files_on_disk(dir: &Path, year: i32) -> Result<BTreeMap<String, PathBuf>> {
    Ok(existing_year_files(dir)?.remove(&year).unwrap_or_default())
}

/// `2026.csv` -> (2026, 1); `2026-v2.csv` -> (2026, 2).
fn parse_year_file(name: &str) -> Option<(i32, u32)> {
    let stem = name.strip_suffix(".csv")?;
    match stem.split_once("-v") {
        None if stem.len() == 4 => Some((stem.parse().ok()?, 1)),
        Some((year, variant)) if year.len() == 4 => {
            let variant: u32 = variant.parse().ok()?;
            let year: i32 = year.parse().ok()?;
            (variant >= 1).then_some((year, variant))
        }
        _ => None,
    }
}

/// Write `account.json` if it is missing or its content changed. Returns
/// whether the file was written.
fn ensure_account_json(dir: &Path, account: &Account) -> Result<bool> {
    let path = dir.join("account.json");
    let contents = account_json(account);
    write_if_changed(&path, contents.as_bytes())
}

fn account_json(account: &Account) -> String {
    format!(
        "{{\n  \"name\": {},\n  \"institution\": {},\n  \"kind\": {}\n}}\n",
        json_string(&account.name),
        json_string(&account.institution.name),
        json_string(account.kind.as_str()),
    )
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool> {
    match fs::read(path) {
        Ok(existing) if existing == contents => Ok(false),
        _ => {
            write_atomic(path, contents)?;
            Ok(true)
        }
    }
}

/// Temp file in the same directory, fsync, then rename over the target.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(|source| io_error(dir, source))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("master.csv");
    let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));

    let result = (|| -> std::io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        if let Ok(dir) = File::open(dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|source| io_error(path, source))
}

fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> DbError {
    DbError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{CommitOptions, ImportOutcome, commit_import, undo_last_import};
    use crate::{create_account, get_account, open_in_memory};
    use financi_core::{AccountKind, RawFile, read_raw_file};
    use rusqlite::Connection;

    const AMEX_HEADER: &str = "Date,Date Processed,Description,Amount";

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("financi-master-csv-{}", uuid::Uuid::new_v4()))
    }

    fn account(conn: &Connection, institution: &str, name: &str) -> i64 {
        let kind = match institution {
            "simplii" => AccountKind::Chequing,
            _ => AccountKind::CreditCard,
        };
        create_account(conn, institution, name, kind).unwrap()
    }

    fn import(
        conn: &mut Connection,
        account_id: i64,
        parser_id: &str,
        bytes: &[u8],
        name: &str,
    ) -> ImportOutcome {
        let file: RawFile = read_raw_file(bytes).unwrap();
        let parser = financi_parsers::by_id(parser_id).unwrap();
        commit_import(
            conn,
            account_id,
            parser.as_ref(),
            &file,
            name,
            "2026-11-07T12:00:00",
            CommitOptions::default(),
        )
        .unwrap()
    }

    fn amex_file(rows: &[&str]) -> Vec<u8> {
        let mut out = format!("{AMEX_HEADER}\n");
        out.push_str(&rows.join("\n"));
        out.into_bytes()
    }

    fn account_dir_for(conn: &Connection, root: &Path, account_id: i64) -> PathBuf {
        account_dir(root, &get_account(conn, account_id).unwrap())
    }

    fn read(path: &Path) -> Vec<u8> {
        fs::read(path).unwrap()
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn db_raw_rows(conn: &Connection, account_id: i64, newest_first: bool) -> Vec<Vec<u8>> {
        let order = if newest_first { "DESC" } else { "ASC" };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT raw_bytes FROM transactions WHERE account_id = ?1
                  ORDER BY posted_date {order}, import_id, source_line"
            ))
            .unwrap();
        stmt.query_map([account_id], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn overlapping_imports_collapse_into_one_byte_exact_year_file() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();

        let first = amex_file(&[
            "05 Nov 2026,05 Nov 2026,COFFEE A,1.00",
            "01 Oct 2026,01 Oct 2026,COFFEE B,2.00",
            "15 Sep 2026,15 Sep 2026,COFFEE C,3.00",
        ]);
        let second = amex_file(&[
            "05 Dec 2026,05 Dec 2026,COFFEE D,4.00",
            "20 Nov 2026,20 Nov 2026,COFFEE E,5.00",
            "05 Nov 2026,05 Nov 2026,COFFEE A,1.00",
            "10 Oct 2026,10 Oct 2026,COFFEE F,6.00",
        ]);
        let a = import(&mut conn, acct, "amex", &first, "sep-nov.csv");
        let b = import(&mut conn, acct, "amex", &second, "oct-dec.csv");
        assert_eq!(a.affected_years, [2026].into_iter().collect());
        assert_eq!(
            b.inserted, 3,
            "the overlapping Nov row is not inserted twice"
        );
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();

        let dir = account_dir_for(&conn, &root, acct);
        assert_eq!(names_in(&dir), ["2026.csv", "account.json"]);

        let expected = [
            "05 Dec 2026,05 Dec 2026,COFFEE D,4.00",
            "20 Nov 2026,20 Nov 2026,COFFEE E,5.00",
            "05 Nov 2026,05 Nov 2026,COFFEE A,1.00",
            "10 Oct 2026,10 Oct 2026,COFFEE F,6.00",
            "01 Oct 2026,01 Oct 2026,COFFEE B,2.00",
            "15 Sep 2026,15 Sep 2026,COFFEE C,3.00",
        ];
        let mut wanted = format!("{AMEX_HEADER}\n");
        wanted.push_str(&expected.join("\n"));
        assert_eq!(read(&dir.join("2026.csv")), wanted.as_bytes());

        let db_rows = db_raw_rows(&conn, acct, true);
        let file_rows: Vec<Vec<u8>> = read_raw_file(&read(&dir.join("2026.csv")))
            .unwrap()
            .records
            .into_iter()
            .map(|r| r.bytes)
            .collect();
        assert_eq!(file_rows, db_rows, "file rows equal the DB rows");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_dec_jan_import_splits_into_two_year_files() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        let file = amex_file(&[
            "05 Jan 2026,05 Jan 2026,NEW YEAR,1.00",
            "20 Dec 2025,20 Dec 2025,OLD YEAR,2.00",
        ]);
        let outcome = import(&mut conn, acct, "amex", &file, "dec-jan.csv");
        assert_eq!(outcome.affected_years, [2025, 2026].into_iter().collect());
        rewrite_years(&conn, &root, acct, &[2025, 2026]).unwrap();

        let dir = account_dir_for(&conn, &root, acct);
        assert_eq!(names_in(&dir), ["2025.csv", "2026.csv", "account.json"]);
        assert_eq!(
            read(&dir.join("2025.csv")),
            format!("{AMEX_HEADER}\n20 Dec 2025,20 Dec 2025,OLD YEAR,2.00").as_bytes()
        );
        assert_eq!(
            read(&dir.join("2026.csv")),
            format!("{AMEX_HEADER}\n05 Jan 2026,05 Jan 2026,NEW YEAR,1.00").as_bytes()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_identical_repeats_appear_twice() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        let row = "05 Nov 2026,05 Nov 2026,SAME SHOP,4.50";
        let file = amex_file(&[row, row]);
        import(&mut conn, acct, "amex", &file, "repeat.csv");
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();

        let dir = account_dir_for(&conn, &root, acct);
        let contents = String::from_utf8(read(&dir.join("2026.csv"))).unwrap();
        assert_eq!(
            contents.matches(row).count(),
            2,
            "identical repeats are kept as many times as they occurred"
        );
        assert_eq!(db_raw_rows(&conn, acct, true).len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn undo_removes_only_its_rows_and_deletes_an_empty_year_file() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        let first = amex_file(&[
            "05 Nov 2026,05 Nov 2026,ONE,1.00",
            "04 Nov 2026,04 Nov 2026,TWO,2.00",
        ]);
        let second = amex_file(&["03 Nov 2026,03 Nov 2026,THREE,3.00"]);
        let first_import = import(&mut conn, acct, "amex", &first, "first.csv");
        let second_import = import(&mut conn, acct, "amex", &second, "second.csv");
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();

        let dir = account_dir_for(&conn, &root, acct);
        let undone = undo_last_import(&mut conn, acct, second_import.import_id.unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(undone.affected_years, [2026].into_iter().collect());
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();
        let after_first = String::from_utf8(read(&dir.join("2026.csv"))).unwrap();
        assert!(!after_first.contains("THREE"), "{after_first}");
        assert!(after_first.contains("ONE") && after_first.contains("TWO"));

        let undone = undo_last_import(&mut conn, acct, first_import.import_id.unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(undone.removed, 2);
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();
        assert_eq!(
            names_in(&dir),
            ["account.json"],
            "the empty year is deleted"
        );
        assert_eq!(db_raw_rows(&conn, acct, true).len(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hidden_rows_stay_in_the_files() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        let file = amex_file(&[
            "05 Nov 2026,05 Nov 2026,VISIBLE,1.00",
            "04 Nov 2026,04 Nov 2026,HIDDEN,2.00",
        ]);
        import(&mut conn, acct, "amex", &file, "hidden.csv");
        conn.execute(
            "UPDATE transactions SET hidden = 1 WHERE description_raw = 'HIDDEN'",
            [],
        )
        .unwrap();
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();

        let dir = account_dir_for(&conn, &root, acct);
        let contents = String::from_utf8(read(&dir.join("2026.csv"))).unwrap();
        assert!(contents.contains("HIDDEN"), "{contents}");
        assert!(contents.contains("VISIBLE"), "{contents}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_different_header_goes_to_v2() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        let first = amex_file(&["05 Nov 2026,05 Nov 2026,FIRST,1.00"]);
        // Same parser, different raw header (trailing space).
        let second = format!("{AMEX_HEADER} \n04 Nov 2026,04 Nov 2026,SECOND,2.00").into_bytes();
        import(&mut conn, acct, "amex", &first, "one.csv");
        import(&mut conn, acct, "amex", &second, "two.csv");
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();

        let dir = account_dir_for(&conn, &root, acct);
        assert_eq!(names_in(&dir), ["2026-v2.csv", "2026.csv", "account.json"]);
        let v1 = String::from_utf8(read(&dir.join("2026.csv"))).unwrap();
        let v2 = String::from_utf8(read(&dir.join("2026-v2.csv"))).unwrap();
        assert_eq!(
            v1,
            format!("{AMEX_HEADER}\n05 Nov 2026,05 Nov 2026,FIRST,1.00")
        );
        assert_eq!(
            v2,
            format!("{AMEX_HEADER} \n04 Nov 2026,04 Nov 2026,SECOND,2.00")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_file_row_is_an_import_record() {
        let mut conn = open_in_memory().unwrap();
        let amex = account(&conn, "amex", "Cobalt");
        let chequing = account(&conn, "simplii", "No Fee Chequing");
        let root = temp_root();
        import(
            &mut conn,
            amex,
            "amex",
            &amex_file(&[
                "05 Nov 2026,05 Nov 2026,AMEX ONE,1.00",
                "04 Nov 2026,04 Nov 2026,AMEX TWO,2.00",
            ]),
            "amex.csv",
        );
        let simplii = b"Date, Transaction Details, Funds Out, Funds In \n\
10/07/2026, E-TRANSFER SEND,1450.00,\n\
10/08/2026, E-TRANSFER RECEIVE,,40.00\n";
        import(
            &mut conn,
            chequing,
            "simplii-chequing",
            simplii,
            "simplii.csv",
        );
        rewrite_years(&conn, &root, amex, &[2026]).unwrap();
        rewrite_years(&conn, &root, chequing, &[2026]).unwrap();

        for (account_id, parser_id, newest_first) in
            [(amex, "amex", true), (chequing, "simplii-chequing", false)]
        {
            let dir = account_dir_for(&conn, &root, account_id);
            let db_rows = db_raw_rows(&conn, account_id, newest_first);
            let mut seen = Vec::new();
            for name in names_in(&dir) {
                if !name.ends_with(".csv") {
                    continue;
                }
                let file = read_raw_file(&read(&dir.join(&name))).unwrap();
                for record in file.records {
                    assert!(
                        db_rows.contains(&record.bytes),
                        "{parser_id} row not from an import: {:?}",
                        String::from_utf8_lossy(&record.bytes)
                    );
                    seen.push(record.bytes);
                }
            }
            assert_eq!(seen, db_rows, "file order follows the provider's");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_check_restores_deleted_files_and_removes_extra_ones() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        let file = amex_file(&["05 Nov 2026,05 Nov 2026,ONE,1.00"]);
        import(&mut conn, acct, "amex", &file, "one.csv");
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();
        let dir = account_dir_for(&conn, &root, acct);
        let original = read(&dir.join("2026.csv"));

        fs::remove_file(dir.join("2026.csv")).unwrap();
        fs::write(dir.join("2024.csv"), format!("{AMEX_HEADER}\nstale")).unwrap();
        let report = self_check(&conn, &root).unwrap();
        assert!(report.rewritten_years.contains(&(acct, 2026)));
        assert!(report.rewritten_years.contains(&(acct, 2024)));
        assert_eq!(read(&dir.join("2026.csv")), original);
        assert!(
            !dir.join("2024.csv").exists(),
            "a year with no database rows is removed"
        );

        // A file with the wrong row count is rebuilt too.
        fs::write(dir.join("2026.csv"), format!("{AMEX_HEADER}\n")).unwrap();
        let report = self_check(&conn, &root).unwrap();
        assert!(report.rewritten_years.contains(&(acct, 2026)));
        assert_eq!(read(&dir.join("2026.csv")), original);

        // Same row count, different bytes (3.70): a count-only check never
        // noticed an edited file, the byte comparison rebuilds it.
        let tampered = String::from_utf8(original.clone())
            .unwrap()
            .replace("ONE,1.00", "ONE,9.99");
        assert_ne!(tampered.as_bytes(), original.as_slice());
        fs::write(dir.join("2026.csv"), tampered).unwrap();
        let report = self_check(&conn, &root).unwrap();
        assert!(report.rewritten_years.contains(&(acct, 2026)));
        assert_eq!(read(&dir.join("2026.csv")), original);

        // Files that already match byte for byte are left alone.
        let report = self_check(&conn, &root).unwrap();
        assert!(
            report.rewritten_years.is_empty(),
            "current files are not rebuilt: {:?}",
            report.rewritten_years
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn account_json_has_name_institution_and_kind_and_follows_a_rename() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        let file = amex_file(&["05 Nov 2026,05 Nov 2026,ONE,1.00"]);
        import(&mut conn, acct, "amex", &file, "one.csv");
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();
        let dir = account_dir_for(&conn, &root, acct);
        assert_eq!(
            String::from_utf8(read(&dir.join("account.json"))).unwrap(),
            "{\n  \"name\": \"Cobalt\",\n  \"institution\": \"American Express\",\n  \"kind\": \"credit_card\"\n}\n"
        );

        conn.execute(
            "UPDATE accounts SET name = 'Cobalt \"Rewards\"' WHERE id = ?1",
            [acct],
        )
        .unwrap();
        rewrite_years(&conn, &root, acct, &[]).unwrap();
        let json = String::from_utf8(read(&dir.join("account.json"))).unwrap();
        assert!(json.contains("\"Cobalt \\\"Rewards\\\"\""), "{json}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encoding_bom_line_ending_and_trailing_newline_come_from_the_earliest_import() {
        let mut conn = open_in_memory().unwrap();
        let acct = account(&conn, "amex", "Cobalt");
        let root = temp_root();
        // Earliest import: BOM, CRLF, trailing newline.
        let first = b"\xEF\xBB\xBFDate,Date Processed,Description,Amount\r\n\
05 Nov 2026,05 Nov 2026,ONE,1.00\r\n";
        // Later import: same header/encoding/BOM but LF and no trailing newline.
        let second = b"\xEF\xBB\xBFDate,Date Processed,Description,Amount\n\
04 Nov 2026,04 Nov 2026,TWO,2.00";
        import(&mut conn, acct, "amex", first, "one.csv");
        import(&mut conn, acct, "amex", second, "two.csv");
        rewrite_years(&conn, &root, acct, &[2026]).unwrap();

        let dir = account_dir_for(&conn, &root, acct);
        let contents = read(&dir.join("2026.csv"));
        assert!(
            contents.starts_with(UTF8_BOM),
            "BOM comes from the first import"
        );
        assert_eq!(
            contents,
            b"\xEF\xBB\xBFDate,Date Processed,Description,Amount\r\n\
05 Nov 2026,05 Nov 2026,ONE,1.00\r\n\
04 Nov 2026,04 Nov 2026,TWO,2.00\r\n"
                .to_vec(),
            "line ending and trailing newline follow the earliest import"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn root_for_db_is_beside_the_database_file() {
        assert_eq!(
            root_for_db(Path::new("/tmp/dev.sqlite")),
            PathBuf::from("/tmp/csv")
        );
        assert_eq!(
            root_for_db(Path::new("/home/u/.local/share/financi/financi.sqlite")),
            PathBuf::from("/home/u/.local/share/financi/csv")
        );
    }
}
