//! The Transfers page (3.62), a page entity since 3.79.

use std::collections::HashSet;

use financi_db::TransferCandidate;

use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;

/// The Transfers page (3.74 state, 3.79 entity): the open transfer reviews
/// left behind by pairing. The sidebar pill counts them (from the
/// `Catalog` snapshot, so a hidden page's mirror being stale is fine); the
/// page lists each with its candidates. 3.89: the list is a shared
/// [`DataList`] — one collapsible group per review — so the page also
/// remembers which groups are collapsed and the list's scroll position.
pub(super) struct TransfersPage {
    db: DbWorker,
    /// The shared data model (3.78); `apply` reads the review list off its
    /// snapshot, never a query of its own.
    catalog: Entity<Catalog>,
    /// 5.3 review fix: shared with the shell — while an export or restore
    /// is in flight every write here is refused (the row ids may name
    /// different transactions after it).
    writes_blocked: Rc<Cell<bool>>,
    focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// The `Catalog` subscription (3.79); lives as long as the page.
    _catalog: Subscription,
    // -- State (3.74).
    pub(super) transfer_reviews: Vec<TransferReview>,
    /// Review ids whose candidate rows are hidden (3.89).
    collapsed: HashSet<i64>,
    /// Keeps the list's scroll position across re-renders (3.89).
    pub(super) scroll: VirtualListScrollHandle,
}

impl EventEmitter<PageEvent> for TransfersPage {}

impl PageEntity for TransfersPage {
    /// The review list follows pairing writes and imports; the undo of an
    /// import re-runs the engine and re-opens reviews, so the history too.
    const DEPENDS: Domains = Domains::TRANSFERS.union(Domains::IMPORTS);

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

    fn set_focus_pending(&mut self, pending: bool) {
        self.focus_pending = pending;
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        self.transfer_reviews = self.catalog.read(cx).snap.reviews.clone();
    }
}

impl TransfersPage {
    pub(super) fn new(
        db: DbWorker,
        writes_blocked: Rc<Cell<bool>>,
        catalog: Entity<Catalog>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _catalog = watch_catalog(&catalog, cx);
        Self {
            db,
            catalog,
            writes_blocked,
            focus: cx.focus_handle(),
            // `Shell::new` marks the initial page visible right after
            // construction; every page starts hidden.
            visible: false,
            dirty: true,
            focus_pending: false,
            _catalog,
            transfer_reviews: Vec::new(),
            collapsed: HashSet::new(),
            scroll: VirtualListScrollHandle::new(),
        }
    }

    /// Resolve an open transfer review by pairing its row with `candidate`
    /// (Transfers page, 4.2). The engine re-runs on the same commit, so a
    /// fresh review list and table come back on the next refresh.
    pub(super) fn pair_transfer(
        &mut self,
        transaction_id: i64,
        candidate: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in
        // flight — the row ids may name different transactions after it.
        if self.writes_blocked.get() {
            return;
        }
        let db = self.db.clone();
        // 3.64: detached — a later action must not cancel this write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                resolve_transfer_review(c, transaction_id, Some(candidate))
            })
            .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(_) => this.toast("Paired transfer.", false, cx),
                    Err(e) => this.toast(&format!("Could not pair: {e}"), true, cx),
                }
                cx.emit(PageEvent::Changed(Domains::TRANSFERS_TX));
                cx.notify();
            });
        })
        .detach();
    }

    /// Mark an open review's row *not a transfer* (Transfers page, 4.2): the
    /// row is excluded from pairing for good and the review is dropped.
    pub(super) fn mark_not_transfer(
        &mut self,
        transaction_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let db = self.db.clone();
        // 3.64: detached — see `pair_transfer`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                resolve_transfer_review(c, transaction_id, None)
            })
            .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(_) => this.toast("Marked as not a transfer.", false, cx),
                    Err(e) => this.toast(&format!("Could not update: {e}"), true, cx),
                }
                cx.emit(PageEvent::Changed(Domains::TRANSFERS_TX));
                cx.notify();
            });
        })
        .detach();
    }

    /// 3.89: show or hide one review's candidate rows — the group
    /// header's chevron calls this through the [`DataList`] toggle.
    fn toggle_review(&mut self, transaction_id: i64, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&transaction_id) {
            self.collapsed.insert(transaction_id);
        }
        cx.notify();
    }

    /// Unpair a linked transaction from the row menu (4.2): both sides are
    /// cleared, rules re-apply, and the engine re-runs.
    pub(super) fn unpair_row(
        &mut self,
        transaction_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let db = self.db.clone();
        // 3.64: detached — see `pair_transfer`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| unpair_transaction(c, transaction_id)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(_) => this.toast("Unpaired transaction.", false, cx),
                    Err(e) => this.toast(&format!("Could not unpair: {e}"), true, cx),
                }
                cx.emit(PageEvent::Changed(Domains::TRANSFERS_TX));
                cx.notify();
            });
        })
        .detach();
    }

    /// Feedback goes to the shell's toast stack (3.79: a page never owns
    /// window chrome).
    fn toast(&mut self, message: &str, error: bool, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast {
            message: message.to_string(),
            error,
        });
    }
}

impl Render for TransfersPage {
    /// The Transfers page (4.2), on the shared design system (3.89): one
    /// panel holding one [`DataList`] — a collapsible group header per
    /// open review (description, candidate count, amount, and a *Not a
    /// transfer* action) over the review's candidate rows, each with a
    /// hover-revealed *Pair* button. No banner, no sheet (ui-conventions,
    /// *Pages*, D8); the sidebar pill carries the count.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 3.79: focusing the root when the page is shown waits for the end
        // of the frame so its element exists first (the Rules page's search
        // focus does the same, `rules_focus_pending`).
        if self.focus_pending {
            self.focus_pending = false;
            let focus = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        let muted = cx.theme().muted_foreground;
        let reviews = self.transfer_reviews.clone();
        let body: AnyElement = if reviews.is_empty() {
            panel(cx, false)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .child("No transfer reviews yet."),
                )
                .into_any_element()
        } else {
            // One flat candidate list for the row cells and actions to
            // index, and the list's lines: a group header per review
            // followed by its candidates unless the group is collapsed.
            let mut candidates: Vec<(i64, TransferCandidate)> = Vec::new();
            let mut items: Vec<DataItem> = Vec::new();
            for r in &reviews {
                let txn = r.transaction_id;
                let collapsed = self.collapsed.contains(&txn);
                items.push(DataItem::Group {
                    id: txn,
                    collapsed,
                    name: r.description.clone().into(),
                    count: Some(flow::count(r.candidates.len() as i64).into()),
                    selector: format!("transfers:review:{txn}"),
                });
                for c in &r.candidates {
                    candidates.push((txn, c.clone()));
                    if !collapsed {
                        items.push(DataItem::Row {
                            id: candidates.len() - 1,
                            selected: false,
                            selector: format!("transfers:candidate:{txn}:{}", c.id),
                        });
                    }
                }
            }
            // Amounts per group, for the header's trailing amount.
            let review_amounts: Vec<(i64, i64)> = reviews
                .iter()
                .map(|r| (r.transaction_id, r.amount_cents))
                .collect();
            let columns = vec![
                DataCol::new("Date", Col::Fixed(110.), ColAlign::Left),
                DataCol::new("Account", Col::Fixed(200.), ColAlign::Left),
                DataCol::new("Description", Col::Flex, ColAlign::Left),
                DataCol::new("Amount", Col::Fixed(120.), ColAlign::Right),
            ];
            let candidates = Rc::new(candidates);
            let review_amounts = Rc::new(review_amounts);
            let list = DataList::new(cx.entity(), columns, items)
                .cells({
                    let candidates = candidates.clone();
                    move |_, item, _measuring, _, _| match item {
                        DataItem::Row { id, .. } => {
                            let c = &candidates[*id].1;
                            vec![
                                div().child(c.posted_date.clone()).into_any_element().into(),
                                // Account and description are text cells:
                                // the list ellipsizes them and attaches
                                // the full-text tooltip (3.89 review).
                                ListCell::text(c.account.clone()),
                                ListCell::text(c.description.clone()),
                                div()
                                    .child(flow::money(c.amount_cents))
                                    .into_any_element()
                                    .into(),
                            ]
                        }
                        DataItem::Group { id, .. } => {
                            let amount = review_amounts
                                .iter()
                                .find(|(t, _)| *t == *id)
                                .map(|(_, a)| *a)
                                .unwrap_or(0);
                            vec![div().child(flow::money(amount)).into_any_element().into()]
                        }
                    }
                })
                .scroll(self.scroll.clone())
                .heading_selectors("transfers")
                .selector("transfers:list")
                .toggle(|this, id, window, cx| this.toggle_review(id, window, cx))
                .actions(120., {
                    let candidates = candidates.clone();
                    move |_, item, _, cx| match item {
                        DataItem::Row { id, .. } => {
                            let txn = candidates[*id].0;
                            let cand = candidates[*id].1.id;
                            vec![
                                Button::new(SharedString::from(format!("pair-{txn}-{cand}")))
                                    .label("Pair")
                                    .xsmall()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.pair_transfer(txn, cand, window, cx);
                                    }))
                                    .into_any_element(),
                            ]
                        }
                        DataItem::Group { id, .. } => {
                            let txn = *id;
                            vec![
                                Button::new(SharedString::from(format!("not-transfer-{txn}")))
                                    .label("Not a transfer")
                                    .ghost()
                                    .xsmall()
                                    .tooltip("Never offer this row for pairing")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.mark_not_transfer(txn, window, cx);
                                    }))
                                    .into_any_element(),
                            ]
                        }
                    }
                });
            panel(cx, false).child(list).into_any_element()
        };
        // 3.79: the page's own key context, so Escape is *this* page's
        // Cancel (`ui::init` binds it per context); the root takes focus
        // when the page is shown.
        v_flex()
            .track_focus(&self.focus)
            .key_context("Transfers")
            .on_action(cx.listener(|_, _: &Cancel, _, cx| {
                // Nothing armed and no transient UI on this page (3.79);
                // the binding exists so the page owns Escape.
                cx.propagate();
            }))
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(page_header(
                cx,
                "Transfers",
                format!(
                    "{} to review",
                    flow::count(self.transfer_reviews.len() as i64)
                ),
                Vec::new(),
            ))
            .child(body)
            .into_any_element()
    }
}
