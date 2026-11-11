# financi — Plan & Requirements

2026-11-08 · @Louie

This file holds requirements and settled decisions. `docs/tasks.md` is the task checklist and wins on M2–M5 detail. `docs/ui-conventions.md` covers layout and formats. All product questions are settled; new ones are decided and recorded under **Decisions** rather than left open here.

## Status

| Milestone | State |
| --- | --- |
| M0: Skeleton + GPUI spike | Done |
| M1: Amex import, end to end | Done |
| M2: Master CSVs + categorize | Done |
| M3: Core views | In progress: 3.1–3.21 done. Remaining, in this order: 3.24 (edit and delete rules), 3.25 (dashboard alignment), 3.22 (queue fits the window), 3.23 (Categories page layout), 3.26 (scrollbars everywhere) |
| M4: Simplii Visa, transfer pairing, near-match | Done |
| M5: Export / restore | Not started |

The prototype fixes under **Roadmap** are tracked separately from the milestones.

## Overview

financi is a single local Rust binary that imports CSV exports from banks and card issuers into SQLite and shows spending trends in a GPUI desktop window. It is for one person, offline, with no cloud sync. It targets macOS and Linux.

**Goals**

- Import CSVs from multiple institutions (Amex, Simplii first) through per-institution parsers that all produce one shared transaction model.
- Store every transaction in one local SQLite file, safe to re-import without duplicates.
- Show spend by category over a chosen date range, a drill-down transaction table, and a month-by-category summary.

**v1 scope:** import, categorize workflow, category bar chart, transaction table, monthly summary with income, spend and net, Simplii, export/restore.

**Non-goals (v1)**

- Direct bank connections (Plaid, Flinks, scraping).
- Budgets, forecasting, investments, multi-user access.
- Mobile or web front ends, a CLI, Windows.

**v1 acceptance** (tests, task 5.3)

- An overlapping re-import adds 0 rows and leaves the master CSVs byte-identical.
- Totals match the independent reference totals in `fixtures/demo/expected.json`.
- The export round-trip is byte-exact.

There are no numeric import-speed or categorization-rate targets.

## Core flow

1. Export CSVs from each institution's website by hand, typically the last 30 to 90 days, so overlapping ranges are normal.
2. Open `financi` and drag the CSVs into the window. financi detects the institution from the header and you confirm the account.
3. The matching parser maps each raw record into the shared model.
4. Normalize: sign convention, dates, merchant cleanup, category assignment.
5. Deduplicate against existing rows and insert into SQLite in one database transaction per file, recording the import.
6. Merge the new rows into the master CSV for each account and year they fall in.
7. Pick a date range, read the category chart, click through to filter the transaction table, and scan the monthly summary.

## Import and deduplication

Re-importing overlapping exports must never create duplicates or drop real repeat purchases. Every check uses the **raw CSV fields**, so later edits in the app never change a row's identity.

1. **Raw-row hash.** `row_hash = sha256(account_key, identity fields)`, each field length-prefixed so a delimiter inside a description can't collide. `account_key` is a UUID created with the account, never the database row id, so a restored or fresh database produces the same hashes. Each parser hashes its own identity columns as their exact exported strings, with no trimming, case changes or parsing, and before mapping the row. Columns that can change later or are derived, such as a processed date or running balance, are left out. The hash is not unique, because two identical charges on one day give identical rows.
2. **Count reconciliation.** Rows are grouped by `row_hash`. If the file has n rows with a hash and the account already has m (transactions plus aliases), insert max(0, n − m). Real repeats survive, and overlapping days never double-count.
3. **Near-match review.** A new row in the same account with the same cleaned merchant (case-insensitive), an amount within 20% and a date within ±3 days of an existing row, but a different hash, is listed in the preview as a possibly changed charge, such as a tip added. The default is "Different charge", which imports it as new. "Same charge" replaces the transaction's fields and raw bytes, keeps its id and category, and stores the old hash in `row_aliases`, so later overlapping exports match it silently.
4. **Raw bytes kept.** Each record's exact bytes are stored with the transaction, and the file's encoding, BOM, line ending and trailing newline are stored on the import.

**Preview.** Shows rows to add, rows skipped as already present, and near-matches to review before anything is written.

**Account safety.** The account is always confirmed in the preview, never guessed from the filename (every Amex card downloads as `activity.csv`). New accounts are created inline from the picker. Each parser declares its institution and the account kinds it applies to. If even one row of the file already exists under a different account, the import is blocked with an error naming that account and goes ahead only with an explicit override.

**Undo import.** The latest import of each account can be undone. It hard-deletes that import's rows and aliases and restores any transactions it replaced through near-match. Deleting a single transaction only hides it (see **Export and restore**).

**Accepted trade-off.** The provider's export format is assumed stable. If it changes, previously imported rows hash differently, and near-match review is the safety net.

### Source formats

**Amex (`activity.csv`)**

| Column | Example shape | Maps to |
| --- | --- | --- |
| `Date` | `05 Nov 2026` (`%d %b %Y`) | `posted_date` (transaction date) |
| `Date Processed` | `06 Nov 2026` | `processed_date` |
| `Description` | `MERCHANT NAME␣␣␣␣CITY`, space-padded, occasionally quoted with a comma | `description_raw`; merchant and city split on the run of spaces |
| `Amount` | `15.74`, `-2042.40` | `amount_cents`, **sign flipped**: Amex shows charges positive and payments negative |

- UTF-8, comma-delimited, one header row, newest first, no trailing newline.
- No transaction ID, card or cardmember column, Amex category, or foreign-currency column.
- Identity fields: Date, Description, Amount.
- Payments show as `PAYMENT RECEIVED - THANK YOU` with a negative amount, and are detected as card payments.

**Simplii chequing (`SIMPLII.csv`)**

| Column (raw header) | Example shape | Maps to |
| --- | --- | --- |
| `Date` | `10/07/2026` (`%m/%d/%Y`, month first) | `posted_date` |
| `␣Transaction Details` | `␣INTERAC E-TRANSFER ...`, usually with a leading space | `description_raw` (space kept); merchant is trimmed for display |
| `␣Funds Out` | `25.00` or empty | `amount_cents` negative |
| `␣Funds In␣` | `101.19` or empty | `amount_cents` positive |

- ASCII, LF line endings, trailing newline present, oldest first, no quoting in the sample.
- Header detection compares trimmed names; exact bytes are still stored.
- Exactly one of Funds Out or Funds In is filled per row, both unsigned.
- Identity fields: all four columns as exported. There is no balance column.

**Simplii Visa.** Implemented on the assumption that it uses the chequing format (task 4.1): one `simplii` parser with `account_kinds()` `[Chequing, CreditCard]`, the account chosen at import deciding which; the retired id `simplii-chequing` still resolves in `by_id()`. Funds Out is money out and Funds In money in for both; a Funds In row whose description contains `PAYMENT` is a card payment. Synthetic fixture: `crates/financi-parsers/tests/fixtures/simplii/visa.csv`. **Revisit when a real Visa sample arrives:** add a fixture modelled on it and adjust the parser if the format differs.

**Classifying Simplii rows**

- **Interac e-transfers** mix moving your own money and paying people, so they get no default category and land in the To categorize queue. A rule on the counterparty name covers later rows.
- **Own-money e-transfers** are paired automatically when the other side is in an imported account.
- **Internet bill payments** are paired with a card's payment row and become Transfer, or are categorized as bills by rules on the payee name.

## Master CSVs

financi keeps its own provider-format CSV for every account and calendar year, built at import time. It holds every row the account has, deduplicated as the database is, with each row's bytes untouched. Uploaded files are not archived; these files replace them.

```
<app data dir>/financi/
├── financi.sqlite
└── csv/
    ├── amex/
    │   └── <account_key>/
    │       ├── account.json      # name, institution, kind
    │       ├── 2025.csv
    │       └── 2026.csv
    └── simplii/
        └── <account_key>/
            ├── account.json
            └── 2026.csv
```

1. **Location.** `csv/` sits beside the database, and `FINANCI_DB` moves it too. Folders use the institution key and `account_key`, so renaming an account moves nothing; `account.json` is rewritten on rename.
2. **Which file.** A row goes in the file for the calendar year of its transaction date. An import spanning Dec–Jan splits across two files.
3. **Contents.** The provider's header, then each row's stored `raw_bytes`. Real identical repeats appear as many times as they occurred, and hidden rows stay. Rows are ordered by date in the provider's direction (Amex newest first, Simplii oldest first), then import order, then `source_line`. Encoding, BOM, line ending and trailing newline come from the earliest import in that file. Rows whose import has a different header, encoding or BOM go to `<year>-v2.csv`, `-v3`, and so on.
4. **When written.** After any commit that changes an account's rows (import, undo, near-match confirm), each affected year file is rewritten on the `DbWorker` thread. Writes are atomic: temp file, fsync, rename. An empty year's file is deleted.
5. **Rows are added or removed, never edited.** Confirming a near-match removes the old row and adds the new one exactly as exported.
6. **Self-check.** At startup, any year file that is missing, extra, or has a row count that doesn't match the database is rewritten and a notice is logged.

## Export and restore

**Export** writes one zip holding a database snapshot and the master CSVs, copied as they are:

```
financi-export-2026-11-07.zip
├── financi.sqlite          # snapshot via SQLite backup API / VACUUM INTO
├── manifest.json           # versions, accounts, per-file row count + SHA-256
└── csv/                    # the master CSVs, same layout as on disk
```

- The save dialog defaults to `~/Desktop/financi-export-YYYY-MM-DD.zip`.
- `manifest.json` records the financi and schema versions, export time, the database's SHA-256, and each CSV's account (`account_key`, name, institution, kind), year, row count and SHA-256. The financi version is the build's `git describe` value.
- Edits (categories, rules, overrides, flag decisions) exist only in `financi.sqlite`. The CSVs hold provider data and nothing else.
- Deleting in the app hides a row (`transactions.hidden`). Hidden rows stay in the CSVs, and their stored hash means a re-import never brings them back.

**Restore** checks every file against its manifest SHA-256, refuses a schema newer than the build and migrates an older one. It backs up the database to `financi.sqlite.bak` and `csv/` to `csv.bak/`, then replaces both. Categories, rules, overrides and flag decisions come back with it.

**Round-trip guarantee (tested).** Importing the export's CSVs into an empty database yields the same multiset of `(account_key, row_hash)` with identical row bytes, and importing them into the source database adds 0 rows.

## Categorization

- **Categories** have two levels, parent and child. Transactions are assigned to children only. The chart shows parents and drills into children.
- **Kinds.** Each category is Expense, Income or Transfer, and transfer status comes only from this.
- **System categories.** Transfers (Card Payment, Between Accounts) is present in every database and can't be renamed, moved or deleted.
- **Default categories and rules.** The app ships a list of common expense and income categories plus substring rules for well-known merchants, so a first import is largely categorized. They are inserted once at first run of the real app, recorded in the `settings` table so deleted defaults never return, and only into a database with no transactions and no user categories. They are ordinary editable rows, not system rows. Existing databases are left alone, `FINANCI_DEV_SEED` runs skip them, and migrations still create only the system categories, so the "no categories" tests stay valid.
- **Rules** match `description_raw`, case-insensitively, by substring or regex. Substring rules beat regex rules; within each kind the longest pattern wins; ties go to the lowest rule id. There are no account or sign filters.
- **Timing.** Rules apply at import and re-run over all unlocked rows whenever a rule is created, edited or deleted.
- **Card payments** are detected by a built-in per-parser check that beats user rules.
- **Manual edit** locks the row against rules and offers "Apply to all matching MERCHANT", which creates a rule.
- **To categorize queue** groups Uncategorized rows by merchant with count and total. Choosing a category creates a rule for the group.
- **Deleting a category** requires choosing where its transactions and rules go.
- **Refunds.** Positive amounts in an expense category net against it. Uncategorized rows count as spend.
- **Currency.** CAD only, the amount the card charged. Original currency and amount are kept when the CSV has them.

**Transfers.** Pairing runs automatically after each import. Card payments pair within 5 days, own-account transfers within 2 days. Exactly one candidate pairs automatically; more than one goes to the Transfers page, with a count in the sidebar. Rows can be unpaired. An unpaired card payment still counts as a transfer. Transfers are excluded from totals by default.

## Architecture

A Cargo workspace keeps parsing and storage free of GPUI, so they can be tested without a window.

| Crate | Role | Key deps |
| --- | --- | --- |
| `financi-core` | Raw file reader, shared model, parsing helpers, `StatementParser` trait, `row_hash`, rule matching | `chrono`, `sha2`, `regex` |
| `financi-parsers` | One module per institution; detection by header | `financi-core` |
| `financi-db` | Migrations, import with dedup, master CSVs, categories, rules, view queries, `DbWorker` | `rusqlite` (bundled), `directories`, `uuid` |
| `financi` (bin) | Entry point, import flow helpers, GPUI views (`ui.rs`) | `gpui-kit =0.9.0` on `gpui-pre =0.3.11` |

- Core reads each file once into a header and raw records, so bytes, positions and encoding survive for hashing and export. Parsers receive a `RawRecord` and implement identity, detection, mapping, `row_hash`, row order and card-payment detection.
- Parsers produce a raw transaction; core normalizes it. Adding an institution means one small module and a fixture-based test.
- SQLite runs in WAL mode on one `DbWorker` thread. The UI sends queries to it and re-queries on a "data changed" event.
- The database is plain SQLite in the per-OS app data directory (`~/Library/Application Support/financi/` on macOS, `$XDG_DATA_HOME/financi/` on Linux). Disk encryption (FileVault / LUKS) is relied on.
- GPUI and gpui-kit are pinned exactly, with `runtime_shaders`, so no full Xcode is needed.

## Data model

Migrations in `crates/financi-db/migrations/` are the source of truth for the schema. `0001_init.sql` is frozen. Money is integer cents with a currency code; negative means money out.

| Table | Purpose |
| --- | --- |
| `institutions`, `accounts` | Built-in institutions; accounts under them with a stable `account_key` and a kind (chequing, savings, credit card) |
| `categories` | Two levels, `kind`, `is_system` (0002) |
| `category_rules` | Pattern, `is_regex` (0002), target child category |
| `imports` | One per file, with header bytes and file format details |
| `transactions` | Raw bytes, `row_hash`, parsed fields, `category_locked`, `transfer_pair_id`, `hidden` |
| `row_aliases` | Older hashes a confirmed near-match still answers to |
| `settings` | UI preferences and first-run markers |
| `flags` | Created in 0001, unused until v1.2 |
| `transfer_reviews`, `replacements` | Planned in migration 0003 (tasks 4.2 and 4.3) |

Recurring-series storage is deferred along with the recurring view.

## UI requirements

Sidebar of pages: Dashboard, To categorize, Categories, Transfers, Imports. Layout and formats are in `docs/ui-conventions.md`.

**Dashboard**, linked through one shared filter state:

- **Filters.** Date range presets (This month, Last month, Last 3 / 6 / 12 months, Year to date, All time) plus custom dates; default Last 3 months as whole calendar months, remembered between launches. Accounts filter: a dropdown of checkboxes built from gpui-kit parts, one per account, default all. "Include transfers", default off.
- **Category chart.** gpui-kit `BarChart` beside a clickable legend (the chart takes no clicks). Clicking a parent filters the table and drills into its children; clicking a child again or pressing Esc clears.
- **Monthly summary.** Parent rows, expandable to children, then Income, Spend and Net, one column per month plus Total. Clicking a cell filters the table to that category and month.
- **Transaction table.** Date, account, merchant, category, amount. Search, sort by date or amount, "Uncategorized only" and "Show hidden" toggles. Virtualized. Right-click: Change category, Hide, Unhide.

**Money display.** `$1,234.56`. Spend is positive in charts and summaries. The table is signed, with money in shown green.

**Commands.** The native menu bar is File > Import CSV… ⌘O, Undo last import, Export… ⌘E, Restore…, Quit ⌘Q. Nothing depends on it: on both platforms the Imports page exposes Import, Undo, Export and Restore. Restore also appears on the empty screen.

## Decisions

Decisions recorded in the sections above are not repeated. These are the remaining ones, with the reasoning worth keeping.

| Topic | Decision |
| --- | --- |
| Accounts (v1) | Amex credit cards (more than one), Simplii chequing, Simplii Visa. |
| Dedup key | Stable account key plus each source's raw identity fields. No file hash, because overlapping exports differ as files. |
| Amex hash | Date Processed is excluded because it is the field most likely to change before a charge settles. |
| Master CSVs | One file per account per year, never per institution, built at import rather than at export. |
| Possible duplicates | Handled by near-match review at import, not as a flag, because identical repeat purchases are kept on purpose. |
| Component kit | gpui-kit was verified in M0 for file drop, `BarChart`, virtual `DataTable` and `Select`. It has no multi-select. |
| Commit versions | A tracked post-commit hook tags each `main` commit `vX.Y.Z` from its Conventional Commit type. |
| Agent workflow | `CLAUDE.md` and `AGENTS.md`: one milestone at a time, continuing to the next without a check-in, through the end of v1. |

## Roadmap

**v1:** M3 (remaining: 3.24, 3.25, 3.22, 3.23, 3.26, in that order), then M5, per `docs/tasks.md`.

**Prototype fixes** (feedback from 2026-11-08)

- [ ] **Let rules be edited and deleted.** *(Added 2026-11-10; task 3.24 in `docs/tasks.md`. Do first.)* Rules can only be added today, so a wrong pattern can't be corrected and keeps miscategorizing every import. Each rule on the Categories page gets Edit (pattern, regex switch and category, in place, with the live match count) and Delete… (two-click confirm). Saving or deleting re-runs all rules over unlocked transactions, as adding one does.
- [ ] **Line up the dashboard.** *(Added 2026-11-10; task 3.25.)* Since labels were added, the filter controls sit at different heights and the page's regions don't share edges. Labelled controls, checkboxes and the transaction count share one center line; the two filter rows share column edges; the filters, chart, summary and table share the page's left and right edges; the chart and summary panels match in height and title position.
- [ ] **Show a scrollbar on everything that scrolls.** *(Added 2026-11-10; task 3.26. Do after 3.22 and 3.23, which re-lay out two of the lists.)* Every scrollable region (category tree, rule list, To categorize queue, chart legend, monthly summary, Transfers list, tables, long popovers and dropdowns) shows a scrollbar whenever its content overflows, visible without having to scroll first, and none when the content fits.
- [ ] **Ship default categories and merchant rules**, as specified under **Categorization**.
- [ ] **Keep the app responsive with thousands of rows.** Profile the slowdown after large imports, then render only the rows on screen in every long list, starting with the To categorize queue. Create its per-row category dropdowns only when needed.
- [ ] **Make category dropdowns usable with thousands of categories.** Scrolling a flat list won't work at that size, so every dropdown that picks a category (rules, the To categorize queue, the move and delete targets on the Categories page) becomes searchable: type to filter, match on both parent and child names, and show each result with its parent so same-named children can be told apart. Works from the keyboard and renders only the visible options.
- [ ] **Give each section of the Categories page real boundaries and its own scrolling.** *(Tracked as tasks 3.21 and 3.23 in `docs/tasks.md`.)* As rules accumulate they crowd the page and push the new category section past the window edge. Each section (new category controls, category tree, selected category details, rules list) gets a defined boundary, and long content scrolls inside its own section. At the minimum window size and above, no section overflows the page or pushes another out of view.
- [ ] **Make the charts scale cleanly with the window.** Chart lines currently overlap the labels, pieces are cut off, and the layout doesn't adapt to the space available. At the minimum window size and above, every chart stays inside its panel, nothing is clipped, and lines and labels don't overlap, with labels shortened or thinned out when space is tight.
- [ ] **Align the columns in the To categorize table.** *(Tracked as tasks 3.21 and 3.22 in `docs/tasks.md`.)* Every column lines up from row to row. Text beyond a column's limit is cut with an ellipsis (…) and shown in full on hover.
- [x] **Rename the "Spend" category type to "Expense".** The kind is `expense` in code, database and UI. `0001_init.sql` was edited in place because no data existed.
- [ ] **Label the category type dropdown** with a visible "Category type" label.
- [ ] **Make account selection on import clearer.** Show the detected institution prominently next to the account selection. Account names exclude the institution, so the picker shows "Amex · Cobalt" from institution and name.
- [x] **Tag every commit with a semver version.** `.githooks/post-commit` creates an annotated tag on `main`: breaking change = major, `feat` = minor, anything else = patch. Enable per clone with `git config core.hooksPath .githooks`; publish with `git push --follow-tags`.
- [ ] **Show the version subtly in the UI.** Embed `git describe --tags --always` in a `build.rs`, falling back to `CARGO_PKG_VERSION`. Show it in small muted text at the bottom of the sidebar, and use the same value in the export manifest.

**After v1**

- v1.1: spend-over-time line (total or selected category, with income and net as optional series) and top merchants (total, count, average; click filters the table).
- v1.2: flags with a review list; a dismissed flag never returns. Thresholds live in settings. Unusually large: above 3x the median for the merchant, or for the category if the merchant has fewer than 3 past charges. New merchant: first appearance of the cleaned name. FX / fees: foreign currency, or a description matching FX fee, interest, NSF, late fee or annual fee.
- v1.3: recurring / subscriptions: detect regular cadence at a similar amount; show cadence, last charge, next expected charge and yearly cost; flag price increases and stopped subscriptions.
- Later: more institutions, CSV export of filtered rows.
