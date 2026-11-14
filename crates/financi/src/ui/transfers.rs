//! The Transfers page (3.62).

use super::*;
use crate::ui::widgets::*;

/// This page's state (3.74): the open transfer reviews left behind by
/// pairing. The sidebar pill counts them; the page lists each with its
/// candidates.
pub(super) struct TransfersState {
    pub(super) transfer_reviews: Vec<TransferReview>,
}

impl TransfersState {
    pub(super) fn new(_window: &mut Window, _cx: &mut Context<Shell>) -> Self {
        Self {
            transfer_reviews: Vec::new(),
        }
    }
}

impl Shell {
    /// Apply this page's part of a refresh result (3.74).
    pub(super) fn apply_transfers(&mut self, reviews: Vec<TransferReview>) {
        self.transfers.transfer_reviews = reviews;
    }
}

impl Shell {
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
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.notify_ok("Paired transfer.", cx),
                    Err(e) => this.notify_err(format!("Could not pair: {e}"), cx),
                }
                this.changed(Domains::TRANSFERS_TX, window, cx);
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
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.notify_ok("Marked as not a transfer.", cx),
                    Err(e) => this.notify_err(format!("Could not update: {e}"), cx),
                }
                this.changed(Domains::TRANSFERS_TX, window, cx);
            });
        })
        .detach();
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
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.notify_ok("Unpaired transaction.", cx),
                    Err(e) => this.notify_err(format!("Could not unpair: {e}"), cx),
                }
                this.changed(Domains::TRANSFERS_TX, window, cx);
            });
        })
        .detach();
    }

    /// The Transfers page (4.2): one bordered panel per open review — the
    /// row under review, then every candidate with a *Pair* button, then a
    /// *Not a transfer* opt-out. No banner, no sheet (ui-conventions,
    /// *Pages*, D8); the sidebar pill carries the count.
    pub(super) fn render_transfers(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, border, radius) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.border, theme.radius)
        };
        let reviews = self.transfers.transfer_reviews.clone();
        let body: AnyElement = if reviews.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child("No transfer reviews yet.")
                .into_any_element()
        } else {
            v_flex()
                .id("transfer-reviews")
                .flex_1()
                .min_h_0()
                .gap_3()
                .overflow_y_scroll()
                .children(reviews.into_iter().map(|r| {
                    let txn = r.transaction_id;
                    let candidates = r.candidates.clone();
                    v_flex()
                        .gap_2()
                        .p_4()
                        .rounded(radius)
                        .border_1()
                        .border_color(border)
                        .debug_selector(move || format!("transfers:review:{txn}"))
                        // The row under review.
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(r.account.clone()),
                                )
                                .child(div().text_color(muted).child(r.posted_date.clone()))
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(r.description.clone()),
                                )
                                .child(div().flex_1())
                                .child(div().flex_none().child(flow::money(r.amount_cents))),
                        )
                        .child(div().text_xs().text_color(muted).child("Pair with one of:"))
                        // Every candidate offers one *Pair* button.
                        .children(candidates.into_iter().map(|c| {
                            let cand = c.id;
                            h_flex()
                                .gap_3()
                                .items_center()
                                .debug_selector(move || format!("transfers:candidate:{txn}:{cand}"))
                                .child(div().flex_none().w(px(110.)).child(c.posted_date))
                                .child(
                                    div()
                                        .w(px(200.))
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(c.account),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(c.description),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(120.))
                                        .text_right()
                                        .child(flow::money(c.amount_cents)),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("pair-{txn}-{cand}")))
                                        .label("Pair")
                                        .compact()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.pair_transfer(txn, cand, window, cx);
                                        })),
                                )
                        }))
                        // Opt out: never offer this row again.
                        .child(
                            Button::new(SharedString::from(format!("not-transfer-{txn}")))
                                .label("Not a transfer")
                                .link()
                                .compact()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.mark_not_transfer(txn, window, cx);
                                })),
                        )
                }))
                .into_any_element()
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Transfers"),
                    )
                    .child(div().text_sm().text_color(muted).child(format!(
                        "{} to review",
                        flow::count(self.transfers.transfer_reviews.len() as i64)
                    ))),
            )
            .child(body)
            .into_any_element()
    }
}
