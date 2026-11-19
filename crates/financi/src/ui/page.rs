//! The page-entity pattern every page follows (3.79), and the events a
//! page uses to reach `Shell`.
//!
//! A page is an `Entity` owned by `Shell` (one field per page, e.g.
//! `Shell::imports: Entity<ImportsPage>`) that owns its own state, renders
//! itself (`Render`), and refreshes itself from the `Catalog`. `Shell`
//! navigates by calling [`PageEntity::set_visible`] and never touches page
//! fields; a page never holds `Shell`.
//!
//! The shape:
//!
//! - State fields moved off `Shell`, plus `visible: bool`, `dirty: bool`,
//!   a `focus: FocusHandle`, `focus_pending: bool` and the `Subscription`
//!   returned by [`watch_catalog`].
//! - `const DEPENDS: Domains` — the domains the page renders. The page
//!   subscribes to `Catalog`'s `DataChanged` through [`watch_catalog`] and
//!   filters on it: while shown, an event in its domains re-applies the
//!   current snapshot; while hidden it only marks `dirty`, and the next
//!   show applies. Pages never run list queries of their own.
//! - `set_visible(true)` applies the snapshot when it changed while hidden
//!   and raises `focus_pending`; the page's `render` turns that into a
//!   deferred `window.focus` once its root element exists (the same
//!   pattern as the Rules page's search focus, `rules_focus_pending`).
//!   `set_visible(false)` marks `dirty` so the next show catches up.
//! - The root element takes `track_focus(&focus)` and
//!   `key_context("<Page>")`; Escape is the page's [`Cancel`] action,
//!   bound per context in [`crate::ui::init`] and handled on the root.
//! - Anything a page needs from outside — navigation, toasts, a full
//!   reload — is a [`PageEvent`] emitted on its own entity; `Shell`
//!   subscribes in its `new`.
//!
//! Because a hidden page defers applying, anything that reads page state
//! while the page may be hidden reads the `Catalog` snapshot instead (the
//! sidebar's Transfers pill) or calls [`PageEntity::apply_if_dirty`]
//! first (a global action like *Undo last import*).

use gpui_kit::{Context, Entity, Subscription, actions};

use crate::ui::catalog::{Catalog, DataChanged, Domains};

// Escape on a page's own key context: close the page's armed confirmation
// or open transient UI. Bound per context (`"Transfers"`, `"Imports"`, …)
// in `crate::ui::init`, handled on the page root.
actions!(financi_page, [Cancel]);

/// What a page asks `Shell` for. Emitted on the page's own entity; pages
/// never call `Shell` directly.
pub enum PageEvent {
    /// Show a toast (the same text a notify helper would print).
    Toast { message: String, error: bool },
    /// A write landed: refresh the shared state the domains feed and keep
    /// the shown page (what `Shell::changed` does).
    Changed(Domains),
    /// A rule write landed, carrying the use counts the write itself
    /// counted (3.55): the rules reload takes these instead of running the
    /// `rule_use_stats` table scan again. Same shape as
    /// [`financi_db::rules::reapply_rules_with_use`]'s result.
    ChangedWithRuleUse(
        Domains,
        std::collections::HashMap<i64, financi_db::rules::RuleUse>,
    ),
    /// Reload everything from the DB (launch, import, undo, restore).
    Refresh,
    /// Drop every page's transient state and re-read it from the DB (after
    /// a restore replaced the database under us).
    ResetPages,
    /// Navigate to another page.
    Go(crate::ui::Page),
}

/// The behaviour every extracted page has: visibility bookkeeping, a root
/// focus handle, and snapshot application.
pub trait PageEntity: Sized {
    /// The domains this page renders; `Catalog` events outside them are
    /// ignored.
    const DEPENDS: Domains;

    /// Whether the page is currently the shown page.
    fn visible(&self) -> bool;

    /// Whether a `DataChanged` in [`Self::DEPENDS`] arrived while hidden.
    fn dirty(&self) -> bool;

    /// Set the dirty flag; `set_visible(false)` raises it itself.
    fn set_dirty(&mut self, dirty: bool);

    /// Record whether the page is the shown one; called by the default
    /// [`Self::set_visible`] before it applies or dirties.
    fn set_shown(&mut self, visible: bool);

    /// Raise the "focus my root when the frame lands" flag that `render`
    /// consumes (see the module docs).
    fn set_focus_pending(&mut self, pending: bool);

    /// Pull the current snapshot into the page's own state. Runs when
    /// shown after a change and on every `DataChanged` in
    /// [`Self::DEPENDS`] while shown.
    fn apply(&mut self, cx: &mut Context<Self>);

    /// Called by `Shell` when the page is shown or hidden. Showing applies
    /// the current snapshot when it changed while hidden and asks the
    /// page's `render` to focus the root; hiding marks the page dirty so
    /// showing it again catches up.
    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.set_shown(visible);
        if visible {
            self.apply_if_dirty(cx);
            self.set_focus_pending(true);
        } else {
            self.set_dirty(true);
        }
    }

    /// Apply now if a `DataChanged` arrived while the page was hidden.
    /// Call before acting on mirrored state from a path that can fire
    /// while the page is not shown (a global action, for one).
    fn apply_if_dirty(&mut self, cx: &mut Context<Self>) {
        if self.dirty() {
            self.set_dirty(false);
            self.apply(cx);
        }
    }
}

/// Subscribes the page being built to `catalog`: snapshot events in
/// [`PageEntity::DEPENDS`] apply while the page is shown and mark it
/// dirty otherwise. Every page's `new` calls this and stores the returned
/// [`Subscription`] — dropping it would cancel the subscription.
pub fn watch_catalog<P: PageEntity + 'static>(
    catalog: &Entity<Catalog>,
    cx: &mut Context<'_, P>,
) -> Subscription {
    cx.subscribe(catalog, move |page, _, event: &DataChanged, cx| {
        if !event.domains.intersects(P::DEPENDS) {
            return;
        }
        if page.visible() {
            page.apply(cx);
        } else {
            page.set_dirty(true);
        }
        cx.notify();
    })
}
