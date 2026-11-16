//! The transaction table delegate and its row menu (2.9, 3.29, 4.2).

use super::*;

/// Callbacks from a transaction row's context menu back to the [`Shell`]
/// (2.9): set a category, hide/unhide, and apply the post-edit rule offer.
type SetCategoryFn = Rc<dyn Fn(i64, i64, &mut Window, &mut App)>;
type SetHiddenFn = Rc<dyn Fn(i64, bool, &mut Window, &mut App)>;
type ApplyRuleFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;
pub(crate) type UnpairFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;
/// Called with the deciding rule's id from the *Show rule* menu item (3.29).
pub(crate) type ShowRuleFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// The painted text of one transaction row, built when the rows are set
/// (3.85): the *Date*, *Account*, *Description*, *Category* and *Amount*
/// cells plus the parsed merchant, so painting a cell never formats or
/// parses anything — it only clones a [`SharedString`].
pub(in crate::ui) struct TxRowCells {
    text: [SharedString; 5],
    /// `financi_core::parse::merchant` over the description, for the
    /// post-edit rule offer (2.6), which asks for it on every repaint of
    /// the row's *Category* cell.
    merchant: SharedString,
}

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
/// locked rows); an unlocked row asks [`CompiledRules::winning`] — the same
/// matcher import runs — so no rule id is ever stored on the transaction.
/// 3.85: the rules arrive pre-compiled (the Catalog's cache).
pub fn row_show_rule(
    row: &TransactionRow,
    rules: &CompiledRules,
    card_payment_category: Option<i64>,
) -> RowShowRule {
    if row.category_locked {
        if row.category_id.is_some() && row.category_id == card_payment_category {
            RowShowRule::CardPayment
        } else {
            RowShowRule::SetByHand
        }
    } else if let Some(id) = rules.winning(&row.description) {
        RowShowRule::Found(id)
    } else {
        RowShowRule::NoRule
    }
}

pub struct TxTable {
    pub(in crate::ui) rows: Vec<TransactionRow>,
    /// One entry per row of `rows`, built by [`TxTable::set_rows`]: the
    /// formatted cell text and parsed merchant, so `render_td` never
    /// works per frame (3.85).
    pub(in crate::ui) cells: Vec<TxRowCells>,
    /// Counts `set_rows` calls, for the counter test; proves the cells are
    /// built once per row load, not once per repaint (3.85).
    #[cfg(test)]
    pub(in crate::ui) cells_built: std::cell::Cell<usize>,
    /// Child categories as `(id, "Parent › Child")` for the *Change category…*
    /// submenu, mirroring the shell's `child_choices`.
    pub(in crate::ui) child_choices: Vec<(i64, String)>,
    /// The row that just got a manual category and now offers to apply it to
    /// every matching row (2.6's rule offer, 2.9). Its transaction id.
    pub(in crate::ui) rule_offer: Option<i64>,
    /// The shell's rules, compiled once per rules reload (3.85, the
    /// Catalog's cache), kept in sync on every refresh: the *Show rule*
    /// item (3.29) runs [`CompiledRules::winning`] over them when the
    /// menu opens.
    pub(in crate::ui) rules: Rc<CompiledRules>,
    /// The id of the system `Transfers > Card Payment` category (3.29), the
    /// same lookup `financi-db` uses; `None` only before the first refresh.
    pub(in crate::ui) card_payment_category: Option<i64>,
    /// The table's current ordering, kept in sync with the shell so the
    /// header's sort icon reflects it after every refresh (3.4).
    pub(in crate::ui) sort: TxSort,
    pub(in crate::ui) on_set_category: SetCategoryFn,
    pub(in crate::ui) on_set_hidden: SetHiddenFn,
    pub(in crate::ui) on_apply_rule: ApplyRuleFn,
    /// Called with a paired row's id from the *Unpair* menu item (4.2).
    pub(in crate::ui) on_unpair: UnpairFn,
    /// Called with the deciding rule's id from the *Show rule* menu item (3.29).
    pub(in crate::ui) on_show_rule: ShowRuleFn,
    /// Called with the new ordering when a header's sort icon is clicked;
    /// the shell re-queries and refreshes.
    pub(in crate::ui) on_sort: SortFn,
    /// Widths of the three flexible columns (Account, Description,
    /// Category), kept in sync with the window (3.25, `tx_flex_widths`).
    pub(in crate::ui) flex_widths: (f32, f32, f32),
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
            let merchant = self.cells[row_ix].merchant.clone();
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
    /// Replace the rows and build their display cells in one step, so the
    /// cache can never lag behind `rows` (3.85). Every write of `rows`
    /// goes through here — including the 3.84 optimistic row edits, hides
    /// and their rollbacks, which take the vec, change it and hand it
    /// back, so a repaint always draws the model.
    pub(in crate::ui) fn set_rows(&mut self, rows: Vec<TransactionRow>) {
        self.cells = rows
            .iter()
            .map(|r| TxRowCells {
                text: [
                    r.posted_date.clone().into(),
                    r.account.clone().into(),
                    r.description.trim().into(),
                    r.category
                        .clone()
                        .unwrap_or_else(|| "Uncategorized".into())
                        .into(),
                    flow::money(r.amount_cents).into(),
                ],
                merchant: financi_core::parse::merchant(&r.description).into(),
            })
            .collect();
        self.rows = rows;
        #[cfg(test)]
        self.cells_built.set(self.cells_built.get() + 1);
    }

    pub(in crate::ui) fn text(&self, row_ix: usize, col_ix: usize) -> SharedString {
        self.cells[row_ix].text[col_ix].clone()
    }
}
