//! Export: one zip holding a database snapshot, `manifest.json` and the
//! master CSV tree copied as-is (5.1, plan.md → *Export and restore*).
//!
//! Three parts, per plan.md:
//!
//! - `financi.sqlite` — a `VACUUM INTO` snapshot taken on a dedicated
//!   read-only connection, like the worker's reader (3.77), so exporting
//!   never blocks the writer.
//! - `manifest.json` — integer `format_version` (restore refuses a newer
//!   one; the app version string is informational and never parsed), the
//!   schema version, `exported_at`, the database's SHA-256, and one entry
//!   per year CSV: `account_key`, institution, account name, kind, year,
//!   row count and SHA-256 of the exact bytes in the zip.
//! - `csv/…` — every file under the master CSV root, byte for byte. The
//!   2.2 self-check runs first, so missing or damaged year files are
//!   rebuilt before anything is zipped.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Write, copy};
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::master_csv::{self, parse_year_file};
use crate::worker::BUSY_TIMEOUT;
use crate::{Account, DbError, Result, SCHEMA_VERSION, list_accounts};

/// The manifest and zip layout version. Restore refuses an export written
/// by a newer format; bump only when the layout changes incompatibly.
pub const FORMAT_VERSION: u32 = 1;

/// The database snapshot's name inside the zip.
pub const DB_NAME: &str = "financi.sqlite";

/// The manifest's name inside the zip.
pub const MANIFEST_NAME: &str = "manifest.json";

/// The directory the master CSV tree is zipped under.
pub const CSV_DIR: &str = "csv";

/// Write the export zip `dest`: the self-check runs first, then the
/// snapshot, `manifest.json` and the `csv/` tree are zipped. `app_version`
/// is the app's `VERSION` string, recorded for information only.
pub fn export(conn: &Connection, root: &Path, dest: &Path, app_version: &str) -> Result<()> {
    // Rebuild year files that are missing, extra or damaged (2.2) so the
    // zip holds the complete tree the database implies.
    master_csv::self_check(conn, root)?;
    let snapshot = vacuum_snapshot(conn, dest)?;
    let result = write_zip(conn, root, dest, app_version, Path::new(&snapshot));
    let _ = std::fs::remove_file(&snapshot);
    result
}

/// `VACUUM INTO` a fresh temp file beside `dest`, returning its path.
///
/// A file database is snapshotted through its own read-only connection —
/// the 3.77 reader pattern — so the connection passed in is untouched and
/// the export never blocks the writer. An in-memory database has no path
/// to reopen and no separate writer to block, so it is snapshotted in
/// place.
fn vacuum_snapshot(conn: &Connection, dest: &Path) -> Result<String> {
    let tmp = dest.with_file_name(format!(".financi-snapshot-{}.sqlite", uuid::Uuid::new_v4()));
    // Bind and read back the same string so a lossy path cannot diverge
    // between SQLite and our own file access.
    let target = tmp.to_string_lossy().into_owned();
    let reader = conn
        .path()
        .filter(|p| !p.is_empty())
        .map(|path| {
            let reader = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            reader.busy_timeout(BUSY_TIMEOUT)?;
            Ok::<_, DbError>(reader)
        })
        .transpose()?;
    let snapshottable = reader.as_ref().unwrap_or(conn);
    snapshottable.execute("VACUUM INTO ?1", [&target])?;
    Ok(target)
}

fn write_zip(
    conn: &Connection,
    root: &Path,
    dest: &Path,
    app_version: &str,
    snapshot: &Path,
) -> Result<()> {
    let accounts: BTreeMap<String, Account> = list_accounts(conn)?
        .into_iter()
        .map(|account| (account.account_key.0.clone(), account))
        .collect();

    // Read the whole tree and build the manifest before opening the zip,
    // so every hash covers the exact bytes written into it.
    let mut tree: Vec<(String, std::path::PathBuf)> = Vec::new();
    list_tree(root, "", &mut tree)?;
    tree.sort();

    let mut csvs: Vec<(String, Vec<u8>)> = Vec::with_capacity(tree.len());
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for (rel, path) in &tree {
        let zip_name = format!("{CSV_DIR}/{rel}");
        let bytes = std::fs::read(path).map_err(|source| io_error(path, source))?;
        if let Some(entry) = year_entry(&accounts, rel, &zip_name, path, &bytes)? {
            entries.push(entry);
        }
        csvs.push((zip_name, bytes));
    }

    let manifest = serde_json::json!({
        "format_version": FORMAT_VERSION,
        "financi_version": app_version,
        "schema_version": SCHEMA_VERSION,
        "exported_at": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        "database": {
            "path": DB_NAME,
            "sha256": sha256_file(snapshot)?,
        },
        "files": entries,
    });
    let manifest = serde_json::to_vec_pretty(&manifest)
        .map_err(|source| io_error(dest, std::io::Error::other(source)))?;

    let file = File::create(dest).map_err(|source| io_error(dest, source))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    write_zip_entry(&mut zip, dest, MANIFEST_NAME, options, |zip| {
        zip.write_all(&manifest)
    })?;
    let mut snapshot = File::open(snapshot).map_err(|source| io_error(snapshot, source))?;
    write_zip_entry(&mut zip, dest, DB_NAME, options, |zip| {
        copy(&mut snapshot, zip).map(|_| ())
    })?;
    for (zip_name, bytes) in &csvs {
        write_zip_entry(&mut zip, dest, zip_name, options, |zip| {
            zip.write_all(bytes)
        })?;
    }
    zip.finish().map_err(|source| zip_error(dest, source))?;
    Ok(())
}

fn write_zip_entry<W: std::io::Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    dest: &Path,
    name: &str,
    options: SimpleFileOptions,
    contents: impl FnOnce(&mut ZipWriter<W>) -> std::io::Result<()>,
) -> Result<()> {
    zip.start_file(name, options)
        .map_err(|source| zip_error(dest, source))?;
    contents(zip).map_err(|source| io_error(dest, source))
}

/// The manifest entry for one CSV: any `.csv` named as a year file inside a
/// known account's directory (`<institution>/<account_key>/<year>[-vN].csv`).
/// Everything else in the tree — `account.json`, stray files — is zipped
/// without an entry.
fn year_entry(
    accounts: &BTreeMap<String, Account>,
    rel: &str,
    zip_name: &str,
    path: &Path,
    bytes: &[u8],
) -> Result<Option<serde_json::Value>> {
    let parts: Vec<&str> = rel.split('/').collect();
    let [_institution, account_key, name] = parts.as_slice() else {
        return Ok(None);
    };
    let Some((year, _variant)) = parse_year_file(name) else {
        return Ok(None);
    };
    let Some(account) = accounts.get(*account_key) else {
        return Ok(None);
    };
    // Every row in the file counts, hidden ones included; the master CSVs
    // hold them all.
    let row_count = financi_core::read_raw_file(bytes)
        .map(|file| file.records.len())
        .map_err(|source| io_error(path, std::io::Error::other(source)))?;
    Ok(Some(serde_json::json!({
        "path": zip_name,
        "account_key": account.account_key.0,
        "institution": account.institution.key,
        "name": account.name,
        "kind": account.kind.as_str(),
        "year": year,
        "row_count": row_count,
        "sha256": sha256_hex(bytes),
    })))
}

/// Every file under `dir` as `(slash-separated path relative to `dir`, path
/// on disk)`. A missing `dir` — an account-less database that has never run
/// the self-check — is simply empty.
fn list_tree(dir: &Path, rel: &str, out: &mut Vec<(String, std::path::PathBuf)>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error(dir, source)),
    };
    let mut names: Vec<std::fs::DirEntry> = entries
        .collect::<std::result::Result<_, _>>()
        .map_err(|source| io_error(dir, source))?;
    names.sort_by_key(|entry| entry.file_name());
    for entry in names {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let rel = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if path.is_dir() {
            list_tree(&path, &rel, out)?;
        } else {
            out.push((rel, path));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n =
            std::io::Read::read(&mut file, &mut buf).map_err(|source| io_error(path, source))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn io_error(path: &Path, source: std::io::Error) -> DbError {
    DbError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn zip_error(path: &Path, source: zip::result::ZipError) -> DbError {
    io_error(path, std::io::Error::other(source))
}
