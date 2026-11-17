# M3 — the last feature tasks

The architecture and backend queues are done. What's left of M3, one task
from each of two lanes at a time, top to bottom within a lane
(`docs/tasks.md` → *Picking the next task*). UI tasks: read
`docs/tasks/gpui.md` and `docs/ui-conventions.md` first.

## Lane A — dashboard layout, labels and the pie chart

Order: 3.98 → 3.97 → 3.100 → 3.101 → 3.99. The dashboard becomes a dock
of resizable, rearrangeable panels (user decision, 2026-11-17: dock
splits, not a scrolling grid).

- [ ] **3.98 Month labels on the stacked bars aren't cut off.** On
  *Monthly spending by category* the month labels under the bars are
  clipped (user report, 2026-11-17).
  - **Cause:** `StackedBarChart` (`ui/widgets.rs`) puts each label in
    its own column, sized `w_full()` with `overflow_hidden()` and
    `text_ellipsis()`. `month_label_plan` thins the labels so that, for
    example, every third one shows, but a shown label is still only as
    wide as its own column, so "Oct 2026" is clipped whenever a column is
    narrower than the text (6+ months at the minimum width, and every
    long range). Also check the vertical fit: the label row is a fixed
    `STACKED_LABEL_H` = 16 px, which may be shorter than `text_xs`'s line
    height.
  - **Fix:** paint each shown label in an absolutely positioned,
    unclipped box centred on its column, as wide as the label width
    `month_label_plan` budgets for (8 characters plus `CHART_LABEL_GAP`).
    It may overlap the neighbouring columns, which `bar_label_slots`
    already leaves empty. Clamp the first and last labels so they stay
    inside the plot instead of being cut off at the edge. Derive the label
    row height from the measured line height (the same font metrics behind
    `chart_char_px`), not from a literal. Keep the click target on the
    label and the `stacked:month-label:<ix>` selector. If "Oct 2026"
    still leaves too few labels on narrow plots, show "Oct" and add the
    year only on January and on the first shown label ("Jan 2026"), and
    record that in `ui-conventions.md`.
  - Tests (`ui/tests/dashboard/stacked_bars.rs`): with 3, 12 and 36
    months at 1200 × 720, every painted label's bounds are at least as
    wide as its text's shaped width (no ellipsis), no two labels overlap,
    and every label sits inside the plot's bounds; clicking a label still
    selects its month.

- [ ] **3.97 Each dashboard panel is its own entity.** Refactor for
  3.100, no visible change. Today `DashboardPage` renders all four panels
  from methods (`chart_panel`, `stacked_panel`, `summary_panel`, and the
  table wrapper in `ui/dashboard/render.rs`). gpui-base's dock holds
  panels as entities (`PanelView`), so each needs its own.
  - Move the shared state (filter, narrow, selection, loaded
    `DashboardData`, caches from 3.85) into one `DashboardState` entity
    that owns the loads and emits a change event, as `Catalog` does
    (3.78). `ChartPanel`, `StackedPanel`, `SummaryPanel` and
    `TablePanel` are entities that read it, subscribe to it, and write
    selections back through it. The filter row stays on the page.
  - Each panel keeps its own measured-bounds probes (3.86), scroll
    handles and debug selectors (`panel:chart`, `panel:stacked`,
    `panel:summary`, `panel:table`), so tests keep finding them.
  - Each panel entity re-renders only when the state it reads changes (no
    regression on 3.85's per-frame work).
  - Split files the way `AGENTS.md` → *File size* says
    (`ui/dashboard/panels/…`). `refactor:` commit.
  - Tests: every existing dashboard test passes unchanged except for
    the paths it imports; add one that a selection made in the summary
    reaches the chart and table panels through `DashboardState`.

- [ ] **3.100 The dashboard is a dock of resizable, movable panels.**
  The four panels stacked in one scrolling column (chart, monthly bars,
  summary, transactions) aren't usable: the user wants to rearrange
  them and resize them both ways (user request, 2026-11-17). Two related
  reports from the same day are fixed here too: the panels touch with no
  gap between them, and the dashboard has no scrollbar.
  - **Component:** gpui-base's dock (`gpui_base::dock`: `DockArea`,
    `DockLayout`, `PaneTree`, `TabGroup`, the `Panel` trait; module docs
    in `gpui-base-0.9.0/src/dock/mod.rs`). It supplies splits with
    draggable dividers, drag-and-drop of panels into other splits or tab
    groups, and keyboard handling. We supply the appearance through
    `DockAreaRenderer` / `TabGroupRenderer`. Don't hand-roll splits.
  - **Default layout** (the user's pick): a vertical split of two rows,
    top row Chart | Monthly bars, bottom row Summary | Transactions, all
    halves 50 %. The filter row stays pinned above the dock, outside it.
  - **Appearance:** each panel keeps the `Panel` look (border, radius,
    `region_title`). The title row is the drag handle (`cursor_grab`).
    Gutters between panels are 12 px (`gap_3`, "between page regions"),
    with the divider's hit area in the gutter and a `border` colored
    line on hover or drag. The drop indicator is a `list_active` overlay
    of the target half or tab. Tabs appear only when two panels share a
    group: a `text_xs` tab strip in the title row.
  - **No page scroll:** the dashboard scroller and its
    `bounds_height_probe` go away, and the dock fills the space under
    the filter row. Each panel scrolls inside itself when its content is
    taller (summary rows, legend, stacked months sideways; the table
    already does), with gpui-base's `Scrollbar` always visible while the
    content overflows. That answers "no scrollbar on the dashboard" now,
    without waiting for 3.26.
  - **Minimum sizes:** a panel can't be dragged smaller than 240 × 160
    (the table 320 × 200). Dividers stop there. Everything must still
    fit at 1200 × 720.
  - **Gaps trap:** the missing gap came from `.gap_3()` on the old
    scroller, which wasn't a flex container (GPUI applies `gap` only to
    flex and grid). Grep `ui/` for other `.gap_*` calls on non-flex divs
    and fix them; add the pitfall to `docs/tasks/gpui.md`.
  - Update the *Dashboard* paragraph and *Layout* section of
    `ui-conventions.md` (dock, gutters, drag handle, per-panel scroll)
    and the *Dashboard* list in `plan.md`.
  - Tests (`ui/tests/dashboard/layout.rs`, plus a `dock.rs`): the
    default layout puts the four panels in a 2 × 2 grid with 12 px
    between neighbours (bounds) at 1200 × 720 and 1600 × 1000; dragging
    the vertical divider widens one half and narrows the other, and stops at
    the minimum; dragging the Summary title onto the Chart's right half
    moves it there; dropping it on the Chart's title makes a tab group
    and switching tabs shows each panel; a summary with 40 rows shows a
    scrollbar inside `panel:summary` and the amounts don't sit under it;
    filters, drill-down and selection still reach every panel after a
    move.

- [ ] **3.101 The dashboard layout is remembered, and can be reset.**
  After 3.100.
  - Save `DockArea::dump()` (`DockAreaState`, JSON) to the `settings`
    table under `dashboard.layout` through `DbWorker`, 500 ms after the
    last `DockEvent::LayoutChanged` (one keyed latest-wins job, 3.77).
    Restore it with `DockArea::load` and a `PanelRegistry` that builds
    the four panels by name.
  - A stored layout that fails to load (bad JSON, an unknown panel, a
    panel missing) falls back to the default layout and logs once. It
    never blocks startup. Store a layout version; a version bump resets
    to the default.
  - **Reset layout:** an icon button (Lucide `layout-dashboard`,
    tooltip "Reset layout") at the right end of the filter row, and a
    *View > Reset dashboard layout* menu item. It restores the default
    and saves it.
  - Tests: DB — the setting round-trips; UI — move a panel and resize a
    divider, rebuild the window from the same DB, and the layout is the
    same (bounds); a corrupt stored value gives the default layout;
    Reset layout restores the default and stores it.

- [ ] **3.99 Spending by category as a pie chart.** Replace the
  *Spending by category* `BarChart` with a donut chart (user request,
  2026-11-17). Do it last in the lane, after the dock layout (3.100), so
  it is laid out for a resizable panel; it reworks the chart panel.
  - **Component:** gpui-kit's `PieChart`
    (`gpui_kit::component::chart::PieChart`, source in
    `gpui-component-0.9.0/src/chart/pie_chart.rs`). It draws arcs with a
    hover lift and tooltip (`name`, `value`, `inner_radius`,
    `outer_radius`, `pad_angle`, `interactive`). It takes no clicks,
    same as the bar chart today, so the legend stays the click target.
  - **Shape:** a donut (`inner_radius` about 60 % of the outer one) with
    the range's total spend in the hole (`text_lg` semibold amount,
    `text_xs` muted "Expense" under it; the drilled parent's name and
    total while drilled). Slices keep the category palette and order of
    today's bars (largest first, clockwise from 12 o'clock).
  - **Many categories:** the largest 8 slices are shown, the rest are merged
    into one muted "Other" slice. The legend still lists every category,
    and Other is a legend chip too (tooltip lists what it holds; clicking it
    does nothing). Drilled into a parent, its children get the same rule.
  - **Selection:** a selected category (legend or summary click) shows
    the way the bar chart shows it: other slices dim to about 35 %.
    Tooltips stay `Name` over `amount · share`.
  - **Layout:** the panel is resizable (3.100), so it adapts to its
    measured bounds (3.86 probes): wider than tall, the donut sits left
    as a square of the panel's inner height with the legend chips
    wrapping to its right; otherwise the legend wraps under the donut.
    The donut never drops below 120 px across; under that the panel
    shows the legend alone.
    Drop the share labels and band-label thinning (3.15 / 3.31) that only
    the bars need, together with their tests. Keep refunds netting as
    today; a category whose net is ≤ 0 has no slice. Fit 1200 × 720.
  - Update the *Dashboard* paragraph of `ui-conventions.md` (the chart
    `Panel` description) and the *Category chart* line of `plan.md`
    (UI requirements).
  - Tests (`ui/tests/dashboard/chart.rs`, replacing the bar-only ones):
    the slice values equal `spend_by_category` for the range; with 12
    parents there are 8 named slices plus Other summing to the total;
    drilling shows the children's slices and the parent's total in the
    hole; a selected category dims the others; an empty range shows the
    existing empty state; at the default layout and at the panel's
    minimum size the donut and legend fit inside the panel and switch
    between side-by-side and stacked as its aspect changes (bounds).
    `expected.json` totals still match.

## Lane B — Categories and Rules

Order: 3.102 → 3.103 → 3.47 + 3.54 (one branch).

- [ ] **3.102 Preview a pattern's matches, backend.** For 3.103 (user
  request, 2026-11-17). Backend only.
  - `preview_pattern_matches(conn, pattern, is_regex, exclude_rule_id,
    limit) -> Result<PatternPreview>` in `financi-db/src/rules.rs`, with
    `PatternPreview { total: i64, decides: i64, rows: Vec<RuleMatchRow> }`.
    It compiles the pattern once (as `count_rule_matches` does) and
    computes each matching row's status **as if the pattern were a new
    rule**: `Decides` if it would win, `WonBy(id)` if an existing rule
    would still win (same tie-break as `CompiledRules::pick`; a new rule
    gets the highest id, so it loses ties), `Locked` for locked rows.
    Hidden and card-payment rows are excluded, as in `rule_matches`.
    Rows: `Decides` first, then newest posted date, capped at `limit`;
    `total` and `decides` count every match. An empty or invalid pattern
    returns an empty preview, never an error. `exclude_rule_id` is the
    rule being edited, as in `count_rule_matches`.
  - Expose it through `DbWorker` as a keyed latest-wins read (3.77), the
    way the live count already runs. `count_rule_matches` stays for its
    other callers unless this replaces all of them.
  - Tests: total / decides / rows on the dev seed for a substring and a
    regex pattern; a pattern an existing longer rule beats reports
    `WonBy` that rule; a tie with an existing equal-length rule reports
    `WonBy` it; locked, hidden and card-payment rows behave as above; the
    cap limits rows but not the counts; empty and invalid patterns give
    an empty preview.

- [ ] **3.103 A tidier add-rule form with a match preview.** On the
  Categories page, the add-rule row under a category's rules crams a
  Pattern input, the regex switch, the Subcategory picker (when a
  category is selected), the Add rule button and the "matches N" count
  into one line. The user finds it messy (user request, 2026-11-17).
  It stays visible by default (user's call); re-lay it out and show what
  the pattern would catch. Needs 3.102.
  - **Layout:** the add-rule row becomes a two-line `InlineForm`, the
    same shape as the rule editor (3.48), so adding and editing look
    alike. Line 1: "Pattern" label over a full-width `Input`. Line 2:
    the regex `Switch` at the left, then the "Subcategory…" `Select`
    when the selection is a category (the 3.53 rules for which picker
    shows, unchanged), then, right-aligned, the match summary and
    **Add rule**. No `control_slot` hacks on one line. Update the
    `ui-conventions.md` *Categories* paragraph so the add-rule row is
    described as this form, and change the convention rather than
    adding an exception (`docs/tasks.md` → *Writing tasks*).
  - **Match preview:** while the pattern is non-empty, a compact
    `DataList` under the form shows up to 8 of the matching transactions
    from `preview_pattern_matches` (Date, Merchant ellipsized, Amount)
    with a muted tag on rows another rule would keep ("kept by
    `<pattern>`") or that are locked ("locked"). Above it, a summary:
    "Matches 42 transactions · 37 would move here". Under 8 rows the list
    shrinks to fit. "No transactions match" when empty, and nothing at
    all for an empty pattern. The query reruns 150 ms after the last
    keystroke (latest wins). An invalid regex shows the existing error
    text in place of the preview.
  - Reuse the same preview in the rule editor (Edit on a rule row),
    with `exclude_rule_id` = the edited rule. The Rules page's matches
    panel (3.52) is unchanged.
  - **Space:** the detail panel scrolls if form + preview + rules
    outgrow it. At 1200 × 720 the form and a full preview fit without
    pushing the rule list out of view entirely.
  - Tests (`ui/tests/categories*`): the form's two lines (pattern on
    its own line, switch / picker / button on the second) by bounds;
    typing a pattern shows the summary and up to 8 preview rows matching
    3.102's result; a row another rule keeps shows its tag; clearing the
    pattern hides the preview; an invalid regex shows the error; Add
    rule still adds the rule to the right subcategory (the 3.53 tests
    pass); the editor shows the preview without counting its own rule.

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
  - **Review update (2026-11-11):** Do it after 3.84 (feedback within a frame) and 3.64 (per-action tasks); `FINANCI_TIMINGS` comes from 3.77, and the scoped reloads from 3.78 / 3.57.
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
  - **Review update (2026-11-11):** Do it with 3.47, after 3.84. The `Names` list built once and shared, and the `transactions(category_id)` index, are already done by 3.78 and 3.73: drop those bullets and measure what's left.
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

## Lane C — small tasks

Order: 3.105 → 3.95 → 3.104 (3.105 and 3.95 both change `ui/queue.rs`).

- [ ] **3.105 To categorize buttons work after scrolling.** Scroll far
  enough down the To categorize page and no Categorize button works
  (user report, 2026-11-17). Reproduced headlessly: a visible row's
  picker is a new entity on every frame, so its menu never opens and a
  pick never lands. The button stays disabled.
  - **Cause:** gpui-base 0.9.0's `VirtualList` calls the row closure
    twice per frame: once in `request_layout` to measure one row
    (`measure_item` → `render_items(0..1)`, `virtual_list.rs:325`), and
    again in `prepaint` for the visible range (`:706`). `DataList`
    (`ui/widgets.rs`, the `v_virtual_list` closure) passes both calls to
    `on_pass`. So every frame `QueuePage::note_paint_pass(0..1)` runs
    first, and the cells callback for row 0 then runs `prune_visible`
    with a keep window of rows 0–24. That drops every un-picked picker
    further down, and `prepaint` builds new ones for the visible rows.
    The click, hitboxes and merchant lookups are fine.
  - **Fix:** in `DataList::render`, don't report the measure call to
    `on_pass`. It is always the first call of the closure in a render (a
    `Cell` counter created per render; or detect it as the one-row call
    made before any visible pass). Also skip page-side work that only
    belongs to a real pass during the measure call: pass a `measuring`
    flag to `cells` (or a separate `measure_cells`), so `QueuePage`
    neither builds a picker for row 0 nor prunes while measuring. Check
    every other `on_pass` / cells user (Categories tree, Rules list) for
    the same window shrink, and fix them the same way.
  - Add the pitfall to `docs/tasks/gpui.md`: `v_virtual_list`'s closure
    also runs once per frame to measure a single row, so it must not
    hold side effects keyed to "the visible range".
  - Tests (`ui/tests/queue.rs`): with a 300-merchant queue scrolled to
    row 150, a visible row's picker keeps its entity id across two
    frames; opening it and picking a subcategory enables the row's
    Categorize, and clicking it adds the merchant's rule and removes the
    row; the same at the end of the list; the 3.83 prune test still
    passes (pickers far from the visible range are still dropped).
    `DataList` unit test: `on_pass` sees only the visible range, once
    per frame.

- [ ] **3.104 Rename is the same size as the other category actions.**
  On the Categories detail title row, Rename is bigger than Add
  subcategory, Move and Delete (user report, 2026-11-17).
  - **Cause:** in `ui/categories.rs` the `rename-category` `Button` has no
    `.small()`, while `title-add-sub`, `title-move` and `title-delete`
    all have it (the convention says the title row's actions are
    `small`).
  - **Fix:** add `.small()`, and order the group the way
    `ui-conventions.md` lists it (Rename; Add subcategory, Move, Delete)
    if it doesn't already match. Look for other button rows where one
    button's size differs from its neighbours (`grep -n 'Button::new'
    ui/`), and fix those in the same commit.
  - Tests: on a selected category and on a selected subcategory, every
    title-row button has the same height (bounds); an existing 3.43
    Rename test still finds its pencil icon.

## Housekeeping

- [ ] **H5 Stale comments and test nits from reviews.** Pure clean-up,
  `refactor:`/`test:` commit, no behaviour change; run when a slot is idle
  and nothing in flight touches these files. Collected from review notes
  in `docs/tasks/done.md` (2026-11-15 to 11-17); skip any line that a
  later change already fixed:
  - `ui/rules.rs`: `start_edit_rule`'s doc comment still describes the
    pre-3.48 one-line editor with a Category picker.
  - `ui/categories.rs`: the sentence about the "context-less binding"
    behind Escape, and the "no text-measurement path" parenthetical
    (there is one since 3.86).
  - `ui/dashboard/mod.rs`: the `#[allow(unused_imports)]` comments say
    every `load` re-export is test-used; only some are. Say which.
  - `ui/dashboard/table.rs` and `ui/dashboard/view.rs`: stale layout doc
    comments from before 3.86.
  - `ui/tests/rules.rs`: the "Delete…" wording in doc comments (the
    button has been "Delete" since 3.43).
  - `ui/tests/categories*`: `categories_heading_shows_uses_tip` checks
    that the tip paints; also assert its exact text.
  - `note_painted` / `note_tree_painted` (H4 note from 3.91): same
    pass-width edge case, fix both together.
  - `STACKED_BAR_INSET_H` / `STACKED_LEGEND_EXTRA_W`: derive the `2.`
    and `8.` literals from the gap helpers; the stacked-bars test's
    literal `18.` full height comes from the constant.
  - `scripts/bundle-macos.sh`: confirm the Cargo.toml version fallback
    is reachable when `git` is missing (add `|| true` if `set -o
    pipefail` would abort first).

## Last in M3

- [ ] **3.26 A scrollbar on everything that scrolls.** Most scrolling
  areas give no sign that there is more content or where you are in it.
  Every scrollable region shows a vertical scrollbar whenever its content
  is taller than its viewport, and none when it fits. Do this last in
  M3: 3.45, 3.52, 3.58 and 3.59 added or re-laid out scrolling regions, and 3.93 / 3.99 change the dashboard panels.
  - **Review update (2026-11-11):** After 3.86 (measured layout), since the 16 px scrollbar gutter would break window-width arithmetic. Give `DataList` (3.89) a scrollbar so every list gets one. The dashboard no longer has a page scroll: 3.100 already gives each dock panel its own scrollbar, so here just check those follow the app-wide mode.
  - **Find every region at the time:** grep the UI code for
    `overflow_y_scroll`, `overflow_x_scroll` and `v_virtual_list`. Besides
    the lists below, that includes the Rules page list (`rules_scroll`),
    the dashboard dock panels (3.100, already done there), the rule matches panel (3.52), the add-rule preview (3.103) and
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
