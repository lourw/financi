//! Chart and summary panels of the Dashboard page (split out of
//! `dashboard.rs` in 3.74): the category bar chart, its label math, and the
//! summary table with their Shell methods.

use super::*;
use crate::ui::widgets::*;

const CHART_PANEL_SHARE: f32 = 0.45;
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
/// so that slice always buys `chars · char_px + CHART_LABEL_GAP`.
/// `char_px` is the chart font's per-character width — measured from the
/// rendered font (3.86), or `CHART_CHAR_PX` as the pre-measurement seed.
pub(super) fn band_label_budget(chart_width: f32, count: usize, char_px: f32) -> (usize, usize) {
    if count == 0 {
        return (0, 0);
    }
    let char_px = char_px.max(1.);
    let chars_for = |labels: usize| {
        let per = chart_width / labels as f32;
        (((per - CHART_LABEL_GAP) / char_px).max(0.)) as usize
    };
    // Prefer labelling every band.
    if chars_for(count) >= CHART_LABEL_MIN_CHARS {
        return (chars_for(count).min(CHART_LABEL_MAX_CHARS), count);
    }
    // Too tight: thin the labels out until each drawn one buys the
    // minimum budget, then truncate to what its wider slice allows.
    let min_spacing = CHART_LABEL_MIN_CHARS as f32 * char_px + CHART_LABEL_GAP;
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
/// `BAR_LABEL_MAX_CHARS · char_px + CHART_LABEL_GAP` apart; the
/// even first-to-last spread keeps the closest pair
/// `floor((count − 1) / (labels − 1))` bands apart, which the formula
/// below inverts. `char_px` is the measured chart-font character width
/// (3.86), as in [`band_label_budget`].
pub(super) fn bar_label_plan(chart_width: f32, count: usize, char_px: f32) -> usize {
    if count <= 1 {
        return count;
    }
    let band_w = chart_width / count as f32;
    let need = BAR_LABEL_MAX_CHARS as f32 * char_px.max(1.) + CHART_LABEL_GAP;
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
/// labelled or not. `char_px` is the measured chart-font character width
/// the plan uses (3.86).
fn bar_share_labels(rows: &mut [ChartRow], plot_width: f32, char_px: f32) {
    let slots = bar_label_slots(rows.len(), bar_label_plan(plot_width, rows.len(), char_px));
    for (row, labeled) in rows.iter_mut().zip(slots) {
        if labeled {
            row.bar_label = bar_share_label(row.share).into();
        }
    }
}

// ------------------------------------------------------------- 3.36 legend

/// A legend chip's name is ellipsized at this width (3.36): a category
/// name longer than it gets a `…` and its full text in the tooltip, which
/// bounds one chip's width so the two lines hold a useful number of them.
pub(super) const LEGEND_CHIP_NAME_MAX_W: f32 = 120.;
/// The gap between legend chips in a line (3.36): `gap_x_3`.
pub(super) const LEGEND_GAP_X: f32 = 12.;
/// The gap between two legend lines (3.36): `gap_y_1`.
pub(super) const LEGEND_GAP_Y: f32 = 4.;
/// How many lines of chips the legend gets (3.36): the panel is a fixed
/// height, so the legend caps at two lines and the categories past them
/// hide behind a "+N more" chip.
pub(super) const LEGEND_LINES: usize = 2;
/// The legend row's height cap (3.36): two `text_xs` lines plus the gap
/// between them, with room to spare. The row is clipped to it, so even a
/// width estimate that ran short can never push the panel taller.
pub(super) const LEGEND_MAX_H: f32 = 44.;
/// A chip's horizontal padding on each side (3.36): `px_1`, which gives
/// the selected chip's background its breathing room.
const LEGEND_CHIP_PAD_X: f32 = 4.;
/// A chip's fixed parts (3.36): the padding on both sides plus the 10 px
/// swatch and the `gap_1` between swatch and name.
const LEGEND_CHIP_FIXED_W: f32 = 2. * LEGEND_CHIP_PAD_X + 10. + 4.;
/// The chip that clears a drill (3.36): first in the row while the chart
/// is inside a category.
const LEGEND_ALL_LABEL: &str = "← All categories";

/// A legend chip's estimated width (3.36): swatch, padding and the name
/// at `char_px` per character, capped at the ellipsis width. `char_px` is
/// the measured `M` advance at the chip's `text_xs` size, so the estimate
/// errs wide: a planned line can under-fill, never overflow.
pub(super) fn legend_chip_w(name_chars: usize, char_px: f32) -> f32 {
    LEGEND_CHIP_FIXED_W + (name_chars as f32 * char_px).min(LEGEND_CHIP_NAME_MAX_W)
}

/// The label of the chip that counts the categories the lines can't show
/// (3.36): `+N more`.
pub(super) fn legend_more_label(hidden: usize) -> String {
    format!("+{hidden} more")
}

/// The "+N more" chip's estimated width (3.36): no swatch, just the
/// label.
pub(super) fn legend_more_w(hidden: usize, char_px: f32) -> f32 {
    2. * LEGEND_CHIP_PAD_X + legend_more_label(hidden).chars().count() as f32 * char_px
}

/// The "← All categories" chip's estimated width (3.36): no swatch.
pub(super) fn legend_all_w(char_px: f32) -> f32 {
    2. * LEGEND_CHIP_PAD_X + LEGEND_ALL_LABEL.chars().count() as f32 * char_px
}

/// How many lines chips of `widths` wrap into at `avail` wide with
/// `gap_x` between chips (3.36, the greedy wrapping a flex-wrap row
/// does): a chip that doesn't fit the current line starts the next one;
/// an empty legend is no lines at all.
pub(super) fn legend_lines(widths: &[f32], avail: f32, gap_x: f32) -> usize {
    if widths.is_empty() {
        return 0;
    }
    let mut lines = 1;
    let mut used = 0.;
    for &w in widths {
        if used > 0. {
            if used + gap_x + w <= avail {
                used += gap_x + w;
                continue;
            }
            lines += 1;
        }
        used = w;
    }
    lines
}

/// Which of `widths` the legend shows (3.36): `None` when all of them fit
/// the `lines` lines, otherwise `Some(count)`, the longest prefix that
/// also fits a "+N more" chip after it — the plan never overflows the cap
/// it was given. An unmeasured `avail` shows everything: the row is about
/// to be laid out at its real width, and clipping to nothing until then
/// would flash an empty legend.
pub(super) fn legend_visible(
    widths: &[f32],
    avail: f32,
    gap_x: f32,
    lines: usize,
    char_px: f32,
) -> Option<usize> {
    if avail <= 0. || legend_lines(widths, avail, gap_x) <= lines {
        return None;
    }
    for visible in (0..widths.len()).rev() {
        let mut shown: Vec<f32> = widths[..visible].to_vec();
        shown.push(legend_more_w(widths.len() - visible, char_px));
        if legend_lines(&shown, avail, gap_x) <= lines {
            return Some(visible);
        }
    }
    Some(0)
}

impl DashboardPage {
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter.legend_selection = match id {
            None => (self.filter.legend_selection != Some(LegendSelection::Uncategorized))
                .then_some(LegendSelection::Uncategorized),
            Some(id) if self.filter.chart_parent.is_none() => {
                self.filter.chart_parent = Some(id);
                Some(LegendSelection::Category(id))
            }
            Some(id) => (self.filter.legend_selection != Some(LegendSelection::Category(id)))
                .then_some(LegendSelection::Category(id)),
        };
        if matches!(
            self.filter.legend_selection,
            Some(LegendSelection::Category(_))
        ) {
            self.filter.uncategorized_only = false;
        }
        // The month only ever narrows a category selection (the cell
        // highlight and the chip read both from the pair), so clearing the
        // selection clears it too — Esc does the same for both.
        if self.filter.legend_selection.is_none() {
            self.filter.summary_month = None;
        }
        // 3.65: the legend highlight paints with this click, not with the
        // answer (the rows and totals are the only part that waits).
        cx.notify();
        self.reload(cx);
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
        cx.notify();
        self.reload(cx);
    }
    /// Move the chart so a selection made outside the legend (a summary
    /// cell) is visible in the legend rows: a parent shows at the top
    /// level, a child only while drilled into its parent. Keeps the
    /// refresh revalidation of `legend_selection` from dropping a
    /// summary-made selection.
    fn sync_chart_to_selection(&mut self) {
        let Some(selection) = self.filter.legend_selection else {
            return;
        };
        match selection {
            LegendSelection::Uncategorized => self.filter.chart_parent = None,
            LegendSelection::Category(id) => {
                if let Some(parent) = self.parent_of(id) {
                    self.filter.chart_parent = Some(parent);
                } else if self.filter.chart_parent != Some(id) {
                    self.filter.chart_parent = None;
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
        if !self.expanded_parents.insert(id) {
            self.expanded_parents.remove(&id);
        }
        // 3.85: the expanded set picks which child rows are in the grid.
        self.summary_views_dirty = true;
    }
    /// 3.85: the grid's rows from the cache, rebuilt on the first render
    /// after a summary load, a parent toggle or a category change — the
    /// only inputs `summary_rows` reads.
    pub(super) fn summary_views_mut(&mut self) -> Rc<Vec<SummaryRowView>> {
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
    pub(super) fn summary_rows(&self) -> Vec<SummaryRowView> {
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
}

impl DashboardPage {
    /// The plot's width the last probe measured (3.86): the `chart:plot`
    /// container's own laid-out width, reported from its `bounds_probe`
    /// as the view tree paints. `0.` until the first frame has measured
    /// it, which the label plans degrade safely.
    pub(super) fn chart_plot_width(&self) -> f32 {
        self.plot_w
    }
    /// Store a freshly measured plot width (3.86, from the probe on the
    /// `chart:plot` container); repaint for the new label plan only when
    /// the width actually moved, so the measure-then-repaint loop settles.
    pub(in crate::ui) fn set_plot_width(&mut self, w: f32, cx: &mut Context<Self>) {
        if (self.plot_w - w).abs() < 0.5 {
            return;
        }
        self.plot_w = w;
        cx.notify();
    }
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
        cx.notify();
    }
    /// The chart's label plan (3.15) for the measured plot: how many
    /// characters a band label may use and how many labels are drawn.
    pub(super) fn chart_label_plan(&self) -> (usize, usize) {
        band_label_budget(
            self.chart_plot_width(),
            self.chart_rows.len(),
            self.chart_char_px,
        )
    }
    /// The chart's rows prepared for drawing (3.15, 3.31): band labels
    /// fitted to the current budget, each row's share over its bar
    /// thinned to `plot_width`, and `Uncategorized` last whatever its
    /// amount. `colors` maps category ids to palette indices (3.85, the
    /// Catalog's map); `palette` is the current theme's 12 colours and
    /// `muted` paints the `Uncategorized` swatch.
    pub(super) fn chart_bars(
        &self,
        plot_width: f32,
        colors: &HashMap<i64, usize>,
        palette: [Hsla; 12],
        muted: Hsla,
    ) -> Vec<ChartRow> {
        let (label_chars, _) = self.chart_label_plan();
        let shorts = short_chart_labels(
            &self
                .chart_rows
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<String>>(),
            label_chars,
        );
        let mut rows: Vec<ChartRow> = self
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
                color: r.id.map_or(muted, |id| {
                    colors.get(&id).map(|ix| palette[ix % 12]).unwrap_or(muted)
                }),
            })
            .collect();
        // Uncategorized is always the last category row, whatever its amount.
        rows.sort_by_key(|r| r.id.is_none());
        bar_share_labels(&mut rows, plot_width, self.chart_char_px);
        rows
    }
    /// The dashboard middle row (3.5): the `BarChart` of spend per category
    /// with a clickable chip legend under it (3.36), in the left half; the
    /// monthly summary (3.6) takes the right half later. `BarChart` takes
    /// no clicks, so all interactivity lives in the legend chips
    /// (ui-conventions, *Interaction*).
    pub(super) fn chart_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // 3.85: the palette-index map comes from the Catalog, built once
        // per categories reload; the palette itself is the current theme's.
        let (muted, list_active, radius, colors, palette) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.list_active,
                theme.radius,
                self.catalog.read(cx).cat_colors(),
                category_palette(theme),
            )
        };
        // Band label thinning for the axis (3.15); the rows themselves —
        // fitted names and share labels (3.15, 3.31) — come from the same
        // preparation the tests exercise.
        let label_count = self.chart_label_plan().1;
        let rows = self.chart_bars(self.chart_plot_width(), &colors, palette, muted);
        let selection = self.filter.legend_selection;
        // The `Uncategorized` row is active when the 3.4 checkbox drives the
        // same clause, so the legend never contradicts the toolbar.
        let uncategorized_active = matches!(selection, Some(LegendSelection::Uncategorized))
            || self.filter.uncategorized_only;
        // The parent suffix of the tooltip text (" of Food" once drilled
        // in); empty at the top level.
        let parent_name = self
            .filter
            .chart_parent
            .and_then(|p| self.categories.iter().find(|n| n.id == p))
            .map(|n| format!(" of {}", n.name))
            .unwrap_or_default();
        let bar_parent_name = parent_name.clone();

        let chart = if rows.is_empty() {
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
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
                .min_h_0()
                .relative()
                .debug_selector(|| "chart:plot".into())
                // 3.86: the plot's width comes from this container's own
                // measured bounds (the probe paints nothing and takes no
                // part in layout), not from window-width arithmetic.
                .child(bounds_probe(cx.entity().downgrade(), |this, w, cx| {
                    this.set_plot_width(w, cx)
                }))
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

        // 3.36: the legend sits under the plot, not beside it, so the
        // plot takes the panel's full inner width. The chips wrap into at
        // most `LEGEND_LINES` lines and the categories past them hide
        // behind a "+N more" chip. The plan's available width is the one
        // the plot's own probe measures (the legend spans the same panel
        // inner width); the chip character estimate is the measured `M`
        // advance scaled from the chart's 10 px font to the chips' 12 px
        // `text_xs`.
        let char_xs = self.chart_char_px * 12. / 10.;
        let drilled = self.filter.chart_parent.is_some();
        let mut widths: Vec<f32> = Vec::with_capacity(rows.len() + 1);
        if drilled {
            widths.push(legend_all_w(char_xs));
        }
        widths.extend(
            rows.iter()
                .map(|r| legend_chip_w(r.name.chars().count(), char_xs)),
        );
        let shown = legend_visible(
            &widths,
            self.chart_plot_width(),
            LEGEND_GAP_X,
            LEGEND_LINES,
            char_xs,
        )
        .unwrap_or(widths.len());
        let hidden = widths.len() - shown;
        // The names the "+N more" tooltip lists.
        let hidden_names: String = rows
            .iter()
            .enumerate()
            .filter(|(ix, _)| ix + drilled as usize >= shown)
            .map(|(_, r)| r.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let legend = div()
            .id("legend-list")
            .debug_selector(|| "chart:legend".into())
            .w_full()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap_x(px(LEGEND_GAP_X))
            .gap_y(px(LEGEND_GAP_Y))
            .max_h(px(LEGEND_MAX_H))
            .overflow_hidden()
            .when(drilled && shown >= 1, |list| {
                list.child(
                    div()
                        .id("legend-all")
                        .debug_selector(|| "chart:legend-all".into())
                        .flex()
                        .flex_none()
                        .items_center()
                        .px_1()
                        .rounded(radius)
                        .text_xs()
                        .cursor_pointer()
                        .child(LEGEND_ALL_LABEL)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_all_categories(window, cx);
                        })),
                )
            })
            .children(rows.iter().enumerate().filter_map(|(ix, r)| {
                // 3.36: chips past the two lines are not built at all —
                // a clipped-but-laid-out chip would still answer
                // `debug_bounds` and steal a click.
                if ix + drilled as usize >= shown {
                    return None;
                }
                let id = r.id;
                let selected = match (id, selection) {
                    (None, _) => uncategorized_active,
                    (Some(id), Some(LegendSelection::Category(c))) => c == id,
                    _ => false,
                };
                // The chip is just the key (3.31): swatch and name. What
                // the amount and share columns used to carry lives in
                // the hover tooltip, in the bar's own tooltip text.
                let value = chart_tooltip_value(r.cents, r.share, &parent_name);
                // 3.65: the selector records the highlight, so a test can
                // see the row paint selected in the frame right after the
                // click, while the refresh is still in flight.
                let row_selector = format!(
                    "chart:legend-row:{ix}{}",
                    if selected { ":active" } else { "" }
                );
                Some(
                    div()
                        .id(SharedString::from(format!("legend-{ix}")))
                        .debug_selector(move || row_selector.clone())
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .px_1()
                        .rounded(radius)
                        .cursor_pointer()
                        .when(selected, |row| row.bg(list_active))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.click_legend_row(id, window, cx);
                        }))
                        .child(div().size(px(10.)).flex_none().rounded(px(2.)).bg(r.color))
                        .child(
                            div()
                                .text_xs()
                                .max_w(px(LEGEND_CHIP_NAME_MAX_W))
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
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(name.clone()),
                                        )
                                        .child(value.clone())
                                })
                                .build(window, cx)
                            }
                        }),
                )
            }))
            .when(hidden > 0, |list| {
                list.child(
                    div()
                        .id("legend-more")
                        .debug_selector(move || format!("chart:legend-more:{hidden}"))
                        .flex()
                        .flex_none()
                        .items_center()
                        .px_1()
                        .rounded(radius)
                        .text_xs()
                        .text_color(muted)
                        .child(legend_more_label(hidden))
                        .tooltip(move |window, cx| {
                            let names = hidden_names.clone();
                            Tooltip::element(move |_, _| {
                                let names = names.clone();
                                v_flex()
                                    .gap_0p5()
                                    .debug_selector({
                                        let names = names.clone();
                                        move || {
                                            format!("chart:legend-more-tooltip:{hidden}:{names}")
                                        }
                                    })
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(names))
                            })
                            .build(window, cx)
                        }),
                )
            });

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
                // 3.91: the shared `panel`, `flex_none` so its relative
                // width is not overridden by the panel's own flex.
                // 3.36: title, plot, legend — the legend a wrapping chip
                // row under the plot instead of a column beside it.
                panel(cx, true)
                    .w(relative(CHART_PANEL_SHARE))
                    .h_full()
                    .min_w_0()
                    .flex_none()
                    .debug_selector(|| "panel:chart".into())
                    .gap_2()
                    .child(region_title("Spending by category"))
                    .child(chart)
                    .child(legend),
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
        // 3.85: palette indices from the Catalog (see `chart_panel`).
        let (border, muted, list_active, danger, success, radius, colors, palette) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.muted_foreground,
                theme.list_active,
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
            return panel(cx, true)
                .h(px(340.))
                .min_w_0()
                .debug_selector(|| "panel:summary".into())
                .gap_2()
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
        let selection = self.filter.legend_selection;
        let selected_month = self.filter.summary_month.clone();
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

        // Pinned row height (3.85): the virtual list must know every row's
        // height before drawing it, and the row markup pins the same
        // height — `text_sm` (15 px, a 22.5 px line box) plus `py_1`
        // (2 × 4 px); the `Income` row adds its 1 px top border.
        const ROW_H: f32 = 30.5;
        const INCOME_ROW_H: f32 = 31.5;
        let item_sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
            views
                .iter()
                .map(|row| {
                    let h = if matches!(row.kind, SummaryRowKind::Income) {
                        INCOME_ROW_H
                    } else {
                        ROW_H
                    };
                    size(px(0.), px(h))
                })
                .collect(),
        );
        // 3.85: the grid body virtualizes (only visible rows build
        // elements) and renders the cached, pre-formatted views, so no
        // `flow::money` call runs per row per frame. The list re-runs
        // this closure on every paint — gpui-base's `VirtualList` keeps
        // no item cache and `track_scroll` persists only the scroll
        // offset — which is why it only copies from `views` and does no
        // deriving of its own. `DataList` was
        // considered and does not fit: it pins 40 px rows (this grid
        // measures 30.5), paints its own row hover and heading, and keys
        // rows by `EntityId`. The `VirtualList` base already locks the
        // scroll axis, replacing the body's `restrict_scroll_to_axis`.
        let strip_months = month_values.clone();
        let scroll_handle = self.summary_scroll.clone();
        let body = v_virtual_list(
            cx.entity(),
            "summary-rows",
            item_sizes,
            move |_this, range, _window, cx| {
                (range.start..range.end)
                    .map(|ix| {
                        let row = &views[ix];
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
                        for (col_ix, &cents) in row.cents.iter().enumerate() {
                            let is_total = col_ix >= month_count;
                            let cell_month = strip_months.get(col_ix).cloned();
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

                        h_flex()
                            .id(SharedString::from(format!("summary-row-{key}")))
                            .w_full()
                            .gap_2()
                            .items_center()
                            // 3.85: pinned to match the virtual list's
                            // `item_sizes`, so measured height never drifts.
                            .h(px(if is_income { INCOME_ROW_H } else { ROW_H }))
                            .overflow_hidden()
                            .px_1()
                            .py_1()
                            .rounded(radius)
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
                    })
                    .collect::<Vec<_>>()
            },
        )
        // The panel gives the list its width; measuring a row at
        // min-content would size the list's canvas to the whole month
        // strip (24 fixed columns) and paint the rows wider than the
        // panel, so take the width from layout instead (3.85).
        .with_sizing_behavior(ListSizingBehavior::Auto)
        // 3.85: page-owned handle (the rules page's pattern) — the
        // wheel scrolls it and the offset survives re-renders.
        .track_scroll(&self.summary_rows_scroll)
        .flex_1()
        .min_h_0();

        panel(cx, true)
            .h(px(340.))
            .min_w_0()
            .relative()
            .debug_selector(|| "panel:summary".into())
            .gap_2()
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
                        Scrollbar::horizontal(&self.summary_scroll)
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
        } else if self.row_count == 0 {
            DashboardBody::NoMatches
        } else {
            DashboardBody::Rows
        }
    }
    /// Whether the monthly summary has anything to draw. `month_grid` pads
    /// the range with zero months, so an empty range is an all-zero grid
    /// rather than an empty `months` (task 3.10).
    pub(super) fn summary_has_data(&self) -> bool {
        !self.summary.months.is_empty()
            && self
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
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_filters(window, cx);
                    })),
            )
            .into_any_element()
    }
}
