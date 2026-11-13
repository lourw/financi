# M3 — backend queue: scale, then correctness

Runs beside the UI queue, in the order listed. Backend only: don't read the UI docs for these. 5.1 and 5.2 (`docs/tasks/m5.md`) follow 3.90.

## Backend queue: scale, then correctness (in this order)

Runs beside the UI queue. 5.1 and 5.2 (in M5) follow 3.90.

- [x] **3.69 Paired transfers stay consistent.** (Code review,
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

- [ ] **3.91 CSV loading runs off the UI thread.** (Follow-up split from
  3.67, 2026-11-12; run after 3.74 has merged — it touches
  `ui/imports.rs`.) `Shell::open_file` calls `flow::load_file` on the UI
  thread, so a large or pathological file freezes the window while it
  parses. Run it on the background executor with an update hop back to
  the entity (the pattern the other DbWorker call sites in
  `ui/imports.rs` use), keeping the toast and error reporting as they
  are. While there, check the file's size against the 50 MB limit via
  `path.metadata()` *before* `std::fs::read`, so a huge drop is
  rejected without loading it into memory (3.67 review note).
