//! The summary grid panel (3.6, its own entity since 3.97): the
//! category × month grid, its row views and cell/month/expand handlers,
//! and the `SummaryPanel` entity. Split out of
//! `ui/dashboard_chart.rs`; the render methods stay on
//! [`DashboardState`] (see [`super::chart`]'s header).

use super::*;
use crate::ui::widgets::*;
use gpui_kit::base::dock::{Panel, PanelEvent};

impl DashboardState {
    /// Click a monthly summary cell (3.6): narrow the table to that
    /// category and month; clicking the same cell again clears both, the
    /// legend toggle. The `Total` column is the category with no month.
    /// The category part drives the shared `legend_selection`, and the
    /// chart moves so the selection stays one of the legend's rows —
    /// legend and summary can never disagree about what is selected.
    pub(in crate::ui) fn click_summary_cell(
        &mut self,
        id: Option<i64>,
        month: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same_category = match id {
            None => matches!(
                self.filter.legend_selection,
                Some(LegendSelection::Uncategorized)
            ),
            Some(id) => {
                matches!(self.filter.legend_selection, Some(LegendSelection::Category(c)) if c == id)
            }
        };
        if same_category && self.filter.summary_month == month {
            self.filter.legend_selection = None;
            self.filter.summary_month = None;
        } else {
            self.filter.legend_selection = Some(match id {
                None => LegendSelection::Uncategorized,
                Some(id) => LegendSelection::Category(id),
            });
            self.filter.summary_month = month;
            if id.is_some() {
                // A real category and the uncategorized checkbox can't
                // both narrow the table (the legend's rule).
                self.filter.uncategorized_only = false;
            }
            self.sync_chart_to_selection();
        }
        // 3.65: the cell and legend highlights paint with this click.
        self.changed(DashboardChanges::SELECTION, cx);
        self.reload(cx);
    }
    /// Click a summary month heading (3.46): that month alone is selected
    /// — the chart narrows to it, the summary tints its column, the table
    /// filters to it. No category is involved; clicking the selected
    /// heading again clears it.
    pub(in crate::ui) fn click_summary_month(&mut self, month: &str, cx: &mut Context<Self>) {
        self.filter.summary_month = if self.filter.summary_month.as_deref() == Some(month) {
            None
        } else {
            Some(month.to_string())
        };
        self.changed(DashboardChanges::SELECTION, cx);
        self.reload(cx);
    }
    /// Click a summary parent's name or chevron: show or hide its child
    /// rows (3.6). Pure view state, so no re-query — just a repaint.
    pub(in crate::ui) fn toggle_summary_parent(&mut self, id: i64) {
        if !self.expanded_parents.insert(id) {
            self.expanded_parents.remove(&id);
        }
        // 3.85: the expanded set picks which child rows are in the grid.
        self.summary_views_dirty = true;
    }
    /// 3.85: the grid's rows from the cache, rebuilt on the first render
    /// after a summary load, a parent toggle or a category change — the
    /// only inputs `summary_rows` reads.
    pub(in crate::ui) fn summary_views_mut(&mut self) -> Rc<Vec<SummaryRowView>> {
        if self.summary_views_dirty {
            #[cfg(test)]
            self.summary_views_built
                .set(self.summary_views_built.get() + 1);
            self.summary_views = Rc::new(self.summary_rows());
            self.summary_views_dirty = false;
        }
        Rc::clone(&self.summary_views)
    }
    /// The summary rows prepared for drawing: the grid's rows with their
    /// kinds resolved (children of collapsed parents left out) and the
    /// month values followed by the `Total`. The query's last three rows
    /// are always `Income`, `Spend` and `Net`.
    pub(in crate::ui) fn summary_rows(&self) -> Vec<SummaryRowView> {
        let n = self.summary.rows.len();
        let mut views = Vec::with_capacity(n);
        for (ix, row) in self.summary.rows.iter().enumerate() {
            let kind = if ix + 3 >= n {
                match n - ix {
                    3 => SummaryRowKind::Income,
                    2 => SummaryRowKind::Spend,
                    _ => SummaryRowKind::Net,
                }
            } else {
                match row.id {
                    None => SummaryRowKind::Uncategorized,
                    Some(id) => match self.parent_of(id) {
                        Some(parent) => {
                            if !self.expanded_parents.contains(&parent) {
                                continue;
                            }
                            SummaryRowKind::Child
                        }
                        None => SummaryRowKind::Parent {
                            expandable: self
                                .categories
                                .iter()
                                .any(|p| p.id == id && !p.children.is_empty()),
                            expanded: self.expanded_parents.contains(&id),
                        },
                    },
                }
            };
            let cents: Vec<i64> = row
                .months
                .iter()
                .copied()
                .chain(std::iter::once(row.total))
                .collect();
            let cells = cents
                .iter()
                .map(|&c| SharedString::from(flow::money(c)))
                .collect();
            let key: SharedString = match kind {
                SummaryRowKind::Income => "income".into(),
                SummaryRowKind::Spend => "spend".into(),
                SummaryRowKind::Net => "net".into(),
                SummaryRowKind::Uncategorized => "uncat".into(),
                _ => row
                    .id
                    .map(|id| SharedString::from(id.to_string()))
                    .unwrap_or_else(|| SharedString::from(format!("row-{ix}"))),
            };
            views.push(SummaryRowView {
                id: row.id,
                name: row.name.clone().into(),
                kind,
                cents,
                cells,
                key,
            });
        }
        // Uncategorized is always the last category row, whatever its
        // amount (ui-conventions, *Numbers, dates and labels*), like the
        // legend; the three total rows stay after it.
        let categories_end = views.len().saturating_sub(3);
        views[..categories_end].sort_by_key(|v| v.kind == SummaryRowKind::Uncategorized);
        views
    }
    /// The monthly summary panel (3.6, 3.45): its own full-width row under
    /// the chart, growing to its content height (the page scrolls it).
    /// Rows are the spend parents — clicking a name or chevron
    /// expands or collapses its children — then `Income`, `Spend` and
    /// `Net`; columns are the months in the range plus the `Total`.
    /// Clicking a category cell filters the table to that category and
    /// month, and clicking the same cell again clears it (ui-conventions,
    /// *Interaction*). Amounts are positive spend; `Net` is the one signed
    /// row, `danger` when negative; `Income` reads as money in (`success`).
    pub(in crate::ui) fn summary_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // 3.85: palette indices from the Catalog (see `chart_panel`).
        let (border, muted, list_active, list_hover, danger, success, radius, colors, palette) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.muted_foreground,
                theme.list_active,
                theme.list_hover,
                theme.danger,
                theme.success,
                theme.radius,
                self.catalog.read(cx).cat_colors(),
                category_palette(theme),
            )
        };
        // `month_grid` fills the range's months even when no transaction
        // falls in it, so a range that excludes all data yields a grid of
        // zeros rather than an empty `months`. Treat an all-zero grid as
        // empty too (task 3.10): the panel says "No spending in this range".
        if !self.summary_has_data() {
            return v_flex()
                .size_full()
                .min_w_0()
                .p_4()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("No spending in this range"),
                )
                .into_any_element();
        }
        let selection = self.filter.legend_selection;
        let selected_month = self.filter.summary_month.clone();
        // 3.46: the "Uncategorized only" checkbox and the legend's
        // Uncategorized chip light the same thing, so the summary's
        // Uncategorized row highlights for either.
        let unc_active = matches!(selection, Some(LegendSelection::Uncategorized))
            || self.filter.uncategorized_only;
        let month_values = self.summary.months.clone();
        // 3.85: the cached, pre-formatted rows (see `summary_views_mut`).
        let views = self.summary_views_mut();

        // Pinned columns and scrolling months (3.30): the name column and
        // `Total` keep fixed widths at the row edges, and the month columns
        // — 88px each, as before — live in a viewport between them that
        // scrolls horizontally. One `ScrollHandle` is tracked by every
        // month viewport, the header's included, so they always show the
        // same slice of months.
        const NAME_W: f32 = 160.;
        const MONTH_W: f32 = 88.;
        const TOTAL_W: f32 = 96.;
        let month_count = month_values.len();
        // 3.85: the viewport's element id is keyed by the row's category
        // (or kind) key, never its index, so collapsing a sibling cannot
        // hand one row's scroll state to another. The debug selector
        // stays index-based for the tests.
        let month_viewport = |id: String, selector: String, handle: &ScrollHandle| {
            div()
                .id(SharedString::from(id))
                .flex_1()
                .min_w_0()
                .overflow_x_scroll()
                // Keep a vertical wheel moving the rows: without this a
                // vertical-only delta on an x-scrollable div is remapped to
                // the horizontal axis instead of reaching `summary-rows`.
                .restrict_scroll_to_axis()
                .track_scroll(handle)
                .debug_selector(move || selector.clone())
        };
        // The strip is flush right against `Total` while every month fits —
        // today's look — and once it overflows it packs to its natural
        // width and scrolls.
        let months_strip = move || {
            div()
                .flex()
                .gap_2()
                .w_full()
                .min_w(px(
                    month_count as f32 * MONTH_W + (month_count as f32 - 1.) * 8.
                ))
                .justify_end()
        };

        let header = h_flex()
            .gap_2()
            .px_1()
            .child(div().w(px(NAME_W)).flex_none())
            .child(
                month_viewport(
                    "summary:months-header".into(),
                    "summary:months-header".into(),
                    &self.summary_scroll,
                )
                .child(months_strip().children(
                    month_values.iter().enumerate().map(|(col_ix, month)| {
                        // 3.46: the heading selects its month on its own
                        // (chart, summary column tint and table), and the
                        // selected one is tinted like the column below it.
                        let month = month.clone();
                        let is_month = selected_month.as_deref() == Some(month.as_str());
                        let label = flow::month_label(month.as_str());
                        div()
                            .id(SharedString::from(format!(
                                "summary:month-heading-{}",
                                month
                            )))
                            .w(px(MONTH_W))
                            .flex_none()
                            .rounded(px(2.))
                            .debug_selector(move || format!("summary:month-label-{col_ix}"))
                            .text_right()
                            .text_sm()
                            .text_color(muted)
                            .when(is_month, |d| d.bg(list_hover))
                            .cursor_pointer()
                            .child(label)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.click_summary_month(&month, cx);
                            }))
                    }),
                )),
            )
            .child(
                div()
                    .w(px(TOTAL_W))
                    .flex_none()
                    .text_right()
                    .text_sm()
                    .text_color(muted)
                    .child("Total"),
            );

        // Pinned row height: the row markup pins a fixed height —
        // `text_sm` (15 px, a 22.5 px line box) plus `py_1`
        // (2 × 4 px); the `Income` row adds its 1 px top border.
        const ROW_H: f32 = 30.5;
        const INCOME_ROW_H: f32 = 31.5;
        // 3.45: the rows are plain children of a `v_flex` now — the page
        // owns the vertical scroll, so the panel grows to its content
        // height instead of virtualizing rows under its own handle (the
        // cached, pre-formatted `views` still do all the per-row work).
        let strip_months = month_values.clone();
        let scroll_handle = self.summary_scroll.clone();
        let body = v_flex().children(views.iter().enumerate().map(|(ix, row)| {
            let id = row.id;
            let key = row.key.clone();
            let is_category = !matches!(
                row.kind,
                SummaryRowKind::Income | SummaryRowKind::Spend | SummaryRowKind::Net
            );
            let swatch = match row.kind {
                SummaryRowKind::Parent { .. } | SummaryRowKind::Child => Some(
                    id.and_then(|id| colors.get(&id))
                        .map(|ix| palette[*ix % 12])
                        .unwrap_or(muted),
                ),
                SummaryRowKind::Uncategorized => Some(muted),
                _ => None,
            };
            let is_income = matches!(row.kind, SummaryRowKind::Income);

            // Name cell: chevron + swatch + name. On a parent it also
            // toggles the child rows.
            let (chevron, expandable) = match row.kind {
                SummaryRowKind::Parent {
                    expandable,
                    expanded,
                } => (
                    if expandable {
                        if expanded { "▾" } else { "▸" }
                    } else {
                        ""
                    },
                    expandable,
                ),
                _ => ("", false),
            };
            let name_cell = h_flex()
                .id(SharedString::from(format!("summary-name-{key}")))
                .w(px(NAME_W))
                .flex_none()
                .debug_selector(move || format!("summary:name-{ix}"))
                .gap_2()
                .items_center()
                .when(matches!(row.kind, SummaryRowKind::Child), |c| c.pl(px(14.)))
                .when(expandable, |c| {
                    c.cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(id) = id {
                                this.toggle_summary_parent(id);
                                this.changed(DashboardChanges::SELECTION, cx);
                            }
                        }))
                })
                .child(
                    div()
                        .w(px(12.))
                        .flex_none()
                        .text_xs()
                        .text_color(muted)
                        .child(chevron),
                )
                .when_some(swatch, |c, color| {
                    c.child(div().size(px(10.)).flex_none().rounded(px(2.)).bg(color))
                })
                // The fixed name column truncates long names, so hover
                // shows the full one (3.30), same tooltip idiom as the
                // queue's merchants.
                .tooltip({
                    let full = row.name.clone();
                    move |window, cx| {
                        let tip = full.clone();
                        Tooltip::element(move |_, _| {
                            div()
                                .debug_selector({
                                    let n = tip.clone();
                                    move || format!("summary:name-tooltip:{n}")
                                })
                                .child(tip.clone())
                        })
                        .build(window, cx)
                    }
                })
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_sm()
                        .when(matches!(row.kind, SummaryRowKind::Parent { .. }), |d| {
                            d.font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(matches!(row.kind, SummaryRowKind::Uncategorized), |d| {
                            d.text_color(muted)
                        })
                        .child(row.name.clone()),
                );

            // The month cells go into the scrolling viewport; the
            // `Total` cell stays pinned right. Both keep their exact
            // behaviour — click-to-filter, the selected highlight, the
            // `Income`/`Net` colours (3.6).
            let mut month_cells: Vec<AnyElement> = Vec::with_capacity(month_count);
            let mut total_cell: AnyElement = div().w(px(TOTAL_W)).flex_none().into_any_element();
            for (col_ix, &cents) in row.cents.iter().enumerate() {
                let is_total = col_ix >= month_count;
                let cell_month = strip_months.get(col_ix).cloned();
                // 3.46: the selected month tints its whole column; the
                // selected cell (category and month together) keeps the
                // stronger `list_active`.
                let in_month_col =
                    !is_total && selected_month.is_some() && cell_month == selected_month;
                let selected = is_category
                    && cell_month == selected_month
                    && match (id, selection) {
                        (None, Some(LegendSelection::Uncategorized)) => true,
                        (Some(id), Some(LegendSelection::Category(c))) => c == id,
                        _ => false,
                    };
                let color = match row.kind {
                    SummaryRowKind::Income => Some(success),
                    SummaryRowKind::Net if cents < 0 => Some(danger),
                    _ => None,
                };
                let cell = div()
                    .id(SharedString::from(format!("summary-cell-{key}-{col_ix}")))
                    .w(if is_total { px(TOTAL_W) } else { px(MONTH_W) })
                    .flex_none()
                    .debug_selector(move || format!("summary:cell:{ix}:{col_ix}"))
                    .text_right()
                    .text_sm()
                    .rounded(px(2.))
                    .when_some(color, |c, color| c.text_color(color))
                    .when(selected, |c| c.bg(list_active))
                    .when(in_month_col && !selected, |c| c.bg(list_hover))
                    .when(is_category, |c| {
                        let month = cell_month.clone();
                        c.cursor_pointer().on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                this.click_summary_cell(id, month.clone(), window, cx);
                            },
                        ))
                    })
                    .child(row.cells[col_ix].clone())
                    .into_any_element();
                if is_total {
                    total_cell = cell;
                } else {
                    month_cells.push(cell);
                }
            }

            // 3.46: the selected category's whole row carries `list_active`
            // (the legend and summary always agree on the selection); the
            // `Uncategorized` row highlights for the checkbox too.
            let row_selected = is_category
                && match (id, selection) {
                    (None, Some(LegendSelection::Uncategorized)) => true,
                    (None, None) => unc_active,
                    (Some(id), Some(LegendSelection::Category(c))) => c == id,
                    _ => false,
                };
            // 3.46: the selector records the highlight, so a test can see
            // the selected row paint (same rule as the legend rows).
            let row_selector = format!(
                "summary:row:{ix}{}",
                if row_selected { ":active" } else { "" }
            );
            h_flex()
                .id(SharedString::from(format!("summary-row-{key}")))
                .w_full()
                .gap_2()
                .items_center()
                .debug_selector(move || row_selector.clone())
                // 3.85: the row markup pins its height so the measured
                // height never drifts (3.45 moved the vertical scroll to
                // the page, so there is no item cache to match).
                .h(px(if is_income { INCOME_ROW_H } else { ROW_H }))
                .overflow_hidden()
                .px_1()
                .py_1()
                .rounded(radius)
                .when(row_selected, |r| r.bg(list_active))
                // A hairline above `Income` separates the totals from
                // the category rows.
                .when(is_income, |r| r.border_t_1().border_color(border))
                .child(name_cell)
                .child(
                    month_viewport(
                        format!("summary:months-{key}"),
                        format!("summary:months-{ix}"),
                        &scroll_handle,
                    )
                    .child(months_strip().children(month_cells)),
                )
                .child(total_cell)
                .into_any_element()
        }));

        // 3.100: the dock owns the panel box and the title; the grid
        // scrolls inside the panel now (there is no page scroll), with an
        // always-visible vertical scrollbar and the month strip's own
        // horizontal one overlaid as before.
        v_flex()
            .size_full()
            .min_w_0()
            .p_4()
            .gap_2()
            .relative()
            .child(
                div()
                    .id("summary-vscroll")
                    .debug_selector(|| "dashboard:summary-scroll".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    // 3.30, 3.100: a horizontal swipe over the months
                    // must not be remapped onto this scroller's axis.
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.summary_rows_scroll)
                    // Month header and rows keep their tighter `gap_1`.
                    .child(v_flex().w_full().gap_1().child(header).child(body)),
            )
            // The always-visible vertical scrollbar (3.100): a thin overlay
            // at the panel's right edge, inside the padding.
            .child(
                div()
                    .id("summary-vscrollbar")
                    .debug_selector(|| "dashboard:summary-vscrollbar".into())
                    .absolute()
                    .right(px(4.))
                    .top(px(4.))
                    .bottom(px(4.))
                    .w(px(12.))
                    .child(
                        Scrollbar::vertical(&self.summary_rows_scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
            )
            // The month strip's horizontal scrollbar (3.30): a thin overlay
            // pinned to the panel's bottom edge, inset to span only the
            // month zone — the name column and `Total` stay clear of it.
            // `Always` so it shows while the strip overflows (the theme
            // default hides it until hover); it disappears by itself when
            // every month fits.
            .child(
                div()
                    .id("summary-hscrollbar")
                    .absolute()
                    .left(px(20. + NAME_W + 8.))
                    .right(px(20. + TOTAL_W + 8.))
                    .bottom(px(4.))
                    .h(px(12.))
                    .child(
                        Scrollbar::horizontal(&self.summary_scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
            )
            .into_any_element()
    }
    /// Whether the monthly summary has anything to draw. `month_grid` pads
    /// the range with zero months, so an empty range is an all-zero grid
    /// rather than an empty `months` (task 3.10).
    pub(in crate::ui) fn summary_has_data(&self) -> bool {
        !self.summary.months.is_empty()
            && self
                .summary
                .rows
                .iter()
                .any(|r| r.months.iter().any(|v| *v != 0))
    }
}

/// The monthly summary grid (3.97): a dock-shaped entity (3.100) that owns nothing but
/// its handle on [`DashboardState`] and a subscription narrowed to the
/// changes it paints from — a repaint of this panel is a `cx.notify()`
/// on this entity alone, and its paint reads the state back through
/// [`DashboardState::summary_panel`].
pub(in crate::ui::dashboard) struct SummaryPanel {
    /// The shared state this panel reads and writes selections through.
    state: Entity<DashboardState>,
    /// The state subscription; lives as long as the panel.
    _subs: Subscription,
    /// Dock focus handle (3.100: `Panel`'s `Focusable` bound).
    focus: FocusHandle,
}

impl SummaryPanel {
    pub(super) fn new(state: Entity<DashboardState>, cx: &mut Context<Self>) -> Self {
        const MASK: DashboardChanges = DashboardChanges::DATA
            .union(DashboardChanges::SELECTION)
            .union(DashboardChanges::MEASURED);
        let _subs = cx.subscribe(&state, |_, _, ev: &DashboardEvent, cx| {
            if MASK.intersects(ev.0) {
                cx.notify();
            }
        });
        Self {
            state,
            _subs,
            focus: cx.focus_handle(),
        }
    }
}

impl Render for SummaryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state.update(cx, |state, cx| state.summary_panel(cx))
    }
}

/// Dock plumbing (3.100): the focus handle and the `Panel` identity. The
/// four panels are the dock's fixed set, so none of them is closable or
/// zoomable — the grid is only ever moved and resized.
impl Focusable for SummaryPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<PanelEvent> for SummaryPanel {}

impl Panel for SummaryPanel {
    fn panel_name(&self) -> &'static str {
        "summary"
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> bool {
        false
    }
}
