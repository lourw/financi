# M3 — feature lanes and the last M3 task

Start these only after both queues are done (`docs/tasks.md` → *Picking the next task*). Within a lane, top to bottom; one task from each of two lanes at a time. UI tasks: read `docs/tasks/gpui.md` and `docs/ui-conventions.md` first.

## Lane A — dashboard (after both queues)

- [ ] **3.36 Chart legend under the chart.** The legend column beside the
  bars (`LEGEND_W`, 140 px) still takes width the plot needs. Move it
  below the chart and condense it (user request, 2026-11-11; changes
  decision D1 from "beside" to "below"):
  - **Review update (2026-11-11):** On `DashboardPage` (3.82), after 3.86: the plot width is measured, so drop the `LEGEND_W` arithmetic rather than adjusting it. Leave room in the chips for 3.46's selected / dimmed state.
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

- [ ] **3.58 A cleaner Rules page with totals.** The Rules page (3.28)
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

- [ ] **3.48 The rule editor gives the pattern room.** Clicking Edit on
  a rule swaps the row for an editor whose Pattern input shrinks so far
  that the pattern can't really be edited (user report, 2026-11-11).
  Cause: `rule_editor_row` is one `h_flex` with Pattern `flex_1` +
  `min_w_0` and, after it, the regex switch, a 160 px Category picker,
  Save and Cancel. In the detail panel (page minus the 210 px sidebar
  and the 300 px tree panel) the fixed controls take ~400 px of ~600, and
  3.53 removes the editor's picker, which frees most of that. The user also
  saw the editor's regex switch sit off the row's line (3.42's cause).
  - **Review update (2026-11-11):** Build the editor with 3.89's `InlineForm`. After 3.53 removed the picker, check whether the add row still needs two lines; if not, leave it on one.
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
  - **Review update (2026-11-11):** Implement with 3.89's `InlineForm`; the label rule it describes is now the general rule in `ui-conventions.md` → *Patterns*, so no exception note is needed.
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
  - **Review update (2026-11-11):** Forms are 3.89's `InlineForm`s; Escape closes them through `CategoriesPage`'s `Cancel` action (3.81).
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
  - **Review update (2026-11-11):** Also moves the Categories rule list onto 3.89's `DataList` with 40 px rows, using the same row renderer as the Rules page (3.81), so a rule looks the same on both pages. Uses come from 3.87's `rule_use_stats`.
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
  - **Review update (2026-11-11):** The *Copy* wording is already in `ui-conventions.md` (rewritten 2026-11-11). Rule-row Edit / Delete come from 3.89's `row_action_button`; what's left is the title-row and header buttons on the Categories page, and the guard test.
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

## Last in M3

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
