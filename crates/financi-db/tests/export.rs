//! 5.1 export tests: the zip's three parts, manifest hashes and row counts
//! against the zipped bytes, the self-check rebuilding a missing year file
//! before zipping, and hidden rows staying in the CSVs.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use financi_core::{AccountKind, RawFile, read_raw_file};
use financi_db::export::{self, CSV_DIR, DB_NAME, FORMAT_VERSION, MANIFEST_NAME};
use financi_db::import::{CommitOptions, commit_import};
use financi_db::master_csv::{rewrite_years, root_for_db};
use financi_db::rusqlite::Connection;
use financi_db::{SCHEMA_VERSION, create_account, list_accounts, open, open_in_memory};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

const AMEX_HEADER: &str = "Date,Date Processed,Description,Amount";
const APP_VERSION: &str = "v9.9.9-test";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("financi-export-{tag}-{}", uuid::Uuid::new_v4()));
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

fn commit(conn: &mut Connection, account_id: i64, parser_id: &str, file: &RawFile, name: &str) {
    let parser = financi_parsers::by_id(parser_id).unwrap();
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
/// 2025 and 2026. Returns the connection, the CSV root and the temp dir
/// (the caller removes the dir).
fn fixture(tag: &str) -> (Connection, PathBuf, PathBuf) {
    let dir = temp_dir(tag);
    let mut conn = open(&dir.join("financi.sqlite")).unwrap();
    let acct = create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    commit(
        &mut conn,
        acct,
        "amex",
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

fn archive(dest: &Path) -> ZipArchive<fs::File> {
    ZipArchive::new(fs::File::open(dest).unwrap()).unwrap()
}

fn zip_names(zip: &mut ZipArchive<fs::File>) -> Vec<String> {
    let mut names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
        .collect();
    names.sort();
    names
}

fn zip_bytes(zip: &mut ZipArchive<fs::File>, name: &str) -> Vec<u8> {
    let mut entry = zip
        .by_name(name)
        .unwrap_or_else(|e| panic!("{name} missing from the zip: {e}"));
    let mut out = Vec::new();
    entry.read_to_end(&mut out).unwrap();
    out
}

fn manifest_of(dest: &Path) -> serde_json::Value {
    let bytes = zip_bytes(&mut archive(dest), MANIFEST_NAME);
    serde_json::from_slice(&bytes).unwrap()
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

/// Every file under `dir` as paths relative to it, `/`-separated, sorted.
fn tree(dir: &Path, rel: &str) -> Vec<String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        let path = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if entry.path().is_dir() {
            out.extend(tree(&entry.path(), &path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn the_zip_holds_the_three_parts() {
    let (conn, root, dir) = fixture("parts");
    let dest = dir.join("export.zip");
    export::export(&conn, &root, &dest, APP_VERSION).unwrap();

    let mut zip = archive(&dest);
    let names = zip_names(&mut zip);
    assert!(names.contains(&MANIFEST_NAME.to_string()));
    assert!(names.contains(&DB_NAME.to_string()));
    let csv_entries: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with(&format!("{CSV_DIR}/")))
        .cloned()
        .collect();
    let expected: Vec<String> = tree(&root, "")
        .into_iter()
        .map(|rel| format!("{CSV_DIR}/{rel}"))
        .collect();
    assert_eq!(csv_entries, expected, "the csv/ tree is zipped as-is");

    // Every zipped CSV is byte-identical to the file on disk.
    for rel in tree(&root, "") {
        let on_disk = fs::read(root.join(&rel)).unwrap();
        assert_eq!(
            zip_bytes(&mut zip, &format!("{CSV_DIR}/{rel}")),
            on_disk,
            "{rel} is copied as-is"
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn every_manifest_hash_and_row_count_matches_the_zipped_file() {
    let (conn, root, dir) = fixture("hashes");
    let dest = dir.join("export.zip");
    export::export(&conn, &root, &dest, APP_VERSION).unwrap();

    let manifest = manifest_of(&dest);
    let files = manifest["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "one entry per year CSV");
    let mut zip = archive(&dest);
    for entry in files {
        let path = entry["path"].as_str().unwrap();
        let bytes = zip_bytes(&mut zip, path);
        assert_eq!(
            entry["sha256"].as_str().unwrap(),
            sha256_hex(&bytes),
            "SHA-256 of the exact bytes in the zip"
        );
        let rows = read_raw_file(&bytes).unwrap();
        assert_eq!(
            entry["row_count"].as_u64().unwrap() as usize,
            rows.records.len(),
            "row count of the file itself"
        );
    }
    assert_eq!(
        manifest["database"]["sha256"].as_str().unwrap(),
        sha256_hex(&zip_bytes(&mut zip, DB_NAME)),
        "the database file's SHA-256"
    );

    // The account fields come from the database.
    let accounts = list_accounts(&conn).unwrap();
    for entry in files {
        let account = accounts
            .iter()
            .find(|a| a.account_key.0 == entry["account_key"].as_str().unwrap())
            .expect("the entry names a real account_key");
        assert_eq!(entry["name"].as_str().unwrap(), account.name);
        assert_eq!(
            entry["institution"].as_str().unwrap(),
            account.institution.key
        );
        assert_eq!(entry["kind"].as_str().unwrap(), account.kind.as_str());
        assert!(entry["year"].as_i64().unwrap() >= 2000);
    }

    // The snapshot is a database that opens, at the current schema, with
    // the same transactions as the source.
    let snap = dir.join("snapshot-check.sqlite");
    fs::write(&snap, zip_bytes(&mut zip, DB_NAME)).unwrap();
    let restored = Connection::open(&snap).unwrap();
    let version: u32 = restored
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    let count: i64 = restored
        .query_row("SELECT count(*) FROM transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn manifest_records_versions_and_export_time() {
    let (conn, root, dir) = fixture("versions");
    let dest = dir.join("export.zip");
    export::export(&conn, &root, &dest, APP_VERSION).unwrap();

    let manifest = manifest_of(&dest);
    assert_eq!(
        manifest["format_version"].as_u64(),
        Some(FORMAT_VERSION as u64),
        "an integer format_version, never the version string"
    );
    assert_eq!(manifest["financi_version"].as_str().unwrap(), APP_VERSION);
    assert_eq!(
        manifest["schema_version"].as_u64().unwrap() as u32,
        SCHEMA_VERSION
    );
    let exported_at = manifest["exported_at"].as_str().unwrap();
    chrono::DateTime::parse_from_rfc3339(exported_at)
        .unwrap_or_else(|e| panic!("exported_at {exported_at:?}: {e}"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_missing_year_file_is_rebuilt_by_the_self_check_before_zipping() {
    let (conn, root, dir) = fixture("rebuild");
    let key = list_accounts(&conn).unwrap()[0].account_key.0.clone();
    let file = root.join("amex").join(&key).join("2025.csv");
    let original = fs::read(&file).unwrap();
    fs::remove_file(&file).unwrap();

    let dest = dir.join("export.zip");
    export::export(&conn, &root, &dest, APP_VERSION).unwrap();

    let mut zip = archive(&dest);
    let zipped = zip_bytes(&mut zip, &format!("{CSV_DIR}/amex/{key}/2025.csv"));
    assert_eq!(zipped, original, "the zip holds the rebuilt year file");
    assert!(file.exists(), "the self-check also restored it on disk");
    let manifest = manifest_of(&dest);
    let entry = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["year"] == serde_json::json!(2025))
        .expect("the rebuilt year has a manifest entry");
    assert_eq!(entry["sha256"].as_str().unwrap(), sha256_hex(&zipped));
    assert_eq!(entry["row_count"].as_u64().unwrap(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hidden_rows_stay_in_the_zipped_csvs() {
    let (conn, root, dir) = fixture("hidden");
    conn.execute(
        "UPDATE transactions SET hidden = 1 WHERE description_raw = 'HOTEL'",
        [],
    )
    .unwrap();

    let dest = dir.join("export.zip");
    export::export(&conn, &root, &dest, APP_VERSION).unwrap();

    let manifest = manifest_of(&dest);
    let key = list_accounts(&conn).unwrap()[0].account_key.0.clone();
    let entry = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["year"] == serde_json::json!(2025))
        .unwrap();
    assert_eq!(
        entry["row_count"].as_u64().unwrap(),
        1,
        "the hidden row counts"
    );
    let mut zip = archive(&dest);
    let bytes = zip_bytes(&mut zip, &format!("{CSV_DIR}/amex/{key}/2025.csv"));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("HOTEL"), "{text}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn variant_year_files_get_their_own_entries() {
    let (mut conn, root, dir) = fixture("variants");
    let acct = list_accounts(&conn).unwrap()[0].id;
    // Same parser, different raw header: the rows land in `2026-v2.csv`.
    let v2 = read_raw_file(format!("{AMEX_HEADER} \n04 Nov 2026,04 Nov 2026,TIP,3.00").as_bytes())
        .unwrap();
    commit(&mut conn, acct, "amex", &v2, "v2.csv");
    rewrite_years(&conn, &root, acct, &[2026]).unwrap();

    let dest = dir.join("export.zip");
    export::export(&conn, &root, &dest, APP_VERSION).unwrap();

    let key = list_accounts(&conn).unwrap()[0].account_key.0.clone();
    let mut zip = archive(&dest);
    let manifest = manifest_of(&dest);
    let entries: Vec<&serde_json::Value> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["year"] == serde_json::json!(2026))
        .collect();
    assert_eq!(entries.len(), 2, "one entry per variant file");
    for name in ["2026.csv", "2026-v2.csv"] {
        let entry = entries
            .iter()
            .find(|e| e["path"].as_str().unwrap().ends_with(name))
            .unwrap_or_else(|| panic!("no manifest entry for {name}"));
        let bytes = zip_bytes(&mut zip, &format!("{CSV_DIR}/amex/{key}/{name}"));
        assert_eq!(
            entry["sha256"].as_str().unwrap(),
            sha256_hex(&bytes),
            "{name}"
        );
        assert_eq!(entry["row_count"].as_u64().unwrap(), 1, "{name}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_in_memory_database_exports_too() {
    let mut conn = open_in_memory().unwrap();
    let acct = create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    commit(
        &mut conn,
        acct,
        "amex",
        &amex_file(&["05 Nov 2026,05 Nov 2026,COFFEE,5.00"]),
        "one.csv",
    );
    let root = temp_dir("memory-csv");
    rewrite_years(&conn, &root, acct, &[2026]).unwrap();
    let dir = temp_dir("memory");
    let dest = dir.join("export.zip");

    export::export(&conn, &root, &dest, APP_VERSION).unwrap();

    let mut zip = archive(&dest);
    let names = zip_names(&mut zip);
    assert!(names.contains(&MANIFEST_NAME.to_string()));
    assert!(names.contains(&DB_NAME.to_string()));
    assert!(names.iter().any(|n| n.starts_with("csv/")));
    let manifest = manifest_of(&dest);
    assert_eq!(manifest["files"].as_array().unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(dir).unwrap();
}
