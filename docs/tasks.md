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
a task was written, not when to do it): **3.24 → 3.25 → 3.22 → 3.23 →
3.26**. They are listed below in that order. 3.24 comes first because it is
the only one that blocks the user: a wrong rule can't be corrected today.
3.25 is a small fix to a regression on the main page. 3.26 goes last because
it attaches scrollbars to lists that 3.22 and 3.23 re-lay out.

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

- [ ] **3.25 Dashboard alignment.** Since the labelling pass (3.16) the
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
    the left edges of the filter region, chart panel and table are equal,
    and so are the right edges of the filter region, summary panel and
    table; the *Spending by category* and *Monthly summary* titles have
    the same top; with the Custom date range open (From / To visible) the
    row still fits and stays aligned. Attach before/after screenshots to
    the handoff.

- [ ] **3.22 To categorize queue fits the window.** At the minimum window
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

- [ ] **3.23 Categories page layout.** The page reads as scattered
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

**Done when:** on the demo data the dashboard totals match
`expected.json`, filters compose, tests cover 3.1–3.26.

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
