//! Export, verify and restore: one zip holding a database snapshot,
//! `manifest.json` and the master CSV tree copied as-is (5.1, 5.2,
//! plan.md → *Export and restore*).
//!
//! Export has three parts, per plan.md:
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
//!
//! Restore is the mirror image: [`verify_export`] checks a zip against its
//! manifest without changing anything, and [`restore`] takes a `.bak`
//! copy of the live database, writes the snapshot into the live
//! connection with rusqlite's backup API (no file swapping), migrates an
//! older schema, and stages the zip's tree before moving it in over
//! `csv/`.

use std::collections::BTreeMap;
use std::ffi::c_int;
use std::fs::File;
use std::io::{Read, Write, copy};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::master_csv::{self, parse_year_file};
use crate::worker::BUSY_TIMEOUT;
use crate::{
    Account, DbError, DbWorker, Result, SCHEMA_VERSION, list_accounts, migrate, on_db_typed,
};

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

// ---------------------------------------------------------------------------
// Verify and restore (5.2)
// ---------------------------------------------------------------------------

/// Pages copied per backup step; a busy source or destination only slows
/// the copy (both worker connections carry `busy_timeout`, 3.77).
const BACKUP_PAGES: c_int = 1024;

/// The parts of `manifest.json` that restore acts on (5.1 writes it).
#[derive(Debug, Clone)]
pub struct Manifest {
    /// The manifest and zip layout version.
    pub format_version: u32,
    /// The schema version of the snapshot and CSVs inside.
    pub schema_version: u32,
    /// The export time, for confirming with the user (5.3).
    pub exported_at: String,
    /// SHA-256 of the `financi.sqlite` snapshot.
    pub database_sha256: String,
    /// One entry per listed CSV. `account.json` and stray files have no
    /// entries and are not hash-checked.
    pub files: Vec<ManifestFile>,
}

/// One CSV's manifest entry: its path in the zip and its SHA-256.
#[derive(Debug, Clone)]
pub struct ManifestFile {
    /// Path inside the zip, e.g. `csv/amex/<key>/2026.csv`.
    pub path: String,
    /// SHA-256 of the exact bytes in the zip.
    pub sha256: String,
}

/// Check an export zip without changing anything: `manifest.json` parses,
/// its format and schema versions are no newer than this build, every
/// file it lists matches its SHA-256, and no `csv/` entry carries a path
/// that would escape the tree. Files the manifest does not list —
/// `account.json`, stray files — carry no hash and are not checked, but
/// their names are: a stray `csv/../../` entry must not pass this gate
/// only to fail the restore after the database is replaced (5.3).
pub fn verify_export(zip_path: &Path) -> Result<Manifest> {
    let mut zip = open_zip(zip_path)?;
    verify_zip(&mut zip, zip_path)
}

/// Restore the export zip `zip_path` into the live `conn` and the master
/// CSV root `root`, returning the manifest it restored (5.2).
///
/// Order is the safety: everything the zip owes is verified first, so a
/// tampered file, a newer schema, or a `csv/` path that would escape the
/// tree refuses before anything changes. Then the current database is
/// copied to `<db>.bak` beside `conn`; the snapshot replaces the live
/// database through rusqlite's backup API on the live connection (no
/// file swapping); an older snapshot migrates to this build's schema;
/// and the zip's tree extracts to `<root>.tmp-<uuid>`, after which
/// `csv/` moves to `csv.bak/` and the staging copy takes its place. A
/// failure part-way — say a corrupt stray file — leaves the live
/// database usable, restored or untouched and never half-written, with
/// the CSV tree and its `csv.bak` left exactly as they were.
///
/// `conn` must be a file database (it needs a path to back up) and not in
/// a transaction: in the app this runs on the writer thread through
/// [`restore_job`].
pub fn restore(conn: &mut Connection, root: &Path, zip_path: &Path) -> Result<Manifest> {
    let mut zip = open_zip(zip_path)?;
    let manifest = verify_zip(&mut zip, zip_path)?;
    let db_path = conn
        .path()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .ok_or(DbError::RestoreNeedsFile)?;
    // The snapshot lands in a temp file beside the live database — a
    // source to open a backup connection from, never swapped in for it.
    let snapshot =
        db_path.with_file_name(format!(".financi-restore-{}.sqlite", uuid::Uuid::new_v4()));
    let result = restore_verified(conn, root, zip_path, &mut zip, &db_path, &snapshot);
    remove_db_files(&snapshot);
    result?;
    Ok(manifest)
}

/// [`restore`] as a [`DbWorker`] job: it runs on the writer thread against
/// the live connection; the reader connection is reused and sees the
/// restored data on its next read (WAL, 3.77). The app refreshes its
/// views when the job returns Ok — `Catalog`'s DataChanged is 5.3.
pub async fn restore_job(db: &DbWorker, root: &Path, zip_path: &Path) -> Result<Manifest> {
    let (root, zip_path) = (root.to_path_buf(), zip_path.to_path_buf());
    on_db_typed(db, move |conn| restore(conn, &root, &zip_path)).await
}

fn open_zip(zip_path: &Path) -> Result<ZipArchive<File>> {
    let file = File::open(zip_path).map_err(|source| io_error(zip_path, source))?;
    ZipArchive::new(file).map_err(|source| invalid_export(zip_path, source.to_string()))
}

/// The manifest, parsed and proven: versions this build can take, and a
/// matching SHA-256 for every file the manifest lists.
fn verify_zip(zip: &mut ZipArchive<File>, zip_path: &Path) -> Result<Manifest> {
    let manifest = parse_manifest(zip_path, &read_entry(zip, zip_path, MANIFEST_NAME)?)?;
    if manifest.format_version > FORMAT_VERSION {
        return Err(DbError::ExportFormatTooNew {
            found: manifest.format_version,
            supported: FORMAT_VERSION,
        });
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(DbError::TooNew {
            found: manifest.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    check_hash(zip, zip_path, DB_NAME, &manifest.database_sha256)?;
    for file in &manifest.files {
        check_hash(zip, zip_path, &file.path, &file.sha256)?;
    }
    // A `csv/` entry whose name `csv_target` refuses can only fail while
    // extracting — after the database is replaced. Refuse those names
    // here, before anything changes. Names only: the root they would land
    // under is irrelevant to the shape of the path.
    let prefix = format!("{CSV_DIR}/");
    for name in zip.file_names() {
        let name = name.map_err(|source| zip_error(zip_path, source))?;
        if let Some(rel) = name.strip_prefix(&prefix) {
            csv_target(zip_path, Path::new("/"), rel)?;
        }
    }
    Ok(manifest)
}

/// Pull the fields restore needs out of `manifest.json`.
fn parse_manifest(zip_path: &Path, bytes: &[u8]) -> Result<Manifest> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|source| invalid_export(zip_path, format!("{MANIFEST_NAME}: {source}")))?;
    let uint = |name: &str| -> Result<u32> {
        value
            .get(name)
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| {
                invalid_export(zip_path, format!("{MANIFEST_NAME} has no integer {name:?}"))
            })
    };
    let text = |where_: &serde_json::Value, name: &str| -> Result<String> {
        where_
            .get(name)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                invalid_export(zip_path, format!("{MANIFEST_NAME} has no string {name:?}"))
            })
    };
    // The snapshot is `financi.sqlite` by contract (5.1); an export that
    // says otherwise is not this format.
    let database = value.get("database").ok_or_else(|| {
        invalid_export(zip_path, format!("{MANIFEST_NAME} has no database object"))
    })?;
    let db_name = text(database, "path")?;
    if db_name != DB_NAME {
        return Err(invalid_export(
            zip_path,
            format!("unexpected database path {db_name:?}"),
        ));
    }
    let listed = value
        .get("files")
        .and_then(|f| f.as_array())
        .ok_or_else(|| invalid_export(zip_path, format!("{MANIFEST_NAME} has no files array")))?;
    let mut files = Vec::with_capacity(listed.len());
    for entry in listed {
        files.push(ManifestFile {
            path: text(entry, "path")?,
            sha256: text(entry, "sha256")?,
        });
    }
    Ok(Manifest {
        format_version: uint("format_version")?,
        schema_version: uint("schema_version")?,
        exported_at: text(&value, "exported_at")?,
        database_sha256: text(database, "sha256")?,
        files,
    })
}

/// One entry's bytes, or an `InvalidExport` naming the zip.
fn read_entry(zip: &mut ZipArchive<File>, zip_path: &Path, name: &str) -> Result<Vec<u8>> {
    let mut entry = zip
        .by_name(name)
        .map_err(|source| invalid_export(zip_path, format!("{name}: {source}")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(zip_path, source))?;
    Ok(bytes)
}

fn check_hash(
    zip: &mut ZipArchive<File>,
    zip_path: &Path,
    name: &str,
    expected: &str,
) -> Result<()> {
    let found = sha256_hex(&read_entry(zip, zip_path, name)?);
    if found != expected {
        return Err(DbError::ExportHashMismatch {
            path: zip_path.to_path_buf(),
            entry: name.to_string(),
            expected: expected.to_string(),
            found,
        });
    }
    Ok(())
}

/// The mutation half of [`restore`], after the zip has been verified.
fn restore_verified(
    conn: &mut Connection,
    root: &Path,
    zip_path: &Path,
    zip: &mut ZipArchive<File>,
    db_path: &Path,
    snapshot: &Path,
) -> Result<()> {
    // A database at a schema this build can migrate, checked before the
    // live data is touched. The manifest's own version was verified
    // already; a manifest that lies cannot get past the snapshot's.
    write_entry(zip, zip_path, DB_NAME, snapshot)?;
    let source = Connection::open(snapshot).map_err(|source| {
        invalid_export(
            zip_path,
            format!("the snapshot is not a database: {source}"),
        )
    })?;
    source.busy_timeout(BUSY_TIMEOUT)?;
    let found: u32 = source.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(DbError::TooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }

    // `.bak` copies of the live data, then the restore itself. The backup
    // API writes through the live connection (no file swapping), so the
    // reader connection of 3.77 stays open and sees the new data in WAL.
    let db_backup = backup_path(db_path);
    let mut backup_conn = Connection::open(&db_backup)
        .map_err(|source| io_error(&db_backup, std::io::Error::other(source)))?;
    run_backup(conn, &mut backup_conn)?;
    drop(backup_conn);
    run_backup(&source, conn)?;
    drop(source);
    // A snapshot from an older build catches up (5.2).
    migrate(conn)?;

    // Then the CSV tree, staged: the zip's tree extracts into
    // `<root>.tmp-<uuid>` first, and only once the whole tree has landed
    // does `csv/` move aside to `csv.bak/` and the staging copy take its
    // place. If an entry fails to extract, the live tree and the earlier
    // `csv.bak` are left alone — a retry cannot clobber the only good
    // backup with half a tree. The database is already restored by this
    // point: usable, restored or untouched, never half-written.
    let staging = staging_csv_root(root);
    let staged = std::fs::create_dir_all(&staging)
        .map_err(|source| io_error(&staging, source))
        .and_then(|()| extract_csv_tree(zip, zip_path, &staging))
        .and_then(|()| {
            back_up_csv_root(root)?;
            std::fs::rename(&staging, root).map_err(|source| io_error(root, source))
        });
    if staged.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    staged
}

/// `<root>.tmp-<uuid>`: the staging directory the zip's tree extracts
/// into before it replaces the live root.
fn staging_csv_root(root: &Path) -> PathBuf {
    let name = root.file_name().unwrap_or_default().to_string_lossy();
    root.with_file_name(format!("{name}.tmp-{}", uuid::Uuid::new_v4()))
}

/// Pages through one online backup. `run_to_completion` retries while the
/// other connection holds a lock, so a busy source or destination only
/// slows it down.
fn run_backup(from: &Connection, to: &mut Connection) -> Result<()> {
    let backup = rusqlite::backup::Backup::new(from, to)?;
    backup.run_to_completion(BACKUP_PAGES, Duration::from_millis(10), None)?;
    Ok(())
}

/// `<name>.bak` beside `path` (`financi.sqlite` → `financi.sqlite.bak`).
fn backup_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{name}.bak"))
}

/// A database file and its WAL sidecars, if any.
fn remove_db_files(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

/// Move `root` aside as `<root>.bak`, replacing an earlier backup; a
/// missing tree has nothing to back up. Runs only after the zip's tree
/// has fully extracted into the staging directory, so a failed restore
/// never reaches it and never destroys the earlier backup.
fn back_up_csv_root(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let bak = backup_path(root);
    if bak.exists() {
        let removed = if bak.is_dir() {
            std::fs::remove_dir_all(&bak)
        } else {
            std::fs::remove_file(&bak)
        };
        removed.map_err(|source| io_error(&bak, source))?;
    }
    std::fs::rename(root, &bak).map_err(|source| io_error(root, source))
}

/// Extract the zip's `csv/` tree under `dest` — the staging directory,
/// which the caller renames onto the live root only after this returns
/// Ok. The manifest and the snapshot belong to the other steps; a
/// corrupt stray entry — one the manifest never hashed, so verification
/// passed — fails here, part-way, harmlessly in the staging copy.
fn extract_csv_tree(zip: &mut ZipArchive<File>, zip_path: &Path, dest: &Path) -> Result<()> {
    let prefix = format!("{CSV_DIR}/");
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|source| zip_error(zip_path, source))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .map_err(|source| zip_error(zip_path, source))?
            .into_owned();
        let Some(rel) = name.strip_prefix(&prefix) else {
            continue;
        };
        let path = csv_target(zip_path, dest, rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }
        let mut out = File::create(&path).map_err(|source| io_error(&path, source))?;
        copy(&mut entry, &mut out).map_err(|source| io_error(&path, source))?;
    }
    Ok(())
}

/// Where a `csv/…` entry lands under `root`; paths that climb out of the
/// tree are refused.
fn csv_target(zip_path: &Path, root: &Path, rel: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        if part == ".." {
            return Err(invalid_export(
                zip_path,
                format!("{CSV_DIR}/{rel} escapes the csv tree"),
            ));
        }
        if part.is_empty() || part == "." {
            return Err(invalid_export(
                zip_path,
                format!("{CSV_DIR}/{rel} is not a file name"),
            ));
        }
        path.push(part);
    }
    if path == root {
        return Err(invalid_export(
            zip_path,
            format!("{CSV_DIR}/ is not a file"),
        ));
    }
    Ok(path)
}

/// Write one zip entry to `dest`, reading it through the zip crate so its
/// CRC check applies to what lands on disk.
fn write_entry(zip: &mut ZipArchive<File>, zip_path: &Path, name: &str, dest: &Path) -> Result<()> {
    let mut entry = zip
        .by_name(name)
        .map_err(|source| invalid_export(zip_path, format!("{name}: {source}")))?;
    let mut out = File::create(dest).map_err(|source| io_error(dest, source))?;
    copy(&mut entry, &mut out).map_err(|source| io_error(dest, source))?;
    Ok(())
}

fn invalid_export(path: &Path, reason: impl Into<String>) -> DbError {
    DbError::InvalidExport {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}
