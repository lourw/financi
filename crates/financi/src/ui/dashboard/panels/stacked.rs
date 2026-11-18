//! The monthly stacked bars panel (3.93, its own entity since 3.97):
//! the month stack shapes, their paint math and the `StackedPanel`
//! entity. Split out of `ui/dashboard_chart.rs`; the render methods
//! stay on [`DashboardState`] (see [`super::chart`]'s header).

use super::chart::*;
use super::*;
use crate::ui::widgets::*;
use gpui_kit::base::dock::{Panel, PanelEvent};

// --------------------------------------------------- 3.93 monthly stacks

/// One month of the *Monthly spending by category* stacked bar chart
/// (3.93): the month key (`YYYY-MM`), the bar's height in positive cents
/// (the `Spend` row, or the drilled-in parent's row while drilled) and
/// its segments bottom-up.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MonthStack {
    pub(crate) month: SharedString,
    pub(crate) total: i64,
    pub(crate) segments: Vec<StackSegment>,
}

/// One segment of a [`MonthStack`]: the category (`None` — the
/// `Uncategorized` pseudo-row), its name, and its spend for that month
/// in positive cents.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StackSegment {
    pub(crate) id: Option<i64>,
    pub(crate) name: SharedString,
    pub(crate) cents: i64,
}

/// Shape [`MonthlySummary`](financi_db::MonthlySummary) into the stacked
/// bars (3.93) — GPUI-free so the rules are unit-testable:
///
/// - one bar per month, in the summary's month order;
/// - top level: one segment per spend parent row (the `Transfers` row
///   included exactly when the summary carries it, i.e. when the filter
///   includes transfers), plus `Uncategorized`; the `Income`, `Spend`
///   and `Net` rows and every child row are left out;
/// - drilled into a parent: the segments are that parent's children and
///   the bar height is the parent's own row;
/// - `total` is the month's `Spend` row (or the parent's row when
///   drilled), clamped to zero, so the bar's segments sum to the bar;
/// - the segment order is the summary's row order (largest total over
///   the range at the bottom, `Uncategorized` last) and is identical in
///   every bar, so a category keeps its place and color month to month.
///
/// `categories` is the category tree: the summary's rows are parents
/// each followed by their children, and only the tree says which is
/// which (the same lookup as [`DashboardState::parent_of`]).
pub(crate) fn monthly_stacks(
    summary: &MonthlySummary,
    drill: Option<i64>,
    categories: &[CategoryNode],
) -> Vec<MonthStack> {
    // The query's last three rows are always Income, Spend and Net
    // (`summary_rows` in `financi-db`); above them sit the spend
    // parents, each followed by its own children.
    let n = summary.rows.len();
    if n < 3 {
        return Vec::new();
    }
    let category_rows = &summary.rows[..n - 3];
    let parent_of = |id: i64| {
        categories
            .iter()
            .find(|p| p.children.iter().any(|c| c.id == id))
            .map(|p| p.id)
    };
    // Which summary rows the segments are drawn from, in bar order.
    let seg_rows: Vec<&MonthlySummaryRow> = match drill {
        None => {
            let mut rows: Vec<&MonthlySummaryRow> = category_rows
                .iter()
                .filter(|r| match r.id {
                    None => true, // the `Uncategorized` pseudo-row
                    Some(id) => parent_of(id).is_none(),
                })
                .collect();
            // `Uncategorized` is always the last category, whatever its
            // amount (ui-conventions *Categories*), like the legend and
            // the summary grid.
            rows.sort_by_key(|r| r.id.is_none());
            rows
        }
        Some(parent) => {
            let child_ids: Vec<i64> = categories
                .iter()
                .find(|p| p.id == parent)
                .map(|p| p.children.iter().map(|c| c.id).collect())
                .unwrap_or_default();
            category_rows
                .iter()
                .filter(|r| r.id.is_some_and(|id| child_ids.contains(&id)))
                .collect()
        }
    };
    // The row whose month values are the bar height: the `Spend` row at
    // the top level, the drilled parent's own row when drilled (which
    // can be absent if the parent has no spend in range — then every
    // bar is an empty slot).
    let height_row = match drill {
        None => Some(&summary.rows[n - 2]),
        Some(parent) => category_rows.iter().find(|r| r.id == Some(parent)),
    };
    summary
        .months
        .iter()
        .enumerate()
        .map(|(ix, month)| {
            let total = height_row
                .and_then(|r| r.months.get(ix))
                .copied()
                .unwrap_or(0)
                .max(0);
            let segments = seg_rows
                .iter()
                .filter_map(|r| {
                    let cents = r.months.get(ix).copied().unwrap_or(0);
                    (cents > 0).then(|| StackSegment {
                        id: r.id,
                        name: SharedString::from(r.name.as_str()),
                        cents,
                    })
                })
                .collect();
            MonthStack {
                month: SharedString::from(month.as_str()),
                total,
                segments,
            }
        })
        .collect()
}

/// A left-axis gridline amount (3.93), compact so the gutter stays
/// narrow: `$1.2k` from $1,000 up (one decimal, dropped when whole),
/// whole dollars below (`$86`), and cents only under a dollar (`$0.42`).
pub(crate) fn axis_amount(cents: i64) -> String {
    if cents >= 100_000 {
        let s = format!("{:.1}", cents as f64 / 100_000.);
        format!("${}k", s.strip_suffix(".0").unwrap_or(&s))
    } else if cents >= 100 {
        format!("${}", cents.div_euclid(100))
    } else {
        format!("${:.2}", cents as f64 / 100.)
    }
}

/// Smallest width a month's bar may shrink to before the stacked bars'
/// strip scrolls sideways (3.93, ui-conventions *Dashboard*).
pub(in crate::ui) const STACKED_MIN_BAR_W: f32 = 12.;
/// Width of the stacked chart's axis gutter (3.93). The chip legend spans
/// the panel's inner width — gutter + row gap + plot — so its fit plan
/// measures `plot + STACKED_LEGEND_EXTRA_W`, not the plot alone.
const STACKED_GUTTER_W: f32 = 48.;
/// The row gap between the stacked chart's gutter and plot: the row's
/// `gap_2` (3.93).
const STACKED_ROW_GAP: f32 = 8.;
/// The legend's extra width over the plot: the gutter plus the row's
/// `gap_2` (3.93).
const STACKED_LEGEND_EXTRA_W: f32 = STACKED_GUTTER_W + STACKED_ROW_GAP;
// Pin the 56 px the legend's fit plan has measured with since 3.93, so
// naming the gap literal could not have shifted it.
const _: () = assert!(STACKED_LEGEND_EXTRA_W == 56.);

/// One painted segment of the stacked bars (3.93): the shaping's cents,
/// the resolved colour (dimmed when the selection says so), and the two
/// tooltip lines — `Shopping · Oct 2026` over `$412.30 · 22.4%`.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::ui) struct StackSegPaint {
    pub(in crate::ui) id: Option<i64>,
    pub(in crate::ui) name: SharedString,
    pub(in crate::ui) cents: i64,
    pub(in crate::ui) color: Hsla,
    pub(in crate::ui) tooltip_title: SharedString,
    pub(in crate::ui) tooltip_value: SharedString,
}

/// One painted month column (3.93): the display label (`Oct 2026`),
/// the bar's total, whether the month is the selected one (the
/// `list_hover` column tint), whether its label survives the thinning
/// plan, and its segments bottom-up.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::ui) struct StackColPaint {
    pub(in crate::ui) month: SharedString,
    pub(in crate::ui) total: i64,
    pub(in crate::ui) month_selected: bool,
    pub(in crate::ui) show_label: bool,
    pub(in crate::ui) segments: Vec<StackSegPaint>,
}

impl DashboardState {
    /// Store the stacked bars' measured plot viewport width (3.93, from
    /// the probe on `stacked:plot`); it only drives month-label thinning.
    /// Notify-on-change only, like [`Self::set_plot_width`].
    pub(in crate::ui) fn set_stacks_plot_width(&mut self, w: f32, cx: &mut Context<Self>) {
        if (self.stacks_plot_w - w).abs() < 0.5 {
            return;
        }
        self.stacks_plot_w = w;
        self.changed(DashboardChanges::MEASURED, cx);
    }
    /// The stacked bars' shaped months (3.93), from the cached summary —
    /// rebuilt when the summary, the tree or the chart's drill level
    /// moved, not per frame (the 3.85 cache pattern).
    pub(in crate::ui) fn stacks_mut(&mut self) -> Rc<Vec<MonthStack>> {
        if self.stacks_dirty || self.stacks_drill != self.filter.chart_parent {
            self.stacks = Rc::new(monthly_stacks(
                &self.summary,
                self.filter.chart_parent,
                &self.categories,
            ));
            self.stacks_dirty = false;
            self.stacks_drill = self.filter.chart_parent;
        }
        self.stacks.clone()
    }
    /// How many `Oct 2026` month labels fit across the stacked bars'
    /// measured plot (3.93). The drawn ones spread evenly first-to-last
    /// through [`bar_label_slots`], so they never collide; below one the
    /// plan still paints the first month. 3.98: each shown label's box
    /// is [`Self::stacked_label_w`] wide, centred on its column, so the
    /// plan also reserves the overhang the first and last labels lose
    /// when their box clamps against the strip's edge.
    pub(in crate::ui) fn month_label_plan(&self, count: usize) -> usize {
        if count <= 1 {
            return count;
        }
        let label_w = self.stacked_label_w();
        let strip_w = self.stacked_strip_w(count);
        let col_w = self.stacked_col_w(count);
        // A column narrower than the label budget shifts its edge
        // label's box by up to half the overhang to stay in the plot;
        // every gap must clear that on top of the label width.
        let shift = ((label_w - col_w) / 2.).max(0.);
        let need = label_w + shift;
        let band_w = strip_w / count as f32;
        let step = (need / band_w.max(1.)).ceil().max(1.) as usize;
        (count - 1) / step + 1
    }
    /// 3.98: the width of one stacked month label's box: the budget
    /// `Oct 2026` (8 characters at the measured `text_xs` `M` width,
    /// 3.86) plus the label gap.
    pub(in crate::ui) fn stacked_label_w(&self) -> f32 {
        let char_xs = self.chart_char_px * 12. / 10.;
        8. * char_xs + CHART_LABEL_GAP
    }
    /// 3.98: the strip's laid-out width — its viewport once every column
    /// fits, else the `min_w` floor the strip paints at while scrolling.
    fn stacked_strip_w(&self, count: usize) -> f32 {
        self.stacks_plot_w
            .max(count as f32 * (STACKED_MIN_BAR_W + STACKED_BAR_GAP))
    }
    /// 3.98: one month column's laid-out width inside the strip.
    fn stacked_col_w(&self, count: usize) -> f32 {
        ((self.stacked_strip_w(count) - (count.saturating_sub(1)) as f32 * STACKED_BAR_GAP)
            / count.max(1) as f32)
            .max(STACKED_MIN_BAR_W)
    }
    /// 3.98: the strip-coordinate left edge of a shown label's box:
    /// centred on its column and clamped so the box stays inside the
    /// strip — the first and last labels are never cut off at the plot
    /// edge.
    fn stacked_label_left(&self, count: usize, ix: usize) -> f32 {
        let label_w = self.stacked_label_w();
        let centre = self.stacked_col_w(count) / 2.
            + ix as f32 * (self.stacked_col_w(count) + STACKED_BAR_GAP);
        (centre - label_w / 2.).clamp(0., (self.stacked_strip_w(count) - label_w).max(0.))
    }
    /// 3.98: what the value axis (gridlines and gutter) insets at its
    /// bottom so its baseline lands on the bars' baseline: the measured
    /// label row plus the `gap_0p5` above it.
    pub(in crate::ui) fn stacked_bar_inset_h(&self) -> f32 {
        self.chart_label_h + STACKED_LABEL_GAP
    }
    /// The stacked columns prepared for drawing (3.93): palette colours
    /// per segment, dimmed to a third when another category holds the
    /// selection — the chart's 3.46 rule, including "drilled into the
    /// selection dims nothing" — the month tint flag, the label plan and
    /// both tooltip lines. Kept free of widgets so the tests can read
    /// the alpha without pixels.
    pub(in crate::ui) fn stacked_paint(
        &self,
        stacks: &[MonthStack],
        colors: &HashMap<i64, usize>,
        palette: [Hsla; 12],
        muted: Hsla,
    ) -> Vec<StackColPaint> {
        let selected = match &self.filter.legend_selection {
            Some(LegendSelection::Category(id)) => Some(*id),
            _ => None,
        };
        let drilled_into_selection = selected.is_some() && self.filter.chart_parent == selected;
        let unc_active = matches!(
            self.filter.legend_selection,
            Some(LegendSelection::Uncategorized)
        ) || self.filter.uncategorized_only;
        let slots = bar_label_slots(stacks.len(), self.month_label_plan(stacks.len()));
        stacks
            .iter()
            .zip(slots)
            .map(|(s, show_label)| {
                StackColPaint {
                    month: flow::month_label(&s.month).into(),
                    total: s.total,
                    month_selected: self.filter.summary_month.as_deref() == Some(s.month.as_str()),
                    show_label,
                    segments: s
                        .segments
                        .iter()
                        .map(|seg| {
                            let color = seg.id.map_or(muted, |id| {
                                colors.get(&id).map(|ix| palette[ix % 12]).unwrap_or(muted)
                            });
                            // Same rule as `chart_bars`: the
                            // `Uncategorized` pseudo-segment dims for any
                            // real-category selection unless the
                            // "Uncategorized only" filter is on.
                            let dimmed = match seg.id {
                                Some(id) => {
                                    (selected.is_some() || unc_active)
                                        && !drilled_into_selection
                                        && selected != Some(id)
                                }
                                None => selected.is_some() && !unc_active,
                            };
                            let share = if s.total > 0 {
                                seg.cents as f64 / s.total as f64
                            } else {
                                0.
                            };
                            StackSegPaint {
                                id: seg.id,
                                name: seg.name.clone(),
                                cents: seg.cents,
                                color: if dimmed { color.opacity(0.35) } else { color },
                                tooltip_title: format!(
                                    "{} · {}",
                                    seg.name,
                                    flow::month_label(&s.month)
                                )
                                .into(),
                                tooltip_value: chart_tooltip_value(seg.cents, share, "").into(),
                            }
                        })
                        .collect(),
                }
            })
            .collect()
    }
    /// The 3.93 panel: *Monthly spending by category* — the summary's
    /// monthly figures as stacked bars, one segment per top-level parent
    /// (the drilled-in parent's children when drilled), full width and
    /// 340 px tall between the chart and the summary. A segment click is
    /// a summary-cell click (category + month); the month label under a
    /// bar selects the month alone; the 3.36-style chip legend under the
    /// plot selects categories. Past a 12 px bar floor the month strip
    /// scrolls sideways, parked at the latest month.
    pub(in crate::ui) fn stacked_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, list_active, radius, border, colors, palette) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.list_active,
                theme.radius,
                theme.border,
                self.catalog.read(cx).cat_colors(),
                category_palette(theme),
            )
        };
        let stacks = self.stacks_mut();
        let paint = self.stacked_paint(&stacks, &colors, palette, muted);
        // 3.100: the dock owns the panel box, the title and the size;
        // both states paint the padded column below the title row.
        let panel_shell = || v_flex().size_full().min_w_0().p_4().gap_2();
        // *States*: no spend in the range is the empty state, whatever
        // the shape says (income-only months stack no bars either).
        if !self.summary_has_data() || !stacks.iter().any(|s| s.total > 0) {
            return panel_shell()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(muted)
                        .debug_selector(|| "stacked:empty".into())
                        .child("No spending in this range"),
                )
                .into_any_element();
        }
        // The view model with its click handlers: a segment is a summary
        // cell (its category, its month), a month label is the month
        // alone — the same shared selection state the grid uses.
        let columns: Vec<StackColView> = paint
            .iter()
            .enumerate()
            .zip(stacks.iter())
            .map(|((ix, col), stack)| {
                let segments = col
                    .segments
                    .iter()
                    .map(|seg| {
                        let id = seg.id;
                        let month = stack.month.to_string();
                        StackSegView {
                            cents: seg.cents,
                            color: seg.color,
                            tooltip_title: seg.tooltip_title.clone(),
                            tooltip_value: seg.tooltip_value.clone(),
                            on_click: Rc::new(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    this.click_summary_cell(id, Some(month.clone()), window, cx);
                                },
                            ))
                                as Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
                        }
                    })
                    .collect();
                let month = stack.month.to_string();
                StackColView {
                    month: col.month.clone(),
                    total: col.total,
                    month_selected: col.month_selected,
                    // 3.98: the label box's strip-coordinate left edge,
                    // centred on this column and clamped inside the plot.
                    label_left: col
                        .show_label
                        .then(|| self.stacked_label_left(paint.len(), ix)),
                    segments,
                    on_label_click: Rc::new(cx.listener(
                        move |this, _: &ClickEvent, _window, cx| {
                            this.click_summary_month(&month, cx);
                        },
                    ))
                        as Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
                }
            })
            .collect();
        // Axis gutter: the top gridline names the tallest bar, the
        // middle its half, and the baseline is zero — compact amounts
        // (`$1.2k`) so the gutter stays narrow. The bottom padding keeps
        // `$0` on the bars' baseline, not the panel's (the bars stop
        // above the month-label row).
        let max_total = stacks.iter().map(|s| s.total).max().unwrap_or(0).max(1);
        // 3.98: the inset follows the measured label row height, so the
        // axis keeps sitting on the bars' baseline.
        let inset = self.stacked_bar_inset_h();
        let gutter = div()
            .flex_none()
            .w(px(STACKED_GUTTER_W))
            .min_h_0()
            .flex()
            .flex_col()
            .justify_between()
            .pb(px(inset))
            .text_xs()
            .text_right()
            .text_color(muted)
            .debug_selector(|| "stacked:axis".into())
            .child(div().child(SharedString::from(axis_amount(max_total))))
            .child(div().child(SharedString::from(axis_amount(max_total / 2))))
            .child(div().child("$0"));
        // Gridlines behind the bars: three hairlines spread
        // top-to-bottom, aligned with the gutter's three amounts. The
        // bottom inset matches the bars' area, so the `$0` hairline is
        // their baseline (3.93 review).
        let grid_line = || {
            div()
                .w_full()
                .flex_none()
                .h(px(1.))
                .bg(border)
                .into_any_element()
        };
        let grid = div()
            .absolute()
            .inset_0()
            .bottom(px(inset))
            .flex()
            .flex_col()
            .justify_between()
            .debug_selector(|| "stacked:grid".into())
            .child(grid_line())
            .child(grid_line())
            .child(grid_line());
        let plot = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .debug_selector(|| "stacked:plot".into())
            .child(bounds_probe(cx.entity().downgrade(), |this, w, cx| {
                this.set_stacks_plot_width(w, cx)
            }))
            .child(grid)
            .child(
                div()
                    .id("stacked-scroll")
                    .absolute()
                    .inset_0()
                    .overflow_x_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.stacks_scroll)
                    .debug_selector(|| "stacked:scroll".into())
                    .child(StackedBarChart {
                        columns: Rc::new(columns),
                        min_bar_w: STACKED_MIN_BAR_W,
                        label_w: self.stacked_label_w(),
                        label_h: self.chart_label_h,
                    }),
            );
        // The chip legend (3.36): one chip per segment, first-seen order
        // across the months, the chip's amount its total over the range.
        // Selecting a chip selects the category alone (a summary cell
        // with no month) — this chart is not the drill's front door.
        let selection = self.filter.legend_selection;
        let uncategorized_active = matches!(selection, Some(LegendSelection::Uncategorized))
            || self.filter.uncategorized_only;
        let mut legend: Vec<(Option<i64>, SharedString, Hsla, i64)> = Vec::new();
        let mut grand: i64 = 0;
        for col in &paint {
            for seg in &col.segments {
                grand += seg.cents;
                match legend.iter_mut().find(|(id, _, _, _)| *id == seg.id) {
                    Some(entry) => entry.3 += seg.cents,
                    None => legend.push((seg.id, seg.name.clone(), seg.color, seg.cents)),
                }
            }
        }
        let char_xs = self.chart_char_px * 12. / 10.;
        let widths: Vec<f32> = legend
            .iter()
            .map(|(_, name, _, _)| legend_chip_w(name.chars().count(), char_xs))
            .collect();
        let shown = legend_visible(
            &widths,
            self.stacks_plot_w + STACKED_LEGEND_EXTRA_W,
            LEGEND_GAP_X,
            LEGEND_LINES,
            char_xs,
        )
        .unwrap_or(widths.len());
        let hidden = widths.len() - shown;
        let hidden_names: String = legend
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix >= shown)
            .map(|(_, (_, name, _, _))| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let legend_list = div()
            .id("stacked-legend-list")
            .debug_selector(|| "stacked:legend".into())
            .w_full()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap_x(px(LEGEND_GAP_X))
            .gap_y(px(LEGEND_GAP_Y))
            .max_h(px(LEGEND_MAX_H))
            .overflow_hidden()
            .children(
                legend
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, (id, name, color, total))| {
                        if ix >= shown {
                            return None;
                        }
                        let id = *id;
                        let selected = match (id, selection) {
                            (None, _) => uncategorized_active,
                            (Some(id), Some(LegendSelection::Category(c))) => c == id,
                            _ => false,
                        };
                        let share = if grand > 0 {
                            *total as f64 / grand as f64
                        } else {
                            0.
                        };
                        let value = chart_tooltip_value(*total, share, "");
                        let row_selector = format!(
                            "stacked:legend-row:{ix}{}",
                            if selected { ":active" } else { "" }
                        );
                        Some(
                            div()
                                .id(SharedString::from(format!("stacked-legend-{ix}")))
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
                                    this.click_summary_cell(id, None, window, cx);
                                }))
                                .child(div().size(px(10.)).flex_none().rounded(px(2.)).bg(*color))
                                .child(
                                    div()
                                        .text_xs()
                                        .max_w(px(LEGEND_CHIP_NAME_MAX_W))
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .debug_selector({
                                            let name = name.clone();
                                            move || format!("stacked:legend-text:{ix}:{name}")
                                        })
                                        .when(id.is_none(), |name| name.text_color(muted))
                                        .child(name.clone()),
                                )
                                .tooltip({
                                    let (name, value) = (name.clone(), value);
                                    move |window, cx| {
                                        let (name, value) = (name.clone(), value.clone());
                                        Tooltip::element(move |_, _| {
                                            v_flex()
                                                .gap_0p5()
                                                .debug_selector({
                                                    let name = name.clone();
                                                    move || {
                                                        format!(
                                                            "stacked:legend-tooltip:{ix}:{name}"
                                                        )
                                                    }
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
                    }),
            )
            .when(hidden > 0, |list| {
                list.child(
                    div()
                        .id("stacked-legend-more")
                        .debug_selector(move || format!("stacked:legend-more:{hidden}"))
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
                                            format!("stacked:legend-more-tooltip:{hidden}:{names}")
                                        }
                                    })
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(names))
                            })
                            .build(window, cx)
                        }),
                )
            });
        panel_shell()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(gutter)
                    .child(plot),
            )
            .child(legend_list)
            .into_any_element()
    }
}

/// The monthly stacked bars (3.97): a dock-shaped entity (3.100) that owns nothing but
/// its handle on [`DashboardState`] and a subscription narrowed to the
/// changes it paints from — a repaint of this panel is a `cx.notify()`
/// on this entity alone, and its paint reads the state back through
/// [`DashboardState::stacked_panel`].
pub(in crate::ui::dashboard) struct StackedPanel {
    /// The shared state this panel reads and writes selections through.
    state: Entity<DashboardState>,
    /// The state subscription; lives as long as the panel.
    _subs: Subscription,
    /// Dock focus handle (3.100: `Panel`'s `Focusable` bound).
    focus: FocusHandle,
}

impl StackedPanel {
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

impl Render for StackedPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.state.update(cx, |state, cx| state.stacked_panel(cx))
    }
}

/// Dock plumbing (3.100): the focus handle and the `Panel` identity. The
/// four panels are the dock's fixed set, so none of them is closable or
/// zoomable — the grid is only ever moved and resized.
impl Focusable for StackedPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<PanelEvent> for StackedPanel {}

impl Panel for StackedPanel {
    fn panel_name(&self) -> &'static str {
        "stacked"
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> bool {
        false
    }
}
