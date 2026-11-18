//! The dashboard dock's saved layout (3.101): store what the user made
//! of the 2×2, restore it at launch, and put the default back on demand.
//!
//! The arrangement lives in the `settings` table under
//! [`DASHBOARD_LAYOUT`] as the JSON [`DockArea::dump`] produces, whose
//! `version` field carries [`LAYOUT_VERSION`]. The page owns the timing:
//! the dock's `DockEvent::LayoutChanged` subscription calls
//! [`DashboardState::schedule_layout_save`] (a 500 ms debounce, the 3.56
//! search pattern — one task field, latest wins), and the dock's first
//! paint calls [`DashboardState::start_layout_restore`], which reads the
//! stored JSON through the worker and loads it before the page ever
//! blocks on it.
//!
//! A stored layout is *usable* only if it parses, carries the current
//! version, and names exactly the four panels — gpui-base's `load`
//! turns an unknown name into an invisible placeholder, so a layout
//! from a future build (or hand-edited) is rejected whole and the
//! default 2×2 shown instead, once, on stderr.
//!
//! `dump` resolves every slot to concrete pixels, so a save always
//! writes what is on screen; a restore's own `load` and the start-up
//! `set_center(default)` emit `LayoutChanged` too, which is why saves
//! stay gated until the restore read has resolved.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::dock::{
    DockArea, DockAreaState, PanelBuildContext, PanelState, PanelView, register_panel,
};

use super::dock::default_layout;
use super::panels::DashboardPanels;
use super::*;

/// The layout schema version written into every stored layout. Bump it
/// when the panel set or the default tree changes shape: stored layouts
/// carrying any other version fall back to the default (3.101).
pub(in crate::ui) const LAYOUT_VERSION: usize = 1;

/// The four panels a usable stored layout must name exactly once — the
/// [`Panel::panel_name`](gpui_kit::base::dock::Panel::panel_name)s of
/// [`DashboardPanels`]. Must stay sorted.
const PANEL_NAMES: [&str; 4] = ["chart", "stacked", "summary", "table"];

/// The names gpui-base gives its container nodes in a dump; anything
/// else in the tree is a panel leaf. (The crate's own constants are
/// `pub(crate)`, so this mirrors them — see its `state_convert`.)
const CONTAINER_NAMES: [&str; 2] = ["StackPanel", "TabPanel"];

/// How long after the last `LayoutChanged` the save runs: a divider
/// drag emits an event per settle; one write should follow the drag.
pub(in crate::ui) const LAYOUT_SAVE_DELAY: Duration = Duration::from_millis(500);

/// Serialize the live dock. `dump` never fails; serialization can only
/// fail on a non-UTF-8 panel name, in which case there is nothing
/// worth storing.
pub(in crate::ui) fn encode_layout(dock: &DockArea, cx: &App) -> Option<String> {
    serde_json::to_string(&dock.dump(cx)).ok()
}

/// Parse a stored layout, rejecting everything the dock should not try
/// to load: bad JSON, another version, or a tree whose panel leaves are
/// not exactly the four current panels (unknown, missing or duplicated
/// names — a layout a Reset or a future build cannot be trusted with).
pub(in crate::ui) fn decode_layout(json: &str) -> Option<DockAreaState> {
    let state: DockAreaState = serde_json::from_str(json).ok()?;
    if state.version != Some(LAYOUT_VERSION) {
        return None;
    }
    let mut names: Vec<&str> = Vec::new();
    collect_leaf_names(&state.center, &mut names);
    names.sort_unstable();
    (names == PANEL_NAMES).then_some(state)
}

/// The panel-leaf names below `node`, skipping container nodes.
pub(in crate::ui) fn collect_leaf_names<'a>(node: &'a PanelState, into: &mut Vec<&'a str>) {
    if !CONTAINER_NAMES.contains(&node.panel_name.as_str()) {
        into.push(&node.panel_name);
        return;
    }
    for child in &node.children {
        collect_leaf_names(child, into);
    }
}

/// Register the four dashboard panels so `DockArea::load` can rebuild
/// the tree from a dump. Each builder hands back the page's *own* panel
/// entities — a restored dashboard shows the live state, not clones —
/// so the closures capture the handles the dock was built with. The
/// registry is global and re-registration overwrites, which makes this
/// safe to call on every dock creation (there is one per app).
pub(in crate::ui::dashboard) fn register_dock_panels(panels: &DashboardPanels, cx: &mut App) {
    let chart = panels.chart.clone();
    register_panel(
        cx,
        "chart",
        move |_: PanelBuildContext, _: &mut Window, _: &mut App| {
            Arc::new(chart.clone()) as Arc<dyn PanelView>
        },
    );
    let stacked = panels.stacked.clone();
    register_panel(
        cx,
        "stacked",
        move |_: PanelBuildContext, _: &mut Window, _: &mut App| {
            Arc::new(stacked.clone()) as Arc<dyn PanelView>
        },
    );
    let summary = panels.summary.clone();
    register_panel(
        cx,
        "summary",
        move |_: PanelBuildContext, _: &mut Window, _: &mut App| {
            Arc::new(summary.clone()) as Arc<dyn PanelView>
        },
    );
    let table = panels.table.clone();
    register_panel(
        cx,
        "table",
        move |_: PanelBuildContext, _: &mut Window, _: &mut App| {
            Arc::new(table.clone()) as Arc<dyn PanelView>
        },
    );
}

impl DashboardState {
    /// Read the stored layout through the worker and apply it once it
    /// lands (3.101). Called from the dock's first paint; the page
    /// never blocks — the default 2×2 is on screen until (or unless)
    /// the read resolves. A layout that is unusable, and any failed
    /// read, leave the default in place; the gate for the saves lifts
    /// either way, so the user's next edit is what gets written.
    pub(in crate::ui::dashboard) fn start_layout_restore(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layout_restore_gen += 1;
        let restore_gen = self.layout_restore_gen;
        let db = self.db.clone();
        // Detached: the read must run to the end (it lifts the save
        // gate), and the generation check drops it if it is stale.
        cx.spawn_in(window, async move |this, cx| {
            let stored = db
                .call_read(|conn| settings::dashboard_layout(conn))
                .await
                .ok()
                .and_then(|r| r.ok())
                .flatten();
            let _ = this.update_in(cx, move |this, window, cx| {
                // A Reset while this read was in flight wins: this read
                // describes the pre-reset world, so drop it (the gate
                // still lifts, on the reset's own terms).
                if this.layout_restore_gen == restore_gen {
                    this.apply_stored_layout(stored.as_deref(), window, cx);
                }
            });
        })
        .detach();
    }

    /// Put the stored layout on screen, or keep the default and say why
    /// once (stderr; the app has no log sink). Then lift the save gate.
    fn apply_stored_layout(
        &mut self,
        stored: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(json) = stored {
            match decode_layout(json) {
                Some(state) => {
                    if let Some(dock) = &self.dock {
                        dock.update(cx, |dock, cx| {
                            let _ = dock.load(state, window, cx);
                        });
                    }
                }
                None => {
                    eprintln!("financi: stored dashboard layout is unusable, using the default");
                }
            }
        }
        self.layout_ready = true;
    }

    /// The dock's `LayoutChanged` subscription calls this. One task
    /// field, replaced per event: a burst of events (a drag) leaves a
    /// single save 500 ms after the last one, and the save dumps the
    /// tree *then*, so what lands in the database is what is on screen.
    /// Gated until the start-up restore has resolved (see
    /// [`Self::start_layout_restore`]).
    pub(in crate::ui::dashboard) fn schedule_layout_save(&mut self, cx: &mut Context<Self>) {
        if !self.layout_ready {
            return;
        }
        let db = self.db.clone();
        self.layout_save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LAYOUT_SAVE_DELAY).await;
            let json = this
                .read_with(cx, |this, cx| {
                    // 5.3: refuse writes while an export/restore owns the
                    // database; the layout is not worth racing one.
                    if this.writes_blocked.get() {
                        return None;
                    }
                    let dock = this.dock.as_ref()?.read(cx);
                    encode_layout(dock, cx)
                })
                .ok()
                .flatten();
            let Some(json) = json else { return };
            // The writer thread (writes never go to the reader), FIFO:
            // an older queued save can never land after a newer one.
            let _ = on_db(&db, move |conn| settings::set_dashboard_layout(conn, &json)).await;
        }));
    }

    /// Reset the dashboard layout (3.101): put the default 2×2 back on
    /// screen and save it, so the reset survives a restart. Reached
    /// from the filter row's icon button and *View ▸ Reset dashboard
    /// layout*. `set_center` emits `LayoutChanged`, which schedules the
    /// save; lifting the gate first is what makes that save happen even
    /// if the click lands before the start-up restore resolved (the
    /// generation bump drops that restore instead of letting it paint
    /// over the reset).
    pub(in crate::ui) fn reset_dock_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(dock), Some(panels)) = (self.dock.clone(), self.dock_panels.clone()) else {
            return; // no dock yet — the default is already what shows.
        };
        self.layout_restore_gen += 1;
        self.layout_ready = true;
        dock.update(cx, |dock, cx| {
            dock.set_center(default_layout(&panels), window, cx);
        });
    }
}
