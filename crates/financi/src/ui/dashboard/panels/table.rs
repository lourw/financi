//! The transactions panel (3.45, its own entity since 3.97): the
//! `panel:table` wrapper — its height, width probe and selector — the
//! `DataTable` and its empty state, plus the viewport mirror that sizes
//! them. Split out of `ui/dashboard_chart.rs` and `dashboard/render.rs`;
//! the render methods stay on [`DashboardState`] (see [`super::chart`]'s
//! header).

use super::*;
use crate::ui::widgets::*;

impl DashboardState {
    /// Store a freshly measured table-container width (3.86, from the
    /// probe on `panel:table`) and re-flex the table's columns to it —
    /// notify-on-change only, like [`Self::set_plot_width`].
    pub(in crate::ui) fn set_table_width(&mut self, w: f32, cx: &mut Context<Self>) {
        if (self.tx_panel_w - w).abs() < 0.5 {
            return;
        }
        self.tx_panel_w = w;
        let flex = tx_flex_widths(w);
        self.table.update(cx, |t, cx| {
            t.delegate_mut().flex_widths = flex;
            t.refresh(cx);
        });
        self.changed(DashboardChanges::MEASURED, cx);
    }
    /// Store the page scroller's measured viewport height (3.45, from the
    /// `bounds_height_probe` on its wrapper): the transactions table's
    /// height, so the table fills the view below the pinned filter row and
    /// scrolls inside itself. Notify-on-change only, like
    /// [`Self::set_table_width`].
    pub(in crate::ui) fn set_viewport_height(&mut self, h: f32, cx: &mut Context<Self>) {
        if (self.tx_panel_h - h).abs() < 0.5 {
            return;
        }
        self.tx_panel_h = h;
        self.changed(DashboardChanges::MEASURED, cx);
    }
    /// The transactions panel (3.45): fills the viewport height the
    /// page's probe measured (`Self::set_viewport_height`), flexes its
    /// columns to its own measured width (`Self::set_table_width`), and
    /// holds the table or its empty state. The `panel:table` wrapper and
    /// its probe live here (3.97), so the page no longer repaints when
    /// they move.
    pub(in crate::ui) fn render_table_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .min_w_0()
            .flex_none()
            .h(px(self.tx_panel_h))
            .relative()
            .occlude()
            .debug_selector(|| "panel:table".into())
            // 3.86: the column plan comes from this panel's own
            // measured width, reported by the probe (no window-width
            // arithmetic).
            .child(bounds_probe(cx.entity().downgrade(), |this, w, cx| {
                this.set_table_width(w, cx)
            }))
            .child(if self.dashboard_body() == DashboardBody::NoMatches {
                self.no_matches_panel(cx)
            } else {
                DataTable::new(&self.table).stripe(false).into_any_element()
            })
            .into_any_element()
    }
    /// The table's empty state (task 3.10, ui-conventions *States*): shown
    /// when transactions exist but the current filters select none. A
    /// muted centered message plus a "Clear filters" link. 3.46: the link
    /// runs the same reset as the filter row's "Clear all" button, so it
    /// restores every filter, the header controls included.
    pub(in crate::ui) fn no_matches_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        panel(cx, false)
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(muted)
            .child("No transactions match these filters")
            .child(
                Button::new("clear-filters")
                    .link()
                    .label("Clear filters")
                    .debug_selector(|| "dashboard:no-matches-clear".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_all_filters(window, cx);
                    })),
            )
            .into_any_element()
    }
}

/// The transactions table (3.97): a dock-shaped entity (3.100) that owns nothing but
/// its handle on [`DashboardState`] and a subscription narrowed to the
/// changes it paints from — a repaint of this panel is a `cx.notify()`
/// on this entity alone, and its paint reads the state back through
/// [`DashboardState::render_table_panel`].
pub(in crate::ui::dashboard) struct TablePanel {
    /// The shared state this panel reads and writes selections through.
    state: Entity<DashboardState>,
    /// The state subscription; lives as long as the panel.
    _subs: Subscription,
}

impl TablePanel {
    pub(super) fn new(state: Entity<DashboardState>, cx: &mut Context<Self>) -> Self {
        const MASK: DashboardChanges = DashboardChanges::ALL;
        let _subs = cx.subscribe(&state, |_, _, ev: &DashboardEvent, cx| {
            if MASK.intersects(ev.0) {
                cx.notify();
            }
        });
        Self { state, _subs }
    }
}

impl Render for TablePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state
            .update(cx, |state, cx| state.render_table_panel(cx))
    }
}
