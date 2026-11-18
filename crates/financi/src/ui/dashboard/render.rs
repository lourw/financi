//! The page's render (3.35, 3.82): the body lives on the shared state
//! as [`DashboardState::render_dashboard`] (3.97), and the thin view
//! entity in [`entity`](super::entity) forwards to it.

use std::rc::Rc;

use super::dock::{DOCK_GUTTER, DashboardDock, clamp_min_sizes, default_layout};
use super::dock_layout::{LAYOUT_VERSION, register_dock_panels};
use super::panels::DashboardPanels;
use super::*;
use crate::ui::dashboard_filters::{
    FILTER_DATE_MIN_W, FILTER_DATE_RANGE_W, FILTER_DATE_W, FILTER_SEARCH_MIN_W,
};
use crate::ui::page::Cancel;
use gpui_kit::base::dock::{DockArea, DockEvent};

impl DashboardState {
    /// The whole page body (3.97): the filter row, the chips and the
    /// panel stack — the four [`DashboardPanels`] entities, mounted in
    /// the scroller — painted from the shared state. Called by
    /// [`DashboardPage`]'s `Render` impl; it returns `AnyElement`
    /// because the body's shape (empty state vs. scroller) varies.
    pub(in crate::ui::dashboard) fn render_dashboard(
        &mut self,
        panels: &DashboardPanels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 3.82: the page takes focus on the frame it is first shown, like
        // the other extracted pages — deferred so no element is borrowed
        // while the focus moves, and only once (D5).
        if self.focus_pending {
            self.focus_pending = false;
            let focus = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        // A load that landed without a window (3.82) marked the pickers;
        // re-sync them now that one is here.
        self.sync_date_widgets(window, cx);
        let muted = cx.theme().muted_foreground;
        // 3.46: "Clear all" shows while any filter differs from the default.
        let filtered = self.filter != DashboardFilter::default();
        let body = self.dashboard_body();
        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            // 3.82: the page is a focus root on its own key context, and
            // Esc — `Cancel` on "Dashboard" (bound in `ui::init`) — clears
            // the category selection, the behaviour the Root-scoped
            // `ClearCategorySelection` binding had before the extraction
            // (3.5). Deeper contexts still win: an open picker or menu
            // takes the Escape first.
            .track_focus(&self.focus)
            .key_context("Dashboard")
            .on_action(cx.listener(|this, action: &Cancel, window, cx| {
                this.clear_category_selection(action, window, cx);
            }))
            // One filter row (3.35, ui-conventions *Pages*; 3.91: the
            // shared `filter_row` strip): no region title and no label
            // lines. Every control is 32 px tall and
            // centred on the row; the controls without a visible label
            // name themselves in a hover tooltip. Left to right: date
            // range, the custom From/To pickers, accounts, search (takes
            // the slack), Options (the three toggles), then the count.
            .child(
                filter_row()
                    .debug_selector(|| "filters:region".into())
                    .child(
                        filter_tip(
                            "filters:date-range-tip",
                            "Date range",
                            div()
                                .debug_selector(|| "filters:date-range".into())
                                .w_full()
                                .min_w_0()
                                .child(Select::new(&self.date_preset_select)),
                        )
                        // 3.46: with Clear all shown and the custom range
                        // open the row runs out of room at 1200 px, so the
                        // preset select shrinks with the pickers (floor
                        // 120 px) instead of pushing the count off-page.
                        .flex_shrink(1.)
                        .w(FILTER_DATE_RANGE_W)
                        .min_w(px(120.)),
                    )
                    // The From/To pickers want 140 px but shrink first
                    // (floor 110 px) when the row runs out of room, so the
                    // custom range fits the minimum window (3.25, 3.35).
                    .when(
                        matches!(self.filter.date_range, DateRange::Custom { .. }),
                        |g| {
                            g.child(
                                div()
                                    .debug_selector(|| "filters:from".into())
                                    .w(FILTER_DATE_W)
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W)
                                    .child(DatePicker::new(&self.date_from_picker)),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "filters:to".into())
                                    .w(FILTER_DATE_W)
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W)
                                    .child(DatePicker::new(&self.date_to_picker)),
                            )
                        },
                    )
                    // Accounts multi-select (3.3): a dropdown popover of
                    // checkboxes, one per account, all checked by default.
                    // The selector (test builds only, 3.11) lets headless
                    // tests find the trigger and click it.
                    .child(
                        filter_tip(
                            "filters:accounts-tip",
                            "Accounts",
                            div()
                                .debug_selector(|| "accounts:filter".into())
                                .child(self.accounts_popover(cx)),
                        )
                        .flex_none(),
                    )
                    // Search (3.4) takes the row's slack; the placeholder
                    // and the leading icon say what it is.
                    .child(
                        div()
                            .debug_selector(|| "filters:search".into())
                            .flex_1()
                            .min_w(FILTER_SEARCH_MIN_W)
                            .child(
                                Input::new(&self.search_input)
                                    .prefix(Icon::new(IconName::Search).small().text_color(muted)),
                            ),
                    )
                    .child(
                        filter_tip(
                            "filters:options-tip",
                            "Options",
                            div()
                                .debug_selector(|| "filters:options".into())
                                .child(self.options_popover(cx)),
                        )
                        .flex_none(),
                    )
                    // 3.46: "Clear all" sits at the right of the filter
                    // row, just before the count, and appears only when
                    // some filter differs from the default. It resets
                    // everything — the persisted settings included.
                    .when(filtered, |g| {
                        g.child(
                            Button::new("clear-all")
                                .xsmall()
                                .ghost()
                                .icon(IconName::X)
                                .label("Clear all")
                                .debug_selector(|| "filters:clear-all".into())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.clear_all_filters(window, cx);
                                })),
                        )
                    })
                    // The count ends the row at the page's right edge;
                    // the Clear all button takes the slack when it shows.
                    .child(
                        div()
                            .debug_selector(|| "filters:count".into())
                            .flex_none()
                            .when(!filtered, |d| d.ml_auto())
                            .whitespace_nowrap()
                            .text_sm()
                            .text_color(muted)
                            .child(format!(
                                "{} transactions",
                                flow::count(self.row_count as i64)
                            )),
                    )
                    // 3.101: the layout reset — the last thing on the
                    // row's right edge, beside the count it never
                    // shifts. Puts the default 2×2 back and saves it;
                    // *View ▸ Reset dashboard layout* does the same.
                    .child(
                        div()
                            .debug_selector(|| "filters:reset-layout".into())
                            .flex_none()
                            .child(
                                row_action_button(
                                    "reset-layout",
                                    IconName::LayoutDashboard,
                                    "Reset layout",
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.reset_dock_layout(window, cx);
                                    },
                                )),
                            ),
                    ),
            )
            // 3.46: the filter chips row — each active narrowing the row
            // itself doesn't show — pinned under the filter row, above
            // the scroller. Empty (no space) when nothing is active.
            .child(self.filter_chips_row(cx))
            .child(match body {
                // 5.3: an empty database is also where a restore begins,
                // so the empty state offers it beside the import hint.
                DashboardBody::NoData => v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(muted)
                    .child("Drop an Amex or Simplii CSV export here, or press ⌘O")
                    .child(
                        Button::new("restore-from-export")
                            .link()
                            .label("Restore from export…")
                            .debug_selector(|| "dashboard:restore-link".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.restore_backup(window, cx);
                            })),
                    )
                    .into_any_element(),
                _ => {
                    // 3.100: the dock — the four panels in a resizable,
                    // movable 2×2 — replaces the page scroller. There is
                    // no page scroll: each panel scrolls its own content.
                    // The area is built on the first paint (building it
                    // needs a window); its subscription re-applies the
                    // panel minimums after every layout edit or divider
                    // drag.
                    if self.dock.is_none() {
                        let weak = cx.entity().downgrade();
                        let dock = cx.new(|cx| {
                            DockArea::new("dashboard-dock", Some(LAYOUT_VERSION), window, cx)
                                .with_renderer(Rc::new(DashboardDock::new(weak)))
                        });
                        let page = cx.entity().downgrade();
                        let sub = cx.subscribe_in(
                            &dock,
                            window,
                            move |_,
                                  area: &Entity<DockArea>,
                                  ev: &DockEvent,
                                  window: &mut Window,
                                  cx| {
                                if matches!(ev, DockEvent::LayoutChanged) {
                                    // Deferred: the clamp must not re-enter
                                    // the dock mid-edit (this event *is* the
                                    // edit settling).
                                    let area = area.clone();
                                    window.defer(cx, move |window, cx| {
                                        clamp_min_sizes(&area, window, cx);
                                    });
                                    // 3.101: the debounced layout save, on
                                    // the same deferral — the page is being
                                    // borrowed here, and it starts its timer
                                    // outside the dock's edit.
                                    let page = page.clone();
                                    window.defer(cx, move |_, cx| {
                                        let _ = page.update(cx, |page, cx| {
                                            page.schedule_layout_save(cx);
                                        });
                                    });
                                }
                            },
                        );
                        self._dock_subs.push(sub);
                        // 3.101: the panel builders a restored layout loads
                        // through, and the handles a layout reset rebuilds
                        // the default 2×2 from.
                        register_dock_panels(panels, cx);
                        self.dock_panels = Some(panels.clone());
                        dock.update(cx, |area, cx| {
                            area.set_center(default_layout(panels), window, cx)
                        });
                        self.dock = Some(dock);
                        // Read the stored layout behind the default; the
                        // save gate lifts when the read resolves.
                        self.start_layout_restore(window, cx);
                    }
                    let dock = self.dock.clone().expect("just created");
                    // No `w_full()`: with the negative side margins an
                    // auto-width stretched child grows 12 px past the page's
                    // content box, which is exactly the half-ring on each
                    // side. `w_full()` would pin the old width and lose the
                    // right-hand half.
                    div()
                        .min_w_0()
                        .flex_1()
                        .min_h_0()
                        // 12 px gutters: each group frame adds a 6 px
                        // ring, so the dock pulls itself back by half a
                        // gutter to keep the page's 16 px padding at the
                        // window edge.
                        .ml(-DOCK_GUTTER)
                        .mr(-DOCK_GUTTER)
                        .mb(-DOCK_GUTTER)
                        .child(dock)
                        .into_any_element()
                }
            })
            .into_any_element()
    }
    /// Which dashboard empty state applies (task 3.10, ui-conventions
    /// *States*). `total_visible` counts every non-hidden row ignoring all
    /// filters, so it is the "empty database" signal; `row_count` is the
    /// filtered table, so `0` with data present means the filters excluded
    /// everything.
    pub(in crate::ui) fn dashboard_body(&self) -> DashboardBody {
        if self.total_visible == 0 {
            DashboardBody::NoData
        } else if self.row_count == 0 {
            DashboardBody::NoMatches
        } else {
            DashboardBody::Rows
        }
    }
}
