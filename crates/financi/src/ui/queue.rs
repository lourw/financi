//! The To categorize page (3.62).

use super::*;
use crate::ui::widgets::*;

/// The To-categorize columns (3.22): heading row and row bodies share these
/// so each heading sits over its column, and Merchant — the one flex column
/// — absorbs a narrow window instead of pushing the picker and the
/// Categorize button past the app's minimum width.
const QUEUE_TXNS_W: f32 = 96.;
const QUEUE_TOTAL_W: f32 = 110.;
const QUEUE_DATES_W: f32 = 190.;
const QUEUE_PICKER_W: f32 = 220.;

/// This page's state (3.74): the pending list grouped by merchant, the
/// per-visible-row category pickers and the error banner.
pub(super) struct QueueState {
    /// The queue: uncategorized transactions grouped by merchant, newest
    /// first (as [`uncategorized_by_merchant`] returns them).
    pub(super) groups: Vec<UncategorizedGroup>,
    /// 3.19: pickers exist only for rows `render_queue` has built, so this
    /// map stays the size of the visible slice, not the queue.
    pub(super) pickers: HashMap<String, Entity<SelectState<Names>>>,
    /// The category list the pickers above currently hold; `sync_queue`
    /// re-sets items only when it changes, not on every refresh.
    pub(super) picker_items: Option<Names>,
    /// 3.34: one per live picker — a `Confirm` re-renders the rows so the
    /// row's Categorize button flips from disabled to enabled the frame a
    /// category is picked. Retired with its picker in `sync_queue`.
    pub(super) picker_subs: HashMap<String, Subscription>,
    /// The banner text above the list.
    pub(super) error: Option<String>,
    /// 3.21: the virtualized list's scroll handle, alive as long as the
    /// `Shell` (see the note on `tree_scroll` in 3.21).
    pub(super) scroll: VirtualListScrollHandle,
    /// 3.19: how many queue rows the virtualized list built for the last
    /// layout pass — the visible slice, not the whole queue. Observability
    /// for the long-queue tests (like `tree_rows_built`).
    #[cfg(test)]
    pub(super) rows_built: std::cell::Cell<usize>,
    /// 3.21: the range of rows the queue built last frame.
    #[cfg(test)]
    pub(super) range: std::cell::Cell<(usize, usize)>,
}

impl QueueState {
    pub(super) fn new(_window: &mut Window, _cx: &mut Context<Shell>) -> Self {
        Self {
            groups: Vec::new(),
            pickers: HashMap::new(),
            picker_items: None,
            picker_subs: HashMap::new(),
            error: None,
            scroll: VirtualListScrollHandle::new(),
            #[cfg(test)]
            rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            range: std::cell::Cell::new((0, 0)),
        }
    }
}

impl Shell {
    /// Apply this page's part of a refresh result (3.74).
    pub(super) fn apply_queue(&mut self, groups: Vec<UncategorizedGroup>) {
        self.queue.groups = groups;
    }
}

impl Shell {
    /// 3.19: pickers are built lazily by `render_queue` for the rows it
    /// shows, never up front per group. This retires pickers whose group
    /// left the queue and re-sets the items on the survivors only when the
    /// category list changed, so a refresh doesn't touch every picker.
    pub(super) fn sync_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current: HashSet<&str> = self
            .queue
            .groups
            .iter()
            .map(|g| g.merchant.as_str())
            .collect();
        self.queue
            .pickers
            .retain(|merchant, _| current.contains(merchant.as_str()));
        // 3.34: a picker's Categorize-mirror subscription retires with it.
        self.queue
            .picker_subs
            .retain(|merchant, _| current.contains(merchant.as_str()));
        let names: Names = self
            .child_choices
            .iter()
            .map(|(_, n)| SharedString::from(n.clone()))
            .collect();
        if self.queue.picker_items.as_ref() != Some(&names) {
            self.queue.picker_items = Some(names.clone());
            for picker in self.queue.pickers.values() {
                let items = names.clone();
                picker.update(cx, |s, cx| s.set_items(items, window, cx));
            }
        }
    }

    /// 3.19: the picker for one queue row, built on first use. Only visible
    /// rows render, so the shell holds a picker for what's on screen — a
    /// new one gets the current category list.
    pub(super) fn queue_picker(
        &mut self,
        merchant: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Names>> {
        if let Some(picker) = self.queue.pickers.get(merchant) {
            return picker.clone();
        }
        let items: Names = self.queue.picker_items.clone().unwrap_or_else(|| {
            self.child_choices
                .iter()
                .map(|(_, n)| SharedString::from(n.clone()))
                .collect()
        });
        let picker = cx.new(|cx| SelectState::new(items, None, window, cx));
        // 3.34: the row's Categorize button mirrors the picker — disabled
        // with no pick, enabled with one — so a confirmed pick re-renders
        // the rows. The subscription lives exactly as long as the picker.
        let sub = cx.subscribe(&picker, |_, _, ev: &SelectEvent<Names>, cx| {
            if matches!(ev, SelectEvent::Confirm(_)) {
                cx.notify();
            }
        });
        self.queue
            .pickers
            .insert(merchant.to_string(), picker.clone());
        self.queue.picker_subs.insert(merchant.to_string(), sub);
        picker
    }

    /// Categorize a whole merchant group: creates the substring rule and
    /// re-runs the rules (2.7's `categorize_group`).
    pub(super) fn categorize_merchant(
        &mut self,
        merchant: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.queue.pickers.get(&merchant).cloned() else {
            return;
        };
        let Some(ix) = picker.read(cx).selected_index(cx) else {
            self.queue.error = Some("Choose a category for this merchant first.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        let db = self.db.clone();
        let merchant_for_db = merchant.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                categorize_group(c, &merchant_for_db, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_rule_id) => {
                        this.notify_ok(format!("Categorized {merchant}"), cx);
                        this.queue.error = None;
                    }
                    Err(e) => this.queue.error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    // -- row context menu (2.9) ---------------------------------------------

    pub(super) fn render_queue(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, danger, border, list_hover, radius) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.danger,
                theme.border,
                theme.list_hover,
                theme.radius,
            )
        };
        // 3.34: the header summary counts what the list below shows —
        // transactions and merchants, both through `flow::count`. Its
        // debug selector records the exact text so a test can pin it.
        let summary = format!(
            "{} transactions · {} merchants",
            flow::count(self.queue.groups.iter().map(|g| g.count).sum()),
            flow::count(self.queue.groups.len() as i64)
        );
        let summary_sel = format!("queue:summary:{summary}");
        // 3.34: one bordered panel (D7: bordered, never filled) fills the
        // rest of the page and holds the list; the empty state is centered
        // inside it.
        let panel = v_flex()
            .flex_1()
            .min_h_0()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .overflow_hidden()
            .debug_selector(|| "queue:panel".to_string());
        let body: AnyElement = if self.queue.groups.is_empty() {
            panel
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .child("Nothing waiting — every transaction has a category."),
                )
                .into_any_element()
        } else {
            let panel = panel.child(
                // 3.16: a heading row labels every column, the category
                // picker included — its placeholder alone is not a label.
                // 3.34: a 32 px row inside the panel — `text_sm` muted,
                // `px_3`, with a bottom divider — whose children mirror the
                // queue rows below in order and width, down to an invisible
                // copy of the Categorize button at its new `small` size, so
                // each heading sits over its column (the picker's heading
                // would otherwise slide right by the button's width).
                // 3.22: the widths come from the QUEUE_*_W consts and
                // Merchant is the only flex column, so the whole row fits
                // the minimum window and the headings stay put.
                h_flex()
                    .h(px(32.))
                    .px_3()
                    .gap_3()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        heading_label(muted, "Merchant")
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis(),
                    )
                    .child(
                        heading_label(muted, "Transactions")
                            .w(px(QUEUE_TXNS_W))
                            .text_right(),
                    )
                    .child(
                        heading_label(muted, "Total")
                            .w(px(QUEUE_TOTAL_W))
                            .text_right(),
                    )
                    .child(heading_label(muted, "Dates").w(px(QUEUE_DATES_W)))
                    .child(heading_label(muted, "Category").w(px(QUEUE_PICKER_W)))
                    .child(
                        Button::new("queue-heading-categorize-spacer")
                            .label("Categorize")
                            .small()
                            .opacity(0.0)
                            .flex_shrink_0(),
                    ),
            );
            // 3.19: virtualized like the Categories-page lists (3.12). The
            // closure runs during layout with the visible range, so a queue
            // of thousands builds only the rows that can be on screen, and
            // `queue_picker` creates a picker only with its row. 3.34: rows
            // stay 40 px, now `px_3` with a bottom divider (none after the
            // last row) and `list_hover` on hover.
            let sizes = Rc::new(vec![size(px(0.), px(40.)); self.queue.groups.len()]);
            panel
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "queue-list",
                        sizes,
                        move |this, range, window, cx| {
                            #[cfg(test)]
                            let range_bounds = (range.start, range.end);
                            let start = range.start;
                            let groups = this.queue.groups[range].to_vec();
                            let last = this.queue.groups.len().saturating_sub(1);
                            let mut rows = Vec::with_capacity(groups.len());
                            for (i, g) in groups.into_iter().enumerate() {
                                let merchant = g.merchant.clone();
                                let picker = this.queue_picker(&merchant, window, cx);
                                rows.push(
                                    h_flex()
                                        // Fill the list's width, or the row
                                        // stays at its natural width and the
                                        // Merchant column never grows: the
                                        // picker and button would then float
                                        // left of the column headings.
                                        .w_full()
                                        .h(px(40.))
                                        .px_3()
                                        .gap_3()
                                        .items_center()
                                        // 3.34: `list_hover` on hover, and
                                        // a bottom divider between rows —
                                        // none after the last row, where
                                        // the panel's own border ends the
                                        // list.
                                        .hover(move |s| s.bg(list_hover))
                                        .when(start + i < last, |row| {
                                            row.border_b_1().border_color(border)
                                        })
                                        .debug_selector({
                                            let m = merchant.clone();
                                            move || format!("queue:row:{m}")
                                        })
                                        .child(
                                            // 3.22: Merchant is the flex
                                            // column; a name too long for
                                            // the leftover space truncates,
                                            // and the tooltip still shows
                                            // the full one. 3.34: normal
                                            // weight `foreground` — the
                                            // row's action is what stands
                                            // out, not its label.
                                            div()
                                                .id(SharedString::from(format!(
                                                    "queue-merchant-{merchant}"
                                                )))
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .debug_selector({
                                                    let m = merchant.clone();
                                                    move || {
                                                        format!("queue:merchant-col:{m}")
                                                    }
                                                })
                                                .tooltip({
                                                    let full = merchant.clone();
                                                    move |window, cx| {
                                                        let tip = full.clone();
                                                        Tooltip::element(move |_, _| {
                                                            div()
                                                                .debug_selector({
                                                                    let m = tip.clone();
                                                                    move || {
                                                                        format!(
                                                                            "queue:merchant-tooltip:{m}"
                                                                        )
                                                                    }
                                                                })
                                                                .child(tip.clone())
                                                        })
                                                        .build(window, cx)
                                                    }
                                                })
                                                .child(g.merchant.clone()),
                                        )
                                        .child(
                                            // The Transactions cell is just
                                            // the number (3.22); its debug
                                            // selector records the exact
                                            // text so a test can pin that.
                                            div()
                                                .w(px(QUEUE_TXNS_W))
                                                .text_right()
                                                .debug_selector({
                                                    let m = merchant.clone();
                                                    move || {
                                                        format!("queue:txns-col:{m}")
                                                    }
                                                })
                                                .child(
                                                    div()
                                                        .debug_selector({
                                                            let m = merchant.clone();
                                                            let n = g.count;
                                                            move || {
                                                                format!(
                                                                    "queue:txns-text:{m}={}",
                                                                    flow::count(n)
                                                                )
                                                            }
                                                        })
                                                        .child(flow::count(g.count)),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .w(px(QUEUE_TOTAL_W))
                                                .text_right()
                                                .debug_selector({
                                                    let m = merchant.clone();
                                                    move || {
                                                        format!("queue:total-col:{m}")
                                                    }
                                                })
                                                .child(flow::money(g.total_cents)),
                                        )
                                        .child(
                                            div()
                                                .w(px(QUEUE_DATES_W))
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .text_sm()
                                                .text_color(muted)
                                                .debug_selector({
                                                    let m = merchant.clone();
                                                    move || {
                                                        format!("queue:dates-col:{m}")
                                                    }
                                                })
                                                .child(flow::date_range(
                                                    &g.first_date,
                                                    &g.last_date,
                                                )),
                                        )
                                        .child(
                                            div()
                                                .w(px(QUEUE_PICKER_W))
                                                .flex_shrink_0()
                                                .debug_selector({
                                                    let m = merchant.clone();
                                                    move || {
                                                        format!("queue:picker-col:{m}")
                                                    }
                                                })
                                                .child(
                                                    Select::new(&picker).placeholder("Category…"),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .debug_selector({
                                                    let m = merchant.clone();
                                                    move || {
                                                        format!("queue:categorize-col:{m}")
                                                    }
                                                })
                                                .child({
                                                    // 3.34: quiet and
                                                    // disabled until the
                                                    // row's picker holds a
                                                    // category; `small` and
                                                    // outline once one is
                                                    // picked — row-level
                                                    // buttons are never
                                                    // primary (D12), so the
                                                    // page keeps its one
                                                    // primary and only rows
                                                    // ready to apply are
                                                    // clickable. The debug
                                                    // selector records the
                                                    // state for tests.
                                                    let picked =
                                                        picker.read(cx).selected_value().is_some();
                                                    let m = merchant.clone();
                                                    Button::new(SharedString::from(format!(
                                                        "categorize-{}",
                                                        g.merchant
                                                    )))
                                                    .label("Categorize")
                                                    .small()
                                                    .outline()
                                                    .disabled(!picked)
                                                    .debug_selector(move || {
                                                        format!(
                                                            "queue:categorize-state:{m}={}",
                                                            if picked { "enabled" } else { "disabled" }
                                                        )
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.categorize_merchant(
                                                                merchant.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    ))
                                                },
                                                ),
                                        ),
                                );
                            }
                            #[cfg(test)]
                            this.queue.rows_built.set(rows.len());
                            #[cfg(test)]
                            this.queue.range.set(range_bounds);
                            rows
                        },
                    )
                    // 3.21: Shell-owned handle (see the tree).
                    .track_scroll(&self.queue.scroll)
                    .flex_1()
                    .min_h_0(),
                )
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
                            .child("To categorize"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .debug_selector(move || summary_sel.clone())
                            .child(summary),
                    ),
            )
            .children(
                self.queue
                    .error
                    .clone()
                    .map(|e| div().text_sm().text_color(danger).child(e)),
            )
            .child(body)
            .into_any_element()
    }
}
