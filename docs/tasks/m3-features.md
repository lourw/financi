# M3 — the last feature tasks

The architecture and backend queues are done. What's left of M3, one task
from each of two lanes at a time, top to bottom within a lane
(`docs/tasks.md` → *Picking the next task*). UI tasks: read
`docs/tasks/gpui.md` and `docs/ui-conventions.md` first.

## Lane A — dashboard layout, labels and the pie chart

Order: 3.98 → 3.97 → 3.100 → 3.101 → 3.99. The dashboard becomes a dock
of resizable, rearrangeable panels (user decision, 2026-11-17: dock
splits, not a scrolling grid).

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

- [x] **3.103 A tidier add-rule form with a match preview.** On the
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

## Lane C — small tasks

Order: 3.105 → 3.95 → 3.104 (3.105 and 3.95 both change `ui/queue.rs`).

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
