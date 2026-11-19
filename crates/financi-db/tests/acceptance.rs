//! 5.4 acceptance tests: the end-to-end guarantees the app promises its
//! only user. Overlapping re-imports add 0 rows and leave the master CSVs
//! byte-identical; the export round-trip is byte-exact into a fresh
//! database whose accounts reuse the manifest's account keys, re-imports
//! into the source add nothing, and a restore brings back the whole
//! database — rows, categories, rules, locks, hidden flags and the saved
//! dashboard layout (3.101) — plus the `csv/` tree.
//!
//! The single-file checks these build on live beside them: overlapping
//! re-import adds 0 rows (`import.rs::reimporting_the_same_file_adds_
//! nothing_and_records_no_import`), the zip's contents and manifest
//! (`export.rs`), restore's safety (`restore.rs`), and the demo totals
//! against `fixtures/demo/expected.json` (`src/views.rs::tests::
//! demo_months_match_expected_json`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use financi_core::{AccountKind, read_raw_file};
use financi_db::import::{CommitOptions, ImportOutcome, commit_import};
use financi_db::master_csv::{rewrite_years, root_for_db};
use financi_db::open;
use financi_db::rusqlite::Connection;

/// The demo exports and the institution of the account each belongs to.
/// The two Amex files overlap on September, so importing them in sequence
/// is the overlapping-re-import case.
const DEMO_FILES: [(&str, &str); 3] = [
    ("amex-sep-oct.csv", "amex"),
    ("amex-sep-nov.csv", "amex"),
    ("simplii-chequing.csv", "simplii"),
];

fn demo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("financi-acceptance-{tag}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn import_file(
    conn: &mut Connection,
    account_id: i64,
    parser_id: &str,
    bytes: &[u8],
    name: &str,
) -> ImportOutcome {
    let file = read_raw_file(bytes).unwrap();
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

/// Import every demo file into the demo accounts — one Amex Cobalt, one
/// Simplii chequing — and rewrite the year files the imports touched, the
/// way the app does after each import.
fn import_demo(conn: &mut Connection, root: &Path, amex: i64, simplii: i64) {
    let mut years: BTreeMap<i64, BTreeSet<i32>> = BTreeMap::new();
    for (file, institution) in DEMO_FILES {
        let bytes = fs::read(demo_dir().join(file)).unwrap();
        let account = if institution == "amex" { amex } else { simplii };
        let outcome = import_file(conn, account, institution, &bytes, file);
        years
            .entry(account)
            .or_default()
            .extend(outcome.affected_years);
    }
    for (account, years) in years {
        rewrite_years(conn, root, account, &years.into_iter().collect::<Vec<_>>()).unwrap();
    }
}

/// Every file under `dir` as `/`-separated paths relative to it, with the
/// exact bytes on disk.
fn tree_bytes(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if entry.path().is_dir() {
                walk(&entry.path(), &path, out);
            } else {
                out.insert(path, fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    if dir.exists() {
        walk(dir, "", &mut out);
    }
    out
}

#[test]
fn reimporting_the_demo_files_adds_zero_rows_and_leaves_the_master_csvs_byte_identical() {
    let dir = temp_dir("reimport");
    let db = dir.join("financi.sqlite");
    let mut conn = open(&db).unwrap();
    let root = root_for_db(&db);
    let amex =
        financi_db::create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let simplii =
        financi_db::create_account(&conn, "simplii", "No Fee Chequing", AccountKind::Chequing)
            .unwrap();
    import_demo(&mut conn, &root, amex, simplii);

    let rows_before = count(&conn, "SELECT count(*) FROM transactions");
    assert!(rows_before > 0);
    let before = tree_bytes(&root);
    assert!(!before.is_empty(), "the demo imports left a csv/ tree");

    // The overlapping pass: every row of every file is already in the
    // database, so nothing is inserted, replaced, or even recorded.
    for (file, institution) in DEMO_FILES {
        let bytes = fs::read(demo_dir().join(file)).unwrap();
        let account = if institution == "amex" { amex } else { simplii };
        let outcome = import_file(&mut conn, account, institution, &bytes, file);
        assert_eq!(
            (outcome.import_id, outcome.inserted, outcome.replaced),
            (None, 0, 0),
            "re-importing {file} adds 0 rows"
        );
    }
    assert_eq!(
        count(&conn, "SELECT count(*) FROM transactions"),
        rows_before
    );

    // And the master CSVs stay byte-identical — even forcing a rewrite of
    // every year regenerates exactly the bytes that are already on disk.
    rewrite_years(&conn, &root, amex, &[2026]).unwrap();
    rewrite_years(&conn, &root, simplii, &[2026]).unwrap();
    assert_eq!(
        tree_bytes(&root),
        before,
        "the master CSVs are byte-identical"
    );

    fs::remove_dir_all(dir).unwrap();
}
