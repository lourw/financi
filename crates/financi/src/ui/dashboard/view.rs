//! The chart and summary panel view types and layout helpers
//! (3.5, 3.6, 3.25, 3.31).

use super::*;

// ---------------------------------------------------------------------------
// Import history table (Imports page)

/// The one dashboard category selection (3.5): a legend row — a real
/// category (parent or child) or the `Uncategorized` pseudo-row — that
/// narrows the table. `Uncategorized` rides the table's existing
/// uncategorized-only clause; a category id matches that category and,
/// for a parent, its children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LegendSelection {
    Category(i64),
    Uncategorized,
}

/// Which of the dashboard's three empty states applies (task 3.10,
/// ui-conventions *States*): the whole-window drop-a-CSV prompt is only for
/// an empty database; with data present, the chart/summary and the table
/// carry their own empty states instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DashboardBody {
    /// No transactions at all: the whole-window import prompt.
    NoData,
    /// Transactions exist but the range/filters select none: the table
    /// shows "No transactions match these filters" with a "Clear filters"
    /// link, and the chart/summary say "No spending in this range".
    NoMatches,
    /// Rows to show.
    Rows,
}

/// A `spend_by_category` row prepared for drawing: the resolved palette
/// color and the fields `BarChart` and the legend render. `name` is the
/// full category name (legend, tooltip); `short` is the fitted band-axis
/// label (3.15), produced by [`short_chart_labels`]; `bar_label` is the
/// share drawn above the bar (3.31), empty when the bar is thinned out
/// of the label plan by [`bar_label_plan`].
#[derive(Clone)]
pub(crate) struct ChartRow {
    pub(in crate::ui) id: Option<i64>,
    pub(in crate::ui) name: SharedString,
    pub(in crate::ui) short: SharedString,
    pub(in crate::ui) bar_label: SharedString,
    pub(in crate::ui) cents: i64,
    pub(in crate::ui) share: f64,
    pub(in crate::ui) color: Hsla,
}

/// The legend column inside the chart panel (3.31): just swatch and name,
/// so it no longer carries the amount and share columns and the plot gets
/// the width back.
pub(crate) const LEGEND_W: f32 = 140.;
/// The dashboard table's fixed columns (3.25): the `Date` and `Amount`
/// widths, which never change with the window.
pub(crate) const TX_FIXED_W: f32 = 110. + 120.;
/// The dashboard table's `Account`, `Description` and `Category` widths for
/// `page_w` pixels of page (3.25, the same shape as `history_flex_widths`):
/// what the fixed columns leave over, split in the widths' original
/// proportions, with the flexed total floored at three times
/// `HISTORY_FLEX_MIN_W`. The sum always reaches the page's right edge, so
/// the table never overhangs it.
pub(crate) fn tx_flex_widths(page_w: f32) -> (f32, f32, f32) {
    let flex = (page_w - TX_FIXED_W - TABLE_BORDER_X).max(3. * HISTORY_FLEX_MIN_W);
    const TOTAL: f32 = 300. + 360. + 260.;
    (
        flex * 300. / TOTAL,
        flex * 360. / TOTAL,
        flex * 260. / TOTAL,
    )
}

// -- Bar share labels (3.31) --------------------------------------------
//
// Each bar carries its share of the spend above it (`18%`), drawn by
// `BarChart::label` in gpui-kit's 10px chart font, centred on the bar.
// `gpui-component` reserves a line of text past the tallest bar for
// them, but offers no thinning of its own, so the app skips labels the
// way band labels are thinned (3.15): a whole number of bands per drawn
// label, spread evenly first-to-last, so centred labels cannot collide.

/// What a row of the monthly summary grid (3.6) is, resolved against the
/// category tree: a spend parent (with its expand/collapse state), one of
/// its children, the `Uncategorized` pseudo-row, or one of the three
/// fixed total rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SummaryRowKind {
    Parent { expandable: bool, expanded: bool },
    Child,
    Uncategorized,
    Income,
    Spend,
    Net,
}

/// A summary row prepared for drawing: the grid values in month order with
/// the `Total` last. 3.85: the cells arrive pre-formatted and the row
/// carries its element-id key, so the grid's render path formats nothing
/// and allocates no ids per frame.
#[derive(Clone, Debug)]
pub(crate) struct SummaryRowView {
    pub(in crate::ui) id: Option<i64>,
    pub(in crate::ui) name: SharedString,
    pub(in crate::ui) kind: SummaryRowKind,
    /// Raw cents, months then `Total`: what the sign-dependent colours
    /// (the `Net` row's danger red) read.
    pub(in crate::ui) cents: Vec<i64>,
    /// `cents` through `flow::money` — the drawn text.
    pub(in crate::ui) cells: Vec<SharedString>,
    /// Element-id key: the category id, or `income` / `spend` / `net` /
    /// `uncat` for the fixed rows. 3.85: ids follow the category, not the
    /// row's position, so a collapsed sibling cannot collide.
    pub(in crate::ui) key: SharedString,
}
