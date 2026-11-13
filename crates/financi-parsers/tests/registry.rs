//! Parser registry guard rails (3.90): unique ids, at least one account
//! kind per parser, and an `institutions` row for every institution. A new
//! parser that breaks one of these turns the app's import flow into an
//! error or a panic, so catch it here instead.

use std::collections::HashSet;

#[test]
fn registry_is_consistent() {
    let parsers = financi_parsers::all();
    assert!(!parsers.is_empty(), "the registry must have parsers");

    let mut ids: HashSet<&str> = HashSet::new();
    let mut institutions: HashSet<&str> = HashSet::new();
    for p in &parsers {
        assert!(ids.insert(p.id()), "duplicate parser id {:?}", p.id());
        assert!(
            !p.account_kinds().is_empty(),
            "parser {:?} has no account kinds: new_account_defaults \
             would have nothing to pick",
            p.id()
        );
        institutions.insert(p.institution());
    }

    // Every parser's institution must have a row in the institutions table
    // (seeded by 0001), or create_account rejects the import preview's
    // new-account defaults.
    let conn = financi_db::open_in_memory().expect("in-memory DB");
    let keys: HashSet<String> = conn
        .prepare("SELECT key FROM institutions")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
        .into_iter()
        .collect();
    for institution in institutions {
        assert!(
            keys.contains(institution),
            "parser institution {institution:?} has no row in the institutions table",
        );
    }
}
