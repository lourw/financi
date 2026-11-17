//! The page entity: its state, construction and the `PageEntity`
//! lifecycle (3.56, 3.78, 3.82).

use super::*;
use crate::ui::page::{PageEntity, PageEvent, watch_catalog};

/// The Dashboard page (3.82): its own entity — the filter, the chart and
/// summary panels' data, the date-range pickers and the transaction table
/// all live here, and the page drives its one load job (3.56) from the
/// `Catalog` events it watches (see [`crate::ui::page`]). Like the other
/// extracted pages it renders itself, refreshes itself, and hears nothing
/// from `Shell` except through visibility calls; it keeps the two mirrors
/// the row menus need and the test idle counters through the same weak
/// handle back the Categories page uses (see `CategoriesPage`'s doc).
pub(crate) struct DashboardPage {
    pub(super) db: DbWorker,
    /// The shared data model (3.78); `apply` reads the account list off
    /// its snapshot, never a query of its own.
    pub(in crate::ui) catalog: Entity<Catalog>,
    /// 5.3 review fix: shared with the shell — while an export or restore
    /// is in flight every write here is refused.
    pub(super) writes_blocked: Rc<Cell<bool>>,
    pub(in crate::ui) focus: FocusHandle,
    visible: bool,
    dirty: bool,
    pub(super) focus_pending: bool,
    /// The `Catalog` subscription (3.82); lives as long as the page.
    _catalog: Subscription,
    /// Back-pointer for the few things that are not this page's: the
    /// restore link (the Imports page owns the flow), *Unpair* (the
    /// Transfers page's), the rule-row mirrors the table delegate is
    /// filled from, and the test idle counters the load job shares with
    /// the shell's reloads.
    pub(super) shell: WeakEntity<Shell>,
    /// The Rules page entity: *Show rule* forwards there, and Esc
    /// cancels an open rule editor before clearing a selection (3.80).
    pub(super) rules_page: Entity<RulesPage>,
    /// 3.56: the dispatch generation of the newest load this page
    /// started. A result whose dispatch is older than this was overtaken
    /// — the newer load's result replaces it, so the stale one is
    /// dropped whole.
    pub(super) load_gen: u64,
    // -- Page state (3.74).
    /// 3.56: the whole filter state, one comparable value (see
    /// [`DashboardFilter`]).
    pub(in crate::ui) filter: DashboardFilter,
    /// The write-behind task for `ui.include_transfers`.
    pub(in crate::ui) include_transfers_task: Option<Task<()>>,
    /// 3.65: true until the first dashboard load has loaded the saved
    /// settings (`ui.date_range`, `ui.include_transfers`) inside its own
    /// query; the first load consumes it, so later ones skip the reads.
    pub(in crate::ui) startup_settings: bool,
    /// A persisted control (the date range or the transfers toggle) moved
    /// since launch. The 3.65 "their change wins" rule needs this in 3.82:
    /// a scripted change (the tests set a preset right after opening) can
    /// land *before* the launch load is even dispatched, so the generation
    /// guard alone would let the launch settings overwrite it.
    pub(super) filter_touched: bool,
    /// The search box.
    pub(in crate::ui) search_input: Entity<InputState>,
    /// 3.56: the pending debounced search. Each keystroke replaces it, so
    /// a burst of typing runs one query after the last key; Enter and the
    /// other controls apply at once (dropping this).
    pub(in crate::ui) search_task: Option<Task<()>>,
    /// The rows behind the chart and legend (top level or the drilled-in
    /// parent's children).
    pub(in crate::ui) chart_rows: Vec<CategorySpend>,
    /// The summary panel and the parents whose child rows are expanded.
    pub(in crate::ui) summary: MonthlySummary,
    /// 3.85: the cached `summary_rows()` the grid renders from — built
    /// once per summary load, parent toggle or category change, not per
    /// frame.
    pub(in crate::ui) summary_views: Rc<Vec<SummaryRowView>>,
    pub(in crate::ui) summary_views_dirty: bool,
    /// 3.85 test hook: how many times the cached views were (re)built.
    #[cfg(test)]
    pub(in crate::ui) summary_views_built: std::cell::Cell<usize>,
    pub(in crate::ui) expanded_parents: HashSet<i64>,
    /// One horizontal scroll handle for the whole month strip of the
    /// dashboard summary (3.30): the header's month row and every row's
    /// month cells track it, so they scroll together, while the name
    /// column and the `Total` column stay pinned.
    pub(in crate::ui) summary_scroll: ScrollHandle,
    /// 3.45: the page-level vertical scroller's handle. The scroll
    /// position survives refreshes and filter changes (the handle lives
    /// as long as the page); `set_visible` resets it to the top on page
    /// entry.
    pub(in crate::ui) page_scroll: ScrollHandle,
    /// 3.45: the scroller's measured viewport height — the page minus the
    /// pinned filter row — from the `bounds_height_probe` on the scroller's
    /// wrapper. The transactions table's height, so it fills the view and
    /// scrolls inside itself (mirrors `tx_panel_w`).
    pub(in crate::ui) tx_panel_h: f32,
    /// The [`ViewFilter`] the newest applied load ran with — the record
    /// of what produced the rows on screen (3.56). The pickers and the
    /// checkbox handlers keep it mirrored so it can be read directly.
    pub(in crate::ui) view_filter: ViewFilter,
    /// The date-range picker entities.
    pub(in crate::ui) date_preset_select: Entity<SelectState<Names>>,
    pub(in crate::ui) date_from_picker: Entity<DatePickerState>,
    pub(in crate::ui) date_to_picker: Entity<DatePickerState>,
    pub(in crate::ui) date_range_task: Option<Task<()>>,
    pub(in crate::ui) table: Entity<TableState<TxTable>>,
    pub(in crate::ui) row_count: usize,
    /// The transaction a manual category edit just made, `(transaction_id,
    /// category_id)`, whose row now offers to extend that choice to every
    /// matching row (2.6's rule offer, surfaced from the 2.9 menu).
    pub(in crate::ui) rule_offer: Option<(i64, i64)>,
    /// 3.64: the last manual category choice issued, `(transaction_id,
    /// category_id)`. The detached `change_row_category` offers its rule
    /// only while its own choice is the newest one on that row — the
    /// table's rows are a refresh behind, so they cannot answer that.
    pub(super) last_category_choice: Option<(i64, i64)>,
    /// The account list from the `Catalog` snapshot (3.82): the filter
    /// checkboxes and their labels, and what a load is resolved against.
    pub(in crate::ui) accounts: Vec<Account>,
    /// The category tree from the `Catalog` snapshot (3.82): the chart
    /// colors and the drill-down's parent lookup, like the shell mirror
    /// before the extraction.
    pub(in crate::ui) categories: Vec<CategoryNode>,
    /// The sidebar's visible-row total from the snapshot (3.82): the
    /// "empty database" signal for the empty state, like the shell mirror
    /// before the extraction.
    pub(in crate::ui) total_visible: i64,
    /// 3.75 idle markers shared with the shell: the page takes part in the
    /// same counting so `wait_idle` covers its loads too, and so the
    /// restore test can prove work happened between two points. Cells
    /// because `apply` runs while the shell is being borrowed, and a
    /// re-entrant `shell.update` there would panic.
    pub(super) refresh_started: Rc<Cell<u64>>,
    pub(super) refresh_applied: Rc<Cell<u64>>,
    /// 3.86: the chart's plot-area width in px, measured by the
    /// `chart:plot` bounds probe (0. until the first frame paints it). The
    /// label plan reads this, never window-width arithmetic.
    pub(in crate::ui) plot_w: f32,
    /// 3.86: one 10 px chart label's advance width, measured once against
    /// the real text system at construction — the old `CHART_CHAR_PX`
    /// guess lives on only as the seed of that measurement's docs. The
    /// label-plan math converts px budgets to character counts with this.
    pub(in crate::ui) chart_char_px: f32,
    /// 3.86: the transactions panel's (`panel:table`) measured width in px
    /// (0. until the first frame paints it). The flex column plan is
    /// derived from it through [`Self::set_table_width`].
    pub(in crate::ui) tx_panel_w: f32,
    /// 3.82: `set_date_range` marks the pickers; `render` re-syncs them
    /// (a load landing has no window, see [`Self::sync_date_widgets`]).
    pub(super) date_widgets_stale: bool,
    /// The pickers and inputs above subscribe here.
    _subs: Vec<Subscription>,
}

impl DashboardPage {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui) fn new(
        db: DbWorker,
        writes_blocked: Rc<Cell<bool>>,
        catalog: &Entity<Catalog>,
        rules_page: Entity<RulesPage>,
        shell: WeakEntity<Shell>,
        refresh_started: Rc<Cell<u64>>,
        refresh_applied: Rc<Cell<u64>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 3.76: the table's callbacks capture the page *weakly* — the
        // table is a child entity, and a strong handle here would form a
        // reference cycle that keeps every page alive for the whole app.
        let on_set_category = {
            let shell = cx.weak_entity();
            Rc::new(
                move |txn: i64, category: i64, window: &mut Window, cx: &mut App| {
                    let _ = shell.update(cx, |this, cx| {
                        this.change_row_category(txn, category, window, cx);
                    });
                },
            ) as Rc<dyn Fn(i64, i64, &mut Window, &mut App)>
        };
        let on_set_hidden = {
            let shell = cx.weak_entity();
            Rc::new(
                move |txn: i64, hidden: bool, window: &mut Window, cx: &mut App| {
                    let _ = shell.update(cx, |this, cx| {
                        this.set_row_hidden(txn, hidden, window, cx);
                    });
                },
            ) as Rc<dyn Fn(i64, bool, &mut Window, &mut App)>
        };
        let on_apply_rule = {
            let shell = cx.weak_entity();
            Rc::new(move |txn: i64, window: &mut Window, cx: &mut App| {
                let _ = shell.update(cx, |this, cx| this.apply_row_rule(txn, window, cx));
            }) as Rc<dyn Fn(i64, &mut Window, &mut App)>
        };
        let on_sort = {
            let shell = cx.weak_entity();
            Rc::new(move |sort: TxSort, window: &mut Window, cx: &mut App| {
                let _ = shell.update(cx, |this, cx| this.set_tx_sort(sort, window, cx));
            }) as Rc<dyn Fn(TxSort, &mut Window, &mut App)>
        };
        let on_unpair = {
            let shell = cx.weak_entity();
            Rc::new(move |txn: i64, window: &mut Window, cx: &mut App| {
                let _ = shell.update(cx, |this, cx| this.unpair_row(txn, window, cx));
            }) as UnpairFn
        };
        let on_show_rule = {
            let shell = cx.weak_entity();
            Rc::new(move |rule: i64, window: &mut Window, cx: &mut App| {
                let _ = shell.update(cx, |this, cx| this.show_rule(rule, window, cx));
            }) as ShowRuleFn
        };
        // 3.86: one 10 px chart label's advance width, measured against
        // the real text system instead of the old `CHART_CHAR_PX` guess.
        // Ten `M`s — the widest common capital — so the per-character
        // average is a safe upper bound for band labels.
        let chart_char_px =
            gpui_kit::base::plot::label::measure_text_width(&"MMMMMMMMMM".into(), px(10.), window)
                / 10.;
        let table = cx.new(|cx| {
            TableState::new(
                TxTable {
                    rows: Vec::new(),
                    cells: Vec::new(),
                    #[cfg(test)]
                    cells_built: std::cell::Cell::new(0),
                    child_choices: Vec::new(),
                    rule_offer: None,
                    rules: Rc::new(CompiledRules::new(&[])),
                    card_payment_category: None,
                    sort: TxSort::default(),
                    on_set_category,
                    on_set_hidden,
                    on_apply_rule,
                    on_unpair,
                    on_show_rule,
                    on_sort,
                    // 3.86: start collapsed; the `panel:table` bounds
                    // probe feeds the real width through `set_table_width`
                    // on the first frame.
                    flex_widths: tx_flex_widths(0.),
                },
                window,
                cx,
            )
        });
        // -- Dashboard date range (3.2). The picker starts on the default
        // (Last 3 months); the saved setting is restored from the database
        // below and shown once it arrives.
        let date_preset_select = cx.new(|cx| {
            SelectState::new(
                DatePreset::ALL.iter().map(|p| p.label().into()).collect(),
                Some(gpui_kit::component::IndexPath::new(
                    DatePreset::ALL
                        .iter()
                        .position(|p| *p == DateRange::default().preset())
                        .unwrap_or(0),
                )),
                window,
                cx,
            )
        });
        let date_from_picker =
            cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d"));
        let date_to_picker = cx.new(|cx| DatePickerState::new(window, cx).date_format("%Y-%m-%d"));
        let date_preset_sub = cx.subscribe_in(
            &date_preset_select,
            window,
            |this, _, ev: &SelectEvent<Names>, window, cx| {
                let SelectEvent::Confirm(choice) = ev;
                if let Some(preset) = choice
                    .as_ref()
                    .and_then(|label| DatePreset::ALL.iter().find(|p| label.as_str() == p.label()))
                    .copied()
                {
                    this.change_date_preset(preset, window, cx);
                }
            },
        );
        let date_from_sub = cx.subscribe_in(
            &date_from_picker,
            window,
            |this, _, ev: &DatePickerEvent, window, cx| {
                let DatePickerEvent::Change(value) = ev;
                if let Some(day) = value.start().map(|dt| dt.date()) {
                    this.date_picked(false, day, window, cx);
                }
            },
        );
        let date_to_sub = cx.subscribe_in(
            &date_to_picker,
            window,
            |this, _, ev: &DatePickerEvent, window, cx| {
                let DatePickerEvent::Change(value) = ev;
                if let Some(day) = value.start().map(|dt| dt.date()) {
                    this.date_picked(true, day, window, cx);
                }
            },
        );
        // The dashboard search box (3.56): typing debounces 200 ms — each
        // keystroke replaces the pending task, so a burst of keys runs
        // one query after the last one. Enter applies at once.
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search transactions"));
        let search_sub = cx.subscribe_in(
            &search_input,
            window,
            |this, input, ev: &InputEvent, window, cx| {
                match ev {
                    InputEvent::Change => {
                        let value = input.read(cx).value().trim().to_string();
                        this.search_task = Some(cx.spawn_in(window, async move |this, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(200))
                                .await;
                            let _ = this.update_in(cx, move |this, window, cx| {
                                this.search_task = None;
                                this.apply_search(&value, window, cx);
                            });
                        }));
                    }
                    InputEvent::PressEnter { .. } => {
                        // Enter means "now": cancel the pending debounce
                        // and apply the current text.
                        this.search_task = None;
                        let value = input.read(cx).value().trim().to_string();
                        this.apply_search(&value, window, cx);
                    }
                    _ => {}
                }
            },
        );
        Self {
            db,
            catalog: catalog.clone(),
            writes_blocked,
            focus: cx.focus_handle(),
            visible: false,
            dirty: true,
            focus_pending: false,
            _catalog: watch_catalog(catalog, cx),
            shell,
            rules_page,
            load_gen: 0,
            filter: DashboardFilter::default(),
            include_transfers_task: None,
            startup_settings: true,
            filter_touched: false,
            search_input,
            search_task: None,
            chart_rows: Vec::new(),
            summary: MonthlySummary {
                months: Vec::new(),
                rows: Vec::new(),
            },
            summary_views: Rc::new(Vec::new()),
            summary_views_dirty: true,
            #[cfg(test)]
            summary_views_built: std::cell::Cell::new(0),
            expanded_parents: HashSet::new(),
            summary_scroll: ScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            tx_panel_h: 0.,
            view_filter: ViewFilter::default(),
            date_preset_select,
            date_from_picker,
            date_to_picker,
            date_range_task: None,
            table,
            row_count: 0,
            rule_offer: None,
            last_category_choice: None,
            accounts: Vec::new(),
            categories: Vec::new(),
            total_visible: 0,
            refresh_started,
            refresh_applied,
            // 3.86: the measured-width mirrors start at zero; the bounds
            // probes fill them on the first painted frame (the flex plan
            // is seeded collapsed to match).
            plot_w: 0.,
            chart_char_px,
            tx_panel_w: 0.,
            date_widgets_stale: false,
            _subs: vec![date_preset_sub, date_from_sub, date_to_sub, search_sub],
        }
    }
}

impl EventEmitter<PageEvent> for DashboardPage {}

impl PageEntity for DashboardPage {
    /// The rows, chart and summary answer to everything except the
    /// transfer reviews (3.56's `dash_touched`, now the watcher's filter):
    /// accounts, categories, rules, transactions, imports — and the
    /// page's own settings domain, which the launch load consumes.
    const DEPENDS: Domains = Domains::CATEGORIES_RULES_TX
        .union(Domains::ACCOUNTS)
        .union(Domains::IMPORTS)
        .union(Domains::SETTINGS);

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

    /// 3.11: no transient UI travels across a page switch — the rule offer
    /// belongs to the dashboard it was raised on. The filter selections are
    /// *not* cleared: the date range and toggles are persisted settings and
    /// the legend/month selection survives a visit like they do (3.5/3.6).
    ///
    /// Unlike the trait default, hiding does **not** raise the dirty flag:
    /// this page's `apply` runs a query, not just a snapshot copy, and 3.78's
    /// page-switch invariant holds — a revisit with nothing changed issues
    /// no query. Staleness is exactly a `DataChanged` arriving while hidden,
    /// which `watch_catalog` already records.
    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.rule_offer = None;
        self.set_shown(visible);
        if visible {
            // 3.45: a page entry starts at the top of the scrolling
            // content; within a visit the position survives refreshes and
            // filter changes (the handle is page-owned).
            self.page_scroll.set_offset(gpui::Point::default());
            self.apply_if_dirty(cx);
            self.set_focus_pending(true);
        }
    }

    fn set_focus_pending(&mut self, pending: bool) {
        self.focus_pending = pending;
    }

    /// Pull the account list out of the snapshot — the checkboxes resolve
    /// against it (task 3.17) — and start the page's one load (3.56).
    /// The snapshot is merged before `DataChanged` is emitted, so the
    /// list handed to the job is the one the write produced: the same
    /// round trip still settles the checkboxes (3.17), and the job never
    /// runs `list_accounts` of its own. The launch call carries the
    /// `startup_settings` flag, so the saved date range and transfers
    /// choice are queried in the same trip (3.65).
    fn apply(&mut self, cx: &mut Context<Self>) {
        // Applying consumes the drift `watch_catalog` recorded, whether it
        // arrived through a show or straight through the visible branch.
        self.set_dirty(false);
        let snap = {
            let cat = self.catalog.read(cx);
            (
                cat.snap.accounts.clone(),
                cat.snap.tree.clone(),
                cat.snap.total,
            )
        };
        self.accounts = snap.0;
        self.categories = snap.1;
        // 3.85: row kinds (expandable parents, child grouping) derive
        // from the tree, so the summary view cache follows it.
        self.summary_views_dirty = true;
        self.total_visible = snap.2;
        self.sync_account_filter();
        let startup = std::mem::take(&mut self.startup_settings);
        // An empty account list means the snapshot has not landed yet
        // (the launch `apply` runs off the first `DataChanged`, which may
        // carry only some sections): the job then reads the accounts
        // itself, like the stale-filter re-query below.
        let accounts = (!self.accounts.is_empty()).then(|| self.accounts.clone());
        self.start_load(accounts, startup, cx);
    }
}

impl DashboardPage {
    /// Today, for the preset ranges; `preset_range` itself never reads a
    /// clock.
    pub(in crate::ui) fn today() -> NaiveDate {
        chrono::Local::now().date_naive()
    }
}
