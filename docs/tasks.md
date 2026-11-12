# financi — implementation tasks

This is the working checklist for the rest of v1. It turns `plan.md` (the
requirements) into concrete tasks. Where the two disagree, **this file
wins** for M2–M5 detail.

Finished tasks are archived in `docs/tasks-done.md`. Don't read it at the
start of a session; grep it for a task number only when an open task
refers to one (e.g. "3.24's row editor").

## How to work

1. Do **one milestone at a time**, picking tasks as *Picking the next task*
   says. When every box in it is ticked, continue to the next milestone
   without waiting for a go-ahead. Stop and report to the user when v1 is
   complete (the *Final* task done).
2. Keep **two sub-agents in flight** whenever two tasks can run in parallel
   (`AGENTS.md` → *Keep the pool full*). Tick a box (`- [x]`) in the same
   commit that completes it.
3. **Ask before deviating.** If a task is unclear, an API doesn't fit, or you
   would have to guess at behaviour, stop and ask. Don't improvise.
4. Every task lands with tests and synthetic data (see `CLAUDE.md`). Never
   put real bank data in the repo.
5. Commit after each logical piece (usually one task), Conventional Commits,
   then verify that commit in a clean checkout (see `CLAUDE.md`).
6. When a task is merged, move its text from this file to
   `docs/tasks-done.md` (under its milestone, in number order) in the same
   merge commit or a `docs:` commit right after. Only open tasks stay here.
7. New user requests and bug reports become numbered tasks here (next free
   number), placed in a lane below.

## Picking the next task

Task numbers record when a task was written, not when to do it. Open M3
tasks are grouped in **lanes** that touch different code, so a task from
one lane can run alongside a task from another. Within a lane, go top to
bottom: that order is the user's priority (dashboard first, then
responsiveness, set 2026-11-11). Start every task's worktree from the
current `main`.

1. **Now:** finish the two features the paused run left in flight, each
   in its existing worktree: **3.34** (uncommitted work) and **3.53 +
   3.42** (see 3.53).
2. **Then 3.62** (split `ui.rs`): the only UI branch in flight, with
   **5.1 / 5.2** (M5 backend) in the other slot. No other UI branch
   starts until 3.62 is merged.
3. **Then lanes A, B and C**, one task from each of two lanes at a time:
   - **Lane A, dashboard and refresh (first priority):** 3.56 → 3.57 →
     3.36 → 3.45 → 3.46.
   - **Lane B, Categories and Rules pages:** 3.58 → 3.48 → 3.50 + 3.51
     (one branch) → 3.49 → 3.43 → 3.60 → 3.52 → 3.47 + 3.54 (one branch,
     only after 3.57 has merged).
   - **Lane C, small tasks on other pages (fillers):** 3.63 → 3.37 → 3.61 → 3.59 (only after 3.51, 3.53 and 3.58 have merged).
   - **3.57 touches every refresh call site:** while it runs, the other
     slot takes a Lane C task or M5 backend work, never Lane B.
   - If no lane task can start (all blocked), take the next M5 backend
     task (5.1, 5.2). This is the one exception to "one milestone at a
     time": backend-only M5 work may fill an idle slot.
4. **Last in M3: 3.26** (scrollbars), once lanes A–C are empty.
5. **Then M5** (5.3, 5.4 and whatever of 5.1 / 5.2 is left), **then the
   Final task** (3.55, performance at scale).

## Where things are

| Path | What |
|---|---|
| `crates/financi-core` | `raw` (`RawFile`/`RawRecord`, `read_raw_file`), `parse` (cents, dates, merchant), `dates`, `rules` (`CompiledRules`, `pick_category`, `winning_rule`), `StatementParser` trait, `row_hash` |
| `crates/financi-parsers` | `amex`, `simplii` (old id `simplii-chequing` still resolves); `all()`, `detect()`, `by_id()` |
| `crates/financi-db` | `migrations/0001`–`0004` (0001 frozen), `accounts`, `import`, `master_csv`, `categories`, `rules`, `overrides`, `queue`, `views`, `transfers`, `settings`, `worker::DbWorker`, `seed` (feature `dev-seed`) |
| `crates/financi/src/flow.rs` | GPUI-free helpers (formatting, import flow), unit-tested |
| `crates/financi/src/ui.rs` | every view: `Shell` and all pages (split into `ui/` modules by 3.62) |
| `crates/financi/src/ui_tests.rs` | headless UI tests (pattern to copy; moves to `ui/tests/` in 3.62) |
| `crates/financi/src/version.rs`, `build.rs` | build version from `git describe` (3.39) |
| `assets/icon/` | app icon source (3.63) |
| `fixtures/demo/` | synthetic exports + `expected.json` reference totals |
| `spikes/gpui-spike` | sandbox for UI experiments; keep, don't ship |
| `docs/ui-conventions.md` | layout, spacing, colors, formats; read before any UI task |

## Coding practices (all UI tasks)

These come from earlier tasks and reviews. Follow them without being told.

- **State lives on `Shell`, created once.** Scroll handles, `SelectState`s,
  inputs and caches are `Shell` fields made in `new`, never in `render`
  (a handle made in `render` resets every frame; see 3.21).
- **No data-sized work in `render`.** Build derived lists, labels and
  formatted strings when their inputs change and cache them on `Shell`.
  Render reads the cache.
- **Refresh only what changed.** Once 3.56 / 3.57 land, every action calls
  the narrowest `RefreshScope` (the table is on `RefreshScope`'s doc
  comment). Never add a full `refresh` call for a local action.
- **Feedback within a frame:** a click either changes the screen
  optimistically (with rollback on error) or shows `.loading(true)` and
  disables the control until the DB returns.
- **One width const per column**, used by both the heading and the rows;
  tests compare the heading's and the column's left edges.
- **Text never pushes its container wider:** flex children holding text get
  `min_w_0()` plus ellipsis, and a tooltip with the full text when it can
  be cut.
- **Virtual lists:** when a row's height changes (an editor opens, an error
  line shows), update the list's item sizes in the same change.
- **Test hooks:** give elements tests need a `debug_selector` named
  `page:thing[:id]` (e.g. `cat:new-form`, `history:undo-btn`). Put new
  tests next to related ones. When a layout changes, update the old tests;
  don't delete them.
- **Pure logic goes in pure functions** (`flow.rs`, or a free fn next to the
  view) so it can be unit-tested without a window: labels, width math,
  sort keys.
- **Format through the helpers** (`flow::money`, `flow::count`, ISO dates)
  and color through theme tokens only.
- **Dependencies:** don't bump pinned versions. A crate already in
  `Cargo.lock` may be added as a direct dependency at its locked version
  (say so in the handoff); anything new, ask first.

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
- gpui-base's `virtual_list()` makes a new scroll handle on every call;
  pass the list's own `VirtualListScrollHandle` from `Shell` with
  `.track_scroll(&handle)` or it won't scroll (3.21).
- A scroll container (`overflow_y_scroll`) gives its children no height to
  fill: `flex_1` children collapse. Give every panel inside one an
  explicit height (3.45).
- Screenshots (macOS): `FINANCI_DB=/tmp/x.sqlite FINANCI_DEV_SEED=1
  FINANCI_DEV_OPEN=fixtures/demo/amex-sep-nov.csv cargo run -p financi`,
  then capture the window by id (`screencapture -o -l<id>`; find the id with
  CoreGraphics `CGWindowListCopyWindowInfo`). Window capture may need the
  user to grant Screen Recording permission. On this Linux machine no
  screenshot tool is installed: rely on the UI tests' bounds checks, and
  ask the user for a screenshot if a task needs one.

---

## M3 — Core views (one dashboard)

Layout, top to bottom: filter row; then the category chart and the monthly
summary; then the transaction table. Done tasks 3.1–3.25, 3.27–3.33, 3.35,
3.38–3.41 and 3.44 are in `docs/tasks-done.md`.

### Now: 3.34 and 3.53 + 3.42, then 3.62

Finish the two in-flight features, then split the UI code before the lanes
start.

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
  - **In progress:** worktree `../worktrees/financi/3-34-to-categorize`
    (branch `feat/3-34-to-categorize`) has ~170 uncommitted lines in
    `ui.rs` from the paused run (panel, heading row, Categorize button
    mirroring its picker). Continue from them; don't start over.

- [ ] **3.53 No category picker where the page already says it.** The
  Categories page's add-rule row and rule editor each carry a Category
  picker, but the rule list sits inside a selected category, so the
  picker mostly repeats the selection (user request, 2026-11-11).
  Replaces 3.41.
  - **In progress, with 3.42.** Branch `feat/3-41-42-rule-rows`
    (worktree `../worktrees/financi/3-41-42-rule-rows`) holds 3.42's
    commits 7d879e6 and 31dec49, plus cdeff74, a 3.41 commit (full
    category names in the rule pickers). User decision 2026-11-11: keep
    the branch for 3.53 + 3.42. First `git revert cdeff74` (keep any
    menu-width code the parent-scoped picker below can reuse), then
    implement 3.53, then review 3.42 and 3.53 together.
  - **Subcategory selected:** the add-rule row has no picker. `add_rule`
    targets the selected subcategory. The row is Pattern, regex switch,
    Add rule.
  - **Parent selected:** rules must target a child (`ensure_assignable`),
    so the add-rule row keeps a picker. It lists **only that parent's
    subcategories by their own names** (no "Parent › Child"), placeholder
    "Subcategory…", no default pick. The trigger is wide enough for the
    longest child name up to 200 px; names past that ellipsize with a
    tooltip, and the open menu is at least as wide as the longest name
    (`menu_width`, max 320 px), so no item is cut off. A parent with no
    subcategories shows "Add a subcategory first" (muted) in place of the
    add-rule row.
  - **Rule editor:** no picker on either. Edit changes pattern and regex
    only; `update_rule` keeps the rule's `category_id`. Moving a rule to
    another category is the Rules page's job (3.28's Move bar). Add a
    muted one-line hint under the rule list on this page: "To move rules
    to another category, use the Rules page."
  - Drop the now-unused `rule_edit_target`. Keep `child_choices` for the
    places that still list every child (transactions, queue, Rules page).
  - Fits 1200 × 720 and 1600 × 900 with a long subcategory name; Add
    rule / Save / Cancel paint inside the detail panel (3.23's fit rule).
  - Tests:
    - on a subcategory, no picker paints and Add rule creates a rule
      targeting it;
    - on a parent, the picker lists only its children by short name, and
      Add rule without a pick shows "Choose the subcategory the rule
      assigns.";
    - with a 40-character child name, the menu item paints unclipped;
    - a parent with no children shows the hint;
    - Edit has no picker and Save keeps the category;
    - 3.24's add/edit tests are updated, not deleted.

    Update the *Categories* row of `ui-conventions.md`.

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
  - Done on 3.53's branch; see 3.53.
  - Tests: in both rows the Switch's vertical centre equals the Pattern
    input's centre within 1 px, and the Save / Add rule button's centre
    too. Note the rule in `ui-conventions.md` (*Forms*: an unlabelled
    control in a labelled row sits in a 32 px slot, centred).

- [ ] **3.62 Split `ui.rs` and `ui_tests.rs` by page.** Pure refactor,
  no behaviour change (planned 2026-11-11 in a docs review). `ui.rs` is
  ~7 500 lines, one `impl Shell` block of ~5 500 of them, and
  `ui_tests.rs` is ~8 500 lines. Every UI task edits both, so two UI
  branches in flight conflict at merge, and one file costs an agent most
  of its 130k context to navigate.
  - `git mv crates/financi/src/ui.rs crates/financi/src/ui/mod.rs` first,
    in its own commit, so history follows the file. Then move code into
    sibling modules under `crates/financi/src/ui/`:
    - `mod.rs`: `Shell` (the struct and `new`), `Page`, `Render for
      Shell`, the page switch, `refresh` and `on_db`;
    - `widgets.rs`: shared helpers (`checkbox`, `field_label`,
      `region_title`, `filter_tip`, `category_palette`, toasts, shared
      width consts);
    - `sidebar.rs`, `dashboard.rs` (filters, chart, summary,
      `TxTable`, row menu), `queue.rs`, `categories.rs`, `rules.rs`,
      `transfers.rs`, `imports.rs` (`ImportPanel`,
      `ImportHistoryTable`).
  - Each page module adds its own `impl Shell { … }` block. Child modules
    can read the parent module's private fields, so `Shell`'s fields stay
    private. Items moved out of `mod.rs` that siblings use become
    `pub(super)`; nothing becomes `pub` that wasn't.
  - Tests: `ui_tests.rs` → `ui/tests/mod.rs` (shared helpers:
    `wait_until`, seeding, bounds helpers) plus one file per page, the
    same split as the code. Every test keeps its name. The number of
    tests listed by `cargo test --workspace --features
    financi-db/dev-seed -- --list` is equal before and after (put both
    counts in the commit message).
  - Aim for no file over ~1 500 lines. Move code; don't rewrite it.
    `git diff --stat` should show adds ≈ removes.
  - Update the docs that name the files: `CLAUDE.md` (*UI docs are for UI
    work only*: `crates/financi/src/ui/`), `AGENTS.md` (*Keep the pool
    full*: UI work lands in per-page modules), `docs/tasks.md` (*Where
    things are*), `docs/ui-conventions.md` (`ui_tests.rs` mentions). Open
    task texts that say "`ui.rs`" mean the matching module from now on;
    don't rewrite them.
  - **Scheduling:** the only UI branch in flight while it runs (the
    other slot does M5 backend work). Start it after 3.53's branch merges,
    and merge it before any other UI branch starts.

### Lane A — dashboard and refresh (first priority)

- [ ] **3.56 Dashboard filters apply at once.** Changing a dashboard
  filter (date range, accounts, search, the Options toggles, a legend /
  summary click) takes effect "really, really slowly" (user report,
  2026-11-11). Cause, from the code:
  - Every filter setter calls `Shell::refresh`, which runs **every
    page's** queries in one `on_db` job: transactions, chart, summary,
    sidebar counts, import history, category tree, transaction counts,
    rules, `rule_use_counts` (a full rule pass over every row), the
    queue, and transfer reviews.
  - `DbWorker` is one FIFO thread with no cancellation. Replacing
    `refresh_task` drops the UI continuation, but the queued DB job
    still runs.
  - Search calls `apply_search` → `refresh` on **every keystroke**, so
    typing six letters queues six full refreshes back to back, and the
    last one, the one that matters, waits behind the rest.
  - The main thread then applies each result: table refresh, history
    table, queue sync, pickers.

  The fix:
  - **Measure first** with 3.47's `FINANCI_TIMINGS=1` logging (introduce
    it here; 3.47, 3.54 and 3.57 reuse it): per-query times inside `refresh`,
    queue wait, and main-thread apply time, on the dev seed and on a
    throwaway local 10 000-row / 300-rule seed (don't commit it; 3.55
    adds the real scale seed at the very end). Numbers go in the commit message.
  - **Split the refresh.** Add `refresh_dashboard`, which runs only what
    the dashboard filters change: the filtered transactions, chart rows,
    monthly summary and the count. Every dashboard filter, selection and
    sort change calls it. `refresh` (everything) stays for data changes:
    import, undo, rule and category edits, restore. 3.57 extends this to
    every action; build it so it can, e.g. a `RefreshScope` bitset passed
    to one function.
  - **Latest wins.** Give the shell a refresh generation counter (an
    `Arc<AtomicU64>`). Each refresh captures its number, and the DB
    closure returns early without querying if a newer one has been
    requested by the time it starts. A stale result that still arrives is
    dropped on the UI side. A burst of changes thus runs at most the
    in-flight job plus the newest.
  - **Debounce search** by 200 ms (a `Timer` task replaced on each
    keystroke); Enter applies at once. The other controls apply at once.
  - **Don't re-sync untouched views:** a dashboard-scope result only
    updates the transactions table, chart and summary. It does not touch
    the history table, queue pickers, rule lists or category pickers.
  - Target: a filter change shows its result within 100 ms (DB +
    apply) on a year of data. The timed test for it is written in 3.55.
  - Tests:
    - a dashboard filter change runs only the dashboard queries (a test
      hook counts the queries per scope);
    - five rapid search keystrokes run one dashboard query, after the
      debounce;
    - a burst of three filter changes ends with the last filter's rows,
      and a stale result arriving late doesn't overwrite them;
    - an import still refreshes every page;
    - the existing filter, sort and selection tests pass.

- [ ] **3.57 The whole app responds at once.** Beyond the dashboard
  filters (3.56), the app is "just not very responsive when we change
  things like filters or buttons" (user report, 2026-11-11). It's the same
  root cause, app-wide: ~30 call sites run the full `Shell::refresh`
  after an action, all on one FIFO `DbWorker`, with nothing on screen
  until it returns. Do this right after 3.56 and build on its
  `RefreshScope` and generation counter. It touches every refresh call
  site, so pair it only with a Lane C task or M5 backend work.
  - **Scope every action.** Give each `refresh(window, cx)` call site
    the narrowest `RefreshScope` that covers what the action can change,
    and write the mapping as a table in the code (doc comment on
    `RefreshScope`). For example:

    | Action | Refreshes |
    |---|---|
    | filters, selection, sort | dashboard |
    | change a row's category, hide, unhide | dashboard + sidebar counts + queue + category counts |
    | rule add / edit / delete / move | dashboard + rules + uses + queue + category counts |
    | category add / rename / move / delete | tree + pickers + dashboard + rules |
    | import / undo / restore | everything |
    | page switch | nothing; data is already loaded |

    The full refresh stays only for data-wide changes. A test asserts
    each action's scope (via the query-count hook from 3.56).
  - **Latest wins, everywhere:** every scoped refresh goes through 3.56's
    generation check, per scope, so a burst of clicks runs at most the
    in-flight job plus the newest.
  - **Feedback within a frame.** Every button that starts DB work either
    updates the screen optimistically (row category change, hide /
    unhide, queue Categorize) or
    shows `.loading(true)` and is disabled until it finishes (as 3.40 did
    for rule Save; rule and category edits get this here, and 3.47 / 3.54
    make them optimistic later). No click may leave the screen unchanged for longer
    than one frame. Add this rule to `ui-conventions.md` (*Interaction*).
  - **Render-path audit.** Log frame times with `FINANCI_TIMINGS=1` and
    find per-frame work that scales with data. Candidates seen while
    planning:
    - `visible_rules()` clones a `Vec<Rule>` on each call;
    - `rules_page_items` rebuilds the Rules page items every render;
    - `chart_bars` and the label plan are recomputed every render;
    - `category_colors` builds a map every render.

    Cache these on the shell, rebuilt only when their inputs change.
    Target: no frame over 16 ms while hovering, scrolling or typing (the
    timed test is written in 3.55).
  - **Measure before and after:** the click-to-result time for ten
    common actions (each filter kind, change category, hide, rule save,
    rule delete, category move, queue Categorize, page switch), on the
    dev seed and 3.56's throwaway 10 000-row seed. Put the table in the commit message.
  - Tests:
    - the per-action scope table above (one test per row);
    - a click on change-category / hide / Categorize updates the row in
      the same frame, before the DB task runs, and rolls back on error;
    - a page switch issues no DB query;
    - the caches rebuild when their inputs change and not otherwise
      (counter hook);
    - the existing behaviour tests pass.

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

### Lane B — Categories and Rules pages

- [ ] **3.58 A cleaner Rules page with totals.** The Rules page (3.28)
  "is very very ugly". The user also wants each rule's **total amount**
  next to its count, and the page must stay virtualized and responsive
  (user request, 2026-11-11). Today it has 24 px rows on the page
  background with no panel, a filter row of mixed controls, plain group
  header rows, and a Move bar that pushes the list down. Keep every
  behaviour of 3.28 / 3.29 (search, Test a description, Type, Unused,
  grouping and collapse, sort, checkboxes and Move, Show rule landing);
  change the look and add the totals:
  - **Header:** "Rules" and a muted summary "N rules · M unused"
    (`flow::count`), then **New rule** (`plus` icon, primary, no "…",
    3.43) at the right.
  - **Filter row:** one 32 px row like the dashboard's (3.35): Search
    `flex_1` with a leading search icon and the placeholder "Search rules
    or categories". Then a Type select (~140 px) and an **Options**
    popover holding **Test a description** and **Unused only**, showing
    "Options · N" when N are on. Each control without a visible label
    gets a tooltip.
  - **One bordered panel** (D7, `radius`, no fill) fills the rest of the
    page and holds the list:
    - a 32 px heading row inside it (`text_sm` muted, `px_3`, bottom
      `border`), with the columns: checkbox (24 px), **Pattern**
      (`flex_1`), **Type** (80 px), **Uses** (80 px, right), **Total**
      (110 px, right), then actions;
    - **group headers** are 36 px rows on `muted` background: chevron,
      "Food › Coffee" semibold, then the muted "4 rules", and the group's
      summed Uses and Total right-aligned under their columns;
    - **rule rows** are 40 px, `px_3`, with a bottom `border` divider and
      `list_hover`. The pattern is in `mono_font_family` (regexes are
      easier to read), ellipsized with a tooltip. Type is a small
      outline tag ("text" / "regex") instead of plain text. A 0 in Uses
      is muted.
    - Edit / Delete (icon-only ghost buttons, `pencil` / `trash-2`,
      with tooltips; see 3.43) show only on row hover or when the row
      is focused or has the editor open. Their column keeps its width so
      nothing shifts.
  - **Total column:** the signed sum of the transactions the rule
    *decides* (same set as Uses), formatted as the transactions table
    does (`flow::money`, money in shown in `success`). The header's sort
    buttons get **Total** too (by absolute amount, descending).
  - **Move bar:** with rows checked it **replaces the heading row**
    inside the panel (same 32 px), with no extra row pushing the list:
    "N selected", the subcategory picker (searchable), **Move**, and
    **Clear**.
  - **DB:** replace `rule_use_counts` with `rule_use_stats(conn) ->
    HashMap<i64, RuleUse { count, total_cents }>`, which also sums
    `amount_cents`, in the same single pass. Callers (Uses on this page,
    3.49 on the Categories page, 3.52) use it.  Group sums are
    computed in Rust from the per-rule stats.
  - **Virtualized and responsive:** the list stays one `v_virtual_list`
    (headers included). Build `rules_page_items` and the per-row display
    data (formatted totals, labels) **only when** rules, stats, filters,
    collapse state or sort change, cached on the shell (3.57's render
    audit), not every frame. Search filtering is debounced by 150 ms.
    Per-row widgets (checkbox, buttons) are built only for visible rows
    (3.24's test pattern). Test with 1 000 rules built inside the test; the timed
    16 ms-a-frame budget is checked at the very end (3.55).
  - Must fit 1200 × 720: no horizontal overflow, nothing clipped, with
    long patterns and category names.
  - Tests:
    - the panel and heading row paint inside the page;
    - Total shows each rule's signed sum, matching a direct SQL sum for
      the rows it decides;
    - group headers show the summed Uses and Total;
    - sorting by Total works;
    - Edit / Delete paint only on hover / focus;
    - the Move bar replaces the heading row, and the list's top doesn't
      move when rows are checked;
    - with 1 000 rules only the visible rows are built, and the item
      cache rebuilds only on input changes (counter hook);
    - every 3.28 / 3.29 behaviour test still passes (update their
      selectors, don't delete them).

    Update the *Rules* row of `ui-conventions.md`.

- [ ] **3.48 The rule editor gives the pattern room.** Clicking Edit on
  a rule swaps the row for an editor whose Pattern input shrinks so far
  that the pattern can't really be edited (user report, 2026-11-11).
  Cause: `rule_editor_row` is one `h_flex` with Pattern `flex_1` +
  `min_w_0` and, after it, the regex switch, a 160 px Category picker,
  Save and Cancel. In the detail panel (page minus the 210 px sidebar
  and the 300 px tree panel) the fixed controls take ~400 px of ~600, and
  3.53 removes the editor's picker, which frees most of that. The user also
  saw the editor's regex switch sit off the row's line (3.42's cause).
  - **Two lines:** line 1 is the "Pattern" label over the input at the
    editor's **full width**. Line 2 is one `h_flex().items_center()` of
    32 px controls, with no label line: the regex switch, then `ml_auto`
    Save and Cancel (no Category picker, 3.53). Line 2 has no labels,
    so the switch lines up without 3.42's slot (3.42 still applies to
    the add-rule row). The preview count and error line stay under both
    lines.
  - Bump the editor's entry in `rule_sizes` (today 80 / 108 px) to the
    new height (~two control lines + label + preview, ~120 / ~148 px with
    an error) so the virtual list doesn't overlap the next row.
  - Enter saves, Escape cancels, focus lands in the pattern input with
    the caret at the end (check it does today; if not, add it).
  - The add-rule row: if its Pattern input is < 240 px wide at
    1200 × 720 after 3.53, give it the same two-line shape; otherwise
    leave it on one line.
  - Tests: at 1200 × 720 the editor's pattern input is at least 90 % of
    the detail panel's inner width; the switch, Save and Cancel
    share a vertical centre within 1 px; the row below the editor doesn't
    overlap it (bounds); Save/Cancel/Enter/Escape tests from 3.24 pass.
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

- [ ] **3.49 A Uses column instead of Category.** Since 3.23 the
  Categories page's rule list only shows the selected category's rules, so
  on a subcategory every Category cell says the same thing. The user
  wants the count of transactions each rule matches there instead (user
  requests, 2026-11-11).
  - **Replace the Category column with Uses** (`RULE_USES_W`, 80 px,
    right-aligned, `flow::count`), the same number and wording as the
    Rules page's Uses column (`rule_use_stats` from 3.58, its `count`): the transactions
    the rule *decides*, so it agrees with stored categories and with
    3.52's "decided by this rule". A heading tooltip says "Transactions
    this rule categorizes". 0 shows in `muted_foreground`. The counts are
    loaded in the same refresh as the rules (one `rule_use_stats` call; no
    per-row queries) and update after every rule change.
  - **Parent selected:** its list mixes its subcategories' rules, so the
    subcategory also shows: a muted `text_xs` tag with the child's name
    (not "Parent › Child") after the pattern, inside the Pattern column,
    ellipsized first. A subcategory selection shows no tag.
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

- [ ] **3.43 No buttons end in "…".** The user doesn't want "…" on
  buttons; use an icon instead, or just drop it (user request,
  2026-11-11). This applies 3.32's Imports-header rule to every button.
  All of them are on the Categories page today:
  - **New category…** (`NEW_CATEGORY`, header, primary): `plus` icon +
    "New category".
  - **Rename…** (detail panel title row): `pencil` icon + "Rename".
  - **Delete…** in the Delete form (3.51 moved it there; `confirm_label`):
    `trash-2` icon + "Delete"; armed, it still reads "Click again to
    delete" in `danger`.
  - Rule rows (on both the Categories and the Rules page; skip what
    3.58 already changed): **Edit** gets `pencil`, **Delete…** becomes `trash-2` +
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

- [ ] **3.60 "Category type" label.** From `plan.md` → *Label the
  category type dropdown* (never made a task; user confirmed
  2026-11-11). The New category form labels its kind picker "Kind". Make
  the field label "Category type" (`ui.rs`, the `cat:new-form`). The
  detail panel's muted "Expense · N transactions" line shows the value,
  not the word, and stays. Check no other user-visible "Kind" remains.
  Keep the form fitting the header row at 1200 × 720. Tests: the form
  paints "Category type" and no "Kind" label. Tick the `plan.md` box and
  update the *Categories* row of `ui-conventions.md`.

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
    Use one `CompiledRules` per call, as `rule_use_stats` (3.58) does, so the
    "decides" set agrees with Uses (3.28). Loaded through `DbWorker` when
    focus changes and after every refresh while focused; a stale result
    for a previous focus is dropped (keep the `Task`, drop on refocus).
  - **While editing** the focused rule, the panel keeps showing the
    saved rule's matches (the live count under the editor is the
    preview); it reloads after Save.
  - Empty state inside the panel: "This rule matches no transactions".
  - Tests:
    - DB: statuses for decides / won-by / locked rows, consistent with
      `rule_use_stats`; an invalid regex returns empty.
    - UI: clicking a row focuses it and the panel paints below the list
      inside the section; the summary counts are right; a won-by row shows
      its tag; Escape and re-click clear focus; focus changes drop a late
      result; at 1200 × 720 both the list and the panel are at least their
      minimum heights with 3.51's forms closed.

    Update the *Categories* row of `ui-conventions.md`.

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
  - **After 3.57.** 3.56 / 3.57 scope the refresh and add the
    `FINANCI_TIMINGS=1` log; this task adds optimistic list updates and
    fixes whatever the measurements still show.
  - **Measure first.** With 3.56's `FINANCI_TIMINGS=1` log, record the time from the confirming click to (a)
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
  - Tests: after the confirming click, the rule is gone from the list in
    the same frame, before the DB task completes (run the UI without
    advancing the executor); a failing delete (rule removed underneath)
    restores the row and shows the error; an edit's new pattern shows
    before the save completes; a new rule appears before its add
    completes and then carries its real id. The 3.24 delete and edit
    tests still pass.

- [ ] **3.54 Category changes don't lag the page.** Deleting a category
  and moving a subcategory (and likely add / rename) freeze or lag the
  page noticeably (user report, 2026-11-11). The DB side is small:
  `move_child` is one `UPDATE`, `delete_category` two `UPDATE`s and a
  `DELETE`, with no rule re-run. That leaves two suspects:
  - (a) As in 3.47, the change waits for the full `refresh`, which re-runs
    every page's queries.
  - (b) **UI-thread work on the result.** A changed category list
    re-sets items on every category picker: `sync_queue` re-sets every
    surviving queue picker, plus the rule / move / delete targets, the
    Rules page's Move picker and the table delegate's `child_choices`.
    The tree virtual list is also rebuilt.

  There's also no index on `transactions(category_id)`, so the
  reassign `UPDATE` and `transaction_counts` scan the table.
  - **Measure first,** with 3.56's `FINANCI_TIMINGS=1` logging, after
    3.57's scoped refreshes (do 3.47 and 3.54 in one branch), on the dev-seed DB and on a copy of the real
    DB. Log for each of delete / move / add / rename: click to DB
    returning, to refresh result, the refresh's per-query times, and the
    **main-thread time spent applying the result** (from entering the
    `update_in` closure to its return, broken down by picker re-sets and
    table refresh). Numbers go in the commit message; fix what dominates.
  - **Optimistic tree update:** delete removes the node (and its row
    count moves to the reassign target) and move re-parents the node in
    `self.categories` right away, with `cx.notify()`; roll back and show
    the error on failure. The refresh then reconciles.
  - **Cheaper picker refresh:** build the `Names` list once per refresh
    and share it (`Rc`/`SharedString`s), and re-set items only on pickers
    that are **built and visible** (queue pickers are already lazy per
    3.19; do the same for the rest). Other pickers re-set lazily when
    next opened or shown (a `categories_version` counter compared on
    render).
  - **Index:** if the measurements show the scans matter, add a numbered
    migration `CREATE INDEX tx_category ON transactions(category_id)`
    (0001 stays frozen).
  - Tests:
    - after a confirmed delete or a move, the tree shows the change in
      the same frame, before the DB task completes;
    - a failing move (e.g. target removed underneath) rolls back and
      shows the error;
    - after a category change, the hidden pickers are re-set on their
      next render, not during the refresh (count re-sets with a test
      hook);
    - if the index is added, a migration test checks it exists;
    - the existing category CRUD tests pass.

### Lane C — small tasks on other pages

- [ ] **3.63 An app icon.** The window and Dock / taskbar show the
  default icon (user request, 2026-11-11). The source artwork is
  committed: `assets/icon/financi.svg` (1024 × 1024, macOS icon grid: a
  dark rounded square with three rising bars, the tallest green). Don't
  redesign it; ask the user if it needs changes.
  - **Rendered PNGs, committed:** `assets/icon/png/financi-<n>.png` for n
    = 16, 32, 64, 128, 256, 512, 1024, made by `scripts/icons.sh` with
    `rsvg-convert` (installed here) and exposed as `mise run icons`. The
    build reads the PNGs, so it needs no SVG renderer.
  - **Linux, X11:** set `WindowOptions.icon` (`gpui-pre`
    `platform.rs`, X11 only) from the 256 px PNG, embedded with
    `include_bytes!` and decoded with the `image` crate. `image` 0.25 is
    already in `Cargo.lock` through GPUI; add it as a direct dependency at
    that version with `default-features = false, features = ["png"]`.
  - **Linux, Wayland:** compositors ignore per-window icons and match the
    window's `app_id` to a desktop entry. Set `WindowOptions.app_id =
    Some(APP_ID)` (`const APP_ID: &str = "financi"`) and add
    `packaging/linux/financi.desktop` (`Name=financi`, `Exec=financi`,
    `Icon=financi`, `StartupWMClass=financi`, `Categories=Office;Finance;`).
    `mise run install-desktop` copies it to `~/.local/share/applications/`
    and the PNGs to `~/.local/share/icons/hicolor/<n>x<n>/apps/financi.png`.
  - **macOS:** the Dock icon comes from the app bundle. Add `mise run
    bundle` (`scripts/bundle-macos.sh`): build release, make
    `target/release/financi.app` with an `Info.plist`
    (`CFBundleIconFile`, `CFBundleIdentifier` `dev.financi.app`,
    `CFBundleShortVersionString` from the 3.39 version) and an `.icns`
    made with `iconutil` from an `.iconset` of the PNGs. For `cargo run`,
    also set the Dock icon at startup with
    `NSApplication::setApplicationIconImage` through `objc2-app-kit`
    (0.3.2 is already in `Cargo.lock`; add it as a macOS-only dependency
    at that version). If either dependency can't be added at the locked
    version, stop and ask.
  - Tests: the embedded PNG decodes to 256 × 256 RGBA; the desktop file's
    `Icon` and `StartupWMClass` equal `APP_ID`; `scripts/icons.sh` is
    idempotent (re-running changes no committed PNG). Check by hand on
    this Linux machine that the taskbar shows the icon after `mise run
    install-desktop`, and say which session type (X11 / Wayland) you
    checked. Add the new `mise` tasks to `CLAUDE.md` → *Commands*.

- [ ] **3.37 In-row Undo says "Undo".** 3.32 made the newest history
  row's Undo an icon-only button, which the user finds unclear. Make it
  the `undo-2` icon **plus the label "Undo"** (still `.xsmall().ghost()`,
  vertically centered with ≥ 4 px above and below); armed it still reads
  "Click again to undo" in `danger`. Keep the "Undo this import" tooltip.
  Recompute `history_undo_col_w` so both labels show in full and the
  column is no wider than needed. Tests: the idle button's text is
  "Undo" and it paints an icon; 3.32's bounds and fit tests still pass
  in both states. Update the *Imports* row of `ui-conventions.md`.

- [ ] **3.61 The import preview names the institution.** From `plan.md`
  → *Make account selection on import clearer* (never made a task; user
  confirmed 2026-11-11). The account picker already lists "Institution ·
  Account" (`Account::display_name`), but the preview's only sign of the
  institution is a muted `text_xs` "Detected format: …" after the file
  name, which is easy to miss when choosing which card a file belongs to.
  - Line one of the preview: file name, then the **institution name**
    (e.g. "American Express", from the parser's institution, not the
    parser id) as an outline tag in `text_sm`, replacing "Detected format:
    …".
  - The "Into" picker lists only that institution's eligible accounts, as
    now; check each item reads "Institution · Account" and that "+ New
    account…" creates the account under the detected institution.
  - Tests: the preview paints the institution name and no "Detected
    format"; the picker items carry the institution. Tick the `plan.md`
    box and update the *Import preview* bullet of `ui-conventions.md`.

- [ ] **3.59 Searchable category pickers.** From `plan.md` → *Make
  category dropdowns usable with thousands of categories* (never made a
  task; user confirmed 2026-11-11). A flat scrolling list doesn't work at
  that size, and same-named subcategories under different categories
  can't be told apart.
  - gpui-component's `SelectState` has `.searchable(true)`
    (`gpui-component-0.9.0/src/select.rs`, ~line 305: a search input at
    the top of the menu), `SearchableVec` and `SelectGroup` for grouped
    items, and `Select::menu_width`. Use them; don't hand-roll a picker.
  - **Which pickers:** every `Select` that picks a category or
    subcategory: the To categorize row pickers, the Rules page Move bar,
    the Categories page's Move form (categories) and Delete form
    (subcategories), the parent-scoped add-rule picker (3.53; searchable
    once it has more than 10 items), and any others a grep for
    `child_choices` / `SelectState::new` finds. The table's right-click
    *Change category…* submenu is a menu, not a `Select`; leave it.
  - **Items:** subcategories grouped under their category (`SelectGroup`,
    the category as the group title), so each result is seen with its
    category. Typing matches category **and** subcategory names, case-
    insensitive: "food" shows every subcategory of Food. If grouped search
    can't match group titles, use flat items labelled `Category ›
    Subcategory` with the category part muted, and say which you chose in
    the handoff.
  - Keyboard: the search input has focus when the menu opens; arrows move,
    Enter picks, Escape closes. The menu is wide enough for the longest
    label up to 320 px; longer labels ellipsize.
  - Renders only the visible options (check the gpui-component list is
    virtual; if it isn't, stop and report).
  - Tests: with 2 000 subcategories across 200 categories, typing a
    subcategory name shows it with its category and Enter picks it;
    typing a category name lists its subcategories; two same-named
    subcategories under different categories are both shown, each under
    its category; opening the menu builds only the visible items; the
    existing categorize / move / delete tests pass. Add a rule to
    `docs/ui-conventions.md` (*Interaction*): every category picker is
    searchable and shows the category with each subcategory. Tick the
    `plan.md` box.

### Last in M3

- [ ] **3.26 A scrollbar on everything that scrolls.** Most scrolling
  areas give no sign that there is more content or where you are in it.
  Every scrollable region shows a vertical scrollbar whenever its content
  is taller than its viewport, and none when it fits. Do this last in
  M3: 3.45, 3.52, 3.58 and 3.59 add or re-lay out scrolling regions.
  - **Find every region at the time:** grep the UI code for
    `overflow_y_scroll`, `overflow_x_scroll` and `v_virtual_list`. Besides
    the lists below, that includes the Rules page list (`rules_scroll`),
    the dashboard page scroll (3.45), the rule matches panel (3.52) and
    the summary's month strip (3.30 already has its horizontal bar). 3.36
    removes the scrolling legend list.
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
`expected.json`, filters compose, and every M3 task above is merged.

---

## M5 — Export / restore

Design: `plan.md` → *Export and restore*. The per-account, per-year master
CSVs already exist from 2.2; export copies them instead of rebuilding
anything. 5.1 and 5.2 are backend only and may run in a free slot while M3
is still open (see *Picking the next task*); 5.3 and 5.4 wait for M3.

- [ ] **5.1 Export, backend** (`financi-db/src/export.rs`, crates `zip`,
  `serde_json`).
  - `export(conn, root, dest, app_version)`: run the 2.2 self-check, then
    zip `financi.sqlite` (snapshot via `VACUUM INTO` or the backup API),
    `manifest.json`, and the `csv/` tree copied as-is (no rebuilding).
  - `manifest.json`: financi version (passed in by the app, from 3.39's
    `version.rs`), schema version, exported_at, and per file:
    account_key, institution, account name, kind, year, row count,
    SHA-256; plus the database file's SHA-256.
  - Tests: the zip holds the three parts; every manifest hash and row
    count matches the file; a missing year file is rebuilt by the
    self-check before zipping; hidden rows are in the CSVs.

- [ ] **5.2 Restore, backend.**
  - `verify_export(zip)` checks every file against the manifest SHA-256s
    and refuses a schema version newer than this build. `restore(...)`
    backs up the current database to `financi.sqlite.bak` (backup API)
    and `csv/` to `csv.bak/`, restores into the live connection with
    rusqlite's backup API on the worker thread (no file swapping),
    migrates an older schema, and replaces `csv/` with the zip's tree.
    Expose it as a `DbWorker` job.
  - Tests: a tampered file is refused before anything changes; a newer
    schema is refused; an older one migrates; the `.bak` copies exist
    after a restore; a failure part-way leaves the live database usable.

- [ ] **5.3 Export and restore in the app.**
  - File → Export… (⌘E) and the Imports page's **Export** button open a
    save dialog defaulting to `~/Desktop/financi-export-YYYY-MM-DD.zip`.
  - File → Restore…, the Imports page's **Restore** button and a
    "Restore from export…" link on the empty-state screen open a file
    dialog, then restore and refresh every view. Both say what happened in
    a toast; errors stay until closed.
  - Remove the "… is not available yet." toasts (`Shell::export_data`
    and its Restore twin). UI tests for both flows
    (stub the dialogs), and update the *Imports* row of
    `ui-conventions.md`.

- [ ] **5.4 Acceptance tests** (all must pass):
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

**Done when:** 5.1–5.4 pass. Then do the final task below.

---

## Final — performance at scale (last of all)

Runs after M5, when every other task is done (user decision, 2026-11-11).

- [ ] **3.55 Fast at real scale: hundreds of rules, dozens of
  categories.** The user has hundreds of rules and dozens of categories
  and is worried about performance (2026-11-11). The dev seed (18 rules,
  458 rows) hides scale problems. `CompiledRules::pick` loops over **every
  rule for every row**, and a rule change runs that loop twice: once in
  `reapply_rules`, and again in `rule_use_stats` (3.58) inside `refresh`. At
  1 000 rules × 100 000 rows (10 years at the user's ~10 000 a year)
  that's ~100 M checks per pass, likely around a
  second or more. The live preview (`count_rule_matches`) scans every row
  on each keystroke. **Runs last** (user decision, 2026-11-11): after M5 and every other
  task. Earlier tasks measure on the dev seed and a throwaway local seed;
  the committed scale seed and the timed budgets land here.
  - **Scale fixture:** a `seed_large` (dev-seed feature) and a
    `FINANCI_DEV_SEED=large` switch. It builds **1 000 rules** (80 %
    substring, 20 % regex, overlapping on purpose), **80 categories**
    (15 parents), and **100 000 transactions** over 10 years (the user has ~1 800 today
    and expects ~10 000 a year) with realistic
    merchant strings, all deterministic. It uses only synthetic data.
  - **Budgets** (release build, on the large seed), as `#[ignore]` perf
    tests runnable with `cargo test --release -- --ignored perf_`, which
    print their times:
    - `reapply_rules` < 150 ms;
    - `rule_use_stats` < 150 ms;
    - `count_rule_matches` < 30 ms;
    - the full `refresh` query set < 250 ms;
    - main-thread apply of a refresh result < 16 ms;
    - opening the Categories and Rules pages < 16 ms per frame;
    - a dashboard filter change shows its result within 100 ms (from
      3.56);
    - no frame over 16 ms while hovering, scrolling or typing, on the
      one-year seed (from 3.57);
    - the Rules page with 1 000 rules scrolls and filters under 16 ms a
      frame (from 3.58).

    Debug-build versions of the DB budgets (×10) run in the normal test
    suite on a 10 000-row / 300-rule seed (one year) so regressions fail CI.
  - **Matching in one pass per row:** in `CompiledRules`, replace the
    per-rule loop with an **Aho-Corasick** automaton over the lowercased
    substring patterns (`aho-corasick` crate, already in the tree via
    `regex`; `MatchKind::Standard` with overlapping search to collect
    every match) and one **`RegexSet`** for the regex rules (case-
    insensitive). Each row is scanned once per kind; the winner is the
    longest match, ties to the lowest id, substring before regex, exactly
    as today. Invalid regexes stay excluded. The existing core tests are
    the contract; add a property test that the new `pick` equals the old
    per-rule loop on random rules and descriptions.
  - **One pass per rule change:** have `reapply_rules` return the per-rule
    use counts it computed, and cache them on the shell until the next
    rule or import change, so `refresh` doesn't recompute them. If that
    isn't enough, store the deciding rule: add a nullable
    `transactions.category_rule_id` column in a new migration, set by
    `reapply_rules` and at import. Uses then becomes a `GROUP BY`, and
    3.52's "decides" set becomes a query. Only add the column if the
    budgets need it.
  - **Live preview:** debounce `preview_rule_match` by 150 ms, and keep
    the in-flight task cancellable, as now.
  - **UI at scale:** with 1 000 rules and 80 categories, check that the
    rule lists, tree and pickers stay virtual or lazy (3.12, 3.19, 3.21).
    Category pickers with 65 children stay usable (3.59 made
    them searchable; check it holds at this size).
  - Commit message: the before / after numbers for every budget.

---

## After v1 (not now)

v1.1 spend-over-time line + top merchants · v1.2 flags (large, new merchant,
FX/fees) · v1.3 recurring/subscriptions · default categories and merchant
rules on first run (`plan.md` → *Categorization*; moved out of v1
2026-11-11) · more institutions · CSV export of filtered rows. Real Simplii
Visa sample → revisit 4.1.
