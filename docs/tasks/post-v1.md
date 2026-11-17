# After v1 — roadmap and tasks

Written 2026-11-17 from `plan.md` → *After v1* and the parked items in
`docs/tasks.md`. **None of this starts before v1 is complete** (3.55 done).
Each milestone ends in a tagged minor release. Within a milestone, a
backend task comes before its UI task; the backend task of the next
milestone may run beside the current milestone's UI task. Task numbers
follow the milestone (6.x for M6, and so on), like M4 and M5.

| Milestone | Release | What | Tasks |
|---|---|---|---|
| M6 | v1.1 | Spending over time and top merchants on the dashboard | 6.1–6.4 |
| M7 | v1.2 | Flags: unusually large, new merchant, FX and fees | 7.1–7.3 |
| M8 | v1.3 | Recurring charges and subscriptions | 8.1–8.3 |
| M9 | v1.4 | First-run defaults and CSV export of filtered rows | 9.1–9.3 |
| M10 | v1.5 | More institutions | 10.1–10.2 |

The dashboard is a dock after 3.100: every new dashboard view is a new
dock panel entity (3.97 pattern) registered in the layout's
`PanelRegistry` (3.101). New panels aren't in the default layout. Users
add them with *View > Show panel*, built in 6.2. Every new page follows
`docs/ui-conventions.md`.

## M6 — v1.1: spending over time, top merchants

- [ ] **6.1 Spend-over-time query.** Backend.
  `spend_over_time(conn, filter, narrow, bucket) -> Vec<TimePoint>` in
  `financi-db/src/views.rs`, built on the shared filter WHERE builder
  (3.88) so every dashboard filter applies. `bucket` is `Month` or
  `Week` (ISO weeks); `TimePoint { start: NaiveDate, expense_cents,
  income_cents, net_cents }`, with empty buckets filled with zeros across
  the range. A selected category narrows to it (children included). Same
  refund netting and transfer exclusion as `monthly_summary`.
  - Tests: monthly totals equal `monthly_summary`'s Spend / Income / Net
    rows for the demo data; week buckets sum to the month; gaps are
    zero-filled; a category narrow and the accounts filter apply.

- [ ] **6.2 Spending over time panel.** UI, after 6.1. A dock panel
  *Spending over time*: gpui-kit `LineChart`
  (`gpui-component-0.9.0/src/chart/line_chart.rs`) of Expense, with
  Income and Net as optional series toggled by legend chips (Expense on
  by default). Month buckets, or weeks when the range is 3 months or
  shorter. Hover tooltip `Oct 2026` over the three amounts. Clicking a
  point selects that month (the 3.46 month selection). Follows the
  selected category. Add *View > Show panel* (lists hidden panels; picking one
  docks it to the right of the center) so new panels can be added to a
  saved layout.
  - Tests: points equal 6.1's for the range; toggling Income adds a
    series; clicking a point sets the month selection and filters the
    table; a selected category narrows the line; the panel opens from
    *View > Show panel* and is remembered (3.101).

- [ ] **6.3 Top merchants query.** Backend.
  `top_merchants(conn, filter, narrow, limit) -> Vec<MerchantStat>`:
  cleaned merchant, total spend, count and average, ordered by total,
  expense rows only, under every dashboard filter.
  - Tests: totals and counts on the demo data; refunds net; the limit
    applies; filters apply.

- [ ] **6.4 Top merchants panel.** UI, after 6.3. A dock panel *Top
  merchants*: a `DataList` (Merchant, Total, Count, Average; sortable
  headings like the Rules page) of the top 50. Clicking a row sets the
  table's search to that merchant (the same as typing it). The selected
  row stays highlighted while the search matches.
  - Tests: rows equal 6.3's; clicking filters the table; sorting by
    each column; empty range shows "No spending in this range".

## M7 — v1.2: flags

`plan.md` → *After v1* has the rules. The `flags` table exists since
0001 (`kind` in large / new_merchant / fx_fee, `status` open / dismissed
/ confirmed, unique per transaction and kind).

- [ ] **7.1 Flag detection.** Backend. `detect_flags(conn, import_id)`
  runs after every import (after transfer pairing) over that import's new
  rows, and `detect_all_flags(conn)` runs once on upgrade, recorded in
  `settings`. Thresholds live in `settings` with defaults: large = 3× the
  merchant's median (category median if the merchant has fewer than 3
  past charges); new merchant = first appearance of the cleaned name;
  FX / fees = a foreign currency, or a description matching FX fee,
  interest, NSF, late fee or annual fee (case-insensitive list in
  settings). Insert with `ON CONFLICT DO NOTHING` so a dismissed flag
  never comes back. Transfers are never flagged. Undo import removes its
  flags (cascade).
  - Tests: each kind on synthetic rows; a dismissed flag survives
    re-detection; re-import adds no flags; transfers and hidden rows are
    skipped; thresholds read from settings.

- [ ] **7.2 Flag queries and actions.** Backend.
  `list_flags(conn, status)` (with transaction fields),
  `set_flag_status(conn, id, status)`, `open_flag_count(conn)`, through
  `DbWorker`, with a new `Domains::FLAGS` change domain (3.78).
  - Tests: listing order (newest first), status changes, count.

- [ ] **7.3 Flags page.** UI, after 7.2. A sidebar page *Flags* under
  *Organize*, with an open-count pill. A `DataList` grouped by kind
  (Unusually large, New merchant, FX and fees) of open flags: date,
  merchant, amount, a muted reason ("3.4× the usual $42.10"), and Dismiss /
  Looks right row actions (both remove it from the list optimistically,
  3.84). A "Show dismissed" toggle. A dashboard table row with an open flag
  gets a small flag icon. A *Flag thresholds* section edits the settings.
  - Tests: the list and pill match the seed's flags; Dismiss removes the
    row in the same frame and lowers the count; the toggle shows it again;
    a threshold change re-runs detection for new imports only.

## M8 — v1.3: recurring and subscriptions

- [ ] **8.1 Recurring detection.** Backend. A new migration with
  `recurring_series (id, merchant, cadence_days, amount_cents, first_seen,
  last_seen, status)` and `recurring_members (series_id,
  transaction_id)`. `detect_recurring(conn)` groups expense rows by
  cleaned merchant and finds a regular cadence (weekly, monthly,
  quarterly, yearly within ±20 % day tolerance; at least 3 occurrences)
  at a similar amount (±15 %). It runs after every import and rule change
  and keeps user-set statuses (ignored, confirmed) across runs.
  - Tests: monthly, yearly and weekly synthetic series; jittered dates;
    a price change stays in one series; irregular merchants aren't
    series; an ignored series stays ignored.

- [ ] **8.2 Subscription signals.** Backend. Per series: next expected
  date, yearly cost, price increase (latest amount > previous by more than
  5 %), and stopped (no charge for 1.5 × the cadence past the expected
  date). Exposed through `list_recurring(conn)`.
  - Tests: each signal on synthetic series.

- [ ] **8.3 Recurring page.** UI, after 8.2. A sidebar page *Recurring*:
  a `DataList` of series (merchant, cadence, last charge, next expected,
  yearly cost) with tags for "price up" and "stopped", a total yearly
  cost in the `PageHeader` summary, and Ignore / Confirm row actions.
  Clicking a row shows its charges under the list (the 3.52 pattern).
  - Tests: rows and yearly total match 8.2; tags show; Ignore hides the
    series and survives a re-detection; clicking lists its charges.

## M9 — v1.4: first run and export

- [ ] **9.1 Default categories and rules on first run.** Backend, per
  `plan.md` → *Categorization* → *Default categories and rules*. A
  curated list (about 15 categories with subcategories, and about 150
  substring rules for common Canadian merchants) as a data file in
  `financi-db`. It's inserted once at the real app's first run, only into a
  database with no transactions and no user categories, and recorded in
  `settings`, so deleted defaults never return. Not in dev-seed runs or
  migrations.
  - Tests: a fresh DB gets them once; a second start inserts nothing;
    a DB with a user category or a transaction gets nothing; dev seed
    skips them; importing the demo files categorizes most rows (assert
    a floor like 60 %).

- [ ] **9.2 Export filtered rows as CSV, backend.**
  `export_filtered_csv(conn, filter, narrow, table_filter, path)`:
  the rows the dashboard table shows (same filter and sort), as UTF-8 CSV
  with Date, Account, Merchant, Category, Subcategory, Amount (signed,
  `1234.56`), Description. Written atomically (temp file and rename).
  - Tests: the rows and order equal `list_transactions_filtered`;
    quoting of commas and quotes; hidden rows only when Show hidden.

- [ ] **9.3 Export filtered rows as CSV, UI.** After 9.2. *File > Export
  table as CSV…* and an icon button (Lucide `download`, tooltip "Export
  these rows as CSV") at the right of the transactions panel's title. A
  save dialog defaults to `financi-<from>-<to>.csv`. Shows a toast with
  the row count.
  - Tests: the action writes the shown rows to the chosen path (test
    hook instead of the dialog); the toast shows the count.

## M10 — v1.5: more institutions

- [ ] **10.1 Widen the parser trait.** Backend, before any new parser.
  `detect` sees the file (`&RawFile` header and first records), not just
  the header row. A parser can skip preamble rows and pick its header row.
  Each parser supplies its institution's display name, replacing the
  match in `flow::format_label`. Amex and Simplii keep their hashes:
  their `row_hash` inputs don't change. Re-run the real-sample check
  (`CLAUDE.md`).
  - Tests: existing parser tests unchanged; a fixture with two preamble
    lines is detected and parsed by a test-only parser; display names
    come from the parser.

- [ ] **10.2 The next institution.** Needs a real export from the user
  first (ask which bank and get a sample; never commit it). Then: one
  parser module, a synthetic fixture with the same shape, detection and
  mapping tests, and its institution row in a new migration. Also
  revisit 4.1 (Simplii Visa's assumed format) if a real Visa sample is
  available by then.
