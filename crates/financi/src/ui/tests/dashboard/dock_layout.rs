//! The saved dashboard layout (3.101): a move and a divider drag land in
//! the settings table 500 ms after the last edit and come back when a
//! second window opens over the same database; a corrupt stored value
//! falls back to the default 2×2; the Reset button puts the default back
//! *and* saves it, so the reset survives too.
//!
//! The second window is a fresh `Shell` over a clone of the first one's
//! `DbWorker` — the same database, a fresh dock, the restore read the
//! user would see after a restart.

use std::time::Duration;

use super::dock::drag;
use super::*;
use crate::ui::dashboard::dock_layout::{
    LAYOUT_SAVE_DELAY, LAYOUT_VERSION, collect_leaf_names, decode_layout, encode_layout,
};
use financi_db::settings::DASHBOARD_LAYOUT;

/// The layout as stored right now, read through the worker — `None`
/// while nothing has been written.
fn stored_layout(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<String> {
    let db = shell.read_with(cx, |s, _| s.db.clone());
    block_on(db.call(|conn| {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [DASHBOARD_LAYOUT],
            |row| row.get::<_, String>(0),
        )
        .ok()
    }))
    .ok()
    .flatten()
}

/// The live dock's tree, serialized exactly as the save would.
fn live_json(cx: &mut TestAppContext, shell: &Entity<Shell>) -> String {
    let dock = dash(cx, shell)
        .read_with(cx, |d, _| d.dock.clone())
        .expect("the dock exists once the dashboard has painted")
        .clone();
    cx.update(|cx| {
        let dock = dock.read(cx);
        encode_layout(dock, cx).expect("the live dock serializes")
    })
}

/// Let the 500 ms save debounce elapse and the write land. The debounce
/// fires inside `advance_clock`; the save task then queues its write on
/// the writer thread, and a no-op job behind it completes only once the
/// save has run (the worker is FIFO).
fn flush_save(cx: &mut TestAppContext, shell: &Entity<Shell>) {
    cx.executor()
        .advance_clock(LAYOUT_SAVE_DELAY + Duration::from_millis(100));
    let db = shell.read_with(cx, |s, _| s.db.clone());
    let _ = block_on(db.call(|_| ()));
}

/// Open a second window over the same database: a fresh Shell (fresh
/// panels, fresh dock) whose restore reads what the first window saved.
fn reopen(cx: &mut TestAppContext, shell: &Entity<Shell>) -> (AnyWindowHandle, Entity<Shell>) {
    let (db, db_path) = shell.read_with(cx, |s, _| (s.db.clone(), s.db_path.clone()));
    cx.update(|cx| {
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1200.), px(1440.)),
        };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Shell::new(db_path, db, window, cx)),
        )
        .unwrap();
        (window, shell)
    })
}

/// The four default frames side by side, each in its own group.
fn assert_default_frames(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    for name in ["chart", "stacked", "summary", "table"] {
        assert!(
            paints(cx, handle, format!("panel:{name}")),
            "the default dock shows the {name} panel"
        );
    }
    assert!(
        !paints(cx, handle, "dashboard:tab:summary"),
        "no panel shares a group in the default 2×2"
    );
}

/// Move Summary on to Chart's title (a tab), drag the Chart/Monthly
/// divider, and let the save settle. Returns the live JSON saved.
#[gpui::test]
fn a_moved_and_resized_dashboard_comes_back_from_the_same_database(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
    });
    settle(cx, handle);
    // The start-up restore resolved; nothing stored yet, so no save yet.
    wait_until(cx, "the start-up layout read resolved", |cx| {
        dash(cx, &shell).read(cx).layout_ready
    });
    assert!(stored_layout(cx, &shell).is_none());

    // Move: Summary tabs on to Chart (the 3.100 drag pattern).
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
    assert!(
        paints(cx, handle, "dashboard:tab:summary"),
        "the drag tabbed the Summary panel"
    );
    // Back to Chart: the group shows its content, which the restore
    // check below compares against.
    click(cx, handle, "dashboard:tab:chart");
    settle(cx, handle);
    assert!(
        paints(cx, handle, "chart:plot"),
        "clicking the Chart tab shows the Chart panel"
    );

    // Resize: the divider between the Chart group and Monthly, 120 px
    // right. It is measured off Monthly's left edge — inside a tab
    // group the frame-level `panel:` selector is gone.
    let stacked_before = bounds(cx, handle, "panel:stacked");
    let grab = point(
        px(f32::from(stacked_before.origin.x) - 2.),
        px(f32::from(stacked_before.origin.y) + 40.),
    );
    drag(
        cx,
        handle,
        grab,
        point(px(f32::from(grab.x) + 120.), grab.y),
        8,
    );
    settle(cx, handle);
    let stacked_moved = bounds(cx, handle, "panel:stacked");
    assert!(
        f32::from(stacked_moved.origin.x) > f32::from(stacked_before.origin.x) + 40.,
        "the divider drag moved Monthly right\nbefore: {stacked_before:?}\nafter:  {stacked_moved:?}"
    );

    // One write, after the debounce, holding the edited tree.
    flush_save(cx, &shell);
    let stored = stored_layout(cx, &shell).expect("the edit was stored");
    let decoded = decode_layout(&stored).expect("what was stored is a current layout");
    assert_eq!(decoded.version, Some(LAYOUT_VERSION));
    let live = decode_layout(&live_json(cx, &shell)).expect("the live dock serializes");
    assert_eq!(decoded.center, live.center, "the write matches the screen");

    // The restart: a second window over the same database restores the
    // same tree — the tabbed group shows, and the dragged widths hold.
    let (handle2, shell2) = reopen(cx, &shell);
    settle(cx, handle2);
    wait_until(cx, "the second window restored the layout", |cx| {
        dash(cx, &shell2).read(cx).layout_ready
    });
    wait_until(cx, "the second window loaded its rows", |cx| {
        dash(cx, &shell2).read(cx).row_count == 4
    });
    settle(cx, handle2);
    assert!(
        paints(cx, handle2, "dashboard:tab:summary"),
        "the moved panel is a tab again in the new window"
    );
    assert!(
        !paints(cx, handle2, "panel:summary"),
        "the restored Summary panel has no frame of its own"
    );
    assert!(
        paints(cx, handle2, "chart:plot"),
        "the restored group shows the Chart panel, active as saved"
    );
    let stacked2 = bounds(cx, handle2, "panel:stacked");
    assert!(
        (f32::from(stacked2.origin.x) - f32::from(stacked_moved.origin.x)).abs() < 2.
            && (f32::from(stacked2.size.width) - f32::from(stacked_moved.size.width)).abs() < 2.,
        "the divider kept the dragged width across the restart\nfirst:  {stacked_moved:?}\nsecond: {stacked2:?}"
    );
}

/// A stored value that is not a usable layout — bad JSON here — must not
/// take the dashboard down with it: the default 2×2 shows instead.
#[gpui::test]
fn a_corrupt_stored_layout_falls_back_to_the_default(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
    });
    settle(cx, handle);
    wait_until(cx, "the start-up layout read resolved", |cx| {
        dash(cx, &shell).read(cx).layout_ready
    });

    // Corrupt the stored layout the way a hand-edited or half-written
    // row would look, then restart the window.
    let db = shell.read_with(cx, |s, _| s.db.clone());
    block_on(db.call(|conn| {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            (DASHBOARD_LAYOUT, "not json at all"),
        )
    }))
    .unwrap()
    .unwrap();
    let (handle2, shell2) = reopen(cx, &shell);
    settle(cx, handle2);
    wait_until(cx, "the second window's layout read resolved", |cx| {
        dash(cx, &shell2).read(cx).layout_ready
    });
    settle(cx, handle2);
    assert_default_frames(cx, handle2);
    assert!(
        decode_layout(&live_json(cx, &shell2)).is_some(),
        "the fallback is a live, current dock"
    );
    let _ = handle;
}

/// Reset puts the default 2×2 back *and* stores it: after the click the
/// frames are the default, and a restart comes back to the default.
#[gpui::test]
fn the_reset_button_restores_the_default_and_saves_it(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_chart_rows);
    wait_until(cx, "table loaded", |cx| {
        dash(cx, &shell).read(cx).row_count == 4
    });
    settle(cx, handle);
    wait_until(cx, "the start-up layout read resolved", |cx| {
        dash(cx, &shell).read(cx).layout_ready
    });

    // Edit first: tab Summary on to Chart, so Reset has something to undo.
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
    flush_save(cx, &shell);
    assert!(
        paints(cx, handle, "dashboard:tab:summary"),
        "the edit tabbed the panels before the reset"
    );

    click(cx, handle, "filters:reset-layout");
    settle(cx, handle);
    assert_default_frames(cx, handle);
    flush_save(cx, &shell);
    let stored = stored_layout(cx, &shell).expect("the reset was stored");
    let decoded = decode_layout(&stored).expect("the reset stores a current layout");
    let dock = dash(cx, &shell)
        .read_with(cx, |d, _| d.dock.clone())
        .expect("dock")
        .clone();
    let live =
        decode_layout(&cx.update(|cx| encode_layout(dock.read(cx), cx).expect("serializes")))
            .expect("live serializes");
    assert_eq!(
        decoded.center, live.center,
        "the reset write matches the screen"
    );

    // And the reset survives the restart.
    let (handle2, shell2) = reopen(cx, &shell);
    settle(cx, handle2);
    wait_until(cx, "the second window restored the default", |cx| {
        dash(cx, &shell2).read(cx).layout_ready
    });
    settle(cx, handle2);
    assert_default_frames(cx, handle2);
}

// ---- decode rules (pure, no window) -----------------------------------

/// A dump of the default dashboard in the persisted schema: a vertical
/// `StackPanel` of two horizontal ones, each holding two panel leaves.
/// Built by hand so the tests pin the *stored* shape, not our code.
fn default_json() -> String {
    fn leaf(name: &str) -> serde_json::Value {
        serde_json::json!({
            "panel_name": name,
            "children": [],
            "info": { "panel": null },
        })
    }
    fn stack(axis: usize, children: Vec<serde_json::Value>) -> serde_json::Value {
        let sizes = vec![480.0; children.len()];
        serde_json::json!({
            "panel_name": "StackPanel",
            "children": children,
            "info": { "stack": { "sizes": sizes, "axis": axis } },
        })
    }
    serde_json::to_string(&serde_json::json!({
        "version": LAYOUT_VERSION,
        "center": stack(
            1,
            vec![
                stack(0, vec![leaf("chart"), leaf("stacked")]),
                stack(0, vec![leaf("summary"), leaf("table")]),
            ],
        ),
    }))
    .unwrap()
}

#[gpui::test]
fn the_default_layout_decodes_with_all_four_panels(_cx: &mut TestAppContext) {
    let json = default_json();
    let state = decode_layout(&json).expect("default layout decodes");
    assert_eq!(state.version, Some(LAYOUT_VERSION));
    let mut names: Vec<&str> = Vec::new();
    collect_leaf_names(&state.center, &mut names);
    names.sort_unstable();
    assert_eq!(names, ["chart", "stacked", "summary", "table"]);
}

#[gpui::test]
fn garbage_and_wrong_versions_decode_to_nothing(_cx: &mut TestAppContext) {
    // Not JSON at all (the corrupt-value fallback).
    assert!(decode_layout("not json").is_none());
    // Right shape, wrong version: an old layout is dropped, not guessed at.
    let mut value: serde_json::Value = serde_json::from_str(&default_json()).unwrap();
    value["version"] = serde_json::json!(LAYOUT_VERSION + 1);
    assert!(decode_layout(&value.to_string()).is_none());
    value["version"] = serde_json::Value::Null;
    assert!(decode_layout(&value.to_string()).is_none());
}

#[gpui::test]
fn layouts_that_are_not_the_four_panels_decode_to_nothing(_cx: &mut TestAppContext) {
    // A panel name from a future or foreign layout.
    assert!(decode_layout(&default_json().replace("stacked", "budgets")).is_none());
    // A panel missing entirely (its slot now duplicates another).
    assert!(decode_layout(&default_json().replace("summary", "table")).is_none());
    // No panels at all: an empty tree.
    let empty = serde_json::json!({
        "version": LAYOUT_VERSION,
        "center": {
            "panel_name": "TabPanel",
            "children": [],
            "info": { "tabs": { "active_index": 0 } },
        },
    });
    assert!(decode_layout(&empty.to_string()).is_none());
}
