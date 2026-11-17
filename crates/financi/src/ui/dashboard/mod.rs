//! The Dashboard page: filters, chart, summary, the transactions table
//! and its row menu (3.62).
//!
//! The page lives in these sub-modules (H3, a pure split along the seams
//! the 3.82 extraction left): [`entity`] (state, construction, lifecycle),
//! [`load`] (the filter state and the one keyed load), [`actions`] (the
//! filter controls and row actions), [`render`] (the `Render` impl),
//! [`table`] (the table delegate and row menu) and [`view`] (the chart and
//! summary view types). The `use` below keeps every `dashboard::` path
//! resolving as before; chart/summary rendering and the filter popovers
//! live in the sibling modules [`crate::ui::dashboard_chart`] and
//! [`crate::ui::dashboard_filters`].

use super::*;

mod actions;
mod entity;
mod load;
mod render;
mod table;
mod view;

// The `allow` covers the paths the UI tests reach through `ui` (the row
// menu, the job body, the layout constants): the bin build never names
// them, but every `dashboard::` path stays resolvable (H3).
pub(super) use self::entity::DashboardPage;
#[allow(unused_imports)] // used by tests via `ui::…`
pub(super) use self::load::{DashboardData, DashboardFilter, load_dashboard};
#[allow(unused_imports)] // used by tests via `ui::…`
pub use self::table::{RowMenuItem, RowShowRule, TxTable, row_menu_items, row_show_rule};
pub(super) use self::table::{ShowRuleFn, UnpairFn};
#[allow(unused_imports)] // used by tests via `ui::…`
pub(super) use self::view::{
    ChartRow, DashboardBody, LegendSelection, SummaryRowKind, SummaryRowView, TX_FIXED_W,
    tx_flex_widths,
};
