//! The To categorize page (3.62, page entity in 3.83). 3.91: the list is
//! the shared `DataList` inside a `panel()` under a `page_header()`.

use std::cell::RefCell;

use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;

/// The To-categorize columns (3.22, on the `DataList` in 3.91): heading row
/// and rows share the column definitions, and Merchant — the one flex
/// column — absorbs a narrow window instead of pushing the picker and the
/// Categorize button past the app's minimum width.
const QUEUE_TXNS_W: f32 = 96.;
const QUEUE_TOTAL_W: f32 = 110.;
const QUEUE_DATES_W: f32 = 190.;
const QUEUE_PICKER_W: f32 = 220.;
/// The `small` Categorize button's column, wide enough for the button so
/// the flex Merchant column is what gives at the minimum window (3.16's
/// heading-over-column rule now holds by construction: the heading and the
/// rows share these widths).
const QUEUE_CATEGORIZE_W: f32 = 110.;

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
    /// 3.19: pickers exist only for rows the list has built, and 3.83
    /// prunes the ones that drift outside the rendered range, so this map
    /// stays the size of the visible slice, not the queue. 3.91: the
    /// `DataList` paints rows through `Fn(&V, …)`, so the map lives behind
    /// a `RefCell` — the cells callback gets or creates pickers through it
    /// with only `&self`.
    pub(super) pickers: RefCell<HashMap<String, Entity<SelectState<SubChoices>>>>,
    /// The grouped category items the pickers above currently hold (the
    /// `Catalog` Rc pushed into them); `apply` re-sets items only when
    /// that pointer changes, not on every refresh.
    pub(super) picker_items: Option<Rc<Vec<SelectGroup<SubChoice>>>>,
    /// 3.34: one per live picker — a `Confirm` re-renders the rows so the
    /// row's Categorize button flips from disabled to enabled the frame a
    /// category is picked. Retired with its picker in `apply` and in
    /// [`Self::prune_visible`].
    pub(super) picker_subs: RefCell<HashMap<String, Subscription>>,
    /// 3.83: `apply` has no window, and `SelectState::set_items` needs one,
    /// so a category-list change is flagged here and the pickers catch up
    /// at the top of `render`.
    pickers_need_items: bool,
    /// The banner text above the list.
    pub(super) error: Option<String>,
    /// 3.21: the virtualized list's scroll handle, alive as long as the
    /// page (see the note on `tree_scroll` in 3.21).
    pub(super) scroll: VirtualListScrollHandle,
    /// 3.91: `[painted_lo, painted_hi)` — the group indices the current
    /// paint pass builds; the prune window (3.83). H4: set from the
    /// exact pass range `DataList::on_pass` reports.
    painted_lo: Cell<usize>,
    painted_hi: Cell<usize>,
    /// 3.21: the range of rows the queue built last frame, for the
    /// long-queue tests (3.91: folded up from the `DataList` cells calls).
    #[cfg(test)]
    pub(super) range: std::cell::Cell<(usize, usize)>,
    /// 3.19: how many queue rows the virtualized list built for the last
    /// layout pass — the visible slice, not the whole queue. Observability
    /// for the long-queue tests (like `tree_rows_built`).
    #[cfg(test)]
    pub(super) rows_built: std::cell::Cell<usize>,
    /// "Parent › Child" labels of the child categories, in picker order —
    /// the flat mirror of the tree the tests read; the pickers offer the
    /// same children grouped by category (3.59) and resolve a pick by the
    /// item's id value.
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
        let (groups, tree, choices) = {
            let cat = self.catalog.read(cx);
            (
                cat.snap.queue.clone(),
                cat.snap.tree.clone(),
                cat.cat_choices(),
            )
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
            .borrow_mut()
            .retain(|merchant, _| current.contains(merchant.as_str()));
        self.picker_subs
            .borrow_mut()
            .retain(|merchant, _| self.pickers.borrow().contains_key(merchant.as_str()));
        // 3.19: re-set the survivors' items only when the category list
        // changed, not on every refresh — and 3.83: `set_items` needs a
        // window, so flag it and let `render` push the new items.
        // 3.59: the items are the shared grouped list; a rebuild in
        // `Catalog` hands out a fresh `Rc`, so a pointer compare is the
        // change test.
        if self
            .picker_items
            .as_ref()
            .is_none_or(|held| !Rc::ptr_eq(held, &choices))
        {
            self.picker_items = Some(choices);
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
            pickers: RefCell::new(HashMap::new()),
            picker_items: None,
            picker_subs: RefCell::new(HashMap::new()),
            pickers_need_items: false,
            error: None,
            scroll: VirtualListScrollHandle::new(),
            painted_lo: Cell::new(0),
            painted_hi: Cell::new(0),
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
        self.pickers.borrow_mut().clear();
        self.picker_subs.borrow_mut().clear();
        self.picker_items = None;
        self.pickers_need_items = false;
        self.error = None;
        cx.notify();
    }

    /// 3.19: the picker for one queue row, built on first use. Only visible
    /// rows render, so the page holds a picker for what's on screen — a
    /// new one gets the current category list. 3.91: the `DataList` cells
    /// callback only has `&self`, so the map is a `RefCell` and this takes
    /// `&self` too; `queue_picker` is the same call for tests and callers
    /// holding `&mut`.
    fn picker_for_row(
        &self,
        merchant: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SubChoices>> {
        let existing = self.pickers.borrow().get(merchant).cloned();
        if let Some(picker) = existing {
            return picker;
        }
        // 3.59: the shared grouped items — every row's picker offers the
        // same list, searchable, and each owns its own filter state.
        let items = self
            .picker_items
            .clone()
            .unwrap_or_else(|| self.catalog.read(cx).cat_choices());
        let picker = cx.new(|cx| {
            SelectState::new(SearchableVec::new((*items).clone()), None, window, cx)
                .searchable(true)
        });
        // 3.34: the row's Categorize button mirrors the picker — disabled
        // with no pick, enabled with one — so a confirmed pick re-renders
        // the rows. The subscription lives exactly as long as the picker.
        let sub = cx.subscribe(&picker, |_, _, ev: &SelectEvent<SubChoices>, cx| {
            if matches!(ev, SelectEvent::Confirm(_)) {
                cx.notify();
            }
        });
        self.pickers
            .borrow_mut()
            .insert(merchant.to_string(), picker.clone());
        self.picker_subs
            .borrow_mut()
            .insert(merchant.to_string(), sub);
        picker
    }

    /// 3.19: the picker for one queue row (see [`Self::picker_for_row`]).
    /// 3.91: rows build their picker through `picker_for_row` directly, so
    /// this entry point is the tests' (they drive a pick the way the
    /// popup's `Confirm` would).
    #[cfg(test)]
    pub(super) fn queue_picker(
        &mut self,
        merchant: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<SubChoices>> {
        self.picker_for_row(merchant, window, cx)
    }

    /// 3.91: record the lines one `DataList` paint pass builds. H4:
    /// `DataList::on_pass` hands over the exact range — the range
    /// argument the old `v_virtual_list` closure got — so the window is
    /// set, not inferred from the row ids: an exact-page scroll (PageDown
    /// at a viewport height that's a multiple of the 40 px row)
    /// continues the ids with no break, and the old sequence heuristic
    /// let the pass, and the prune window below it, grow unbounded.
    /// `pub(super)` so the tests can drive passes the list would report.
    pub(super) fn note_paint_pass(&self, range: std::ops::Range<usize>) {
        self.painted_lo.set(range.start);
        self.painted_hi.set(range.end);
        #[cfg(test)]
        {
            self.rows_built.set(range.len());
            self.range.set((range.start, range.end));
        }
    }

    /// 3.83: pickers are built with their rows, so they must also go when
    /// their rows stop being built: keep the pickers whose merchant sits in
    /// `[painted_lo - margin, painted_hi + margin]` of the range the list
    /// just drew, plus any picker holding an active selection — a pick made
    /// further up the list is a pending action, and the user may be
    /// mid-interaction with it. A pruned picker rebuilds lazily from the
    /// current category list the next time its row renders. 3.91: the
    /// cells callback is where the page learns what painted, so the prune
    /// runs there too, once per built row; H4: the window is the exact
    /// pass range, set before any row builds, so the survivors match the
    /// old once-per-frame prune. `pub(super)` for the same tests.
    pub(super) fn prune_visible(&self, cx: &App) {
        if self.pickers.borrow().is_empty() {
            return;
        }
        let lo = self.painted_lo.get().saturating_sub(QUEUE_PICKER_MARGIN);
        let hi = (self.painted_hi.get() + QUEUE_PICKER_MARGIN).min(self.groups.len());
        let keep: HashSet<&str> = self.groups[lo..hi]
            .iter()
            .map(|g| g.merchant.as_str())
            .collect();
        self.pickers.borrow_mut().retain(|merchant, picker| {
            keep.contains(merchant.as_str()) || picker.read(cx).selected_value().is_some()
        });
        // 3.34: the Categorize-mirror subscriptions retire with pickers.
        self.picker_subs
            .borrow_mut()
            .retain(|merchant, _| self.pickers.borrow().contains_key(merchant.as_str()));
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
        let Some(picker) = self.pickers.borrow().get(&merchant).cloned() else {
            return;
        };
        // 3.59: the grouped items carry the sub-category id as their
        // value, so the pick resolves directly — no positional lookup.
        let Some(category_id) = picker.read(cx).selected_value().copied() else {
            self.error = Some("Choose a category for this merchant first.".into());
            cx.notify();
            return;
        };
        // 3.84: the paint — the group leaves the queue in the click's
        // frame, the way its rows leave the list on the write's reload.
        // A failed write puts it back at its index (unless a reload has
        // meanwhile brought it back); the row's picker kept its pick
        // through the removal, so a rolled-back row reads as it did.
        let at = self.groups.iter().position(|g| g.merchant == merchant);
        let undo = at.map(|index| (index, self.groups.remove(index)));
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
                    Err(e) => {
                        if let Some((index, group)) = undo
                            && !this.groups.iter().any(|g| g.merchant == group.merchant)
                        {
                            let at = index.min(this.groups.len());
                            this.groups.insert(at, group);
                        }
                        this.error = Some(e);
                    }
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    // -- Test-only views of the picker maps (3.91: they live behind
    // RefCells now; the long-queue tests read them through here).
    #[cfg(test)]
    pub(super) fn has_picker(&self, merchant: &str) -> bool {
        self.pickers.borrow().contains_key(merchant)
    }

    #[cfg(test)]
    pub(super) fn picker(&self, merchant: &str) -> Option<Entity<SelectState<SubChoices>>> {
        self.pickers.borrow().get(merchant).cloned()
    }

    #[cfg(test)]
    pub(super) fn picker_count(&self) -> usize {
        self.pickers.borrow().len()
    }

    #[cfg(test)]
    pub(super) fn picker_sub_count(&self) -> usize {
        self.picker_subs.borrow().len()
    }
}

impl Render for QueuePage {
    /// The To-categorize page (3.16, 3.19, 3.22, 3.34, 3.91): page header
    /// with the counts summary, one bordered panel holding the shared
    /// `DataList` — its heading row over the virtualized merchant rows,
    /// each row with its category picker and Categorize button.
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
                let pickers: Vec<_> = self.pickers.borrow().values().cloned().collect();
                for picker in pickers {
                    picker.update(cx, |s, cx| {
                        s.set_items(SearchableVec::new((*items).clone()), window, cx)
                    });
                }
            }
        }
        // 3.59: menu width for the row pickers — the longest grouped
        // label up to 320 px, computed once for the frame.
        let picker_menu_w = sub_choices_menu_width(&self.catalog.read(cx).cat_choices());
        let (muted, danger) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.danger)
        };
        // 3.34: the header summary counts what the list below shows —
        // transactions and merchants, both through `flow::count`. Its
        // debug selector records the exact text so a test can pin it;
        // 3.91: `page_header` builds the row, so the selector rides on it.
        let summary = format!(
            "{} transactions · {} merchants",
            flow::count(self.groups.iter().map(|g| g.count).sum()),
            flow::count(self.groups.len() as i64)
        );
        let header = page_header(cx, "To categorize", summary.clone(), Vec::new())
            .debug_selector(move || format!("queue:summary:{summary}"));
        // 3.34: one bordered panel (D7: bordered, never filled) fills the
        // rest of the page and holds the list; the empty state is centered
        // inside it.
        let panel = panel(cx, false).debug_selector(|| "queue:panel".to_string());
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
            // 3.91: the shared `DataList` (ui-conventions *Lists*, *To
            // categorize*): its 32 px heading row labels every column —
            // the category picker included, its placeholder alone is not a
            // label — and, unlike the hand-built row this replaces, the
            // headings share the rows' column widths by construction, so
            // every heading sits over its column and the invisible
            // spacer-button hack is gone. Merchant is the one flex column,
            // so the whole row fits the minimum window (3.22).
            let items: Vec<DataItem> = self
                .groups
                .iter()
                .enumerate()
                .map(|(id, g)| DataItem::Row {
                    id,
                    selected: false,
                    selector: format!("queue:row:{}", g.merchant),
                })
                .collect();
            let columns = vec![
                DataCol::new("Merchant", Col::Flex, ColAlign::Left),
                DataCol::new("Transactions", Col::Fixed(QUEUE_TXNS_W), ColAlign::Right),
                DataCol::new("Total", Col::Fixed(QUEUE_TOTAL_W), ColAlign::Right),
                DataCol::new("Dates", Col::Fixed(QUEUE_DATES_W), ColAlign::Left),
                DataCol::new("Category", Col::Fixed(QUEUE_PICKER_W), ColAlign::Left),
                DataCol::new("Categorize", Col::Fixed(QUEUE_CATEGORIZE_W), ColAlign::Left),
            ];
            panel
                .child(
                    DataList::new(cx.entity(), columns, items)
                        // H4: the prune window is the exact pass range,
                        // so an exact-page scroll moves it instead of
                        // growing it (see [`Self::note_paint_pass`]).
                        .on_pass(move |this, range| this.note_paint_pass(range))
                        // 3.19: the cells callback runs during layout for
                        // the visible slice only, so a queue of thousands
                        // builds (and picks) only what can be on screen;
                        // 3.83: it is also where the prune runs, so
                        // pickers retire once their rows stop being built.
                        .cells(move |this, item, window, cx| {
                            let DataItem::Row { id, .. } = item else {
                                return Vec::new();
                            };
                            let g = &this.groups[*id];
                            let merchant = g.merchant.clone();
                            let picker = this.picker_for_row(&merchant, window, cx);
                            this.prune_visible(cx);
                            // 3.34: `list_hover`, dividers and the 40 px
                            // `text_sm` row come from the list; the cells
                            // are the page's own elements so every test
                            // selector survives the move.
                            //
                            // 3.22: Merchant is the flex column; a name
                            // too long for the leftover space truncates,
                            // and the tooltip still shows the full one.
                            // 3.34: normal weight `foreground` — the row's
                            // action is what stands out, not its label.
                            let merchant_cell = div()
                                .id(SharedString::from(format!("queue-merchant-{merchant}")))
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .debug_selector({
                                    let m = merchant.clone();
                                    move || format!("queue:merchant-col:{m}")
                                })
                                .tooltip({
                                    let full = merchant.clone();
                                    move |window, cx| {
                                        let tip = full.clone();
                                        Tooltip::element(move |_, _| {
                                            let sel =
                                                format!("queue:merchant-tooltip:{}", tip.clone());
                                            div()
                                                .debug_selector(move || sel.clone())
                                                .child(tip.clone())
                                        })
                                        .build(window, cx)
                                    }
                                })
                                .child(merchant.clone());
                            // The Transactions cell is just the number
                            // (3.22); its debug selector records the exact
                            // text so a test can pin that.
                            let n = g.count;
                            let txns_cell = div()
                                .w_full()
                                .debug_selector({
                                    let m = merchant.clone();
                                    move || format!("queue:txns-col:{m}")
                                })
                                .child(
                                    div()
                                        .debug_selector({
                                            let m = merchant.clone();
                                            move || {
                                                format!("queue:txns-text:{m}:{}", flow::count(n))
                                            }
                                        })
                                        .child(flow::count(n)),
                                );
                            let total_cell = div()
                                .w_full()
                                .debug_selector({
                                    let m = merchant.clone();
                                    move || format!("queue:total-col:{m}")
                                })
                                .child(flow::money(g.total_cents));
                            // 3.34: the dates are `text_sm` muted (the
                            // list already sets `text_sm`).
                            let dates_cell = div()
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_color(muted)
                                .debug_selector({
                                    let m = merchant.clone();
                                    move || format!("queue:dates-col:{m}")
                                })
                                .child(flow::date_range(&g.first_date, &g.last_date));
                            let picker_cell = div()
                                .w_full()
                                .debug_selector({
                                    let m = merchant.clone();
                                    move || format!("queue:picker-col:{m}")
                                })
                                .child(
                                    Select::new(&picker)
                                        .menu_width(picker_menu_w)
                                        .placeholder("Category…"),
                                );
                            // 3.34: quiet and disabled until the row's
                            // picker holds a category; `small` and outline
                            // once one is picked — row-level buttons are
                            // never primary (D12), so the page keeps its
                            // one primary and only rows ready to apply are
                            // clickable. The debug selector records the
                            // state for tests.
                            let picked = picker.read(cx).selected_value().is_some();
                            let categorize_cell = div()
                                .w_full()
                                .debug_selector({
                                    let m = merchant.clone();
                                    move || format!("queue:categorize-col:{m}")
                                })
                                .child({
                                    let m = merchant.clone();
                                    Button::new(SharedString::from(format!(
                                        "categorize-{merchant}"
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
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.categorize_merchant(merchant.clone(), window, cx);
                                    }))
                                });
                            vec![
                                ListCell::from(merchant_cell.into_any_element()),
                                ListCell::from(txns_cell.into_any_element()),
                                ListCell::from(total_cell.into_any_element()),
                                ListCell::from(dates_cell.into_any_element()),
                                ListCell::from(picker_cell.into_any_element()),
                                ListCell::from(categorize_cell.into_any_element()),
                            ]
                        })
                        .heading_selectors("queue")
                        // 3.21: page-owned handle (see the tree).
                        .scroll(self.scroll.clone())
                        .selector("queue:list"),
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
            .child(header)
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
