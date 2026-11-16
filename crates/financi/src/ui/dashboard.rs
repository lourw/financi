//! The Dashboard page: filters, chart, summary, the transactions table
//! and its row menu (3.62).

use super::dashboard_filters::{
    FILTER_CONTROL_H, FILTER_DATE_MIN_W, FILTER_DATE_RANGE_W, FILTER_DATE_W, FILTER_SEARCH_MIN_W,
};
use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;
use financi_db::QueryLog;

/// Callbacks from a transaction row's context menu back to the [`Shell`]
/// (2.9): set a category, hide/unhide, and apply the post-edit rule offer.
type SetCategoryFn = Rc<dyn Fn(i64, i64, &mut Window, &mut App)>;
type SetHiddenFn = Rc<dyn Fn(i64, bool, &mut Window, &mut App)>;
type ApplyRuleFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;
pub(super) type UnpairFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;
/// Called with the deciding rule's id from the *Show rule* menu item (3.29).
pub(super) type ShowRuleFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// One entry of a transaction row's right-click menu (2.9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowMenuItem {
    ChangeCategory,
    Hide,
    Unhide,
    Unpair,
}

/// The context-menu entries for a row, as a pure function of its state so the
/// gating rule is testable without a window: *Change category…* always, then
/// *Unhide* for a hidden row and *Hide* otherwise (a row can only move in one
/// direction), then *Unpair* only for a row the pairing engine linked to a
/// partner (4.2). Hidden rows only reach the menu once the "show hidden"
/// toggle (3.4) puts them back in the table.
pub fn row_menu_items(row: &TransactionRow) -> Vec<RowMenuItem> {
    let mut items = vec![
        RowMenuItem::ChangeCategory,
        if row.hidden {
            RowMenuItem::Unhide
        } else {
            RowMenuItem::Hide
        },
    ];
    if row.transfer_pair_id.is_some() {
        items.push(RowMenuItem::Unpair);
    }
    items
}

/// What the *Show rule* item (3.29) of a row's context menu can do: find
/// the rule that decided the row, or the reason the item is disabled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowShowRule {
    /// The rule that decides the row; *Show rule* opens the Rules page on it.
    Found(i64),
    /// The category was set by hand (the row is locked).
    SetByHand,
    /// Built-in card-payment detection decided the row (locked at import).
    CardPayment,
    /// No rule matches the description.
    NoRule,
}

impl RowShowRule {
    /// The reason a disabled *Show rule* item shows next to its label.
    pub fn disabled_reason(&self) -> Option<&'static str> {
        match self {
            RowShowRule::Found(_) => None,
            RowShowRule::SetByHand => Some("Set by hand"),
            RowShowRule::CardPayment => Some("Card payment (built in)"),
            RowShowRule::NoRule => Some("No rule matches"),
        }
    }

    /// The menu item's label: when disabled, the reason rides along
    /// (`Show rule — Set by hand`), since a greyed item has nowhere else
    /// to say why.
    pub fn menu_label(&self) -> String {
        match self.disabled_reason() {
            Some(reason) => format!("Show rule — {reason}"),
            None => "Show rule".into(),
        }
    }
}

/// The *Show rule* state (3.29) for a row, as a pure function so the gating
/// rule is testable without a window. Card payments are locked at import
/// (2.5), so the card-payment check comes before *Set by hand* (both are
/// locked rows); an unlocked row asks [`winning_rule`] — the same matcher
/// import runs — so no rule id is ever stored on the transaction.
pub fn row_show_rule(
    row: &TransactionRow,
    rules: &[Rule],
    card_payment_category: Option<i64>,
) -> RowShowRule {
    if row.category_locked {
        if row.category_id.is_some() && row.category_id == card_payment_category {
            RowShowRule::CardPayment
        } else {
            RowShowRule::SetByHand
        }
    } else if let Some(id) = winning_rule(rules, &row.description) {
        RowShowRule::Found(id)
    } else {
        RowShowRule::NoRule
    }
}

pub struct TxTable {
    pub(super) rows: Vec<TransactionRow>,
    /// Child categories as `(id, "Parent › Child")` for the *Change category…*
    /// submenu, mirroring the shell's `child_choices`.
    pub(super) child_choices: Vec<(i64, String)>,
    /// The row that just got a manual category and now offers to apply it to
    /// every matching row (2.6's rule offer, 2.9). Its transaction id.
    pub(super) rule_offer: Option<i64>,
    /// The shell's rules, kept in sync on every refresh: the *Show rule*
    /// item (3.29) runs [`winning_rule`] over them when the menu opens.
    pub(super) rules: Vec<Rule>,
    /// The id of the system `Transfers > Card Payment` category (3.29), the
    /// same lookup `financi-db` uses; `None` only before the first refresh.
    pub(super) card_payment_category: Option<i64>,
    /// The table's current ordering, kept in sync with the shell so the
    /// header's sort icon reflects it after every refresh (3.4).
    pub(super) sort: TxSort,
    pub(super) on_set_category: SetCategoryFn,
    pub(super) on_set_hidden: SetHiddenFn,
    pub(super) on_apply_rule: ApplyRuleFn,
    /// Called with a paired row's id from the *Unpair* menu item (4.2).
    pub(super) on_unpair: UnpairFn,
    /// Called with the deciding rule's id from the *Show rule* menu item (3.29).
    pub(super) on_show_rule: ShowRuleFn,
    /// Called with the new ordering when a header's sort icon is clicked;
    /// the shell re-queries and refreshes.
    pub(super) on_sort: SortFn,
    /// Widths of the three flexible columns (Account, Description,
    /// Category), kept in sync with the window (3.25, `tx_flex_widths`).
    pub(super) flex_widths: (f32, f32, f32),
}

/// Callback from a sortable table header back to the [`Shell`] (task 3.4).
type SortFn = Rc<dyn Fn(TxSort, &mut Window, &mut App)>;

impl TableDelegate for TxTable {
    fn columns_count(&self, _: &App) -> usize {
        5
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        // Every column sorts (3.4, 3.44). The active column carries its
        // direction so the header icon matches the ordering; the others show
        // faint chevrons.
        let sort = |column: TxSortColumn, base: Column| match self.sort {
            TxSort {
                column: active,
                descending: true,
            } if active == column => base.descending(),
            TxSort {
                column: active,
                descending: false,
            } if active == column => base.ascending(),
            _ => base.sortable(),
        };
        match col_ix {
            0 => sort(
                TxSortColumn::Date,
                Column::new("date", "Date").width(px(110.)),
            ),
            1 => sort(
                TxSortColumn::Account,
                Column::new("account", "Account").width(px(self.flex_widths.0)),
            ),
            2 => sort(
                TxSortColumn::Description,
                Column::new("description", "Description").width(px(self.flex_widths.1)),
            ),
            // Room for the post-edit "Apply to all matching …" affordance.
            3 => sort(
                TxSortColumn::Category,
                Column::new("category", "Category").width(px(self.flex_widths.2)),
            ),
            _ => sort(
                TxSortColumn::Amount,
                Column::new("amount", "Amount").width(px(120.)).text_right(),
            ),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        // Every column sorts (3.4, 3.44); the table has already cycled its
        // own icon state (off → Descending → Ascending → off), so map the
        // column and its cycled value to a `TxSort` and hand it to the
        // shell, which re-queries. `Default` (the icon's off step) falls
        // back to that column's natural first direction. `cx` coerces to
        // `&mut App` like the menu callbacks; the shell is a different
        // entity, so updating it here is not re-entrant.
        let column = match col_ix {
            0 => TxSortColumn::Date,
            1 => TxSortColumn::Account,
            2 => TxSortColumn::Description,
            3 => TxSortColumn::Category,
            4 => TxSortColumn::Amount,
            _ => return,
        };
        // Date and Amount start out newest/biggest first (3.4); a text
        // column's first click sorts A→Z (3.44). The cycle then alternates
        // directions: from A→Z the icon sits on Ascending, whose next step
        // is the off step, which maps to Z→A.
        let descending = match column {
            TxSortColumn::Date | TxSortColumn::Amount => !matches!(sort, ColumnSort::Ascending),
            _ => matches!(sort, ColumnSort::Default),
        };
        let next = TxSort { column, descending };
        self.sort = next;
        (self.on_sort)(next, window, cx);
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        // 3.76: a detached write can empty or shorten `rows` while a menu
        // for an old index opens; offer nothing instead of panicking.
        let Some(row) = self.rows.get(row_ix) else {
            return menu;
        };
        let (txn, current) = (row.id, row.category_id);
        let on_change = self.on_set_category.clone();
        let submenu = PopupMenu::build(window, cx, {
            let choices = self.child_choices.clone();
            move |menu, _, _| {
                choices.into_iter().fold(menu, |menu, (id, label)| {
                    let on_change = on_change.clone();
                    menu.item(
                        PopupMenuItem::new(label)
                            .checked(Some(id) == current)
                            .on_click(move |_, window, cx| on_change(txn, id, window, cx)),
                    )
                })
            }
        });
        row_menu_items(row)
            .into_iter()
            .fold(menu, |menu, item| match item {
                RowMenuItem::ChangeCategory => {
                    // *Show rule* (3.29) follows the category actions: it is
                    // about how the row got its category. Disabled with the
                    // reason when no rule decided the row.
                    let menu =
                        menu.item(PopupMenuItem::submenu("Change category…", submenu.clone()));
                    match row_show_rule(row, &self.rules, self.card_payment_category) {
                        RowShowRule::Found(rule_id) => {
                            let on_show = self.on_show_rule.clone();
                            menu.item(
                                PopupMenuItem::new("Show rule")
                                    .on_click(move |_, window, cx| on_show(rule_id, window, cx)),
                            )
                        }
                        disabled => {
                            menu.item(PopupMenuItem::new(disabled.menu_label()).disabled(true))
                        }
                    }
                }
                RowMenuItem::Hide => {
                    let on_hide = self.on_set_hidden.clone();
                    menu.item(
                        PopupMenuItem::new("Hide")
                            .on_click(move |_, window, cx| on_hide(txn, true, window, cx)),
                    )
                }
                RowMenuItem::Unhide => {
                    let on_show = self.on_set_hidden.clone();
                    menu.item(
                        PopupMenuItem::new("Unhide")
                            .on_click(move |_, window, cx| on_show(txn, false, window, cx)),
                    )
                }
                RowMenuItem::Unpair => {
                    let on_unpair = self.on_unpair.clone();
                    menu.item(
                        PopupMenuItem::new("Unpair")
                            .on_click(move |_, window, cx| on_unpair(txn, window, cx)),
                    )
                }
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        // 3.76: a repaint can arrive with a row index a refresh already
        // removed (the table keeps its scroll position); render nothing.
        if row_ix >= self.rows.len() {
            return div().into_any_element();
        }
        if col_ix == 3 {
            let row = &self.rows[row_ix];
            let merchant = financi_core::parse::merchant(&row.description);
            if self.rule_offer == Some(row.id) && !merchant.is_empty() {
                let txn = row.id;
                let on_apply = self.on_apply_rule.clone();
                return div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(self.text(row_ix, col_ix)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("apply-rule-{txn}")))
                            .link()
                            .compact()
                            .label(format!("Apply to all matching \"{merchant}\""))
                            .on_click(move |_, window, cx| on_apply(txn, window, cx)),
                    )
                    .into_any_element();
            }
        }
        // Hidden rows read as secondary (grey); a money-in amount is green
        // (ui-conventions *Colors* and decision D3). Grey wins for hidden.
        let row = &self.rows[row_ix];
        let theme = cx.theme();
        let color = if row.hidden {
            Some(theme.muted_foreground)
        } else if col_ix == 4 && row.amount_cents > 0 {
            Some(theme.success)
        } else {
            None
        };
        // A paired row carries the link glyph on its description cell; the
        // colour (hidden / income) carries over to the icon (4.2).
        if col_ix == 2 && row.transfer_pair_id.is_some() {
            let cell = h_flex()
                .gap_1()
                .items_center()
                .child(IconName::Link)
                .child(self.text(row_ix, col_ix));
            match color {
                Some(color) => cell.text_color(color).into_any_element(),
                None => cell.into_any_element(),
            }
        } else {
            let cell = div().child(self.text(row_ix, col_ix));
            match color {
                Some(color) => cell.text_color(color).into_any_element(),
                None => cell.into_any_element(),
            }
        }
    }
}

impl TxTable {
    pub(super) fn text(&self, row_ix: usize, col_ix: usize) -> String {
        let r = &self.rows[row_ix];
        match col_ix {
            0 => r.posted_date.clone(),
            1 => r.account.clone(),
            2 => r.description.trim().to_string(),
            3 => r.category.clone().unwrap_or_else(|| "Uncategorized".into()),
            _ => flow::money(r.amount_cents),
        }
    }
}

// ---------------------------------------------------------------------------
// Import history table (Imports page)

/// The one dashboard category selection (3.5): a legend row — a real
/// category (parent or child) or the `Uncategorized` pseudo-row — that
/// narrows the table. `Uncategorized` rides the table's existing
/// uncategorized-only clause; a category id matches that category and,
/// for a parent, its children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum LegendSelection {
    Category(i64),
    Uncategorized,
}

/// Which of the dashboard's three empty states applies (task 3.10,
/// ui-conventions *States*): the whole-window drop-a-CSV prompt is only for
/// an empty database; with data present, the chart/summary and the table
/// carry their own empty states instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DashboardBody {
    /// No transactions at all: the whole-window import prompt.
    NoData,
    /// Transactions exist but the range/filters select none: the table
    /// shows "No transactions match these filters" with a "Clear filters"
    /// link, and the chart/summary say "No spending in this range".
    NoMatches,
    /// Rows to show.
    Rows,
}

/// A `spend_by_category` row prepared for drawing: the resolved palette
/// color and the fields `BarChart` and the legend render. `name` is the
/// full category name (legend, tooltip); `short` is the fitted band-axis
/// label (3.15), produced by [`short_chart_labels`]; `bar_label` is the
/// share drawn above the bar (3.31), empty when the bar is thinned out
/// of the label plan by [`bar_label_plan`].
#[derive(Clone)]
pub(super) struct ChartRow {
    pub(super) id: Option<i64>,
    pub(super) name: SharedString,
    pub(super) short: SharedString,
    pub(super) bar_label: SharedString,
    pub(super) cents: i64,
    pub(super) share: f64,
    pub(super) color: Hsla,
}

/// The legend column inside the chart panel (3.31): just swatch and name,
/// so it no longer carries the amount and share columns and the plot gets
/// the width back.
pub(super) const LEGEND_W: f32 = 140.;
/// The dashboard table's fixed columns (3.25): the `Date` and `Amount`
/// widths, which never change with the window.
pub(super) const TX_FIXED_W: f32 = 110. + 120.;
/// The dashboard table's `Account`, `Description` and `Category` widths for
/// `page_w` pixels of page (3.25, the same shape as `history_flex_widths`):
/// what the fixed columns leave over, split in the widths' original
/// proportions, with the flexed total floored at three times
/// `HISTORY_FLEX_MIN_W`. The sum always reaches the page's right edge, so
/// the table never overhangs it.
pub(super) fn tx_flex_widths(page_w: f32) -> (f32, f32, f32) {
    let flex = (page_w - TX_FIXED_W - TABLE_BORDER_X).max(3. * HISTORY_FLEX_MIN_W);
    const TOTAL: f32 = 300. + 360. + 260.;
    (
        flex * 300. / TOTAL,
        flex * 360. / TOTAL,
        flex * 260. / TOTAL,
    )
}

// -- Bar share labels (3.31) --------------------------------------------
//
// Each bar carries its share of the spend above it (`18%`), drawn by
// `BarChart::label` in gpui-kit's 10px chart font, centred on the bar.
// `gpui-component` reserves a line of text past the tallest bar for
// them, but offers no thinning of its own, so the app skips labels the
// way band labels are thinned (3.15): a whole number of bands per drawn
// label, spread evenly first-to-last, so centred labels cannot collide.

/// What a row of the monthly summary grid (3.6) is, resolved against the
/// category tree: a spend parent (with its expand/collapse state), one of
/// its children, the `Uncategorized` pseudo-row, or one of the three
/// fixed total rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SummaryRowKind {
    Parent { expandable: bool, expanded: bool },
    Child,
    Uncategorized,
    Income,
    Spend,
    Net,
}

/// A summary row prepared for drawing: the grid values in month order with
/// the `Total` last.
#[derive(Clone, Debug)]
pub(super) struct SummaryRowView {
    pub(super) id: Option<i64>,
    pub(super) name: SharedString,
    pub(super) kind: SummaryRowKind,
    pub(super) cells: Vec<i64>,
}

/// 3.56: the dashboard's whole filter state in one comparable value —
/// every input its queries take, from the date range down to the sort.
/// A change to any filter control updates this and starts the dashboard's
/// one keyed load; the table, chart and summary are its three results.
/// `PartialEq` is what makes "did anything actually change?" one check.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct DashboardFilter {
    /// The saved date range shown by the preset picker.
    pub(super) date_range: DateRange,
    /// The accounts the user unchecked in the dropdown (empty = all).
    pub(super) excluded_accounts: Vec<i64>,
    /// The search term. Empty means "no narrowing".
    pub(super) search: String,
    /// The "Include transfers" flag persisted as `ui.include_transfers`.
    pub(super) include_transfers: bool,
    /// The "Uncategorized only" and "Show hidden" table toggles.
    pub(super) uncategorized_only: bool,
    pub(super) show_hidden: bool,
    /// The legend selection (3.5) and the summary-cell month (3.6) —
    /// the dashboard's category and month narrowing — and the chart and
    /// legend's drill-down level (`None` = the parent level).
    pub(super) legend_selection: Option<LegendSelection>,
    pub(super) summary_month: Option<String>,
    pub(super) chart_parent: Option<i64>,
    /// The table's column sort (3.4).
    pub(super) tx_sort: TxSort,
}

impl DashboardFilter {
    /// The shared [`ViewFilter`] for a run of the queries: the range's
    /// dates as of `today`, the accounts the checkboxes still include,
    /// and the transfers flag. An empty exclusion list means all
    /// accounts — an empty `account_ids` too.
    fn view_filter(&self, today: NaiveDate, account_ids: Vec<i64>) -> ViewFilter {
        let (from, to) = self.date_range.range(today);
        ViewFilter {
            from,
            to,
            account_ids: if self.excluded_accounts.is_empty() {
                Vec::new()
            } else {
                account_ids
            },
            include_transfers: self.include_transfers,
        }
    }

    /// The table's extra filters (3.4): search, the two toggles and the
    /// sort, plus the legend selection (3.5) — the `Uncategorized` row
    /// rides `uncategorized_only` — and the summary-cell month (3.6).
    fn table_filter(&self) -> TxTableFilter {
        TxTableFilter {
            category_id: match self.legend_selection {
                Some(LegendSelection::Category(id)) => Some(id),
                Some(LegendSelection::Uncategorized) | None => None,
            },
            month: self.summary_month.clone(),
            search: if self.search.is_empty() {
                None
            } else {
                Some(self.search.clone())
            },
            uncategorized_only: self.uncategorized_only
                || matches!(self.legend_selection, Some(LegendSelection::Uncategorized)),
            show_hidden: self.show_hidden,
            sort: self.tx_sort,
        }
    }
}

/// The result of the dashboard's one load (3.56): everything the page
/// renders from the database, and the [`ViewFilter`] the queries actually
/// ran with — the shell stamps it as the record of the last applied
/// query. `settings` carries the persisted pair the launch load reads
/// inside its own query (3.65), so the first frame shows the saved view.
pub(super) struct DashboardData {
    pub(super) rows: Vec<TransactionRow>,
    pub(super) chart: Vec<CategorySpend>,
    pub(super) summary: MonthlySummary,
    pub(super) view_filter: ViewFilter,
    pub(super) settings: Option<(DateRange, bool)>,
}

/// The dashboard's job body (3.56): resolve the checked accounts against
/// the account list, then run the filtered table query and the one
/// category/summary aggregation — or, drilled into a parent, the child
/// chart and the grid alone. `startup` reads the two persisted view
/// settings in the same trip and queries *with* them (3.65).
///
/// `accounts` is the shell's mirror when a filter control changed (the
/// list is on screen already); `None` makes the job read the list itself,
/// which is what the launch and the after-a-write reloads need
/// so account changes resolve in the same round trip (3.17).
pub(super) fn load_dashboard(
    c: &mut Connection,
    log: &QueryLog,
    mut filter: DashboardFilter,
    accounts: Option<Vec<Account>>,
    startup: bool,
) -> Result<DashboardData, DbError> {
    let settings = if startup {
        log.record("settings::date_range");
        let range = settings::date_range(c)?;
        log.record("settings::include_transfers");
        let transfers = settings::include_transfers(c)?;
        // The queries below run with the saved values, so the first
        // frame is already the saved view.
        filter.date_range = range;
        filter.include_transfers = transfers;
        Some((range, transfers))
    } else {
        None
    };
    let fetched;
    let accounts: &[Account] = match &accounts {
        Some(list) => list,
        None => {
            log.record("list_accounts");
            fetched = list_accounts(c)?;
            &fetched
        }
    };
    let ids: Vec<i64> = accounts
        .iter()
        .filter(|a| !filter.excluded_accounts.contains(&a.id))
        .map(|a| a.id)
        .collect();
    // "Nothing checked" with accounts present: empty results, no queries.
    if ids.is_empty() && !accounts.is_empty() {
        let view_filter = filter.view_filter(DashboardPage::today(), ids);
        return Ok(DashboardData {
            rows: Vec::new(),
            chart: Vec::new(),
            summary: MonthlySummary {
                months: Vec::new(),
                rows: Vec::new(),
            },
            view_filter,
            settings,
        });
    }
    let view_filter = filter.view_filter(DashboardPage::today(), ids);
    let table_filter = filter.table_filter();
    log.record("list_transactions_filtered");
    let rows = list_transactions_filtered(c, &view_filter, &table_filter)?;
    let (chart, summary) = match filter.chart_parent {
        None => {
            // Both aggregations in one pass (3.88).
            log.record("dashboard");
            financi_db::dashboard(c, &view_filter)?
        }
        Some(parent) => {
            log.record("spend_by_category");
            let chart = spend_by_category(c, &view_filter, Some(parent))?;
            log.record("monthly_summary");
            let summary = monthly_summary(c, &view_filter)?;
            (chart, summary)
        }
    };
    Ok(DashboardData {
        rows,
        chart,
        summary,
        view_filter,
        settings,
    })
}

/// The Dashboard page (3.82): its own entity — the filter, the chart and
/// summary panels' data, the date-range pickers and the transaction table
/// all live here, and the page drives its one load job (3.56) from the
/// `Catalog` events it watches (see [`crate::ui::page`]). Like the other
/// extracted pages it renders itself, refreshes itself, and hears nothing
/// from `Shell` except through visibility calls; it keeps the two mirrors
/// the row menus need and the test idle counters through the same weak
/// handle back the Categories page uses (see `CategoriesPage`'s doc).
pub(super) struct DashboardPage {
    db: DbWorker,
    /// The shared data model (3.78); `apply` reads the account list off
    /// its snapshot, never a query of its own.
    catalog: Entity<Catalog>,
    /// 5.3 review fix: shared with the shell — while an export or restore
    /// is in flight every write here is refused.
    writes_blocked: Rc<Cell<bool>>,
    pub(super) focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// The `Catalog` subscription (3.82); lives as long as the page.
    _catalog: Subscription,
    /// Back-pointer for the few things that are not this page's: the
    /// restore link (the Imports page owns the flow), *Unpair* (the
    /// Transfers page's), the rule-row mirrors the table delegate is
    /// filled from, and the test idle counters the load job shares with
    /// the shell's reloads.
    shell: WeakEntity<Shell>,
    /// The Rules page entity: *Show rule* forwards there, and Esc
    /// cancels an open rule editor before clearing a selection (3.80).
    rules_page: Entity<RulesPage>,
    /// 3.56: the dispatch generation of the newest load this page
    /// started. A result whose dispatch is older than this was overtaken
    /// — the newer load's result replaces it, so the stale one is
    /// dropped whole.
    load_gen: u64,
    // -- Page state (3.74).
    /// 3.56: the whole filter state, one comparable value (see
    /// [`DashboardFilter`]).
    pub(super) filter: DashboardFilter,
    /// The write-behind task for `ui.include_transfers`.
    pub(super) include_transfers_task: Option<Task<()>>,
    /// 3.65: true until the first dashboard load has loaded the saved
    /// settings (`ui.date_range`, `ui.include_transfers`) inside its own
    /// query; the first load consumes it, so later ones skip the reads.
    pub(super) startup_settings: bool,
    /// A persisted control (the date range or the transfers toggle) moved
    /// since launch. The 3.65 "their change wins" rule needs this in 3.82:
    /// a scripted change (the tests set a preset right after opening) can
    /// land *before* the launch load is even dispatched, so the generation
    /// guard alone would let the launch settings overwrite it.
    filter_touched: bool,
    /// The search box.
    pub(super) search_input: Entity<InputState>,
    /// 3.56: the pending debounced search. Each keystroke replaces it, so
    /// a burst of typing runs one query after the last key; Enter and the
    /// other controls apply at once (dropping this).
    pub(super) search_task: Option<Task<()>>,
    /// The rows behind the chart and legend (top level or the drilled-in
    /// parent's children).
    pub(super) chart_rows: Vec<CategorySpend>,
    /// The summary panel and the parents whose child rows are expanded.
    pub(super) summary: MonthlySummary,
    pub(super) expanded_parents: HashSet<i64>,
    /// One horizontal scroll handle for the whole month strip of the
    /// dashboard summary (3.30): the header's month row and every row's
    /// month cells track it, so they scroll together, while the name
    /// column and the `Total` column stay pinned.
    pub(super) summary_scroll: ScrollHandle,
    /// The [`ViewFilter`] the newest applied load ran with — the record
    /// of what produced the rows on screen (3.56). The pickers and the
    /// checkbox handlers keep it mirrored so it can be read directly.
    pub(super) view_filter: ViewFilter,
    /// The date-range picker entities.
    pub(super) date_preset_select: Entity<SelectState<Names>>,
    pub(super) date_from_picker: Entity<DatePickerState>,
    pub(super) date_to_picker: Entity<DatePickerState>,
    pub(super) date_range_task: Option<Task<()>>,
    pub(super) table: Entity<TableState<TxTable>>,
    pub(super) row_count: usize,
    /// The transaction a manual category edit just made, `(transaction_id,
    /// category_id)`, whose row now offers to extend that choice to every
    /// matching row (2.6's rule offer, surfaced from the 2.9 menu).
    pub(super) rule_offer: Option<(i64, i64)>,
    /// 3.64: the last manual category choice issued, `(transaction_id,
    /// category_id)`. The detached `change_row_category` offers its rule
    /// only while its own choice is the newest one on that row — the
    /// table's rows are a refresh behind, so they cannot answer that.
    last_category_choice: Option<(i64, i64)>,
    /// The account list from the `Catalog` snapshot (3.82): the filter
    /// checkboxes and their labels, and what a load is resolved against.
    pub(super) accounts: Vec<Account>,
    /// The category tree from the `Catalog` snapshot (3.82): the chart
    /// colors and the drill-down's parent lookup, like the shell mirror
    /// before the extraction.
    pub(super) categories: Vec<CategoryNode>,
    /// The sidebar's visible-row total from the snapshot (3.82): the
    /// "empty database" signal for the empty state, like the shell mirror
    /// before the extraction.
    pub(super) total_visible: i64,
    /// 3.75 idle markers shared with the shell: the page takes part in the
    /// same counting so `wait_idle` covers its loads too, and so the
    /// restore test can prove work happened between two points. Cells
    /// because `apply` runs while the shell is being borrowed, and a
    /// re-entrant `shell.update` there would panic.
    refresh_started: Rc<Cell<u64>>,
    refresh_applied: Rc<Cell<u64>>,
    /// The window width `window_bounds_changed` last reported — the
    /// chart's label plan reads it (3.15).
    pub(super) window_width: f32,
    /// 3.82: `set_date_range` marks the pickers; `render` re-syncs them
    /// (a load landing has no window, see [`Self::sync_date_widgets`]).
    date_widgets_stale: bool,
    /// The pickers and inputs above subscribe here.
    _subs: Vec<Subscription>,
}

impl DashboardPage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
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
        let table = cx.new(|cx| {
            TableState::new(
                TxTable {
                    rows: Vec::new(),
                    child_choices: Vec::new(),
                    rule_offer: None,
                    rules: Vec::new(),
                    card_payment_category: None,
                    sort: TxSort::default(),
                    on_set_category,
                    on_set_hidden,
                    on_apply_rule,
                    on_unpair,
                    on_show_rule,
                    on_sort,
                    // 3.25: start at the widths the current window allows.
                    flex_widths: tx_flex_widths(
                        (window.bounds().size.width.as_f32() - SIDEBAR_W - 2. * PAGE_PAD).max(0.),
                    ),
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
            expanded_parents: HashSet::new(),
            summary_scroll: ScrollHandle::new(),
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
            // Seed the width mirror from the window the page is built on,
            // like the shell does; later resizes arrive through
            // `widths_changed` (3.82).
            window_width: window.bounds().size.width.as_f32(),
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
    pub(super) fn today() -> NaiveDate {
        chrono::Local::now().date_naive()
    }

    /// Take the next dispatch generation. It comes off the shell's shared
    /// reload counter (3.75): the tests' `wait_idle` compares that
    /// counter's two marks, and this page's loads must be counted in it
    /// or waiting would race them. The cell — not the shell entity — is
    /// what makes the bump safe to take from inside the page's own
    /// update even when the shell is borrowed around it (an apply
    /// arriving through `go_page`).
    fn next_generation(&self) -> u64 {
        let g = self.refresh_started.get() + 1;
        self.refresh_started.set(g);
        g
    }

    /// Stamp the idle marker (3.75): the maximum, since generations only
    /// move forward and a replaced load never stamps.
    fn stamp_applied(&self, generation: u64) {
        self.refresh_applied
            .set(self.refresh_applied.get().max(generation));
    }

    /// 3.56: start the page's one load — filtered rows, chart and summary
    /// in a single keyed job (3.77: a burst of filter changes leaves at
    /// most one in flight plus the newest, and every replaced job resolves
    /// `Superseded` without touching the screen). The filter is snapshotted
    /// now, so the load runs with exactly what the controls showed when it
    /// started. `accounts`: the snapshot list `apply` just pulled (3.82);
    /// `None` reads it in the job (the stale-filter re-query below).
    /// `startup` reads the persisted settings in the same trip (3.65).
    fn start_load(
        &mut self,
        accounts: Option<Vec<Account>>,
        startup: bool,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_generation();
        self.load_gen = generation;
        let filter = self.filter.clone();
        let db = self.db.clone();
        let log = db.query_log();
        // Detached (3.64): nothing may cancel the continuation between
        // the query finishing and the result being guarded in.
        cx.spawn(async move |this, cx| {
            let result = db
                .call_keyed("dashboard", move |c| {
                    load_dashboard(c, &log, filter, accounts, startup)
                })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(data) => this.finish_load(generation, data, cx),
                // A newer filter change took over while this ran; its
                // result replaces this one's.
                Err(DbError::Superseded) => {}
                Err(e) => {
                    this.notify_err(format!("Database error: {e}"), cx);
                    this.stamp_applied(generation);
                }
            });
        })
        .detach();
    }

    /// A dashboard load came back: apply it, unless a newer load was
    /// dispatched while it ran — then this result is stale whole, rows
    /// included, and the newer load's landing replaces it (3.56).
    fn finish_load(&mut self, generation: u64, mut data: DashboardData, cx: &mut Context<Self>) {
        if generation < self.load_gen {
            return;
        }
        // 3.65: the launch settings the job already queried with land on
        // the state and the filter controls before the data of the same
        // round trip is applied, so the first frame shows the saved range
        // everywhere. The guard above is what "the user changed it since,
        // and their change wins" means here: a newer dispatch replaced
        // this load, and its result carries the newer controls. In 3.82
        // the change can also arrive *before* the launch load was even
        // dispatched (a scripted preset change right after opening): the
        // job still ran with the saved values, so the result is dropped
        // and the user's filter is re-queried instead of applied.
        if let Some((range, include_transfers)) = data.settings {
            if self.filter_touched {
                self.reload(cx);
                self.stamp_applied(generation);
                return;
            }
            self.set_date_range(range, cx);
            self.filter.include_transfers = include_transfers;
            data.view_filter.include_transfers = include_transfers;
        }
        self.apply_dashboard_rows(data, cx);
        self.stamp_applied(generation);
        cx.notify();
    }

    /// Re-run the load with the current controls (3.56): what every
    /// filter-control change below ends in, in place of the old
    /// `Shell::changed(Domains::SETTINGS)` round trip — the shared
    /// sections never change on a filter change, only this page's rows.
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        // A control can move before the first snapshot lands (the tests
        // script a preset change right after opening the window): with no
        // account list yet, the job reads it, so a scripted change never
        // supersedes the launch load with an empty-account query.
        let accounts = (!self.accounts.is_empty()).then(|| self.accounts.clone());
        self.start_load(accounts, false, cx);
    }

    /// Feedback out: the shell paints toasts (3.27); the page only
    /// reports what happened.
    fn notify_err(&self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast {
            message: message.into(),
            error: true,
        });
    }

    fn notify_ok(&self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast {
            message: message.into(),
            error: false,
        });
    }

    /// Apply the dashboard's one load result (3.56), last: the table's
    /// delegate reads the child choices and card-payment category the
    /// shell's `sync_category_choices` set on the last `DataChanged` —
    /// which arrived before this job was even dispatched. A selection the
    /// new data no longer supports is corrected and the load restarted —
    /// and the half-matched rows are *not* applied (3.56: the screen
    /// never shows rows queried under a filter the controls no longer
    /// hold).
    fn apply_dashboard_rows(&mut self, data: DashboardData, cx: &mut Context<Self>) {
        // Revalidate the legend state against the rows we can actually
        // show: a date/accounts change can empty the chart out from under
        // a selection or a drill-down, and the table filter must not keep
        // narrowing by a category that is no longer there.
        let mut stale = false;
        let drilled = self.filter.chart_parent;
        if drilled.is_some() && data.chart.is_empty() {
            self.filter.chart_parent = None;
            stale = true;
        }
        let selection_gone = match self.filter.legend_selection {
            Some(LegendSelection::Category(id)) => {
                // While drilled, the selected parent itself is not among
                // the child rows — it stays.
                drilled != Some(id) && !data.chart.iter().any(|r| r.id == Some(id))
            }
            Some(LegendSelection::Uncategorized) => {
                self.filter.chart_parent.is_none() && !data.chart.iter().any(|r| r.id.is_none())
            }
            None => false,
        };
        if selection_gone {
            self.filter.legend_selection = None;
            stale = true;
        }
        // The month selection (3.6) goes the same way when the grid no
        // longer contains that month.
        if self.filter.summary_month.is_some()
            && !data
                .summary
                .months
                .iter()
                .any(|m| Some(m.as_str()) == self.filter.summary_month.as_deref())
        {
            self.filter.summary_month = None;
            stale = true;
        }
        if stale {
            // Re-query with the corrected filter so the table, chart and
            // state agree again. The half-matched rows are dropped: the
            // new load's result takes their place.
            self.reload(cx);
            return;
        }
        self.row_count = data.rows.len();
        self.chart_rows = data.chart;
        // A range change shows new months: park the month strip (3.30) at
        // its right end so the most recent month sits beside the pinned
        // `Total` column. The huge negative offset clamps to the true
        // maximum when the strip next paints.
        if self.summary.months != data.summary.months {
            self.summary_scroll
                .set_offset(point(px(-1_000_000.), px(0.)));
        }
        self.summary = data.summary;
        // Record the filter the applied rows were queried with (3.56).
        self.view_filter = data.view_filter;
        // The rule offer follows a row the user just edited; a load
        // that drops the row (hidden, filtered out, deleted) drops the
        // offer with it.
        if self
            .rule_offer
            .is_some_and(|(id, _)| !data.rows.iter().any(|r| r.id == id))
        {
            self.rule_offer = None;
        }
        let rows = data.rows;
        // 3.82: the row-menu mirrors live on the shell (the Rules and
        // Categories pages read them too); every write refreshed them
        // before this job was dispatched, so they name the same rows
        // these do.
        let (child_choices, rules, card_payment) = self
            .shell
            .update(cx, |shell, _| {
                (
                    shell.child_choices.clone(),
                    shell.rules.clone(),
                    shell.card_payment_category,
                )
            })
            .unwrap_or_default();
        let rule_offer = self.rule_offer.map(|(id, _)| id);
        let sort = self.filter.tx_sort;
        self.table.update(cx, |t, cx| {
            let delegate = t.delegate_mut();
            delegate.rows = rows;
            delegate.child_choices = child_choices;
            delegate.rule_offer = rule_offer;
            delegate.rules = rules;
            delegate.card_payment_category = card_payment;
            delegate.sort = sort;
            t.refresh(cx);
        });
    }
}

impl DashboardPage {
    /// Put a date range on the state and the controls without persisting
    /// and without refreshing (3.65: split from `apply_date_range` so the
    /// launch settings can land inside the refresh that loaded them).
    /// 3.82: the widgets are only marked — a load landing has no window
    /// to drive them with, and `render` catches up the same frame.
    pub(super) fn set_date_range(&mut self, range: DateRange, _cx: &mut Context<Self>) {
        self.filter.date_range = range;
        let (from, to) = range.range(Self::today());
        self.view_filter.from = from;
        self.view_filter.to = to;
        self.date_widgets_stale = true;
    }

    /// Mirror `filter.date_range` into the preset Select and the From/To
    /// pickers (3.82, run from `render`): `set_selected_index`,
    /// `set_value` and `set_date` don't emit their Change events, so this
    /// never re-persists or re-selects.
    fn sync_date_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.date_widgets_stale) {
            return;
        }
        let index = DatePreset::ALL
            .iter()
            .position(|p| *p == self.filter.date_range.preset())
            .unwrap_or(0);
        self.date_preset_select.update(cx, |select, cx| {
            select.set_selected_index(Some(gpui_kit::component::IndexPath::new(index)), window, cx);
        });
        if let DateRange::Custom { from, to } = self.filter.date_range {
            self.date_from_picker.update(cx, |picker, cx| {
                picker.set_date(CalendarDate::Single(Some(from)), window, cx);
            });
            self.date_to_picker.update(cx, |picker, cx| {
                picker.set_date(CalendarDate::Single(Some(to)), window, cx);
            });
        }
    }

    /// Apply a date range to the state and the controls without
    /// persisting; user changes persist on top via [`Self::persist_date_range`].
    /// The table reads the date range, so this re-queries.
    fn apply_date_range(&mut self, range: DateRange, cx: &mut Context<Self>) {
        self.filter_touched = true;
        self.set_date_range(range, cx);
        self.reload(cx);
        cx.notify();
    }

    /// Write the current selection to the `settings` table through the
    /// worker. A failed write only costs the restore at the next launch.
    fn persist_date_range(&mut self, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let db = self.db.clone();
        let range = self.filter.date_range;
        self.date_range_task = Some(cx.spawn(async move |_, _| {
            let _ = on_db(&db, move |c| settings::set_date_range(c, &range)).await;
        }));
    }

    /// The user picked a preset in the Select (task 3.2).
    pub(super) fn change_date_preset(
        &mut self,
        preset: DatePreset,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = if preset == DatePreset::Custom {
            // A custom range starts on the current month, so the pickers
            // open on a date the calendar can show (unlike "All time").
            let (from, to) = preset_range(DatePreset::ThisMonth, Self::today());
            DateRange::Custom { from, to }
        } else {
            DateRange::Preset(preset)
        };
        self.apply_date_range(range, cx);
        self.persist_date_range(cx);
    }

    /// The user changed one of the two custom [`DatePicker`]s.
    pub(super) fn date_picked(
        &mut self,
        is_end: bool,
        day: NaiveDate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (from, to) = match self.filter.date_range {
            DateRange::Custom { from, to } => {
                if is_end {
                    (from, day)
                } else {
                    (day, to)
                }
            }
            // The pickers only render while Custom; keep the state honest.
            DateRange::Preset(_) => (day, day),
        };
        // Picking the end before the start normalizes to the swapped pair
        // (a reversed range would match no rows and blank the dashboard on
        // every later launch). `apply_date_range` pushes the corrected
        // order back into both pickers, so the widgets never show `from >
        // to` either.
        let (from, to) = if from > to { (to, from) } else { (from, to) };
        self.apply_date_range(DateRange::Custom { from, to }, cx);
        self.persist_date_range(cx);
    }

    /// The accounts checked in the filter dropdown: every account whose id
    /// is not in `excluded_accounts` (task 3.3).
    fn selected_account_ids(&self) -> Vec<i64> {
        self.accounts
            .iter()
            .filter(|a| !self.filter.excluded_accounts.contains(&a.id))
            .map(|a| a.id)
            .collect()
    }

    /// Mirror the account checkboxes into [`ViewFilter::account_ids`]
    /// (empty = all accounts). With *every* account unchecked the list
    /// collapses to empty, which reads as "all" to the chart queries; the
    /// table path handles that case itself in [`Self::refresh`].
    pub(super) fn sync_account_filter(&mut self) {
        self.view_filter.account_ids = if self.filter.excluded_accounts.is_empty() {
            Vec::new()
        } else {
            self.selected_account_ids()
        };
    }

    /// Check or uncheck one account in the filter dropdown; the table
    /// follows immediately.
    pub(super) fn set_account_checked(
        &mut self,
        id: i64,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter.excluded_accounts.retain(|x| *x != id);
        if !on {
            self.filter.excluded_accounts.push(id);
        }
        self.sync_account_filter();
        // 3.65: the checkbox repaints with this click, not when the
        // answer comes back.
        cx.notify();
        self.reload(cx);
    }

    /// The "Include transfers" checkbox: update the filter, refresh the
    /// table, and persist the choice as `ui.include_transfers`.
    pub(super) fn set_include_transfers(
        &mut self,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        if on == self.filter.include_transfers {
            return;
        }
        self.filter_touched = true;
        self.filter.include_transfers = on;
        self.view_filter.include_transfers = on;
        let db = self.db.clone();
        self.include_transfers_task = Some(cx.spawn(async move |_, _| {
            let _ = on_db(&db, move |c| settings::set_include_transfers(c, on)).await;
        }));
        // 3.65: the checkbox and the Options label repaint with this click.
        cx.notify();
        self.reload(cx);
    }

    /// The search box changed: re-run the table query with the new term.
    /// Empty text clears the search.
    pub(super) fn apply_search(
        &mut self,
        term: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if term == self.filter.search {
            return;
        }
        self.filter.search = term.to_string();
        // 3.65: the box repaints with the keystroke, not with the answer.
        cx.notify();
        self.reload(cx);
    }

    /// The "Uncategorized only" checkbox: narrow the table to rows with no
    /// category. Mutually exclusive with a category legend selection (3.5) —
    /// both feed the one table filter, and `category = X AND no category`
    /// could only ever be an empty table.
    pub(super) fn set_uncategorized_only(
        &mut self,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if on == self.filter.uncategorized_only {
            return;
        }
        self.filter.uncategorized_only = on;
        if on
            && matches!(
                self.filter.legend_selection,
                Some(LegendSelection::Category(_))
            )
        {
            self.filter.legend_selection = None;
            self.filter.summary_month = None;
        }
        // 3.65: the box and any cleared legend highlight repaint at once.
        cx.notify();
        self.reload(cx);
    }

    /// The "Show hidden" checkbox: include hidden rows in the table (greyed).
    pub(super) fn set_show_hidden(
        &mut self,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if on == self.filter.show_hidden {
            return;
        }
        self.filter.show_hidden = on;
        // 3.65: the checkbox repaints with this click, not with the answer.
        cx.notify();
        self.reload(cx);
    }

    /// A header sort icon was clicked: adopt the new ordering and re-query.
    /// This runs through `on_sort` from inside the table's own update, so it
    /// must not touch the table entity again (that would double-lease and
    /// panic). `TxTable::perform_sort` already set the delegate's `sort` to
    /// this value, and the async refresh continuation re-syncs it outside the
    /// lease, so only the shell's copy needs updating here.
    pub(super) fn set_tx_sort(
        &mut self,
        sort: TxSort,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter.tx_sort = sort;
        // 3.65: the header icon repaints with this click.
        cx.notify();
        self.reload(cx);
    }

    /// Click the "← All categories" row: back to the parent level with no
    /// category filter on the table.
    pub(super) fn show_all_categories(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.filter.chart_parent = None;
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        // 3.65: the legend repaints with this click, not with the answer.
        cx.notify();
        self.reload(cx);
    }

    /// Esc (the `dashboard::Cancel` action): clear the
    /// drill-down, the legend selection and the summary month together —
    /// they are one concept. 3.24: an open rule editor is the innermost
    /// thing, so Esc cancels it first and nothing else.
    pub(super) fn clear_category_selection(
        &mut self,
        _: &Cancel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 3.80: the editor lives on the Rules page.
        let editing = self.rules_page.read(cx).editing_rule.is_some();
        if editing {
            self.rules_page.update(cx, |p, cx| p.cancel_edit_rule(cx));
            return;
        }
        if self.filter.chart_parent.is_none()
            && self.filter.legend_selection.is_none()
            && self.filter.summary_month.is_none()
        {
            return;
        }
        self.filter.chart_parent = None;
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        // 3.65: the drill-down, legend highlight and summary-cell highlight
        // repaint with this Esc, not with the answer.
        cx.notify();
        self.reload(cx);
    }

    /// Change one transaction's category from the row menu (2.9). `set_category`
    /// locks the row so rules never move it again (2.6); on success the row
    /// starts offering to extend this choice to its whole merchant group.
    pub(super) fn change_row_category(
        &mut self,
        transaction_id: i64,
        category_id: i64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in
        // flight — the row id may name a different transaction after it.
        if self.writes_blocked.get() {
            return;
        }
        let db = self.db.clone();
        // 3.64: record the choice now — the continuation compares against
        // it, and the table's rows lag a whole refresh behind.
        self.last_category_choice = Some((transaction_id, category_id));
        // 3.64: detached — a later action must not cancel the write's
        // continuation and swallow its refresh.
        cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| set_category(c, transaction_id, category_id)).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        // The row itself starts offering the rule (2.6); no
                        // separate feedback. Don't open the offer over a
                        // newer choice made on this row while the write was
                        // in flight (3.64): only a *different* later choice
                        // on the same row supersedes this one.
                        let superseded = this
                            .last_category_choice
                            .is_some_and(|(id, cat)| id == transaction_id && cat != category_id);
                        if !superseded {
                            this.rule_offer = Some((transaction_id, category_id));
                        }
                    }
                    Err(e) => this.notify_err(e, cx),
                }
                // 3.82: the write's report goes out as a `PageEvent`; the
                // shell reloads the shared sections, the `DataChanged`
                // comes back, and this page reloads its rows with it.
                cx.emit(PageEvent::Changed(Domains::TX));
            });
        })
        .detach();
    }

    /// Hide or unhide a transaction from the row menu (2.9). Hidden rows leave
    /// the table, the sidebar counts and the To-categorize queue at once.
    pub(super) fn set_row_hidden(
        &mut self,
        transaction_id: i64,
        hidden: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Hiding the row a rule offer is on would strand the offer.
        if self.rule_offer.is_some_and(|(id, _)| id == transaction_id) {
            self.rule_offer = None;
        }
        let db = self.db.clone();
        // 3.64: detached — see `change_row_category`.
        cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| set_hidden(c, transaction_id, hidden)).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.notify_err(e, cx);
                }
                cx.emit(PageEvent::Changed(Domains::TX));
            });
        })
        .detach();
    }

    /// Accept the post-edit offer from the row (2.6, 2.9): turn the merchant of
    /// the row just edited into a substring rule for the category chosen,
    /// categorizing every matching row, then clear the offer.
    pub(super) fn apply_row_rule(
        &mut self,
        transaction_id: i64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some((_, category_id)) = self.rule_offer.filter(|(id, _)| *id == transaction_id) else {
            return;
        };
        self.rule_offer = None;
        let db = self.db.clone();
        // 3.64: detached — see `change_row_category`.
        cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| {
                apply_rule_from_merchant(c, transaction_id, category_id)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.notify_ok("Applied the category to all matching rows", cx);
                    }
                    Err(e) => this.notify_err(e, cx),
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    // -- render (3.82) --------------------------------------------------------
}

impl Render for DashboardPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 3.82: the page takes focus on the frame it is first shown, like
        // the other extracted pages — deferred so no element is borrowed
        // while the focus moves, and only once (D5).
        if self.focus_pending {
            self.focus_pending = false;
            let focus = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        // A load that landed without a window (3.82) marked the pickers;
        // re-sync them now that one is here.
        self.sync_date_widgets(window, cx);
        let muted = cx.theme().muted_foreground;
        let body = self.dashboard_body();
        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            // 3.82: the page is a focus root on its own key context, and
            // Esc — `Cancel` on "Dashboard" (bound in `ui::init`) — clears
            // the category selection, the behaviour the Root-scoped
            // `ClearCategorySelection` binding had before the extraction
            // (3.5). Deeper contexts still win: an open picker or menu
            // takes the Escape first.
            .track_focus(&self.focus)
            .key_context("Dashboard")
            .on_action(cx.listener(|this, action: &Cancel, window, cx| {
                this.clear_category_selection(action, window, cx);
            }))
            // One filter row (3.35, ui-conventions *Pages*): no region
            // title and no label lines. Every control is 32 px tall and
            // centred on the row; the controls without a visible label
            // name themselves in a hover tooltip. Left to right: date
            // range, the custom From/To pickers, accounts, search (takes
            // the slack), Options (the three toggles), then the count.
            .child(
                h_flex()
                    .w_full()
                    .h(FILTER_CONTROL_H)
                    .debug_selector(|| "filters:region".into())
                    .gap_2()
                    .items_center()
                    .child(
                        filter_tip(
                            "filters:date-range-tip",
                            "Date range",
                            div()
                                .debug_selector(|| "filters:date-range".into())
                                .w(FILTER_DATE_RANGE_W)
                                .child(Select::new(&self.date_preset_select)),
                        )
                        .flex_none(),
                    )
                    // The From/To pickers want 140 px but shrink first
                    // (floor 110 px) when the row runs out of room, so the
                    // custom range fits the minimum window (3.25, 3.35).
                    .when(
                        matches!(self.filter.date_range, DateRange::Custom { .. }),
                        |g| {
                            g.child(
                                div()
                                    .debug_selector(|| "filters:from".into())
                                    .w(FILTER_DATE_W)
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W)
                                    .child(DatePicker::new(&self.date_from_picker)),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "filters:to".into())
                                    .w(FILTER_DATE_W)
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W)
                                    .child(DatePicker::new(&self.date_to_picker)),
                            )
                        },
                    )
                    // Accounts multi-select (3.3): a dropdown popover of
                    // checkboxes, one per account, all checked by default.
                    // The selector (test builds only, 3.11) lets headless
                    // tests find the trigger and click it.
                    .child(
                        filter_tip(
                            "filters:accounts-tip",
                            "Accounts",
                            div()
                                .debug_selector(|| "accounts:filter".into())
                                .child(self.accounts_popover(cx)),
                        )
                        .flex_none(),
                    )
                    // Search (3.4) takes the row's slack; the placeholder
                    // and the leading icon say what it is.
                    .child(
                        div()
                            .debug_selector(|| "filters:search".into())
                            .flex_1()
                            .min_w(FILTER_SEARCH_MIN_W)
                            .child(
                                Input::new(&self.search_input)
                                    .prefix(Icon::new(IconName::Search).small().text_color(muted)),
                            ),
                    )
                    .child(
                        filter_tip(
                            "filters:options-tip",
                            "Options",
                            div()
                                .debug_selector(|| "filters:options".into())
                                .child(self.options_popover(cx)),
                        )
                        .flex_none(),
                    )
                    // The count ends the row at the page's right edge.
                    .child(
                        div()
                            .debug_selector(|| "filters:count".into())
                            .flex_none()
                            .ml_auto()
                            .whitespace_nowrap()
                            .text_sm()
                            .text_color(muted)
                            .child(format!(
                                "{} transactions",
                                flow::count(self.row_count as i64)
                            )),
                    ),
            )
            .child(match body {
                // 5.3: an empty database is also where a restore begins,
                // so the empty state offers it beside the import hint.
                DashboardBody::NoData => v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(muted)
                    .child("Drop an Amex or Simplii CSV export here, or press ⌘O")
                    .child(
                        Button::new("restore-from-export")
                            .link()
                            .label("Restore from export…")
                            .debug_selector(|| "dashboard:restore-link".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.restore_backup(window, cx);
                            })),
                    )
                    .into_any_element(),
                _ => {
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .flex_1()
                        .min_h_0()
                        .gap_3()
                        // Middle row: chart + legend left (3.5), the monthly
                        // summary right (3.6), both inside `chart_panel`.
                        .child(self.chart_panel(cx))
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex_1()
                                .min_h_0()
                                .debug_selector(|| "panel:table".into())
                                .child(if body == DashboardBody::NoMatches {
                                    self.no_matches_panel(cx)
                                } else {
                                    DataTable::new(&self.table).stripe(false).into_any_element()
                                }),
                        )
                        .into_any_element()
                }
            })
            .into_any_element()
    }
}

impl DashboardPage {
    /// "Clear filters": drop every filter that narrows the table — search,
    /// "Uncategorized only", and the category/month selection with its
    /// chart drill-down. `set_value` doesn't emit `InputEvent::Change`, so
    /// `self.filter.search` is cleared here too before the single re-query.
    pub(super) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A keystroke within the last 200 ms left a debounced search pending
        // that would re-apply the dead query right after this reset, so drop
        // it with the text.
        self.search_task = None;
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.filter.search.clear();
        self.filter.uncategorized_only = false;
        self.filter.chart_parent = None;
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        // 3.65: the cleared box, legend highlight and summary-cell highlight
        // repaint with this click, not with the answer.
        cx.notify();
        self.reload(cx);
    }
}

impl DashboardPage {
    // -- shell forwards (3.82) ------------------------------------------------
    //
    // The file actions live on the Imports page and transfer pairing on the
    // Transfers page, like the other extracted pages' forwards; the shared
    // rule rows and the *Show rule* reveal live on the Rules page. The page
    // reaches them through the shell handle — the same routes the sidebar
    // and the global action listeners use, so the behaviour is identical.

    pub(super) fn restore_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shell = self.shell.clone();
        if let Some(shell) = shell.upgrade() {
            shell.update(cx, |this, cx| this.restore_backup(window, cx));
        }
    }

    pub(super) fn unpair_row(
        &mut self,
        transaction_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = self.shell.clone();
        if let Some(shell) = shell.upgrade() {
            shell.update(cx, |this, cx| this.unpair_row(transaction_id, window, cx));
        }
    }

    pub(super) fn show_rule(&mut self, rule_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.rules_page
            .update(cx, |p, cx| p.show_rule(rule_id, window, cx));
    }

    /// A window resize re-flexes the table's columns (3.25) and re-renders
    /// for the chart's label plan (3.15); `Shell::window_bounds_changed`
    /// forwards the window width here (3.82). The chart deducts the chrome
    /// itself, so the page keeps the raw width.
    pub(super) fn widths_changed(&mut self, window_width: f32, cx: &mut Context<Self>) {
        self.window_width = window_width;
        let page_w = (window_width - SIDEBAR_W - 2. * PAGE_PAD).max(0.);
        let flex = tx_flex_widths(page_w);
        self.table.update(cx, |t, cx| {
            t.delegate_mut().flex_widths = flex;
            t.refresh(cx);
        });
    }

    /// A restore replaced the whole database behind the UI (5.3): the
    /// legend, month and drill-down selections name categories and months
    /// the restored file may not have, and the rule offer names rows whose
    /// dense ids may point elsewhere now. `Shell::reset_pages_after_restore`
    /// forwards here (3.82).
    pub(super) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        self.filter.chart_parent = None;
        self.rule_offer = None;
        self.last_category_choice = None;
        cx.notify();
    }
}
