//! The dashboard's panel entities (3.97): each panel of the page is its
//! own entity so it can subscribe to just the [`DashboardChanges`] it
//! paints from, keep its own measured-bounds probes and debug selectors
//! (`panel:chart`, `panel:stacked`, `panel:summary`, `panel:table`), and
//! — the point of the split — mount as a `PanelView` in the 3.100 dock.
//! The render methods stay on [`DashboardState`]: the panel entities own
//! a handle and a subscription, nothing else, so there is no entity
//! cycle (the page owns the panels; the panels own the state).

use super::*;

pub(in crate::ui) mod chart;
pub(in crate::ui) mod stacked;
mod summary;
mod table;

pub(super) use self::{
    chart::ChartPanel, stacked::StackedPanel, summary::SummaryPanel, table::TablePanel,
};

/// The page's four panel entities (3.97), built once beside the state
/// and mounted by [`DashboardState::render_dashboard`].
pub(super) struct DashboardPanels {
    pub(super) chart: Entity<ChartPanel>,
    pub(super) stacked: Entity<StackedPanel>,
    pub(super) summary: Entity<SummaryPanel>,
    pub(super) table: Entity<TablePanel>,
}

impl DashboardPanels {
    pub(super) fn new(state: &Entity<DashboardState>, cx: &mut App) -> Self {
        Self {
            chart: cx.new(|cx| ChartPanel::new(state.clone(), cx)),
            stacked: cx.new(|cx| StackedPanel::new(state.clone(), cx)),
            summary: cx.new(|cx| SummaryPanel::new(state.clone(), cx)),
            table: cx.new(|cx| TablePanel::new(state.clone(), cx)),
        }
    }
}
