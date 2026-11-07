# financi — implementation tasks (M2 → M5)

This is the working checklist for the remaining v1 milestones. It turns
`plan.md` (the requirements, copied from the plan doc) into concrete tasks.
Where the two disagree, **this file wins** for M2–M5 detail. Older parts of
`plan.md` (the Architecture table's `rust_decimal`/`csv`/`serde`/`refinery`,
"GPUI has no chart library", `uniform_list`) are out of date.

## How to work

1. Do **one milestone at a time**. When every box in it is ticked, stop and
   report to the user; wait for a go-ahead before the next milestone.
2. Work the tasks in order. Tick a box (`- [x]`) in the same commit that
   completes it.
3. **Ask before deviating.** If a task is unclear, an API doesn't fit, or you
   would have to guess at behaviour, stop and ask. Don't improvise.
4. Every task lands with tests and synthetic data (see `CLAUDE.md`). Never
   put real bank data in the repo.
5. Commit after each logical piece (usually one task), Conventional Commits,
   then verify that commit in a clean checkout (see `CLAUDE.md`).

## Where things are (state after M1, commit 215c87d)

| Path | What |
|---|---|
| `crates/financi-core` | `RawFile`/`RawRecord`, `read_raw_file`, `parse::{cents, date_*, merchant}`, `AccountKind`, `CategoryKind`, `StatementParser` trait, `row_hash` |
| `crates/financi-parsers` | `amex`, `simplii_chequing`; `all()`, `detect()`, `by_id()` |
| `crates/financi-db` | `migrations/0001_init.sql`, `accounts`, `import` (plan/commit/undo/list), `worker::DbWorker`, `seed` (feature `dev-seed`) |
| `crates/financi/src/flow.rs` | GPUI-free import-flow helpers (tested) |
| `crates/financi/src/ui.rs` | `Shell`, `ImportPanel`, `TxTable` |
| `crates/financi/src/ui_tests.rs` | headless UI tests (pattern to copy) |
| `fixtures/demo/` | synthetic exports + `expected.json` reference totals |
| `spikes/gpui-spike` | sandbox for UI experiments; keep, don't ship |
| `docs/ui-conventions.md` | layout, spacing, colors, formats; read before any UI task |

## GPUI / gpui-kit pitfalls (already hit — don't rediscover)

- Pinned: `gpui-kit =0.9.0` on `gpui-pre =0.3.11`, with
  `gpui-pre-platform` feature `runtime_shaders` (no full Xcode needed). Don't
  bump versions without asking.
- Do **not** enable `gpui-kit/test-support`: a Cargo weak-dependency bug pulls
  in `gpui-fast` and the lockfile fails to resolve. UI tests use GPUI's own
  `TestAppContext` (`gpui` dev-dependency with `test-support`).
- In test modules, never `use gpui_kit::*;` — it imports GPUI's `test` macro,
  which shadows `#[test]` and makes `#[gpui::test]` recurse. Import names
  explicitly (see `ui_tests.rs`).
- UI tests talk to a real `DbWorker` thread: call
  `cx.executor().allow_parking()` and poll with the `wait_until` helper.
- `BarChart` has no click handler. `SelectState::set_items` and
  `set_selected_index` need a `&mut Window`, so async updates use
  `cx.spawn_in(window, …)` + `this.update_in(cx, |this, window, cx| …)`.
- All DB work goes through `DbWorker` via the `on_db` helper in `ui.rs`.
- Screenshots: `FINANCI_DB=/tmp/x.sqlite FINANCI_DEV_SEED=1
  FINANCI_DEV_OPEN=fixtures/demo/amex-sep-nov.csv cargo run -p financi`,
  then capture the window by id (`screencapture -o -l<id>`; find the id with
  CoreGraphics `CGWindowListCopyWindowInfo`). Window capture may need the
  user to grant Screen Recording permission.

---

## M2 — Master CSVs, categorize

Schema changes go in a **new** migration `0002_categorize.sql` (0001 is
frozen from here on; real databases may exist).

- [x] **2.1 Migration 0002.**
  - `category_rules`: add `is_regex INTEGER NOT NULL DEFAULT 0`.
  - `categories`: add `is_system INTEGER NOT NULL DEFAULT 0`.
  - Insert the system categories: parent `Transfers` (kind `transfer`,
    `is_system = 1`) with children `Card Payment` and `Between Accounts`
    (kind `transfer`, `is_system = 1`). No other categories by default.
  - Tests: fresh DB has exactly these 3 categories; migrating a DB at
    version 1 that already has transactions keeps them.
  - Update `seed.rs` so its own categories don't collide with the system
    ones (reuse the system Transfers/Card Payment instead of creating them).

- [ ] **2.2 Master CSVs** (`financi-db/src/master_csv.rs`).
  - Design: `plan.md` → *Master CSVs*. Rows are only ever added or
    removed; a row's bytes are never edited.
  - Existing databases: the startup self-check builds the files for data
    imported before this task.
  - Layout: `<db dir>/csv/<institution-key>/<account_key>/<YYYY>.csv`, plus
    `account.json` (name, institution, kind) per account folder, rewritten on
    account rename. The directory follows `FINANCI_DB`.
  - Year = calendar year of the row's parsed transaction date. Rows are the
    stored `raw_bytes`, **including hidden rows**, written with the earliest
    import's header, encoding, BOM, line ending and trailing-newline settings
    for that year. Rows whose import has a different header/encoding/BOM go
    to `<YYYY>-v2.csv`, `-v3`…
  - Order: by posted date (newest first for Amex, oldest first for Simplii —
    add `fn newest_first(&self) -> bool` to the parser trait), then import
    order, then `source_line`.
  - `rewrite_years(conn, root, account_id, years)` regenerates the given year
    files from the database. Called on the `DbWorker` thread after the commit
    of: import and undo import (near-match confirm hooks in at 4.3).
    Atomic write: temp file in the same dir, fsync, rename. Empty year →
    delete the file.
  - Startup self-check: any year file missing, extra, or with a row count
    that doesn't match the database is rewritten; log a notice.
  - Tests: overlapping imports (Sep–Nov then Oct–Dec) give one file whose
    rows equal the DB rows, byte-exact; a Dec–Jan file splits into two year
    files; real identical repeats appear twice; undo removes exactly the
    import's rows (and deletes a now-empty year file); hidden rows stay; a
    header change produces `-v2`; every row in every file equals some
    import's original record bytes; a deleted year file is restored by the
    startup check.

- [ ] **2.3 Category model** (`financi-db/src/categories.rs`).
  - `list_categories(conn) -> Vec<CategoryNode>` (parents with children,
    sorted by name), `create_parent(name, kind)`, `create_child(parent_id,
    name)` (child inherits the parent's kind), `rename(id, name)`,
    `move_child(child_id, new_parent_id)` (only between parents of the same
    kind).
  - `delete_category(id, reassign_to: i64)`: moves that category's
    transactions and rules to `reassign_to` (must be a child), then deletes.
    Deleting a parent requires it to have no children. System categories
    can't be renamed, moved or deleted.
  - Transactions may only be assigned to **children**, never parents.
  - Tests for every rule above, including the refusals.

- [ ] **2.4 Rule engine** (`financi-core` for matching, `financi-db` for
  storage).
  - A rule is `pattern` + `is_regex` + `category_id` (a child).
  - Substring rules: case-insensitive, matched against `description_raw`.
  - Regex rules: Rust `regex` crate, case-insensitive, against
    `description_raw`. Validate on save; reject invalid patterns with a
    readable error.
  - Winner when several match: **substring rules first, longest pattern
    wins**; only if no substring rule matches, regex rules, longest pattern
    wins; ties broken by lowest rule id.
  - Description only. No account or sign filters.
  - Pure function `pick_category(rules, description) -> Option<category_id>`
    with unit tests for each ordering case.

- [ ] **2.5 Applying rules.**
  - At import (`commit_import`): after inserting, categorize the new rows.
  - When a rule is created, edited or deleted: re-run all rules over every
    transaction with `category_locked = 0` (rows matching nothing become
    Uncategorized, i.e. `category_id NULL`).
  - Locked rows (`category_locked = 1`) are never touched by rules.
  - **Built-in card-payment detection runs first** and beats user rules:
    add `fn is_card_payment(&self, t: &RawTransaction) -> bool` to
    `StatementParser` (default `false`). Amex: description starts with
    `PAYMENT RECEIVED`. Matching rows get `Transfers > Card Payment`.
  - Tests: import categorizes; new rule recategorizes unlocked rows only;
    deleting a rule uncategorizes its rows; card payments ignore user rules.

- [ ] **2.6 Manual edits.**
  - `set_category(transaction_id, category_id)` sets the category and
    `category_locked = 1`. `clear_override(id)` unlocks and re-applies rules.
  - After a manual edit the UI offers *"Apply to all matching \"MERCHANT\""*,
    which creates a substring rule from the cleaned merchant.
  - Tests for lock/unlock behaviour.

- [ ] **2.7 Uncategorized queue query.** `uncategorized_by_merchant(conn) ->
  Vec<{merchant, count, total_cents, first_date, last_date}>`, sorted by
  count descending. Choosing a category for a group creates a substring rule
  on that merchant (which categorizes the whole group via 2.5).

- [ ] **2.8 Categories sheet UI.** A "Categories" button opens a side panel
  (gpui-kit `sheet`) with:
  - a parent/child tree: add parent (name + kind), add child, rename, move
    child, delete (with a "move its transactions to…" picker);
  - a rules list: pattern, substring/regex toggle, target category, and a
    live match preview ("matches 14 transactions") before saving;
  - the Uncategorized queue (2.7) grouped by merchant, each with a category
    picker.
  - UI tests in `ui_tests.rs` style for: create parent+child, create a rule
    and see rows categorized, categorize a queue group, delete with reassign.

- [ ] **2.9 Table context menu.** Right-click a transaction row (use
  `TableDelegate::context_menu`): *Change category…* (then offer the rule as
  in 2.6), *Hide*, and *Unhide* (only when hidden rows are shown, see 3.4).

**Done when:** the demo files import and every row can be categorized from
the UI; tests cover 2.1–2.9. Stop and report.

---

## M3 — Core views (one dashboard)

Layout, top to bottom: filter bar; then the category chart + legend (left)
beside the monthly summary (right); then the transaction table.

- [ ] **3.1 Filter model + queries** (`financi-db/src/views.rs`), all taking
  one `ViewFilter { from: NaiveDate, to: NaiveDate, account_ids: Vec<i64>
  (empty = all), include_transfers: bool }`.
  - Spend/income rules: a row counts as **spend** if its category's kind is
    `spend` *or it is uncategorized*; **income** if kind `income`;
    transfers only when `include_transfers`. Spend is shown positive:
    `spend = -(sum of amount_cents)` over spend rows, so refunds net
    against their category. Hidden rows are excluded.
  - `spend_by_category(filter, parent: Option<i64>)`: with `None`, one row
    per parent (plus an "Uncategorized" row); with `Some(parent)`, one row
    per child of that parent. Each row: id, name, spend_cents, share of total.
    Sorted by spend descending.
  - `monthly_summary(filter)`: months in range (`YYYY-MM`), one row per
    parent with per-month spend and its children's rows, plus Income, Spend
    and Net rows (Net = Income − Spend).
  - Tests: compare against `fixtures/demo/expected.json`. Import the demo
    files with **no categories** (so every row is uncategorized spend, card
    payments aside) and check each month's money out/in per account matches
    the file. Add focused tests for refunds netting, transfer exclusion and
    hidden rows.

- [ ] **3.2 Date range control.**
  - Presets: This month, Last month, Last 3 months, Last 6 months, Last 12
    months, Year to date, All time, Custom….
  - "Last N months" = whole calendar months: the current month plus the N−1
    before it (on 2026-11-07, Last 3 months = 2026-09-01 … 2026-11-30).
  - Custom: two gpui-kit `DatePicker`s (start, end).
  - Default **Last 3 months**. Persist the chosen preset (and custom dates)
    in the `settings` table under `ui.date_range`; restore at launch.
  - Unit-test the preset → date range function, including year boundaries.

- [ ] **3.3 Accounts filter + transfers toggle.** Multi-select of accounts
  (default all) and an "Include transfers" checkbox (default off, persisted
  as `ui.include_transfers`). If gpui-kit has no multi-select, ask before
  building one.

- [ ] **3.4 Transaction table upgrade.**
  - Filtered by the `ViewFilter`, the selected category (parent or child)
    and an optional month (set from the summary).
  - Text search box: case-insensitive match on description or merchant.
  - Sortable columns: Date (default, newest first) and Amount.
  - Toggles: "Uncategorized only" and "Show hidden" (hidden rows greyed,
    with Unhide in the context menu).
  - Amounts: signed, `$1,234.56`, money in shown in green.

- [ ] **3.5 Category chart + legend.**
  - gpui-kit `BarChart` (spend per category, values positive) for the
    picture; beside it a **clickable legend list** (name, `$1,234.56`, %)
    because `BarChart` can't take clicks.
  - Clicking a parent in the legend filters the table to that parent and
    switches chart + legend to its children, with "← All categories" to go
    back. Clicking a child filters further; clicking it again or pressing
    Esc clears.

- [ ] **3.6 Monthly summary table.** Rows = parent categories (expandable to
  children) then Income, Spend, Net; columns = months in range plus Total.
  Clicking a cell filters the table to that category and month. Summary
  amounts are positive `$1,234.56`.

- [ ] **3.7 Native menus + shortcuts.** macOS menu bar via GPUI menus:
  File → Import CSV… (⌘O), Undo last import, Export… (⌘E, wired in M5),
  Restore… (wired in M5), Quit (⌘Q). The toolbar keeps Import CSV… and
  Categories. Undo leaves the toolbar: it's offered in the success message
  after an import and in the File menu (`docs/ui-conventions.md`, D4).

- [ ] **3.8 UI tests** for: preset change updates totals, legend drill-down
  and back, summary cell click filters the table, search, hidden toggle.

**Done when:** on the demo data the dashboard totals match
`expected.json`, filters compose, tests cover 3.1–3.8. Stop and report.

---

## M4 — Simplii Visa, transfer pairing, near-match review

- [ ] **4.1 Simplii Visa (assumed format).** No real Visa sample exists yet.
  Assume it uses the **same format as Simplii chequing**. Because two
  parsers can't claim the same header, replace `simplii-chequing` with one
  parser `simplii` whose `account_kinds()` is `[Chequing, CreditCard]`; the
  chosen account decides which. Funds Out = money out, Funds In = money in
  for both. Keep accepting the old parser id `simplii-chequing` in
  `by_id()` (existing `imports.parser_id` rows). `is_card_payment`: a
  credit-card row with Funds In whose description contains `PAYMENT`.
  Add a synthetic Visa fixture. Note in `plan.md` that this needs revisiting
  when a real sample arrives.

- [ ] **4.2 Transfer pairing** (runs automatically after every import).
  - Card payments: a chequing row with amount −X and a credit-card row with
    amount +X categorized Card Payment, dates within **5 days**, both
    unpaired.
  - Own-account transfers: rows in two different chequing/savings accounts,
    −X and +X, dates within **2 days**, at least one description containing
    `TRANSFER`.
  - Exactly one candidate → pair: set `transfer_pair_id` on both, and
    categorize both as `Card Payment` or `Between Accounts`.
  - More than one candidate → **always ask**: record it in a new table
    (migration 0003):
    ```sql
    CREATE TABLE transfer_reviews (
      transaction_id INTEGER PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
      candidate_ids  TEXT NOT NULL  -- comma-separated transaction ids
    );
    ```
  - UI: a dashboard banner ("2 transfers need review") opens a sheet; for
    each, pick the matching row or "Not a transfer".
  - Paired rows show a link icon in the table; context menu *Unpair* clears
    both sides and re-applies rules.
  - Tests: single match pairs, tie goes to review, resolving a review,
    unpair, an unrelated equal amount is not paired.

- [ ] **4.3 Near-match review in the import preview.**
  - After reconciliation, for each new row find existing rows in the same
    account with the **same cleaned merchant** (case-insensitive), amount
    within **20%** of the existing amount, date within **±3 days**, not
    already matched to another new row.
  - The preview lists each suspected pair with "Same charge" / "Different
    charge" (default: different, so it imports as new).
  - On commit, for each "Same charge": keep the existing transaction id and
    category, replace its date/amount/description/merchant/raw bytes/
    `row_hash` with the new row's, and add the **old** hash to
    `row_aliases` (with the new `import_id`).
  - Reversible undo: migration 0003 also adds
    ```sql
    CREATE TABLE replacements (
      import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
      transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
      old_raw_bytes BLOB NOT NULL, old_row_hash TEXT NOT NULL,
      old_posted_date TEXT NOT NULL, old_processed_date TEXT,
      old_amount_cents INTEGER NOT NULL, old_description_raw TEXT NOT NULL,
      old_merchant TEXT,
      PRIMARY KEY (import_id, transaction_id)
    );
    ```
    `undo_last_import` restores every replaced transaction from this table
    *before* deleting the import.
  - Master CSVs (2.2): after a commit with "Same charge" rows, and after
    undoing one, rewrite the year files for both the old and new dates. The
    old row is removed and the new one added exactly as exported; no row's
    bytes are edited.
  - Tests: tip-added charge is offered; "same" replaces and aliases; a later
    overlapping export then adds 0; undo restores the old values; "different"
    imports as new; the master CSV drops the old row and holds the new one,
    and undo puts the old row back.

**Done when:** tests cover 4.1–4.3. Stop and report.

---

## M5 — Export / restore

Design: `plan.md` → *Export*. The per-account, per-year master CSVs already
exist from 2.2; export copies them instead of rebuilding anything.

- [ ] **5.1 Export** (`financi-db/src/export.rs`, crates `zip`, `serde_json`).
  - Run the 2.2 self-check, then zip: `financi.sqlite` (snapshot via
    `VACUUM INTO` or the backup API), `manifest.json`, and the `csv/` tree
    copied as-is (no rebuilding in export).
  - `manifest.json`: financi version, schema version, exported_at, and per
    file: account_key, institution, account name, kind, year, row count,
    SHA-256; plus the database file's SHA-256.
  - UI: File → Export… (and a toolbar entry) opens a save dialog defaulting
    to `~/Desktop/financi-export-YYYY-MM-DD.zip`.

- [ ] **5.2 Restore.**
  - File → Restore…, and a "Restore from export…" link in the empty-state
    screen.
  - Verify every file against the manifest SHA-256s; refuse if the schema
    version is newer than this build; migrate if older.
  - Back up the current database to `financi.sqlite.bak` (backup API) and
    the current `csv/` to `csv.bak/`, then restore into the live connection
    with rusqlite's backup/restore API on the worker thread (no file
    swapping), replace `csv/` with the zip's tree, then refresh all views.

- [ ] **5.3 Acceptance tests** (all must pass):
  - Overlapping re-import adds 0 rows (exists; keep it) and leaves the
    master CSVs byte-identical.
  - Totals match `fixtures/demo/expected.json` (from 3.1).
  - **Export round-trip, byte-exact:** import every demo file, hide a row,
    export, then import the zip's CSVs into a fresh database whose accounts
    use the manifest's account keys → same multiset of
    `(account_key, row_hash)` and identical row bytes, and the fresh
    database's master CSVs are byte-identical to the zip's; re-importing into
    the source database adds 0 rows; restore from the zip gives an identical
    database (same rows, categories, rules, locks, hidden flags) and `csv/`.

**Done when:** 5.1–5.3 pass. v1 complete. Stop and report.

---

## After v1 (not now)

v1.1 spend-over-time line + top merchants · v1.2 flags (large, new merchant,
FX/fees) · v1.3 recurring/subscriptions · more institutions · CSV export of
filtered rows. Real Simplii Visa sample → revisit 4.1.
