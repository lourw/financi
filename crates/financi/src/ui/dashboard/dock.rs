//! The Dashboard dock (3.100): the 2×2 grid of resizable, movable panels.
//!
//! The page body below the pinned filter row is a [`DockArea`] (gpui-base's
//! dock) holding the four panels — chart, monthly bars, summary, table —
//! in a 2×2 split by default. This module supplies the *skin* (the
//! [`DockAreaRenderer`] / [`TabGroupRenderer`] pair the dock paints itself
//! from), the default layout, and the min-size clamp.
//!
//! The skin is hand-written against the base traits rather than built on
//! `gpui_component`'s `TabPanelRenderer`: the component chrome deviates from
//! `docs/ui-conventions.md` (it brings its own padding and typography, and
//! its group border does not match the `panel()` look every other page
//! uses). The base traits carry no appearance of their own, so the skin is
//! where the Panel look lives:
//!
//! - one bordered, rounded box per group — the title row draws the top half
//!   (rounded top, no bottom border) and the content frame the bottom half;
//! - the title row doubles as the drag handle (`cursor_grab`): dragging it
//!   moves the panel, dropping it on another group's title or tab merges
//!   the two into a tab group;
//! - 12 px gutters: every group frame carries a 6 px ring, so adjacent
//!   groups leave 12 px between their borders (the dock container undoes 6
//!   px at the page edge so the page keeps its 16 px padding);
//! - dividers: the base hit band (transparent, inside the gutter) plus a
//!   2 px `border`-coloured line while hovered or dragged;
//! - min sizes: [`PANEL_MIN`] per panel (320×200 for the table, 240×160 for
//!   the rest) enforced on the group frame and — because base's divider
//!   floor is its own 100 px — re-applied by [`clamp_min_sizes`] whenever
//!   the layout settles.

use std::rc::Rc;

use gpui_kit::base::dock::{
    DockArea, DockAreaRenderer, DockLayout, DockPlacement, DragPanel, DropIndicator, NodeId,
    PaneNode, PaneRef, TabGroupContext, TabGroupRenderer,
};
use gpui_kit::base::{ResizeHandleContext, ResizeHandleState};
use gpui_kit::component::theme::ActiveTheme as _;

use super::panels::DashboardPanels;
use super::*;
use crate::ui::widgets::*;
use gpui_kit::StyleRefinement;

/// Half the 12 px gutter: every group frame is ringed by this, so two
/// adjacent groups leave twice it between their borders.
pub(in crate::ui) const DOCK_GUTTER: Pixels = px(6.);
/// The divider line shown while a split handle is hovered or dragged.
pub(in crate::ui) const DIVIDER_W: Pixels = px(2.);
/// The size of the drag preview a title/tab drag carries.
const DRAG_PREVIEW_W: Pixels = px(200.);
const DRAG_PREVIEW_H: Pixels = px(44.);

/// The minimum size of one panel, by [`Panel::panel_name`] — the floor the
/// dividers stop at (spec 3.100: 240×160, the table 320×200 so its four
/// fixed columns stay legible).
pub(in crate::ui) fn panel_min(name: &str) -> (Pixels, Pixels) {
    match name {
        "table" => (px(320.), px(200.)),
        _ => (px(240.), px(160.)),
    }
}

/// The dock title of one panel — the same wording the scrolling page used
/// (the chart names the selected month, 3.46).
fn panel_title(name: &str, state: Option<&DashboardState>) -> SharedString {
    match name {
        "chart" => match state.and_then(|s| s.filter.summary_month.as_deref()) {
            Some(m) => format!("Spending by category · {}", flow::month_label(m)).into(),
            None => "Spending by category".into(),
        },
        "stacked" => "Monthly spending by category".into(),
        "summary" => "Monthly summary".into(),
        "table" => "Transactions".into(),
        _ => "".into(),
    }
}

/// The four panels in the default 2×2: chart | monthly bars on top,
/// summary | transactions below. The one place the default layout is
/// built (3.101 persists whatever the user made of it and falls back
/// here).
pub(in crate::ui::dashboard) fn default_layout(panels: &DashboardPanels) -> DockLayout {
    DockLayout::v_split()
        .child(
            DockLayout::h_split()
                .child(DockLayout::tabs().panel(panels.chart.clone()), None)
                .child(DockLayout::tabs().panel(panels.stacked.clone()), None),
            None,
        )
        .child(
            DockLayout::h_split()
                .child(DockLayout::tabs().panel(panels.summary.clone()), None)
                .child(DockLayout::tabs().panel(panels.table.clone()), None),
            None,
        )
}

/// The area skin: the layout and nothing else, plus the divider line.
pub(in crate::ui) struct DashboardDock {
    /// Read for the chart title's month (the dock paints titles).
    state: WeakEntity<DashboardState>,
}

impl DashboardDock {
    pub(in crate::ui) fn new(state: WeakEntity<DashboardState>) -> Self {
        Self { state }
    }
}

impl DockAreaRenderer for DashboardDock {
    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(DashboardTabGroup {
            state: self.state.clone(),
        })
    }

    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        // Base's band is transparent and carries only the drag; the line in
        // `border` shows while the pointer is on it or a drag is running.
        let (border, active) = {
            let theme = cx.theme();
            (
                theme.border,
                !matches!(handle.state(), ResizeHandleState::Idle),
            )
        };
        let line = |d: Div| if active { d.bg(border) } else { d }.into_any_element();
        Some(match handle.axis() {
            Axis::Horizontal => line(div().w(DIVIDER_W).h_full()),
            Axis::Vertical => line(div().h(DIVIDER_W).w_full()),
        })
    }
}

/// The group skin: the Panel look (bordered rounded box drawn by the title
/// row and the content frame), the title row as drag handle, the tab strip
/// once two or more panels share the group, and the drop indicator.
pub(in crate::ui) struct DashboardTabGroup {
    /// Read for the chart title's month.
    state: WeakEntity<DashboardState>,
}

impl TabGroupRenderer for DashboardTabGroup {
    fn frame(&self, group: &TabGroupContext, _window: &mut Window, cx: &mut App) -> Stateful<Div> {
        // The ring is the gutter's half; the min size includes it, so two
        // groups' borders never come closer than the panel minimums.
        let (mut min_w, mut min_h) = (px(0.), px(0.));
        for panel in group.panels() {
            let (w, h) = panel_min(panel.panel_name(cx));
            min_w = min_w.max(w);
            min_h = min_h.max(h);
        }
        let ring = DOCK_GUTTER * 2.;
        div()
            .id(("dock-group", group.node().as_u64()))
            .p(DOCK_GUTTER)
            .min_w(min_w + ring)
            .min_h(min_h + ring)
            .debug_selector(move || {
                format!(
                    "panel:{}",
                    group
                        .panels()
                        .iter()
                        .map(|p| p.panel_name(cx))
                        .collect::<Vec<_>>()
                        .join("+")
                )
            })
    }

    fn content_frame(
        &self,
        group: &TabGroupContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        // The bottom half of the box; the base adds the drop handling and
        // the flex that fills the group.
        let (border, radius) = {
            let theme = cx.theme();
            (theme.border, theme.radius)
        };
        // The top corners stay square (the title row above carries them).
        div()
            .id(("dock-group-content", group.node().as_u64()))
            .border_1()
            .border_color(border)
            .rounded_b(radius)
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        // Owned context: the tab and drop handlers need a `'static` handle
        // on the group (the borrow of the renderer argument is not).
        let group = group.clone();
        let (border, radius, muted, list_active) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.radius,
                theme.muted_foreground,
                theme.list_active,
            )
        };
        let dash = self.state.upgrade();
        let dash = dash.as_ref().map(|e| e.read(cx));
        let count = group.panels().len();
        let active_ix = group.active_ix();
        let active_name = group
            .active_panel()
            .map(|p| p.panel_name(cx))
            .unwrap_or_default();
        let title = panel_title(active_name, dash);
        let node = group.node();
        let droppable = group.is_droppable();

        // The title row: the Panel's title, and the group's drag handle —
        // `cursor_grab`, and releasing a dragged panel on it merges into
        // the group (base's own drop zones split on the content body).
        let title_row = div()
            .id(("dock-title", node.as_u64()))
            .w_full()
            .flex_none()
            .px_4()
            .pt_4()
            .pb_2()
            .flex()
            .items_center()
            .gap_2()
            .cursor_grab()
            .border_1()
            .border_color(border)
            .border_b_0()
            .rounded_tl(radius)
            .rounded_tr(radius)
            .debug_selector(move || format!("dashboard:title:{active_name}"))
            .child(region_title(title.clone()))
            .when_some(
                group
                    .is_draggable()
                    .then(|| group.drag_panel(active_ix, cx))
                    .flatten(),
                |this, drag| {
                    let preview = title.clone();
                    this.on_drag(drag, move |drag: &DragPanel, offset, _window, cx| {
                        drag.set_drag_offset(offset);
                        drag.set_preview_size(size(DRAG_PREVIEW_W, DRAG_PREVIEW_H));
                        let title = preview.clone();
                        cx.new(|_| DockDragPreview { title })
                    })
                },
            )
            .when(droppable, {
                let group = group.clone();
                move |this| {
                    this.drag_over::<DragPanel>(|this, _, _, cx| {
                        let border = cx.theme().border;
                        this.bg(border.opacity(0.15))
                    })
                    .on_drop(move |drag: &DragPanel, window, cx| {
                        if drag.source() != node {
                            group.drop_panel(drag.clone(), None, true, window, cx);
                        }
                    })
                }
            });

        if count < 2 {
            return title_row.into_any_element();
        }

        // Two or more panels share the group: a `text_xs` strip picks the
        // shown one; each tab drags its own panel and takes a dropped one
        // into its slot.
        let tabs = div()
            .id(("dock-tabs", node.as_u64()))
            .w_full()
            .flex_none()
            .px_4()
            .pb_2()
            .flex()
            .items_center()
            .gap_1()
            .overflow_x_scroll()
            .border_1()
            .border_color(border)
            .border_b_0()
            .debug_selector(move || format!("dashboard:tabs:{}", node.as_u64()))
            .children((0..count).map(|ix| {
                let group = group.clone();
                let panel = &group.panels()[ix];
                let name = panel.panel_name(cx);
                let tab_title = panel_title(name, dash);
                let is_active = ix == active_ix;
                let tab = div()
                    .id(("dock-tab", ix))
                    .flex_none()
                    .px_2()
                    .py_0p5()
                    .rounded(px(3.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .when(is_active, |t| {
                        t.bg(list_active).font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!is_active, |t| {
                        t.text_color(muted)
                            .cursor_pointer()
                            .hover(|t| t.text_color(list_active))
                    })
                    .debug_selector(move || format!("dashboard:tab:{name}"))
                    .on_click({
                        let group = group.clone();
                        move |_, window, cx| group.select_tab(ix, window, cx)
                    })
                    .when_some(
                        group
                            .is_draggable()
                            .then(|| group.drag_panel(ix, cx))
                            .flatten(),
                        |this, drag| {
                            let preview = tab_title.clone();
                            this.on_drag(drag, move |drag: &DragPanel, offset, _window, cx| {
                                drag.set_drag_offset(offset);
                                drag.set_preview_size(size(DRAG_PREVIEW_W, DRAG_PREVIEW_H));
                                let title = preview.clone();
                                cx.new(|_| DockDragPreview { title })
                            })
                        },
                    );
                if droppable {
                    tab.drag_over::<DragPanel>(|this, _, _, cx| {
                        let border = cx.theme().border;
                        this.bg(border.opacity(0.15))
                    })
                    .on_drop(move |drag: &DragPanel, window, cx| {
                        group.drop_panel(drag.clone(), Some(ix), true, window, cx);
                    })
                    .into_any_element()
                } else {
                    tab.into_any_element()
                }
            }));

        v_flex()
            .w_full()
            .flex_none()
            .children([title_row.into_any_element(), tabs.into_any_element()])
            .into_any_element()
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        _group: &TabGroupContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        // The panel fills the content region; `cached` keeps a repaint of
        // the dock from repainting panels that did not change.
        panel
            .cached(StyleRefinement::default().size_full())
            .into_any_element()
    }

    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let to = indicator.to();
        let color = cx.theme().list_active.opacity(0.35);
        Some(
            div()
                .absolute()
                .left(to.origin().x)
                .top(to.origin().y)
                .w(to.size().width)
                .h(to.size().height)
                .rounded(px(4.))
                .bg(color)
                .debug_selector(|| "dashboard:drop-indicator".into())
                .into_any_element(),
        )
    }
}

/// What a title/tab drag carries: a small card naming the panel (base's
/// `DragPanel` payload draws nothing itself).
struct DockDragPreview {
    title: SharedString,
}

impl Render for DockDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (border, radius) = {
            let theme = cx.theme();
            (theme.border, theme.radius)
        };
        div()
            .w(DRAG_PREVIEW_W)
            .px_3()
            .py_2()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(cx.theme().background)
            .text_xs()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .debug_selector(|| "dashboard:drag-preview".into())
            .child(self.title.clone())
    }
}

/// The minimum a node's subtree needs along `axis` — a tab group needs its
/// widest/tallest member, a split the sum (same axis) or the max (cross).
fn subtree_min(node: &PaneNode, axis: Axis, area: &DockArea, cx: &App) -> Pixels {
    match node.kind() {
        // The split's children are group *frames*, ring included, so the
        // ring comes out of the panel's usable minimum — same as the
        // frame's own `min_w`/`min_h`.
        PaneRef::Tabs { panels, .. } => {
            let ring = DOCK_GUTTER * 2.;
            panels
                .iter()
                .map(|p| {
                    let name = area.panel(*p).map(|v| v.panel_name(cx)).unwrap_or("");
                    let (w, h) = panel_min(name);
                    if axis == Axis::Horizontal {
                        w + ring
                    } else {
                        h + ring
                    }
                })
                .max()
                .unwrap_or(px(0.))
        }
        PaneRef::Split {
            axis: inner,
            children,
            ..
        } => {
            let mins: Vec<Pixels> = children
                .iter()
                .map(|c| subtree_min(c, axis, area, cx))
                .collect();
            if axis == inner {
                mins.into_iter().fold(px(0.), |a, b| a + b)
            } else {
                mins.into_iter().max().unwrap_or(px(0.))
            }
        }
    }
}

/// Re-apply the panel minimums after a layout edit or divider drag (3.100).
///
/// Base's own divider floor is a flat 100 px, so a drag can end with a pane
/// under its minimum; this raises undersized panes and pays for it out of
/// the panes with slack, keeping the split's total width. Called deferred —
/// it runs after the dock finished its own edit, never inside it.
pub(in crate::ui) fn clamp_min_sizes(area: &Entity<DockArea>, window: &mut Window, cx: &mut App) {
    let mut plans: Vec<(NodeId, Vec<Pixels>)> = Vec::new();
    {
        let a = area.read(cx);
        let Some(tree) = a.layout(DockPlacement::Center) else {
            return;
        };
        tree.root().walk(&mut |node| {
            let PaneRef::Split {
                axis,
                children,
                sizes,
            } = node.kind()
            else {
                return;
            };
            // Only splits whose sizes are all measured pixels can be
            // re-set by id; an unmeasured split is still flex-laid and
            // already respects the frames' min sizes.
            if sizes.iter().any(|s| s.is_none()) {
                return;
            }
            let cur: Vec<Pixels> = sizes.iter().map(|s| s.unwrap()).collect();
            let mins: Vec<Pixels> = children
                .iter()
                .map(|c| subtree_min(c, axis, a, cx))
                .collect();
            let fixed = clamp_sizes(&cur, &mins);
            if fixed != cur {
                plans.push((node.id(), fixed));
            }
        });
    }
    for (node, sizes) in plans {
        area.update(cx, |a, cx| a.set_split_sizes(node, sizes, window, cx));
    }
}

/// Raise every size to its minimum, paying for it out of the panes with
/// slack (proportionally), keeping the total. When the container cannot fit
/// all minimums at all, the minimums themselves are returned and the flex
/// layout's own floors take over.
fn clamp_sizes(cur: &[Pixels], mins: &[Pixels]) -> Vec<Pixels> {
    let total: f32 = cur.iter().map(|p| p.as_f32()).sum();
    let mut s: Vec<f32> = cur
        .iter()
        .zip(mins)
        .map(|(c, m)| c.as_f32().max(m.as_f32()))
        .collect();
    let need: f32 = s.iter().sum::<f32>() - total;
    if need > 0.01 {
        let slack: f32 = s
            .iter()
            .zip(mins)
            .map(|(v, m)| *v - m.as_f32())
            .map(|v| v.max(0.))
            .sum();
        if slack > need {
            for (v, m) in s.iter_mut().zip(mins) {
                let give = (*v - m.as_f32()).max(0.);
                *v -= need * give / slack;
            }
        } else {
            s = mins.iter().map(|m| m.as_f32()).collect();
        }
    }
    s.into_iter().map(px).collect()
}
