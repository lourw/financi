//! The page's view entity (3.97): the state lives in [`state`]; this
//! entity renders it. It owns the panel entities (3.97) and subscribes
//! to the state's change events so a change repaints the page.

use super::panels::DashboardPanels;
use super::*;

pub(crate) struct DashboardPage {
    /// The shared state (3.97): everything the page and the panels
    /// paint from, including the `PageEntity` lifecycle fields.
    pub(in crate::ui) state: Entity<DashboardState>,
    /// The four panel entities (3.97), mounted by the page.
    panels: DashboardPanels,
    /// The state subscription; lives as long as the page.
    _subs: Vec<Subscription>,
}

impl DashboardPage {
    pub(in crate::ui) fn new(state: Entity<DashboardState>, cx: &mut Context<Self>) -> Self {
        // 3.97: each panel is its own entity, subscribed to the changes
        // it paints from; the page itself repaints only for the filter
        // row, the chips and the count (a `MEASURED` event moves the
        // panels, not the page).
        const PAGE_MASK: DashboardChanges = DashboardChanges::DATA
            .union(DashboardChanges::FILTER)
            .union(DashboardChanges::SELECTION);
        let panels = DashboardPanels::new(&state, cx);
        let state_sub = cx.subscribe(&state, |_, _, ev: &DashboardEvent, cx| {
            if PAGE_MASK.intersects(ev.0) {
                cx.notify();
            }
        });
        Self {
            state,
            panels,
            _subs: vec![state_sub],
        }
    }
}

impl Render for DashboardPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state.update(cx, |state, cx| {
            state.render_dashboard(&self.panels, window, cx)
        })
    }
}
