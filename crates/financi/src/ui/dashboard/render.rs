//! The page's `Render` impl (3.35, 3.82).

use super::*;
use crate::ui::dashboard_filters::{
    FILTER_DATE_MIN_W, FILTER_DATE_RANGE_W, FILTER_DATE_W, FILTER_SEARCH_MIN_W,
};
use crate::ui::page::Cancel;

impl Render for DashboardPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                                .w(FILTER_DATE_RANGE_W)
                                .child(Select::new(&self.date_preset_select)),
                        )
                        .flex_none(),
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
                    // The count ends the row at the page's right edge.
                    .child(
                        div()
                            .debug_selector(|| "filters:count".into())
                            .flex_none()
                            .ml_auto()
                            .whitespace_nowrap()
                            .text_sm()
                            .text_color(muted)
                            .child(format!(
                                "{} transactions",
                                flow::count(self.row_count as i64)
                            )),
                    ),
            )
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
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .flex_1()
                        .min_h_0()
                        .gap_3()
                        // Middle row: chart + legend left (3.5), the monthly
                        // summary right (3.6), both inside `chart_panel`.
                        .child(self.chart_panel(cx))
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex_1()
                                .min_h_0()
                                .debug_selector(|| "panel:table".into())
                                .child(if body == DashboardBody::NoMatches {
                                    self.no_matches_panel(cx)
                                } else {
                                    DataTable::new(&self.table).stripe(false).into_any_element()
                                }),
                        )
                        .into_any_element()
                }
            })
            .into_any_element()
    }
}
