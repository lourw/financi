//! The chart panel (3.97): the category bar chart, its label and legend
//! math, the legend/drill selection handlers, and the `ChartPanel`
//! entity. Split out of `ui/dashboard_chart.rs` (3.74's file, 3.97's
//! entities); the render methods stay on [`DashboardState`] — the panel
//! entity mounts them and subscribes to what they read.

use super::*;
use crate::ui::widgets::*;

/// Minimum horizontal gap between two neighbouring drawn band labels.
pub(in crate::ui) const CHART_LABEL_GAP: f32 = 8.;
/// Cap on a band label, so a wide chart never paints a sentence.
pub(in crate::ui) const CHART_LABEL_MAX_CHARS: usize = 16;
/// Smallest budget that makes a label worth drawing; below it labels are
/// thinned out instead of reduced to unreadable slivers.
pub(in crate::ui) const CHART_LABEL_MIN_CHARS: usize = 4;
/// The chart's label plan for `chart_width` pixels across `count` bands:
/// band labels are truncated to `chars` characters (with an ellipsis)
/// and only `labels` of them are drawn, spread evenly first-to-last by
/// `BarChart::band_tick_count`. A centred label cannot collide with its
/// neighbours once every drawn label owns `chart_width / labels` pixels,
/// so that slice always buys `chars · char_px + CHART_LABEL_GAP`.
/// `char_px` is the chart font's per-character width — measured from the
/// rendered font (3.86), or `CHART_CHAR_PX` as the pre-measurement seed.
pub(in crate::ui) fn band_label_budget(
    chart_width: f32,
    count: usize,
    char_px: f32,
) -> (usize, usize) {
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
pub(in crate::ui) fn short_chart_labels(names: &[String], chars: usize) -> Vec<SharedString> {
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
pub(in crate::ui) const BAR_LABEL_MAX_CHARS: usize = 4;
/// A bar's share label: whole percent (`18%`), one decimal only below
/// one percent (`0.4%`) so a thin slice still reads — and never `0.0%`:
/// a sliver too small to round to a tenth of a percent reads `0.1%`.
pub(in crate::ui) fn bar_share_label(share: f64) -> String {
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
pub(in crate::ui) fn chart_tooltip_value(cents: i64, share: f64, parent: &str) -> String {
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
pub(in crate::ui) fn bar_label_plan(chart_width: f32, count: usize, char_px: f32) -> usize {
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
pub(in crate::ui) fn bar_label_slots(count: usize, labels: usize) -> Vec<bool> {
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
pub(in crate::ui) const LEGEND_CHIP_NAME_MAX_W: f32 = 120.;
/// The gap between legend chips in a line (3.36): `gap_x_3`.
pub(in crate::ui) const LEGEND_GAP_X: f32 = 12.;
/// The gap between two legend lines (3.36): `gap_y_1`.
pub(in crate::ui) const LEGEND_GAP_Y: f32 = 4.;
/// How many lines of chips the legend gets (3.36): the panel is a fixed
/// height, so the legend caps at two lines and the categories past them
/// hide behind a "+N more" chip.
pub(in crate::ui) const LEGEND_LINES: usize = 2;
/// The legend row's height cap (3.36): two `text_xs` lines plus the gap
/// between them, with room to spare. The row is clipped to it, so even a
/// width estimate that ran short can never push the panel taller.
pub(in crate::ui) const LEGEND_MAX_H: f32 = 44.;
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
pub(in crate::ui) fn legend_chip_w(name_chars: usize, char_px: f32) -> f32 {
    LEGEND_CHIP_FIXED_W + (name_chars as f32 * char_px).min(LEGEND_CHIP_NAME_MAX_W)
}

/// The label of the chip that counts the categories the lines can't show
/// (3.36): `+N more`.
pub(in crate::ui) fn legend_more_label(hidden: usize) -> String {
    format!("+{hidden} more")
}

/// The "+N more" chip's estimated width (3.36): no swatch, just the
/// label.
pub(in crate::ui) fn legend_more_w(hidden: usize, char_px: f32) -> f32 {
    2. * LEGEND_CHIP_PAD_X + legend_more_label(hidden).chars().count() as f32 * char_px
}

/// The "← All categories" chip's estimated width (3.36): no swatch.
pub(in crate::ui) fn legend_all_w(char_px: f32) -> f32 {
    2. * LEGEND_CHIP_PAD_X + LEGEND_ALL_LABEL.chars().count() as f32 * char_px
}

/// How many lines chips of `widths` wrap into at `avail` wide with
/// `gap_x` between chips (3.36, the greedy wrapping a flex-wrap row
/// does): a chip that doesn't fit the current line starts the next one;
/// an empty legend is no lines at all.
pub(in crate::ui) fn legend_lines(widths: &[f32], avail: f32, gap_x: f32) -> usize {
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
pub(in crate::ui) fn legend_visible(
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

impl DashboardState {
    /// Click a legend row (3.5). A parent at the top level drills the chart
    /// and legend into its children and filters the table to that parent;
    /// a child filters the table to itself, and clicking the selected child
    /// again clears the filter (the chart stays on the children). The
    /// `Uncategorized` pseudo-row toggles the uncategorized filter.
    /// Selecting a real category clears the "Uncategorized only" checkbox —
    /// the two can't both narrow the table.
    pub(in crate::ui) fn click_legend_row(
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
            Some(id) => {
                let selected = (self.filter.legend_selection
                    != Some(LegendSelection::Category(id)))
                .then_some(LegendSelection::Category(id));
                if selected.is_some() {
                    // 3.46: selecting a child here must expand its parent's
                    // summary row too, or the selected row never paints
                    // (`click_summary_cell` does the same through
                    // `sync_chart_to_selection`).
                    if let Some(parent) = self.parent_of(id) {
                        self.expand_parent_row(parent);
                    }
                }
                selected
            }
        };
        if matches!(
            self.filter.legend_selection,
            Some(LegendSelection::Category(_))
        ) {
            self.filter.uncategorized_only = false;
        }
        // 3.46: the month is an independent global filter, so selecting
        // or clearing a category leaves it alone. Esc still clears just
        // the selection (the chips and Clear all take the month).
        // 3.65: the legend highlight paints with this click, not with the
        // answer (the rows and totals are the only part that waits).
        self.changed(DashboardChanges::SELECTION, cx);
        self.reload(cx);
    }
    /// Move the chart so a selection made outside the legend (a summary
    /// cell) is visible in the legend rows: a parent shows at the top
    /// level, a child only while drilled into its parent. Keeps the
    /// refresh revalidation of `legend_selection` from dropping a
    /// summary-made selection.
    pub(super) fn sync_chart_to_selection(&mut self) {
        let Some(selection) = self.filter.legend_selection else {
            return;
        };
        match selection {
            LegendSelection::Uncategorized => self.filter.chart_parent = None,
            LegendSelection::Category(id) => {
                if let Some(parent) = self.parent_of(id) {
                    self.filter.chart_parent = Some(parent);
                    // 3.46: selecting a child from the grid reveals its
                    // row — the parent expands so the selected child is
                    // visible with its own highlighted cell.
                    self.expand_parent_row(parent);
                } else if self.filter.chart_parent != Some(id) {
                    self.filter.chart_parent = None;
                }
            }
        }
    }
    /// Expand `parent`'s children in the summary grid (3.46): a selected
    /// child's `list_active` row only exists while its parent is expanded,
    /// so both selection paths (legend chip and summary cell) go through
    /// here.
    pub(super) fn expand_parent_row(&mut self, parent: i64) {
        if !self.expanded_parents.contains(&parent) {
            self.expanded_parents.insert(parent);
            self.summary_views_dirty = true;
        }
    }
    /// The id of `id`'s parent in the category tree, `None` for a
    /// top-level category.
    pub(super) fn parent_of(&self, id: i64) -> Option<i64> {
        self.categories
            .iter()
            .find(|p| p.children.iter().any(|c| c.id == id))
            .map(|p| p.id)
    }
    /// The plot's width the last probe measured (3.86): the `chart:plot`
    /// container's own laid-out width, reported from its `bounds_probe`
    /// as the view tree paints. `0.` until the first frame has measured
    /// it, which the label plans degrade safely.
    pub(in crate::ui) fn chart_plot_width(&self) -> f32 {
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
        self.changed(DashboardChanges::MEASURED, cx);
    }
    /// The chart's label plan (3.15) for the measured plot: how many
    /// characters a band label may use and how many labels are drawn.
    pub(in crate::ui) fn chart_label_plan(&self) -> (usize, usize) {
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
    pub(in crate::ui) fn chart_bars(
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
        // 3.46: a selection keeps its bar and dims the others to about a
        // third rather than filtering them away, so the bars stay
        // comparable. `BarChart` has no per-row alpha, so the colour is
        // dimmed before it goes in. Drilled into the selected parent,
        // every bar is part of the selection, so nothing dims there.
        let selected = match &self.filter.legend_selection {
            Some(LegendSelection::Category(id)) => Some(*id),
            _ => None,
        };
        let drilled_into_selection = selected.is_some() && self.filter.chart_parent == selected;
        let unc_active = matches!(
            self.filter.legend_selection,
            Some(LegendSelection::Uncategorized)
        ) || self.filter.uncategorized_only;
        let mut rows: Vec<ChartRow> = self
            .chart_rows
            .iter()
            .zip(shorts)
            .map(|(r, short)| {
                let color = r.id.map_or(muted, |id| {
                    colors.get(&id).map(|ix| palette[ix % 12]).unwrap_or(muted)
                });
                // The `Uncategorized` pseudo-row is never the selected
                // category, so it dims for any real-category selection,
                // even while the chart is drilled into one.
                let dimmed = match r.id {
                    Some(id) => {
                        (selected.is_some() || unc_active)
                            && !drilled_into_selection
                            && selected != Some(id)
                    }
                    None => selected.is_some() && !unc_active,
                };
                ChartRow {
                    id: r.id,
                    name: r.name.clone().into(),
                    short,
                    bar_label: SharedString::default(),
                    cents: r.spend_cents,
                    share: r.share,
                    color: if dimmed { color.opacity(0.35) } else { color },
                }
            })
            .collect();
        // Uncategorized is always the last category row, whatever its amount.
        rows.sort_by_key(|r| r.id.is_none());
        bar_share_labels(&mut rows, plot_width, self.chart_char_px);
        rows
    }
    /// The chart's row (3.5, 3.45): the `BarChart` of spend per category
    /// with a clickable chip legend under it (3.36), full width and 340 px
    /// tall, above the summary's own row. `BarChart` takes
    /// no clicks, so all interactivity lives in the legend chips
    /// (ui-conventions, *Interaction*).
    pub(in crate::ui) fn chart_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

        // 3.45: the chart takes its own full-width row above the summary
        // (the old side-by-side 45/55 split is gone). The 340 px height
        // stays explicit — the page scroller gives its children no height
        // to fill — and `flex_none` keeps it at exactly that.
        // 3.46: a selected month narrows the chart too, and the title says
        // which month ("Spending by category · Oct 2026").
        let title: SharedString = match self.filter.summary_month.as_deref() {
            Some(m) => format!("Spending by category · {}", flow::month_label(m)).into(),
            None => "Spending by category".into(),
        };
        panel(cx, true)
            .w_full()
            .h(px(340.))
            .flex_none()
            .min_w_0()
            .debug_selector(|| "panel:chart".into())
            .gap_2()
            .child(region_title(title))
            .child(chart)
            .child(legend)
            .into_any_element()
    }
}

/// The category bar chart and its legend, with `panel:chart`'s probes and selectors (3.97): a dock-shaped entity (3.100) that owns nothing but
/// its handle on [`DashboardState`] and a subscription narrowed to the
/// changes it paints from — a repaint of this panel is a `cx.notify()`
/// on this entity alone, and its paint reads the state back through
/// [`DashboardState::chart_panel`].
pub(in crate::ui::dashboard) struct ChartPanel {
    /// The shared state this panel reads and writes selections through.
    state: Entity<DashboardState>,
    /// The state subscription; lives as long as the panel.
    _subs: Subscription,
}

impl ChartPanel {
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

impl Render for ChartPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state.update(cx, |state, cx| state.chart_panel(cx))
    }
}
