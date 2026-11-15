//! The To categorize page (3.62, page entity in 3.83).

use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;

/// The To-categorize columns (3.22): heading row and row bodies share these
/// so each heading sits over its column, and Merchant — the one flex column
/// — absorbs a narrow window instead of pushing the picker and the
/// Categorize button past the app's minimum width.
const QUEUE_TXNS_W: f32 = 96.;
const QUEUE_TOTAL_W: f32 = 110.;
const QUEUE_DATES_W: f32 = 190.;
const QUEUE_PICKER_W: f32 = 220.;

/// 3.83: how far outside the rendered range a picker survives a prune — a
/// screenful (24 rows at 40 px), so scrolling back by less than a page
/// never drops a row's picker or its pick.
const QUEUE_PICKER_MARGIN: usize = 24;

/// The To-categorize page (3.83): the pending list grouped by merchant,
/// the per-visible-row category pickers and the error banner. The list is
/// a mirror of the `Catalog` snapshot; `apply` refreshes it and retires
/// pickers whose merchant left the queue.
pub(super) struct QueuePage {
    db: DbWorker,
    /// The shared data model (3.78); `apply` reads the queue and the
    /// category tree off its snapshot, never a query of its own.
    catalog: Entity<Catalog>,
    /// 5.3 review fix: shared with the shell — while an export or restore
    /// is in flight every write here is refused (the group's row ids may
    /// name different rows after it).
    writes_blocked: Rc<Cell<bool>>,
    pub(super) focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// The `Catalog` subscription (3.83); lives as long as the page.
    _catalog: Subscription,
    // -- Page state (3.74).
    /// The queue: uncategorized transactions grouped by merchant, newest
    /// first (as [`uncategorized_by_merchant`] returns them).
    pub(super) groups: Vec<UncategorizedGroup>,
    /// 3.19: pickers exist only for rows `render` has built, and 3.83
    /// prunes the ones that drift outside the rendered range, so this map
    /// stays the size of the visible slice, not the queue.
    pub(super) pickers: HashMap<String, Entity<SelectState<Names>>>,
    /// The category list the pickers above currently hold; `apply` re-sets
    /// items only when it changes, not on every refresh.
    pub(super) picker_items: Option<Names>,
    /// 3.34: one per live picker — a `Confirm` re-renders the rows so the
    /// row's Categorize button flips from disabled to enabled the frame a
    /// category is picked. Retired with its picker in `apply` and in
    /// [`Self::prune_pickers`].
    pub(super) picker_subs: HashMap<String, Subscription>,
    /// 3.83: `apply` has no window, and `SelectState::set_items` needs one,
    /// so a category-list change is flagged here and the pickers catch up
    /// at the top of `render`.
    pickers_need_items: bool,
    /// The banner text above the list.
    pub(super) error: Option<String>,
    /// 3.21: the virtualized list's scroll handle, alive as long as the
    /// page (see the note on `tree_scroll` in 3.21).
    pub(super) scroll: VirtualListScrollHandle,
    /// 3.21: the range of rows the queue built last frame, for the
    /// long-queue tests (the prune reads the list's own range argument).
    #[cfg(test)]
    pub(super) range: std::cell::Cell<(usize, usize)>,
    /// 3.19: how many queue rows the virtualized list built for the last
    /// layout pass — the visible slice, not the whole queue. Observability
    /// for the long-queue tests (like `tree_rows_built`).
    #[cfg(test)]
    pub(super) rows_built: std::cell::Cell<usize>,
    /// "Parent › Child" labels of the child categories, in picker order —
    /// what the pickers offer and what a row's Categorize resolves its
    /// pick against.
    pub(super) child_choices: Vec<(i64, String)>,
}

impl EventEmitter<PageEvent> for QueuePage {}

impl PageEntity for QueuePage {
    /// The list renders the uncategorized transactions and the category
    /// labels; the writes here (a new rule plus its re-run) move both.
    const DEPENDS: Domains = Domains::CATEGORIES_RULES_TX;

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
        let (groups, tree) = {
            let cat = self.catalog.read(cx);
            (cat.snap.queue.clone(), cat.snap.tree.clone())
        };
        self.groups = groups;
        self.child_choices = tree
            .iter()
            .flat_map(|p| {
                p.children
                    .iter()
                    .map(move |c| (c.id, format!("{} › {}", p.name, c.name)))
            })
            .collect();
        // 3.19: a refresh that dropped a group drops its picker; 3.34: the
        // Categorize-mirror subscription retires with it.
        let current: HashSet<&str> = self.groups.iter().map(|g| g.merchant.as_str()).collect();
        self.pickers
            .retain(|merchant, _| current.contains(merchant.as_str()));
        self.picker_subs
            .retain(|merchant, _| current.contains(merchant.as_str()));
        // 3.19: re-set the survivors' items only when the category list
        // changed, not on every refresh — and 3.83: `set_items` needs a
        // window, so flag it and let `render` push the new items.
        let names: Names = self
            .child_choices
            .iter()
            .map(|(_, n)| SharedString::from(n.clone()))
            .collect();
        if self.picker_items.as_ref() != Some(&names) {
            self.picker_items = Some(names);
            self.pickers_need_items = true;
        }
    }
}

impl QueuePage {
    pub(super) fn new(
        db: DbWorker,
        writes_blocked: Rc<Cell<bool>>,
        catalog: &Entity<Catalog>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _catalog = watch_catalog(catalog, cx);
        Self {
            db,
            catalog: catalog.clone(),
            writes_blocked,
            focus: cx.focus_handle(),
            // `Shell::new` marks the initial page visible right after
            // construction; every page starts hidden.
            visible: false,
            dirty: true,
            focus_pending: false,
            _catalog,
            groups: Vec::new(),
            pickers: HashMap::new(),
            picker_items: None,
            picker_subs: HashMap::new(),
            pickers_need_items: false,
            error: None,
            scroll: VirtualListScrollHandle::new(),
            #[cfg(test)]
            range: std::cell::Cell::new((0, 0)),
            #[cfg(test)]
            rows_built: std::cell::Cell::new(0),
            child_choices: Vec::new(),
        }
    }

    /// 5.3: a restore replaced the database — every picker selection names
    /// a row or category that may not exist in the new file.
    pub(super) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
        self.pickers.clear();
        self.picker_subs.clear();
        self.picker_items = None;
        self.pickers_need_items = false;
        self.error = None;
        cx.notify();
    }

    /// 3.19: the picker for one queue row, built on first use. Only visible
    /// rows render, so the page holds a picker for what's on screen — a
    /// new one gets the current category list.
    pub(super) fn queue_picker(
        &mut self,
        merchant: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Names>> {
        if let Some(picker) = self.pickers.get(merchant) {
            return picker.clone();
        }
        let items: Names = self.picker_items.clone().unwrap_or_else(|| {
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
        self.pickers.insert(merchant.to_string(), picker.clone());
        self.picker_subs.insert(merchant.to_string(), sub);
        picker
    }

    /// 3.83: pickers are built with their rows, so they must also go when
    /// their rows stop being built: keep the pickers whose merchant sits in
    /// `[start - margin, end + margin]` of the range the list just drew,
    /// plus any picker holding an active selection — a pick made further up
    /// the list is a pending action, and the user may be mid-interaction
    /// with it. A pruned picker rebuilds lazily from the current category
    /// list the next time its row renders.
    fn prune_pickers(&mut self, start: usize, end: usize, cx: &App) {
        let lo = start.saturating_sub(QUEUE_PICKER_MARGIN);
        let hi = (end + QUEUE_PICKER_MARGIN).min(self.groups.len());
        let keep: HashSet<&str> = self.groups[lo..hi]
            .iter()
            .map(|g| g.merchant.as_str())
            .collect();
        self.pickers.retain(|merchant, picker| {
            keep.contains(merchant.as_str()) || picker.read(cx).selected_value().is_some()
        });
        // 3.34: the Categorize-mirror subscriptions retire with pickers.
        self.picker_subs
            .retain(|merchant, _| self.pickers.contains_key(merchant.as_str()));
    }

    /// Categorize a whole merchant group: creates the substring rule and
    /// re-runs the rules (2.7's `categorize_group`).
    pub(super) fn categorize_merchant(
        &mut self,
        merchant: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in
        // flight — the group's row ids may name different rows after it.
        if self.writes_blocked.get() {
            return;
        }
        let Some(picker) = self.pickers.get(&merchant).cloned() else {
            return;
        };
        let Some(ix) = picker.read(cx).selected_index(cx) else {
            self.error = Some("Choose a category for this merchant first.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        let db = self.db.clone();
        let merchant_for_db = merchant.clone();
        // 3.64: detached — a later action must not cancel this write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                categorize_group(c, &merchant_for_db, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(_rule_id) => {
                        cx.emit(PageEvent::Toast {
                            message: format!("Categorized {merchant}"),
                            error: false,
                        });
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for QueuePage {
    /// The To-categorize page (3.16, 3.19, 3.22, 3.34): header summary, one
    /// bordered panel holding the heading row and the virtualized list of
    /// merchant groups, each row with its category picker and Categorize
    /// button.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 3.83: focusing the root when the page is shown waits for the end
        // of the frame so its element exists first (as the other pages).
        if self.focus_pending {
            self.focus_pending = false;
            let focus = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        // 3.83: a category-list change from `apply` lands here, where the
        // window exists for `set_items`.
        if self.pickers_need_items {
            self.pickers_need_items = false;
            if let Some(items) = self.picker_items.clone() {
                for picker in self.pickers.values() {
                    picker.update(cx, |s, cx| s.set_items(items.clone(), window, cx));
                }
            }
        }
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
            flow::count(self.groups.iter().map(|g| g.count).sum()),
            flow::count(self.groups.len() as i64)
        );
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
        let body: AnyElement = if self.groups.is_empty() {
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
            let sizes = Rc::new(vec![size(px(0.), px(40.)); self.groups.len()]);
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
                            let groups = this.groups[range.clone()].to_vec();
                            let last = this.groups.len().saturating_sub(1);
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
                                                                    "queue:txns-text:{m}:{}",
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
                                                            "queue:categorize-state:{m}:{}",
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
                            // 3.83: rows that stopped being built retire
                            // their pickers, so the map tracks the window.
                            this.prune_pickers(range.start, range.end, cx);
                            #[cfg(test)]
                            this.rows_built.set(rows.len());
                            #[cfg(test)]
                            this.range.set(range_bounds);
                            rows
                        },
                    )
                    // 3.21: page-owned handle (see the tree).
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h_0(),
                )
                .into_any_element()
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            // 3.83: the page's own key context, so Escape is *this* page's
            // Cancel (`ui::init` binds it per context); the root takes
            // focus when the page is shown.
            .track_focus(&self.focus)
            .key_context("Queue")
            .on_action(cx.listener(|this, _: &Cancel, _, cx| {
                // Escape unwinds one level per press (3.80,
                // *Interaction*): the error banner first; with nothing
                // left it propagates to the window-level bindings.
                if this.error.take().is_some() {
                    cx.notify();
                    return;
                }
                cx.propagate();
            }))
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
                            .debug_selector({
                                let summary = summary.clone();
                                move || format!("queue:summary:{summary}")
                            })
                            .child(summary),
                    ),
            )
            .children(self.error.clone().map(|e| {
                div()
                    .text_sm()
                    .text_color(danger)
                    .debug_selector({
                        let e = e.clone();
                        move || format!("queue:error:{e}")
                    })
                    .child(e)
            }))
            .child(body)
            .into_any_element()
    }
}
