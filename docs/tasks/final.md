# Final — performance at scale (last of all)

Runs after M5, when every other task is done (user decision, 2026-11-11).

- [ ] **3.55 Fast at real scale: hundreds of rules, dozens of
  categories.** The user has hundreds of rules and dozens of categories
  and is worried about performance (2026-11-11). The dev seed (18 rules,
  458 rows) hides scale problems. `CompiledRules::pick` loops over **every
  rule for every row**, and a rule change runs that loop twice: once in
  `reapply_rules`, and again in `rule_use_stats` (3.58) inside `refresh`. At
  1 000 rules × 100 000 rows (10 years at the user's ~10 000 a year)
  that's ~100 M checks per pass, likely around a
  second or more. The live preview (`count_rule_matches`) scans every row
  on each keystroke. **Runs last** (user decision, 2026-11-11): after M5 and every other
  task. Earlier tasks measure on the dev seed and a throwaway local seed;
  the committed scale seed and the timed budgets land here.
  - **Review update (2026-11-11):** Added 2026-11-11 (review): **table paging** — `list_transactions_filtered` has no `LIMIT`, so "All time" loads every row on each change; page the table with `TableDelegate::has_more` / `load_more` plus a `COUNT(*)`. Timed tests for pairing at 100 000 rows (3.73) and the reader connection (3.77). `RegexSet::new` fails as a whole when one pattern or the set exceeds the size limit: fall back to per-regex matching for the patterns that don't fit, test with 200 `\w+`-heavy rules, and keep 3.72's Unicode case folding and `chars().count()` lengths.
  - **Scale fixture:** a `seed_large` (dev-seed feature) and a
    `FINANCI_DEV_SEED=large` switch. It builds **1 000 rules** (80 %
    substring, 20 % regex, overlapping on purpose), **80 categories**
    (15 parents), and **100 000 transactions** over 10 years (the user has ~1 800 today
    and expects ~10 000 a year) with realistic
    merchant strings, all deterministic. It uses only synthetic data.
  - **Budgets** (release build, on the large seed), as `#[ignore]` perf
    tests runnable with `cargo test --release -- --ignored perf_`, which
    print their times:
    - `reapply_rules` < 150 ms;
    - `rule_use_stats` < 150 ms;
    - `count_rule_matches` < 30 ms;
    - the full `refresh` query set < 250 ms;
    - main-thread apply of a refresh result < 16 ms;
    - opening the Categories and Rules pages < 16 ms per frame;
    - a dashboard filter change shows its result within 100 ms (from
      3.56);
    - no frame over 16 ms while hovering, scrolling or typing, on the
      one-year seed (from 3.57);
    - the Rules page with 1 000 rules scrolls and filters under 16 ms a
      frame (from 3.58).

    Debug-build versions of the DB budgets (×10) run in the normal test
    suite on a 10 000-row / 300-rule seed (one year) so regressions fail CI.
  - **Matching in one pass per row:** in `CompiledRules`, replace the
    per-rule loop with an **Aho-Corasick** automaton over the lowercased
    substring patterns (`aho-corasick` crate, already in the tree via
    `regex`; `MatchKind::Standard` with overlapping search to collect
    every match) and one **`RegexSet`** for the regex rules (case-
    insensitive). Each row is scanned once per kind; the winner is the
    longest match, ties to the lowest id, substring before regex, exactly
    as today. Invalid regexes stay excluded. The existing core tests are
    the contract; add a property test that the new `pick` equals the old
    per-rule loop on random rules and descriptions.
  - **One pass per rule change:** have `reapply_rules` return the per-rule
    use counts it computed, and cache them on the shell until the next
    rule or import change, so `refresh` doesn't recompute them. If that
    isn't enough, store the deciding rule: add a nullable
    `transactions.category_rule_id` column in a new migration, set by
    `reapply_rules` and at import. Uses then becomes a `GROUP BY`, and
    3.52's "decides" set becomes a query. Only add the column if the
    budgets need it.
  - **Live preview:** debounce `preview_rule_match` by 150 ms, and keep
    the in-flight task cancellable, as now.
  - **UI at scale:** with 1 000 rules and 80 categories, check that the
    rule lists, tree and pickers stay virtual or lazy (3.12, 3.19, 3.21).
    Category pickers with 65 children stay usable (3.59 made
    them searchable; check it holds at this size).
  - Commit message: the before / after numbers for every budget.
