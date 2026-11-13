//! The Dashboard page: filters, chart, summary, the transactions table
//! and its row menu (3.62).

use super::dashboard_filters::{
    FILTER_CONTROL_H, FILTER_DATE_MIN_W, FILTER_DATE_RANGE_W, FILTER_DATE_W, FILTER_SEARCH_MIN_W,
};
use super::*;
use crate::ui::widgets::*;

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
        let row = &self.rows[row_ix];
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

/// This page's state (3.74): the filters, the chart and summary panels'
/// selections, the date-range pickers and the transaction table.
pub(super) struct DashboardState {
    /// The accounts the user unchecked in the dropdown (empty = all).
    pub(super) excluded_accounts: Vec<i64>,
    /// The "Include transfers" flag persisted as `ui.include_transfers`.
    pub(super) include_transfers: bool,
    pub(super) include_transfers_task: Option<Task<()>>,
    /// The search term and the table-only toggles and sort. Empty / false /
    /// the default sort mean "no narrowing". (The category and month the
    /// query also supports are chosen from the chart and legend in 3.5/3.6.)
    pub(super) search_input: Entity<InputState>,
    pub(super) search: String,
    pub(super) uncategorized_only: bool,
    pub(super) show_hidden: bool,
    pub(super) tx_sort: TxSort,
    /// The chart and legend drill into (`None` = the parent level), the one
    /// legend selection that drives the table's category filter, and the
    /// `spend_by_category` rows behind the chart and legend.
    pub(super) chart_parent: Option<i64>,
    pub(super) legend_selection: Option<LegendSelection>,
    pub(super) chart_rows: Vec<CategorySpend>,
    /// The panel, the parents whose child rows are expanded, and the month
    /// chosen from a summary cell (fed into the table's `month` filter; the
    /// `Total` column and "no month narrowing" are both `None`).
    pub(super) summary: MonthlySummary,
    pub(super) expanded_parents: HashSet<i64>,
    pub(super) summary_month: Option<String>,
    /// One horizontal scroll handle for the whole month strip of the
    /// dashboard summary (3.30): the header's month row and every row's
    /// month cells track it, so they scroll together, while the name
    /// column and the `Total` column stay pinned.
    pub(super) summary_scroll: ScrollHandle,
    /// The saved date range and the picker entities.
    pub(super) date_range: DateRange,
    pub(super) view_filter: ViewFilter,
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
    /// The pickers and inputs above subscribe here.
    _subs: Vec<Subscription>,
}

impl DashboardState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let on_set_category = {
            let shell = cx.entity();
            Rc::new(
                move |txn: i64, category: i64, window: &mut Window, cx: &mut App| {
                    shell.update(cx, |this, cx| {
                        this.change_row_category(txn, category, window, cx);
                    });
                },
            ) as Rc<dyn Fn(i64, i64, &mut Window, &mut App)>
        };
        let on_set_hidden = {
            let shell = cx.entity();
            Rc::new(
                move |txn: i64, hidden: bool, window: &mut Window, cx: &mut App| {
                    shell.update(cx, |this, cx| this.set_row_hidden(txn, hidden, window, cx));
                },
            ) as Rc<dyn Fn(i64, bool, &mut Window, &mut App)>
        };
        let on_apply_rule = {
            let shell = cx.entity();
            Rc::new(move |txn: i64, window: &mut Window, cx: &mut App| {
                shell.update(cx, |this, cx| this.apply_row_rule(txn, window, cx));
            }) as Rc<dyn Fn(i64, &mut Window, &mut App)>
        };
        let on_sort = {
            let shell = cx.entity();
            Rc::new(move |sort: TxSort, window: &mut Window, cx: &mut App| {
                shell.update(cx, |this, cx| this.set_tx_sort(sort, window, cx));
            }) as Rc<dyn Fn(TxSort, &mut Window, &mut App)>
        };
        let on_unpair = {
            let shell = cx.entity();
            Rc::new(move |txn: i64, window: &mut Window, cx: &mut App| {
                shell.update(cx, |this, cx| this.unpair_row(txn, window, cx));
            }) as UnpairFn
        };
        let on_show_rule = {
            let shell = cx.entity();
            Rc::new(move |rule: i64, window: &mut Window, cx: &mut App| {
                shell.update(cx, |this, cx| this.show_rule(rule, window, cx));
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
        // The dashboard search box filters the table on every keystroke.
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search transactions"));
        let search_sub = cx.subscribe_in(
            &search_input,
            window,
            |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let value = this
                        .dashboard
                        .search_input
                        .read(cx)
                        .value()
                        .trim()
                        .to_string();
                    this.apply_search(&value, window, cx);
                }
            },
        );
        Self {
            excluded_accounts: Vec::new(),
            include_transfers: false,
            include_transfers_task: None,
            search_input,
            search: String::new(),
            uncategorized_only: false,
            show_hidden: false,
            tx_sort: TxSort::default(),
            chart_parent: None,
            legend_selection: None,
            chart_rows: Vec::new(),
            summary: MonthlySummary {
                months: Vec::new(),
                rows: Vec::new(),
            },
            expanded_parents: HashSet::new(),
            summary_month: None,
            summary_scroll: ScrollHandle::new(),
            date_range: DateRange::default(),
            view_filter: ViewFilter::default(),
            date_preset_select,
            date_from_picker,
            date_to_picker,
            date_range_task: None,
            table,
            row_count: 0,
            rule_offer: None,
            last_category_choice: None,
            _subs: vec![date_preset_sub, date_from_sub, date_to_sub, search_sub],
        }
    }
}

impl Shell {
    /// Apply the dashboard's part of a refresh result (3.74), last: the
    /// table's delegate reads the child choices and card-payment category
    /// `sync_category_choices` just set.
    pub(super) fn apply_dashboard(
        &mut self,
        rows: Vec<TransactionRow>,
        chart_rows: Vec<CategorySpend>,
        summary: MonthlySummary,
        sidebar: (i64, i64),
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) {
        self.dashboard.row_count = rows.len();
        self.dashboard.chart_rows = chart_rows;
        // A range change shows new months: park the month strip (3.30) at
        // its right end so the most recent month sits beside the pinned
        // `Total` column. The huge negative offset clamps to the true
        // maximum when the strip next paints.
        if self.dashboard.summary.months != summary.months {
            self.dashboard
                .summary_scroll
                .set_offset(point(px(-1_000_000.), px(0.)));
        }
        self.dashboard.summary = summary;
        // Revalidate the legend state against the rows we can actually
        // show: a date/accounts change can empty the chart out from under a
        // selection or a drill-down, and the table filter must not keep
        // narrowing by a category that is no longer there.
        let mut stale = false;
        let drilled = self.dashboard.chart_parent;
        if drilled.is_some() && self.dashboard.chart_rows.is_empty() {
            self.dashboard.chart_parent = None;
            stale = true;
        }
        let selection_gone = match self.dashboard.legend_selection {
            Some(LegendSelection::Category(id)) => {
                // While drilled, the selected parent itself is not among
                // the child rows — it stays.
                drilled != Some(id) && !self.dashboard.chart_rows.iter().any(|r| r.id == Some(id))
            }
            Some(LegendSelection::Uncategorized) => {
                self.dashboard.chart_parent.is_none()
                    && !self.dashboard.chart_rows.iter().any(|r| r.id.is_none())
            }
            None => false,
        };
        if selection_gone {
            self.dashboard.legend_selection = None;
            stale = true;
        }
        // The month selection (3.6) goes the same way when the grid no
        // longer contains that month.
        if self.dashboard.summary_month.is_some()
            && !self
                .dashboard
                .summary
                .months
                .iter()
                .any(|m| Some(m.as_str()) == self.dashboard.summary_month.as_deref())
        {
            self.dashboard.summary_month = None;
            stale = true;
        }
        if stale {
            // Re-query with the corrected filter so the table, chart and
            // state agree again.
            self.refresh(window, cx);
        }
        self.total_visible = sidebar.0;
        self.uncategorized_visible = sidebar.1;
        // The rule offer follows a row the user just edited; a refresh
        // that drops the row (hidden, filtered out, deleted) drops the
        // offer with it.
        if self
            .dashboard
            .rule_offer
            .is_some_and(|(id, _)| !rows.iter().any(|r| r.id == id))
        {
            self.dashboard.rule_offer = None;
        }
        let child_choices = self.child_choices.clone();
        let rule_offer = self.dashboard.rule_offer.map(|(id, _)| id);
        let sort = self.dashboard.tx_sort;
        let rules = self.rules.clone();
        let card_payment = self.card_payment_category;
        self.dashboard.table.update(cx, |t, cx| {
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

impl Shell {
    /// Today, for the preset ranges; `preset_range` itself never reads a
    /// clock.
    pub(super) fn today() -> NaiveDate {
        chrono::Local::now().date_naive()
    }

    /// Apply a date range to the state and the controls without
    /// persisting; user changes persist on top via [`Self::persist_date_range`].
    fn apply_date_range(&mut self, range: DateRange, window: &mut Window, cx: &mut Context<Self>) {
        self.dashboard.date_range = range;
        let (from, to) = range.range(Self::today());
        self.dashboard.view_filter.from = from;
        self.dashboard.view_filter.to = to;
        let index = DatePreset::ALL
            .iter()
            .position(|p| *p == range.preset())
            .unwrap_or(0);
        self.dashboard.date_preset_select.update(cx, |select, cx| {
            select.set_selected_index(Some(gpui_kit::component::IndexPath::new(index)), window, cx);
        });
        if let DateRange::Custom { from, to } = range {
            // `set_date` does not emit `Change`, so this never re-persists.
            self.dashboard.date_from_picker.update(cx, |picker, cx| {
                picker.set_date(CalendarDate::Single(Some(from)), window, cx);
            });
            self.dashboard.date_to_picker.update(cx, |picker, cx| {
                picker.set_date(CalendarDate::Single(Some(to)), window, cx);
            });
        }
        // The table now reads the date range, so it re-queries (the launch
        // restore path refreshes right after, so this is cheap there too).
        self.refresh(window, cx);
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
        let range = self.dashboard.date_range;
        self.dashboard.date_range_task = Some(cx.spawn(async move |_, _| {
            let _ = on_db(&db, move |c| settings::set_date_range(c, &range)).await;
        }));
    }

    /// The user picked a preset in the Select (task 3.2).
    pub(super) fn change_date_preset(
        &mut self,
        preset: DatePreset,
        window: &mut Window,
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
        self.apply_date_range(range, window, cx);
        self.persist_date_range(cx);
    }

    /// The user changed one of the two custom [`DatePicker`]s.
    pub(super) fn date_picked(
        &mut self,
        is_end: bool,
        day: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (from, to) = match self.dashboard.date_range {
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
        self.apply_date_range(DateRange::Custom { from, to }, window, cx);
        self.persist_date_range(cx);
    }

    /// Restore the persisted `ui.date_range` (default Last 3 months) at
    /// launch; the controls and `view_filter` follow the saved value.
    pub(super) fn load_date_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        self.dashboard.date_range_task = Some(cx.spawn_in(window, async move |this, cx| {
            let saved = on_db(&db, |c| settings::date_range(c)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Ok(range) = saved {
                    this.apply_date_range(range, window, cx);
                }
            });
        }));
    }

    /// The accounts checked in the filter dropdown: every account whose id
    /// is not in `excluded_accounts` (task 3.3).
    fn selected_account_ids(&self) -> Vec<i64> {
        self.accounts
            .iter()
            .filter(|a| !self.dashboard.excluded_accounts.contains(&a.id))
            .map(|a| a.id)
            .collect()
    }

    /// Mirror the account checkboxes into [`ViewFilter::account_ids`]
    /// (empty = all accounts). With *every* account unchecked the list
    /// collapses to empty, which reads as "all" to the chart queries; the
    /// table path handles that case itself in [`Self::refresh`].
    pub(super) fn sync_account_filter(&mut self) {
        self.dashboard.view_filter.account_ids = if self.dashboard.excluded_accounts.is_empty() {
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dashboard.excluded_accounts.retain(|x| *x != id);
        if !on {
            self.dashboard.excluded_accounts.push(id);
        }
        self.sync_account_filter();
        self.refresh(window, cx);
    }

    /// The "Include transfers" checkbox: update the filter, refresh the
    /// table, and persist the choice as `ui.include_transfers`.
    pub(super) fn set_include_transfers(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        if on == self.dashboard.include_transfers {
            return;
        }
        self.dashboard.include_transfers = on;
        self.dashboard.view_filter.include_transfers = on;
        let db = self.db.clone();
        self.dashboard.include_transfers_task = Some(cx.spawn(async move |_, _| {
            let _ = on_db(&db, move |c| settings::set_include_transfers(c, on)).await;
        }));
        self.refresh(window, cx);
    }

    /// The search box changed: re-run the table query with the new term.
    /// Empty text clears the search.
    pub(super) fn apply_search(&mut self, term: &str, window: &mut Window, cx: &mut Context<Self>) {
        if term == self.dashboard.search {
            return;
        }
        self.dashboard.search = term.to_string();
        self.refresh(window, cx);
    }

    /// The "Uncategorized only" checkbox: narrow the table to rows with no
    /// category. Mutually exclusive with a category legend selection (3.5) —
    /// both feed the one table filter, and `category = X AND no category`
    /// could only ever be an empty table.
    pub(super) fn set_uncategorized_only(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if on == self.dashboard.uncategorized_only {
            return;
        }
        self.dashboard.uncategorized_only = on;
        if on
            && matches!(
                self.dashboard.legend_selection,
                Some(LegendSelection::Category(_))
            )
        {
            self.dashboard.legend_selection = None;
            self.dashboard.summary_month = None;
        }
        self.refresh(window, cx);
    }

    /// The "Show hidden" checkbox: include hidden rows in the table (greyed).
    pub(super) fn set_show_hidden(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if on == self.dashboard.show_hidden {
            return;
        }
        self.dashboard.show_hidden = on;
        self.refresh(window, cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dashboard.tx_sort = sort;
        self.refresh(window, cx);
    }

    /// Click the "← All categories" row: back to the parent level with no
    /// category filter on the table.
    pub(super) fn show_all_categories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dashboard.chart_parent = None;
        self.dashboard.legend_selection = None;
        self.dashboard.summary_month = None;
        self.refresh(window, cx);
    }

    /// Esc (the `dashboard::ClearCategorySelection` action): clear the
    /// drill-down, the legend selection and the summary month together —
    /// they are one concept. 3.24: an open rule editor is the innermost
    /// thing, so Esc cancels it first and nothing else.
    pub(super) fn clear_category_selection(
        &mut self,
        _: &ClearCategorySelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_rule.is_some() {
            self.cancel_edit_rule(cx);
            return;
        }
        if self.dashboard.chart_parent.is_none()
            && self.dashboard.legend_selection.is_none()
            && self.dashboard.summary_month.is_none()
        {
            return;
        }
        self.dashboard.chart_parent = None;
        self.dashboard.legend_selection = None;
        self.dashboard.summary_month = None;
        self.refresh(window, cx);
    }

    /// Restore the persisted `ui.include_transfers` (default off) at
    /// launch; unlike the date range the table query reads the filter, so
    /// a saved "on" refreshes the rows.
    pub(super) fn load_include_transfers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        self.dashboard.include_transfers_task = Some(cx.spawn_in(window, async move |this, cx| {
            let saved = on_db(&db, |c| settings::include_transfers(c)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Ok(true) = saved {
                    this.dashboard.include_transfers = true;
                    this.dashboard.view_filter.include_transfers = true;
                    this.refresh(window, cx);
                }
            });
        }));
    }

    /// Change one transaction's category from the row menu (2.9). `set_category`
    /// locks the row so rules never move it again (2.6); on success the row
    /// starts offering to extend this choice to its whole merchant group.
    pub(super) fn change_row_category(
        &mut self,
        transaction_id: i64,
        category_id: i64,
        window: &mut Window,
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
        self.dashboard.last_category_choice = Some((transaction_id, category_id));
        // 3.64: detached — a later action must not cancel the write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| set_category(c, transaction_id, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        // The row itself starts offering the rule (2.6); no
                        // separate feedback. Don't open the offer over a
                        // newer choice made on this row while the write was
                        // in flight (3.64): only a *different* later choice
                        // on the same row supersedes this one.
                        let superseded = this
                            .dashboard
                            .last_category_choice
                            .is_some_and(|(id, cat)| id == transaction_id && cat != category_id);
                        if !superseded {
                            this.dashboard.rule_offer = Some((transaction_id, category_id));
                        }
                    }
                    Err(e) => this.notify_err(e, cx),
                }
                this.refresh(window, cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Hiding the row a rule offer is on would strand the offer.
        if self
            .dashboard
            .rule_offer
            .is_some_and(|(id, _)| id == transaction_id)
        {
            self.dashboard.rule_offer = None;
        }
        let db = self.db.clone();
        // 3.64: detached — see `change_row_category`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| set_hidden(c, transaction_id, hidden)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(e) = result {
                    this.notify_err(e, cx);
                }
                this.refresh(window, cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some((_, category_id)) = self
            .dashboard
            .rule_offer
            .filter(|(id, _)| *id == transaction_id)
        else {
            return;
        };
        self.dashboard.rule_offer = None;
        let db = self.db.clone();
        // 3.64: detached — see `change_row_category`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                apply_rule_from_merchant(c, transaction_id, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.notify_ok("Applied the category to all matching rows", cx);
                    }
                    Err(e) => this.notify_err(e, cx),
                }
                this.refresh(window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    // -- toasts (3.27) ------------------------------------------------------

    pub(super) fn render_dashboard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let body = self.dashboard_body();
        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
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
                                .child(Select::new(&self.dashboard.date_preset_select)),
                        )
                        .flex_none(),
                    )
                    // The From/To pickers want 140 px but shrink first
                    // (floor 110 px) when the row runs out of room, so the
                    // custom range fits the minimum window (3.25, 3.35).
                    .when(
                        matches!(self.dashboard.date_range, DateRange::Custom { .. }),
                        |g| {
                            g.child(
                                div()
                                    .debug_selector(|| "filters:from".into())
                                    .w(FILTER_DATE_W)
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W)
                                    .child(DatePicker::new(&self.dashboard.date_from_picker)),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "filters:to".into())
                                    .w(FILTER_DATE_W)
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W)
                                    .child(DatePicker::new(&self.dashboard.date_to_picker)),
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
                                Input::new(&self.dashboard.search_input)
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
                                flow::count(self.dashboard.row_count as i64)
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
                                    DataTable::new(&self.dashboard.table)
                                        .stripe(true)
                                        .into_any_element()
                                }),
                        )
                        .into_any_element()
                }
            })
            .into_any_element()
    }

    /// "Clear filters": drop every filter that narrows the table — search,
    /// "Uncategorized only", and the category/month selection with its
    /// chart drill-down. `set_value` doesn't emit `InputEvent::Change`, so
    /// `self.dashboard.search` is cleared here too before the single re-query.
    pub(super) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dashboard
            .search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.dashboard.search.clear();
        self.dashboard.uncategorized_only = false;
        self.dashboard.chart_parent = None;
        self.dashboard.legend_selection = None;
        self.dashboard.summary_month = None;
        self.refresh(window, cx);
    }
}
