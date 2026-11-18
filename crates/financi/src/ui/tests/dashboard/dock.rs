//! The dock itself (3.100): four frames by default, dragging a title on
//! to another panel's title tabs them, tabs switch the shown panel, and
//! divider drags resize the panels either side — clamping at the minimum
//! sizes.
//!
//! Drags follow base's own test pattern: press, a few moves with the
//! button held, release. The title row is the drag handle, so a panel
//! moves by its title; a drop on another group's *title* joins it as a
//! tab (a drop on a content body splits that group instead).

use gpui_kit::MouseButton;

use super::layout::open_dashboard_for_geometry;
use super::*;
use crate::ui::dashboard::dock::{DOCK_GUTTER, panel_min};

/// The dashboard at `w` × `h` with the demo data, probes settled.
fn open(cx: &mut TestAppContext, w: f32, h: f32) -> AnyWindowHandle {
    let (handle, _shell) = open_dashboard_for_geometry(cx, w, h);
    settle(cx, handle);
    handle
}

/// Press at `from`, move in `steps` increments to `to`, release.
fn drag(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    from: Point<Pixels>,
    to: Point<Pixels>,
    steps: i32,
) {
    let mut window = VisualTestContext::from_window(handle, cx);
    window.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    for i in 1..=steps {
        let f = i as f32 / steps as f32;
        let x = f32::from(from.x) + (f32::from(to.x) - f32::from(from.x)) * f;
        let y = f32::from(from.y) + (f32::from(to.y) - f32::from(from.y)) * f;
        window.simulate_mouse_move(
            point(px(x), px(y)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
    }
    window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
}

/// The default layout: four frames, each titled, and no tab strip —
/// tabs appear only once two panels share a group.
#[gpui::test]
fn the_default_dock_shows_four_panels_without_tabs(cx: &mut TestAppContext) {
    let handle = open(cx, 1200., 720.);
    for name in ["chart", "stacked", "summary", "table"] {
        assert!(
            paints(cx, handle, format!("panel:{name}")),
            "the default dock shows the {name} panel"
        );
        assert!(
            paints(cx, handle, format!("dashboard:title:{name}")),
            "the {name} panel's frame carries its title"
        );
    }
    assert!(
        !paints(cx, handle, "dashboard:tab:chart"),
        "a dock of single-panel groups shows no tab strip"
    );
}

/// Dragging the Summary title on to the Chart frame's title tabs them:
/// both become tabs of one group, the Summary frame leaves its old spot,
/// and the other two panels keep their places.
#[gpui::test]
fn dragging_a_title_on_to_another_panel_tabs_them(cx: &mut TestAppContext) {
    let handle = open(cx, 1200., 720.);
    let summary_title = bounds(cx, handle, "dashboard:title:summary");
    let chart_title = bounds(cx, handle, "dashboard:title:chart");
    let stacked_before = bounds(cx, handle, "panel:stacked");

    drag(
        cx,
        handle,
        point(
            px(f32::from(summary_title.origin.x) + 60.),
            px(f32::from(summary_title.origin.y) + 8.),
        ),
        point(
            px(f32::from(chart_title.origin.x) + 60.),
            px(f32::from(chart_title.origin.y) + 8.),
        ),
        12,
    );
    settle(cx, handle);

    assert!(
        paints(cx, handle, "dashboard:tab:summary"),
        "the moved panel appears as a tab of the Chart group"
    );
    assert!(
        paints(cx, handle, "dashboard:tab:chart"),
        "the panel it joined appears as a tab too"
    );
    assert!(
        !paints(cx, handle, "panel:summary"),
        "the Summary panel no longer has a frame of its own"
    );
    let stacked_after = bounds(cx, handle, "panel:stacked");
    assert!(
        (f32::from(stacked_after.origin.y) - f32::from(stacked_before.origin.y)).abs() < 2.0
            && (f32::from(stacked_after.origin.x) - f32::from(stacked_before.origin.x)).abs() < 2.0,
        "the panels the drag did not touch keep their places\nbefore: {stacked_before:?}\nafter:  {stacked_after:?}"
    );
}

/// With two panels in one group, clicking a tab switches which panel's
/// content the frame shows.
#[gpui::test]
fn clicking_a_tab_switches_the_shown_panel(cx: &mut TestAppContext) {
    let handle = open(cx, 1200., 720.);
    // Tab Summary on to Chart first.
    let summary_title = bounds(cx, handle, "dashboard:title:summary");
    let chart_title = bounds(cx, handle, "dashboard:title:chart");
    drag(
        cx,
        handle,
        point(
            px(f32::from(summary_title.origin.x) + 60.),
            px(f32::from(summary_title.origin.y) + 8.),
        ),
        point(
            px(f32::from(chart_title.origin.x) + 60.),
            px(f32::from(chart_title.origin.y) + 8.),
        ),
        12,
    );
    settle(cx, handle);

    click(cx, handle, "dashboard:tab:summary");
    settle(cx, handle);
    assert!(
        paints(cx, handle, "summary:name-0"),
        "clicking the Summary tab shows the Summary panel"
    );
    assert!(
        !paints(cx, handle, "chart:plot"),
        "the hidden Chart panel draws nothing"
    );

    click(cx, handle, "dashboard:tab:chart");
    settle(cx, handle);
    assert!(
        paints(cx, handle, "chart:plot"),
        "clicking the Chart tab shows the Chart panel again"
    );
    assert!(
        !paints(cx, handle, "summary:name-0"),
        "the Summary panel is hidden once Chart is shown"
    );
}

/// Dragging the divider between the two top panels moves it: the left
/// panel gains exactly the width the right one loses.
#[gpui::test]
fn dragging_a_divider_resizes_the_panels_either_side(cx: &mut TestAppContext) {
    let handle = open(cx, 1200., 720.);
    let chart_before = bounds(cx, handle, "panel:chart");
    let stacked_before = bounds(cx, handle, "panel:stacked");
    // Frames tile flush; the handle band straddles their shared edge.
    let grab = point(
        px(f32::from(chart_before.origin.x) + f32::from(chart_before.size.width) - 2.),
        px(f32::from(chart_before.origin.y) + 40.),
    );

    drag(
        cx,
        handle,
        grab,
        point(px(f32::from(grab.x) + 120.), grab.y),
        8,
    );
    settle(cx, handle);

    let chart_after = bounds(cx, handle, "panel:chart");
    let stacked_after = bounds(cx, handle, "panel:stacked");
    let gained = f32::from(chart_after.size.width) - f32::from(chart_before.size.width);
    let lost = f32::from(stacked_before.size.width) - f32::from(stacked_after.size.width);
    assert!(
        gained > 40.,
        "the Chart panel grew when its divider was dragged right\nbefore: {chart_before:?}\nafter:  {chart_after:?}"
    );
    assert!(
        (gained - lost).abs() < 2.,
        "the Monthly panel gave up exactly what Chart gained\nchart: {chart_before:?} -> {chart_after:?}\nstacked: {stacked_before:?} -> {stacked_after:?}"
    );
}

/// A divider drag far past the minimum stops at it: neither panel goes
/// below its minimum frame size, and the row still fills the page.
#[gpui::test]
fn a_divider_drag_clamps_at_the_minimum_size(cx: &mut TestAppContext) {
    let handle = open(cx, 1200., 720.);
    let chart_before = bounds(cx, handle, "panel:chart");
    let stacked_before = bounds(cx, handle, "panel:stacked");
    let grab = point(
        px(f32::from(chart_before.origin.x) + f32::from(chart_before.size.width) - 2.),
        px(f32::from(chart_before.origin.y) + 40.),
    );

    // 600 px right: far more than the Monthly panel can give up.
    drag(
        cx,
        handle,
        grab,
        point(px(f32::from(grab.x) + 600.), grab.y),
        10,
    );
    settle(cx, handle);

    let chart_after = bounds(cx, handle, "panel:chart");
    let stacked_after = bounds(cx, handle, "panel:stacked");
    // The frame carries a min of the panel min plus its 6 px ring.
    let stacked_floor = f32::from(panel_min("stacked").0) + 2. * f32::from(DOCK_GUTTER) - 1.;
    assert!(
        f32::from(stacked_after.size.width) >= stacked_floor,
        "the Monthly panel is never narrower than its minimum: {:?}",
        stacked_after.size.width
    );
    let chart_floor = f32::from(panel_min("chart").0) + 2. * f32::from(DOCK_GUTTER) - 1.;
    assert!(
        f32::from(chart_after.size.width) >= chart_floor,
        "the Chart panel is never narrower than its minimum: {:?}",
        chart_after.size.width
    );
    // The row was re-balanced, not shrunk: it spans the same width.
    let span = |l: &Bounds<Pixels>, r: &Bounds<Pixels>| {
        f32::from(r.origin.x) + f32::from(r.size.width) - f32::from(l.origin.x)
    };
    assert!(
        (span(&chart_after, &stacked_after) - span(&chart_before, &stacked_before)).abs() < 2.,
        "the top row still fills the page after clamping"
    );
}
