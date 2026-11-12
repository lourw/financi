//! Row hashes are sacred (CLAUDE.md: they use the raw exported strings), so
//! every fixture's hashes are pinned here. The digests for the fixtures that
//! existed before task 3.67 were captured *before* its CSV-reading changes
//! and must never move; the newer fixtures are pinned to lock in behaviour.
//! A digest covers every record's `row_hash` in order, so it detects any
//! change to splitting, decoding or hashing.

use financi_core::{AccountKey, read_raw_file};
use sha2::{Digest, Sha256};

fn key() -> AccountKey {
    AccountKey("00000000-0000-4000-8000-0000000000f1".into())
}

fn digest(bytes: &[u8]) -> (usize, String) {
    let file = read_raw_file(bytes).expect("fixture parses");
    let parser = financi_parsers::detect(&file.header).expect("fixture detected");
    let mut hasher = Sha256::new();
    for r in &file.records {
        hasher.update(parser.row_hash(&key(), r).0.as_bytes());
        hasher.update(b"\n");
    }
    (file.records.len(), format!("{:x}", hasher.finalize()))
}

/// `<fixture path> <record count> <digest>` — one line per fixture.
const PINNED: &[&str] = &[
    // demo exports (captured 2026-11-12, before 3.67)
    "../../fixtures/demo/amex-sep-nov.csv 102 226ffc98faa640e119b957e79d4d29f76f346fbc7e0b58a3cd6f0e64cb3bdd10",
    "../../fixtures/demo/amex-sep-oct.csv 68 d40c8e90b69a77ac128fba598b631ed6e2590b7c2a67626d2780dbbd029192ea",
    "../../fixtures/demo/simplii-chequing.csv 11 ae038dee41349ca95ee8da6941f0ae9ba7eae3d14eaa562a9f27daae4c3675c9",
    // parser fixtures (captured 2026-11-12, before 3.67)
    "tests/fixtures/amex/bad_date.csv 2 3824999f7880bc909f9f5ff762734bf39e202239e27aceecac00fd60cf2c8323",
    "tests/fixtures/amex/basic.csv 5 d10213932ebf72a8a060da8dfa085f8a4461093821723e49fa4c30993099831a",
    "tests/fixtures/amex/overlap.csv 4 23ce42bb5b1a5c9670cb59a5c180e056c556070b886f295614a107205113341e",
    "tests/fixtures/simplii/basic.csv 6 9690885e16c6b9cc49a86ab7aabe69694eac81b95631a2030fcacda7c0baa87e",
    "tests/fixtures/simplii/both_amounts.csv 1 637060e39c3fdfbd0a1cb723d9189fb4bcdde5dbe7894120a2a783abfbde9ebd",
    "tests/fixtures/simplii/visa.csv 7 f85b0402a124288355daa210dbb19ce7dc2c681eeb3932fdbe0e41d544b04e99",
    // added by 3.67
    "tests/fixtures/amex/crlf_bom.csv 3 dfed655661e31cee45fca8889d781682f58cc35e51f255b4a85d5ce6e74aae78",
    "tests/fixtures/amex/stray_quotes.csv 3 591ff10184e7bec701d3fae36dda74c2302b2e98dd98b2a9bc1bbf8e15e5f807",
    "tests/fixtures/simplii/windows1252.csv 2 05bbbfeaa5a55020741488765205430f64dc8a61a6ae71a6a686573e43d6f66b",
];

#[test]
fn every_fixture_hashes_to_its_pinned_digest() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut seen = std::collections::BTreeSet::new();
    for pinned in PINNED {
        let mut parts = pinned.split(' ');
        let rel = parts.next().unwrap();
        let count: usize = parts.next().unwrap().parse().unwrap();
        let want = parts.next().unwrap();
        seen.insert(rel);
        let bytes = std::fs::read(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let (got_count, got) = digest(&bytes);
        assert_eq!(got_count, count, "{rel}: record count changed");
        if want == "TBD" {
            println!("PIN {rel} {got_count} {got}");
        } else {
            assert_eq!(got.as_str(), want, "{rel}: row hashes changed");
        }
    }
    // no fixture may escape pinning
    for dir in [
        root.join("tests/fixtures/amex"),
        root.join("tests/fixtures/simplii"),
        root.join("../../fixtures/demo"),
    ] {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "csv") {
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                assert!(seen.contains(rel.as_str()), "{rel} is not pinned");
            }
        }
    }
}
