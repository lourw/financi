//! The chart panel (3.97, donut in 3.99): the category donut chart, its
//! layout and legend math, the legend/drill selection handlers, and the
//! `ChartPanel` entity. Split out of `ui/dashboard_chart.rs` (3.74's
//! file, 3.97's entities); the render methods stay on [`DashboardState`]
//! — the panel entity mounts them and subscribes to what they read.
//!
//! 3.99 replaced the `BarChart` with gpui-kit's `PieChart`: a donut of
//! the range's spend per category (largest first, clockwise from 12
//! o'clock, the total in the hole), sized and placed from the measured
//! content bounds. `PieChart` takes no clicks, so the legend chips stay
//! the only click target, exactly as the bars were.

use super::*;
use crate::ui::widgets::*;
use gpui_kit::base::dock::{Panel, PanelEvent};
use gpui_kit::component::chart::PieChart;

/// The tooltip body of a chart row (3.31), shared by the slice's own
/// tooltip and the legend row's: `$1,234.56 · 18.2% of Food`. `parent`
/// carries the ` of …` suffix when the chart is drilled into a category,
/// and is empty at the top level.
pub(in crate::ui) fn chart_tooltip_value(cents: i64, share: f64, parent: &str) -> String {
    format!("{} · {:.1}%{parent}", flow::money(cents), share * 100.)
}

/// Which of `count` slots carry a label — `labels` of them, spread
/// evenly from the first to the last: the thinning
/// `BarChart::band_tick_count` applies to band labels (3.15), done here
/// because `BarChart::label` paints whatever text it is handed for
/// every slot. The 3.99 donut dropped its share labels and needs this
/// no more; the monthly stacked panel (3.93) still thins its month
/// labels with it.
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

// ------------------------------------------------------------- 3.36 legend

/// A legend chip's name is ellipsized at this width (3.36): a category
/// name longer than it gets a `…` and its full text in the tooltip, which
/// bounds one chip's width so the two lines hold a useful number of them.
pub(in crate::ui) const LEGEND_CHIP_NAME_MAX_W: f32 = 120.;
/// The gap between legend chips in a line (3.36): `gap_x_3`.
pub(in crate::ui) const LEGEND_GAP_X: f32 = 12.;
/// The gap between two legend lines (3.36): `gap_y_1`.
pub(in crate::ui) const LEGEND_GAP_Y: f32 = 4.;
/// A legend chip's painted line box (3.99): the swatch and the text line
/// with the chip's own padding, measured at 19.5 px — what
/// [`LEGEND_MAX_H`] budgets two of plus a gap. The side-by-side plan
/// counts lines with this, not with the bare text line, or the "+N more"
/// chip plans below the box it is clipped to and can't be hovered.
pub(in crate::ui) const LEGEND_CHIP_H: f32 = 19.5;
/// The gap between a month label and its bar on the stacked chart (3.31):
/// the donut paints no band labels, but [`super::stacked`] still spaces
/// its labels by this, so the constant stays here with the shared
/// label-planning helpers.
pub(in crate::ui) const CHART_LABEL_GAP: f32 = 8.;
/// How many lines of chips the legend gets when it wraps *under* the
/// donut (3.36): the legend caps at two lines and the categories past
/// them hide behind a "+N more" chip. Beside the donut (3.99) the lines
/// are whatever the panel's measured height fits.
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

// -------------------------------------------------------- 3.99 donut model

/// How many slices the donut shows (3.99): the largest `PIE_TOP_N`
/// categories keep their own slice and the rest merge into one muted
/// "Other" slice. The legend still lists every category.
pub(in crate::ui) const PIE_TOP_N: usize = 8;
/// The merged tail slice's name (3.99). It is never clickable: its
/// tooltip lists the categories it holds.
pub(in crate::ui) const OTHER_NAME: &str = "Other";
/// The donut never drops below this across (3.99); under it the panel
/// shows the legend alone.
pub(in crate::ui) const DONUT_MIN: f32 = 120.;
/// The gap between the donut and the legend, and between the legend and
/// the donut, in px (`gap_2`).
pub(in crate::ui) const DONUT_GAP: f32 = 8.;
/// The ring's outer radius sits this far inside half the donut box, so a
/// slice lifting out on hover (gpui-kit lifts it 6 px) never paints over
/// the panel's padding.
const DONUT_EDGE_MARGIN: f32 = 8.;
/// The hole's inner radius as a fraction of the outer one (3.99): "about
/// 60 %", room for the total in the middle.
const DONUT_INNER_FRACTION: f32 = 0.6;

/// One slice of the donut: a category (or the merged `Other`), already
/// carrying the palette colour — dimmed when another category is
/// selected — and the amount and share for the tooltips. `Other` is
/// marked by [`PieSlice::is_other`].
#[derive(Clone, Debug)]
pub(in crate::ui) struct PieSlice {
    pub(in crate::ui) is_other: bool,
    pub(in crate::ui) name: SharedString,
    pub(in crate::ui) cents: i64,
    pub(in crate::ui) share: f64,
    pub(in crate::ui) color: Hsla,
}

/// The radii of a donut drawn in a box `side` px across: `(inner, outer)`,
/// the outer one clear of the hover lift, the inner 60 % of it.
pub(in crate::ui) fn donut_radii(side: f32) -> (f32, f32) {
    let outer = (side / 2. - DONUT_EDGE_MARGIN).max(0.);
    (outer * DONUT_INNER_FRACTION, outer)
}

/// Merge everything past the top [`PIE_TOP_N`] into one muted "Other"
/// slice (3.99): the rows arrive in draw order (largest first,
/// `Uncategorized` last) and only positive ones are here, so a tail of
/// `rows.len() − PIE_TOP_N` categories folds into the final slice. With
/// `PIE_TOP_N` or fewer rows nothing merges and the input comes back.
pub(in crate::ui) fn merge_top_slices(rows: &[PieSlice], muted: Hsla) -> Vec<PieSlice> {
    if rows.len() <= PIE_TOP_N {
        return rows.to_vec();
    }
    let mut slices = rows[..PIE_TOP_N].to_vec();
    let rest = &rows[PIE_TOP_N..];
    slices.push(PieSlice {
        is_other: true,
        name: OTHER_NAME.into(),
        cents: rest.iter().map(|s| s.cents).sum(),
        share: rest.iter().map(|s| s.share).sum(),
        color: muted,
    });
    slices
}

/// The donut/legend split of the measured content area (3.99): which
/// side the legend takes, how big the donut is, and what the legend may
/// wrap within. `donut == 0.` means the area is too small for a donut
/// (or unmeasured) and the panel shows the legend alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::ui) struct ChartLayout {
    pub(in crate::ui) side_by_side: bool,
    pub(in crate::ui) donut: f32,
    pub(in crate::ui) legend_w: f32,
    pub(in crate::ui) legend_lines: usize,
    pub(in crate::ui) legend_h: f32,
}

/// The donut's layout for a content area of `w` × `h` px (3.99, from
/// the 3.86 probe): wider than tall, the donut sits left as a square of
/// the area's height with the legend chips wrapping to its right;
/// otherwise the legend wraps under the donut, capped at
/// [`LEGEND_LINES`] lines. The donut never drops below [`DONUT_MIN`]
/// across — under that the legend takes the panel alone. `chip_widths`
/// are the [`legend_chip_w`] widths of the chips (the stacked legend's
/// height, and with it the donut's share of the column, follow how they
/// wrap); `line_h` is the measured `text_xs` line height.
///
/// An unmeasured area (either dimension `0.`) yields the legend alone
/// with its unmeasured plan — every chip shown — so the first painted
/// frame flashes nothing; the probe settles the plan one frame later.
pub(in crate::ui) fn chart_layout(w: f32, h: f32, chip_widths: &[f32], line_h: f32) -> ChartLayout {
    if w <= 0. || h <= 0. {
        return ChartLayout {
            side_by_side: false,
            donut: 0.,
            legend_w: w,
            legend_lines: LEGEND_LINES,
            legend_h: LEGEND_MAX_H,
        };
    }
    if w > h {
        let donut = if h >= DONUT_MIN { h } else { 0. };
        let legend_w = if donut > 0. {
            (w - donut - DONUT_GAP).max(1.)
        } else {
            w
        };
        // Beside the donut the legend gets as many lines as the area's
        // height fits (3.99), counting [`LEGEND_CHIP_H`] per line — the
        // chip's real line box, not the text's. The two-line cap is the
        // stacked form's.
        let legend_lines =
            (((h + LEGEND_GAP_Y) / (LEGEND_CHIP_H + LEGEND_GAP_Y)).floor() as usize).max(1);
        let legend_h = (legend_lines as f32 * (LEGEND_CHIP_H + LEGEND_GAP_Y) - LEGEND_GAP_Y).min(h);
        ChartLayout {
            side_by_side: true,
            donut,
            legend_w,
            legend_lines,
            legend_h,
        }
    } else {
        let lines = legend_lines(chip_widths, w, LEGEND_GAP_X).min(LEGEND_LINES);
        let legend_h = if lines <= 1 {
            line_h.min(h)
        } else {
            LEGEND_MAX_H.min(h)
        };
        let avail = (h - DONUT_GAP - legend_h).max(0.);
        let donut = w.min(avail);
        ChartLayout {
            side_by_side: false,
            donut: if donut >= DONUT_MIN { donut } else { 0. },
            legend_w: w,
            legend_lines: LEGEND_LINES,
            legend_h,
        }
    }
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
    /// The donut's content area measured the last probe (3.86, 3.99): the
    /// `chart:plot-area` container's own laid-out `(width, height)`,
    /// reported from its probe as the view tree paints. `0.` until the
    /// first frame has measured it, which the layout degrades safely
    /// (legend alone, every chip shown).
    pub(in crate::ui) fn chart_area(&self) -> (f32, f32) {
        (self.plot_w, self.plot_h)
    }
    /// Store a freshly measured content area (3.86, 3.99, from the probe
    /// on the `chart:plot-area` container); repaint for the new layout
    /// only when a dimension actually moved, so the measure-then-repaint
    /// loop settles.
    pub(in crate::ui) fn set_plot_size(&mut self, w: f32, h: f32, cx: &mut Context<Self>) {
        if (self.plot_w - w).abs() < 0.5 && (self.plot_h - h).abs() < 0.5 {
            return;
        }
        self.plot_w = w;
        self.plot_h = h;
        self.changed(DashboardChanges::MEASURED, cx);
    }
    /// The legend's rows (3.15, 3.31, 3.99): every `chart_rows` category
    /// with its palette colour, dimmed to about a third when another
    /// category is selected, and `Uncategorized` last whatever its
    /// amount. `colors` maps category ids to palette indices (3.85, the
    /// Catalog's map); `palette` is the current theme's 12 colours and
    /// `muted` paints the `Uncategorized` swatch.
    pub(in crate::ui) fn legend_rows(
        &self,
        colors: &HashMap<i64, usize>,
        palette: [Hsla; 12],
        muted: Hsla,
    ) -> Vec<ChartRow> {
        // 3.46: a selection keeps its bar and dims the others to about a
        // third rather than filtering them away, so the slices stay
        // comparable. `PieChart` has no per-slice alpha beyond the colour,
        // so the colour is dimmed before it goes in. Drilled into the
        // selected parent, every row is part of the selection, so nothing
        // dims there.
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
            .map(|r| {
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
                    cents: r.spend_cents,
                    share: r.share,
                    color: if dimmed { color.opacity(0.35) } else { color },
                }
            })
            .collect();
        // Uncategorized is always the last category row, whatever its amount.
        rows.sort_by_key(|r| r.id.is_none());
        rows
    }
    /// The donut's slices (3.99): the legend's rows with a net of more
    /// than zero — refunds net as today, and a category whose net is ≤ 0
    /// has no slice — merged down to [`PIE_TOP_N`] named slices plus a
    /// muted "Other". Draw order is legend order: largest first,
    /// `Uncategorized` last, `Other` last; `PieChart` sweeps them
    /// clockwise from 12 o'clock.
    pub(in crate::ui) fn chart_slices(
        &self,
        colors: &HashMap<i64, usize>,
        palette: [Hsla; 12],
        muted: Hsla,
    ) -> Vec<PieSlice> {
        let positive: Vec<PieSlice> = self
            .legend_rows(colors, palette, muted)
            .into_iter()
            .filter(|r| r.cents > 0)
            .map(|r| PieSlice {
                is_other: false,
                name: r.name,
                cents: r.cents,
                share: r.share,
                color: r.color,
            })
            .collect();
        let mut merged = merge_top_slices(&positive, muted);
        // 3.99: `Other` is never the selection, so it dims with the
        // rest of the unselected categories; drilled into the selected
        // parent, every slice is part of the selection and none dims.
        if let Some(other) = merged.last_mut().filter(|s| s.is_other) {
            let selected = match &self.filter.legend_selection {
                Some(LegendSelection::Category(id)) => Some(*id),
                _ => None,
            };
            let unc_active = matches!(
                self.filter.legend_selection,
                Some(LegendSelection::Uncategorized)
            ) || self.filter.uncategorized_only;
            let drilled_into_selection = selected.is_some() && self.filter.chart_parent == selected;
            if (selected.is_some() || unc_active) && !drilled_into_selection {
                other.color = muted.opacity(0.35);
            }
        }
        merged
    }
    /// The total painted in the donut's hole (3.99): the sum of the
    /// range's (or the drilled parent's) `chart_rows` — the same total
    /// the shares are taken over.
    pub(in crate::ui) fn donut_total(&self) -> i64 {
        self.chart_rows.iter().map(|r| r.spend_cents).sum()
    }
    /// The chart's row (3.5, 3.45, donut since 3.99): the category donut
    /// with its total in the hole, a clickable chip legend (3.36), and
    /// the 3.100 dock giving it the panel's size. `PieChart` takes no
    /// clicks, so all interactivity lives in the legend chips
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
        let rows = self.legend_rows(&colors, palette, muted);
        let slices = self.chart_slices(&colors, palette, muted);
        let selection = self.filter.legend_selection;
        // The `Uncategorized` row is active when the 3.4 checkbox drives the
        // same clause, so the legend never contradicts the toolbar.
        let uncategorized_active = matches!(selection, Some(LegendSelection::Uncategorized))
            || self.filter.uncategorized_only;
        // The parent suffix of the tooltip text (" of Food" once drilled
        // in); empty at the top level. The bare name labels the hole.
        let drilled_name = self
            .filter
            .chart_parent
            .and_then(|p| self.categories.iter().find(|n| n.id == p))
            .map(|n| n.name.clone());
        let parent_name = drilled_name
            .as_ref()
            .map(|n| format!(" of {n}"))
            .unwrap_or_default();
        let bar_parent_name = parent_name.clone();

        // 3.99: the donut and legend are laid out from the measured
        // content area (3.86): side-by-side when it's wider than tall,
        // stacked otherwise, legend alone under the 120 px floor.
        let (area_w, area_h) = self.chart_area();
        let char_xs = self.chart_char_px * 12. / 10.;
        let drilled = self.filter.chart_parent.is_some();
        // 3.99: the merged "Other" slice and the names its tooltip lists.
        let positive: Vec<&str> = rows
            .iter()
            .filter(|r| r.cents > 0)
            .map(|r| r.name.as_str())
            .collect();
        let has_other = positive.len() > PIE_TOP_N;
        let other_members = positive.get(PIE_TOP_N..).unwrap_or(&[]).join(", ");
        let other = slices.iter().find(|s| s.is_other);

        let mut widths: Vec<f32> = Vec::with_capacity(rows.len() + 2);
        widths.extend(
            rows.iter()
                .map(|r| legend_chip_w(r.name.chars().count(), char_xs)),
        );
        if has_other {
            widths.push(legend_chip_w(OTHER_NAME.chars().count(), char_xs));
        }
        let layout = chart_layout(area_w, area_h, &widths, self.chart_label_h);
        // The chip plan spans the legend's laid-out width; the "← All
        // categories" chip leads the row while drilled.
        if drilled {
            widths.insert(0, legend_all_w(char_xs));
        }
        let shown = legend_visible(
            &widths,
            layout.legend_w,
            LEGEND_GAP_X,
            layout.legend_lines,
            char_xs,
        )
        .unwrap_or(widths.len());
        let hidden = widths.len() - shown;
        // The names the "+N more" tooltip lists (3.36), the "Other" chip
        // among them (3.99).
        let hidden_names: String = rows
            .iter()
            .map(|r| r.name.as_str())
            .chain(has_other.then_some(OTHER_NAME))
            .enumerate()
            .filter(|(ix, _)| ix + drilled as usize >= shown)
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
            .join(", ");

        let donut = if rows.is_empty() || layout.donut < DONUT_MIN || slices.is_empty() {
            None
        } else {
            let side = layout.donut;
            let (inner, outer) = donut_radii(side);
            let amount = flow::money(self.donut_total());
            let hole_sub = drilled_name.clone().unwrap_or_else(|| "Expense".into());
            Some(
                div()
                    .flex_none()
                    .size(px(side))
                    .relative()
                    .debug_selector(|| "chart:plot".into())
                    .child(
                        div().absolute().inset_0().child(
                            PieChart::new(slices.clone())
                                .id("category-chart")
                                .value(|s| s.cents as f32)
                                .color(|s| s.color)
                                .inner_radius(inner)
                                .outer_radius(outer)
                                .tooltip_name(|s| s.name.clone())
                                .tooltip_value(move |s, _, _| {
                                    chart_tooltip_value(s.cents, s.share, &bar_parent_name).into()
                                }),
                        ),
                    )
                    // The range's total spend sits in the hole (3.99):
                    // the amount over a muted "Expense", the drilled
                    // parent's name over its total while drilled. It
                    // takes no clicks; the ring's hover and the legend
                    // chips stay live.
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_0p5()
                            .debug_selector({
                                let amount = amount.clone();
                                move || format!("chart:donut-center:{amount}")
                            })
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(amount.clone()),
                            )
                            .child(div().text_xs().text_color(muted).child(hole_sub.clone())),
                    ),
            )
        };

        // 3.36: the legend is a wrapping chip row capped at
        // `layout.legend_lines` lines, the categories past them hidden
        // behind a "+N more" chip. The plan's available width is the
        // legend's own laid-out width from the layout; the chip
        // character estimate is the measured `M` advance scaled from the
        // chart's 10 px font to the chips' 12 px `text_xs`.
        let legend = div()
            .id("legend-list")
            .debug_selector(|| "chart:legend".into())
            .when(!layout.side_by_side, |list| list.w_full().flex_none())
            .when(layout.side_by_side, |list| list.flex_1().min_w_0())
            .flex()
            .flex_wrap()
            .content_start()
            .gap_x(px(LEGEND_GAP_X))
            .gap_y(px(LEGEND_GAP_Y))
            // 3.99: the chip plan already keeps the stacked legend to
            // two lines, and the donut takes only the height left over;
            // capping the box (or letting the donut crowd it out) would
            // clip the chips' hit area and swallow their clicks.
            .when(layout.side_by_side, |list| {
                list.max_h(px(layout.legend_h)).overflow_hidden()
            })
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
                // 3.36: chips past the planned lines are not built at
                // all — a clipped-but-laid-out chip would still answer
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
                // the hover tooltip, in the slice's own tooltip text.
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
            // 3.99: "Other" is a legend chip too — a muted swatch and the
            // merged amount — but it is not a click target: its tooltip
            // lists the categories the slice holds.
            .when(
                has_other && (rows.len() + drilled as usize) < shown,
                |list| {
                    let (cents, share) = other.map_or((0, 0.), |s| (s.cents, s.share));
                    let value = chart_tooltip_value(cents, share, &parent_name);
                    list.child(
                        div()
                            .id("legend-other")
                            .debug_selector(|| "chart:legend-other".into())
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .px_1()
                            .rounded(radius)
                            .child(
                                div()
                                    .size(px(10.))
                                    .flex_none()
                                    .rounded(px(2.))
                                    .bg(other.map_or(muted, |s| s.color)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .debug_selector(|| format!("chart:legend-text:{OTHER_NAME}"))
                                    .child(OTHER_NAME),
                            )
                            .tooltip(move |window, cx| {
                                let (value, members) = (value.clone(), other_members.clone());
                                Tooltip::element(move |_, _| {
                                    let (value, members) = (value.clone(), members.clone());
                                    v_flex()
                                        .gap_0p5()
                                        .debug_selector({
                                            let members = members.clone();
                                            move || format!("chart:legend-other-tooltip:{members}")
                                        })
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(OTHER_NAME),
                                        )
                                        .child(value)
                                        .child(members)
                                })
                                .build(window, cx)
                            }),
                    )
                },
            )
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
        } else if let Some(donut) = donut {
            // 3.99: wider than tall, the donut sits left and the chips
            // wrap to its right; otherwise the legend wraps under it,
            // the donut centred over it.
            if layout.side_by_side {
                div()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .items_start()
                    .gap(px(DONUT_GAP))
                    .child(donut)
                    .child(legend)
                    .into_any_element()
            } else {
                v_flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .gap(px(DONUT_GAP))
                    .child(div().w_full().flex().justify_center().child(donut))
                    .child(legend)
                    .into_any_element()
            }
        } else {
            // Too small for a donut (3.99): the legend takes the panel.
            div()
                .size_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .items_start()
                .child(legend)
                .into_any_element()
        };

        // 3.100: the dock owns the panel box, the title (including the
        // selected-month suffix, 3.46 — the dock paints it) and the size;
        // the panel content is the padded column below the title row.
        // 3.99: the column is the probe's containing block — the donut
        // and legend plan is computed from the area this column gets.
        v_flex()
            .size_full()
            .min_w_0()
            .p_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .relative()
                    .debug_selector(|| "chart:plot-area".into())
                    .child(area_probe(cx.entity().downgrade(), |this, w, h, cx| {
                        this.set_plot_size(w, h, cx)
                    }))
                    .child(chart),
            )
            .into_any_element()
    }
}

/// The content area's `(width, height)` probe (3.86, 3.99): like
/// [`bounds_probe`] but reporting both dimensions — the donut's layout
/// needs the area's aspect, not just its width. Absolutely positioned,
/// it takes no part in layout and reports in `prepaint` through
/// `window.defer`; `set` must store and notify only on real movement.
fn area_probe(
    observee: WeakEntity<DashboardState>,
    set: impl Fn(&mut DashboardState, f32, f32, &mut Context<DashboardState>) + 'static,
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
            window.defer(cx, move |_, cx| {
                observee.update(cx, |this, cx| set(this, w, h, cx)).ok();
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// The category donut chart and its legend, with `panel:chart`'s probes
/// and selectors (3.97): a dock-shaped entity (3.100) that owns nothing
/// but its handle on [`DashboardState`] and a subscription narrowed to
/// the changes it paints from — a repaint of this panel is a `cx.notify()`
/// on this entity alone, and its paint reads the state back through
/// [`DashboardState::chart_panel`].
pub(in crate::ui::dashboard) struct ChartPanel {
    /// The shared state this panel reads and writes selections through.
    state: Entity<DashboardState>,
    /// The state subscription; lives as long as the panel.
    _subs: Subscription,
    /// Dock focus handle (3.100: `Panel`'s `Focusable` bound).
    focus: FocusHandle,
}

impl ChartPanel {
    pub(super) fn new(state: Entity<DashboardState>, cx: &mut Context<Self>) -> Self {
        const MASK: DashboardChanges = DashboardChanges::ALL;
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

impl Render for ChartPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state.update(cx, |state, cx| state.chart_panel(cx))
    }
}

/// Dock plumbing (3.100): the focus handle and the `Panel` identity. The
/// four panels are the dock's fixed set, so none of them is closable or
/// zoomable — the grid is only ever moved and resized.
impl Focusable for ChartPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<PanelEvent> for ChartPanel {}

impl Panel for ChartPanel {
    fn panel_name(&self) -> &'static str {
        "chart"
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> bool {
        false
    }
}
