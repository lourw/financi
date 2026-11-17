# M3 — feature lanes and the last M3 task

Start these only after both queues are done (`docs/tasks.md` → *Picking the next task*). Within a lane, top to bottom; one task from each of two lanes at a time. UI tasks: read `docs/tasks/gpui.md` and `docs/ui-conventions.md` first.

## Lane A — dashboard (after both queues)

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
