# M3 — backend queue: scale, then correctness

Runs beside the UI queue, in the order listed. Backend only: don't read the UI docs for these. 5.1 and 5.2 (`docs/tasks/m5.md`) follow 3.90.

## Backend queue: scale, then correctness (in this order)

Runs beside the UI queue. 5.1 and 5.2 (in M5) follow 3.90.

- [x] **3.87 Rule use stats.** (Split out of 3.58, 2026-11-11.) Backend
  only (`financi-db/src/rules.rs`); runs in a free slot before 3.58.
  Replace `rule_use_counts` with `rule_use_stats(conn) ->
  HashMap<i64, RuleUse { count, total_cents }>`, one pass over unlocked,
  non-card-payment, **non-hidden** rows (today `rule_use_counts` and
  `count_rule_matches` count hidden rows while every other count excludes
  them, so Uses disagrees with category counts once a row is hidden;
  make `count_rule_matches` exclude them too). Callers switch to it.
  Tests: count and signed total match a direct SQL sum for the rows each
  rule decides; hidden rows are excluded from both functions.

- [ ] **3.66 Simplii card payments only on the card.** (Code review,
  2026-11-11.) `Simplii::is_card_payment` (`financi-parsers/src/simplii.rs`)
  only sees the `RawTransaction`, so the Visa rule ("money in whose
  description contains PAYMENT") also runs on **chequing** imports. A
  chequing deposit such as "GST PAYMENT CANADA", "PAYMENT REVERSAL" or an
  e-transfer memo saying payment becomes `Transfers › Card Payment`,
  locked against rules and left out of totals: income vanishes. On the
  Visa side, a refund from a merchant whose name contains PAYMENT
  ("PAYPAL *PAYMENTS") is locked as a transfer too.
  - Change the trait to `fn is_card_payment(&self, txn: &RawTransaction,
    kind: AccountKind) -> bool` and pass the account's kind at import
    (`import.rs`) and in `overrides.rs`. Simplii returns false unless
    `kind == CreditCard`, and matches `PAYMENT` only together with
    `RECEIVED` or `THANK YOU` (as Amex's `PAYMENT RECEIVED`). Amex
    ignores `kind`.
  - **Repair existing data:** add a numbered migration (or a one-time
    startup step recorded in `settings`) that, for chequing and savings
    accounts, finds rows in Card Payment with `amount_cents > 0` and no
    `transfer_pair_id`, clears `category_locked` and re-runs rules on
    them. Before writing it, run a read-only count on a copy of the
    user's DB (`FINANCI_DB=.cache/real-copy.sqlite`, counts only, never
    commit it) and put the count in the handoff.
  - The opt-in `real_samples` test prints the card-payment count per
    file.
  - Tests: a chequing fixture row ` GST PAYMENT CANADA,,100.00` is not a
    card payment and counts as income; "PAYPAL *PAYMENTS" money in on the
    Visa is not a card payment; "PAYMENT RECEIVED THANK YOU" on the Visa
    still is; the repair unlocks and recategorizes an affected row and
    leaves a paired or negative one alone.

- [ ] **3.67 The CSV reader survives real-world files.** (Code review,
  2026-11-11.) All in `financi-core/src/raw.rs`, `lib.rs` and
  `financi/src/flow.rs`.
  - **Quotes only open at a field's start.** `split_records` and
    `split_fields` flip the quote state on every `"`, so an Amex
    description like `BEST BUY 55" TV    TORONTO` swallows the following
    lines until the next `"` and the whole file fails to import. Open a
    quote only at the start of a field; inside an unquoted field `"` is a
    literal; a quoted field closes at `"` followed by `,`, end of line or
    end of input; `""` stays an escaped quote. Hashing must not change
    for files that parse today (they contain no such quotes): add a test
    that every existing fixture hashes the same before and after.
  - **Line numbers users can find.** `source_line` is a 0-based record
    index (it skips blank lines), but errors print it as "record 0". Keep
    `source_line` (it's stored and orders rows) and add
    `RawRecord.file_line`, the 1-based physical line where the record
    starts; errors say "line 2: bad Date …".
  - **Non-CSV files fail fast.** Reject files over 50 MB or containing a
    NUL byte with "This isn't a CSV export.". Cut the header shown in the
    unknown-format message to ~120 characters. An unclosed quote in a
    file with no recognized header reports the unknown-format message.
    Run `flow::load_file` on the background executor, not the UI thread.
  - **Fixtures end to end:** an Amex file with CRLF and a BOM, a Simplii
    file in Windows-1252 with an accented description, a quoted amount
    `"1,234.56"`, a blank line mid-file, and the stray-quote row; plus a
    test that the same rows hash identically across the LF / CRLF / BOM
    variants.
  - `parse::cents` accepts commas only as thousands groups (`1,234.56`);
    `"1,2,3"` and `"12,34"` are rejected.

- [ ] **3.68 Near-match review asks only about changed charges.** (Code
  review, 2026-11-11.) In `financi-db/src/import.rs`:
  - **Repeat purchases aren't "changed charges."** `find_near_matches`
    compares new rows with existing rows that the same file still
    contains unchanged. A re-export holding yesterday's $4.50 coffee
    plus today's $5.00 one offers today's as "Same charge?" against
    yesterday's. Exclude existing rows whose hash (or alias) the file
    already contains. Test: two coffees on consecutive days, the earlier
    still in the file → no near-match.
  - **Bounded scan.** It loads every visible row of the account and
    allocates the cleaned merchant twice per pair. Limit the SQL to
    `posted_date` between the file's min date − 3 days and max date + 3
    days, compute each existing row's lowercased merchant once, and look
    candidates up in a `HashMap<merchant, Vec<row>>`.
  - **A replacement keeps a valid master CSV.** "Same charge" puts the
    new export's bytes on a row whose `import_id` is the old import, so
    the master CSV writes them under the old import's header and
    encoding. Don't offer a near-match when the two imports' header /
    encoding variant differs (the row imports as new). Test with a
    changed header.
  - **FX columns follow the replacement.** `apply_replacement` doesn't
    update `original_currency` / `original_amount_cents`, and
    `replacements` can't restore them. Add both `old_…` columns to
    `replacements` in a new migration; update on replace, restore on
    undo; test it.

- [ ] **3.69 Paired transfers stay consistent.** (Code review,
  2026-11-11.) In `financi-db` (`overrides.rs`, `transfers.rs`,
  `import.rs`):
  - **Editing one side unpairs both.** Totals exclude transfers by
    category kind, so `set_category` on a paired row to "Rent" makes that
    side spend while its partner stays a transfer, and the link icon
    stays. `set_category` on a paired row clears `transfer_pair_id` on
    both sides in the same transaction; the edited row keeps its manual
    category; the partner is unlocked and rules re-run on it.
  - **Unpair keeps manual choices.** `unpair_transaction` calls
    `clear_override` on both sides, discarding a manual category. Only
    clear the override on a side whose category is still transfer-kind.
  - **Undo re-pairs.** `undo_last_import` doesn't re-run pairing, so
    partners released by the undo stay unpaired until the next import,
    and reviews can point at deleted rows. Run pairing at the end of the
    undo, and drop reviews left with fewer than two live candidates in
    `list_transfer_reviews`.
  - **Pairing settles.** `pair_mutuals` makes one pass, so a row whose
    only competitor was paired in the same run stays unpaired with no
    review. Repeat until nothing changes.
  - **Card-payment pairs need one card and one chequing row.**
    `pair_categories_for` returns Card Payment when either side is a card,
    so a stale review could pair two card rows. Require exactly one of
    each.
  - Tests for each bullet.

- [ ] **3.70 Database guard rails.** (Code review, 2026-11-11.) Small
  `financi-db` fixes, one commit each or together:
  - **Undo names its import.** `undo_last_import(account_id)` deletes
    whatever import is newest; the UI looks the account up from the row
    the user clicked, so stale history could delete a different import.
    Take `expected_import_id` and refuse if it isn't the newest.
  - **Deleting a category keeps its kind.** `delete_category` accepts a
    reassign target of another kind, so deleting an expense subcategory
    into an income one silently turns spend into income. Refuse with
    `CategoryKindMismatch`, as `move_child` does.
  - **The startup self-check compares content.** `year_is_current` only
    compares names and row counts, so a replacement (or a failed rewrite
    after a commit) that keeps the count is never repaired. Compare the
    file's bytes with the bytes `year_files` already builds.
  - **Worker panics say so.** A panicking job reaches the caller as
    `WorkerGone` ("database worker is gone") although the worker
    survives. Add `DbError::JobPanicked`.
  - Tests for each.

- [ ] **3.72 Text rules ignore case for accented letters.** (Code review,
  2026-11-11.) Substring rules lowercase with `to_ascii_lowercase`
  (`financi-core/src/rules.rs`), regex rules are Unicode
  case-insensitive. So the text rule `Montréal` never matches
  `CAFE    MONTRÉAL`, while the regex `montréal` does. Use
  `to_lowercase()` on pattern and description, keep tie-break lengths as
  `chars().count()` of the original pattern, and add tests with accented
  upper and lower case. 3.55's Aho-Corasick matcher must keep this
  folding.

- [ ] **3.71 Row hashes don't depend on the encoding guess.** (Code
  review, 2026-11-11.) Parsers hash `record.fields`, which are decoded
  strings, and the encoding is guessed once per file: one invalid UTF-8
  byte makes the whole file Windows-1252. Then every correctly encoded
  `É` in that file decodes as `Ã‰`, those rows hash differently from the
  same rows imported earlier from a clean file, and they're imported
  twice (near-match misses them too). With a BOM, `from_utf8_lossy` maps
  different bad bytes to the same U+FFFD, so distinct rows can share a
  hash and real rows are dropped.
  - Keep each field's undecoded bytes on `RawRecord` (quotes removed) and
    hash those, length-prefixed as now.
  - This changes the hash of rows with non-ASCII characters already in
    the database. **Stop and ask the user** with two options and the
    count of affected rows in a copy of their DB (counts only): (a) a
    migration that re-hashes every row from its stored `raw_bytes` and
    rewrites `row_aliases`, or (b) keep decoded hashing for Amex and
    Simplii and use byte hashing only for new parsers, documented in
    `plan.md`.
  - Tests: an all-ASCII row's hash is unchanged; the same accented row in
    a clean UTF-8 file and in a file with one stray invalid byte hashes
    the same; two rows differing only in invalid bytes hash differently.

- [ ] **3.90 Build and tooling.** (Code review, 2026-11-11.) A `chore:`
  filler for any free slot.
  - Drop `spikes/*` from the workspace `members` (keep the directory, as
    `CLAUDE.md` says): every build, clippy and clean-checkout verify
    compiles that extra GPUI binary today.
  - Add `rust-version` to `[workspace.package]` matching `mise.toml`.
  - `build.rs`: also `rerun-if-changed` on `git rev-parse --git-path
    packed-refs` and `--git-path index`, so the version follows commits
    and packed tags.
  - `.githooks/post-commit`: before 1.0 a breaking change bumps the minor
    version, not 1.0.0; for a merge commit, scan the merged commits'
    subjects and take the highest bump.
  - `flow::parser()` returns `Option` instead of panicking on an unknown
    id, and `new_account_defaults` doesn't index `account_kinds()[0]`;
    add a parser-registry test: unique ids, at least one account kind
    each, and every parser's institution exists in the `institutions`
    table.
