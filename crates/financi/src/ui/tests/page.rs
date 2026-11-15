//! Tests for the 3.79 page-entity pattern (`ui/page.rs`): a hidden page
//! defers applying until it shows, `DEPENDS` filters what a page reacts
//! to, and `Cancel` reaches the page's own dismiss behaviour.

use super::*;
use crate::ui::catalog::{DataChanged, Domains};
use crate::ui::page::{PageEntity, watch_catalog};

use super::imports::seed_three_imports;

/// A minimal [`PageEntity`] to test the pattern itself: it counts its
/// applies and depends only on transfers.
struct OnlyTransfers {
    visible: bool,
    dirty: bool,
    applied: usize,
}

impl PageEntity for OnlyTransfers {
    const DEPENDS: Domains = Domains::TRANSFERS;
    fn visible(&self) -> bool {
        self.visible
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }
    fn set_shown(&mut self, visible: bool) {
        self.visible = visible;
    }
    fn set_focus_pending(&mut self, _pending: bool) {}
    fn apply(&mut self, _cx: &mut gpui_kit::Context<Self>) {
        self.applied += 1;
    }
}

fn data_changed(domains: Domains) -> DataChanged {
    DataChanged {
        domains,
        generation: 1,
        error: None,
        startup: None,
    }
}

#[gpui::test]
fn a_page_listens_only_to_the_domains_it_declares(cx: &mut TestAppContext) {
    let (_handle, shell) = open_shell(cx, |_| {});
    wait_idle(cx, &shell);
    let catalog = cx.update(|cx| shell.read(cx).catalog.clone());
    let page = cx.new(|_cx| OnlyTransfers {
        visible: true,
        dirty: false,
        applied: 0,
    });
    let _watch = page.update(cx, |_, cx| watch_catalog(&catalog, cx));

    // A change outside `DEPENDS` is ignored: neither applied nor dirtied.
    cx.update(|cx| {
        catalog.update(cx, |_, cx| {
            cx.emit(data_changed(Domains::CATEGORIES_RULES_TX))
        })
    });
    cx.update(|cx| {
        let p = page.read(cx);
        assert!(!p.dirty && p.applied == 0, "unrelated domains are filtered");
    });

    // Its own domain applies at once while shown…
    cx.update(|cx| catalog.update(cx, |_, cx| cx.emit(data_changed(Domains::TRANSFERS))));
    cx.update(|cx| assert_eq!(page.read(cx).applied, 1, "shown: applied at once"));

    // …and only marks the wait while hidden, until the show applies.
    page.update(cx, |p, cx| p.set_visible(false, cx));
    cx.update(|cx| catalog.update(cx, |_, cx| cx.emit(data_changed(Domains::TRANSFERS))));
    cx.update(|cx| {
        let p = page.read(cx);
        assert!(p.dirty && p.applied == 1, "hidden: waits to apply");
    });
    page.update(cx, |p, cx| p.set_visible(true, cx));
    cx.update(|cx| {
        let p = page.read(cx);
        assert_eq!(p.applied, 2, "the show applied what it had waited for");
        assert!(!p.dirty, "and cleared the wait");
    });
}

#[gpui::test]
fn a_hidden_page_defers_its_data_until_it_shows(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    wait_idle(cx, &shell);

    // The shell opens on the dashboard. The catalog loaded the seeded
    // imports, but the hidden Imports page applied nothing and waits.
    cx.update(|cx| {
        assert_eq!(shell.read(cx).catalog.read(cx).snap.imports.len(), 3);
        let p = shell.read(cx).imports.read(cx);
        assert!(p.import_history.is_empty(), "hidden page has not applied");
        assert!(p.dirty(), "and waits to apply when shown");
        assert!(shell.read(cx).transfers.read(cx).dirty());
    });

    // A full reload while hidden lands in the catalog but not the page.
    with_window(cx, handle, |window, cx| {
        shell.update(cx, |s, cx| s.refresh(window, cx))
    });
    wait_idle(cx, &shell);
    cx.update(|cx| {
        let p = shell.read(cx).imports.read(cx);
        assert!(p.import_history.is_empty(), "still deferred");
        assert!(p.dirty(), "still waiting");
    });

    // Showing the page applies the snapshot in the same step.
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    cx.update(|cx| {
        let p = shell.read(cx).imports.read(cx);
        assert_eq!(p.import_history.len(), 3, "applying was synchronous");
        assert!(!p.dirty(), "the wait was cleared");
    });
}

#[gpui::test]
fn escape_closes_the_armed_undo_on_the_imports_page(cx: &mut TestAppContext) {
    let (handle, shell) = open_shell(cx, seed_three_imports);
    wait_idle(cx, &shell);
    cx.update(|cx| shell.update(cx, |s, cx| s.go_page(Page::Imports, cx)));
    wait_idle(cx, &shell);

    // Arm the newest row's Undo button (3.32): the first click arms, it
    // writes nothing until the second.
    with_window(cx, handle, |window, cx| {
        let id = shell.read(cx).imports.read(cx).import_history[0].id;
        shell.update(cx, |s, cx| {
            s.imports.update(cx, |p, cx| p.undo_import(id, window, cx))
        });
        assert_eq!(
            shell.read(cx).imports.read(cx).confirm_undo_import,
            Some(id),
            "first click only asks to confirm"
        );
    });

    // Escape on the page's root closes it again (3.79). Actions dispatch
    // along the focus path, so focus the root — as the page's own
    // show-time focus does for a user.
    let root = cx.update(|cx| shell.read(cx).imports.read(cx).focus.clone());
    with_window(cx, handle, |window, cx| window.focus(&root, cx));
    cx.simulate_keystrokes(handle, "escape");
    cx.update(|cx| {
        let p = shell.read(cx).imports.read(cx);
        assert_eq!(p.confirm_undo_import, None, "escape closed the confirm");
    });
}
