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
use std::io::Read;
use std::path::{Path, PathBuf};

use financi_core::{AccountKind, CategoryKind, read_raw_file};
use financi_db::categories::{create_child, create_parent};
use financi_db::export::{self, CSV_DIR, MANIFEST_NAME};
use financi_db::import::{CommitOptions, ImportOutcome, commit_import};
use financi_db::master_csv::{rewrite_years, root_for_db};
use financi_db::overrides::{set_category, set_hidden};
use financi_db::rusqlite::{self, Connection};
use financi_db::{add_rule, create_account, open, settings};
use zip::ZipArchive;

const APP_VERSION: &str = "v9.9.9-test";

/// A non-default `dashboard.layout` value (3.101); the setting is opaque
/// JSON to the database, so any string the app would save will do.
const LAYOUT_JSON: &str = r#"{"version":1,"dock":{"mode":"bottom","height":0.42}}"#;

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
    let amex = create_account(&conn, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let simplii =
        create_account(&conn, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();
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

/// One entry of the zip, decompressed.
fn zip_bytes(zip_path: &Path, name: &str) -> Vec<u8> {
    let mut zip = ZipArchive::new(fs::File::open(zip_path).unwrap()).unwrap();
    let mut entry = zip
        .by_name(name)
        .unwrap_or_else(|e| panic!("{name} missing from the zip: {e}"));
    let mut out = Vec::new();
    entry.read_to_end(&mut out).unwrap();
    out
}

fn manifest_of(zip_path: &Path) -> serde_json::Value {
    serde_json::from_slice(&zip_bytes(zip_path, MANIFEST_NAME)).unwrap()
}

/// The zip's `csv/` tree keyed relative to `csv/` — the same keys as
/// `tree_bytes` gives for a master CSV root.
fn zip_csv_tree(zip_path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(fs::File::open(zip_path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let Some(rel) = name.strip_prefix(&format!("{CSV_DIR}/")).map(str::to_owned) else {
            continue;
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        out.insert(rel, bytes);
    }
    out
}

/// Create an account with an explicit `account_key`. `create_account`
/// always mints a fresh UUID; the round-trip guarantee needs the fresh
/// database's accounts to carry the manifest's keys, because the master
/// CSVs live under `<institution>/<account_key>/`.
fn create_account_with_key(
    conn: &Connection,
    key: &str,
    institution: &str,
    name: &str,
    kind: AccountKind,
) -> i64 {
    conn.execute(
        "INSERT INTO accounts (account_key, institution_id, name, kind)
         SELECT ?1, id, ?3, ?4 FROM institutions WHERE key = ?2",
        (key, institution, name, kind.as_str()),
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A whole query, sorted — table snapshots compared across databases.
fn dump<T: Ord>(
    conn: &Connection,
    sql: &str,
    mut row: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>,
) -> Vec<T> {
    let mut rows: Vec<T> = conn
        .prepare(sql)
        .unwrap()
        .query_map([], |r| row(r))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.sort();
    rows
}

/// Every transaction as `(account_key, row_hash, raw_bytes)`: the
/// round-trip guarantee's multiset with the exact row bytes attached.
fn row_fingerprints(conn: &Connection) -> Vec<(String, String, Vec<u8>)> {
    dump(
        conn,
        "SELECT a.account_key, t.row_hash, t.raw_bytes
           FROM transactions t JOIN accounts a ON a.id = t.account_id",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

/// What a restore must carry per row: category, manual lock, hidden flag.
fn row_state(conn: &Connection) -> Vec<(String, String, String, i64, i64)> {
    dump(
        conn,
        "SELECT a.account_key, t.row_hash, COALESCE(c.name, ''),
                t.category_locked, t.hidden
           FROM transactions t
           JOIN accounts a ON a.id = t.account_id
           LEFT JOIN categories c ON c.id = t.category_id",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
}

fn account_state(conn: &Connection) -> Vec<(String, String, String, String)> {
    dump(
        conn,
        "SELECT a.account_key, i.key, a.name, a.kind
           FROM accounts a JOIN institutions i ON i.id = a.institution_id",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
}

fn category_state(conn: &Connection) -> Vec<(String, String, String)> {
    dump(
        conn,
        "SELECT COALESCE(p.name, ''), c.name, c.kind
           FROM categories c LEFT JOIN categories p ON p.id = c.parent_id",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

fn rule_state(conn: &Connection) -> Vec<(String, i64, String)> {
    dump(
        conn,
        "SELECT r.pattern, r.is_regex, c.name
           FROM category_rules r JOIN categories c ON c.id = r.category_id",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

fn settings_state(conn: &Connection) -> Vec<(String, String)> {
    dump(conn, "SELECT key, value FROM settings", |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
}

#[test]
fn the_export_round_trip_is_byte_exact() {
    // -- the source database: the demo data, one hidden row, one manually
    // -- categorized (and so locked) row, a rule and a saved layout.
    let dir = temp_dir("round-trip");
    let src_db = dir.join("source/financi.sqlite");
    fs::create_dir_all(src_db.parent().unwrap()).unwrap();
    let mut src = open(&src_db).unwrap();
    let src_root = root_for_db(&src_db);
    let amex = create_account(&src, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let simplii =
        create_account(&src, "simplii", "No Fee Chequing", AccountKind::Chequing).unwrap();
    import_demo(&mut src, &src_root, amex, simplii);

    let hidden_id: i64 = src
        .query_row(
            "SELECT id FROM transactions WHERE description_raw LIKE '%STREAMFLIX%' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    set_hidden(&src, hidden_id, true).unwrap();
    let coffee = create_child(
        &src,
        create_parent(&src, "Acceptance", CategoryKind::Expense).unwrap(),
        "Coffee",
    )
    .unwrap();
    let locked_id: i64 = src
        .query_row(
            "SELECT id FROM transactions WHERE description_raw LIKE '%EVERYTHING STORE%' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    set_category(&src, locked_id, coffee).unwrap();
    add_rule(&src, "ZIPRIDE", false, coffee).unwrap();
    settings::set_dashboard_layout(&src, LAYOUT_JSON).unwrap();

    let zip = dir.join("export.zip");
    export::export(&src, &src_root, &zip, APP_VERSION).unwrap();
    let manifest = manifest_of(&zip);
    let files = manifest["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "one year CSV per demo account");

    // -- a fresh database whose accounts reuse the manifest's account
    // -- keys, rebuilt by importing the zip's CSVs.
    let fresh_db = dir.join("fresh/financi.sqlite");
    fs::create_dir_all(fresh_db.parent().unwrap()).unwrap();
    let mut fresh = open(&fresh_db).unwrap();
    let fresh_root = root_for_db(&fresh_db);
    let mut by_key: BTreeMap<String, i64> = BTreeMap::new();
    for entry in files {
        let key = entry["account_key"].as_str().unwrap();
        by_key.entry(key.to_owned()).or_insert_with(|| {
            create_account_with_key(
                &fresh,
                key,
                entry["institution"].as_str().unwrap(),
                entry["name"].as_str().unwrap(),
                AccountKind::parse(entry["kind"].as_str().unwrap()).unwrap(),
            )
        });
    }
    let mut fresh_years: BTreeMap<i64, BTreeSet<i32>> = BTreeMap::new();
    for entry in files {
        let path = entry["path"].as_str().unwrap();
        let account = by_key[entry["account_key"].as_str().unwrap()];
        let outcome = import_file(
            &mut fresh,
            account,
            entry["institution"].as_str().unwrap(),
            &zip_bytes(&zip, path),
            path.rsplit('/').next().unwrap(),
        );
        assert_eq!(
            outcome.inserted as u64,
            entry["row_count"].as_u64().unwrap(),
            "{path}: every row is new to the fresh database"
        );
        fresh_years
            .entry(account)
            .or_default()
            .insert(entry["year"].as_i64().unwrap() as i32);
    }
    for (account, years) in &fresh_years {
        rewrite_years(
            &fresh,
            &fresh_root,
            *account,
            &years.iter().copied().collect::<Vec<_>>(),
        )
        .unwrap();
    }

    // Same multiset of (account_key, row_hash) — with the row bytes equal.
    assert_eq!(row_fingerprints(&fresh), row_fingerprints(&src));

    // And the fresh database's master CSVs are byte-identical to the zip's.
    assert_eq!(tree_bytes(&fresh_root), zip_csv_tree(&zip));

    // -- re-importing the zip's CSVs into the source database adds 0 rows.
    let src_rows = count(&src, "SELECT count(*) FROM transactions");
    for entry in files {
        let path = entry["path"].as_str().unwrap();
        let account: i64 = src
            .query_row(
                "SELECT id FROM accounts WHERE account_key = ?1",
                [entry["account_key"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let outcome = import_file(
            &mut src,
            account,
            entry["institution"].as_str().unwrap(),
            &zip_bytes(&zip, path),
            path.rsplit('/').next().unwrap(),
        );
        assert_eq!(
            (outcome.import_id, outcome.inserted, outcome.replaced),
            (None, 0, 0),
            "re-importing {path} into the source adds 0 rows"
        );
    }
    assert_eq!(count(&src, "SELECT count(*) FROM transactions"), src_rows);

    // -- restoring the zip into another database (dirty: its own account,
    // -- an old layout, a stray CSV) gives an identical database and csv/.
    let res_db = dir.join("restored/financi.sqlite");
    fs::create_dir_all(res_db.parent().unwrap()).unwrap();
    let mut restored = open(&res_db).unwrap();
    let restored_root = root_for_db(&res_db);
    create_account(&restored, "amex", "Junk", AccountKind::CreditCard).unwrap();
    settings::set_dashboard_layout(&restored, r#"{"version":1,"dock":{"mode":"hidden"}}""#)
        .unwrap();
    fs::create_dir_all(&restored_root).unwrap();
    fs::write(restored_root.join("stray.txt"), b"the old tree").unwrap();

    let restored_manifest = export::restore(&mut restored, &restored_root, &zip).unwrap();
    assert_eq!(restored_manifest.files.len(), files.len());

    assert_eq!(account_state(&restored), account_state(&src));
    assert_eq!(row_state(&restored), row_state(&src));
    assert_eq!(category_state(&restored), category_state(&src));
    assert_eq!(rule_state(&restored), rule_state(&src));
    assert_eq!(settings_state(&restored), settings_state(&src));
    assert_eq!(
        count(
            &restored,
            "SELECT count(*) FROM transactions WHERE hidden = 1"
        ),
        1,
        "the hidden row comes back hidden"
    );
    assert_eq!(
        count(
            &restored,
            "SELECT count(*) FROM transactions
              WHERE category_locked = 1
                AND category_id = (SELECT id FROM categories WHERE name = 'Coffee')",
        ),
        1,
        "the manually categorized row comes back locked"
    );
    assert_eq!(
        settings::dashboard_layout(&restored).unwrap().as_deref(),
        Some(LAYOUT_JSON),
        "3.101: the saved dashboard layout comes back"
    );
    assert_eq!(tree_bytes(&restored_root), zip_csv_tree(&zip));
    assert!(
        !restored_root.join("stray.txt").exists(),
        "the old tree is replaced, not merged"
    );

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_restored_database_brings_back_the_saved_dashboard_layout() {
    // 3.101: export with a non-default `dashboard.layout`, restore into a
    // fresh database that has a different one saved, and the exported
    // layout wins — the setting lives in `financi.sqlite` and travels with
    // the snapshot.
    let dir = temp_dir("layout");
    let src_db = dir.join("source/financi.sqlite");
    fs::create_dir_all(src_db.parent().unwrap()).unwrap();
    let mut src = open(&src_db).unwrap();
    let src_root = root_for_db(&src_db);
    let amex = create_account(&src, "amex", "Cobalt", AccountKind::CreditCard).unwrap();
    let bytes = fs::read(demo_dir().join("amex-sep-nov.csv")).unwrap();
    import_file(&mut src, amex, "amex", &bytes, "amex-sep-nov.csv");
    rewrite_years(&src, &src_root, amex, &[2026]).unwrap();
    settings::set_dashboard_layout(&src, LAYOUT_JSON).unwrap();

    let zip = dir.join("export.zip");
    export::export(&src, &src_root, &zip, APP_VERSION).unwrap();

    let res_db = dir.join("restored/financi.sqlite");
    fs::create_dir_all(res_db.parent().unwrap()).unwrap();
    let mut restored = open(&res_db).unwrap();
    let restored_root = root_for_db(&res_db);
    settings::set_dashboard_layout(&restored, r#"{"version":1,"dock":{"mode":"right"}}""#).unwrap();
    fs::create_dir_all(&restored_root).unwrap();
    export::restore(&mut restored, &restored_root, &zip).unwrap();

    assert_eq!(
        settings::dashboard_layout(&restored).unwrap().as_deref(),
        Some(LAYOUT_JSON),
        "the restored database brings back the saved layout"
    );

    fs::remove_dir_all(dir).unwrap();
}
