# M3 — feature lanes and the last M3 task

Start these only after both queues are done (`docs/tasks.md` → *Picking the next task*). Within a lane, top to bottom; one task from each of two lanes at a time. UI tasks: read `docs/tasks/gpui.md` and `docs/ui-conventions.md` first.

## Lane A — dashboard (after both queues)

- [ ] **3.45 Chart and summary on their own rows.** The middle row puts
  the chart panel (`CHART_PANEL_SHARE`, 45 %) and the monthly summary side
  by side at 340 px tall; both are cramped (user request, 2026-11-11;
  changes D1's "chart beside the summary"). The user chose **a scrolling
  dashboard** over tabs or shorter panels:
  - **Review update (2026-11-11):** On `DashboardPage` (3.82), after 3.86 (measured layout): widths come from measured bounds, not window-width arithmetic.
  - **Review update (2026-11-11):** Avoid nested vertical scrollers: the summary panel takes its content height (it keeps only its horizontal month scroll) so the page scroll owns vertical movement, and the table's height is the viewport minus the pinned filter row, not a fixed 480 px. These replace the 340 px summary and ~480 px table bullets below.
  - **Layout, top to bottom:** the filter row (3.35) stays **pinned**
    above the scroll area. Below it, one vertical scroll container
    (`id` + `overflow_y_scroll` + a `ScrollHandle` kept on the shell)
    holds the **chart panel, full width, 340 px**; then the **summary
    panel, full width, 340 px**; then the **transactions table, full
    width, fixed ~480 px**, scrolling inside itself. `gap_3` between
    them, page padding as now. Nothing in the scroll area is `flex_1`
    in height; a scroll container gives children no height to fill (see
    `docs/tasks/gpui.md` → *GPUI pitfalls*), so every panel has an explicit height.
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
  - **Review update (2026-11-11):** Its backend half (the shared WHERE builder and one aggregate query) is now 3.88; build on it. The filter state is 3.56's `DashboardFilter`, so Clear all is `= default` and "shown" is `!= default`.
  - **Review update (2026-11-11):** Filter chips move under the pinned filter row, next to Clear all.
  - **Review update (2026-11-11):** Escape is the `DashboardPage`'s `Cancel` action (3.82), not a global handler.
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

- [ ] **3.93 Monthly stacked bar chart.** A new chart panel showing
  the *Monthly summary* as a picture: one bar per month, split into
  category segments, so the user can see how each month's spending
  divides up (user request, 2026-11-12). It comes after 3.46 so it
  joins the finished global filter model. It needs no new data or query.
  - **Data:** `MonthlySummary`, which is already loaded
    (`self.dashboard.summary`). Months are the columns (`months`). The
    segments are the parent spend rows: one per parent category, plus
    `Uncategorized`, plus `Transfers` when transfers are included. Leave
    out the `Income`, `Spend` and `Net` rows and all child rows. Show
    positive spend, like the other charts (ui-conventions → *Money*). A
    month's bar height is that month's `Spend` row, so the bar's total
    matches the summary. When the chart is drilled into a parent (3.5),
    the segments become that parent's children and the bar height
    becomes the parent's row. Put the GPUI-free shaping in a function
    `monthly_stacks(&MonthlySummary, drill: Option<i64>) ->
    Vec<MonthStack { month, total, segments: Vec<(Option<i64>, name,
    cents)> }>` with unit tests.
  - **Drawing:** gpui-kit 0.9's `BarChart` is single-series and can't
    stack, so build a small `StackedBarChart` in `widgets.rs` from plain
    divs, not a canvas. Each month is a column whose height is
    `total / max_total` of the plot height. Each segment is a div sized
    by its share of that month and filled with its category color from
    `category_colors` (Uncategorized is `muted_foreground`). Keep the
    segment order the same in every bar (largest total over the range at
    the bottom, matching the summary's row order) so colors line up from
    month to month. Show the month label under each bar (`Sep 2026`,
    thinned with `band_label_budget` when the bars are narrow), a few
    value gridlines with compact amounts on the left axis, and no labels
    inside segments. Zero-spend months keep an empty slot. Name the chart
    after its gpui-kit siblings so it can be upstreamed later.
  - **Panel:** *Monthly spending by category*, full width, 340 px. It
    sits in the scrolling dashboard (3.45) between the *Spending by
    category* chart and the *Monthly summary*, directly above the table
    it pictures. Below it goes the same chip legend as 3.36 (two lines
    at most, "+N more"). Empty, loading and error states follow
    ui-conventions → *States* for Chart / summary.
  - **Interaction** (same model as a summary cell): hovering a segment
    shows a tooltip `Shopping · Oct 2026` over `$412.30 · 22.4%` (share
    of that month). Clicking a segment selects that category and month,
    exactly like `click_summary_cell`. Clicking the month label selects
    the month alone (3.46's month-heading behaviour). Clicking a legend
    chip selects the category. Selection shows the way 3.46 shows it on
    the bar chart: unselected segments dim to about 35 % and the selected
    month's column gets a `list_hover` tint. Every dashboard filter
    narrows this chart too, because it reads the same summary.
  - **Many months:** with more months than fit at a 12 px minimum bar
    width, the plot scrolls sideways and starts at the latest month,
    like the summary's month scroll (a separate handle is fine).
  - Tests: `monthly_stacks` unit tests (segments sum to the `Spend` row
    for each month; Income, Net and child rows are excluded; drilling
    swaps in children; Transfers appears only when included; the order
    is stable across months). UI tests in `ui/tests/dashboard*`: the
    panel paints between the chart and the summary at full width; with
    3 months of seed data there are 3 bars, and the tallest month's bar
    fills the plot height; clicking a segment sets the category and
    month selection and filters the table; clicking a month label
    selects the month alone; a selected category dims the other
    segments; an empty range shows "No spending in this range". Add the
    panel to the *Dashboard* row of `ui-conventions.md` and add a
    `StackedBarChart` entry under *Components*.

## Lane B — Categories and Rules pages (after both queues)

- [x] **3.58 A cleaner Rules page with totals.** The Rules page (3.28)
  "is very very ugly". The user also wants each rule's **total amount**
  next to its count, and the page must stay virtualized and responsive
  (user request, 2026-11-11). Today it has 24 px rows on the page
  background with no panel, a filter row of mixed controls, plain group
  header rows, and a Move bar that pushes the list down. Keep every
  behaviour of 3.28 / 3.29 (search, Test a description, Type, Unused,
  grouping and collapse, sort, checkboxes and Move, Show rule landing);
  change the look and add the totals:
  - **Review update (2026-11-11):** Split: the DB part (`rule_use_stats`, hidden rows excluded) is now 3.87; this task is the UI on `RulesPage` (3.80), built from 3.89's `PageHeader`, `FilterRow`, `DataList` (40 px rows, 36 px group headers on `muted`) and `row_action_button`. Ignore the bullets below that restate those components' geometry; follow the components.
  - **Review update (2026-11-11):** While its add form is open, the header's New rule shows outline (one primary per page).
  - **Review update (2026-11-11):** Cached `CompiledRules` and labels come from `Catalog` (3.85); group with a map, not a linear `find`.
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

- [ ] **3.52 A rule's transactions under the rule list.** Nice-to-have
  (user request, 2026-11-11): clicking a rule shows, in its own scrollable
  area, the transactions that rule matches. Do it after 3.51 (which frees
  the vertical space) and after 3.28 (whose row checkboxes are the
  multi-select; this is a separate single **focus**).
  - **Review update (2026-11-11):** The matches panel is a 3.89 `DataList`; its loader is a keyed read job (`rule_matches:<id>`, 3.77) instead of a hand-kept `Task`; the decides set agrees with 3.87's `rule_use_stats`.
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

## Lane C — small tasks (after both queues)

- [ ] **3.59 Searchable category pickers.** From `plan.md` → *Make
  category dropdowns usable with thousands of categories* (never made a
  task; user confirmed 2026-11-11). A flat scrolling list doesn't work at
  that size, and same-named subcategories under different categories
  can't be told apart.
  - **Review update (2026-11-11):** `Catalog` (3.78) builds the grouped `SelectGroup` items once per category change; every picker shares them. 3.83 already prunes the queue's pickers.
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

- [ ] **3.95 Edit the rule while categorizing.** On To categorize, the
  Categorize button always adds a substring rule equal to the merchant
  name (`QueuePage::categorize_merchant` → `queue::categorize_group` →
  `add_rule(merchant, false, …)`). The user wants to write the rule
  there, e.g. a shorter pattern that also catches the merchant's other
  spellings (user request, 2026-11-15).
  - **Backend:** `categorize_group(conn, merchant, pattern: Option<&str>,
    is_regex: bool, category_id)`. A blank or whitespace-only pattern
    means the merchant name as a substring rule (today's behaviour),
    whatever `is_regex` says. Otherwise trim it and add it through
    `add_rule(pattern, is_regex, …)`: substring rules match the
    description case-insensitively, regex rules the way the Rules page's
    do, and an invalid regex fails with `add_rule`'s existing error
    (user follow-up, 2026-11-15: regex allowed here too). Refuse a pattern
    that matches none of the group's transactions (`CompiledPattern` over
    their `description_raw`) with "That rule doesn't match <merchant>"
    and add nothing, so a typo can't leave the row stuck in the queue
    while adding a rule.
  - **UI:** each row gets a small `Input` before the category picker,
    placeholder = the row's merchant name (so blank visibly means "use
    the merchant"), `accessibility_label` "Rule pattern", followed by a
    small regex `Switch` (same control and label as the Rules page's
    add-rule row; off by default). Both are created lazily with the
    row's picker and pruned with it (3.83). Enter in the input
    acts like the row's Categorize button when its picker has a category.
    The button stays disabled until a category is picked; the pattern
    never disables it. An error shows as the error toast; the input keeps
    its text. On success the toast says "Categorized <merchant>"
    (unchanged), plus "· rule: <pattern>" when it differs from the
    merchant.
  - **Layout:** the row must still fit 1200 × 720 with long merchant
    names: the merchant column ellipsizes first; give the input and
    switch a fixed column width shared with the heading row (the
    `QUEUE_*` column constants), heading "Rule". If space is short, the
    switch may be icon-only with a "Regex" tooltip. If it can't fit beside the picker and
    button at the minimum width, stop and ask rather than wrapping.
  - Tests: DB — blank and whitespace patterns add the merchant-name rule;
    a shorter pattern adds that pattern and categorizes every queue row it
    matches (including other merchants' rows); a non-matching pattern
    errors and adds no rule; a regex pattern adds a regex rule and
    categorizes the rows it matches; an invalid regex errors and adds
    nothing; a blank pattern with the switch on still adds the
    merchant-name substring rule. UI — the input's placeholder is the merchant
    name; typing a pattern, picking a category and clicking Categorize
    adds a rule with that pattern (visible on the Rules page); the same
    with the switch on adds a regex rule (shown as regex there); Enter in
    the input does the same; a non-matching pattern shows the error and
    the row stays; the row's columns line up with the heading (bounds).
    Update the *To categorize* paragraph of `ui-conventions.md` and the
    *To categorize queue* line of `plan.md`.

- [ ] **3.26 A scrollbar on everything that scrolls.** Most scrolling
  areas give no sign that there is more content or where you are in it.
  Every scrollable region shows a vertical scrollbar whenever its content
  is taller than its viewport, and none when it fits. Do this last in
  M3: 3.45, 3.52, 3.58 and 3.59 add or re-lay out scrolling regions.
  - **Review update (2026-11-11):** After 3.86 (measured layout), since the 16 px scrollbar gutter would break window-width arithmetic. Add the dashboard page scroll (3.45) and `DataList` (3.89, give the component a scrollbar so every list gets one) to the regions.
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
