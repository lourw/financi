//! The Dashboard page: filters, chart, summary, the transactions table
//! and its row menu (3.62).

use super::*;
use crate::ui::widgets::*;

/// The Options button's label (3.35): "Options", or "Options · N" when N
/// of its toggles (include transfers, uncategorized only, show hidden) are
/// on.
fn options_label(on: usize) -> String {
    if on == 0 {
        "Options".to_string()
    } else {
        format!("Options · {on}")
    }
}

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
    short: SharedString,
    pub(super) bar_label: SharedString,
    cents: i64,
    pub(super) share: f64,
    color: Hsla,
}

/// The legend column inside the chart panel (3.31): just swatch and name,
/// so it no longer carries the amount and share columns and the plot gets
/// the width back.
pub(super) const LEGEND_W: f32 = 140.;
const CHART_PANEL_SHARE: f32 = 0.45;
const REGION_GAP: f32 = 12.;
// Filter row (3.35): the height every filter control shares, the date-range
// select's width (fits "Last 12 months" on one line), and the search box's
// floor.
const FILTER_CONTROL_H: Pixels = px(32.);
const FILTER_DATE_RANGE_W: Pixels = px(170.);
const FILTER_SEARCH_MIN_W: Pixels = px(140.);
/// The custom range's From/To pickers: 140 px when there is room, shrinking
/// first, down to [`FILTER_DATE_MIN_W`], when the window is at its minimum
/// and the rest of the row must still fit on the page (3.25, 3.35).
const FILTER_DATE_W: Pixels = px(140.);
const FILTER_DATE_MIN_W: Pixels = px(110.);

/// Minimum horizontal gap between two neighbouring drawn band labels.
pub(super) const CHART_LABEL_GAP: f32 = 8.;
/// Cap on a band label, so a wide chart never paints a sentence.
pub(super) const CHART_LABEL_MAX_CHARS: usize = 16;
/// Smallest budget that makes a label worth drawing; below it labels are
/// thinned out instead of reduced to unreadable slivers.
pub(super) const CHART_LABEL_MIN_CHARS: usize = 4;

/// The chart's label plan for `chart_width` pixels across `count` bands:
/// band labels are truncated to `chars` characters (with an ellipsis)
/// and only `labels` of them are drawn, spread evenly first-to-last by
/// `BarChart::band_tick_count`. A centred label cannot collide with its
/// neighbours once every drawn label owns `chart_width / labels` pixels,
/// so that slice always buys `chars · CHART_CHAR_PX + CHART_LABEL_GAP`.
pub(super) fn band_label_budget(chart_width: f32, count: usize) -> (usize, usize) {
    if count == 0 {
        return (0, 0);
    }
    let chars_for = |labels: usize| {
        let per = chart_width / labels as f32;
        (((per - CHART_LABEL_GAP) / CHART_CHAR_PX).max(0.)) as usize
    };
    // Prefer labelling every band.
    if chars_for(count) >= CHART_LABEL_MIN_CHARS {
        return (chars_for(count).min(CHART_LABEL_MAX_CHARS), count);
    }
    // Too tight: thin the labels out until each drawn one buys the
    // minimum budget, then truncate to what its wider slice allows.
    let min_spacing = CHART_LABEL_MIN_CHARS as f32 * CHART_CHAR_PX + CHART_LABEL_GAP;
    let labels = ((chart_width / min_spacing) as usize).clamp(1, count);
    (chars_for(labels).clamp(1, CHART_LABEL_MAX_CHARS), labels)
}

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

/// Fit `names` into the band axis (3.15): a name longer than `chars`
/// becomes `chars − 1` characters plus `…` (trailing whitespace is
/// dropped first). Shortenings that collide fall back to the full name,
/// because each row's band text is also its unique key in `BarChart`'s
/// band scale (sibling category names are unique in the database).
/// `chars == 0` means "no budget": the full names come back untouched.
pub(super) fn short_chart_labels(names: &[String], chars: usize) -> Vec<SharedString> {
    if chars == 0 {
        return names
            .iter()
            .map(|n| SharedString::from(n.clone()))
            .collect();
    }
    let shorts: Vec<String> = names
        .iter()
        .map(|name| {
            if name.chars().count() <= chars {
                return name.clone();
            }
            let mut head: String = name.chars().take(chars - 1).collect();
            while head.ends_with(char::is_whitespace) {
                head.pop();
            }
            head.push('…');
            head
        })
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for short in &shorts {
        *counts.entry(short.as_str()).or_default() += 1;
    }
    shorts
        .iter()
        .zip(names)
        .map(|(short, name)| {
            if counts[short.as_str()] > 1 {
                SharedString::from(name.clone())
            } else {
                SharedString::from(short.clone())
            }
        })
        .collect()
}

// -- Bar share labels (3.31) --------------------------------------------
//
// Each bar carries its share of the spend above it (`18%`), drawn by
// `BarChart::label` in gpui-kit's 10px chart font, centred on the bar.
// `gpui-component` reserves a line of text past the tallest bar for
// them, but offers no thinning of its own, so the app skips labels the
// way band labels are thinned (3.15): a whole number of bands per drawn
// label, spread evenly first-to-last, so centred labels cannot collide.

/// Widest share label: `100%`, or `0.4%` below one percent — four
/// characters of the 10px chart font.
pub(super) const BAR_LABEL_MAX_CHARS: usize = 4;

/// A bar's share label: whole percent (`18%`), one decimal only below
/// one percent (`0.4%`) so a thin slice still reads — and never `0.0%`:
/// a sliver too small to round to a tenth of a percent reads `0.1%`.
pub(super) fn bar_share_label(share: f64) -> String {
    let percent = share * 100.;
    if percent <= 0. {
        "0%".into()
    } else if percent < 1. {
        if percent < 0.05 {
            "0.1%".into()
        } else {
            format!("{percent:.1}%")
        }
    } else {
        format!("{percent:.0}%")
    }
}

/// The tooltip body of a chart row (3.31), shared by the bar's own
/// tooltip and the legend row's: `$1,234.56 · 18.2% of Food`. `parent`
/// carries the ` of …` suffix when the chart is drilled into a category,
/// and is empty at the top level.
pub(super) fn chart_tooltip_value(cents: i64, share: f64, parent: &str) -> String {
    format!("{} · {:.1}%{parent}", flow::money(cents), share * 100.)
}

/// How many of `count` bars carry their share label across
/// `chart_width` pixels. A drawn label is at most `BAR_LABEL_MAX_CHARS`
/// characters wide and keeps `CHART_LABEL_GAP` from its neighbours, so
/// their band centres must sit
/// `BAR_LABEL_MAX_CHARS · CHART_CHAR_PX + CHART_LABEL_GAP` apart; the
/// even first-to-last spread keeps the closest pair
/// `floor((count − 1) / (labels − 1))` bands apart, which the formula
/// below inverts.
pub(super) fn bar_label_plan(chart_width: f32, count: usize) -> usize {
    if count <= 1 {
        return count;
    }
    let band_w = chart_width / count as f32;
    let need = BAR_LABEL_MAX_CHARS as f32 * CHART_CHAR_PX + CHART_LABEL_GAP;
    // Whole bands a drawn label needs beside it; at least one. A zero
    // width saturates `step`, and the division below leaves one label.
    let step = (need / band_w).ceil().max(1.) as usize;
    (count - 1) / step + 1
}

/// Which bars carry a share label: `labels` of the `count` bars, spread
/// evenly from the first to the last — the thinning
/// `BarChart::band_tick_count` applies to band labels (3.15), done here
/// because `BarChart::label` paints whatever text it is handed for
/// every bar.
pub(super) fn bar_label_slots(count: usize, labels: usize) -> Vec<bool> {
    let mut slots = vec![false; count];
    match labels {
        0 => {}
        1 if count > 0 => slots[0] = true,
        n if n >= count => slots.iter_mut().for_each(|s| *s = true),
        n => {
            for k in 0..n {
                let ix = (k as f32 * (count - 1) as f32 / (n - 1) as f32).round() as usize;
                slots[ix] = true;
            }
        }
    }
    slots
}

/// Fill each row's `bar_label` with its share of the spend (3.31): on the
/// bars the plan fits across `plot_width`, nothing on the thinned-out
/// ones. The tooltip carries the exact amount and share for every bar,
/// labelled or not.
fn bar_share_labels(rows: &mut [ChartRow], plot_width: f32) {
    let slots = bar_label_slots(rows.len(), bar_label_plan(plot_width, rows.len()));
    for (row, labeled) in rows.iter_mut().zip(slots) {
        if labeled {
            row.bar_label = bar_share_label(row.share).into();
        }
    }
}

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

const ALL_ACCOUNTS: &str = "All accounts";

/// One label per account for the filter dropdown: the account's own name,
/// or the institution-qualified name when two accounts share it
/// (ui-conventions, *Accounts*).
fn account_labels(accounts: &[Account]) -> Vec<String> {
    accounts
        .iter()
        .map(|a| {
            if accounts.iter().filter(|o| o.name == a.name).count() > 1 {
                a.display_name()
            } else {
                a.name.clone()
            }
        })
        .collect()
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
        self.date_range = range;
        let (from, to) = range.range(Self::today());
        self.view_filter.from = from;
        self.view_filter.to = to;
        let index = DatePreset::ALL
            .iter()
            .position(|p| *p == range.preset())
            .unwrap_or(0);
        self.date_preset_select.update(cx, |select, cx| {
            select.set_selected_index(Some(gpui_kit::component::IndexPath::new(index)), window, cx);
        });
        if let DateRange::Custom { from, to } = range {
            // `set_date` does not emit `Change`, so this never re-persists.
            self.date_from_picker.update(cx, |picker, cx| {
                picker.set_date(CalendarDate::Single(Some(from)), window, cx);
            });
            self.date_to_picker.update(cx, |picker, cx| {
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
        let db = self.db.clone();
        let range = self.date_range;
        self.date_range_task = Some(cx.spawn(async move |_, _| {
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
        let (from, to) = match self.date_range {
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
        self.date_range_task = Some(cx.spawn_in(window, async move |this, cx| {
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
            .filter(|a| !self.excluded_accounts.contains(&a.id))
            .map(|a| a.id)
            .collect()
    }

    /// Mirror the account checkboxes into [`ViewFilter::account_ids`]
    /// (empty = all accounts). With *every* account unchecked the list
    /// collapses to empty, which reads as "all" to the chart queries; the
    /// table path handles that case itself in [`Self::refresh`].
    pub(super) fn sync_account_filter(&mut self) {
        self.view_filter.account_ids = if self.excluded_accounts.is_empty() {
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
        self.excluded_accounts.retain(|x| *x != id);
        if !on {
            self.excluded_accounts.push(id);
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
        if on == self.include_transfers {
            return;
        }
        self.include_transfers = on;
        self.view_filter.include_transfers = on;
        let db = self.db.clone();
        self.include_transfers_task = Some(cx.spawn(async move |_, _| {
            let _ = on_db(&db, move |c| settings::set_include_transfers(c, on)).await;
        }));
        self.refresh(window, cx);
    }

    /// The search box changed: re-run the table query with the new term.
    /// Empty text clears the search.
    pub(super) fn apply_search(&mut self, term: &str, window: &mut Window, cx: &mut Context<Self>) {
        if term == self.search {
            return;
        }
        self.search = term.to_string();
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
        if on == self.uncategorized_only {
            return;
        }
        self.uncategorized_only = on;
        if on && matches!(self.legend_selection, Some(LegendSelection::Category(_))) {
            self.legend_selection = None;
            self.summary_month = None;
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
        if on == self.show_hidden {
            return;
        }
        self.show_hidden = on;
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
        self.tx_sort = sort;
        self.refresh(window, cx);
    }

    /// Click a legend row (3.5). A parent at the top level drills the chart
    /// and legend into its children and filters the table to that parent;
    /// a child filters the table to itself, and clicking the selected child
    /// again clears the filter (the chart stays on the children). The
    /// `Uncategorized` pseudo-row toggles the uncategorized filter.
    /// Selecting a real category clears the "Uncategorized only" checkbox —
    /// the two can't both narrow the table.
    pub(super) fn click_legend_row(
        &mut self,
        id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.legend_selection = match id {
            None => (self.legend_selection != Some(LegendSelection::Uncategorized))
                .then_some(LegendSelection::Uncategorized),
            Some(id) if self.chart_parent.is_none() => {
                self.chart_parent = Some(id);
                Some(LegendSelection::Category(id))
            }
            Some(id) => (self.legend_selection != Some(LegendSelection::Category(id)))
                .then_some(LegendSelection::Category(id)),
        };
        if matches!(self.legend_selection, Some(LegendSelection::Category(_))) {
            self.uncategorized_only = false;
        }
        // The month only ever narrows a category selection (the cell
        // highlight and the chip read both from the pair), so clearing the
        // selection clears it too — Esc does the same for both.
        if self.legend_selection.is_none() {
            self.summary_month = None;
        }
        self.refresh(window, cx);
    }

    /// Click the "← All categories" row: back to the parent level with no
    /// category filter on the table.
    pub(super) fn show_all_categories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chart_parent = None;
        self.legend_selection = None;
        self.summary_month = None;
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
        if self.chart_parent.is_none()
            && self.legend_selection.is_none()
            && self.summary_month.is_none()
        {
            return;
        }
        self.chart_parent = None;
        self.legend_selection = None;
        self.summary_month = None;
        self.refresh(window, cx);
    }

    /// Click a monthly summary cell (3.6): narrow the table to that
    /// category and month; clicking the same cell again clears both, the
    /// legend toggle. The `Total` column is the category with no month.
    /// The category part drives the shared `legend_selection`, and the
    /// chart moves so the selection stays one of the legend's rows —
    /// legend and summary can never disagree about what is selected.
    pub(super) fn click_summary_cell(
        &mut self,
        id: Option<i64>,
        month: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same_category = match id {
            None => matches!(self.legend_selection, Some(LegendSelection::Uncategorized)),
            Some(id) => {
                matches!(self.legend_selection, Some(LegendSelection::Category(c)) if c == id)
            }
        };
        if same_category && self.summary_month == month {
            self.legend_selection = None;
            self.summary_month = None;
        } else {
            self.legend_selection = Some(match id {
                None => LegendSelection::Uncategorized,
                Some(id) => LegendSelection::Category(id),
            });
            self.summary_month = month;
            if id.is_some() {
                // A real category and the uncategorized checkbox can't
                // both narrow the table (the legend's rule).
                self.uncategorized_only = false;
            }
            self.sync_chart_to_selection();
        }
        self.refresh(window, cx);
    }

    /// Move the chart so a selection made outside the legend (a summary
    /// cell) is visible in the legend rows: a parent shows at the top
    /// level, a child only while drilled into its parent. Keeps the
    /// refresh revalidation of `legend_selection` from dropping a
    /// summary-made selection.
    fn sync_chart_to_selection(&mut self) {
        let Some(selection) = self.legend_selection else {
            return;
        };
        match selection {
            LegendSelection::Uncategorized => self.chart_parent = None,
            LegendSelection::Category(id) => {
                if let Some(parent) = self.parent_of(id) {
                    self.chart_parent = Some(parent);
                } else if self.chart_parent != Some(id) {
                    self.chart_parent = None;
                }
            }
        }
    }

    /// The id of `id`'s parent in the category tree, `None` for a
    /// top-level category.
    fn parent_of(&self, id: i64) -> Option<i64> {
        self.categories
            .iter()
            .find(|p| p.children.iter().any(|c| c.id == id))
            .map(|p| p.id)
    }

    /// Click a summary parent's name or chevron: show or hide its child
    /// rows (3.6). Pure view state, so no re-query — just a repaint.
    pub(super) fn toggle_summary_parent(&mut self, id: i64) {
        if !self.expanded_parents.insert(id) {
            self.expanded_parents.remove(&id);
        }
    }

    /// The summary rows prepared for drawing: the grid's rows with their
    /// kinds resolved (children of collapsed parents left out) and the
    /// month values followed by the `Total`. The query's last three rows
    /// are always `Income`, `Spend` and `Net`.
    pub(super) fn summary_rows(&self) -> Vec<SummaryRowView> {
        let n = self.summary.rows.len();
        let mut views = Vec::with_capacity(n);
        for (ix, row) in self.summary.rows.iter().enumerate() {
            let kind = if ix + 3 >= n {
                match n - ix {
                    3 => SummaryRowKind::Income,
                    2 => SummaryRowKind::Spend,
                    _ => SummaryRowKind::Net,
                }
            } else {
                match row.id {
                    None => SummaryRowKind::Uncategorized,
                    Some(id) => match self.parent_of(id) {
                        Some(parent) => {
                            if !self.expanded_parents.contains(&parent) {
                                continue;
                            }
                            SummaryRowKind::Child
                        }
                        None => SummaryRowKind::Parent {
                            expandable: self
                                .categories
                                .iter()
                                .any(|p| p.id == id && !p.children.is_empty()),
                            expanded: self.expanded_parents.contains(&id),
                        },
                    },
                }
            };
            let mut cells = row.months.clone();
            cells.push(row.total);
            views.push(SummaryRowView {
                id: row.id,
                name: row.name.clone().into(),
                kind,
                cells,
            });
        }
        // Uncategorized is always the last category row, whatever its
        // amount (ui-conventions, *Numbers, dates and labels*), like the
        // legend; the three total rows stay after it.
        let categories_end = views.len().saturating_sub(3);
        views[..categories_end].sort_by_key(|v| v.kind == SummaryRowKind::Uncategorized);
        views
    }

    /// Restore the persisted `ui.include_transfers` (default off) at
    /// launch; unlike the date range the table query reads the filter, so
    /// a saved "on" refreshes the rows.
    pub(super) fn load_include_transfers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        self.include_transfers_task = Some(cx.spawn_in(window, async move |this, cx| {
            let saved = on_db(&db, |c| settings::include_transfers(c)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Ok(true) = saved {
                    this.include_transfers = true;
                    this.view_filter.include_transfers = true;
                    this.refresh(window, cx);
                }
            });
        }));
    }

    /// Trigger label of the accounts dropdown: "All accounts" while
    /// nothing is unchecked, otherwise how many of the accounts remain.
    fn account_filter_label(&self) -> String {
        let total = self.accounts.len();
        let selected = total.saturating_sub(self.excluded_accounts.len());
        if self.excluded_accounts.is_empty() {
            ALL_ACCOUNTS.to_string()
        } else if selected == 0 {
            "No accounts".to_string()
        } else {
            format!("{selected} of {total} accounts")
        }
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
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| set_category(c, transaction_id, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        // The row itself starts offering the rule (2.6); no
                        // separate feedback.
                        this.rule_offer = Some((transaction_id, category_id));
                    }
                    Err(e) => this.notify_err(e, cx),
                }
                this.refresh(window, cx);
            });
        }));
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
        // Hiding the row a rule offer is on would strand the offer.
        if self.rule_offer.is_some_and(|(id, _)| id == transaction_id) {
            self.rule_offer = None;
        }
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| set_hidden(c, transaction_id, hidden)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(e) = result {
                    this.notify_err(e, cx);
                }
                this.refresh(window, cx);
            });
        }));
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
        let Some((_, category_id)) = self.rule_offer.filter(|(id, _)| *id == transaction_id) else {
            return;
        };
        self.rule_offer = None;
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
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
        }));
        cx.notify();
    }

    // -- toasts (3.27) ------------------------------------------------------

    /// The dashboard middle row (3.5): the `BarChart` of spend per category
    /// beside a clickable legend, in the left half; the monthly summary
    /// (3.6) takes the right half later. `BarChart` takes no clicks, so all
    /// interactivity lives in the legend rows (ui-conventions, *Interaction*).
    /// The palette color of every category (ui-conventions, *Category
    /// colors*): position among the siblings sorted by category id, so a
    /// category keeps its color when the date range reorders the rows;
    /// children start at the front of the palette under their own parent.
    /// Shared by the chart, its legend and the monthly summary swatches.
    pub(super) fn category_colors(
        &self,
        theme: &gpui_kit::component::theme::Theme,
    ) -> HashMap<i64, Hsla> {
        let palette = category_palette(theme);
        let mut colors = HashMap::new();
        let mut parents: Vec<&CategoryNode> = self.categories.iter().collect();
        parents.sort_by_key(|n| n.id);
        for (ix, parent) in parents.iter().enumerate() {
            colors.insert(parent.id, palette[ix % 12]);
            let mut kids: Vec<&CategoryNode> = parent.children.iter().collect();
            kids.sort_by_key(|n| n.id);
            for (cix, kid) in kids.iter().enumerate() {
                colors.insert(kid.id, palette[cix % 12]);
            }
        }
        colors
    }

    /// The plot's width for the current window: the sidebar and page
    /// padding come off the window width, `CHART_PANEL_SHARE` of what
    /// remains is the chart panel, and inside the panel its padding, the
    /// legend and the gap come off before the plot is left.
    pub(super) fn chart_plot_width(&self) -> f32 {
        let page_w = (self.window_width - SIDEBAR_W - 2. * PAGE_PAD).max(0.);
        (page_w * CHART_PANEL_SHARE - 2. * PAGE_PAD - LEGEND_W - REGION_GAP).max(0.)
    }

    /// The chart's label plan (3.15) for the current window: how many
    /// characters a band label may use and how many labels are drawn.
    pub(super) fn chart_label_plan(&self) -> (usize, usize) {
        band_label_budget(self.chart_plot_width(), self.chart_rows.len())
    }

    /// The chart's rows prepared for drawing (3.15, 3.31): band labels
    /// fitted to the current budget, each row's share over its bar
    /// thinned to `plot_width`, and `Uncategorized` last whatever its
    /// amount. `colors` maps category ids to palette colors; `muted`
    /// paints the `Uncategorized` swatch.
    pub(super) fn chart_bars(
        &self,
        plot_width: f32,
        colors: &HashMap<i64, Hsla>,
        muted: Hsla,
    ) -> Vec<ChartRow> {
        let (label_chars, _) = self.chart_label_plan();
        let shorts = short_chart_labels(
            &self
                .chart_rows
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<String>>(),
            label_chars,
        );
        let mut rows: Vec<ChartRow> = self
            .chart_rows
            .iter()
            .zip(shorts)
            .map(|(r, short)| ChartRow {
                id: r.id,
                name: r.name.clone().into(),
                short,
                bar_label: SharedString::default(),
                cents: r.spend_cents,
                share: r.share,
                color: r
                    .id
                    .map_or(muted, |id| colors.get(&id).copied().unwrap_or(muted)),
            })
            .collect();
        // Uncategorized is always the last category row, whatever its amount.
        rows.sort_by_key(|r| r.id.is_none());
        bar_share_labels(&mut rows, plot_width);
        rows
    }

    fn chart_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (border, background, muted, list_active, radius, colors) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.background,
                theme.muted_foreground,
                theme.list_active,
                theme.radius,
                self.category_colors(theme),
            )
        };
        // Band label thinning for the axis (3.15); the rows themselves —
        // fitted names and share labels (3.15, 3.31) — come from the same
        // preparation the tests exercise.
        let label_count = self.chart_label_plan().1;
        let rows = self.chart_bars(self.chart_plot_width(), &colors, muted);
        let selection = self.legend_selection;
        // The `Uncategorized` row is active when the 3.4 checkbox drives the
        // same clause, so the legend never contradicts the toolbar.
        let uncategorized_active =
            matches!(selection, Some(LegendSelection::Uncategorized)) || self.uncategorized_only;
        // The parent suffix of the tooltip text (" of Food" once drilled
        // in); empty at the top level.
        let parent_name = self
            .chart_parent
            .and_then(|p| self.categories.iter().find(|n| n.id == p))
            .map(|n| format!(" of {}", n.name))
            .unwrap_or_default();
        let bar_parent_name = parent_name.clone();

        let chart = if rows.is_empty() {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child("No spending in this range")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .debug_selector(|| "chart:plot".into())
                .child(
                    BarChart::new(rows.clone())
                        .id("category-chart")
                        // The band text doubles as the axis label (3.15):
                        // the fitted short name, drawn on only
                        // `label_count` evenly spread bands so labels
                        // never overlap each other or the bars.
                        .band(|r| r.short.clone())
                        .band_tick_count(label_count)
                        .value(|r| r.cents as f64)
                        .fill(|r, _, _, _| r.color)
                        // The share above each bar (3.31): empty text on
                        // the bars the plan thinned out.
                        .label(|r| r.bar_label.clone())
                        .label_color(move |_| muted)
                        .tooltip_title(|r| r.name.clone())
                        .tooltip_value(move |r, _| {
                            chart_tooltip_value(r.cents, r.share, &bar_parent_name).into()
                        }),
                )
                .into_any_element()
        };

        let legend = v_flex()
            .id("legend-list")
            .debug_selector(|| "chart:legend".into())
            .w(px(LEGEND_W))
            .flex_none()
            .min_h_0()
            .gap_1()
            .overflow_y_scroll()
            .when(self.chart_parent.is_some(), |list| {
                list.child(
                    div()
                        .id("legend-all")
                        .text_sm()
                        .px_1()
                        .py_1()
                        .cursor_pointer()
                        .child("← All categories")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_all_categories(window, cx);
                        })),
                )
            })
            .children(rows.iter().enumerate().map(|(ix, r)| {
                let id = r.id;
                let selected = match (id, selection) {
                    (None, _) => uncategorized_active,
                    (Some(id), Some(LegendSelection::Category(c))) => c == id,
                    _ => false,
                };
                // The row is just the key (3.31): swatch and name. What
                // the amount and share columns used to carry moves into
                // the hover tooltip, in the bar's own tooltip text.
                let value = chart_tooltip_value(r.cents, r.share, &parent_name);
                div()
                    .id(SharedString::from(format!("legend-{ix}")))
                    .debug_selector(move || format!("chart:legend-row:{ix}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .py_1()
                    .rounded(radius)
                    .cursor_pointer()
                    .when(selected, |row| row.bg(list_active))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.click_legend_row(id, window, cx);
                    }))
                    .child(div().size(px(10.)).flex_none().rounded(px(2.)).bg(r.color))
                    .child(
                        div()
                            .text_sm()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .debug_selector({
                                let name = r.name.clone();
                                move || format!("chart:legend-text:{ix}={name}")
                            })
                            .when(id.is_none(), |name| name.text_color(muted))
                            .child(r.name.clone()),
                    )
                    .tooltip({
                        let (name, value) = (r.name.clone(), value);
                        move |window, cx| {
                            let (name, value) = (name.clone(), value.clone());
                            Tooltip::element(move |_, _| {
                                let sel = format!("chart:legend-tooltip:{name}={value}");
                                v_flex()
                                    .gap_0p5()
                                    .debug_selector(move || sel)
                                    .child(
                                        div().font_weight(FontWeight::SEMIBOLD).child(name.clone()),
                                    )
                                    .child(value.clone())
                            })
                            .build(window, cx)
                        }
                    })
            }));

        h_flex()
            .w_full()
            .min_w_0()
            .h(px(340.))
            .flex_none()
            .gap_3()
            .child(
                // 3.16: the panel names itself, as the mockups show. The
                // height is explicit (3.25): the row's cross alignment does
                // not stretch, so without it the panel keeps its content
                // height and its title misses the summary's title line.
                v_flex()
                    .w(relative(CHART_PANEL_SHARE))
                    .h_full()
                    .min_w_0()
                    .debug_selector(|| "panel:chart".into())
                    .gap_2()
                    .p_4()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(background)
                    .child(region_title("Spending by category"))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .gap_3()
                            .child(chart)
                            .child(legend),
                    ),
            )
            .child(self.summary_panel(cx))
            .into_any_element()
    }

    /// The monthly summary panel (3.6): the right half of the dashboard's
    /// middle row. Rows are the spend parents — clicking a name or chevron
    /// expands or collapses its children — then `Income`, `Spend` and
    /// `Net`; columns are the months in the range plus the `Total`.
    /// Clicking a category cell filters the table to that category and
    /// month, and clicking the same cell again clears it (ui-conventions,
    /// *Interaction*). Amounts are positive spend; `Net` is the one signed
    /// row, `danger` when negative; `Income` reads as money in (`success`).
    fn summary_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (border, background, muted, list_active, danger, success, radius, colors) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.background,
                theme.muted_foreground,
                theme.list_active,
                theme.danger,
                theme.success,
                theme.radius,
                self.category_colors(theme),
            )
        };
        // `month_grid` fills the range's months even when no transaction
        // falls in it, so a range that excludes all data yields a grid of
        // zeros rather than an empty `months`. Treat an all-zero grid as
        // empty too (task 3.10): the panel says "No spending in this range".
        if !self.summary_has_data() {
            return v_flex()
                .h(px(340.))
                .flex_1()
                .min_w_0()
                .debug_selector(|| "panel:summary".into())
                .gap_2()
                .p_4()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(background)
                .child(region_title("Monthly summary"))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(muted)
                        .child("No spending in this range"),
                )
                .into_any_element();
        }
        let selection = self.legend_selection;
        let selected_month = self.summary_month.clone();
        let month_values = self.summary.months.clone();
        let rows = self.summary_rows();

        // Pinned columns and scrolling months (3.30): the name column and
        // `Total` keep fixed widths at the row edges, and the month columns
        // — 88px each, as before — live in a viewport between them that
        // scrolls horizontally. One `ScrollHandle` is tracked by every
        // month viewport, the header's included, so they always show the
        // same slice of months.
        const NAME_W: f32 = 160.;
        const MONTH_W: f32 = 88.;
        const TOTAL_W: f32 = 96.;
        let month_count = month_values.len();
        let month_viewport = |selector: String, handle: &ScrollHandle| {
            div()
                .id(SharedString::from(selector.clone()))
                .flex_1()
                .min_w_0()
                .overflow_x_scroll()
                // Keep a vertical wheel moving the rows: without this a
                // vertical-only delta on an x-scrollable div is remapped to
                // the horizontal axis instead of reaching `summary-rows`.
                .restrict_scroll_to_axis()
                .track_scroll(handle)
                .debug_selector(move || selector.clone())
        };
        // The strip is flush right against `Total` while every month fits —
        // today's look — and once it overflows it packs to its natural
        // width and scrolls.
        let months_strip = move || {
            div()
                .flex()
                .gap_2()
                .w_full()
                .min_w(px(
                    month_count as f32 * MONTH_W + (month_count as f32 - 1.) * 8.
                ))
                .justify_end()
        };

        let header = h_flex()
            .gap_2()
            .px_1()
            .child(div().w(px(NAME_W)).flex_none())
            .child(
                month_viewport("summary:months-header".into(), &self.summary_scroll).child(
                    months_strip().children(month_values.iter().enumerate().map(
                        |(col_ix, month)| {
                            div()
                                .w(px(MONTH_W))
                                .flex_none()
                                .debug_selector(move || format!("summary:month-label-{col_ix}"))
                                .text_right()
                                .text_sm()
                                .text_color(muted)
                                .child(flow::month_label(month))
                        },
                    )),
                ),
            )
            .child(
                div()
                    .w(px(TOTAL_W))
                    .flex_none()
                    .text_right()
                    .text_sm()
                    .text_color(muted)
                    .child("Total"),
            );

        let body = v_flex()
            .id("summary-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            // Without this a horizontal wheel delta over the month zone is
            // remapped onto this scroller's vertical axis too, so a swipe
            // would scroll the months and jump the rows at once (3.30).
            .restrict_scroll_to_axis()
            .children(rows.into_iter().enumerate().map(|(ix, row)| {
                let id = row.id;
                let is_category = !matches!(
                    row.kind,
                    SummaryRowKind::Income | SummaryRowKind::Spend | SummaryRowKind::Net
                );
                let swatch = match row.kind {
                    SummaryRowKind::Parent { .. } | SummaryRowKind::Child => {
                        Some(id.and_then(|id| colors.get(&id)).copied().unwrap_or(muted))
                    }
                    SummaryRowKind::Uncategorized => Some(muted),
                    _ => None,
                };
                let is_income = matches!(row.kind, SummaryRowKind::Income);

                // Name cell: chevron + swatch + name. On a parent it also
                // toggles the child rows.
                let (chevron, expandable) = match row.kind {
                    SummaryRowKind::Parent {
                        expandable,
                        expanded,
                    } => (
                        if expandable {
                            if expanded { "▾" } else { "▸" }
                        } else {
                            ""
                        },
                        expandable,
                    ),
                    _ => ("", false),
                };
                let name_cell = h_flex()
                    .id(SharedString::from(format!("summary-name-{ix}")))
                    .w(px(NAME_W))
                    .flex_none()
                    .debug_selector(move || format!("summary:name-{ix}"))
                    .gap_2()
                    .items_center()
                    .when(matches!(row.kind, SummaryRowKind::Child), |c| c.pl(px(14.)))
                    .when(expandable, |c| {
                        c.cursor_pointer().on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                if let Some(id) = id {
                                    this.toggle_summary_parent(id);
                                    cx.notify();
                                }
                            },
                        ))
                    })
                    .child(
                        div()
                            .w(px(12.))
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(chevron),
                    )
                    .when_some(swatch, |c, color| {
                        c.child(div().size(px(10.)).flex_none().rounded(px(2.)).bg(color))
                    })
                    // The fixed name column truncates long names, so hover
                    // shows the full one (3.30), same tooltip idiom as the
                    // queue's merchants.
                    .tooltip({
                        let full = row.name.clone();
                        move |window, cx| {
                            let tip = full.clone();
                            Tooltip::element(move |_, _| {
                                div()
                                    .debug_selector({
                                        let n = tip.clone();
                                        move || format!("summary:name-tooltip:{n}")
                                    })
                                    .child(tip.clone())
                            })
                            .build(window, cx)
                        }
                    })
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_sm()
                            .when(matches!(row.kind, SummaryRowKind::Parent { .. }), |d| {
                                d.font_weight(FontWeight::SEMIBOLD)
                            })
                            .when(matches!(row.kind, SummaryRowKind::Uncategorized), |d| {
                                d.text_color(muted)
                            })
                            .child(row.name.clone()),
                    );

                // The month cells go into the scrolling viewport; the
                // `Total` cell stays pinned right. Both keep their exact
                // behaviour — click-to-filter, the selected highlight, the
                // `Income`/`Net` colours (3.6).
                let mut month_cells: Vec<AnyElement> = Vec::with_capacity(month_count);
                let mut total_cell: AnyElement =
                    div().w(px(TOTAL_W)).flex_none().into_any_element();
                for (col_ix, &cents) in row.cells.iter().enumerate() {
                    let is_total = col_ix >= month_count;
                    let cell_month = month_values.get(col_ix).cloned();
                    let selected = is_category
                        && cell_month == selected_month
                        && match (id, selection) {
                            (None, Some(LegendSelection::Uncategorized)) => true,
                            (Some(id), Some(LegendSelection::Category(c))) => c == id,
                            _ => false,
                        };
                    let color = match row.kind {
                        SummaryRowKind::Income => Some(success),
                        SummaryRowKind::Net if cents < 0 => Some(danger),
                        _ => None,
                    };
                    let cell = div()
                        .id(SharedString::from(format!("summary-cell-{ix}-{col_ix}")))
                        .w(if is_total { px(TOTAL_W) } else { px(MONTH_W) })
                        .flex_none()
                        .debug_selector(move || format!("summary:cell-{ix}-{col_ix}"))
                        .text_right()
                        .text_sm()
                        .rounded(px(2.))
                        .when_some(color, |c, color| c.text_color(color))
                        .when(selected, |c| c.bg(list_active))
                        .when(is_category, |c| {
                            let month = cell_month.clone();
                            c.cursor_pointer().on_click(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    this.click_summary_cell(id, month.clone(), window, cx);
                                },
                            ))
                        })
                        .child(flow::money(cents))
                        .into_any_element();
                    if is_total {
                        total_cell = cell;
                    } else {
                        month_cells.push(cell);
                    }
                }

                h_flex()
                    .id(SharedString::from(format!("summary-row-{ix}")))
                    .gap_2()
                    .items_center()
                    .px_1()
                    .py_1()
                    .rounded(radius)
                    // A hairline above `Income` separates the totals from
                    // the category rows.
                    .when(is_income, |r| r.border_t_1().border_color(border))
                    .child(name_cell)
                    .child(
                        month_viewport(format!("summary:months-{ix}"), &self.summary_scroll)
                            .child(months_strip().children(month_cells)),
                    )
                    .child(total_cell)
                    .into_any_element()
            }));

        v_flex()
            .h(px(340.))
            .flex_1()
            .min_w_0()
            .relative()
            .debug_selector(|| "panel:summary".into())
            .p_4()
            .gap_2()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(background)
            .child(region_title("Monthly summary"))
            // Month header and rows keep their tighter `gap_1`.
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .child(header)
                    .child(body),
            )
            // The month strip's horizontal scrollbar (3.30): a thin overlay
            // pinned to the panel's bottom edge, inset to span only the
            // month zone — the name column and `Total` stay clear of it.
            // `Always` so it shows while the strip overflows (the theme
            // default hides it until hover); it disappears by itself when
            // every month fits.
            .child(
                div()
                    .id("summary-hscrollbar")
                    .absolute()
                    .left(px(20. + NAME_W + 8.))
                    .right(px(20. + TOTAL_W + 8.))
                    .bottom(px(4.))
                    .h(px(12.))
                    .child(
                        Scrollbar::horizontal(&self.summary_scroll)
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
            )
            .into_any_element()
    }

    /// Which dashboard empty state applies (task 3.10, ui-conventions
    /// *States*). `total_visible` counts every non-hidden row ignoring all
    /// filters, so it is the "empty database" signal; `row_count` is the
    /// filtered table, so `0` with data present means the filters excluded
    /// everything.
    pub(super) fn dashboard_body(&self) -> DashboardBody {
        if self.total_visible == 0 {
            DashboardBody::NoData
        } else if self.row_count == 0 {
            DashboardBody::NoMatches
        } else {
            DashboardBody::Rows
        }
    }

    /// Whether the monthly summary has anything to draw. `month_grid` pads
    /// the range with zero months, so an empty range is an all-zero grid
    /// rather than an empty `months` (task 3.10).
    pub(super) fn summary_has_data(&self) -> bool {
        !self.summary.months.is_empty()
            && self
                .summary
                .rows
                .iter()
                .any(|r| r.months.iter().any(|v| *v != 0))
    }

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
                                .child(Select::new(&self.date_preset_select)),
                        )
                        .flex_none(),
                    )
                    // The From/To pickers want 140 px but shrink first
                    // (floor 110 px) when the row runs out of room, so the
                    // custom range fits the minimum window (3.25, 3.35).
                    .when(matches!(self.date_range, DateRange::Custom { .. }), |g| {
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
                    })
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
                DashboardBody::NoData => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop an Amex or Simplii CSV export here, or press ⌘O")
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
                                    DataTable::new(&self.table).stripe(true).into_any_element()
                                }),
                        )
                        .into_any_element()
                }
            })
            .into_any_element()
    }

    /// The table's empty state (task 3.10, ui-conventions *States*): shown
    /// when transactions exist but the current filters select none. A
    /// muted centered message plus a "Clear filters" link that drops every
    /// table-narrowing filter. The header controls (date range, accounts,
    /// transfers) keep their own widgets, so they stay as the user set them.
    fn no_matches_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let (border, background, muted, radius) = {
            let theme = cx.theme();
            (
                theme.border,
                theme.background,
                theme.muted_foreground,
                theme.radius,
            )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(background)
            .text_sm()
            .text_color(muted)
            .child("No transactions match these filters")
            .child(
                Button::new("clear-filters")
                    .link()
                    .label("Clear filters")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_filters(window, cx);
                    })),
            )
            .into_any_element()
    }

    /// "Clear filters": drop every filter that narrows the table — search,
    /// "Uncategorized only", and the category/month selection with its
    /// chart drill-down. `set_value` doesn't emit `InputEvent::Change`, so
    /// `self.search` is cleared here too before the single re-query.
    pub(super) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search.clear();
        self.uncategorized_only = false;
        self.chart_parent = None;
        self.legend_selection = None;
        self.summary_month = None;
        self.refresh(window, cx);
    }

    /// Accounts filter dropdown (task 3.3): gpui-kit has no multi-select,
    /// so it is a [`Popover`] of one [`Checkbox`] per account, all checked
    /// by default. The popover stays open while boxes are clicked.
    fn accounts_popover(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        let labels = account_labels(&self.accounts);
        let excluded = self.excluded_accounts.clone();
        Popover::new("accounts-filter")
            .trigger(Button::new("accounts-filter-trigger").label(self.account_filter_label()))
            .content(move |_, _, _| {
                v_flex()
                    .gap_1()
                    .children(ids.iter().zip(labels.iter()).map(|(id, name)| {
                        let id = *id;
                        let shell = shell.clone();
                        checkbox(format!("account-{id}"))
                            .label(name.clone())
                            .checked(!excluded.contains(&id))
                            .on_click(move |on: &bool, window, cx| {
                                shell.update(cx, |this, cx| {
                                    this.set_account_checked(id, *on, window, cx);
                                });
                            })
                    }))
            })
    }

    /// Options popover (3.35): the three filter toggles that used to sit in
    /// the filter rows — same state and handlers. The trigger counts how
    /// many are on. Right-anchored so it opens inside the page.
    fn options_popover(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.entity();
        let (transfers, uncategorized, hidden) = (
            self.include_transfers,
            self.uncategorized_only,
            self.show_hidden,
        );
        let on = [transfers, uncategorized, hidden]
            .iter()
            .filter(|b| **b)
            .count();
        let label = options_label(on);
        Popover::new("filter-options")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("filter-options-trigger")
                    .icon(IconName::ListFilter)
                    // The selector carries the label so tests can pin the
                    // exact words ("Options · 2") the way `label:<text>` does.
                    .debug_selector(|| format!("filters:options-btn:{label}"))
                    .label(label),
            )
            .content(move |_, _, _| {
                let (s1, s2, s3) = (shell.clone(), shell.clone(), shell.clone());
                v_flex()
                    .gap_2()
                    .child(
                        checkbox("include-transfers")
                            .label("Include transfers")
                            .checked(transfers)
                            .on_click(move |on: &bool, window, cx| {
                                s1.update(cx, |this, cx| {
                                    this.set_include_transfers(*on, window, cx);
                                });
                            }),
                    )
                    .child(
                        checkbox("uncategorized-only")
                            .label("Uncategorized only")
                            .checked(uncategorized)
                            .on_click(move |on: &bool, window, cx| {
                                s2.update(cx, |this, cx| {
                                    this.set_uncategorized_only(*on, window, cx);
                                });
                            }),
                    )
                    .child(
                        checkbox("show-hidden")
                            .label("Show hidden")
                            .checked(hidden)
                            .on_click(move |on: &bool, window, cx| {
                                s3.update(cx, |this, cx| {
                                    this.set_show_hidden(*on, window, cx);
                                });
                            }),
                    )
            })
    }
}
