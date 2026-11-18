# M3 — the last feature tasks

The architecture and backend queues are done. What's left of M3, one task
from each of two lanes at a time, top to bottom within a lane
(`docs/tasks.md` → *Picking the next task*). UI tasks: read
`docs/tasks/gpui.md` and `docs/ui-conventions.md` first.

## Lane A — dashboard layout, labels and the pie chart

Order: 3.98 → 3.97 → 3.100 → 3.101 → 3.99. The dashboard becomes a dock
of resizable, rearrangeable panels (user decision, 2026-11-17: dock
splits, not a scrolling grid).

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

## Lane C — small tasks

Order: 3.105 → 3.95 → 3.104 (3.105 and 3.95 both change `ui/queue.rs`).

## Housekeeping

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
