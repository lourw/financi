# M3 — UI queue: architecture and GPUI best practices

Do these in the order listed (`docs/tasks.md` → *Picking the next task*). Each task is one step of `docs/tasks/gpui.md` → *GPUI best practices*; read that file first.

## UI queue: architecture and GPUI best practices (in this order)

Steps 1–15 of `docs/tasks/gpui.md` → *GPUI best practices*, task by task.

- [ ] **3.89 Shared UI components.** (Design system, user decision
  2026-11-11.) Build the components `docs/ui-conventions.md` →
  *Components* describes, in `crates/financi/src/ui/widgets.rs`, so pages
  stop being styled by hand: `Panel`, `PageHeader`, `DataList` (heading
  row + virtual list; typed columns `Fixed(px) | Flex` with alignment
  shared by heading and rows; 40 px rows with dividers and hover; 36 px
  group headers; a hover-revealed actions column whose width is
  reserved), `FilterRow`, `InlineForm`, `Swatch`, `TypeTag` (on
  gpui-component's outline `Tag`), `control_slot`, `row_action_button`.
  - Each is a `RenderOnce` (or builder fn) with no page state, tested on
    its own in a headless window: heading and column left edges match,
    rows are the token heights, actions don't shift the row, a long label
    ellipsizes.
  - Prove them on the smallest pages: move the Transfers list and the
    Imports page header and history onto them, and set the DataTables to
    dividers instead of stripes (`stripe(false)`, row borders). The
    invisible `.opacity(0.0)` spacer buttons that align headings today go
    away where a `DataList` replaces them.
  - Later restyle tasks (3.34, 3.49, 3.51, 3.52, 3.58) build on these;
    don't restyle those pages here.
  - Remove the "(built in task 3.89; until then, match this by hand)"
    note from `docs/ui-conventions.md` → *Components*.

- [ ] **3.79 Transfers and Imports become their own views.** (Architecture,
  user decision 2026-11-11.) The first page extractions, chosen because
  they're the smallest; they set the pattern the others copy.
  - `TransfersPage` and `ImportsPage` entities in their modules, owning
    the state from 3.74's `TransfersState` / `ImportsState`, rendering
    themselves, and subscribing to `Catalog`'s `DataChanged` for the
    domains they depend on (`const DEPENDS: Domains`). On a matching
    event a visible page reloads; a hidden one marks itself dirty and
    reloads when shown. `Shell` keeps an `Entity` for each and renders it.
  - Pages talk to `Shell` (navigation, toasts) through events, not by
    holding the `Shell`.
  - **Keyboard:** the page root has a `FocusHandle`, `track_focus` and a
    `key_context("Transfers" | "Imports")`. Define one app-wide `Cancel`
    action bound to `escape` per context (as gpui-component does,
    `select.rs:35`); each page's `Cancel` handler closes its own
    transient UI (the Imports page: the armed Undo confirm).
  - Write the pattern down in `docs/tasks/gpui.md` → *Coding practices* (page
    entity, `DEPENDS`, events to `Shell`, `Cancel`) so the next
    extractions follow it.
  - No behaviour change otherwise; existing tests pass (selectors may
    move to the page).

- [ ] **3.80 The Rules page becomes its own view.** (Architecture.) Same
  pattern as 3.79, for `RulesPage`, before 3.58 restyles it.
  - Owns search, filters, collapse, sort, checked rows, the editor and
    pending actions; `DEPENDS = Rules | Categories | Transactions`.
  - Moves the "focus the search when the page opens" side effect out of
    `render` (`rules_focus_pending` + `window.defer`) into the page's
    show hook.
  - Show rule (from the dashboard's row menu) reaches it through an event
    carrying the rule id.
  - `Cancel` closes the editor, disarms a delete, clears the checked rows.

- [ ] **3.81 The Categories page becomes its own view.** (Architecture.)
  Same pattern as 3.79, for `CategoriesPage`, before 3.48. Owns the
  selection, the new-category form, rename, the rule list's editor and
  armed deletes; `DEPENDS = Categories | Rules | Transactions`. The rule
  rows it shares with the Rules page come from one row renderer used by
  both. `Cancel` closes the innermost open editor or form, then disarms.

- [ ] **3.82 The Dashboard becomes its own view.** (Architecture.) Same
  pattern as 3.79, for `DashboardPage`, after 3.56 and before 3.36. Owns
  the `DashboardFilter` (3.56), chart, summary, table and their loaders;
  `DEPENDS = Transactions | Categories | Accounts | Rules`. Its `Cancel`
  clears the innermost selection (chip, then drill-down), replacing the
  global `ClearCategorySelection` escape binding, which today also runs
  on other pages and clears hidden dashboard filters.

- [ ] **3.83 To categorize becomes its own view.** (Architecture.) Same
  pattern as 3.79, for `QueuePage`. Owns the queue and its lazily built
  pickers; `DEPENDS = Transactions | Categories | Rules`. Prune pickers
  for merchants outside the last rendered range plus a margin (today
  `queue_pickers` keeps one `SelectState` per merchant ever scrolled
  into view, each with a full copy of the category names).

- [ ] **3.91 Every existing page uses the shared components.**
  (Architecture, 2026-11-11.) After 3.89 and the page extractions
  (3.79–3.83); one commit per page, so it can be reviewed in parts. Move
  what's built by hand today onto 3.89's components, keeping each page's
  behaviour and tests (update selectors, don't delete tests):
  - **To categorize** (`QueuePage`): the list from 3.34 becomes a
    `DataList` (columns as today) inside a `Panel`, under a `PageHeader`.
  - **Categories** (`CategoriesPage`): the header becomes a `PageHeader`
    and the New category form an `InlineForm`; the tree becomes a
    `DataList` with indented subcategory rows (`Swatch`, name, count
    column), dropping its hand-rolled row layout.
  - **Dashboard** (`DashboardPage`): the filter row becomes a `FilterRow`
    (its Options popover is the shared one); the chart and summary panels
    become `Panel`s with title rows.
  - **Rules** stays for 3.58, and the Categories rule list for 3.49 (both
    already planned on the components).
  - **Sidebar:** replace the ad-hoc `sidebar_foreground.opacity(0.7)` tint
    with the `muted_foreground` token.
  - Done when no page builds its own heading row, divider rows, or
    invisible `.opacity(0.0)` alignment spacers (grep shows none outside
    `widgets.rs`), and every page fits 1200 × 720 as before.

- [ ] **3.57 Every page reloads only what it depends on.** Rewritten
  2026-11-11: the old 3.57 is split into this task, 3.84 (feedback within
  a frame) and 3.85 (render caches). Do it after the page extractions
  (3.79–3.83), as the check that the architecture holds.
  - Every page entity declares `const DEPENDS: Domains`; every mutation
    reports its `Domains` through `Catalog` (3.78). Write both lists as
    one table in the `Domain` doc comment and remove any remaining direct
    `refresh` calls; the full reload is left only behind import, undo and
    restore (all domains).
  - Hidden pages mark themselves dirty and reload when shown; a page
    switch with nothing dirty issues no query.
  - Tests: one per row of the table (the action reloads exactly the pages
    that depend on its domains, via the query-count hook); a page switch
    issues no query; a dirty hidden page reloads once when shown.

- [ ] **3.84 Every click answers within a frame.** (From the old 3.57,
  split 2026-11-11.) After 3.64 (per-action tasks), so a rollback can't
  be cancelled. Every button that starts DB work either updates the
  screen optimistically with rollback on error (row category change,
  hide / unhide, queue Categorize) or shows `.loading(true)` and is
  disabled until it finishes (rule and category edits; 3.47 / 3.54 make
  those optimistic later). Tests: change category, hide and Categorize
  update the row in the same frame before the DB job runs, and roll back
  on error; every other DB-backed button shows loading until done.

- [ ] **3.85 No per-frame work that grows with the data.** (From the old
  3.57, split 2026-11-11; code review list.) Cache derived view data on
  the page that owns it, rebuilt only when its inputs change:
  - **Compiled rules:** "Test a description" and Show rule call
    `winning_rule` / `pattern_matches` in `render` and on every menu open,
    recompiling every regex each frame. Keep one `CompiledRules` (and
    per-rule `CompiledPattern`s) in `Catalog`, rebuilt when rules change.
  - **Summary:** `summary_rows` runs every render with a linear
    `parent_of` per row and clones every row's months; the grid builds
    rows × (months + 1) cells, each with `format!` ids and a listener,
    though most rows are clipped. Build `SummaryRowView`s with
    pre-formatted `SharedString` cells when the summary loads, virtualize
    the rows, and key element ids by category id, not row index (index
    ids shift when a category expands).
  - **Colors and labels:** `category_colors` is built twice per dashboard
    frame and again on Categories; `rule_target_label` scans
    `child_choices` per row. Build both once per category change in
    `Catalog` (`HashMap<i64, SharedString>` labels).
  - **Rules page items:** build the grouped items and their sizes when
    rules, stats, filters, collapse or sort change; group with a map, not
    a linear `find`.
  - **Transactions table:** precompute the merchant and formatted cells
    per row when rows load, instead of `parse::merchant` and `format!` per
    visible cell per frame.
  - **Toasts:** move them into their own child entity so a toast tick
    re-renders only the toast layer, and stop the 100 ms ticker when no
    toast is animating (a sticky error toast ticks forever today).
  - Tests: each cache rebuilds on its input change and not on an
    unrelated notify (counter hooks); a toast tick doesn't re-render the
    page.

- [ ] **3.86 Layout from measured sizes.** (Code review, 2026-11-11.)
  Before 3.36 and 3.26. The chart's label budget and the table's flexible
  column widths are computed from the window width minus constants
  (`chart_plot_width = window_width − SIDEBAR_W − 2·PAGE_PAD …`,
  `tx_flex_widths(page_w)`, `history_flex_widths`) and from guessed
  character widths (`CHART_CHAR_PX`, `TEXT_SM_CHAR_PX`). Every layout
  change (3.36's legend, 3.45's full-width chart, 3.26's scrollbar
  gutter, a different font on Linux) silently breaks them.
  - Measure the plot area and the tables' containers: read their bounds
    in a `canvas` / `on_prepaint`, store them on the page, and
    `cx.notify()` when they change. Derive the chart's label plan and the
    tables' column widths from the measured bounds. DataTable has no flex
    columns, so widths are still computed, but from measured bounds.
  - Measure text with `window.text_system()` instead of per-character
    estimates where labels are truncated or thinned.
  - Delete `SIDEBAR_W`-based arithmetic outside the sidebar itself.
  - Tests: the label plan and column widths follow the measured width at
    1200 and 1600 px; changing the sidebar width constant changes no
    chart or table test result.
