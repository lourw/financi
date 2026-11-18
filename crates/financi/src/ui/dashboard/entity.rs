//! The page's view entity (3.97): the state lives in [`state`]; this
//! entity renders it. It owns the panel entities (3.97) and subscribes
//! to the state's change events so a change repaints the page.

use super::*;

pub(crate) struct DashboardPage {
    /// The shared state (3.97): everything the page and the panels
    /// paint from, including the `PageEntity` lifecycle fields.
    pub(in crate::ui) state: Entity<DashboardState>,
    /// The state subscription; lives as long as the page.
    _subs: Vec<Subscription>,
}

impl DashboardPage {
    pub(in crate::ui) fn new(state: Entity<DashboardState>, cx: &mut Context<Self>) -> Self {
        // 3.97: commit 1 paints the whole page from the state on any
        // change; the panels' selective subscriptions (and this mask's
        // narrowing) land with the panel entities.
        let state_sub = cx.subscribe(&state, |_, _, ev: &DashboardEvent, cx| {
            if ev.0.intersects(DashboardChanges::ALL) {
                cx.notify();
            }
        });
        Self {
            state,
            _subs: vec![state_sub],
        }
    }
}

impl Render for DashboardPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state
            .update(cx, |state, cx| state.render_dashboard(window, cx))
    }
}
