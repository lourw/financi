//! 5.2 restore tests: `verify_export` refuses a tampered file, a newer
//! schema, or a stray `csv/` path that escapes the tree before anything
//! changes; `restore` copies the live database to `financi.sqlite.bak`,
//! writes the snapshot into the live connection (backup API), migrates an
//! older schema, and replaces `csv/` with the zip's tree through a
//! staging copy, so a failure part-way leaves the live database usable
//! and the live tree and its `csv.bak` untouched.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use financi_core::{AccountKind, RawFile, read_raw_file};
use financi_db::export::{self, CSV_DIR, DB_NAME, FORMAT_VERSION, MANIFEST_NAME};
use financi_db::import::{CommitOptions, commit_import};
use financi_db::master_csv::{rewrite_years, root_for_db};
use financi_db::rusqlite::Connection;
use financi_db::{
    DbError, DbWorker, MIGRATIONS, SCHEMA_VERSION, create_account, list_accounts, open,
};
use futures_executor::block_on;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const AMEX_HEADER: &str = "Date,Date Processed,Description,Amount";
const APP_VERSION: &str = "v9.9.9-test";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("financi-restore-{tag}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn amex_file(rows: &[&str]) -> RawFile {
    let text = std::iter::once(AMEX_HEADER)
        .chain(rows.iter().copied())
        .collect::<Vec<_>>()
        .join("\n");
    read_raw_file(text.as_bytes()).unwrap()
}

fn commit(conn: &mut Connection, account_id: i64, file: &RawFile, name: &str) {
    let parser = financi_parsers::by_id("amex").unwrap();
    commit_import(
        conn,
        account_id,
        parser.as_ref(),
        file,
        name,
        "2026-11-07T12:00:00",
        CommitOptions::default(),
    )
    .unwrap();
}

/// A file-backed database with an Amex account whose master CSVs cover
/// 2025 and 2026 (the 5.1 export fixture). Returns the connection, the
/// CSV root and the temp dir (the caller removes the dir).
fn fixture(tag: &str) -> (Connection, PathBuf, PathBuf) {
    let dir = temp_dir(tag);
    let mut conn = open(&dir.join("financi.sqlite")).unwrap();
    let acct = create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    commit(
        &mut conn,
        acct,
        &amex_file(&[
            "05 Nov 2026,05 Nov 2026,COFFEE,5.00",
            "20 Dec 2025,20 Dec 2025,HOTEL,200.00",
        ]),
        "two-years.csv",
    );
    let root = root_for_db(&dir.join("financi.sqlite"));
    rewrite_years(&conn, &root, acct, &[2025, 2026]).unwrap();
    (conn, root, dir)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn account_key(conn: &Connection) -> String {
    list_accounts(conn).unwrap()[0].account_key.0.clone()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sha256_file(path: &Path) -> String {
    sha256_hex(&fs::read(path).unwrap())
}

/// Copy `dir/export.zip` to `dir/name`, running every entry's bytes
/// through `edit` first.
fn rebuild_zip(dir: &Path, name: &str, edit: impl Fn(&str, Vec<u8>) -> Vec<u8>) -> PathBuf {
    let dest = dir.join(name);
    let mut src = ZipArchive::new(File::open(dir.join("export.zip")).unwrap()).unwrap();
    let mut out = ZipWriter::new(File::create(&dest).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for i in 0..src.len() {
        let mut entry = src.by_index(i).unwrap();
        let entry_name = entry.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        out.start_file(&entry_name, options).unwrap();
        out.write_all(&edit(&entry_name, bytes)).unwrap();
    }
    out.finish().unwrap();
    dest
}

/// Build a zip from `(name, bytes)` pairs. `.bin` entries are stored
/// uncompressed so their bytes can be corrupted in place afterwards.
fn build_zip(dir: &Path, name: &str, entries: &[(&str, Vec<u8>)]) -> PathBuf {
    let dest = dir.join(name);
    let mut zip = ZipWriter::new(File::create(&dest).unwrap());
    for (entry_name, bytes) in entries {
        let method = if entry_name.ends_with(".bin") {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let options = SimpleFileOptions::default().compression_method(method);
        zip.start_file(*entry_name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    dest
}

/// A manifest naming the snapshot at `db` and `files` (pairs of zip path
/// and bytes), at the given schema version.
fn manifest_json(schema_version: u32, db: &Path, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let files: Vec<serde_json::Value> = files
        .iter()
        .map(|(path, bytes)| serde_json::json!({ "path": path, "sha256": sha256_hex(bytes) }))
        .collect();
    let manifest = serde_json::json!({
        "format_version": FORMAT_VERSION,
        "financi_version": "v0.0.0-test",
        "schema_version": schema_version,
        "exported_at": "2026-11-01T00:00:00Z",
        "database": { "path": DB_NAME, "sha256": sha256_file(db) },
        "files": files,
    });
    serde_json::to_vec_pretty(&manifest).unwrap()
}

#[test]
fn a_restore_replaces_the_database_and_csv_tree_after_bak_copies() {
    let (mut conn, root, dir) = fixture("round-trip");
    let key = account_key(&conn);
    let year_file = root.join("amex").join(&key).join("2026.csv");
    let zip = dir.join("export.zip");
    export::export(&conn, &root, &zip, APP_VERSION).unwrap();
    let exported_csv = fs::read(&year_file).unwrap();

    // The live data moves on after the export.
    let acct = list_accounts(&conn).unwrap()[0].id;
    commit(
        &mut conn,
        acct,
        &amex_file(&["06 Nov 2026,06 Nov 2026,NEW,7.00"]),
        "new.csv",
    );
    rewrite_years(&conn, &root, acct, &[2026]).unwrap();
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 3);

    // Through the worker, as the app runs it: the restore lands on the
    // writer thread and the reader connection sees the restored data.
    let worker = DbWorker::spawn(conn);
    let manifest = block_on(export::restore_job(&worker, &root, &zip)).unwrap();
    assert_eq!(manifest.schema_version, SCHEMA_VERSION);
    assert_eq!(manifest.files.len(), 2, "the manifest the export wrote");

    // The database is the export's again, at this build's schema.
    let (rows, new_rows, version) = block_on(worker.call(|c| {
        let version: u32 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        (
            count(c, "SELECT count(*) FROM transactions"),
            count(
                c,
                "SELECT count(*) FROM transactions WHERE description_raw = 'NEW'",
            ),
            version,
        )
    }))
    .unwrap();
    assert_eq!((rows, new_rows), (2, 0));
    assert_eq!(version, SCHEMA_VERSION);

    // The reader connection sees the restored rows too (WAL, 3.77): a
    // read through `call_read` after `restore_job` is the export's data.
    let (read_rows, read_new) = block_on(worker.call_read(|c| {
        (
            count(c, "SELECT count(*) FROM transactions"),
            count(
                c,
                "SELECT count(*) FROM transactions WHERE description_raw = 'NEW'",
            ),
        )
    }))
    .unwrap();
    assert_eq!(
        (read_rows, read_new),
        (2, 0),
        "the reader connection sees the restored data"
    );

    // The csv tree is the zip's, byte for byte.
    assert_eq!(fs::read(&year_file).unwrap(), exported_csv);

    // And the `.bak` copies hold what was live before the restore.
    let db_bak = Connection::open(dir.join("financi.sqlite.bak")).unwrap();
    assert_eq!(count(&db_bak, "SELECT count(*) FROM transactions"), 3);
    assert_eq!(
        count(
            &db_bak,
            "SELECT count(*) FROM transactions WHERE description_raw = 'NEW'"
        ),
        1
    );
    let csv_bak = root.with_file_name("csv.bak");
    let bak_text = fs::read_to_string(csv_bak.join("amex").join(&key).join("2026.csv")).unwrap();
    assert!(
        bak_text.contains("NEW"),
        "the pre-restore tree is in csv.bak"
    );

    drop(worker);
    // The threads exit on the last handle; give them a moment to close the
    // files before the temp dir goes away.
    std::thread::sleep(Duration::from_millis(100));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_tampered_file_is_refused_before_anything_changes() {
    let (mut conn, root, dir) = fixture("tampered");
    let key = account_key(&conn);
    let year_file = root.join("amex").join(&key).join("2025.csv");
    let zip = dir.join("export.zip");
    export::export(&conn, &root, &zip, APP_VERSION).unwrap();
    let on_disk = fs::read(&year_file).unwrap();

    let bad = rebuild_zip(&dir, "tampered.zip", |name, bytes| {
        if name.ends_with("2025.csv") {
            b"TAMPERED\n".to_vec()
        } else {
            bytes
        }
    });
    let err = export::verify_export(&bad).unwrap_err();
    assert!(
        matches!(&err, DbError::ExportHashMismatch { entry, .. } if entry.ends_with("2025.csv")),
        "{err}"
    );
    assert!(matches!(
        export::restore(&mut conn, &root, &bad),
        Err(DbError::ExportHashMismatch { .. })
    ));

    // Nothing changed: same rows, same files, no `.bak` written.
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 2);
    assert_eq!(fs::read(&year_file).unwrap(), on_disk);
    assert!(!dir.join("financi.sqlite.bak").exists());
    assert!(!dir.join("csv.bak").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_newer_schema_version_is_refused() {
    let (mut conn, root, dir) = fixture("newer-schema");
    let zip = dir.join("export.zip");
    export::export(&conn, &root, &zip, APP_VERSION).unwrap();

    let bad = rebuild_zip(&dir, "newer-schema.zip", |name, bytes| {
        if name != MANIFEST_NAME {
            return bytes;
        }
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
        serde_json::to_vec(&json).unwrap()
    });
    assert!(matches!(
        export::verify_export(&bad),
        Err(DbError::TooNew { found, .. }) if found == SCHEMA_VERSION + 1
    ));
    assert!(matches!(
        export::restore(&mut conn, &root, &bad),
        Err(DbError::TooNew { .. })
    ));
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 2);
    assert!(!dir.join("financi.sqlite.bak").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_newer_format_version_is_refused() {
    let (conn, root, dir) = fixture("newer-format");
    let zip = dir.join("export.zip");
    export::export(&conn, &root, &zip, APP_VERSION).unwrap();

    let bad = rebuild_zip(&dir, "newer-format.zip", |name, bytes| {
        if name != MANIFEST_NAME {
            return bytes;
        }
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        json["format_version"] = serde_json::json!(FORMAT_VERSION + 1);
        serde_json::to_vec(&json).unwrap()
    });
    assert!(matches!(
        export::verify_export(&bad),
        Err(DbError::ExportFormatTooNew { found, .. }) if found == FORMAT_VERSION + 1
    ));
    assert!(!dir.join("financi.sqlite.bak").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_older_schema_migrates_during_restore() {
    let (mut conn, root, dir) = fixture("older-schema");
    let key = account_key(&conn);

    // A snapshot two migrations behind, with its own transaction.
    let old_version = SCHEMA_VERSION - 2;
    let snap = dir.join("old-snapshot.sqlite");
    {
        let old = Connection::open(&snap).unwrap();
        for sql in &MIGRATIONS[..old_version as usize] {
            old.execute_batch(sql).unwrap();
        }
        old.pragma_update(None, "user_version", old_version)
            .unwrap();
        let acct = create_account(&old, "amex", "Old Card", AccountKind::CreditCard).unwrap();
        old.execute(
            "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'old.csv', '2025-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
            [acct],
        )
        .unwrap();
        old.execute(
            "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, 0, X'00', 'old-hash', '2025-10-31', -100, 'FROM-THE-PAST')",
            (acct, old.last_insert_rowid()),
        )
        .unwrap();
    }

    // The zip's csv tree differs from the live one: the restore must
    // replace the whole tree, not merge.
    let csv_path = format!("{CSV_DIR}/amex/{key}/2026.csv");
    let csv_bytes = fs::read(root.join("amex").join(&key).join("2026.csv")).unwrap();
    let extra_path = format!("{CSV_DIR}/amex/{key}/only-in-the-zip.txt");
    let zip = build_zip(
        &dir,
        "old.zip",
        &[
            (
                MANIFEST_NAME,
                manifest_json(old_version, &snap, &[(&csv_path, csv_bytes.clone())]),
            ),
            (DB_NAME, fs::read(&snap).unwrap()),
            (&csv_path, csv_bytes),
            (&extra_path, b"new tree".to_vec()),
        ],
    );

    let manifest = export::restore(&mut conn, &root, &zip).unwrap();
    assert_eq!(manifest.schema_version, old_version);
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION, "the old snapshot caught up");
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM transactions WHERE description_raw = 'FROM-THE-PAST'"
        ),
        1,
        "the old database's rows are the live rows now"
    );

    // The tree is the zip's: the extra file landed, the old ones are gone.
    assert!(
        root.join("amex")
            .join(&key)
            .join("only-in-the-zip.txt")
            .exists()
    );
    assert!(!root.join("amex").join(&key).join("2025.csv").exists());
    // The old tree is safe in csv.bak, the old database in sqlite.bak.
    assert!(
        root.with_file_name("csv.bak")
            .join("amex")
            .join(&key)
            .join("2025.csv")
            .exists()
    );
    let db_bak = Connection::open(dir.join("financi.sqlite.bak")).unwrap();
    assert_eq!(count(&db_bak, "SELECT count(*) FROM transactions"), 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_failure_part_way_leaves_the_live_database_usable() {
    let (conn, root, dir) = fixture("part-way");
    let key = account_key(&conn);

    // A snapshot of a different database to restore into the live one.
    let snap = dir.join("other.sqlite");
    {
        let other = open(&snap).unwrap();
        let acct = create_account(&other, "amex", "Other", AccountKind::CreditCard).unwrap();
        other
            .execute(
                "INSERT INTO imports (account_id, parser_id, file_name, imported_at, header_raw,
                                  encoding, has_bom, line_ending, trailing_newline, row_count)
             VALUES (?1, 'amex', 'other.csv', '2026-11-01', X'00', 'utf-8', 0, 'lf', 0, 1)",
                [acct],
            )
            .unwrap();
        other
            .execute(
                "INSERT INTO transactions (account_id, import_id, source_line, raw_bytes, row_hash,
                                       posted_date, amount_cents, description_raw)
             VALUES (?1, ?2, 0, X'00', 'r', '2026-11-02', -250, 'RESTORED')",
                (acct, other.last_insert_rowid()),
            )
            .unwrap();
    }

    // A zip whose only unhashed entry — a stray the manifest never lists,
    // so verification passes — is corrupt: reading it fails with a CRC
    // error during the csv replacement, after the database is restored.
    let csv_path = format!("{CSV_DIR}/amex/{key}/2026.csv");
    let csv_bytes = b"Date,Amount\n".to_vec();
    let stray = format!("{CSV_DIR}/stray.bin");
    let stray_bytes = b"STRAY-PAYLOAD-PATTERN-0123456789".to_vec();
    let zip = build_zip(
        &dir,
        "corrupt-stray.zip",
        &[
            (
                MANIFEST_NAME,
                manifest_json(SCHEMA_VERSION, &snap, &[(&csv_path, csv_bytes.clone())]),
            ),
            (DB_NAME, fs::read(&snap).unwrap()),
            (&csv_path, csv_bytes),
            (&stray, stray_bytes.clone()),
        ],
    );
    let mut raw = fs::read(&zip).unwrap();
    let at = raw
        .windows(stray_bytes.len())
        .position(|w| w == &stray_bytes[..])
        .unwrap();
    raw[at] ^= 0xFF;
    fs::write(&zip, &raw).unwrap();

    // Through the worker, as the app will run it.
    let worker = DbWorker::spawn(conn);
    let err = block_on(export::restore_job(&worker, &root, &zip)).unwrap_err();
    assert!(matches!(err, DbError::Io { .. }), "{err}");

    // The database restore completed first and the worker still answers.
    let (rows, restored) = block_on(worker.call(|c| {
        (
            count(c, "SELECT count(*) FROM transactions"),
            count(
                c,
                "SELECT count(*) FROM transactions WHERE description_raw = 'RESTORED'",
            ),
        )
    }))
    .unwrap();
    assert_eq!(
        (rows, restored),
        (1, 1),
        "the restored database is live and usable"
    );

    // The database `.bak` copy is on disk. The CSV extraction ran in a
    // staging copy, so the failure left the live tree and its `csv.bak`
    // exactly as they were, and cleaned the staging copy up.
    assert!(dir.join("financi.sqlite.bak").exists());
    let db_bak = Connection::open(dir.join("financi.sqlite.bak")).unwrap();
    assert_eq!(count(&db_bak, "SELECT count(*) FROM transactions"), 2);
    assert!(
        !root.with_file_name("csv.bak").exists(),
        "a failed extraction never moves the live tree aside"
    );
    let live = fs::read_to_string(root.join("amex").join(&key).join("2026.csv")).unwrap();
    assert!(live.contains("COFFEE"), "the live tree is untouched");
    assert!(!root.join("stray.bin").exists());
    let staging: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("csv.tmp-"))
        .collect();
    assert!(
        staging.is_empty(),
        "the staging copy is removed: {staging:?}"
    );

    drop(worker);
    // The threads exit on the last handle; give them a moment to close the
    // files before the temp dir goes away.
    std::thread::sleep(Duration::from_millis(100));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_stray_escaping_csv_path_is_refused_before_anything_changes() {
    let (mut conn, root, dir) = fixture("escape");
    let key = account_key(&conn);

    // A valid snapshot and manifest — every hash checks out — plus a
    // stray entry the manifest never lists whose path climbs out of the
    // csv tree. It can only be extracted to `dir/escape.txt`; the gate
    // must refuse it on the name alone, before `restore` mutates a thing.
    let snap = dir.join("snap.sqlite");
    {
        let other = open(&snap).unwrap();
        let _ = create_account(&other, "amex", "Escape", AccountKind::CreditCard).unwrap();
    }
    let csv_path = format!("{CSV_DIR}/amex/{key}/2026.csv");
    let csv_bytes = fs::read(root.join("amex").join(&key).join("2026.csv")).unwrap();
    let escape = format!("{CSV_DIR}/../escape.txt");
    let zip = build_zip(
        &dir,
        "escape.zip",
        &[
            (
                MANIFEST_NAME,
                manifest_json(SCHEMA_VERSION, &snap, &[(&csv_path, csv_bytes.clone())]),
            ),
            (DB_NAME, fs::read(&snap).unwrap()),
            (&csv_path, csv_bytes),
            (&escape, b"ESCAPED".to_vec()),
        ],
    );

    let err = export::verify_export(&zip).unwrap_err();
    assert!(
        matches!(&err, DbError::InvalidExport { reason, .. }
            if reason.contains("escapes the csv tree")),
        "{err}"
    );
    assert!(matches!(
        export::restore(&mut conn, &root, &zip),
        Err(DbError::InvalidExport { .. })
    ));

    // Nothing changed: same rows, no `.bak` copies, nothing escaped.
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 2);
    assert!(!dir.join("financi.sqlite.bak").exists());
    assert!(!root.with_file_name("csv.bak").exists());
    assert!(!dir.join("escape.txt").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_retry_after_a_part_way_failure_keeps_the_good_backup() {
    let (mut conn, root, dir) = fixture("retry-backup");
    let key = account_key(&conn);
    let year_file = root.join("amex").join(&key).join("2026.csv");

    // A first, successful restore leaves `csv.bak` holding the tree that
    // was live before it — the only good copy of those bytes.
    let zip = dir.join("export.zip");
    export::export(&conn, &root, &zip, APP_VERSION).unwrap();
    let acct = list_accounts(&conn).unwrap()[0].id;
    commit(
        &mut conn,
        acct,
        &amex_file(&["06 Nov 2026,06 Nov 2026,NEW,7.00"]),
        "new.csv",
    );
    rewrite_years(&conn, &root, acct, &[2026]).unwrap();
    export::restore(&mut conn, &root, &zip).unwrap();
    let csv_bak = root.with_file_name("csv.bak");
    let good_bak = fs::read(csv_bak.join("amex").join(&key).join("2026.csv")).unwrap();
    assert!(good_bak.windows(3).any(|w| w == b"NEW"));
    let live_tree = fs::read(&year_file).unwrap();

    // A second restore — a corrupt-stray zip like the part-way test's —
    // fails during extraction. It must leave `csv.bak` and the live tree
    // untouched, so the good backup survives a retry.
    let snap = dir.join("snap2.sqlite");
    {
        let other = open(&snap).unwrap();
        let _ = create_account(&other, "amex", "Retry", AccountKind::CreditCard).unwrap();
    }
    let csv_path = format!("{CSV_DIR}/amex/{key}/2026.csv");
    let csv_bytes = b"Date,Amount\n".to_vec();
    let stray = format!("{CSV_DIR}/stray.bin");
    let stray_bytes = b"STRAY-PAYLOAD-PATTERN-0123456789".to_vec();
    let bad = build_zip(
        &dir,
        "corrupt-again.zip",
        &[
            (
                MANIFEST_NAME,
                manifest_json(SCHEMA_VERSION, &snap, &[(&csv_path, csv_bytes.clone())]),
            ),
            (DB_NAME, fs::read(&snap).unwrap()),
            (&csv_path, csv_bytes),
            (&stray, stray_bytes.clone()),
        ],
    );
    let mut raw = fs::read(&bad).unwrap();
    let at = raw
        .windows(stray_bytes.len())
        .position(|w| w == &stray_bytes[..])
        .unwrap();
    raw[at] ^= 0xFF;
    fs::write(&bad, &raw).unwrap();

    let err = export::restore(&mut conn, &root, &bad).unwrap_err();
    assert!(matches!(err, DbError::Io { .. }), "{err}");

    assert_eq!(
        fs::read(csv_bak.join("amex").join(&key).join("2026.csv")).unwrap(),
        good_bak,
        "the only good backup survived the failed retry"
    );
    assert_eq!(
        fs::read(&year_file).unwrap(),
        live_tree,
        "the live tree was never moved aside"
    );
    assert!(!root.join("stray.bin").exists());
    fs::remove_dir_all(dir).unwrap();
}
