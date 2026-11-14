//! Chart and summary panels of the Dashboard page (split out of
//! `dashboard.rs` in 3.74): the category bar chart, its label math, and the
//! summary table with their Shell methods.

use super::*;
use crate::ui::widgets::*;

const CHART_PANEL_SHARE: f32 = 0.45;
const REGION_GAP: f32 = 12.;
/// Minimum horizontal gap between two neighbouring drawn band labels.
pub(super) const CHART_LABEL_GAP: f32 = 8.;
/// Cap on a band label, so a wide chart never paints a sentence.
pub(super) const CHART_LABEL_MAX_CHARS: usize = 16;
/// Smallest budget that makes a label worth drawing; below it labels are
/// thinned out instead of reduced to unreadable slivers.
pub(super) const CHART_LABEL_MIN_CHARS: usize = 4;
/// The chart's label plan for `chart_width` pixels across `count` bands:
/// band labels are truncated to `chars` characters (with an ellipsis)
/// and only `labels` of them are drawn, spread evenly first-to-last by
/// `BarChart::band_tick_count`. A centred label cannot collide with its
/// neighbours once every drawn label owns `chart_width / labels` pixels,
/// so that slice always buys `chars · CHART_CHAR_PX + CHART_LABEL_GAP`.
pub(super) fn band_label_budget(chart_width: f32, count: usize) -> (usize, usize) {
    if count == 0 {
        return (0, 0);
    }
    let chars_for = |labels: usize| {
        let per = chart_width / labels as f32;
        (((per - CHART_LABEL_GAP) / CHART_CHAR_PX).max(0.)) as usize
    };
    // Prefer labelling every band.
    if chars_for(count) >= CHART_LABEL_MIN_CHARS {
        return (chars_for(count).min(CHART_LABEL_MAX_CHARS), count);
    }
    // Too tight: thin the labels out until each drawn one buys the
    // minimum budget, then truncate to what its wider slice allows.
    let min_spacing = CHART_LABEL_MIN_CHARS as f32 * CHART_CHAR_PX + CHART_LABEL_GAP;
    let labels = ((chart_width / min_spacing) as usize).clamp(1, count);
    (chars_for(labels).clamp(1, CHART_LABEL_MAX_CHARS), labels)
}
/// Fit `names` into the band axis (3.15): a name longer than `chars`
/// becomes `chars − 1` characters plus `…` (trailing whitespace is
/// dropped first). Shortenings that collide fall back to the full name,
/// because each row's band text is also its unique key in `BarChart`'s
/// band scale (sibling category names are unique in the database).
/// `chars == 0` means "no budget": the full names come back untouched.
pub(super) fn short_chart_labels(names: &[String], chars: usize) -> Vec<SharedString> {
    if chars == 0 {
        return names
            .iter()
            .map(|n| SharedString::from(n.clone()))
            .collect();
    }
    let shorts: Vec<String> = names
        .iter()
        .map(|name| {
            if name.chars().count() <= chars {
                return name.clone();
            }
            let mut head: String = name.chars().take(chars - 1).collect();
            while head.ends_with(char::is_whitespace) {
                head.pop();
            }
            head.push('…');
            head
        })
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for short in &shorts {
        *counts.entry(short.as_str()).or_default() += 1;
    }
    shorts
        .iter()
        .zip(names)
        .map(|(short, name)| {
            if counts[short.as_str()] > 1 {
                SharedString::from(name.clone())
            } else {
                SharedString::from(short.clone())
            }
        })
        .collect()
}
/// Widest share label: `100%`, or `0.4%` below one percent — four
/// characters of the 10px chart font.
pub(super) const BAR_LABEL_MAX_CHARS: usize = 4;
/// A bar's share label: whole percent (`18%`), one decimal only below
/// one percent (`0.4%`) so a thin slice still reads — and never `0.0%`:
/// a sliver too small to round to a tenth of a percent reads `0.1%`.
pub(super) fn bar_share_label(share: f64) -> String {
    let percent = share * 100.;
    if percent <= 0. {
        "0%".into()
    } else if percent < 1. {
        if percent < 0.05 {
            "0.1%".into()
        } else {
            format!("{percent:.1}%")
        }
    } else {
        format!("{percent:.0}%")
    }
}
/// The tooltip body of a chart row (3.31), shared by the bar's own
/// tooltip and the legend row's: `$1,234.56 · 18.2% of Food`. `parent`
/// carries the ` of …` suffix when the chart is drilled into a category,
/// and is empty at the top level.
pub(super) fn chart_tooltip_value(cents: i64, share: f64, parent: &str) -> String {
    format!("{} · {:.1}%{parent}", flow::money(cents), share * 100.)
}
/// How many of `count` bars carry their share label across
/// `chart_width` pixels. A drawn label is at most `BAR_LABEL_MAX_CHARS`
/// characters wide and keeps `CHART_LABEL_GAP` from its neighbours, so
/// their band centres must sit
/// `BAR_LABEL_MAX_CHARS · CHART_CHAR_PX + CHART_LABEL_GAP` apart; the
/// even first-to-last spread keeps the closest pair
/// `floor((count − 1) / (labels − 1))` bands apart, which the formula
/// below inverts.
pub(super) fn bar_label_plan(chart_width: f32, count: usize) -> usize {
    if count <= 1 {
        return count;
    }
    let band_w = chart_width / count as f32;
    let need = BAR_LABEL_MAX_CHARS as f32 * CHART_CHAR_PX + CHART_LABEL_GAP;
    // Whole bands a drawn label needs beside it; at least one. A zero
    // width saturates `step`, and the division below leaves one label.
    let step = (need / band_w).ceil().max(1.) as usize;
    (count - 1) / step + 1
}
/// Which bars carry a share label: `labels` of the `count` bars, spread
/// evenly from the first to the last — the thinning
/// `BarChart::band_tick_count` applies to band labels (3.15), done here
/// because `BarChart::label` paints whatever text it is handed for
/// every bar.
pub(super) fn bar_label_slots(count: usize, labels: usize) -> Vec<bool> {
    let mut slots = vec![false; count];
    match labels {
        0 => {}
        1 if count > 0 => slots[0] = true,
        n if n >= count => slots.iter_mut().for_each(|s| *s = true),
        n => {
            for k in 0..n {
                let ix = (k as f32 * (count - 1) as f32 / (n - 1) as f32).round() as usize;
                slots[ix] = true;
            }
        }
    }
    slots
}
/// Fill each row's `bar_label` with its share of the spend (3.31): on the
/// bars the plan fits across `plot_width`, nothing on the thinned-out
/// ones. The tooltip carries the exact amount and share for every bar,
/// labelled or not.
fn bar_share_labels(rows: &mut [ChartRow], plot_width: f32) {
    let slots = bar_label_slots(rows.len(), bar_label_plan(plot_width, rows.len()));
    for (row, labeled) in rows.iter_mut().zip(slots) {
        if labeled {
            row.bar_label = bar_share_label(row.share).into();
        }
    }
}

impl Shell {
    /// Click a legend row (3.5). A parent at the top level drills the chart
    /// and legend into its children and filters the table to that parent;
    /// a child filters the table to itself, and clicking the selected child
    /// again clears the filter (the chart stays on the children). The
    /// `Uncategorized` pseudo-row toggles the uncategorized filter.
    /// Selecting a real category clears the "Uncategorized only" checkbox —
    /// the two can't both narrow the table.
    pub(super) fn click_legend_row(
        &mut self,
        id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dashboard.legend_selection = match id {
            None => (self.dashboard.legend_selection != Some(LegendSelection::Uncategorized))
                .then_some(LegendSelection::Uncategorized),
            Some(id) if self.dashboard.chart_parent.is_none() => {
                self.dashboard.chart_parent = Some(id);
                Some(LegendSelection::Category(id))
            }
            Some(id) => (self.dashboard.legend_selection != Some(LegendSelection::Category(id)))
                .then_some(LegendSelection::Category(id)),
        };
        if matches!(
            self.dashboard.legend_selection,
            Some(LegendSelection::Category(_))
        ) {
            self.dashboard.uncategorized_only = false;
        }
        // The month only ever narrows a category selection (the cell
        // highlight and the chip read both from the pair), so clearing the
        // selection clears it too — Esc does the same for both.
        if self.dashboard.legend_selection.is_none() {
            self.dashboard.summary_month = None;
        }
        // 3.65: the legend highlight paints with this click, not with the
        // answer (the rows and totals are the only part that waits).
        cx.notify();
        self.changed(Domains::TX, window, cx);
    }
    /// Click a monthly summary cell (3.6): narrow the table to that
    /// category and month; clicking the same cell again clears both, the
    /// legend toggle. The `Total` column is the category with no month.
    /// The category part drives the shared `legend_selection`, and the
    /// chart moves so the selection stays one of the legend's rows —
    /// legend and summary can never disagree about what is selected.
    pub(super) fn click_summary_cell(
        &mut self,
        id: Option<i64>,
        month: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same_category = match id {
            None => matches!(
                self.dashboard.legend_selection,
                Some(LegendSelection::Uncategorized)
            ),
            Some(id) => {
                matches!(self.dashboard.legend_selection, Some(LegendSelection::Category(c)) if c == id)
            }
        };
        if same_category && self.dashboard.summary_month == month {
            self.dashboard.legend_selection = None;
            self.dashboard.summary_month = None;
        } else {
            self.dashboard.legend_selection = Some(match id {
                None => LegendSelection::Uncategorized,
                Some(id) => LegendSelection::Category(id),
            });
            self.dashboard.summary_month = month;
            if id.is_some() {
                // A real category and the uncategorized checkbox can't
                // both narrow the table (the legend's rule).
                self.dashboard.uncategorized_only = false;
            }
            self.sync_chart_to_selection();
        }
        // 3.65: the cell and legend highlights paint with this click.
        cx.notify();
        self.changed(Domains::TX, window, cx);
    }
    /// Move the chart so a selection made outside the legend (a summary
    /// cell) is visible in the legend rows: a parent shows at the top
    /// level, a child only while drilled into its parent. Keeps the
    /// refresh revalidation of `legend_selection` from dropping a
    /// summary-made selection.
    fn sync_chart_to_selection(&mut self) {
        let Some(selection) = self.dashboard.legend_selection else {
            return;
        };
        match selection {
            LegendSelection::Uncategorized => self.dashboard.chart_parent = None,
            LegendSelection::Category(id) => {
                if let Some(parent) = self.parent_of(id) {
                    self.dashboard.chart_parent = Some(parent);
                } else if self.dashboard.chart_parent != Some(id) {
                    self.dashboard.chart_parent = None;
                }
            }
        }
    }
    /// The id of `id`'s parent in the category tree, `None` for a
    /// top-level category.
    fn parent_of(&self, id: i64) -> Option<i64> {
        self.categories
            .iter()
            .find(|p| p.children.iter().any(|c| c.id == id))
            .map(|p| p.id)
    }
    /// Click a summary parent's name or chevron: show or hide its child
    /// rows (3.6). Pure view state, so no re-query — just a repaint.
    pub(super) fn toggle_summary_parent(&mut self, id: i64) {
        if !self.dashboard.expanded_parents.insert(id) {
            self.dashboard.expanded_parents.remove(&id);
        }
    }
    /// The summary rows prepared for drawing: the grid's rows with their
    /// kinds resolved (children of collapsed parents left out) and the
    /// month values followed by the `Total`. The query's last three rows
    /// are always `Income`, `Spend` and `Net`.
    pub(super) fn summary_rows(&self) -> Vec<SummaryRowView> {
        let n = self.dashboard.summary.rows.len();
        let mut views = Vec::with_capacity(n);
        for (ix, row) in self.dashboard.summary.rows.iter().enumerate() {
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
                            if !self.dashboard.expanded_parents.contains(&parent) {
                                continue;
                            }
                            SummaryRowKind::Child
                        }
                        None => SummaryRowKind::Parent {
                            expandable: self
                                .categories
                                .iter()
                                .any(|p| p.id == id && !p.children.is_empty()),
                            expanded: self.dashboard.expanded_parents.contains(&id),
                        },
                    },
                }
            };
            let mut cells = row.months.clone();
            cells.push(row.total);
            views.push(SummaryRowView {
                id: row.id,
                name: row.name.clone().into(),
                kind,
                cells,
            });
        }
        // Uncategorized is always the last category row, whatever its
        // amount (ui-conventions, *Numbers, dates and labels*), like the
        // legend; the three total rows stay after it.
        let categories_end = views.len().saturating_sub(3);
        views[..categories_end].sort_by_key(|v| v.kind == SummaryRowKind::Uncategorized);
        views
    }
    /// The dashboard middle row (3.5): the `BarChart` of spend per category
    /// beside a clickable legend, in the left half; the monthly summary
    /// (3.6) takes the right half later. `BarChart` takes no clicks, so all
    /// interactivity lives in the legend rows (ui-conventions, *Interaction*).
    /// The palette color of every category (ui-conventions, *Category
    /// colors*): position among the siblings sorted by category id, so a
    /// category keeps its color when the date range reorders the rows;
    /// children start at the front of the palette under their own parent.
    /// Shared by the chart, its legend and the monthly summary swatches.
    pub(super) fn category_colors(
        &self,
        theme: &gpui_kit::component::theme::Theme,
    ) -> HashMap<i64, Hsla> {
        let palette = category_palette(theme);
        let mut colors = HashMap::new();
        let mut parents: Vec<&CategoryNode> = self.categories.iter().collect();
        parents.sort_by_key(|n| n.id);
        for (ix, parent) in parents.iter().enumerate() {
            colors.insert(parent.id, palette[ix % 12]);
            let mut kids: Vec<&CategoryNode> = parent.children.iter().collect();
            kids.sort_by_key(|n| n.id);
            for (cix, kid) in kids.iter().enumerate() {
                colors.insert(kid.id, palette[cix % 12]);
            }
        }
        colors
    }
    /// The plot's width for the current window: the sidebar and page
    /// padding come off the window width, `CHART_PANEL_SHARE` of what
    /// remains is the chart panel, and inside the panel its padding, the
    /// legend and the gap come off before the plot is left.
    pub(super) fn chart_plot_width(&self) -> f32 {
        let page_w = (self.window_width - SIDEBAR_W - 2. * PAGE_PAD).max(0.);
        (page_w * CHART_PANEL_SHARE - 2. * PAGE_PAD - LEGEND_W - REGION_GAP).max(0.)
    }
    /// The chart's label plan (3.15) for the current window: how many
    /// characters a band label may use and how many labels are drawn.
    pub(super) fn chart_label_plan(&self) -> (usize, usize) {
        band_label_budget(self.chart_plot_width(), self.dashboard.chart_rows.len())
    }
    /// The chart's rows prepared for drawing (3.15, 3.31): band labels
    /// fitted to the current budget, each row's share over its bar
    /// thinned to `plot_width`, and `Uncategorized` last whatever its
    /// amount. `colors` maps category ids to palette colors; `muted`
    /// paints the `Uncategorized` swatch.
    pub(super) fn chart_bars(
        &self,
        plot_width: f32,
        colors: &HashMap<i64, Hsla>,
        muted: Hsla,
    ) -> Vec<ChartRow> {
        let (label_chars, _) = self.chart_label_plan();
        let shorts = short_chart_labels(
            &self
                .dashboard
                .chart_rows
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<String>>(),
            label_chars,
        );
        let mut rows: Vec<ChartRow> = self
            .dashboard
            .chart_rows
            .iter()
            .zip(shorts)
            .map(|(r, short)| ChartRow {
                id: r.id,
                name: r.name.clone().into(),
                short,
                bar_label: SharedString::default(),
                cents: r.spend_cents,
                share: r.share,
                color: r
                    .id
                    .map_or(muted, |id| colors.get(&id).copied().unwrap_or(muted)),
            })
            .collect();
        // Uncategorized is always the last category row, whatever its amount.
        rows.sort_by_key(|r| r.id.is_none());
        bar_share_labels(&mut rows, plot_width);
        rows
    }
    pub(super) fn chart_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (border, background, muted, list_active, radius, colors) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.background,
                theme.muted_foreground,
                theme.list_active,
                theme.radius,
                self.category_colors(theme),
            )
        };
        // Band label thinning for the axis (3.15); the rows themselves —
        // fitted names and share labels (3.15, 3.31) — come from the same
        // preparation the tests exercise.
        let label_count = self.chart_label_plan().1;
        let rows = self.chart_bars(self.chart_plot_width(), &colors, muted);
        let selection = self.dashboard.legend_selection;
        // The `Uncategorized` row is active when the 3.4 checkbox drives the
        // same clause, so the legend never contradicts the toolbar.
        let uncategorized_active = matches!(selection, Some(LegendSelection::Uncategorized))
            || self.dashboard.uncategorized_only;
        // The parent suffix of the tooltip text (" of Food" once drilled
        // in); empty at the top level.
        let parent_name = self
            .dashboard
            .chart_parent
            .and_then(|p| self.categories.iter().find(|n| n.id == p))
            .map(|n| format!(" of {}", n.name))
            .unwrap_or_default();
        let bar_parent_name = parent_name.clone();

        let chart = if rows.is_empty() {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child("No spending in this range")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .debug_selector(|| "chart:plot".into())
                .child(
                    BarChart::new(rows.clone())
                        .id("category-chart")
                        // The band text doubles as the axis label (3.15):
                        // the fitted short name, drawn on only
                        // `label_count` evenly spread bands so labels
                        // never overlap each other or the bars.
                        .band(|r| r.short.clone())
                        .band_tick_count(label_count)
                        .value(|r| r.cents as f64)
                        .fill(|r, _, _, _| r.color)
                        // The share above each bar (3.31): empty text on
                        // the bars the plan thinned out.
                        .label(|r| r.bar_label.clone())
                        .label_color(move |_| muted)
                        .tooltip_title(|r| r.name.clone())
                        .tooltip_value(move |r, _| {
                            chart_tooltip_value(r.cents, r.share, &bar_parent_name).into()
                        }),
                )
                .into_any_element()
        };

        let legend = v_flex()
            .id("legend-list")
            .debug_selector(|| "chart:legend".into())
            .w(px(LEGEND_W))
            .flex_none()
            .min_h_0()
            .gap_1()
            .overflow_y_scroll()
            .when(self.dashboard.chart_parent.is_some(), |list| {
                list.child(
                    div()
                        .id("legend-all")
                        .text_sm()
                        .px_1()
                        .py_1()
                        .cursor_pointer()
                        .child("← All categories")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_all_categories(window, cx);
                        })),
                )
            })
            .children(rows.iter().enumerate().map(|(ix, r)| {
                let id = r.id;
                let selected = match (id, selection) {
                    (None, _) => uncategorized_active,
                    (Some(id), Some(LegendSelection::Category(c))) => c == id,
                    _ => false,
                };
                // The row is just the key (3.31): swatch and name. What
                // the amount and share columns used to carry moves into
                // the hover tooltip, in the bar's own tooltip text.
                let value = chart_tooltip_value(r.cents, r.share, &parent_name);
                // 3.65: the selector records the highlight, so a test can
                // see the row paint selected in the frame right after the
                // click, while the refresh is still in flight.
                let row_selector = format!(
                    "chart:legend-row:{ix}{}",
                    if selected { ":active" } else { "" }
                );
                div()
                    .id(SharedString::from(format!("legend-{ix}")))
                    .debug_selector(move || row_selector.clone())
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .py_1()
                    .rounded(radius)
                    .cursor_pointer()
                    .when(selected, |row| row.bg(list_active))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.click_legend_row(id, window, cx);
                    }))
                    .child(div().size(px(10.)).flex_none().rounded(px(2.)).bg(r.color))
                    .child(
                        div()
                            .text_sm()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .debug_selector({
                                let name = r.name.clone();
                                move || format!("chart:legend-text:{ix}:{name}")
                            })
                            .when(id.is_none(), |name| name.text_color(muted))
                            .child(r.name.clone()),
                    )
                    .tooltip({
                        let (name, value) = (r.name.clone(), value);
                        move |window, cx| {
                            let (name, value) = (name.clone(), value.clone());
                            Tooltip::element(move |_, _| {
                                v_flex()
                                    .gap_0p5()
                                    .debug_selector({
                                        let (name, value) = (name.clone(), value.clone());
                                        move || format!("chart:legend-tooltip:{name}:{value}")
                                    })
                                    .child(
                                        div().font_weight(FontWeight::SEMIBOLD).child(name.clone()),
                                    )
                                    .child(value.clone())
                            })
                            .build(window, cx)
                        }
                    })
            }));

        h_flex()
            .w_full()
            .min_w_0()
            .h(px(340.))
            .flex_none()
            .gap_3()
            .child(
                // 3.16: the panel names itself, as the mockups show. The
                // height is explicit (3.25): the row's cross alignment does
                // not stretch, so without it the panel keeps its content
                // height and its title misses the summary's title line.
                v_flex()
                    .w(relative(CHART_PANEL_SHARE))
                    .h_full()
                    .min_w_0()
                    .debug_selector(|| "panel:chart".into())
                    .gap_2()
                    .p_4()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(background)
                    .child(region_title("Spending by category"))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .gap_3()
                            .child(chart)
                            .child(legend),
                    ),
            )
            .child(self.summary_panel(cx))
            .into_any_element()
    }
    /// The monthly summary panel (3.6): the right half of the dashboard's
    /// middle row. Rows are the spend parents — clicking a name or chevron
    /// expands or collapses its children — then `Income`, `Spend` and
    /// `Net`; columns are the months in the range plus the `Total`.
    /// Clicking a category cell filters the table to that category and
    /// month, and clicking the same cell again clears it (ui-conventions,
    /// *Interaction*). Amounts are positive spend; `Net` is the one signed
    /// row, `danger` when negative; `Income` reads as money in (`success`).
    fn summary_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (border, background, muted, list_active, danger, success, radius, colors) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.background,
                theme.muted_foreground,
                theme.list_active,
                theme.danger,
                theme.success,
                theme.radius,
                self.category_colors(theme),
            )
        };
        // `month_grid` fills the range's months even when no transaction
        // falls in it, so a range that excludes all data yields a grid of
        // zeros rather than an empty `months`. Treat an all-zero grid as
        // empty too (task 3.10): the panel says "No spending in this range".
        if !self.summary_has_data() {
            return v_flex()
                .h(px(340.))
                .flex_1()
                .min_w_0()
                .debug_selector(|| "panel:summary".into())
                .gap_2()
                .p_4()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(background)
                .child(region_title("Monthly summary"))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(muted)
                        .child("No spending in this range"),
                )
                .into_any_element();
        }
        let selection = self.dashboard.legend_selection;
        let selected_month = self.dashboard.summary_month.clone();
        let month_values = self.dashboard.summary.months.clone();
        let rows = self.summary_rows();

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
        let month_viewport = |selector: String, handle: &ScrollHandle| {
            div()
                .id(SharedString::from(selector.clone()))
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
                    &self.dashboard.summary_scroll,
                )
                .child(months_strip().children(
                    month_values.iter().enumerate().map(|(col_ix, month)| {
                        div()
                            .w(px(MONTH_W))
                            .flex_none()
                            .debug_selector(move || format!("summary:month-label-{col_ix}"))
                            .text_right()
                            .text_sm()
                            .text_color(muted)
                            .child(flow::month_label(month))
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

        let body = v_flex()
            .id("summary-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            // Without this a horizontal wheel delta over the month zone is
            // remapped onto this scroller's vertical axis too, so a swipe
            // would scroll the months and jump the rows at once (3.30).
            .restrict_scroll_to_axis()
            .children(rows.into_iter().enumerate().map(|(ix, row)| {
                let id = row.id;
                let is_category = !matches!(
                    row.kind,
                    SummaryRowKind::Income | SummaryRowKind::Spend | SummaryRowKind::Net
                );
                let swatch = match row.kind {
                    SummaryRowKind::Parent { .. } | SummaryRowKind::Child => {
                        Some(id.and_then(|id| colors.get(&id)).copied().unwrap_or(muted))
                    }
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
                    .id(SharedString::from(format!("summary-name-{ix}")))
                    .w(px(NAME_W))
                    .flex_none()
                    .debug_selector(move || format!("summary:name-{ix}"))
                    .gap_2()
                    .items_center()
                    .when(matches!(row.kind, SummaryRowKind::Child), |c| c.pl(px(14.)))
                    .when(expandable, |c| {
                        c.cursor_pointer().on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                if let Some(id) = id {
                                    this.toggle_summary_parent(id);
                                    cx.notify();
                                }
                            },
                        ))
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
                let mut total_cell: AnyElement =
                    div().w(px(TOTAL_W)).flex_none().into_any_element();
                for (col_ix, &cents) in row.cells.iter().enumerate() {
                    let is_total = col_ix >= month_count;
                    let cell_month = month_values.get(col_ix).cloned();
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
                        .id(SharedString::from(format!("summary-cell-{ix}-{col_ix}")))
                        .w(if is_total { px(TOTAL_W) } else { px(MONTH_W) })
                        .flex_none()
                        .debug_selector(move || format!("summary:cell:{ix}:{col_ix}"))
                        .text_right()
                        .text_sm()
                        .rounded(px(2.))
                        .when_some(color, |c, color| c.text_color(color))
                        .when(selected, |c| c.bg(list_active))
                        .when(is_category, |c| {
                            let month = cell_month.clone();
                            c.cursor_pointer().on_click(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    this.click_summary_cell(id, month.clone(), window, cx);
                                },
                            ))
                        })
                        .child(flow::money(cents))
                        .into_any_element();
                    if is_total {
                        total_cell = cell;
                    } else {
                        month_cells.push(cell);
                    }
                }

                h_flex()
                    .id(SharedString::from(format!("summary-row-{ix}")))
                    .gap_2()
                    .items_center()
                    .px_1()
                    .py_1()
                    .rounded(radius)
                    // A hairline above `Income` separates the totals from
                    // the category rows.
                    .when(is_income, |r| r.border_t_1().border_color(border))
                    .child(name_cell)
                    .child(
                        month_viewport(
                            format!("summary:months-{ix}"),
                            &self.dashboard.summary_scroll,
                        )
                        .child(months_strip().children(month_cells)),
                    )
                    .child(total_cell)
                    .into_any_element()
            }));

        v_flex()
            .h(px(340.))
            .flex_1()
            .min_w_0()
            .relative()
            .debug_selector(|| "panel:summary".into())
            .p_4()
            .gap_2()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(background)
            .child(region_title("Monthly summary"))
            // Month header and rows keep their tighter `gap_1`.
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .child(header)
                    .child(body),
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
                        Scrollbar::horizontal(&self.dashboard.summary_scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
            )
            .into_any_element()
    }
    /// Which dashboard empty state applies (task 3.10, ui-conventions
    /// *States*). `total_visible` counts every non-hidden row ignoring all
    /// filters, so it is the "empty database" signal; `row_count` is the
    /// filtered table, so `0` with data present means the filters excluded
    /// everything.
    pub(super) fn dashboard_body(&self) -> DashboardBody {
        if self.total_visible == 0 {
            DashboardBody::NoData
        } else if self.dashboard.row_count == 0 {
            DashboardBody::NoMatches
        } else {
            DashboardBody::Rows
        }
    }
    /// Whether the monthly summary has anything to draw. `month_grid` pads
    /// the range with zero months, so an empty range is an all-zero grid
    /// rather than an empty `months` (task 3.10).
    pub(super) fn summary_has_data(&self) -> bool {
        !self.dashboard.summary.months.is_empty()
            && self
                .dashboard
                .summary
                .rows
                .iter()
                .any(|r| r.months.iter().any(|v| *v != 0))
    }
    /// The table's empty state (task 3.10, ui-conventions *States*): shown
    /// when transactions exist but the current filters select none. A
    /// muted centered message plus a "Clear filters" link that drops every
    /// table-narrowing filter. The header controls (date range, accounts,
    /// transfers) keep their own widgets, so they stay as the user set them.
    pub(super) fn no_matches_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let (border, background, muted, radius) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.background,
                theme.muted_foreground,
                theme.radius,
            )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(background)
            .text_sm()
            .text_color(muted)
            .child("No transactions match these filters")
            .child(
                Button::new("clear-filters")
                    .link()
                    .label("Clear filters")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_filters(window, cx);
                    })),
            )
            .into_any_element()
    }
}
