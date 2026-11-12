# M3 — UI queue: architecture and GPUI best practices

Do these in the order listed (`docs/tasks.md` → *Picking the next task*). Each task is one step of `docs/tasks/gpui.md` → *GPUI best practices*; read that file first.

## UI queue: architecture and GPUI best practices (in this order)

Steps 1–15 of `docs/tasks/gpui.md` → *GPUI best practices*, task by task.

- [ ] **3.74 Group `Shell`'s state by page.** (Architecture, user decision
  2026-11-11: move to page entities.) Pure refactor, no behaviour change,
  right after 3.62. Today every page module reaches into ~200 shared
  `Shell` fields and a 450-line `Shell::new`, so two lanes still collide
  on the struct, the constructor and `refresh`.
  - Move each page's fields into a struct in its module: `DashboardState`,
    `QueueState`, `CategoriesState`, `RulesPageState`, `TransfersState`,
    `ImportsState`, each with its own `new(window, cx)`; `Shell` holds
    them as `self.dashboard`, `self.rules_page`, …. Shared data (accounts,
    categories, rules, `child_choices`, counts) stays on `Shell` until
    3.78 moves it to `Catalog`.
  - Split `Shell::new` the same way, and the refresh continuation into a
    per-page `apply_*` method.
  - Done when: every page field lives in its page struct, `Shell::new`
    is under ~100 lines, no file over ~1 500 lines, no new `pub`, and
    the test count is unchanged.

- [ ] **3.75 A reliable UI test harness.** (Code review, 2026-11-11.)
  Tests only; rename selectors in the UI code where needed.
  - **Wait for real idleness.** 11 tests wait for "initial load" with
    `refresh_task.is_some()`, which is already true when `Shell::new`
    returns, and `run_until_parked` doesn't wait for the `DbWorker`
    thread, so "nothing happened" assertions can pass vacuously. Add a
    loaded / applied-generation marker set in the refresh continuation
    and a `wait_idle(shell)` helper (no DB job in flight, all results
    applied). Use it before every negative assertion; delete the
    `refresh_task.is_some()` waits.
  - **Click through controls.** ~160 places call `Shell` methods or set
    fields directly, so wiring bugs (like 3.65's missing notify) can't
    show. Add `click(cx, window, selector)` (bounds + `simulate_click`)
    and convert at least one test per control kind (checkbox, button,
    select, legend row, summary cell) to use it.
  - **One bounds helper.** `bounds_of`, `centre_y`, `rules_sel_y` and
    `paints` are the same helper; `wheel_over` / `wheel_over_x` are near
    copies; `leaked()` works around `&'static str` selectors. Make one
    `bounds(window, impl Into<String>)` and one wheel helper in
    `ui/tests/mod.rs`.
  - **Realistic defaults.** `open_shell_at` defaults to 1100 × 720, below
    the app's 1200 × 720 minimum; use 1200 × 720. Give each test its own
    temp DB dir (today one per process is shared by parallel tests that
    write master CSVs).
  - **Selectors follow `page:thing[:id]`.** Rename the stragglers
    (`cat-tree-row-{id}`, `rule-row-{id}`, `add-rule-preview`,
    `rule-edit-error`, `transfer-review:{txn}`, `rules:group-{id}`,
    `summary:cell-{ix}-{col}`, and `rule:` vs `rules:`), and add a test
    that walks a rendered frame's selectors against the pattern.
  - Done when: no `refresh_task.is_some()` waits remain, every negative
    assertion follows `wait_idle`, and the test count is unchanged or
    higher.

- [ ] **3.64 Actions never cancel each other.** (Code review, 2026-11-11.)
  About 20 unrelated actions store their async work in one
  `Shell::task: Option<Task<()>>` (assigned in ~21 places: pick file,
  undo import, category edits, rule add/save/delete/move, queue
  Categorize, change category, hide, pair, unpair, apply rule offer).
  `ImportPanel` does the same with its own `task`. Replacing the slot drops
  the earlier `Task`, which cancels its continuation, while the queued
  `DbWorker` job still commits. So a second click loses the first action's
  follow-up: e.g. Save a rule, then hide a row before it returns → the
  rule is saved but `pending_rule_action` never clears (Save stays
  loading and disabled), the editor never closes, errors and toasts are
  lost, and nothing refreshes.
  - Mutations whose result must always be applied run as
    `cx.spawn_in(..).detach()` and update through the weak entity
    (`this.update_in(..)` already returns `Err` once the view is gone);
    their result handler checks the state it touches is still current
    (e.g. the editor is still on that rule) before changing it.
  - Only work a newer request should replace keeps an `Option<Task>`
    slot, one per purpose: the refresh, the live preview count, settings
    writes. Delete the shared `task` field.
  - **Import while busy:** disable the preview's Cancel and show
    `.loading(true)` on Import while `busy`, and ignore `open_file` (drop,
    ⌘O) while an import is committing; today Cancel drops the panel and
    its task while the commit still runs, so the rows are imported with no
    toast and no master-CSV error.
  - Store the import panel's `Subscription` with the panel
    (`import: Option<(Entity<ImportPanel>, Subscription)>`); today every
    open pushes a new one into `_subs` and never removes it.
  - Tests: start a rule Save and a row Hide in the same frame, then
    both complete (pending cleared, editor closed, row hidden, two
    refreshes); Cancel is disabled while an import commits and a second
    drop is ignored; opening ten files leaves one import subscription.

- [ ] **3.65 Filter clicks redraw at once.** (Code review, 2026-11-11.)
  The dashboard setters (`set_account_checked`, `set_include_transfers`,
  `apply_search`, `set_uncategorized_only`, `set_show_hidden`,
  `set_tx_sort`, `click_legend_row`, `show_all_categories`,
  `clear_category_selection`, `click_summary_cell`) change state and call
  `refresh`, which only spawns a task; the only `cx.notify()` is in the
  task's continuation. gpui-component's `Checkbox` is controlled ("the
  owner must write the requested value and call `cx.notify()`",
  `gpui-component-0.9.0/src/checkbox.rs:105`), so a ticked box, the
  "Options · N" label, the legend highlight and the summary cell
  highlight stay unchanged until every page's queries return. This is a
  large part of the "really slow filters" report.
  - Every setter calls `cx.notify()` right after changing state; only
    the data waits for the DB.
  - **One refresh at startup:** `Shell::new` starts `load_date_range`,
    `load_include_transfers` and `refresh` separately, so launch runs up
    to three full refreshes and the first frame shows the default range.
    Read both settings in the first refresh's DB job (or in `main.rs`
    before the window opens, like `open_db`), apply them, and run exactly
    one refresh.
  - Tests: click the real Options checkbox (`simulate_click` on its
    bounds), draw once without letting the DB job finish, and the box
    paints checked and the label reads "Options · 1"; the same for an
    account checkbox and a legend row; with a saved Custom range, launch
    issues one refresh job (count jobs with a test hook) and the first
    data frame uses the saved range.

- [ ] **3.76 App wiring done once, without reference cycles.** (Code
  review, 2026-11-11.)
  - **Global setup out of the view.** `Shell::new` calls `cx.bind_keys`,
    `cx.set_menus` and registers global `cx.on_action` listeners holding
    a strong `Entity<Shell>`; GPUI has no way to remove those listeners
    (`gpui-pre-0.3.11/src/app.rs:2758`), so a second Shell (a second
    window, or a test opening two) duplicates every handler (⌘O opens two
    pickers). Move them into `ui::init(cx)`, called once from `main.rs`;
    the handlers find the window's Shell through a `Global` holding a
    `WeakEntity<Shell>` (or the active window's root).
  - **Weak captures.** The six `TxTable` callbacks capture a strong
    `cx.entity()`, a cycle that keeps `Shell` alive forever. Capture
    `cx.weak_entity()` and `update(..).ok()`. Better: have `TxTable` emit
    events the Shell subscribes to, which also removes the re-entrancy
    risk of updating the Shell from inside the table's own update.
  - **Stale row indexes.** `TxTable::context_menu` and `render_td` index
    `self.rows[row_ix]`; use `.get()` and return the menu unchanged when
    the row is gone (the index can be stale across a refresh).
  - **macOS reopen.** The default quit mode leaves the app running with
    no window after the window closes, and there's no `on_reopen`. Add
    `on_reopen` to open the window again (or quit on last window closed;
    pick reopen, the macOS norm).
  - **Startup.** `main.rs` runs `master_csv::self_check` and the dev seed
    before the window opens; move `self_check` into the first `DbWorker`
    job so the window shows at once. A startup error (`open_db()?`) only
    prints to stderr, so on macOS the app just vanishes: also write it to
    `<data dir>/financi.log` and show it in a minimal error window.
  - Tests: two Shells register one set of handlers (⌘O fires once);
    dropping the window drops the Shell (a `WeakEntity` upgrade fails); a
    context menu on a row index past the end doesn't panic.

- [ ] **3.78 A shared data model that announces changes.** (Architecture,
  user decision 2026-11-11.) Today one `refresh` re-queries every page
  (a 13-part tuple) and ~30 call sites decide when to call it; each new
  view (v1.1–v1.3) would have to be added to every path. Invert it:
  writers say what they changed, readers say what they depend on.
  - `enum Domain { Accounts, Categories, Rules, Transactions, Imports,
    Transfers, Settings }` as a bitset type `Domains`.
  - Each mutation's UI handler reports the domains it changed, from one
    table in code (doc comment on `Domain`): rule add/edit/delete/move →
    `Rules | Transactions`; set category, hide, unhide →
    `Transactions`; category add/rename/move/delete → `Categories |
    Rules | Transactions`; pair/unpair/review → `Transfers |
    Transactions`; import/undo/restore → all.
  - A `Catalog` entity (`impl EventEmitter<DataChanged>`) owns the shared
    data: accounts, the category tree, `child_choices` and their `Names`
    (built once, shared via `Rc`/`SharedString`), rules, rule-use stats,
    sidebar counts. `Catalog::changed(domains, cx)` reloads only the
    parts tied to those domains (keyed jobs, 3.77), then emits
    `DataChanged(domains)`.
  - Pages still on `Shell` react through one mapping `Shell` keeps until
    they're extracted (3.79–3.83): each page section declares the
    domains it depends on and reloads only on a matching event. Delete
    the full `refresh` except for import/undo/restore.
  - Tests: a rule edit reloads rules, rule stats and the dashboard but
    not imports history or transfer reviews (query-count hook); an import
    reloads everything; a page switch issues no query.

- [ ] **3.56 Dashboard filters apply at once.** Changing a dashboard
  filter (date range, accounts, search, the Options toggles, a legend /
  summary click) takes effect "really, really slowly" (user report,
  2026-11-11). Rewritten 2026-11-11 for the page-entity architecture:
  3.65 already redraws the control at once, 3.77 adds keyed latest-wins
  DB jobs and timing logs, and 3.78 adds `Catalog` and `Domain` events.
  This task gives the dashboard its own loader.
  - **One filter value.** Today the filter is spread over separate
    fields, partly duplicated (`include_transfers` on `Shell` and in
    `view_filter`; accounts in `excluded_accounts` and
    `view_filter.account_ids`), and `refresh` assembles a `TxTableFilter`
    each time. Put one `DashboardFilter` struct (`Clone + PartialEq +
    Default`-ish: date range, accounts, search, the three toggles, category
    selection, month, drill-down, sort) on the dashboard state.
  - **A dashboard-only loader.** Every filter, selection and sort change
    runs one keyed read job (`call_keyed("dashboard", …)` on the reader,
    3.77) for the filtered transactions, chart rows, monthly summary and
    count only, using 3.88's shared `dashboard()` query. A burst of
    changes runs at most the in-flight job plus the newest; a superseded
    result is dropped.
  - **Search waits for a pause:** debounce 200 ms (a `Timer` task
    replaced on each keystroke); Enter applies at once. Other controls
    apply at once.
  - **Stale selection:** when a result shows the selected category is
    gone, correct the selection and start a new load, and return without
    applying the stale rows (today `refresh` calls itself from its own
    continuation and then applies the stale rows anyway).
  - A dashboard-only result updates only the table, chart and summary,
    never the history table, queue pickers, rule lists or category
    pickers.
  - Measure with 3.77's `FINANCI_TIMINGS=1` on the dev seed and a
    throwaway local 10 000-row / 300-rule seed (don't commit it); numbers
    in the commit message. Target: a filter change shows its result
    within 100 ms on a year of data (the timed test is written in 3.55).
  - Tests:
    - a filter change runs only the dashboard job (query-count hook);
    - five rapid search keystrokes run one job, after the debounce;
    - a burst of three filter changes ends with the last filter's rows,
      and a late superseded result doesn't overwrite them;
    - an import still reloads every page;
    - the existing filter, sort and selection tests pass.

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
