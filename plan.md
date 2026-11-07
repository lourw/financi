# financi — Plan & Requirements

2026-11-07 · @Louie

## Overview

financi is a single local Rust binary that imports CSV exports from banks and card issuers into SQLite and shows spending trends in a GPUI desktop window. It is for one person, offline, with no cloud sync.

**Goals**

- Import CSVs from multiple institutions (Amex, Simplii first) through per-institution parsers that all produce one shared transaction model.
- Store every transaction in one local SQLite file, safe to re-import without duplicates.
- Show spend by category over a chosen date range, a drill-down transaction table, and a month-by-category summary.

**Non-goals (v1)**

- Direct bank connections (Plaid, Flinks, scraping).
- Budgets, forecasting, investments, multi-user access.
- Mobile or web front ends.

## Core flow

One binary that opens a GPUI window; importing happens inside it, with no separate CLI step.

1. Export CSVs from each institution's website by hand.
2. Open `financi` and drag the CSVs into the window; financi detects the institution from the header and you pick the account.
3. The matching parser turns each row into a `Transaction` struct.
4. Normalize: sign convention, dates, merchant cleanup, category assignment.
5. Deduplicate against existing rows and insert into SQLite in one database transaction per file; record the import batch.
6. Pick a date range, read the category bar chart, click a bar to filter the transaction table, and scan the monthly summary.

## Import and deduplication

Re-importing overlapping exports must never create duplicates or drop real repeat purchases. Every check uses the **raw CSV fields**, so later edits in the app (rename, recategorize) never change a row's identity.

1. **Raw-row hash.** `row_hash = sha256(account_key, identity fields)`. Each field is length-prefixed so a pipe or comma inside a description can't collide. account_key is a UUID created with the account, never the database row id, so a restored or fresh database produces the same hashes. There is no universal hash: each institution's parser implements its own row_hash over that source's identity columns, and they are hashed as their exact exported strings, with no trimming, case changes or parsing. Identity fields are the ones that define the transaction and won't change between exports: date, description and amount. Columns that can change later or are derived, such as a processed date or a running balance, are left out. The provider's export format is assumed to be stable, so the same transaction exported again always gives the same hash and imports are idempotent. The hash is computed before the parser maps the row, and later edits in the app never touch it. It is not unique, because two identical charges on one day give identical rows.
2. **Count reconciliation.** Rows are grouped by `row_hash`. If the file has n rows with a hash and the account already has m (transactions plus aliases), insert max(0, n − m). Real repeats survive, and overlapping days never double-count.
3. **Near-match review.** A new row in the same account with a similar description, an amount within 20% and a date within ±3 days of an existing row, but a different hash, is shown in the preview as a possibly changed charge (a tip added, a description rewritten on posting). On confirm, the newer version replaces the transaction's fields and raw bytes, and the old hash is kept in row_aliases, so later overlapping exports match it silently.
4. **Raw bytes kept.** Each record's exact bytes are stored with the transaction, and the file's encoding, BOM, line ending and trailing newline are stored on the import, so export can reproduce every row.

**Account safety.** The account is always confirmed in the preview, never guessed from the filename (every Amex card downloads as `activity.csv`). If a large share of the file's hashes already exists under a different account, the preview warns before importing.

**Undo import.** The most recent import of each account can be undone, which hard-deletes its rows and aliases. Limiting undo to the latest import keeps near-match replacements reversible.

Trade-off accepted: if a provider changes its export format, previously imported rows will hash differently. The near-match review is the safety net, flagging those rows for you instead of silently inserting duplicates.

The import preview shows rows to add, rows skipped as already present, and near-matches to review before anything is written.

### Source formats

**Amex (activity.csv)**

| Column | Example shape | Maps to |
| --- | --- | --- |
| `Date` | `05 Nov 2026` (`%d %b %Y`) | `posted_date` (transaction date) |
| `Date Processed` | `06 Nov 2026` | `processed_date` (new column); differs from `Date` on most rows |
| `Description` | `MERCHANT NAME␣␣␣␣CITY`, space-padded, occasionally quoted with a comma | `description_raw`; merchant and city split on the run of spaces |
| `Amount` | `15.74`, `-2042.40` | `amount_cents`, **sign flipped**: Amex shows charges positive and payments negative |

- UTF-8, comma-delimited, one header row, newest first, no trailing newline.
- No transaction ID, card or cardmember column, Amex category, or foreign-currency column. The hash uses Date, Description and Amount. Date Processed is left out because it adds nothing to identify the transaction and is the field most likely to change if a charge is exported before it settles. Account comes from the import picker, and FX detection relies on description matching.
- Payments show as `PAYMENT RECEIVED - THANK YOU` with a negative amount, which makes them the first transfer-detection rule.
- Sample: about 3.5 weeks and 51 rows from one card.

**Simplii chequing (SIMPLII.csv)**

| Column (raw header) | Example shape | Maps to |
| --- | --- | --- |
| `Date` | `10/07/2026` (`%m/%d/%Y`, US order) | `posted_date` |
| `␣Transaction Details` | `␣INTERAC E-TRANSFER ...`, usually with a leading space | `description_raw` (raw, space kept); merchant is trimmed for display |
| `␣Funds Out` | `25.00` or empty | `amount_cents` negative |
| `␣Funds In␣` | `101.19` or empty | `amount_cents` positive |

- ASCII, LF line endings, trailing newline present (Amex has none), oldest first (Amex is newest first), no quoting in the sample.
- The header has a space after each comma plus a trailing space, so detection compares trimmed header names. Exact bytes are still stored for export.
- Exactly one of Funds Out or Funds In is filled per row, and both are unsigned. No balance column, so the hash uses all four fields as exported.
- Dates are ambiguous on their face (`10/07`); month-first is confirmed because the first part only ranges 10 to 11 and the second up to 30.
- Sample: 12 rows over about 4 weeks, mostly Interac e-transfers plus an internet bill payment.

**Classifying Simplii chequing rows**

- **Interac e-transfers** are a mix of moving your own money and paying people, so no default category is applied: they land in the Uncategorized queue. The description carries the counterparty name, so a rule such as "E-TRANSFER ... LANDLORD" sets Rent once and covers every later row.
- **Own-money e-transfers** are paired automatically when the other side is in an imported account: same amount, opposite sign, within 2 days. Both sides become Transfer.
- **Internet bill payments** are either card payments or real bills, depending on the payee. A payment is paired with a card's `PAYMENT RECEIVED` row (same amount, within 5 days) and becomes Transfer. Otherwise rules on the payee name categorize it as a bill.

**Simplii Visa:** sample still needed.

## Export

**File > Export…** writes one zip that holds a copy of the database plus the provider CSVs rebuilt from it. Every row matches the original export byte for byte.

```
financi-export-2026-11-07.zip
├── financi.sqlite          # snapshot via SQLite backup API / VACUUM INTO
├── manifest.json           # versions, accounts, per-file row count + sha256
└── csv/
    ├── amex-cobalt.csv
    ├── simplii-chequing.csv
    └── simplii-visa.csv
```

**Rebuilding the CSVs**

1. One file per account, with all imports for that account merged into it. Deduplicated rows appear once; real identical repeats appear as many times as they occurred.
2. The header is the provider's original header line, stored with each import. If an account's imports have different headers (a format change), the file is split by header and named with a `-v2` suffix.
3. Each row is the stored raw record, written exactly as read: quoting, spacing and amount text untouched. It is captured as the original bytes of the record, not re-serialized from parsed fields.
4. Rows are in the provider's order (Amex: newest first by `Date`), with ties kept in their original file order.
5. Line endings and the trailing newline match the source files.

**Round-trip guarantee (tested):** importing the export's CSVs into an empty database yields the same multiset of (account_key, row_hash) with identical row bytes, and importing them into the source database adds 0 rows. Rows from different imports are ordered by date, then import order, then `source_line`.

**manifest.json** records the financi and schema versions, the export time, and each CSV's account (name, institution, kind), because the Amex file itself has no account column. It also lists each file's row count and SHA-256.

Your edits (categories, rules, overrides, flag decisions) exist only in `financi.sqlite`. The CSVs hold the provider's data and nothing else.

**Decided**

- One CSV per account, never merged per institution.
- Rebuilt CSVs only. Original files are not archived; the stored raw records and the round-trip test make them redundant.
- Deleting in the app only hides a row (`transactions.hidden INTEGER NOT NULL DEFAULT 0`). Hidden rows stay in the exported CSVs, and their stored hash means a re-import never brings them back.
- **File > Restore…** checks every file against its manifest SHA-256 and the schema version, backs up the current database to `financi.sqlite.bak`, then swaps in the saved one. Categories, rules, overrides and flag decisions come back with it.

## Architecture

A Cargo workspace keeps parsing and storage free of GPUI, so they can be tested without a window.

| Crate | Role | Key deps |
| --- | --- | --- |
| `financi-core` | `Transaction` model, `Category`, money type, normalization, categorization rules | `chrono`, `rust_decimal` |
| `financi-parsers` | One module per institution implementing `StatementParser`; source auto-detection by header | `csv`, `serde` |
| `financi-db` | SQLite schema, migrations, insert with dedup, query functions for the UI | `rusqlite` (bundled), `refinery` or hand-rolled migrations |
| `financi` (bin) | Entry point; launches the GPUI app | `gpui` |
| `financi-ui` | GPUI views: date range picker, bar chart, transaction table, monthly summary | `gpui` |

Parser contract (sketch):

```rust
/// Produced by core, not the parser: core reads the file once and
/// splits it, so raw bytes and positions survive for hashing and export.
pub struct RawFile {
    pub header_raw: Vec<u8>,
    pub encoding: Encoding,          // utf-8 | windows-1252
    pub has_bom: bool,
    pub line_ending: LineEnding,
    pub trailing_newline: bool,
    pub records: Vec<RawRecord>,
}

pub struct RawRecord {
    pub bytes: Vec<u8>,              // exact record bytes, quotes included
    pub source_line: usize,
    pub fields: Vec<String>,         // decoded fields, for mapping
}

pub trait StatementParser {
    /// Stable id, e.g. "amex", "simplii-chequing"
    fn id(&self) -> &'static str;
    /// True if this header looks like ours (auto-detect)
    fn detect(&self, header: &[String]) -> bool;
    /// Map one record into the shared model (sign flip, date parse, ...)
    fn map(&self, record: &RawRecord) -> Result<RawTransaction, ParseError>;
    /// This institution's own dedup hash over its identity columns
    fn row_hash(&self, account_key: &AccountKey, record: &RawRecord) -> RowHash;
}
```

Parsers return `RawTransaction` (institution's own category, raw description). Core then normalizes to `Transaction` and applies financi's own categories, so adding an institution means writing one small module and a fixture-based test.

## Data model

Money is stored as integer cents with a currency code; spending is negative, income and refunds positive. Draft schema, to be revised once the open questions are answered:

```sql
-- Single initial migration (merged from all earlier drafts)
-- Institution (parent) -> accounts (siblings under it), each with a type.
CREATE TABLE institutions (
  id    INTEGER PRIMARY KEY,
  key   TEXT NOT NULL UNIQUE,            -- 'amex', 'simplii'; matches StatementParser::institution()
  name  TEXT NOT NULL                    -- 'American Express', 'Simplii Financial'
);                                       -- institutions with a parser are built in
CREATE TABLE accounts (
  id            INTEGER PRIMARY KEY,
  account_key   TEXT NOT NULL UNIQUE,    -- UUID; used in row_hash and the export manifest
  institution_id INTEGER NOT NULL REFERENCES institutions(id),
  name          TEXT NOT NULL,           -- 'Amex Cobalt'
  kind          TEXT NOT NULL,           -- 'chequing' | 'savings' | 'credit_card'
  currency      TEXT NOT NULL DEFAULT 'CAD'
);

CREATE TABLE categories (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  parent_id  INTEGER REFERENCES categories(id),
  kind       TEXT NOT NULL,              -- 'spend' | 'income' | 'transfer': the only source of transfer status
  UNIQUE (parent_id, name)
);

CREATE TABLE imports (
  id               INTEGER PRIMARY KEY,
  account_id       INTEGER NOT NULL REFERENCES accounts(id),
  parser_id        TEXT NOT NULL,        -- 'amex'
  file_name        TEXT NOT NULL,
  imported_at      TEXT NOT NULL,
  header_raw       BLOB NOT NULL,
  encoding         TEXT NOT NULL,        -- 'utf-8' | 'windows-1252'
  has_bom          INTEGER NOT NULL,
  line_ending      TEXT NOT NULL,        -- 'lf' | 'crlf'
  trailing_newline INTEGER NOT NULL,
  row_count        INTEGER NOT NULL
);

CREATE TABLE transactions (
  id                    INTEGER PRIMARY KEY,
  account_id            INTEGER NOT NULL REFERENCES accounts(id),
  import_id             INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
  source_line           INTEGER NOT NULL,  -- record index within its import
  raw_bytes             BLOB NOT NULL,     -- exact record bytes as exported
  row_hash              TEXT NOT NULL,     -- not unique; see Import and deduplication
  posted_date           TEXT NOT NULL,     -- ISO yyyy-mm-dd
  processed_date        TEXT,
  amount_cents          INTEGER NOT NULL,  -- negative = money out
  original_currency     TEXT,              -- FX, when the CSV has it
  original_amount_cents INTEGER,
  description_raw       TEXT NOT NULL,
  merchant              TEXT,              -- cleaned name
  category_id           INTEGER REFERENCES categories(id),
  category_locked       INTEGER NOT NULL DEFAULT 0,  -- user override; rules skip it
  transfer_pair_id      INTEGER REFERENCES transactions(id),
  hidden                INTEGER NOT NULL DEFAULT 0   -- "deleted" in the app; still exported
);
CREATE INDEX tx_reconcile ON transactions(account_id, row_hash);
CREATE INDEX tx_date      ON transactions(posted_date);

CREATE TABLE row_aliases (              -- older hashes a confirmed near-match still answers to
  transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  row_hash       TEXT NOT NULL,
  import_id      INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
  PRIMARY KEY (transaction_id, row_hash)
);

CREATE TABLE category_rules (
  id          INTEGER PRIMARY KEY,
  pattern     TEXT NOT NULL,             -- substring or regex on description
  category_id INTEGER NOT NULL REFERENCES categories(id),
  priority    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE flags (                      -- after v1
  id             INTEGER PRIMARY KEY,
  transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  kind           TEXT NOT NULL,          -- 'large' | 'new_merchant' | 'fx_fee'
  detail         TEXT,
  status         TEXT NOT NULL DEFAULT 'open',  -- 'open' | 'dismissed' | 'confirmed'
  UNIQUE (transaction_id, kind)
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);  -- flag thresholds, UI prefs
```

Recurring-series storage is deferred along with the recurring view. SQLite runs in WAL mode on one DB worker thread; the UI sends queries to it and re-queries on a "data changed" event.

## UI requirements (GPUI)

One window laid out as a single dashboard: filters on top, the category bar chart and monthly summary side by side, the transaction table below, all linked through one shared filter state (date range, accounts, selected category).

**Global controls**

- Date range: presets (this month, last month, last 3 / 6 / 12 months, YTD, all) plus custom start/end.
- Account filter: multi-select; default all.
- Toggle: include or exclude transfers and credit card payments (default exclude).

**1. Category bar chart**

- One bar per category, total spend in the selected range, sorted largest first.
- Hover shows the exact total and share of all spend.
- Click a bar to set the category filter on the table; click again or press Esc to clear.
- GPUI has no chart library, so bars are drawn as sized `div`s or with its path/canvas APIs; a horizontal bar layout keeps long category names readable.

**2. Transaction table**

- Columns: date, account, merchant/description, category, amount.
- Filtered by the global controls and the chart's selected category; text search on description.
- Sortable by date or amount; virtualized list (`uniform_list`) so thousands of rows stay smooth.
- Inline recategorize: change a row's category, optionally "apply to all matching" which creates a rule.

**3. Monthly summary table**

- Rows = categories, columns = months in range, cells = spend; a total row and column.
- Clicking a cell sets both the category and that month as the filter.

Add a total-income row, a total-spend row and a net row, one column per month.

**4. Spend over time**

- Line chart of monthly spend, either total or for the selected category, with income and net as optional series.

**5. Top merchants**

- Ranked by total spend in range, showing transaction count and average. Clicking a merchant filters the table.

**6. Recurring / subscriptions**

- Detect merchants charged on a regular cadence (monthly, yearly) at a similar amount. Show cadence, last charge, next expected charge and yearly cost.
- Flag price increases and subscriptions that stopped.

**7. Flagged transactions**

- After v1. A review list of unusual transactions, each with the reason it was flagged. You can dismiss a flag or confirm it, and a dismissed flag never comes back.
- Rules (thresholds in settings):
    - **Unusually large:** amount above 3x the median for that merchant, or for the category if the merchant has fewer than 3 past charges.
    - **New merchant:** the first time this cleaned merchant name appears.
    - **FX / fees:** foreign currency, or a description matching FX fee, interest, NSF, late fee or annual fee.
- Possible duplicates are handled at import by near-match review, not as a flag, because identical repeat purchases are kept on purpose.

## Review findings

Three independent reviews (data integrity, Rust/GPUI feasibility, product scope) on 2026-11-07 all concluded the plan is not ready for M0 yet. The import and dedup core is sound; these must be settled first.

| # | Finding | Resolution | Status |
| --- | --- | --- | --- |
| 1 | The hash used the local `account_id`, so a restore or fresh database would re-import everything as duplicates. | Hash a stable `account_key` UUID; length-prefix the hashed fields. | Adopted |
| 2 | Confirming a near-match left no record, and the rule ignored the description. | `row_aliases` table counted in reconciliation; similar description required; the "possible duplicate" flag was dropped. | Adopted |
| 3 | A wrong-account import couldn't be undone, and every Amex file is named `activity.csv`. | Always confirm the account, warn on overlap with another account, undo the latest import per account. | Adopted |
| 4 | Nearly all rows start Uncategorized, but categorizing tools came last. | The categorize workflow is now M2, before the core views. | Adopted |
| 5 | TEXT "lines" can't guarantee byte-exact export. | `raw_bytes BLOB`, plus encoding, BOM, line ending and trailing newline per import; the guarantee is restated as a multiset. | Adopted |
| 6 | The parser trait consumes a reader, so raw bytes, line numbers and the header are lost. | Core reads the file into a header and raw records. Each parser keeps its own `row_hash`, but takes a `RawRecord` instead of a reader. | Adopted |
| 7 | Transfer state lived in two places, and categories had no type. | `categories.kind` (spend / income / transfer) is the only source. The payment rule runs before refund netting. | Adopted |
| 8 | Schema drafts disagreed. | Merged into one initial migration in Data model. | Done |
| 9 | GPUI's dependency situation, and a component kit with charts, tables and pickers, are reported but unverified. | Verified in M0: gpui-kit =0.9.0 on gpui-pre =0.3.11, with runtime_shaders so no full Xcode is needed. File drop, BarChart, virtual DataTable and Select all work. BarChart has no click handler, so the clickable category bars are built by hand. | Done |
| 10 | SQLite threading was unspecified. | One DB worker thread, WAL mode, a "data changed" event. | Adopted |
| 11 | Nothing was usable before M3, and M4 bundled five views. | The import window moves to M1. The extras move after v1. | Adopted |
| 12 | No acceptance criteria beyond the round-trip test. | For example: 2 years imports in under 2 s; seeded rules categorize 70% or more; an overlapping re-import adds 0 rows; totals match the statement. | Proposed |

## Decisions

| Topic | Decision |
| --- | --- |
| Platforms | macOS and Linux. Windows is not a v1 target. |
| Database location | Per-OS app data directory via the `directories` crate: `~/Library/Application Support/financi/financi.sqlite` on macOS, `$XDG_DATA_HOME/financi/financi.sqlite` (default `~/.local/share/financi/`) on Linux. Plain SQLite; disk encryption (FileVault / LUKS) is relied on. |
| Accounts (v1) | Amex credit cards (more than one), Simplii chequing, Simplii Visa. Accounts sit under an institution (American Express, Simplii Financial) and have a type: chequing, savings or credit card. Each parser declares its institution and the account types it applies to, so a Simplii Visa file can't go into Simplii chequing. |
| Account mapping | Always confirmed in the import preview, never guessed from the filename. New accounts are created inline from the preview's account picker. If even one row of the file already exists under a different account, the import is blocked with an error naming that account, and goes ahead only if you explicitly override. |
| Import cadence | Exports of the last 30 to 90 days every so often, so overlapping ranges are normal. |
| Dedup | Per-institution hash of the stable account key plus that source's raw identity fields (Amex: Date, Description, Amount), combined with count reconciliation and near-match aliases. No file hash. |
| Undo | The latest import per account can be undone (hard delete). Deleting a single transaction only hides it. |
| Import UI | GUI only: drag-and-drop into the GPUI window, with a preview before you confirm. No CLI. |
| Export / restore | GUI menu items only. No CLI anywhere; the binary just launches the app. |
| Categories | financi owns one category list. Description-match rules assign categories; a manual override locks a row. Amex's CSV has no category column, so rules do all the categorizing. |
| Category levels | Two levels (parent > child), confirmed. The bar chart shows parents, and clicking one drills into its children. Transactions point at a child, and a child points at its parent, so moving or renaming is a one-row update. |
| Category kinds | Each category is spend, income or transfer. Transfer status comes only from this. Real use starts with an empty list; a dev-only seed (categories, rules, synthetic transactions) is loaded for testing. |
| Refunds | Positive amounts in a spend category net against that category. |
| Currency | CAD only, the amount the card charged. The original currency and amount are kept when the CSV has them. |
| Transfers | Card payments and transfers between your own accounts are auto-detected as Transfer and excluded from spend totals by default. An unpaired card payment still counts as a transfer. |
| v1 scope | Import, categorize workflow, category bar chart, transaction table, monthly summary with income, spend and net, Simplii, export/restore. |
| After v1 | Spend-over-time line and top merchants (v1.1), flags (v1.2), recurring/subscriptions (v1.3). |

## Open questions

Each answer gets folded back into the sections above.

**Data and import**

- [ ] Share the Simplii Visa CSV as it is on disk, unedited, so the encoding, BOM, sign convention and any balance column can be checked. Needed by M4, not M0.
- [ ] Agree on acceptance criteria (see review finding 12).

## Milestones

Each phase ends with something usable; dates are not set yet.

**v1**

1. **M0: Skeleton + GPUI spike.** Workspace, merged schema migration, DB worker thread, and a dev-only seed (categories, rules, synthetic transactions) for testing. A throwaway window proving the GPUI dependency and component kit: file drop, a bar chart, a virtual table, a dropdown. Pin the versions exactly and commit `Cargo.lock`.
2. **M1: Amex import, end to end.** Raw-record reader, Amex and Simplii chequing parsers, hashing, count reconciliation. Account setup, the import preview with account confirm and overlap warning, undo import. Usable: drop an Amex file and browse its rows.
3. **M2: Categorize.** Category editor (two levels), rules with a match preview, Uncategorized queue, bulk assign, rule from merchant, re-run rules. The card-payment rule marks transfers.
4. **M3: Core views.** Date range and account filters, category bar chart, transaction table with bar-click filtering, monthly summary with income, spend and net.
5. **M4: Simplii + near-match.** Simplii Visa parser (once a sample arrives), transfer pairing across accounts, near-match review with aliases.
6. **M5: Export / restore.** File menu items, manifest, round-trip test.

**After v1**

- v1.1: spend-over-time line, top merchants.
- v1.2: flags (large, new merchant, FX/fees) and flag review.
- v1.3: recurring / subscriptions.
- Later: more institutions, CSV export of filtered rows.
