# financi — implementation tasks (M2 → M5)

This is the working checklist for the remaining v1 milestones. It turns
`plan.md` (the requirements, copied from the plan doc) into concrete tasks.
Where the two disagree, **this file wins** for M2–M5 detail.

## How to work

1. Do **one milestone at a time**. When every box in it is ticked, continue
   to the next milestone without waiting for a go-ahead. Stop and report to
   the user when v1 is complete.
2. Work the tasks in order, running two at once only when `AGENTS.md` allows
   it. Tick a box (`- [x]`) in the same commit that completes it.
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

- [x] **2.2 Master CSVs** (`financi-db/src/master_csv.rs`).
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

- [x] **2.3 Category model** (`financi-db/src/categories.rs`).
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

- [x] **2.4 Rule engine** (`financi-core` for matching, `financi-db` for
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

- [x] **2.5 Applying rules.**
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

- [x] **2.6 Manual edits.**
  - `set_category(transaction_id, category_id)` sets the category and
    `category_locked = 1`. `clear_override(id)` unlocks and re-applies rules.
  - After a manual edit the UI offers *"Apply to all matching \"MERCHANT\""*,
    which creates a substring rule from the cleaned merchant.
  - Tests for lock/unlock behaviour.

- [x] **2.7 Uncategorized queue query.** `uncategorized_by_merchant(conn) ->
  Vec<{merchant, count, total_cents, first_date, last_date}>`, sorted by
  count descending. Choosing a category for a group creates a substring rule
  on that merchant (which categorizes the whole group via 2.5).

- [x] **2.8 Sidebar shell + category pages.** Layout and look:
  `docs/ui-conventions.md` (*Window layout*). Replace the M1 top bar with a
  gpui-kit `Sidebar` (Dashboard, To categorize, Categories,
  Transfers, Imports; counts in item suffixes) and one page area. Move the
  import preview and Undo to the Imports page (with import history). Then
  the category pages, which together cover:
  - a parent/child tree: add parent (name + kind), add child, rename, move
    child, delete (with a "move its transactions to…" picker);
  - a rules list: pattern, substring/regex toggle, target category, and a
    live match preview ("matches 14 transactions") before saving;
  - the Uncategorized queue (2.7) grouped by merchant, each with a category
    picker.
  - UI tests in `ui_tests.rs` style for: create parent+child, create a rule
    and see rows categorized, categorize a queue group, delete with reassign.

- [x] **2.9 Table context menu.** Right-click a transaction row (use
  `TableDelegate::context_menu`): *Change category…* (then offer the rule as
  in 2.6), *Hide*, and *Unhide* (only when hidden rows are shown, see 3.4).

**Done when:** the demo files import and every row can be categorized from
the UI; tests cover 2.1–2.9.

---

## M3 — Core views (one dashboard)

Layout, top to bottom: filter bar; then the category chart + legend (left)
beside the monthly summary (right); then the transaction table.

- [x] **3.1 Filter model + queries** (`financi-db/src/views.rs`), all taking
  one `ViewFilter { from: NaiveDate, to: NaiveDate, account_ids: Vec<i64>
  (empty = all), include_transfers: bool }`.
  - Spend/income rules: a row counts as **spend** if its category's kind is
    `expense` *or it is uncategorized*; **income** if kind `income`;
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

- [x] **3.2 Date range control.**
  - Presets: This month, Last month, Last 3 months, Last 6 months, Last 12
    months, Year to date, All time, Custom….
  - "Last N months" = whole calendar months: the current month plus the N−1
    before it (on 2026-11-07, Last 3 months = 2026-09-01 … 2026-11-30).
  - Custom: two gpui-kit `DatePicker`s (start, end).
  - Default **Last 3 months**. Persist the chosen preset (and custom dates)
    in the `settings` table under `ui.date_range`; restore at launch.
  - Unit-test the preset → date range function, including year boundaries.

- [x] **3.3 Accounts filter + transfers toggle.** Multi-select of accounts
  (default all) and an "Include transfers" checkbox (default off, persisted
  as `ui.include_transfers`). gpui-kit has no multi-select: build the
  accounts filter as a dropdown popover of gpui-kit checkboxes, one per
  account.

- [x] **3.4 Transaction table upgrade.**
  - Filtered by the `ViewFilter`, the selected category (parent or child)
    and an optional month (set from the summary).
  - Text search box: case-insensitive match on description or merchant.
  - Sortable columns: Date (default, newest first) and Amount.
  - Toggles: "Uncategorized only" and "Show hidden" (hidden rows greyed,
    with Unhide in the context menu).
  - Amounts: signed, `$1,234.56`, money in shown in green.

- [x] **3.5 Category chart + legend.**
  - gpui-kit `BarChart` (spend per category, values positive) for the
    picture; beside it a **clickable legend list** (name, `$1,234.56`, %)
    because `BarChart` can't take clicks.
  - Clicking a parent in the legend filters the table to that parent and
    switches chart + legend to its children, with "← All categories" to go
    back. Clicking a child filters further; clicking it again or pressing
    Esc clears.

- [x] **3.6 Monthly summary table.** Rows = parent categories (expandable to
  children) then Income, Spend, Net; columns = months in range plus Total.
  Clicking a cell filters the table to that category and month. Summary
  amounts are positive `$1,234.56`.

- [x] **3.7 Native menus + shortcuts.** macOS menu bar via GPUI menus:
  File → Import CSV… (⌘O), Undo last import, Export… (⌘E, wired in M5),
  Restore… (wired in M5), Quit (⌘Q). Import CSV… is the Imports page's
  header action (not in the sidebar); Undo lives on the Imports page and in the File menu
  (`docs/ui-conventions.md`, D4). No action may depend on the native menu,
  which may not appear on Linux: the Imports page exposes Import, Undo,
  Export and Restore on both platforms.

- [x] **3.8 UI tests** for: preset change updates totals, legend drill-down
  and back, summary cell click filters the table, search, hidden toggle.

- [x] **3.9 Import end-to-end UI test.** Import a synthetic fixture through
  the real import flow, then visit every page and check the values shown:
  Dashboard rows and amounts, To categorize, Categories counts, Imports
  history and sidebar counts, against `fixtures/demo/expected.json` where
  possible. Must not depend on today's date.

- [x] **3.10 Blank Dashboard after import.** The Dashboard shows nothing even
  though transactions were imported. Find the cause and fix it. If the cause
  is only that the data falls outside the default Last 3 months, ask the user
  how the default or the empty state should change.

- [x] **3.11 Stale page content.** Content from the previous page stays on
  screen after switching pages. Find the cause and fix it.

- [x] **3.12 Virtualize long lists.** The category tree and the rule list
  build every row, so the Categories page gets slow with many categories or
  rules. Render only the visible rows, keeping selection, clicks and
  scrolling. Test with thousands of rows.

- [x] **3.13 Sidebar indent.** Remove the indent on sidebar items so they
  line up with the section labels.

- [x] **3.14 Checkbox check mark.** A checked box changes background but
  shows no check. Find the cause and fix it for every checkbox.

- [x] **3.15 Responsive charts.** Chart lines overlap the labels, pieces are
  cut off, and the layout doesn't adapt to the space available. At the
  minimum window size (1200 × 720) and above, every chart stays inside its
  panel, nothing is clipped, and lines and labels don't overlap. Shorten or
  thin out labels when space is tight. Check it at the minimum size and a
  large window, with long category names and many categories.

- [x] **3.16 Labelling and layout pass.** Every input, dropdown, date picker
  and filter group has a visible label or section heading (a placeholder
  alone doesn't count). Group related controls deliberately instead of
  stacking rows of unlabelled boxes. Record the rule in
  `docs/ui-conventions.md`. Labels and layout only, no new features. Do this
  last: it also needs to cover the views from 3.5 and 3.6.

- [x] **3.17 Stale account filter on import.** `Shell::refresh` computes
  `none_selected` from the accounts list of the previous refresh, so
  importing while every account checkbox is unchecked leaves every view
  blank until some other filter changes. Recompute it from the accounts
  loaded in the same refresh, and test it.

- [x] **3.18 Blank Categories page after virtualization.** Since 3.12
  (commit b2739d0) the Categories page opens blank: the tree shows no rows
  and the rule list shows nothing, even though categories and rules exist
  (the counts and other pages show them). Rows only appear after adding a
  category or a rule. Find the cause and fix it. The 3.12 tests seed data
  before the page first renders, so they miss this path. Add tests that
  open the page first and load categories and rules afterwards, the way a
  normal launch does (async `DbWorker` refresh), and that check rows show
  up on that first open without any other action. Cover both the tree and
  the rule list, and switching to the page from another page.

- [x] **3.19 Virtualize the To categorize queue.** `render_queue` builds
  every merchant group as a row inside a plain scrolling div, and
  `sync_queue` keeps a `SelectState` entity per merchant and calls
  `set_items` on every one of them on each refresh. With a large queue the
  page is very slow to render and laggy to scroll. Render only the visible
  rows (as in 3.12), and stop building or updating a picker for every
  group up front: create pickers only for rows that are shown, or share
  one. Keep categorizing a group, the error message and the empty state
  working. Test with thousands of groups: the page renders, builds only
  the visible rows, and a group at the very bottom can still be
  categorized.

- [x] **3.20 Imports history table overflows.** The history table's fixed
  column widths add up to 1,110 px (150 + 240 + 240 + 110 + 220 + 150),
  wider than the content area beside the sidebar at the minimum window size
  (1200 × 720). The table overflows and the Undo… button is cut off in its
  column, worse once it reads "Click again to undo". Make the table fit the
  space available from the minimum window size up: let File and Account
  shrink and truncate with an ellipsis, and size the Undo column so both
  button labels show in full. Check it at the minimum size and a large
  window, with long file and account names.

- [x] **3.21 Virtualized lists don't scroll.** Since 3.12 / 3.19 the
  category tree, the rule list and the To categorize queue show only their
  first screen of rows. Wheel and trackpad scrolling do nothing, so rows
  past the fold can't be reached. Cause: gpui-base's `virtual_list()`
  (`gpui-base-0.9.0/src/virtual_list.rs`, ~line 186) creates a new
  `VirtualListScrollHandle` on every call, and the list's div
  `track_scroll`s it. GPUI reads the scroll offset from a tracked handle in
  preference to the element state (`gpui-pre-0.3.11/src/elements/div.rs`,
  ~line 2323), and `Shell` re-renders on every frame, so the offset goes
  back to 0 each time. Fix: keep one `VirtualListScrollHandle` per list as
  a `Shell` field (`tree_scroll`, `rule_scroll`, `queue_scroll`) and pass
  it with `.track_scroll(&handle)` on each `v_virtual_list(...)` in
  `ui.rs`. Reset a handle's offset only when its content is actually
  replaced (for example the rule list when the selected category changes in
  3.23), not on every refresh. Tests, for each of the three lists, seeded
  with thousands of rows: scroll it (`simulate_event` with a
  `ScrollWheelEvent` over the list's bounds, or `handle.set_offset`),
  re-render, then check the offset is kept, the built range has moved
  (rows near the end are built and the first rows aren't), and the offset
  survives an unrelated `cx.notify()` / refresh.

**Order for the open M3 tasks** (added 2026-11-10; the numbers record when
a task was written, not when to do it): **3.24 → 3.25 → 3.27 → 3.30 →
3.31 → 3.32 → 3.33 → 3.23 → 3.28 → 3.29 → 3.26** (3.22 is done).
3.33 comes before 3.23 so the re-laid-out page is written with the new
words. 3.24 comes
first because it is the only one that blocks the user: a wrong rule can't
be corrected today. 3.25, 3.27, 3.30, 3.31 and 3.32 are small layout fixes
that give back space on the dashboard and other pages. 3.28 builds on 3.24's row editor, and 3.29 on 3.28's page. 3.26 goes
last because it attaches scrollbars to lists that 3.23, 3.28 and 3.30 lay
out.

- [x] **3.24 Edit and delete rules.** The Categories page can add a rule but
  can't change or remove one, so a wrong pattern keeps miscategorizing
  every import. The storage side exists and is tested
  (`financi_db::rules::update_rule` and `delete_rule`; both validate, then
  re-run all rules over unlocked rows per 2.5). This task is UI only.
  - **Row actions:** each rule row gets **Edit** and **Delete…** at its
    right end (small ghost buttons, visible on every row; no hover-only
    controls). Rows stay 24 px and virtualized; create the buttons only
    for visible rows.
  - **Edit** swaps that row, in place, for an editor in the same shape as
    the add-rule row: Pattern input, regex switch, Category picker (all
    pre-filled from the rule), then Save (primary) and Cancel. It shows
    the same live "matches N transactions" count as add-rule
    (`count_rule_matches`). One rule is edited at a time; opening another
    row's editor, selecting another category or leaving the page cancels
    it. Enter saves, Escape cancels. A validation error (empty pattern,
    bad regex) shows in `danger` under the editor and keeps it open.
  - **Delete…** uses the two-click confirm from `ui-conventions.md` ("Click
    again to delete"), armed per row; clicking anywhere else disarms it.
    No modal.
  - After a save or delete, refresh every view (the rule list, the tree's
    transaction counts, the To categorize count in the sidebar, the
    dashboard), the same way `add_rule` does. Both calls go through
    `on_db`.
  - 3.23 re-lays out the rule list. It must keep these actions: add an
    actions column to its heading row and widths.
  - Tests (`ui_tests.rs`): editing a rule's pattern recategorizes the
    matching unlocked rows and leaves locked rows alone; changing its
    category moves them; an invalid regex shows the error and saves
    nothing; Cancel and Escape leave the rule unchanged; Delete… needs two
    clicks, and after it the rule is gone and its rows are Uncategorized
    (or picked up by another rule); with thousands of rules only visible
    rows build buttons. Update the *Categories* row of
    `docs/ui-conventions.md` in the same commit.

- [x] **3.25 Dashboard alignment.** Since the labelling pass (3.16) the
  dashboard's controls sit at uneven heights and the regions don't share
  edges. Start by taking a screenshot (see *GPUI pitfalls* → Screenshots)
  at 1200 × 720 and at a large window, and list what is off before
  changing code. Known causes in `render_dashboard`:
  - Both filter rows are `h_flex().items_end()` mixing labelled controls
    (label + 32 px control) with unlabelled checkboxes and the
    "N transactions" text, so the checkboxes and the count hang at the
    controls' bottom edge instead of centering on them.
  - The two filter rows use unrelated widths (150 / auto / 240), so
    nothing in row two sits under anything in row one.
  - The transaction count floats mid-row after the last checkbox.

  Fix, layout only (no new controls, no behaviour change):
  - Every item in a filter row is a column of the same shape: a label
    line (`field_label`, or an empty line of the same height for
    checkboxes) over a 32 px control line in which the content is
    vertically centered. Controls and checkboxes then share one center
    line, and all labels share one baseline.
  - Row two's first column (Search) has the same left edge and width as
    row one's first column, or the two rows merge into one when they fit
    at 1200 px. Pick whichever reads cleaner in the screenshot and record
    it in `ui-conventions.md`.
  - The transaction count moves to the right end of the first filter row
    (`ml_auto`), vertically centered on the control line.
  - The filter region, the chart panel, the monthly summary and the table
    share the page's left and right edges exactly; the chart panel and
    summary have equal heights, and their region titles sit at the same
    y. Inside each panel, titles and content share one left edge
    (`p_4`).
  - Spacing stays on the 4/8/12/16 scale.
  - Tests: at 1200 × 720 and 1600 × 1000, the vertical centers of every
    control and checkbox in a filter row are within 1 px of each other;
    the *Spending by category* chart panel and the *Monthly summary* panel
    have the same top and the same height (within 1 px), whatever the
    number of months or legend entries; the left edges of the filter
    region, chart panel and table are equal,
    and so are the right edges of the filter region, summary panel and
    table; the *Spending by category* and *Monthly summary* titles have
    the same top; with the Custom date range open (From / To visible) the
    row still fits and stays aligned. Attach before/after screenshots to
    the handoff.

- [x] **3.22 To categorize queue fits the window.** At the minimum window
  size the queue's fixed columns (280 + 140 + 120 + dates + 260 + button +
  gaps ≈ 1,120 px) are wider than the ~958 px content area, so rows are cut
  off on the right and the Dates cell has no width at all. Make it fit from
  1200 × 720 up, with no horizontal scrolling:
  - **Merchant:** `flex_1().min_w_0()` with ellipsis, so it takes the space
    left over (≥ ~180 px at the minimum size). Each merchant cell gets an
    `id` and a hover `.tooltip(...)` (gpui-base `Tooltip`) showing the
    full merchant name.
  - **Transactions:** the heading says "Transactions" and the cell shows
    just the number (`flow::count`), right-aligned, ~96 px. Drop the
    "N transactions" text.
  - **Total:** ~110 px, right-aligned (unchanged apart from the width).
  - **Dates:** fixed ~190 px, `min_w_0` with ellipsis, `text_sm` muted.
  - **Category picker:** ~220 px. **Categorize** button: natural width.
  - Remove the `flex_1` spacer. The heading row and the data rows share the
    same widths (pull them into consts) so headings stay over their columns.
  - Tests: at 1200 × 720 with a 120-character merchant name, the row's
    right edge (the Categorize button's bounds) is inside the page's
    bounds; each heading's left edge matches its column's left edge (extend
    `the_queue_category_heading_sits_over_the_picker_column` to every
    column); the Transactions cell text is a bare number. Update the
    *To categorize* row of `docs/ui-conventions.md` if it changes.

- [x] **3.23 Categories page layout.** The page reads as scattered
  controls: the new-parent form floats above everything, the detail panel
  is a wrapping row of unrelated pickers, the rules list under it shows
  every rule in the database no matter which category is selected, and
  spacing (`gap_6`, `gap_4`) is off the 4/8/12/16 scale. Lay it out as
  follows. Layout only: no new features, and the existing actions (add
  parent and child, rename, move, delete with its two-click confirm, add
  rule with its live match count) keep working.
  - **Header row:** "Categories" (`text_lg`) and a muted summary
    ("N parents · M categories") on the left; **New category…** at the
    right end (the header action in `ui-conventions.md`). Clicking it
    shows the new-parent form (Name, Kind, Cancel, Add parent) as one
    bordered row under the header, in place, the way the rename editor
    works. Hide the form again after Add parent or Cancel.
  - **Body:** two bordered panels side by side (`gap_3`) that fill the
    remaining height, with their tops and bottoms aligned.
  - **Left: Category tree panel** (300 px, `p_2`). The *Category tree*
    title sits inside the panel, then the virtualized list, which scrolls
    (3.21). Each 32 px row is full width: a 10×10 category color swatch
    (`ui-conventions.md` → *Category colors*), the name (`min_w_0` +
    ellipsis; parents semibold, children indented 16 px), a muted "system"
    tag when relevant, and the transaction count right-aligned in a fixed
    column at the right edge so counts line up down the tree. Selection
    uses `list_active` and hover uses `list_hover`, per the color table,
    instead of `accent`.
  - **Right: detail panel** (`flex_1`, `p_4`). When nothing is selected,
    show only centered muted text ("Select a category to see its rules and
    settings"). When a category is selected, show sections stacked with
    `gap_3`, separated by a `border` divider, each starting with a
    `region_title`, all sharing one left edge:
    1. **Title line:** name (`text_lg`) and muted "Kind · N transactions",
       with Rename… at the right end. The rename editor replaces this line
       while it's open.
    2. **Organize:** for a parent, *New child name* plus Add child. For a
       child, *Move to parent* plus Move. Labels sit above the controls
       (3.16).
    3. **Rules** (takes `flex_1` and scrolls inside itself): the
       add-rule row (Pattern `flex_1`, regex switch, Category picker
       defaulting to the selected category, Add rule as the primary
       button), with "matches N transactions" muted under it; then column
       headings (Pattern, Type, Category) over the virtualized rule list,
       using the same widths as the rows (Pattern `flex_1` + ellipsis +
       tooltip, Type ~80 px, Category ~200 px + ellipsis). List only the
       rules that target the selected category (for a parent, its own
       rules plus its children's). Empty state: "No rules for this
       category yet" in muted text.
    4. **Delete** (pinned at the bottom of the panel, hidden for system
       categories): *Move its transactions to* picker plus Delete…, with
       the two-click confirm unchanged.
  - Everything fits at 1200 × 720 (right panel ≈ 646 px wide) with no
    horizontal overflow; long names truncate instead of pushing widths.
  - Tests: the tree, detail panel and rule list all paint inside the page
    bounds at 1200 × 720 with long names and thousands of rules; selecting
    a category limits the rule list to its rules (parent includes
    children); the add-rule picker defaults to the selected category; New
    category… shows the form, and Add parent / Cancel hide it; the
    section headings paint (`label:` / title debug selectors). Update the
    *Categories* row of `docs/ui-conventions.md` to describe this layout
    in the same commit.

- [ ] **3.26 A scrollbar on everything that scrolls.** Most scrolling
  areas give no sign that there is more content or where you are in it.
  Every scrollable region shows a vertical scrollbar whenever its content
  is taller than its viewport, and none when it fits. Do this after 3.22
  and 3.23, which re-lay out two of the lists.
  - Use gpui-base's `Scrollbar` (`gpui-base-0.9.0/src/scrollbar.rs`:
    `Scrollbar::vertical(&handle)`); it takes a `ScrollHandle`, and
    `VirtualListScrollHandle` implements `ScrollbarHandle` too. Don't
    hand-roll one.
  - **Always visible while the content overflows:** set
    `ScrollbarMode::Always` once, app-wide, on the theme's
    `ScrollbarTheme` at startup. The default (`Scrolling`) only shows the
    bar while scrolling, which is the behaviour being replaced.
  - **Virtualized lists** (category tree, rule list, To categorize queue):
    reuse the handle each list already keeps on `Shell` (`tree_scroll`,
    `rule_scroll`, `queue_scroll`, from 3.21).
  - **Plain scrolling divs** (`overflow_y_scroll` in `ui.rs`: the chart
    legend `legend-list`, the monthly summary `summary-rows`, the Transfers
    list `transfer-reviews`): give each a `ScrollHandle` kept as a `Shell`
    field (one per region, never created in `render`; same trap as 3.21)
    and `track_scroll` it.
  - **Popovers and dropdowns** that can outgrow their height: the Accounts
    filter popover gets a max height, scrolls, and shows a scrollbar; check
    that `Select` menus already show one for long lists (category pickers)
    and fix them if not.
  - **`DataTable`s** (transactions, import history) draw their own
    scrollbars (`scrollbar_visible`); confirm they follow the app-wide
    mode and show both when the content overflows.
  - **Placement:** the bar overlays the region's right edge inside its
    border (wrap the region in a `relative()` container and add the
    `Scrollbar` as an absolutely positioned sibling). Content reserves the
    bar's width (16 px) on the right so the thumb never covers amounts,
    counts or buttons.
  - Add a rule to `docs/ui-conventions.md` (*Layout*): every scrollable
    region has a visible scrollbar while it overflows, and new scrolling
    regions must add one.
  - Tests: for each region above, with enough rows to overflow, the
    scrollbar paints inside the region's bounds and the right-most content
    (amount, count or button) doesn't overlap it; with content that fits,
    no scrollbar paints; dragging the thumb (or `set_offset` on the handle)
    moves the content, and the offset survives an unrelated `cx.notify()`.

- [x] **3.27 Status messages become toasts.** Today `Shell::status` renders
  as a line of text at the top of the content area ("Categorized
  MERCHANT", "Export is not available yet.", errors). It stays until
  something clears it, and while it's there it pushes every page down by
  a line plus a `gap_3`. Replace it with gpui-base toasts
  (`gpui-base-0.9.0/src/toast.rs`), which overlay the window and take no
  layout space.
  - Remove the status line from the content column in `render`; the page
    area starts at the top of the content padding on every page, always.
  - Keep one call site shape (for example `self.notify_ok(msg, cx)` /
    `self.notify_err(msg, cx)`) and route every current
    `self.status = Some(..)` through it.
  - Success and info toasts dismiss themselves after about 4 s. Errors stay
    until the user closes them. Bottom-right corner, stacked, newest on
    top; a burst (categorizing several queue groups quickly) stacks
    instead of replacing.
  - Follow `ui-conventions.md` colors (`danger` for errors) and record the
    toast rule there (*Feedback*: no inline status lines).
  - Tests: categorizing a queue group shows a toast and leaves the page's
    first row at the same y as before the action; an error toast survives
    past the auto-dismiss time and closes on click; nothing in the content
    column renders the old status text.

- [x] **3.28 A Rules page to find and organize rules.** Rules live only in
  each category's detail panel (3.23), so finding "which rule catches
  AMZN?" means guessing the category first, and with hundreds of rules
  there is no overview. Add a **Rules** page to the sidebar, after
  Categories. Editing, deleting and live match counts reuse 3.24's row
  editor; this task adds finding and organizing.
  - **Header row:** "Rules" (`text_lg`) and a muted "N rules" on the left;
    **New rule…** at the right end, opening the same add-rule row as the
    Categories page in place.
  - **Search** (`flex_1`, focused when the page opens): filters as you
    type, case-insensitive, on the pattern **and** on the target
    category's child and parent names. A second mode, **Test a
    description**, takes a pasted bank description and shows only the
    rules that match it, the winner first and marked "wins" (same order as
    `pick_category`: substring before regex, longest first, lowest id).
  - **Filters:** a Type picker (All / Substring / Regex) and an **Unused**
    checkbox (rules that currently decide no transactions).
  - **List:** grouped by parent, then child category, each group with a
    header row ("Food › Coffee · 4 rules") that collapses and expands.
    Columns: Pattern (`flex_1`, ellipsis + tooltip), Type (~80 px),
    **Uses** (right-aligned, ~80 px: transactions this rule currently
    decides), and the 3.24 Edit / Delete… actions. Virtualized, headers
    included, with its own `ScrollHandle` on `Shell` (3.21 rule). Sort
    within a group by pattern (default) or by Uses, via the column
    headings.
  - **Move rules:** each row gets a checkbox; with one or more checked, a
    bar above the list shows "N selected", a Category picker and **Move**,
    which sets every checked rule's category in one DB call and re-runs
    rules once. Single-rule moves still work through Edit.
  - **Backend** (`financi-core` + `financi-db`, with unit tests first):
    `winning_rule(rules, description) -> Option<rule_id>` alongside
    `pick_category` (make `pick_category` call it so the two can't
    drift); `rules::rule_use_counts(conn) -> HashMap<rule_id, usize>` over
    unlocked rows that aren't card payments; `rules::move_rules(conn,
    ids, category_id)` validating the target is a child, in one
    transaction, then `reapply_rules`.
  - The Categories page keeps its per-category rule list; each row there
    gets nothing new.
  - Tests: search matches by pattern and by category name; Test a
    description lists the matching rules with the right winner; Unused
    shows only zero-use rules; collapsing a group hides its rows; moving
    three checked rules recategorizes their unlocked rows and leaves
    locked rows alone; with thousands of rules only visible rows build;
    everything fits at 1200 × 720. Add a *Rules* row to
    `ui-conventions.md`, and drop "there's no Rules page" from its
    *Categories* row.

- [x] **3.29 "Which rule did this?" from a transaction.** On the dashboard
  table's row context menu, add **Show rule**: it opens the Rules page
  with that rule found, scrolled into view and highlighted
  (`list_active`). The item is disabled with a reason when no rule
  decided the row: "Set by hand" (locked), "Card payment (built in)", or
  "No rule matches". Uses 3.28's `winning_rule`; no stored rule id.
  - Tests: Show rule on a rule-categorized row lands on that rule and it
    is visible in the list; locked and card-payment rows show the disabled
    item with the right reason.

- [x] **3.30 Monthly summary scrolls sideways.** The summary's month
  columns are fixed at 88 px each plus a 96 px Total, so a range longer
  than a few months is wider than the panel (≈ 470 px at 1200 × 720): the
  category names shrink to nothing and the right-hand months and Total
  are cut off. Keep the columns at their widths and let the grid scroll
  horizontally instead.
  - **Pinned columns:** the category name column (fixed ~160 px, swatch +
    name with ellipsis and a hover tooltip, child indent unchanged) stays
    on the left, and Total stays on the right; only the month columns
    between them scroll.
  - The header row and the body rows share one horizontal offset (one
    `ScrollHandle` on `Shell`, never created in `render`), so month
    headings stay over their cells. Vertical scrolling of the body
    (`summary-rows`) keeps working and moves the name column with it.
  - Shift + wheel and trackpad horizontal swipes scroll it; a horizontal
    scrollbar shows while the months overflow (3.26 adds the vertical
    one; use the same gpui-base `Scrollbar` and app-wide mode).
  - When the range changes, scroll to the most recent month (right end),
    since that is the month usually wanted. When everything fits, no
    scrolling and no scrollbar, and the months spread as today.
  - Click-to-filter on a cell, the selected-cell highlight and
    expand/collapse keep working while scrolled.
  - Tests: with a 24-month range at 1200 × 720, the name and Total columns
    paint inside the panel and every visible month cell lies between
    them; scrolling moves header and body months by the same amount and
    leaves names and Total in place; a new range lands scrolled to the
    last month; a 3-month range shows no horizontal scrollbar. Update the
    dashboard row of `ui-conventions.md`.

- [x] **3.31 A slimmer chart legend.** The legend beside *Spending by
  category* is 220 px (`LEGEND_W`) because every row repeats the amount
  (76 px) and share (52 px) that the chart already shows, which leaves
  the plot narrow and the band labels short. Make the legend just the key.
  - **Legend rows:** the 10×10 swatch and the category name (ellipsis),
    nothing else. Shrink `LEGEND_W` to about 140 px; `chart_label_plan`
    reads the constant, so the plot and its band labels widen with it.
    Clicking a row still selects or drills into the category, the
    selected row keeps `list_active`, and "← All categories" stays.
  - **Hover tooltip on each legend row** (gpui-base `Tooltip`, the row
    already has an `id`): full name, amount (`flow::money`) and share,
    the same text as the bar's own tooltip ("$1,234.56 · 18.2% of Food").
    Pull that string into one helper used by both.
  - **Share over each bar:** use `BarChart::label` to draw the share
    ("18%", whole percent; one decimal only below 1%, "0.4%") above each
    bar, `label_color` muted. Leave the bar tooltip as it is. Check the
    label stays inside the panel for the tallest bar (add headroom on
    the value axis if gpui-component doesn't) and doesn't collide with
    its neighbours at 1200 × 720 with many bars; if bars are too narrow
    for their text, thin the labels the way band labels are thinned
    (3.15) rather than overlapping them.
  - Tests: legend rows contain no amount or percentage text; the legend
    is ≤ 140 px wide and the plot is wider than before at 1200 × 720;
    hovering a legend row shows the tooltip with amount and share;
    each bar's label is its share, and all labels paint inside the chart
    panel without overlapping at 1200 × 720 with 15 categories. Update
    the dashboard row of `ui-conventions.md`.

- [x] **3.32 Imports page buttons: row-sized Undo, icons instead of "…".** The Undo… button in
  the newest row of the Imports history table (`HistoryDelegate`,
  `Button::new("undo-import")`) is a default medium `Button` (32 px tall)
  inside a 32 px table row (`Size::table_row_height`), so it fills the row
  edge to edge, touches the stripes, and makes that row look taller than
  the others.
  - Make it `.xsmall()` and `.ghost()` (the in-row style that 3.24 uses for
    rule actions), vertically centered in the cell, with at least 4 px of
    space above and below. The armed state ("Click again to undo") keeps
    `danger` text so the confirm still stands out.
  - Recompute the Undo column width (`history_undo_col_w`,
    `BUTTON_PAD_X`) for the small button's padding so both labels still
    show in full and the column isn't wider than it needs to be.
  - The header's **Undo…** next to Import CSV… stays; the two share the
    same confirm state as today.
  - **Icons instead of the trailing "…"** (user request, 2026-11-10). The
    app already registers the full Lucide catalog (`icon_assets()` →
    `AllAssets`), so use `IconName` with `Button::icon`:
    - Header: **Import CSV** (`file-up`, still primary), **Undo**
      (`undo-2`), **Export** (`download`), **Restore**
      (`archive-restore`): icon on the left, label without "…".
    - In-row Undo: icon only (`undo-2`), xsmall ghost, with a hover
      tooltip "Undo this import". When armed it shows the text "Click
      again to undo" in `danger` (with the icon), so the two-click
      confirm still says what it does.
    - Change `ui-conventions.md` to match: the *Imports* row, and the copy
      rule at *Copy* (line ~139): on the Imports page buttons carry an
      icon and no "…". Leave "…" elsewhere (Rename…, Delete…, New
      category…) unless the user asks.
  - Record the rule in `ui-conventions.md` (*Tables*): buttons inside table
    rows are `xsmall` ghost buttons, never the default size.
  - Tests: the in-row Undo button's bounds lie inside its row with ≥ 4 px
    above and below, in both the idle and "Click again to undo" states;
    the newest row is the same height as the other rows; the armed label
    fits the column without clipping; the header buttons' labels contain
    no "…" and each paints an icon; hovering the in-row Undo shows its
    tooltip.

- [x] **3.33 Say "category" and "subcategory", never "parent" and "child".**
  The UI copy uses the data model's words. Users think in categories and
  subcategories. Change every user-visible string; keep code, schema and
  identifiers (`parent_id`, `create_child`, element ids such as
  `add-child`, which tests use) as they are.
  - Categories page (`ui.rs`): "New parent name" → "New category name";
    "Add parent" → "Add category"; "N parents" → "N categories" (and 3.23's
    "N parents · M categories" → "N categories · M subcategories"); "New
    child name" → "New subcategory name"; "Add child" → "Add subcategory";
    "Move to parent" → "Move to category"; "Give the parent a name." →
    "Give the category a name."; "Give the child a name." → "Give the
    subcategory a name."; "Choose the parent to move it under." → "Choose
    the category to move it under."
  - Error messages that reach the UI from `financi_db::Error`
    (`crates/financi-db/src/lib.rs`: "only child categories can hold
    transactions and rules", "category … is a parent", "children cannot
    have children", "still has children", "under the same parent", …):
    reword to category / subcategory and use names, not ids, where the
    caller has them. Update tests that assert on the text.
  - Sweep every page, not only Categories: dashboard (chart tooltip
    "of Food" is fine), To categorize, import preview, pickers'
    placeholders and empty states. `grep -n -i -E '"[^"]*(parent|child)'
    crates/financi/src/ui.rs` must show only ids, debug selectors and
    comments afterwards.
  - Docs: `ui-conventions.md` user-facing copy (the *Categories* row, the
    `Parent › Child` table label becomes `Category › Subcategory`, the
    mockup's "parents × months"); add a *Copy* rule: say category and
    subcategory in the UI. Pending tasks written with the old words (3.23,
    3.28) follow this rule when they're implemented.
  - Tests: the Categories page renders none of the words parent, child
    or children in any label, placeholder, button or error (scan the
    rendered text with a long-named tree, a subcategory selected, and each
    validation error triggered); the renamed buttons still work.

**Bugs reported by the user on 2026-11-11** (3.34–3.52, plus a fix folded
into 3.23). Order: **3.40 → 3.23 → 3.41/3.42/3.48/3.49/3.50/3.51 → 3.43 → 3.44 → 3.47 → 3.35 → 3.36 → 3.45 → 3.46 → 3.37 → 3.38/3.39 → 3.34 → 3.52**, then the rest of
the order above (3.28 → 3.29 → 3.26).

- 3.23 also fixes: **the rule list overflows the page.** Today each rule
  row has a fixed 320 px pattern column, a `flex_1` spacer and Edit /
  Delete… after it, so at 1200 × 720 the buttons are cut off by the window
  edge and can't be clicked. In 3.23's layout, the Edit and Delete…
  buttons (and the editor row's Save / Cancel) must paint fully inside the
  detail panel at 1200 × 720 and at 1600 × 900, with long patterns and
  long category names. Test that at both sizes.

- [x] **3.40 Saving a rule is instant.** Clicking Save on a rule edit (and
  Add / Delete…) takes ~5 s to take effect, with no sign anything is
  happening, so the button looks broken (user report, 2026-11-11). Cause:
  `financi_core::rules::rule_matches` builds the regex with
  `RegexBuilder` on **every call**, and `reapply_rules` calls
  `pick_category` once per unlocked row, so each save compiles every
  regex rule once per transaction (rules × rows compiles).
  `count_rule_matches` does the same per row, so the live preview lags
  too.
  - **Core:** add a compiled matcher, e.g. `CompiledRules::new(&[Rule])`
    (each regex built once, case-insensitive, invalid ones dropped;
    substring patterns lowercased once) with `pick(&self, description)`.
    Keep `pick_category` / `pattern_matches` as thin wrappers with the
    same semantics, so the existing core tests pass unchanged. Add a
    one-pattern compiled form for `count_rule_matches` (new pattern and
    the excluded rule's, each compiled once).
  - **DB:** `reapply_rules` and `count_rule_matches` compile once per
    call. `reapply_rules` reuses one prepared `UPDATE` statement.
  - **UI:** while the save/add/delete task runs, the clicked button shows
    `.loading(true)` and is disabled (no double submit); it clears when the
    task finishes, on success or error.
  - Tests: core tests for `CompiledRules` matching the `pick_category`
    cases (tie-breaks, substring beats regex, invalid regex skipped); a DB
    test that `reapply_rules` over 5 000 unlocked rows with 50 regex rules
    finishes in well under a second in a release-agnostic way (assert
    < 2 s, so a debug build passes); a UI test that Save shows loading
    until the task ends and the editor closes after. Measure the save time
    on the dev-seed DB before and after and put both numbers in the
    commit message.

- [ ] **3.41 Rule category pickers show the full name.** On the
  Categories page the add-rule **Category** picker (`Select::new(&self.
  rule_target)`, in a `div().w(px(160.))`) is too narrow: the items are
  "Parent › Child" (`child_choices`), so both the closed trigger and the
  open menu cut the name off and the right subcategory is hard to pick
  (user report, 2026-11-11). The rule editor's picker (`rule_edit_target`,
  also 160 px) has the same problem. Do this after 3.23 (same row
  layout):
  - **Menu:** give both selects `.menu_width(px(320.))` (gpui-component
    `Select::menu_width`; today `Length::Auto` makes the menu as narrow as
    the trigger), so the open list shows full "Parent › Child" names, and
    make them `.searchable(true)` (gpui-component `SelectState::searchable`)
    so a subcategory can be typed. The menu must stay inside the window
    at 1200 × 720 (it opens from a trigger near the panel's right edge).
  - **Trigger:** widen both to `RULE_CATEGORY_W` (200 px), the width of
    the rule list's Category column, taking the room from the flex
    Pattern field (keep its `min_w_0`). The selected text ellipsizes and
    the trigger gets a hover tooltip with the full "Parent › Child".
  - Must still meet 3.23's fit rule: the Add rule / Save / Cancel buttons
    paint fully inside the detail panel at 1200 × 720 and 1600 × 900 with
    long category names.
  - Tests: with a 40-character "Parent › Child" name, the open menu item's
    text paints unclipped (its bounds fit inside the menu) for both
    pickers; typing in the search narrows the list; the trigger is 200 px
    and shows the tooltip; 3.23's fit tests pass. Update the *Categories*
    row of `ui-conventions.md`.

- [ ] **3.42 The regex switch lines up with the rule fields.** In the
  add-rule row (`cat:add-rule-row`) and the rule editor row, the
  **regex** `Switch` sits lower than the Pattern input, the Category
  picker and the buttons (user report, 2026-11-11). Cause: both rows are
  `h_flex().items_end()`, the labelled fields are a label line over a
  32 px control, and the Switch (no label line, shorter than 32 px) is a
  bare child, so its bottom edge, not its centre, lines up with the
  controls.
  - Wrap the Switch in a box the height of a control: `h_flex().h(px(32.))
    .items_center().flex_shrink_0()`, in both rows, so its centre matches
    the controls' centre while the row stays `items_end`. Better: factor
    a small `control_slot(el)` helper used by both rows, so they can't
    drift apart.
  - Do it in the same branch as 3.41 (same two rows).
  - Tests: in both rows the Switch's vertical centre equals the Pattern
    input's centre within 1 px, and the Save / Add rule button's centre
    too. Note the rule in `ui-conventions.md` (*Forms*: an unlabelled
    control in a labelled row sits in a 32 px slot, centred).

- [ ] **3.48 The rule editor gives the pattern room.** Clicking Edit on
  a rule swaps the row for an editor whose Pattern input shrinks so far
  that the pattern can't really be edited (user report, 2026-11-11).
  Cause: `rule_editor_row` is one `h_flex` with Pattern `flex_1` +
  `min_w_0` and, after it, the regex switch, a 160 px Category picker,
  Save and Cancel. In the detail panel (page minus the 210 px sidebar
  and the 300 px tree panel) the fixed controls take ~400 px of ~600, and
  3.41 widens the picker to 200 px, which makes it worse. The user also
  saw the editor's regex switch sit off the row's line (3.42's cause).
  - **Two lines:** line 1 is the "Pattern" label over the input at the
    editor's **full width**. Line 2 is one `h_flex().items_center()` of
    32 px controls, with no label line: the regex switch, the Category
    picker (3.41's 200 px trigger / 320 px menu, placeholder
    "Category…"), then `ml_auto` Save and Cancel. Line 2 has no labels,
    so the switch lines up without 3.42's slot (3.42 still applies to
    the add-rule row). The preview count and error line stay under both
    lines.
  - Bump the editor's entry in `rule_sizes` (today 80 / 108 px) to the
    new height (~two control lines + label + preview, ~120 / ~148 px with
    an error) so the virtual list doesn't overlap the next row.
  - Enter saves, Escape cancels, focus lands in the pattern input with
    the caret at the end (check it does today; if not, add it).
  - The add-rule row: if its Pattern input is < 240 px wide at
    1200 × 720 after 3.41, give it the same two-line shape; otherwise
    leave it on one line.
  - Tests: at 1200 × 720 the editor's pattern input is at least 90 % of
    the detail panel's inner width; the switch, picker, Save and Cancel
    share a vertical centre within 1 px; the row below the editor doesn't
    overlap it (bounds); Save/Cancel/Enter/Escape tests from 3.24 pass.
    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.49 A Uses column instead of Category.** Since 3.23 the
  Categories page's rule list only shows the selected category's rules, so
  on a subcategory every Category cell says the same thing. The user
  wants the count of transactions each rule matches there instead (user
  requests, 2026-11-11).
  - **Replace the Category column with Uses** (`RULE_USES_W`, 80 px,
    right-aligned, `flow::count`), the same number and wording as the
    Rules page's Uses column (3.28, `rule_use_counts`): the transactions
    the rule *decides*, so it agrees with stored categories and with
    3.52's "decided by this rule". A heading tooltip says "Transactions
    this rule categorizes". 0 shows in `muted_foreground`. The counts are
    loaded in the same refresh as the rules (one `rule_use_counts` call;
    no per-row queries) and update after every rule change.
  - **Parent selected:** its list mixes its subcategories' rules, so the
    subcategory also shows: a muted `text_xs` tag with the child's name
    (not "Parent › Child") after the pattern, inside the Pattern column,
    ellipsized first. A subcategory selection shows no tag.
  - The editor keeps its Category picker (it's how a rule moves to
    another category); a rule saved to a category outside the selection
    leaves the list.
  - Clicking the Uses heading sorts by it (descending), clicking Pattern
    goes back, as on the Rules page. Reuse its sort state if it fits;
    otherwise a separate `cat_rules_sort_by_uses`.
  - Tests:
    - no "Category" heading or cell paints;
    - "Uses" paints with each rule's count, and matches the Rules page
      for the same rule;
    - a rule overlapped by a longer one shows only the rows it wins;
    - the count updates after a rule edit or delete;
    - on a parent, the subcategory tags paint, and on a subcategory they
      don't;
    - Uses sort works;
    - 3.23's heading alignment and fit tests pass.

    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.50 No Organize heading or repeated labels.** In the category
  detail panel, the **Organize** section title and the "Move to
  category" field label just repeat what the picker's placeholder
  ("Move to category…") already says (user request, 2026-11-11). The
  parent side has the same problem: the "New subcategory name" label sits
  over an input whose placeholder is "New subcategory name".
  - Remove `region_title("Organize")` and both `field_label`s in that
    section. Each side becomes one `h_flex().items_center().gap_2()` row:
    - subcategory: `[Move to category… ▾] [Move]`;
    - parent: `[New subcategory name] [Add subcategory]`.
    Keep the `border_t_1` dividers above and below, so it still reads as
    its own section.
  - This overrides 3.16's visible-label rule for this section only; the
    placeholder is the label. Give the input and the select an
    `accessibility_label` with the same text.
  - Tests: no "Organize" or "Move to category" / "New subcategory name"
    label paints in the detail panel (the placeholders still do);
    the control and its button share a vertical centre; move and add-
    subcategory still work (existing tests). Update the *Categories* row
    of `ui-conventions.md` (drop **Organize** from the section list) and
    note the exception in the *Forms* labels rule.

- [ ] **3.51 Organize and Delete hidden until asked for.** The category
  detail panel spends vertical space on two sections that are rarely
  used, **Organize** (add a subcategory / move a subcategory) and
  **Delete** (move its transactions, then delete), so the rules list gets
  less of the panel (user request, 2026-11-11). Hide both by default:
  - **Title row actions:** next to Rename, at the right of the category
    name, add small buttons, each with an icon and no "…" (3.43):
    - **Add subcategory** (`folder-plus`), on a parent;
    - **Move** (`folder-input`), on a subcategory;
    - **Delete** (`trash-2`), not on system categories.
    Rename stays where it is.
  - Each opens an **inline bordered form** right under the title row,
    the same pattern as the header's New category form (3.23):
    - Add subcategory: `[New subcategory name] [Add subcategory]
      [Cancel]`;
    - Move: `[Move to category… ▾] [Move] [Cancel]`;
    - Delete: `[Move its transactions to… ▾] [Delete] [Cancel]`. Its
      Delete is still the two-click "Click again to delete" confirm in
      `danger`.
    The forms have no section titles or field labels (3.50: the
    placeholder is the label; `accessibility_label`s set). One form at a
    time: opening one closes another; Cancel, Escape, a successful
    action, selecting another category or leaving the page closes it.
    The title-row button of the open form shows `.selected(true)`.
  - Remove the Organize and Delete sections and their `border_t_1`
    dividers. With no form open the panel is the title row, a divider,
    then **Rules**, which gets all the remaining height.
  - Must fit the detail panel at 1200 × 720 with a long category name:
    the name ellipsizes before the buttons do. If the buttons can't all
    fit, make Add subcategory / Move / Delete icon-only with tooltips;
    don't let them wrap or clip.
  - 3.50's label removal applies inside the forms; do 3.50 and 3.51 in
    one branch.
  - Tests:
    - with no form open, the Rules section's top sits right under the
      title row's divider and the rule list is taller than before (bounds);
    - each button opens its form and the others stay closed;
    - opening a second form closes the first;
    - Cancel, Escape and selecting another category each close it;
    - add / move / delete still work through the forms (existing tests,
      updated to open the form first);
    - system categories have no Delete button;
    - at 1200 × 720 the buttons paint inside the panel with a 60-character
      name.

    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.43 No buttons end in "…".** The user doesn't want "…" on
  buttons; use an icon instead, or just drop it (user request,
  2026-11-11). This applies 3.32's Imports-header rule to every button.
  All of them are on the Categories page today:
  - **New category…** (`NEW_CATEGORY`, header, primary): `plus` icon +
    "New category".
  - **Rename…** (detail panel title row): `pencil` icon + "Rename".
  - **Delete…** in the detail panel's Delete section (`confirm_label`):
    `trash-2` icon + "Delete"; armed, it still reads "Click again to
    delete" in `danger`.
  - Rule rows: **Edit** gets `pencil`, **Delete…** becomes `trash-2` +
    "Delete" (armed text unchanged); both stay `.ghost()` `text_xs`. The
    invisible heading spacers (`rule-heading-edit-spacer`,
    `rule-heading-delete-spacer`) change the same way so the headings stay
    aligned with the columns.
  - **Not buttons, unchanged:** app-menu items ("Import CSV…", "Export…",
    "Restore…"), the row context menu's "Change category…" submenu,
    select placeholders ("Move to category…", "Category…") and the
    "+ New account…" picker item. Menus keep the platform's "…" for items
    that open a dialog.
  - Check 3.23's fit rule still holds (Edit / Delete paint inside the
    detail panel at 1200 × 720 and 1600 × 900): an icon adds ~20 px per
    button.
  - Tests: each of the buttons above paints an icon and its text has no
    "…"; the armed labels are unchanged (existing two-click tests); a
    guard test that scans `ui.rs` for `.label("…")` / `const … = "……"`
    used as a button label and fails, so no new one creeps in; 3.23's
    heading-alignment and fit tests pass. In `ui-conventions.md`, replace
    the *Copy* bullet's "Use an ellipsis when the button opens a dialog or
    picker" with "Buttons never end in "…"; actions carry a Lucide icon
    left of the label (menus keep "…")", and update the *Categories* row.

- [x] **3.44 Every transactions-table column sorts.** On the dashboard
  only Date and Amount sort (3.4); Account, Description and Category
  don't (user request, 2026-11-11).
  - **DB** (`financi-db/src/views.rs`): add `Account`, `Description` and
    `Category` to `TxSortColumn`. Order by what the cell shows,
    case-insensitively (`COLLATE NOCASE`):
    - Account: `i.name || ' · ' || a.name`.
    - Description: `trim(t.description_raw)`.
    - Category: `COALESCE(c.name, 'Uncategorized')`, so Uncategorized
      sorts by its label rather than first or last.
    - Ties: `t.posted_date DESC, t.import_id DESC, t.source_line ASC`
      (as Amount does), so the order is stable.
    Turn the `match` into a per-column key plus direction instead of one
    arm per pair.
  - **UI** (`TxTable::column` / `perform_sort`): wrap columns 1–3 in the
    same `sort(...)` helper so all five show the sort icon, and map
    `col_ix` 1–3 in `perform_sort`. First click on a text column sorts
    A→Z; Date and Amount keep their current first direction (descending).
  - The sort survives refreshes and filter changes as it does now; the
    column widths don't change (the sort icon fits in the existing
    headers; check at 1200 × 720 that the Account header isn't clipped).
  - Tests: a DB test per new column in both directions (including an
    Uncategorized row and mixed-case names, and a tie falling back to
    date); a UI test that clicking each header re-sorts the rows and the
    header shows the direction; the 3.4 sort tests still pass. Update
    the *Dashboard* row of `ui-conventions.md` ("every column sorts").

- [ ] **3.47 A deleted rule leaves the list at once.** After the second
  click on a rule's Delete, the row stays in the list for a noticeable
  moment before it disappears (user report, 2026-11-11, after 3.40
  landed). 3.40 made `reapply_rules` fast (34 ms for 5 000 rows × 50
  regex rules), so the wait is elsewhere. Today `arm_delete_rule` only
  changes the list after **both** the `delete_rule` task **and** the
  full `refresh` finish: `refresh` re-runs every page's queries
  (transactions, chart, summary, imports, category tree and counts,
  rules, queue, transfer reviews) in one `on_db` call before
  `self.rules` is replaced. On top of that, every `DbWorker` job queues
  behind any in-flight one (the live preview count, a previous refresh).
  - **Measure first.** In a debug build, log (`log::debug!`, behind
    `FINANCI_TIMINGS=1`) the time from the confirming click to (a)
    `delete_rule` returning, (b) the refresh result arriving, and each
    query inside `refresh`. Run it on the dev-seed DB and on a copy of the
    real DB (`FINANCI_DB=.cache/real-copy.sqlite`, timings only; never
    commit it). Put the numbers in the commit message, and fix whatever
    dominates if it isn't covered below (e.g. a missing index for
    `transaction_counts` or `uncategorized_by_merchant`).
  - **Optimistic removal:** on the confirming click, drop the rule from
    `self.rules` (and the visible list) right away and `cx.notify()`,
    keeping a copy. On error, put it back at its position and show the
    error in `rule_error`. The refresh still runs after and brings
    counts and categories up to date.
  - **Do the same for Save and Add**, so a rule edit shows its new
    pattern/category in the row at once, and a new rule appears at once,
    with the same rollback on error. Rollback on a failed Add means
    removing the provisional row; it has no id until the DB returns one,
    so give it a temporary negative id and swap it for the real one.
  - **Narrower refresh after rule changes.** Add a `refresh_after_rules`
    that runs only what a rule change can affect (rules, transactions,
    chart, summary, category counts, queue, sidebar counts), not imports
    history or transfer reviews. Only do this if the measurements show
    the full refresh is a noticeable part of the wait.
  - Tests: after the confirming click, the rule is gone from the list in
    the same frame, before the DB task completes (run the UI without
    advancing the executor); a failing delete (rule removed underneath)
    restores the row and shows the error; an edit's new pattern shows
    before the save completes; a new rule appears before its add
    completes and then carries its real id. The 3.24 delete and edit
    tests still pass.

- [ ] **3.34 To categorize looks cleaner.** The queue is loose rows on the
  page background: no panel, no row separators, a semibold merchant, and a
  bright primary Categorize button on every row, so the page is a column
  of blue buttons. Keep the columns, widths and behavior of 3.22; change
  the look:
  - **Header:** "To categorize" and a muted summary "N transactions · M
    merchants" (`flow::count`).
  - **One bordered panel** (D7, `radius`, no fill) that fills the rest of
    the page and holds the list. Column headings are a 32 px heading row
    inside the panel (`text_sm` muted, `px_3`), with a bottom `border`.
    Rows are 40 px, `px_3`, each with a bottom `border` divider (none
    after the last), `list_hover` on hover. Headings keep sharing the row
    widths.
  - **Merchant** in normal weight `foreground`; Transactions and Total
    right-aligned; Dates `text_sm` muted (as now).
  - **Categorize button:** `.small()` and not primary (outline/ghost
    style) and **disabled until the row's picker has a category**; it
    becomes `primary` once one is chosen, so only rows ready to apply
    stand out. The heading row's invisible spacer matches the new button
    size.
  - Empty state unchanged, centered inside the panel.
  - Still fits 1200 × 720 with no horizontal overflow (3.22 tests stay).
  - Tests: the panel paints inside the page; a row's Categorize is
    disabled with no pick and enabled + primary after picking; the summary
    text shows both counts; 3.22's heading-alignment and fit tests pass.
    Update the *To categorize* row of `ui-conventions.md`.

- [x] **3.35 Dashboard filters on one line.** The filter region (a
  "Filters" title, then two rows each with a label line over a control
  line) takes about a fifth of the window height. Make it one 32 px row
  with no region title and no label lines (user request, 2026-11-11; this
  replaces 3.16's visible-label rule and 3.25's two-row layout for the
  dashboard filters only):
  - Left to right, `gap_2`, all controls the default 32 px height and
    vertically centered: **Date range** select (~170 px); the custom
    **From** / **To** pickers when the range is Custom (140 px each,
    shrinking first to a 110 px floor); **Accounts** popover trigger
    (shows "All accounts" / "N accounts"); **Search** input `flex_1` with a
    `min_w` of ~140 px, a leading search icon and the placeholder "Search
    transactions"; an **Options** button (`list-filter` icon + "Options",
    showing "Options · N" when N of its toggles are on) that opens a
    popover with the three checkboxes **Include transfers**,
    **Uncategorized only** and **Show hidden** (same state and handlers as
    today); then the muted transaction count, `ml_auto`, ending at the
    page's right edge.
  - Each control without a visible label gets a hover tooltip naming it
    (Date range, Accounts, Options).
  - Must fit 1200 × 720 with the custom range open, nothing clipped and
    no wrapping. If it can't, return BLOCKED with the measured widths
    instead of dropping a control.
  - Tests: the filter region is a single row ≤ 40 px tall; every control
    and the count paint inside the page at 1200 × 720 with the custom
    range open; the Options popover's checkboxes toggle the same filters
    as before (reuse the existing toggle tests); "Options · 2" when two
    are on; the chart panel's top is at least 80 px higher than before.
    Update the *Dashboard* row and the *Dashboard filter rows* bullet of
    `ui-conventions.md`.

- [ ] **3.52 A rule's transactions under the rule list.** Nice-to-have
  (user request, 2026-11-11): clicking a rule shows, in its own scrollable
  area, the transactions that rule matches. Do it after 3.51 (which frees
  the vertical space) and after 3.28 (whose row checkboxes are the
  multi-select; this is a separate single **focus**).
  - **Focus:** clicking a rule row (not its checkbox, Edit or Delete)
    makes it the `focused_rule` (`list_active` row background); clicking
    it again, Escape, selecting another category or leaving the page
    clears it. Deleting the focused rule clears it; editing keeps it.
  - **Layout:** with a rule focused, the Rules section splits vertically.
    The rule list stays on top (`flex_1`, `min_h(120)`), then a
    `border_t_1` divider. Below it is the **matches panel** (~45 % of the
    section, `min_h(160)`): a header "Transactions matching `PATTERN`"
    (pattern ellipsized, tooltip) with a muted "N · M decided by this
    rule" summary, then a virtual list (`v_virtual_list`, scrollbar per
    3.26) of rows. With nothing focused the list takes the whole section,
    as now.
  - **Rows:** Date (110 px), Description (`flex_1`, ellipsized with
    tooltip), Account (`text_sm` muted, ~140 px), Amount (right-aligned,
    `flow::money`). Rows the rule matches but doesn't decide carry a
    muted trailing tag and `muted_foreground` text:
    - "won by `OTHER`" when another rule wins (`winning_rule`);
    - "locked" when the row's category is locked.
    Rows the rule decides sort first, then by date descending.
  - **DB:** `rule_matches(conn, rule_id) -> Vec<RuleMatchRow>` in
    `financi-db/src/rules.rs`: every transaction (locked or not) whose
    description the rule's pattern matches (`CompiledPattern`, compiled
    once), with a `status` of `Decides`, `WonBy(rule_id)` or `Locked`.
    Use one `CompiledRules` per call, as `rule_use_counts` does, so the
    "decides" set agrees with Uses (3.28). Loaded through `DbWorker` when
    focus changes and after every refresh while focused; a stale result
    for a previous focus is dropped (keep the `Task`, drop on refocus).
  - **While editing** the focused rule, the panel keeps showing the
    saved rule's matches (the live count under the editor is the
    preview); it reloads after Save.
  - Empty state inside the panel: "This rule matches no transactions".
  - Tests:
    - DB: statuses for decides / won-by / locked rows, consistent with
      `rule_use_counts`; an invalid regex returns empty.
    - UI: clicking a row focuses it and the panel paints below the list
      inside the section; the summary counts are right; a won-by row shows
      its tag; Escape and re-click clear focus; focus changes drop a late
      result; at 1200 × 720 both the list and the panel are at least their
      minimum heights with 3.51's forms closed.

    Update the *Categories* row of `ui-conventions.md`.

- [ ] **3.36 Chart legend under the chart.** The legend column beside the
  bars (`LEGEND_W`, 140 px) still takes width the plot needs. Move it
  below the chart and condense it (user request, 2026-11-11; changes
  decision D1 from "beside" to "below"):
  - The plot takes the panel's full inner width; `chart_label_plan` /
    `chart_plot_width` no longer subtract `LEGEND_W` (remove the const).
  - **Legend:** a wrapping row of chips under the plot (`flex_wrap`,
    `gap_x_3`, `gap_y_1`): each chip is the 10×10 swatch and the name in
    `text_xs`, name ellipsized at ~120 px. Clicking, the tooltip
    (amount · share), the selected chip's `list_active` and "← All
    categories" (first chip when drilled in) all work as in 3.31.
  - At most **two lines** of chips (~40 px). If more chips than fit, the
    last visible chip is "+N more" (muted), whose tooltip lists the hidden
    names; clicking it does nothing. The chart panel keeps its fixed
    height (~340 px); the plot takes what the title and legend leave.
  - Tests: the plot is wider than before at 1200 × 720 (about +150 px);
    the legend paints below the plot's bottom edge and inside the panel;
    with 15 categories it uses ≤ 2 lines and shows "+N more" with the
    right N; clicking a chip still selects / drills; the 3.31 legend and
    bar-label tests are updated, not deleted. Update the *Dashboard* row
    of `ui-conventions.md` and D1.

- [ ] **3.45 Chart and summary on their own rows.** The middle row puts
  the chart panel (`CHART_PANEL_SHARE`, 45 %) and the monthly summary side
  by side at 340 px tall; both are cramped (user request, 2026-11-11;
  changes D1's "chart beside the summary"). The user chose **a scrolling
  dashboard** over tabs or shorter panels:
  - **Layout, top to bottom:** the filter row (3.35) stays **pinned**
    above the scroll area. Below it, one vertical scroll container
    (`id` + `overflow_y_scroll` + a `ScrollHandle` kept on the shell)
    holds the **chart panel, full width, 340 px**; then the **summary
    panel, full width, 340 px**; then the **transactions table, full
    width, fixed ~480 px**, scrolling inside itself. `gap_3` between
    them, page padding as now. Nothing in the scroll area is `flex_1`
    in height; a scroll container gives children no height to fill (see
    *GPUI pitfalls*), so every panel has an explicit height.
  - **Chart:** `chart_plot_width` uses the full page width (drop
    `CHART_PANEL_SHARE`; after 3.36 there's no legend column either).
    The band-label plan (3.15) gets the wider budget for free.
  - **Summary:** the wider panel shows more month columns before its
    horizontal scroll kicks in; `NAME_W` / `MONTH_W` / `TOTAL_W` are
    unchanged. A vertical wheel over its month viewport must scroll the
    page (today it forwards vertical wheel to the rows; check it reaches
    the page scroll when the summary has no vertical overflow).
  - **Table:** wheel over the table scrolls the table. The no-matches
    state fills the same 480 px box.
  - **Scroll position** survives refreshes and filter changes (the handle
    lives on the shell); it resets to the top only on entering the
    dashboard from another page. The empty-window state (no data) is
    unchanged and doesn't scroll.
  - Do it right after 3.36 (both change the chart's width); 3.36's test
    "plot is ~150 px wider" gets updated to the full-width numbers here.
  - Tests: at 1200 × 720 the chart and summary panels both span the page
    width and the summary's top is below the chart's bottom; the table
    is ~480 px tall; scrolling the page moves the panels while the
    filter row stays put; the scroll offset survives a filter change;
    the summary shows more month columns than before with a 12-month
    range. The 3.25 title-alignment test (chart and summary titles on
    one line) is replaced, not deleted. Update the *Dashboard* row of
    `ui-conventions.md` and D1.

- [ ] **3.46 Global dashboard filters, and Clear all.** Every dashboard
  filter should apply to all three views at once: a category picked on
  the chart should show in the summary and the table too. Add a button
  that clears every filter (user request, 2026-11-11). Today the state
  is already shared (`legend_selection` + `summary_month`, set by chart,
  legend and summary-cell clicks; `sync_chart_to_selection`), but only
  the table *filters* by it. The summary highlights just the clicked
  cell, the chart ignores the month, and search / Uncategorized only /
  Show hidden are table-only (3.4) and never reach the chart or summary.
  - **One filter model.** Fold the table-only fields into the shared
    filter: give `spend_by_category` and `monthly_summary` the same
    search, `uncategorized_only` and `show_hidden` narrowing as
    `list_transactions_filtered` (factor the `WHERE` builder in
    `views.rs` so all three queries share one function; no copies).
    Sort stays table-only.
  - **Category selection** (from a bar, a legend chip or a summary cell)
    shows in all three:
    - **Chart:** the selected bar keeps its color and the others dim to
      ~35 % opacity (bars aren't filtered away; the chart stays
      context). The drill-down (3.5) works as now.
    - **Summary:** the selected category's **whole row** is `list_active`;
      a selected subcategory auto-expands its parent. Rows aren't
      removed.
    - **Table:** filters to it (as now).
  - **Month selection** (summary cell) shows in all three:
    - **Chart:** the chart totals cover only that month, and the panel
      title reads "Spending by category · Oct 2026".
    - **Summary:** that month's column is tinted (`list_hover`), the
      selected cell `list_active`.
    - **Table:** filters to it (as now).
    - A month can be selected on its own, without a category: add a click
      on the summary's **month heading** that toggles the month alone.
      Today the month only narrows a category selection; drop that rule
      (keep Esc clearing both).
  - **Clear all:** a ghost button "Clear all" (`x` icon) at the right of
    the 3.35 filter row, just before the count, shown only when any
    filter differs from the default. It resets everything: date range
    to the default, all accounts checked, search empty, the three
    Options toggles off, and the category/month selection and drill-down
    cleared, then one refresh. The no-matches "Clear filters" link calls
    the same function (rename `clear_filters` → `clear_all_filters` and
    extend it). Esc keeps clearing just the selection.
  - The filter row must still fit 1200 × 720 with the custom range open
    and Clear all shown (3.35's fit test, extended).
  - Tests: DB tests that chart and summary totals honour search, hidden
    and uncategorized-only like the table does. UI tests:
    - clicking a bar dims the other bars, highlights the summary row and
      filters the table;
    - a summary cell's month narrows the chart totals and retitles it;
    - a month heading selects the month alone;
    - search narrows all three;
    - Clear all resets every filter (one assertion per filter) and
      hides itself;
    - the no-matches link does the same.

    Update the *Dashboard* row and *Interaction* section of
    `ui-conventions.md` (filters are global; Clear all).

- [ ] **3.37 In-row Undo says "Undo".** 3.32 made the newest history
  row's Undo an icon-only button, which the user finds unclear. Make it
  the `undo-2` icon **plus the label "Undo"** (still `.xsmall().ghost()`,
  vertically centered with ≥ 4 px above and below); armed it still reads
  "Click again to undo" in `danger`. Keep the "Undo this import" tooltip.
  Recompute `history_undo_col_w` so both labels show in full and the
  column is no wider than needed. Tests: the idle button's text is
  "Undo" and it paints an icon; 3.32's bounds and fit tests still pass
  in both states. Update the *Imports* row of `ui-conventions.md`.

- [x] **3.38 Tooltip on the sidebar's database path.** The sidebar footer
  (`Shell::sidebar`, `.footer(...)`) shows the DB path in `text_xs` muted
  with an ellipsis, so on a normal path most of it is cut off by the
  sidebar's width (user request, 2026-11-11). Give the path element an `id`
  and a hover `.tooltip(...)` (gpui-base `Tooltip`, as the queue's merchant
  cell does) showing the full path, prefixed "Database: ". Leave the
  footer's look unchanged otherwise. Tests: hovering the path shows a
  tooltip whose text is the full `db_path`, with a path longer than the
  sidebar. Note it in the *Sidebar* section of `ui-conventions.md`.

- [x] **3.39 Show the version in the sidebar.** From `plan.md` → *Show the
  version subtly in the UI* (planned with the semver tags but never made a
  task; user asked again 2026-11-11). Add a `crates/financi/build.rs` that
  sets `FINANCI_VERSION` from `git describe --tags --always --dirty`,
  falling back to `CARGO_PKG_VERSION` when git or the tags aren't there
  (and `rerun-if-changed` on `.git/HEAD` and the refs so it stays
  current). Expose it as one `pub const`/fn (M5's export manifest will
  reuse it). Show it as the last line of the sidebar footer, `text_xs`
  muted, e.g. "v0.13.7" (or "v0.13.7-3-gabc1234-dirty"), ellipsized with
  the same kind of hover tooltip as 3.38. Do 3.38 and 3.39 in one branch
  (same footer). Tests: the footer paints the version text and it equals
  the const; the fallback path is unit-tested (factor the choice into a
  function that takes the git output). Tick the plan.md box too, and note
  it in the *Sidebar* section of `ui-conventions.md`.


**Done when:** on the demo data the dashboard totals match
`expected.json`, filters compose, tests cover 3.1–3.52.

---

## M4 — Simplii Visa, transfer pairing, near-match review

- [x] **4.1 Simplii Visa (assumed format).** No real Visa sample exists yet.
  Assume it uses the **same format as Simplii chequing**. Because two
  parsers can't claim the same header, replace `simplii-chequing` with one
  parser `simplii` whose `account_kinds()` is `[Chequing, CreditCard]`; the
  chosen account decides which. Funds Out = money out, Funds In = money in
  for both. Keep accepting the old parser id `simplii-chequing` in
  `by_id()` (existing `imports.parser_id` rows). `is_card_payment`: a
  credit-card row with Funds In whose description contains `PAYMENT`.
  Add a synthetic Visa fixture. Note in `plan.md` that this needs revisiting
  when a real sample arrives.

- [x] **4.2 Transfer pairing** (runs automatically after every import).
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
  - UI: the sidebar's Transfers item shows the open-review count and opens
    the Transfers page (no banner, no sheet); for
    each, pick the matching row or "Not a transfer".
  - Paired rows show a link icon in the table; context menu *Unpair* clears
    both sides and re-applies rules.
  - Tests: single match pairs, tie goes to review, resolving a review,
    unpair, an unrelated equal amount is not paired.

- [x] **4.3 Near-match review in the import preview.**
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

**Done when:** tests cover 4.1–4.3.

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
  - UI: File → Export… (and an Export… button on the Imports page) opens a save dialog defaulting
    to `~/Desktop/financi-export-YYYY-MM-DD.zip`.

- [ ] **5.2 Restore.**
  - File → Restore…, a Restore… button on the Imports page, and a "Restore
    from export…" link in the empty-state screen.
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
