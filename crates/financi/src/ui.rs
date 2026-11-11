//! GPUI views. Thin by design: logic lives in `flow` and `financi-db`, and
//! every database call goes through the [`DbWorker`] thread.
//!
//! The window is a [`Sidebar`] of pages and one page area, per
//! `docs/ui-conventions.md` (*Window layout*).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use chrono::NaiveDate;
use financi_core::CategoryKind;
use financi_core::dates::{DatePreset, preset_range};
use financi_core::rules::Rule;
use financi_db::categories::{
    CategoryNode, create_child, create_parent, delete_category, list_categories, move_child,
    rename as rename_category, transaction_counts,
};
use financi_db::import::{
    CommitOptions, ImportEntry, TransactionRow, commit_import, list_imports, plan_import,
    undo_last_import,
};
use financi_db::queue::{UncategorizedGroup, categorize_group, uncategorized_by_merchant};
use financi_db::rules::{add_rule, count_rule_matches, delete_rule, list_rules, update_rule};
use financi_db::settings::{self, DateRange};
use financi_db::{
    Account, CategorySpend, DbWorker, MonthlySummary, TransferReview, TxSort, TxSortColumn,
    TxTableFilter, ViewFilter, apply_rule_from_merchant, create_account, list_accounts,
    list_transactions_filtered, list_transfer_reviews, monthly_summary, resolve_transfer_review,
    rusqlite::Connection, set_category, set_hidden, sidebar_counts, spend_by_category,
    unpair_transaction,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::chart::BarChart;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::sidebar::{Sidebar, SidebarItem, SidebarMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Disableable as _, VirtualListScrollHandle, h_flex, v_flex,
    v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::flow::{self, LoadedFile, PlanSummary};

type Names = Vec<SharedString>;

/// Every checkbox in the app is built through here (3.14) so they are all
/// constructed alike: the check mark itself arrives with the icon assets
/// `crate::icon_assets` registers on the application, and the debug
/// selector (`checkbox:<id>`) lets headless tests find a box and click it.
/// The selector records itself only in debug builds with `test-support`.
fn checkbox(id: impl Into<SharedString>) -> Checkbox {
    let id = id.into();
    let selector = format!("checkbox:{id}");
    Checkbox::new(id).debug_selector(move || selector.clone())
}

/// Field label for a control (3.16, ui-conventions *Numbers, dates and
/// labels*): every input, dropdown and date picker carries a visible label
/// — never a placeholder alone. `text_xs` `muted_foreground` (secondary
/// text), placed above or beside the control. The debug selector
/// (`label:<text>`) lets headless tests assert the label is painted.
fn field_label(muted: Hsla, text: &'static str) -> Div {
    div()
        .text_xs()
        .text_color(muted)
        .debug_selector(move || format!("label:{text}"))
        .child(text)
}

/// Region title (ui-conventions *Typography*: `text_sm`, `font_semibold`).
/// Names a group of related controls or a data panel (3.16: the filter bar,
/// the chart, the monthly summary, the category tree). Same debug selector
/// as [`field_label`].
fn region_title(text: &'static str) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .debug_selector(move || format!("label:{text}"))
        .child(text)
}

/// One column of a dashboard filter row (3.25, ui-conventions *Pages*): a
/// label line over a control line [`FILTER_CONTROL_H`] tall with the control
/// vertically centred, so every control in the row shares one centre line
/// however tall it is. A column whose label is `""` (checkboxes, the
/// transaction count) carries an invisible label line of the same height so
/// its control still sits on the row's control line.
fn filter_column(muted: Hsla, label: &'static str, control: impl IntoElement) -> Div {
    v_flex()
        .gap_1()
        .flex_none()
        .child(if label.is_empty() {
            div().text_xs().invisible().child(".").into_any_element()
        } else {
            field_label(muted, label).into_any_element()
        })
        .child(h_flex().h(FILTER_CONTROL_H).items_center().child(control))
}

/// Run `f` on the database thread, flattening worker and query errors.
async fn on_db<R, E, F>(db: &DbWorker, f: F) -> Result<R, String>
where
    R: Send + 'static,
    E: std::fmt::Display + Send + 'static,
    F: FnOnce(&mut Connection) -> Result<R, E> + Send + 'static,
{
    match db.call(move |c| f(c).map_err(|e| e.to_string())).await {
        Ok(result) => result,
        Err(gone) => Err(gone.to_string()),
    }
}

fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Shorten an ISO timestamp for a history row: `2026-11-08T14:23:05` ->
/// `2026-11-08 14:23`. Anything unparseable comes back unchanged.
fn history_time(iso: &str) -> String {
    match iso.split_once('T') {
        Some((date, time)) if time.len() >= 5 => format!("{date} {}", &time[..5]),
        _ => iso.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Navigation

/// The pages of the sidebar. Launch opens Dashboard; the selection is not
/// persisted (ui-conventions, *Sidebar*).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Dashboard,
    ToCategorize,
    Categories,
    Transfers,
    Imports,
}

impl Page {
    /// Element id of the page content area (3.11): unique per page, so a
    /// switch lands on a fresh element and GPUI cannot reuse the previous
    /// page's painted subtree or its element state under the new page.
    fn content_id(self) -> &'static str {
        match self {
            Page::Dashboard => "page-dashboard",
            Page::ToCategorize => "page-queue",
            Page::Categories => "page-categories",
            Page::Transfers => "page-transfers",
            Page::Imports => "page-imports",
        }
    }
}

/// The left inset (from the sidebar's content edge) shared by the nav items
/// and the section labels (3.13). The gpui-kit nav row brings its own 8px
/// left padding, which sat the items right of the single column the app
/// name, the section labels and the footer share
/// (`docs/ui-conventions.md`, *Sidebar*); both now use this one inset.
/// A collapsed row keeps the component's own padding (see
/// [`NavItem::collapsed`]), which centers the icon either way.
pub(crate) const NAV_ROW_INSET: f32 = 0.;

/// A sidebar entry: a clickable item or a section label (the gpui-kit
/// [`Sidebar`] takes one item type, and the layout mixes items with the
/// *Organize* / *Data* section headers).
#[derive(Clone)]
enum NavItem {
    Item(Box<SidebarMenuItem>),
    Section(SharedString),
}

impl Collapsible for NavItem {
    fn collapsed(self, collapsed: bool) -> Self {
        match self {
            // The expanded row pins its left inset (3.13); a collapsed row
            // goes back to the component's own padding so the icon centers
            // in the rail exactly as the component lays it out.
            Self::Item(item) => {
                let item = item.collapsed(collapsed);
                let item = if collapsed {
                    item.pl_2()
                } else {
                    item.pl(px(NAV_ROW_INSET))
                };
                Self::Item(Box::new(item))
            }
            section => section,
        }
    }

    fn is_collapsed(&self) -> bool {
        match self {
            Self::Item(item) => item.is_collapsed(),
            Self::Section(_) => false,
        }
    }
}

impl SidebarItem for NavItem {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        match self {
            Self::Item(item) => item.render(id, window, cx).into_any_element(),
            Self::Section(label) => h_flex()
                .flex_shrink_0()
                .h_8()
                .items_center()
                .px(px(NAV_ROW_INSET))
                .text_xs()
                .text_color(cx.theme().sidebar_foreground.opacity(0.7))
                .child(label.to_uppercase())
                .into_any_element(),
        }
    }
}

// ---------------------------------------------------------------------------
// Transaction table

/// Callbacks from a transaction row's context menu back to the [`Shell`]
/// (2.9): set a category, hide/unhide, and apply the post-edit rule offer.
type SetCategoryFn = Rc<dyn Fn(i64, i64, &mut Window, &mut App)>;
type SetHiddenFn = Rc<dyn Fn(i64, bool, &mut Window, &mut App)>;
type ApplyRuleFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;
type UnpairFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

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

pub struct TxTable {
    rows: Vec<TransactionRow>,
    /// Child categories as `(id, "Parent › Child")` for the *Change category…*
    /// submenu, mirroring the shell's `child_choices`.
    child_choices: Vec<(i64, String)>,
    /// The row that just got a manual category and now offers to apply it to
    /// every matching row (2.6's rule offer, 2.9). Its transaction id.
    rule_offer: Option<i64>,
    /// The table's current ordering, kept in sync with the shell so the
    /// header's sort icon reflects it after every refresh (3.4).
    sort: TxSort,
    on_set_category: SetCategoryFn,
    on_set_hidden: SetHiddenFn,
    on_apply_rule: ApplyRuleFn,
    /// Called with a paired row's id from the *Unpair* menu item (4.2).
    on_unpair: UnpairFn,
    /// Called with the new ordering when a header's sort icon is clicked;
    /// the shell re-queries and refreshes.
    on_sort: SortFn,
    /// Widths of the three flexible columns (Account, Description,
    /// Category), kept in sync with the window (3.25, `tx_flex_widths`).
    flex_widths: (f32, f32, f32),
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
        // Date and Amount are sortable (3.4). The active column carries its
        // direction so the header icon matches the ordering; the other shows
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
            1 => Column::new("account", "Account").width(px(self.flex_widths.0)),
            2 => Column::new("description", "Description").width(px(self.flex_widths.1)),
            // Room for the post-edit "Apply to all matching …" affordance.
            3 => Column::new("category", "Category").width(px(self.flex_widths.2)),
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
        // Only Date (0) and Amount (4) sort; the table has already cycled its
        // own icon state, so map its value to a `TxSort` and hand it to the
        // shell, which re-queries. `Default` (the icon's off step) falls back
        // to that column's natural first direction. `cx` coerces to `&mut App`
        // like the menu callbacks; the shell is a different entity, so
        // updating it here is not re-entrant.
        let column = match col_ix {
            0 => TxSortColumn::Date,
            4 => TxSortColumn::Amount,
            _ => return,
        };
        let descending = !matches!(sort, ColumnSort::Ascending);
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
                    menu.item(PopupMenuItem::submenu("Change category…", submenu.clone()))
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
    fn text(&self, row_ix: usize, col_ix: usize) -> String {
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

pub struct ImportHistoryTable {
    rows: Vec<ImportEntry>,
    /// The import whose Undo button is waiting for its second click.
    confirm_id: Option<i64>,
    /// Widths of the two flexible columns (`File`, `Account`), kept in
    /// step with the window by the shell (3.20); they truncate with an
    /// ellipsis rather than push the table past the page.
    flex_widths: (f32, f32),
    on_undo: UndoFn,
}

/// Callback from the history table's Undo button back to the [`Shell`].
type UndoFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

impl ImportHistoryTable {
    fn time(entry: &ImportEntry) -> String {
        history_time(&entry.imported_at)
    }

    fn dates(entry: &ImportEntry) -> String {
        match (&entry.first_date, &entry.last_date) {
            (Some(first), Some(last)) => flow::date_range(first, last),
            _ => "—".into(),
        }
    }
}

impl TableDelegate for ImportHistoryTable {
    fn columns_count(&self, _: &App) -> usize {
        6
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("time", "Imported").width(px(HISTORY_TIME_W)),
            1 => Column::new("file", "File").width(px(self.flex_widths.0)),
            2 => Column::new("account", "Account").width(px(self.flex_widths.1)),
            3 => Column::new("rows", "Rows added")
                .width(px(HISTORY_ROWS_W))
                .text_right(),
            4 => Column::new("dates", "Dates").width(px(HISTORY_DATES_W)),
            // Wide enough for both labels (3.20), so neither clips.
            _ => Column::new("undo", "").width(px(history_undo_col_w())),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let entry = &self.rows[row_ix];
        let text = match col_ix {
            0 => Self::time(entry),
            1 => entry.file_name.clone(),
            2 => entry.account.clone(),
            3 => entry.rows_added.to_string(),
            4 => Self::dates(entry),
            _ => String::new(),
        };
        if col_ix == 5 {
            // Undo only on the newest row (ui-conventions, Pages).
            if row_ix == 0 {
                let id = entry.id;
                let label = if self.confirm_id == Some(id) {
                    "Click again to undo"
                } else {
                    "Undo…"
                };
                let on_undo = self.on_undo.clone();
                return div()
                    .child(
                        Button::new("undo-import")
                            .label(label)
                            .on_click(move |_, window, cx| on_undo(id, window, cx)),
                    )
                    .into_any_element();
            }
        }
        if col_ix == 1 || col_ix == 2 {
            // The flexible columns truncate (3.20): a long file name or
            // account name ends in an ellipsis instead of pushing the
            // table wider than the page.
            return div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(text)
                .into_any_element();
        }
        div().child(text).into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Import preview panel

pub enum ImportEvent {
    Imported { message: String, is_error: bool },
    AccountsChanged,
    Cancelled,
}

enum PlanState {
    NeedAccount,
    Loading,
    Ready(PlanSummary),
    Failed(String),
}

pub struct ImportPanel {
    db: DbWorker,
    csv_root: PathBuf,
    file: Arc<LoadedFile>,
    format: String,
    accounts: Vec<Account>,
    names: Names,
    account_select: Entity<SelectState<Names>>,
    new_name: Entity<InputState>,
    creating_new: bool,
    account_id: Option<i64>,
    plan: PlanState,
    override_overlap: bool,
    /// Near-match review (4.3): pairs the user marked "Same charge", keyed
    /// by `(source_line, existing transaction id)`. Absent means "Different
    /// charge" — the default, so an unreviewed row imports as new.
    same_charges: HashSet<(usize, i64)>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

const NEW_ACCOUNT: &str = "+ New account…";

impl EventEmitter<ImportEvent> for ImportPanel {}

impl ImportPanel {
    /// Opens with no account chosen and loads the account list itself, so
    /// it never depends on the main window having finished loading.
    pub fn new(
        db: DbWorker,
        csv_root: PathBuf,
        file: LoadedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let parser = flow::parser(file.parser_id);
        let accounts = Vec::new();
        let names = Self::names_for(&accounts);
        // Never pre-select: the account is always confirmed by the user.
        let account_select = cx.new(|cx| SelectState::new(names.clone(), None, window, cx));
        let new_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Account name, e.g. Cobalt"));
        let sub = cx.subscribe_in(
            &account_select,
            window,
            |this, _, ev: &SelectEvent<Names>, _, cx| {
                let SelectEvent::Confirm(Some(choice)) = ev else {
                    return;
                };
                this.choose(choice.clone(), cx);
            },
        );
        let load = {
            let db = db.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = on_db(&db, |c| list_accounts(c)).await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(all) => this.set_accounts(all, None, window, cx),
                    Err(e) => this.error = Some(e),
                });
            })
        };
        Self {
            db,
            csv_root,
            format: flow::format_label(parser.as_ref()),
            file: Arc::new(file),
            accounts,
            names,
            account_select,
            new_name,
            creating_new: false,
            account_id: None,
            plan: PlanState::NeedAccount,
            override_overlap: false,
            same_charges: HashSet::new(),
            busy: false,
            error: None,
            task: Some(load),
            _subs: vec![sub],
        }
    }

    /// Replace the eligible accounts, optionally selecting one by id.
    fn set_accounts(
        &mut self,
        all: Vec<Account>,
        select: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.accounts = flow::eligible_accounts(&all, flow::parser(self.file.parser_id).as_ref());
        self.names = Self::names_for(&self.accounts);
        let ix = select.and_then(|id| self.accounts.iter().position(|a| a.id == id));
        let names = self.names.clone();
        self.account_select.update(cx, |s, cx| {
            s.set_items(names, window, cx);
            s.set_selected_index(ix.map(gpui_kit::component::IndexPath::new), window, cx);
        });
        cx.notify();
    }

    fn names_for(accounts: &[Account]) -> Names {
        accounts
            .iter()
            .map(|a| SharedString::from(a.display_name()))
            .chain(std::iter::once(NEW_ACCOUNT.into()))
            .collect()
    }

    fn choose(&mut self, choice: SharedString, cx: &mut Context<Self>) {
        self.error = None;
        self.override_overlap = false;
        if choice == NEW_ACCOUNT {
            self.creating_new = true;
            self.account_id = None;
            self.plan = PlanState::NeedAccount;
        } else if let Some(ix) = self.names.iter().position(|n| *n == choice) {
            self.creating_new = false;
            self.account_id = Some(self.accounts[ix].id);
            self.replan(cx);
        }
        cx.notify();
    }

    fn replan(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = self.account_id else {
            return;
        };
        self.plan = PlanState::Loading;
        let (db, file) = (self.db.clone(), self.file.clone());
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| {
                let parser = flow::parser(file.parser_id);
                plan_import(c, account_id, parser.as_ref(), &file.raw, &file.file_name)
                    .map(|p| PlanSummary::from_plan(&p))
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.plan = match result {
                    Ok(summary) => PlanState::Ready(summary),
                    Err(e) => PlanState::Failed(e),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn create_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("Give the account a name.".into());
            cx.notify();
            return;
        }
        let (institution, kind) =
            flow::new_account_defaults(flow::parser(self.file.parser_id).as_ref());
        let db = self.db.clone();
        self.busy = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                let id = create_account(c, institution, &name, kind)?;
                let all = list_accounts(c)?;
                Ok::<_, financi_db::DbError>((id, all))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((id, all)) => {
                        this.set_accounts(all, Some(id), window, cx);
                        this.creating_new = false;
                        this.account_id = Some(id);
                        cx.emit(ImportEvent::AccountsChanged);
                        this.replan(cx);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = self.account_id else {
            return;
        };
        let (db, file) = (self.db.clone(), self.file.clone());
        let csv_root = self.csv_root.clone();
        let options = CommitOptions {
            allow_other_account_overlap: self.override_overlap,
            same_charges: self.same_charges.iter().copied().collect(),
        };
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let name = file.file_name.clone();
            let result = on_db(&db, move |c| {
                let parser = flow::parser(file.parser_id);
                let out = commit_import(
                    c,
                    account_id,
                    parser.as_ref(),
                    &file.raw,
                    &file.file_name,
                    &now_iso(),
                    options,
                )?;
                let csv_error = if out.import_id.is_some() {
                    let years: Vec<i32> = out.affected_years.iter().copied().collect();
                    financi_db::master_csv::rewrite_years(c, &csv_root, account_id, &years)
                        .err()
                        .map(|e| e.to_string())
                } else {
                    None
                };
                Ok::<_, financi_db::import::ImportError>((out, csv_error))
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((out, csv_error)) => {
                        let mut message = format!("Imported {name}: {} new, ", out.inserted);
                        if out.replaced > 0 {
                            message.push_str(&format!("{} replaced, ", out.replaced));
                        }
                        message.push_str(&format!("{} already imported", out.already_present));
                        let mut is_error = false;
                        if let Some(e) = csv_error {
                            message.push_str(&format!(" (master CSV update failed: {e})"));
                            is_error = true;
                        }
                        cx.emit(ImportEvent::Imported { message, is_error });
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn can_import(&self) -> bool {
        match &self.plan {
            PlanState::Ready(s) => !self.busy && (s.overlap.is_none() || self.override_overlap),
            _ => false,
        }
    }

    /// Choose "Same charge" / "Different charge" for one near-match pair
    /// (4.3). The default (absent) is Different: the row imports as new.
    fn set_same_charge(&mut self, key: (usize, i64), same: bool, cx: &mut Context<Self>) {
        if same {
            self.same_charges.insert(key);
        } else {
            self.same_charges.remove(&key);
        }
        cx.notify();
    }
}

impl Render for ImportPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (danger, muted, foreground) = (theme.danger, theme.muted_foreground, theme.foreground);

        // Second line of the preview: plan state in muted text with the key
        // number in the foreground (ui-conventions, *Pages*).
        let summary: AnyElement = match &self.plan {
            PlanState::NeedAccount => div()
                .text_sm()
                .text_color(muted)
                .child("Choose the account this file belongs to.")
                .into_any_element(),
            PlanState::Loading => div()
                .text_sm()
                .text_color(muted)
                .child("Checking for duplicates…")
                .into_any_element(),
            PlanState::Failed(e) => div()
                .text_sm()
                .text_color(danger)
                .child(e.clone())
                .into_any_element(),
            PlanState::Ready(s) => {
                let (key, rest) = s.preview_parts();
                h_flex()
                    .gap_1()
                    .text_sm()
                    .child(div().text_color(foreground).child(key))
                    .child(div().text_color(muted).child(rest))
                    .into_any_element()
            }
        };

        let overlap: AnyElement = match &self.plan {
            PlanState::Ready(s) if s.overlap.is_some() => {
                let (other, rows) = s.overlap.clone().unwrap();
                v_flex()
                    .gap_2()
                    .child(div().text_sm().text_color(danger).child(format!(
                        "{rows} row(s) in this file already exist in {other}. \
                         Check you picked the right account."
                    )))
                    .child(
                        checkbox("override-overlap")
                            .label("Import anyway")
                            .checked(self.override_overlap)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.override_overlap = *checked;
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        };

        // Near-match review (4.3): each suspected pair on one compact line
        // with a Same/Different choice; the default is Different, so an
        // unreviewed row imports as new.
        let near_matches: AnyElement = match &self.plan {
            PlanState::Ready(s) if !s.near_matches.is_empty() => {
                let rows: Vec<AnyElement> = s
                    .near_matches
                    .iter()
                    .map(|m| {
                        let key = (m.source_line, m.existing_id);
                        let same = self.same_charges.contains(&key);
                        let side = |label: &str, date: &str, cents: i64, desc: &str| {
                            div().text_xs().min_w_0().child(format!(
                                "{label} {} · {} · {}",
                                date,
                                flow::money(cents),
                                financi_core::parse::merchant(desc)
                            ))
                        };
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(side(
                                "new",
                                &m.new_date,
                                m.new_amount_cents,
                                &m.new_description_raw,
                            ))
                            .child(side(
                                "existing",
                                &m.existing_date,
                                m.existing_amount_cents,
                                &m.existing_description_raw,
                            ))
                            .child(div().flex_1())
                            .child(
                                Button::new(SharedString::from(format!(
                                    "near-same-{}-{}",
                                    m.source_line, m.existing_id
                                )))
                                .label("Same charge")
                                .when(same, |b| b.primary())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.set_same_charge(key, true, cx),
                                )),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "near-different-{}-{}",
                                    m.source_line, m.existing_id
                                )))
                                .label("Different charge")
                                .when(!same, |b| b.primary())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.set_same_charge(key, false, cx),
                                )),
                            )
                            .into_any_element()
                    })
                    .collect();
                v_flex()
                    .gap_1()
                    .child(div().text_sm().text_color(muted).child(
                        "Possible changed charges — pick Same charge to replace the existing row:",
                    ))
                    .children(rows)
                    .into_any_element()
            }
            _ => div().into_any_element(),
        };

        // New-account field (3.16): a visible label above the input, not
        // just its placeholder.
        let new_account = self.creating_new.then(|| {
            h_flex()
                .gap_2()
                .items_end()
                .child(
                    v_flex()
                        .gap_1()
                        .child(field_label(muted, "New account name"))
                        .child(div().w(px(280.)).child(Input::new(&self.new_name))),
                )
                .child(
                    Button::new("create-account")
                        .label("Create account")
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.create_account(window, cx)),
                        ),
                )
        });

        let import_label = match &self.plan {
            PlanState::Ready(s) => {
                // Rows marked "Same charge" (4.3) replace instead of add.
                let replaced = s
                    .near_matches
                    .iter()
                    .filter(|m| self.same_charges.contains(&(m.source_line, m.existing_id)))
                    .count();
                format!("Import {} transactions", s.new_rows - replaced)
            }
            _ => "Import".into(),
        };

        // Two lines in a bordered panel: identity + actions, then the plan
        // (ui-conventions, *Pages*).
        v_flex()
            .gap_2()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .min_w_0()
                            .child(self.file.file_name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("Detected format: {}", self.format)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("cancel-import")
                            .label("Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ImportEvent::Cancelled))),
                    )
                    .child(
                        Button::new("do-import")
                            .label(import_label)
                            .primary()
                            .disabled(!self.can_import())
                            .on_click(cx.listener(|this, _, _, cx| this.import(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().text_color(muted).child("Into"))
                    .child(
                        div().w(px(260.)).child(
                            Select::new(&self.account_select).placeholder("Choose an account"),
                        ),
                    )
                    .child(summary),
            )
            .child(overlap)
            .child(near_matches)
            .children(new_account)
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_sm().text_color(danger).child(e)),
            )
    }
}

// ---------------------------------------------------------------------------
// Main window

actions!(dashboard, [ClearCategorySelection]);

// File actions (3.7): the native menu bar's File menu, the ⌘ shortcuts and
// the Imports page header buttons all dispatch these, so no action depends
// on the native menu (which may not appear on Linux).
actions!(
    financi,
    [
        ImportCsv,
        UndoLastImport,
        ExportData,
        RestoreBackup,
        QuitApp
    ]
);

/// The application's native menus (3.7). macOS shows this as the menu bar
/// (its first menu is the app menu); Linux has no native menu bar, which is
/// why every item here also lives on the Imports page header.
fn app_menus() -> Vec<Menu> {
    vec![Menu::new("File").items(vec![
        MenuItem::action("Import CSV…", ImportCsv),
        MenuItem::action("Undo last import", UndoLastImport),
        MenuItem::separator(),
        MenuItem::action("Export…", ExportData),
        MenuItem::action("Restore…", RestoreBackup),
        MenuItem::separator(),
        MenuItem::action("Quit", QuitApp),
    ])]
}

/// The one dashboard category selection (3.5): a legend row — a real
/// category (parent or child) or the `Uncategorized` pseudo-row — that
/// narrows the table. `Uncategorized` rides the table's existing
/// uncategorized-only clause; a category id matches that category and,
/// for a parent, its children.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LegendSelection {
    Category(i64),
    Uncategorized,
}

/// Which of the dashboard's three empty states applies (task 3.10,
/// ui-conventions *States*): the whole-window drop-a-CSV prompt is only for
/// an empty database; with data present, the chart/summary and the table
/// carry their own empty states instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DashboardBody {
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
/// label (3.15), produced by [`short_chart_labels`].
#[derive(Clone)]
struct ChartRow {
    id: Option<i64>,
    name: SharedString,
    short: SharedString,
    cents: i64,
    share: f64,
    color: Hsla,
}

// -- Responsive chart labels (3.15) -------------------------------------
//
// `BarChart` paints each band label centred under its band at gpui-kit's
// 10px `TEXT_SIZE` and offers no truncation, so the app prepares the
// text: names are ellipsized to a character budget, and when even short
// labels cannot share a band they are thinned out with
// `BarChart::band_tick_count`. The budgets come from the window's live
// width (tracked through `observe_window_bounds`), so labels grow as the
// window grows and never overlap or spill past the panel at the minimum
// size (1200 × 720) or above.
//
// Layout widths shared by the dashboard and the budget maths: the fixed
// sidebar, the legend column inside the chart panel, the panel's share
// of the dashboard's middle row, the 16px padding of pages and panels
// (`p_4`) and the 12px gap between regions (`gap_3`).
const SIDEBAR_W: f32 = 210.;
const LEGEND_W: f32 = 220.;
const CHART_PANEL_SHARE: f32 = 0.45;
const PAGE_PAD: f32 = 16.;
const REGION_GAP: f32 = 12.;
// Filter rows (3.25): the control line every filter column shares, and the
// width of the first column of each row (the row-2 search box matches it).
const FILTER_CONTROL_H: Pixels = px(32.);
const FILTER_COL_W: Pixels = px(240.);
/// The narrowest the custom range's From/To columns shrink (3.25): row 1
/// gives them 140 px when there is room, and only down to this when the
/// window is at its minimum and the count must still fit on the page.
const FILTER_DATE_MIN_W: Pixels = px(110.);

/// Generous width of one 10px system-font character, rounded up so the
/// budget errs towards labels that are too short rather than overlapping.
const CHART_CHAR_PX: f32 = 6.5;
/// Minimum horizontal gap between two neighbouring drawn band labels.
const CHART_LABEL_GAP: f32 = 8.;
/// Cap on a band label, so a wide chart never paints a sentence.
const CHART_LABEL_MAX_CHARS: usize = 16;
/// Smallest budget that makes a label worth drawing; below it labels are
/// thinned out instead of reduced to unreadable slivers.
const CHART_LABEL_MIN_CHARS: usize = 4;

/// The chart's label plan for `chart_width` pixels across `count` bands:
/// band labels are truncated to `chars` characters (with an ellipsis)
/// and only `labels` of them are drawn, spread evenly first-to-last by
/// `BarChart::band_tick_count`. A centred label cannot collide with its
/// neighbours once every drawn label owns `chart_width / labels` pixels,
/// so that slice always buys `chars · CHART_CHAR_PX + CHART_LABEL_GAP`.
fn band_label_budget(chart_width: f32, count: usize) -> (usize, usize) {
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

// Imports history table widths (3.20). The old fixed columns summed to
// 1,110 px, wider than the page beside the sidebar at the minimum window
// size, which forced the table to scroll sideways. Now `Imported`,
// `Rows added` and `Dates` keep their widths, `Undo` is sized so both
// button labels ("Undo…" and "Click again to undo") show in full, and
// `File` and `Account` share whatever width is left — they truncate
// with an ellipsis as the window narrows and grow with it, so the table
// fits the page from the minimum window size up.
const HISTORY_TIME_W: f32 = 150.;
const HISTORY_ROWS_W: f32 = 110.;
const HISTORY_DATES_W: f32 = 220.;
/// The To-categorize columns (3.22): heading row and row bodies share these
/// so each heading sits over its column, and Merchant — the one flex column
/// — absorbs a narrow window instead of pushing the picker and the
/// Categorize button past the app's minimum width.
const QUEUE_TXNS_W: f32 = 96.;
const QUEUE_TOTAL_W: f32 = 110.;
const QUEUE_DATES_W: f32 = 190.;
const QUEUE_PICKER_W: f32 = 220.;
/// Floor for one flexible column: below this a File or Account cell is
/// unreadable anyway. The app's minimum window never reaches it.
const HISTORY_FLEX_MIN_W: f32 = 80.;
/// A generous width of one 14 px (`text_sm`) system-font character, the
/// same rounding-up spirit as `CHART_CHAR_PX` (which covers the chart's
/// 10 px font, scaled by 14/10): the budget errs towards columns that
/// are too wide rather than labels that clip.
const TEXT_SM_CHAR_PX: f32 = CHART_CHAR_PX * 14. / 10.;
/// The Undo button's widest label; the column must fit this without
/// clipping whatever the window size.
const UNDO_LABEL_MAX: &str = "Click again to undo";
/// Horizontal padding inside a medium `Button`: `px_2p5` per side.
const BUTTON_PAD_X: f32 = 20.;
/// Horizontal padding inside a table cell (8 px per side); gpui-kit
/// draws the cell inside the column width, so it comes off the space
/// the label has.
const CELL_PAD_X: f32 = 16.;
/// The table's 1 px border on each side; the page width includes it.
const TABLE_BORDER_X: f32 = 2.;

/// The Undo column's width: the longest label at `TEXT_SM_CHAR_PX`,
/// plus the button's and the cell's horizontal padding.
fn history_undo_col_w() -> f32 {
    (UNDO_LABEL_MAX.chars().count() as f32 * TEXT_SM_CHAR_PX + BUTTON_PAD_X + CELL_PAD_X).ceil()
}

/// The history table's `File` and `Account` widths for `page_w` pixels
/// of page (window width minus sidebar and page padding): everything
/// the fixed columns leave over, split evenly, never below
/// `HISTORY_FLEX_MIN_W` each.
fn history_flex_widths(page_w: f32) -> (f32, f32) {
    let fixed =
        HISTORY_TIME_W + HISTORY_ROWS_W + HISTORY_DATES_W + history_undo_col_w() + TABLE_BORDER_X;
    let flex = (page_w - fixed).max(2. * HISTORY_FLEX_MIN_W);
    (flex / 2., flex / 2.)
}

/// The dashboard table's fixed columns (3.25): the `Date` and `Amount`
/// widths, which never change with the window.
const TX_FIXED_W: f32 = 110. + 120.;
/// The dashboard table's `Account`, `Description` and `Category` widths for
/// `page_w` pixels of page (3.25, the same shape as `history_flex_widths`):
/// what the fixed columns leave over, split in the widths' original
/// proportions, with the flexed total floored at three times
/// `HISTORY_FLEX_MIN_W`. The sum always reaches the page's right edge, so
/// the table never overhangs it.
fn tx_flex_widths(page_w: f32) -> (f32, f32, f32) {
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
fn short_chart_labels(names: &[String], chars: usize) -> Vec<SharedString> {
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

/// What a row of the monthly summary grid (3.6) is, resolved against the
/// category tree: a spend parent (with its expand/collapse state), one of
/// its children, the `Uncategorized` pseudo-row, or one of the three
/// fixed total rows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SummaryRowKind {
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
struct SummaryRowView {
    id: Option<i64>,
    name: SharedString,
    kind: SummaryRowKind,
    cells: Vec<i64>,
}

/// The category palette (ui-conventions, *Category colors*): the six base
/// colors then their light variants, so red and green — which also mean
/// danger and money in — come last.
fn category_palette(theme: &gpui_kit::component::theme::Theme) -> [Hsla; 12] {
    [
        theme.blue,
        theme.cyan,
        theme.magenta,
        theme.yellow,
        theme.green,
        theme.red,
        theme.blue_light,
        theme.cyan_light,
        theme.magenta_light,
        theme.yellow_light,
        theme.green_light,
        theme.red_light,
    ]
}

pub struct Shell {
    db: DbWorker,
    db_path: PathBuf,
    csv_root: PathBuf,
    accounts: Vec<Account>,
    page: Page,
    // -- Dashboard accounts filter and transfers toggle (3.3): the accounts
    // the user unchecked in the dropdown (empty = all), and the
    // "Include transfers" flag persisted as `ui.include_transfers`.
    excluded_accounts: Vec<i64>,
    include_transfers: bool,
    include_transfers_task: Option<Task<()>>,
    // -- Dashboard table filters (3.4): a case-insensitive search over the
    // description/merchant, the two table-only toggles and the sort. Empty /
    // false / the default sort mean "no narrowing". (The category and month
    // the query also supports are chosen from the chart and legend in 3.5/3.6.)
    search_input: Entity<InputState>,
    search: String,
    uncategorized_only: bool,
    show_hidden: bool,
    tx_sort: TxSort,
    // -- Dashboard category chart + legend (3.5): the parent whose children
    // the chart and legend drill into (`None` = the parent level), the one
    // legend selection that drives the table's category filter, and the
    // `spend_by_category` rows behind the chart and legend.
    chart_parent: Option<i64>,
    legend_selection: Option<LegendSelection>,
    chart_rows: Vec<CategorySpend>,
    // -- Responsive chart (3.15): the window's current width in pixels,
    // tracked with `observe_window_bounds`, so the chart can shorten and
    // thin its band labels to the space left beside the legend.
    window_width: f32,
    // -- Dashboard monthly summary (3.6): the `monthly_summary` grid behind
    // the panel, the parents whose child rows are expanded, and the month
    // chosen from a summary cell (fed into the table's `month` filter; the
    // `Total` column and "no month narrowing" are both `None`).
    summary: MonthlySummary,
    expanded_parents: HashSet<i64>,
    summary_month: Option<String>,
    // -- Dashboard date range (3.2): the control state, the `ViewFilter`
    // it drives (used by 3.4–3.6), and the picker entities.
    date_range: DateRange,
    view_filter: ViewFilter,
    date_preset_select: Entity<SelectState<Names>>,
    date_from_picker: Entity<DatePickerState>,
    date_to_picker: Entity<DatePickerState>,
    date_range_task: Option<Task<()>>,
    table: Entity<TableState<TxTable>>,
    row_count: usize,
    total_visible: i64,
    uncategorized_visible: i64,
    // -- Transfers page (4.2): the open transfer reviews the pairing engine
    // left behind. The sidebar pill counts them; the page lists each with
    // its candidates.
    transfer_reviews: Vec<TransferReview>,
    /// The transaction a manual category edit just made, `(transaction_id,
    /// category_id)`, whose row now offers to extend that choice to every
    /// matching row (2.6's rule offer, surfaced from the 2.9 menu).
    rule_offer: Option<(i64, i64)>,
    import: Option<Entity<ImportPanel>>,
    import_history: Vec<ImportEntry>,
    history_table: Entity<TableState<ImportHistoryTable>>,
    confirm_undo_import: Option<i64>,
    status: Option<(String, bool)>,
    refresh_task: Option<Task<()>>,
    task: Option<Task<()>>,
    // -- Categories page
    categories: Vec<CategoryNode>,
    cat_counts: HashMap<i64, i64>,
    selected_category: Option<i64>,
    new_parent_name: Entity<InputState>,
    new_parent_kind: Entity<SelectState<Names>>,
    new_child_name: Entity<InputState>,
    renaming: bool,
    rename_input: Entity<InputState>,
    move_target: Entity<SelectState<Names>>,
    delete_target: Entity<SelectState<Names>>,
    confirm_delete_category: Option<i64>,
    cat_error: Option<String>,
    rules: Vec<Rule>,
    child_choices: Vec<(i64, String)>,
    rule_pattern: Entity<InputState>,
    rule_regex: bool,
    rule_target: Entity<SelectState<Names>>,
    rule_match_count: Option<i64>,
    rule_task: Option<Task<()>>,
    rule_error: Option<String>,
    /// 3.24: the rule whose row currently shows the in-place editor. One at
    /// a time; `select_category` and `go_page` close it. `start_edit_rule`
    /// loads the rule into the editor's own controls, so the editor is the
    /// add-rule row's shape with the row swapped out.
    editing_rule: Option<i64>,
    /// 3.24: the rule whose Delete… button is armed ("Click again to
    /// delete"). Armed per row; `select_category` and `go_page` disarm it
    /// with the page's other transient confirmations.
    armed_delete_rule: Option<i64>,
    /// 3.24: the editor's own controls, separate entities from the add-rule
    /// row's so the two never share an `InputState`/`SelectState` on screen
    /// at once. While the editor is open the shared `rule_match_count`
    /// preview belongs to it (the add-rule row hides it, like `rule_error`)
    /// and excludes that rule (2.5 re-run semantics); every close path
    /// clears it, and `sync_category_choices` closes the editor when its
    /// rule is gone.
    rule_edit_pattern: Entity<InputState>,
    rule_edit_target: Entity<SelectState<Names>>,
    rule_edit_regex: bool,
    /// 3.21: one scroll handle per virtualized list. gpui-kit's
    /// `v_virtual_list` creates a fresh `VirtualListScrollHandle` on every
    /// render and `track_scroll`s it, and GPUI reads the scroll offset from
    /// a tracked handle in preference to element state — so a per-render
    /// handle snaps the offset back to 0 every frame. One handle per list
    /// lives as long as the `Shell`. Reset an offset only where the list's
    /// content is actually replaced, never on a plain refresh (the rule
    /// list's reset point arrives with 3.23, where the selected category
    /// replaces the rules shown).
    tree_scroll: VirtualListScrollHandle,
    rule_scroll: VirtualListScrollHandle,
    queue_scroll: VirtualListScrollHandle,
    // -- Virtualized Categories-page lists (3.12): how many rows each list
    // built for the last layout pass — the visible slice, not the whole
    // data set. Observability for the long-list tests.
    #[cfg(test)]
    tree_rows_built: std::cell::Cell<usize>,
    /// 3.21: the range of rows the tree built last frame, so the scroll
    /// tests can see the window move.
    #[cfg(test)]
    tree_range: std::cell::Cell<(usize, usize)>,
    #[cfg(test)]
    rule_rows_built: std::cell::Cell<usize>,
    /// 3.21: the range of rows the rule list built last frame.
    #[cfg(test)]
    rule_range: std::cell::Cell<(usize, usize)>,
    // -- To categorize queue
    queue: Vec<UncategorizedGroup>,
    /// 3.19: pickers exist only for rows `render_queue` has built, so this
    /// map stays the size of the visible slice, not the queue.
    queue_pickers: HashMap<String, Entity<SelectState<Names>>>,
    /// The category list the pickers above currently hold; `sync_queue`
    /// re-sets items only when it changes, not on every refresh.
    queue_picker_items: Option<Names>,
    queue_error: Option<String>,
    /// 3.19: how many queue rows the virtualized list built for the last
    /// layout pass — the visible slice, not the whole queue. Observability
    /// for the long-queue tests (like `tree_rows_built`).
    #[cfg(test)]
    queue_rows_built: std::cell::Cell<usize>,
    /// 3.21: the range of rows the queue built last frame.
    #[cfg(test)]
    queue_range: std::cell::Cell<(usize, usize)>,
    _subs: Vec<Subscription>,
}

const KIND_LABELS: [(&str, CategoryKind); 3] = [
    ("Expense", CategoryKind::Expense),
    ("Income", CategoryKind::Income),
    ("Transfer", CategoryKind::Transfer),
];

fn kind_label(kind: CategoryKind) -> &'static str {
    KIND_LABELS
        .iter()
        .find(|(_, k)| *k == kind)
        .map_or("Expense", |(l, _)| l)
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
    pub fn new(
        db_path: PathBuf,
        db: DbWorker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let table = cx.new(|cx| {
            TableState::new(
                TxTable {
                    rows: Vec::new(),
                    child_choices: Vec::new(),
                    rule_offer: None,
                    sort: TxSort::default(),
                    on_set_category,
                    on_set_hidden,
                    on_apply_rule,
                    on_unpair,
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
        let on_undo = {
            let shell = cx.entity();
            Rc::new(move |id: i64, window: &mut Window, cx: &mut App| {
                shell.update(cx, |this, cx| this.undo_import(id, window, cx));
            }) as Rc<dyn Fn(i64, &mut Window, &mut App)>
        };
        let history_table = cx.new(|cx| {
            TableState::new(
                ImportHistoryTable {
                    rows: Vec::new(),
                    confirm_id: None,
                    // 3.20: start at the widths the current window allows.
                    flex_widths: history_flex_widths(
                        (window.bounds().size.width.as_f32() - SIDEBAR_W - 2. * PAGE_PAD).max(0.),
                    ),
                    on_undo,
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
        let csv_root = financi_db::master_csv::root_for_db(&db_path);
        // Esc clears the dashboard's category selection (3.5); the root
        // view handles the action in `Render::render`.
        cx.bind_keys([KeyBinding::new("escape", ClearCategorySelection, None)]);
        // ⌘ shortcuts for the file actions (3.7); `cmd` is ⌘ on macOS and
        // Super on Linux. The same actions reach the user through the
        // Imports page header on every platform.
        cx.bind_keys([
            KeyBinding::new("cmd-o", ImportCsv, None),
            KeyBinding::new("cmd-e", ExportData, None),
            KeyBinding::new("cmd-q", QuitApp, None),
        ]);
        // The native menu bar (macOS; a no-op store elsewhere). Key
        // equivalents are read from the keymap, so bind first.
        cx.set_menus(app_menus());
        // The file actions are handled globally, not on the root element:
        // the native menu dispatches them without a focused element, and
        // `App::is_action_available` — what macOS uses to enable menu
        // items — only counts global listeners in that case.
        let shell = cx.entity();
        let window_handle = window.window_handle();
        Self::on_shell_action(
            &shell,
            window_handle,
            cx,
            |this, _: &ImportCsv, window, cx| {
                this.pick_file(window, cx);
            },
        );
        Self::on_shell_action(
            &shell,
            window_handle,
            cx,
            |this, _: &UndoLastImport, window, cx| {
                this.undo_last_import(window, cx);
            },
        );
        Self::on_shell_action(&shell, window_handle, cx, |this, _: &ExportData, _, cx| {
            this.export_data(cx);
        });
        Self::on_shell_action(
            &shell,
            window_handle,
            cx,
            |this, _: &RestoreBackup, _, cx| {
                this.restore_backup(cx);
            },
        );
        Self::on_shell_action(&shell, window_handle, cx, |_, _: &QuitApp, _, cx| cx.quit());
        let new_parent_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("New parent name"));
        let new_parent_kind = cx.new(|cx| {
            SelectState::new(
                KIND_LABELS.iter().map(|(l, _)| (*l).into()).collect(),
                Some(gpui_kit::component::IndexPath::new(0)),
                window,
                cx,
            )
        });
        let new_child_name = cx.new(|cx| InputState::new(window, cx).placeholder("New child name"));
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New name"));
        let move_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        let delete_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        let rule_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        let rule_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        // The live "matches N transactions" preview runs on every pattern
        // keystroke (ui-conventions, Categories page).
        let preview_sub = cx.subscribe(&rule_pattern, |this, _, ev: &InputEvent, cx| {
            // 3.24: while a rule editor is open, the preview belongs to it.
            if matches!(ev, InputEvent::Change) && this.editing_rule.is_none() {
                this.preview_rule_match(cx);
            }
        });
        // 3.24: the rule editor's Pattern input drives the same preview and
        // saves on Enter (Escape cancels through the window-level binding
        // handled in `clear_category_selection`).
        let rule_edit_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        let rule_edit_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        let rule_edit_sub = cx.subscribe_in(
            &rule_edit_pattern,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change if this.editing_rule.is_some() => this.preview_rule_match(cx),
                InputEvent::PressEnter {
                    secondary: false, ..
                } if this.editing_rule.is_some() => this.save_rule_edit(window, cx),
                _ => {}
            },
        );
        // The dashboard search box filters the table on every keystroke.
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search description or merchant"));
        let search_sub = cx.subscribe_in(
            &search_input,
            window,
            |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let value = this.search_input.read(cx).value().trim().to_string();
                    this.apply_search(&value, window, cx);
                }
            },
        );
        // The chart's label plan follows the window width (3.15), and so
        // do the Imports history table's (3.20) and the dashboard table's
        // flexible columns (3.25).
        let bounds_sub = cx.observe_window_bounds(window, |this, window, cx| {
            let width = window.bounds().size.width.as_f32();
            if this.window_width != width {
                this.window_width = width;
                let page_w = (width - SIDEBAR_W - 2. * PAGE_PAD).max(0.);
                let flex = history_flex_widths(page_w);
                this.history_table.update(cx, |t, cx| {
                    t.delegate_mut().flex_widths = flex;
                    // Re-read the delegate's columns with the new widths.
                    t.refresh(cx);
                });
                let flex = tx_flex_widths(page_w);
                this.table.update(cx, |t, cx| {
                    t.delegate_mut().flex_widths = flex;
                    t.refresh(cx);
                });
                cx.notify();
            }
        });
        let mut shell = Self {
            db,
            db_path,
            csv_root,
            accounts: Vec::new(),
            page: Page::Dashboard,
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
            window_width: window.bounds().size.width.as_f32(),
            summary: MonthlySummary {
                months: Vec::new(),
                rows: Vec::new(),
            },
            expanded_parents: HashSet::new(),
            summary_month: None,
            date_range: DateRange::default(),
            view_filter: ViewFilter::default(),
            date_preset_select,
            date_from_picker,
            date_to_picker,
            date_range_task: None,
            table,
            row_count: 0,
            total_visible: 0,
            uncategorized_visible: 0,
            transfer_reviews: Vec::new(),
            rule_offer: None,
            import: None,
            import_history: Vec::new(),
            history_table,
            confirm_undo_import: None,
            status: None,
            refresh_task: None,
            task: None,
            categories: Vec::new(),
            cat_counts: HashMap::new(),
            selected_category: None,
            new_parent_name,
            new_parent_kind,
            new_child_name,
            renaming: false,
            rename_input,
            move_target,
            delete_target,
            confirm_delete_category: None,
            cat_error: None,
            rules: Vec::new(),
            child_choices: Vec::new(),
            rule_pattern,
            rule_regex: false,
            rule_target,
            rule_match_count: None,
            rule_task: None,
            rule_error: None,
            editing_rule: None,
            armed_delete_rule: None,
            rule_edit_pattern,
            rule_edit_target,
            rule_edit_regex: false,
            tree_scroll: VirtualListScrollHandle::new(),
            rule_scroll: VirtualListScrollHandle::new(),
            queue_scroll: VirtualListScrollHandle::new(),
            #[cfg(test)]
            tree_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            tree_range: std::cell::Cell::new((0, 0)),
            #[cfg(test)]
            rule_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            rule_range: std::cell::Cell::new((0, 0)),
            queue: Vec::new(),
            queue_pickers: HashMap::new(),
            queue_picker_items: None,
            queue_error: None,
            #[cfg(test)]
            queue_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            queue_range: std::cell::Cell::new((0, 0)),
            _subs: vec![
                preview_sub,
                rule_edit_sub,
                search_sub,
                date_preset_sub,
                date_from_sub,
                date_to_sub,
                bounds_sub,
            ],
        };
        shell.load_date_range(window, cx);
        shell.load_include_transfers(window, cx);
        shell.refresh(window, cx);
        shell
    }

    /// Re-query everything the pages show: accounts and transactions for
    /// the dashboard, sidebar counts, and the import history.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        // The account filter resolves against the accounts loaded *inside*
        // this refresh (task 3.17): an import can add accounts while every
        // existing checkbox is unchecked, and reading `self.accounts` here
        // would read "nothing selected" from the previous refresh and blank
        // every view until some other filter changes.
        let excluded = self.excluded_accounts.clone();
        let mut view_filter = self.view_filter.clone();
        view_filter.include_transfers = self.include_transfers;
        // The table-only filters (3.4): search, the two toggles and the sort,
        // folded into the query alongside the shared date-range/accounts/
        // transfers filter. The category slot carries the legend selection
        // (3.5), shared with the summary cells (3.6); the `Uncategorized`
        // legend row rides `uncategorized_only`. The month is the summary
        // cell selection (3.6).
        let table_filter = TxTableFilter {
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
        };
        let chart_parent = self.chart_parent;
        self.refresh_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                let accounts = list_accounts(c)?;
                // Resolve the account filter against those accounts: the
                // ids the user has not excluded. Empty `account_ids` means
                // *all* accounts (the `ViewFilter` convention); "no
                // accounts checked" is handled by skipping the queries.
                let account_ids: Vec<i64> = accounts
                    .iter()
                    .filter(|a| !excluded.contains(&a.id))
                    .map(|a| a.id)
                    .collect();
                let none_selected = account_ids.is_empty() && !accounts.is_empty();
                view_filter.account_ids = if excluded.is_empty() {
                    Vec::new()
                } else {
                    account_ids
                };
                let rows = if none_selected {
                    Vec::new()
                } else {
                    list_transactions_filtered(c, &view_filter, &table_filter)?
                };
                // Chart and legend rows: parents, or the drilled-in parent's
                // children (3.5).
                let chart_rows = if none_selected {
                    Vec::new()
                } else {
                    spend_by_category(c, &view_filter, chart_parent)?
                };
                // The monthly summary grid (3.6): the same filter as the
                // chart, one query for the whole panel.
                let summary = if none_selected {
                    MonthlySummary {
                        months: Vec::new(),
                        rows: Vec::new(),
                    }
                } else {
                    monthly_summary(c, &view_filter)?
                };
                let (total, uncategorized) = sidebar_counts(c)?;
                let history = list_imports(c)?;
                let tree = list_categories(c)?;
                let counts = transaction_counts(c)?;
                let rules = list_rules(c)?;
                let queue = uncategorized_by_merchant(c)?;
                // Open transfer reviews (4.2) for the sidebar pill and the
                // Transfers page.
                let reviews = list_transfer_reviews(c)?;
                Ok::<_, financi_db::DbError>((
                    accounts,
                    rows,
                    chart_rows,
                    summary,
                    total,
                    uncategorized,
                    history,
                    tree,
                    counts,
                    rules,
                    queue,
                    reviews,
                ))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((
                        accounts,
                        rows,
                        chart_rows,
                        summary,
                        total,
                        uncategorized,
                        history,
                        tree,
                        counts,
                        rules,
                        queue,
                        reviews,
                    )) => {
                        if !history
                            .iter()
                            .any(|e| Some(e.id) == this.confirm_undo_import)
                        {
                            this.confirm_undo_import = None;
                        }
                        this.accounts = accounts;
                        // Keep the shared `ViewFilter` in step with the
                        // checkboxes: empty means all accounts.
                        this.sync_account_filter();
                        this.row_count = rows.len();
                        this.chart_rows = chart_rows;
                        this.summary = summary;
                        // Revalidate the legend state against the rows we
                        // can actually show: a date/accounts change can
                        // empty the chart out from under a selection or a
                        // drill-down, and the table filter must not keep
                        // narrowing by a category that is no longer there.
                        let mut stale = false;
                        let drilled = this.chart_parent;
                        if drilled.is_some() && this.chart_rows.is_empty() {
                            this.chart_parent = None;
                            stale = true;
                        }
                        let selection_gone = match this.legend_selection {
                            Some(LegendSelection::Category(id)) => {
                                // While drilled, the selected parent itself
                                // is not among the child rows — it stays.
                                drilled != Some(id)
                                    && !this.chart_rows.iter().any(|r| r.id == Some(id))
                            }
                            Some(LegendSelection::Uncategorized) => {
                                this.chart_parent.is_none()
                                    && !this.chart_rows.iter().any(|r| r.id.is_none())
                            }
                            None => false,
                        };
                        if selection_gone {
                            this.legend_selection = None;
                            stale = true;
                        }
                        // The month selection (3.6) goes the same way when
                        // the grid no longer contains that month.
                        if this.summary_month.is_some()
                            && !this
                                .summary
                                .months
                                .iter()
                                .any(|m| Some(m.as_str()) == this.summary_month.as_deref())
                        {
                            this.summary_month = None;
                            stale = true;
                        }
                        if stale {
                            // Re-query with the corrected filter so the
                            // table, chart and state agree again.
                            this.refresh(window, cx);
                        }
                        this.total_visible = total;
                        this.uncategorized_visible = uncategorized;
                        this.import_history = history;
                        this.categories = tree;
                        this.cat_counts = counts;
                        this.rules = rules;
                        this.queue = queue;
                        this.transfer_reviews = reviews;
                        this.sync_category_choices(window, cx);
                        this.sync_queue(window, cx);
                        // The rule offer follows a row the user just edited; a
                        // refresh that drops the row (hidden, filtered out,
                        // deleted) drops the offer with it.
                        if this
                            .rule_offer
                            .is_some_and(|(id, _)| !rows.iter().any(|r| r.id == id))
                        {
                            this.rule_offer = None;
                        }
                        let child_choices = this.child_choices.clone();
                        let rule_offer = this.rule_offer.map(|(id, _)| id);
                        let sort = this.tx_sort;
                        this.table.update(cx, |t, cx| {
                            let delegate = t.delegate_mut();
                            delegate.rows = rows;
                            delegate.child_choices = child_choices;
                            delegate.rule_offer = rule_offer;
                            delegate.sort = sort;
                            t.refresh(cx);
                        });
                        let confirm = this.confirm_undo_import;
                        let history = this.import_history.clone();
                        this.history_table.update(cx, |t, cx| {
                            let delegate = t.delegate_mut();
                            delegate.rows = history;
                            delegate.confirm_id = confirm;
                            t.refresh(cx);
                        });
                    }
                    Err(e) => this.status = Some((format!("Database error: {e}"), true)),
                }
                cx.notify();
            });
        }));
    }

    /// Today, for the preset ranges; `preset_range` itself never reads a
    /// clock.
    fn today() -> NaiveDate {
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
    fn change_date_preset(
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
    fn date_picked(
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
    fn load_date_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    fn sync_account_filter(&mut self) {
        self.view_filter.account_ids = if self.excluded_accounts.is_empty() {
            Vec::new()
        } else {
            self.selected_account_ids()
        };
    }

    /// Check or uncheck one account in the filter dropdown; the table
    /// follows immediately.
    fn set_account_checked(
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
    fn set_include_transfers(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
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
    fn apply_search(&mut self, term: &str, window: &mut Window, cx: &mut Context<Self>) {
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
    fn set_uncategorized_only(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
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
    fn set_show_hidden(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
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
    fn set_tx_sort(&mut self, sort: TxSort, window: &mut Window, cx: &mut Context<Self>) {
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
    fn click_legend_row(&mut self, id: Option<i64>, window: &mut Window, cx: &mut Context<Self>) {
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
    fn show_all_categories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chart_parent = None;
        self.legend_selection = None;
        self.summary_month = None;
        self.refresh(window, cx);
    }

    /// Esc (the `dashboard::ClearCategorySelection` action): clear the
    /// drill-down, the legend selection and the summary month together —
    /// they are one concept. 3.24: an open rule editor is the innermost
    /// thing, so Esc cancels it first and nothing else.
    fn clear_category_selection(
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
    fn click_summary_cell(
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
    fn toggle_summary_parent(&mut self, id: i64) {
        if !self.expanded_parents.insert(id) {
            self.expanded_parents.remove(&id);
        }
    }

    /// The summary rows prepared for drawing: the grid's rows with their
    /// kinds resolved (children of collapsed parents left out) and the
    /// month values followed by the `Total`. The query's last three rows
    /// are always `Income`, `Spend` and `Net`.
    fn summary_rows(&self) -> Vec<SummaryRowView> {
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
    fn load_include_transfers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn go_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        // 3.11: no transient UI travels across a page switch — the two-click
        // confirmations, the rule offer and the inline rename editor all
        // belong to the page they were opened on. 3.24: the rule editor and
        // the armed rule delete join that list.
        self.confirm_undo_import = None;
        self.rule_offer = None;
        self.renaming = false;
        self.confirm_delete_category = None;
        self.close_rule_editor();
        self.armed_delete_rule = None;
        cx.notify();
    }

    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match flow::load_file(&path) {
            Ok(file) => {
                let db = self.db.clone();
                let csv_root = self.csv_root.clone();
                let panel = cx.new(|cx| ImportPanel::new(db, csv_root, file, window, cx));
                let sub =
                    cx.subscribe_in(&panel, window, |this, _, ev: &ImportEvent, window, cx| {
                        match ev {
                            ImportEvent::Imported { message, is_error } => {
                                this.status = Some((message.clone(), *is_error));
                                this.import = None;
                            }
                            ImportEvent::Cancelled => this.import = None,
                            ImportEvent::AccountsChanged => {}
                        }
                        this.refresh(window, cx);
                    });
                self._subs.push(sub);
                self.import = Some(panel);
                self.status = None;
                // Opening a file switches to the Imports page with the
                // preview open (ui-conventions, *Pages*).
                self.page = Page::Imports;
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
        cx.notify();
    }

    fn pick_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| this.open_file(path, window, cx));
            }
        }));
    }

    /// Run `handler` on the shell in `window`. The global file-action
    /// listeners (3.7) funnel through this, so they work from any focus
    /// state — including none, as the native menu leaves it.
    fn on_shell_action<A: Action>(
        shell: &Entity<Shell>,
        window: AnyWindowHandle,
        cx: &mut App,
        handler: impl Fn(&mut Shell, &A, &mut Window, &mut Context<Shell>) + 'static,
    ) {
        let shell = shell.clone();
        let handler = std::rc::Rc::new(handler);
        cx.on_action(move |action: &A, cx: &mut App| {
            let shell = shell.clone();
            let handler = handler.clone();
            // The listener runs inside the window's update loop, so the
            // shell update has to be deferred out of it (same pattern as
            // `Window::dispatch_action`).
            let action = action.boxed_clone();
            cx.defer(move |cx| {
                let Some(action) = action.as_any().downcast_ref::<A>() else {
                    return;
                };
                let _ = window.update(cx, |_, window, cx| {
                    shell.update(cx, |this, cx| handler(this, action, window, cx));
                });
            });
        });
    }

    /// Undo the most recent successful import (the File menu item and the
    /// Imports header button): the same path as the newest history row's
    /// Undo, two-click confirmation included; a no-op with no history.
    fn undo_last_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.import_history.first() {
            let id = entry.id;
            self.undo_import(id, window, cx);
        }
    }

    /// Export… (⌘E): wired in M5. Until then the menu item, the ⌘E
    /// shortcut and the Imports header button all say so plainly.
    fn export_data(&mut self, cx: &mut Context<Self>) {
        self.status = Some(("Export is not available yet.".into(), false));
        cx.notify();
    }

    /// Restore…: wired in M5, like [`Shell::export_data`].
    fn restore_backup(&mut self, cx: &mut Context<Self>) {
        self.status = Some(("Restore is not available yet.".into(), false));
        cx.notify();
    }

    /// Undo an import from the history: the first click arms the button,
    /// the second runs the undo (ui-conventions, *Interaction*).
    fn undo_import(&mut self, import_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_undo_import != Some(import_id) {
            self.confirm_undo_import = Some(import_id);
            self.history_table.update(cx, |t, _| {
                t.delegate_mut().confirm_id = Some(import_id);
            });
            cx.notify();
            return;
        }
        self.confirm_undo_import = None;
        let Some(account_id) = self
            .import_history
            .iter()
            .find(|e| e.id == import_id)
            .map(|e| e.account_id)
        else {
            cx.notify();
            return;
        };
        let db = self.db.clone();
        let csv_root = self.csv_root.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                let undone = undo_last_import(c, account_id)?;
                let csv_error = match &undone {
                    Some(u) => {
                        let years: Vec<i32> = u.affected_years.iter().copied().collect();
                        financi_db::master_csv::rewrite_years(c, &csv_root, account_id, &years)
                            .err()
                            .map(|e| e.to_string())
                    }
                    None => None,
                };
                Ok::<_, financi_db::DbError>((undone, csv_error))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.status = Some(match result {
                    Ok((Some(u), csv_error)) => {
                        let mut message = format!(
                            "Undid import of {}: removed {} rows",
                            u.file_name, u.removed
                        );
                        let mut is_error = false;
                        if let Some(e) = csv_error {
                            message.push_str(&format!(" (master CSV update failed: {e})"));
                            is_error = true;
                        }
                        (message, is_error)
                    }
                    Ok((None, _)) => ("This account has no imports to undo.".into(), false),
                    Err(e) => (e, true),
                });
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    // -- categories --------------------------------------------------------

    /// Recompute the derived choice lists after a refresh and drop stale
    /// selection state.
    fn sync_category_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let known = |id: i64| {
            self.categories
                .iter()
                .any(|p| p.id == id || p.children.iter().any(|c| c.id == id))
        };
        if self.selected_category.is_some_and(|id| !known(id)) {
            self.selected_category = None;
            self.renaming = false;
        }
        if self.confirm_delete_category.is_some_and(|id| !known(id)) {
            self.confirm_delete_category = None;
        }
        self.child_choices = self
            .categories
            .iter()
            .flat_map(|p| {
                p.children
                    .iter()
                    .map(move |c| (c.id, format!("{} › {}", p.name, c.name)))
            })
            .collect();
        let names: Names = self
            .child_choices
            .iter()
            .map(|(_, n)| SharedString::from(n.clone()))
            .collect();
        self.rule_target
            .update(cx, |s, cx| s.set_items(names.clone(), window, cx));
        self.delete_target
            .update(cx, |s, cx| s.set_items(names, window, cx));
        // 3.24: a refresh that dropped the rule (its category was deleted,
        // say) drops the editor open on it and its armed delete with it,
        // like the `rule_offer` check in `refresh`.
        if let Some(id) = self.editing_rule
            && !self.rules.iter().any(|r| r.id == id)
        {
            self.close_rule_editor();
        }
        if let Some(id) = self.armed_delete_rule
            && !self.rules.iter().any(|r| r.id == id)
        {
            self.armed_delete_rule = None;
        }
    }

    /// The selected node as (node, parent), parents having `None`.
    fn selected_node(&self) -> Option<(&CategoryNode, Option<&CategoryNode>)> {
        let id = self.selected_category?;
        for p in &self.categories {
            if p.id == id {
                return Some((p, None));
            }
            if let Some(c) = p.children.iter().find(|c| c.id == id) {
                return Some((c, Some(p)));
            }
        }
        None
    }

    /// Visible transaction count: a parent owns none itself, the sum of
    /// its children stands for it.
    fn node_count(&self, node: &CategoryNode) -> i64 {
        if node.children.is_empty() {
            self.cat_counts.get(&node.id).copied().unwrap_or(0)
        } else {
            node.children
                .iter()
                .map(|c| self.cat_counts.get(&c.id).copied().unwrap_or(0))
                .sum()
        }
    }

    fn select_category(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_category = Some(id);
        self.renaming = false;
        self.confirm_delete_category = None;
        self.cat_error = None;
        // 3.24: selecting another category closes the rule editor (with its
        // preview count) and disarms any armed rule delete — the same
        // "transient UI belongs to what it was opened on" rule as the page
        // switch.
        self.close_rule_editor();
        self.armed_delete_rule = None;
        // 3.21: the rule list keeps its scroll offset here — its content
        // (all rules) doesn't change when the selection does. When 3.23
        // scopes the rule list to the selected category, this is the reset
        // point: `self.rule_scroll.set_offset(Point::default())`, where
        // the content is actually replaced.
        self.rebuild_move_target(window, cx);
        cx.notify();
    }

    /// Same-kind parents for the Move picker, the child's own parent out.
    fn move_parent_choices(&self) -> Vec<(i64, String)> {
        let Some((node, parent)) = self.selected_node() else {
            return Vec::new();
        };
        if parent.is_none() {
            return Vec::new();
        }
        self.categories
            .iter()
            .filter(|p| p.kind == node.kind && parent.is_none_or(|par| par.id != p.id))
            .map(|p| (p.id, p.name.clone()))
            .collect()
    }

    fn rebuild_move_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Names = self
            .move_parent_choices()
            .into_iter()
            .map(|(_, n)| n.into())
            .collect();
        self.move_target
            .update(cx, |s, cx| s.set_items(names, window, cx));
    }

    fn add_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_parent_name.read(cx).value().trim().to_string();
        let kind = self
            .new_parent_kind
            .read(cx)
            .selected_index(cx)
            .map(|ix| KIND_LABELS[ix.row].1)
            .unwrap_or(CategoryKind::Expense);
        if name.is_empty() {
            self.cat_error = Some("Give the parent a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_parent(c, &name, kind)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(id) => {
                        this.new_parent_name
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.selected_category = Some(id);
                        this.cat_error = None;
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// Add a child under the selected parent.
    fn add_child(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((parent, None)) = self.selected_node() else {
            return;
        };
        let parent_id = parent.id;
        let name = self.new_child_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.cat_error = Some("Give the child a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_child(c, parent_id, &name)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(id) => {
                        this.new_child_name
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.selected_category = Some(id);
                        this.cat_error = None;
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((node, _)) = self.selected_node() else {
            return;
        };
        let name = node.name.clone();
        self.renaming = true;
        self.rename_input
            .update(cx, |i, cx| i.set_value(name, window, cx));
        cx.notify();
    }

    fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_category else {
            return;
        };
        let name = self.rename_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.cat_error = Some("Give the category a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| rename_category(c, id, &name)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.renaming = false;
                        this.cat_error = None;
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    fn move_child_to_selected_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((node, Some(_))) = self.selected_node() else {
            return;
        };
        let child_id = node.id;
        let Some(ix) = self.move_target.read(cx).selected_index(cx) else {
            self.cat_error = Some("Choose the parent to move it under.".into());
            cx.notify();
            return;
        };
        let Some((new_parent_id, _)) = self.move_parent_choices().get(ix.row).cloned() else {
            return;
        };
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_child(c, child_id, new_parent_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => this.cat_error = None,
                    Err(e) => this.cat_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// First click arms the delete, the second runs it (ui-conventions,
    /// *Interaction*).
    fn arm_delete_category(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_category else {
            return;
        };
        if self.confirm_delete_category != Some(id) {
            self.confirm_delete_category = Some(id);
            cx.notify();
            return;
        }
        let Some(ix) = self.delete_target.read(cx).selected_index(cx) else {
            self.cat_error = Some("Choose where its transactions should go.".into());
            cx.notify();
            return;
        };
        let Some((reassign_to, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        self.confirm_delete_category = None;
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| delete_category(c, id, reassign_to)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.selected_category = None;
                        this.cat_error = None;
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    // -- rules -------------------------------------------------------------

    /// Count rows the current pattern would categorize, live as the user
    /// types (ui-conventions, Categories page).
    fn preview_rule_match(&mut self, cx: &mut Context<Self>) {
        // 3.24: while a rule is being edited, the preview reads the
        // editor's controls and excludes that rule, so the count shows what
        // the edit would newly categorize (`count_rule_matches`' contract).
        let editing = self.editing_rule;
        let pattern = match editing {
            Some(_) => self.rule_edit_pattern.read(cx).value().trim().to_string(),
            None => self.rule_pattern.read(cx).value().trim().to_string(),
        };
        if pattern.is_empty() {
            self.rule_task = None;
            self.rule_match_count = None;
            cx.notify();
            return;
        }
        let is_regex = match editing {
            Some(_) => self.rule_edit_regex,
            None => self.rule_regex,
        };
        let db = self.db.clone();
        self.rule_task = Some(cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| {
                count_rule_matches(c, &pattern, is_regex, editing).map_err(|e| e.to_string())
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(count) = result {
                    this.rule_match_count = Some(count);
                    cx.notify();
                }
            });
        }));
    }

    fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pattern = self.rule_pattern.read(cx).value().trim().to_string();
        let is_regex = self.rule_regex;
        let Some(ix) = self.rule_target.read(cx).selected_index(cx) else {
            self.rule_error = Some("Choose the category the rule assigns.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| add_rule(c, &pattern, is_regex, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.rule_pattern
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.rule_error = None;
                    }
                    Err(e) => this.rule_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// 3.24: close the rule editor and drop everything it owns — the
    /// error line and the live preview count, including its in-flight task
    /// (dropping a `Task` cancels it, so a late result can't repopulate
    /// the count after the editor is gone). Returns whether an editor was
    /// open. Every editor-close path goes through here.
    fn close_rule_editor(&mut self) -> bool {
        if self.editing_rule.take().is_some() {
            self.rule_error = None;
            self.rule_task = None;
            self.rule_match_count = None;
            true
        } else {
            false
        }
    }

    /// 3.24: close the rule editor without saving. The editor's controls
    /// keep their values (invisible while closed); `start_edit_rule`
    /// re-fills them from the next rule.
    fn cancel_edit_rule(&mut self, cx: &mut Context<Self>) {
        if self.close_rule_editor() {
            cx.notify();
        }
    }

    /// 3.24: swap a rule's row for its editor, pre-filled from the rule —
    /// same shape as the add-rule row (Pattern, regex switch, Category
    /// picker, Save, Cancel). One rule is edited at a time, so opening a
    /// second row's editor closes the first and disarms any pending delete.
    fn start_edit_rule(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.rules.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        self.editing_rule = Some(id);
        self.armed_delete_rule = None;
        self.rule_error = None;
        self.rule_edit_regex = rule.is_regex;
        self.rule_edit_pattern.update(cx, |i, cx| {
            i.set_value(rule.pattern.clone(), window, cx);
        });
        let names: Names = self
            .child_choices
            .iter()
            .map(|(_, n)| SharedString::from(n.clone()))
            .collect();
        let selected = self
            .child_choices
            .iter()
            .position(|(cid, _)| *cid == rule.category_id);
        self.rule_edit_target.update(cx, |s, cx| {
            s.set_items(names, window, cx);
            s.set_selected_index(
                selected.map(gpui_kit::component::IndexPath::new),
                window,
                cx,
            );
        });
        self.preview_rule_match(cx);
        cx.notify();
    }

    /// 3.24: save the editor through [`update_rule`], which re-runs every
    /// rule over the unlocked rows (2.5). A validation error (empty
    /// pattern, bad regex) lands in `rule_error` — shown in `danger` under
    /// the editor — and the editor stays open. On success every view
    /// refreshes, as after `add_rule`.
    fn save_rule_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editing_rule else {
            return;
        };
        let pattern = self.rule_edit_pattern.read(cx).value().trim().to_string();
        let is_regex = self.rule_edit_regex;
        let Some(ix) = self.rule_edit_target.read(cx).selected_index(cx) else {
            self.rule_error = Some("Choose the category the rule assigns.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                update_rule(c, id, &pattern, is_regex, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.close_rule_editor();
                    }
                    Err(e) => this.rule_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// 3.24: Delete… with the two-click confirm (ui-conventions,
    /// *Interaction*): the first click only arms the button ("Click again
    /// to delete"), the second deletes through [`delete_rule`] — whose
    /// re-run leaves the freed rows to the remaining rules or
    /// Uncategorized — and refreshes every view.
    fn arm_delete_rule(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.armed_delete_rule == Some(id) {
            let db = self.db.clone();
            self.task = Some(cx.spawn_in(window, async move |this, cx| {
                let result = on_db(&db, move |c| delete_rule(c, id)).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    match result {
                        Ok(()) => {
                            this.armed_delete_rule = None;
                            if this.editing_rule == Some(id) {
                                this.close_rule_editor();
                            }
                        }
                        Err(e) => this.rule_error = Some(e),
                    }
                    this.refresh(window, cx);
                });
            }));
        } else {
            self.armed_delete_rule = Some(id);
        }
        cx.notify();
    }

    // -- To categorize queue ------------------------------------------------

    /// 3.19: pickers are built lazily by `render_queue` for the rows it
    /// shows, never up front per group. This retires pickers whose group
    /// left the queue and re-sets the items on the survivors only when the
    /// category list changed, so a refresh doesn't touch every picker.
    fn sync_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current: HashSet<&str> = self.queue.iter().map(|g| g.merchant.as_str()).collect();
        self.queue_pickers
            .retain(|merchant, _| current.contains(merchant.as_str()));
        let names: Names = self
            .child_choices
            .iter()
            .map(|(_, n)| SharedString::from(n.clone()))
            .collect();
        if self.queue_picker_items.as_ref() != Some(&names) {
            self.queue_picker_items = Some(names.clone());
            for picker in self.queue_pickers.values() {
                let items = names.clone();
                picker.update(cx, |s, cx| s.set_items(items, window, cx));
            }
        }
    }

    /// 3.19: the picker for one queue row, built on first use. Only visible
    /// rows render, so the shell holds a picker for what's on screen — a
    /// new one gets the current category list.
    fn queue_picker(
        &mut self,
        merchant: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Names>> {
        if let Some(picker) = self.queue_pickers.get(merchant) {
            return picker.clone();
        }
        let items: Names = self.queue_picker_items.clone().unwrap_or_else(|| {
            self.child_choices
                .iter()
                .map(|(_, n)| SharedString::from(n.clone()))
                .collect()
        });
        let picker = cx.new(|cx| SelectState::new(items, None, window, cx));
        self.queue_pickers
            .insert(merchant.to_string(), picker.clone());
        picker
    }

    /// Categorize a whole merchant group: creates the substring rule and
    /// re-runs the rules (2.7's `categorize_group`).
    fn categorize_merchant(
        &mut self,
        merchant: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.queue_pickers.get(&merchant).cloned() else {
            return;
        };
        let Some(ix) = picker.read(cx).selected_index(cx) else {
            self.queue_error = Some("Choose a category for this merchant first.".into());
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
                        this.status = Some((format!("Categorized {merchant}"), false));
                        this.queue_error = None;
                    }
                    Err(e) => this.queue_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    // -- row context menu (2.9) ---------------------------------------------

    /// Change one transaction's category from the row menu (2.9). `set_category`
    /// locks the row so rules never move it again (2.6); on success the row
    /// starts offering to extend this choice to its whole merchant group.
    fn change_row_category(
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
                        this.rule_offer = Some((transaction_id, category_id));
                        this.status = None;
                    }
                    Err(e) => this.status = Some((e, true)),
                }
                this.refresh(window, cx);
            });
        }));
    }

    /// Hide or unhide a transaction from the row menu (2.9). Hidden rows leave
    /// the table, the sidebar counts and the To-categorize queue at once.
    fn set_row_hidden(
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
                    this.status = Some((e, true));
                }
                this.refresh(window, cx);
            });
        }));
    }

    /// Resolve an open transfer review by pairing its row with `candidate`
    /// (Transfers page, 4.2). The engine re-runs on the same commit, so a
    /// fresh review list and table come back on the next refresh.
    fn pair_transfer(
        &mut self,
        transaction_id: i64,
        candidate: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                resolve_transfer_review(c, transaction_id, Some(candidate))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.status = Some(("Paired transfer.".into(), false)),
                    Err(e) => this.status = Some((format!("Could not pair: {e}"), true)),
                }
                this.refresh(window, cx);
            });
        }));
    }

    /// Mark an open review's row *not a transfer* (Transfers page, 4.2): the
    /// row is excluded from pairing for good and the review is dropped.
    fn mark_not_transfer(
        &mut self,
        transaction_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                resolve_transfer_review(c, transaction_id, None)
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.status = Some(("Marked as not a transfer.".into(), false)),
                    Err(e) => this.status = Some((format!("Could not update: {e}"), true)),
                }
                this.refresh(window, cx);
            });
        }));
    }

    /// Unpair a linked transaction from the row menu (4.2): both sides are
    /// cleared, rules re-apply, and the engine re-runs.
    fn unpair_row(&mut self, transaction_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| unpair_transaction(c, transaction_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => this.status = Some(("Unpaired transaction.".into(), false)),
                    Err(e) => this.status = Some((format!("Could not unpair: {e}"), true)),
                }
                this.refresh(window, cx);
            });
        }));
    }

    /// Accept the post-edit offer from the row (2.6, 2.9): turn the merchant of
    /// the row just edited into a substring rule for the category chosen,
    /// categorizing every matching row, then clear the offer.
    fn apply_row_rule(&mut self, transaction_id: i64, window: &mut Window, cx: &mut Context<Self>) {
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
                        this.status =
                            Some(("Applied the category to all matching rows".into(), false));
                    }
                    Err(e) => this.status = Some((e, true)),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    // -- rendering ---------------------------------------------------------

    fn nav_item(
        &self,
        label: &'static str,
        icon: IconName,
        page: Page,
        cx: &mut Context<Self>,
    ) -> SidebarMenuItem {
        SidebarMenuItem::new(label)
            .icon(icon)
            // Drop the component's own left padding (3.13): the icon sits in
            // the same column as the section labels, the header and footer.
            .pl(px(NAV_ROW_INSET))
            .active(self.page == page)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.go_page(page, cx);
            }))
    }

    /// The sidebar: header, grouped items with count pills, footer
    /// (ui-conventions, *Sidebar*).
    fn sidebar(&mut self, cx: &mut Context<Self>) -> Sidebar<NavItem> {
        let theme = cx.theme();
        let (muted, pill_bg, pill_fg) = (
            theme.muted_foreground,
            theme.primary,
            theme.primary_foreground,
        );
        let dashboard = self.nav_item("Dashboard", IconName::LayoutDashboard, Page::Dashboard, cx);
        let to_categorize = self.nav_item(
            "To categorize",
            IconName::ListChecks,
            Page::ToCategorize,
            cx,
        );
        let to_categorize = if self.uncategorized_visible > 0 {
            let count = self.uncategorized_visible;
            to_categorize.suffix(move |_, _| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(20.))
                    .px_1()
                    .rounded(px(8.))
                    .text_xs()
                    .bg(pill_bg)
                    .text_color(pill_fg)
                    .child(flow::count(count))
            })
        } else {
            to_categorize
        };
        let categories = self.nav_item("Categories", IconName::Tags, Page::Categories, cx);
        // Transfers pill: the number of open reviews (4.2), hidden at 0.
        let transfers = self.nav_item("Transfers", IconName::ArrowLeftRight, Page::Transfers, cx);
        let transfers = if !self.transfer_reviews.is_empty() {
            let count = self.transfer_reviews.len() as i64;
            transfers.suffix(move |_, _| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(20.))
                    .px_1()
                    .rounded(px(8.))
                    .text_xs()
                    .bg(pill_bg)
                    .text_color(pill_fg)
                    .child(flow::count(count))
            })
        } else {
            transfers
        };
        let imports = self.nav_item("Imports", IconName::Download, Page::Imports, cx);

        Sidebar::new("nav")
            .w(px(SIDEBAR_W))
            .collapsible(false)
            .header(div().font_weight(FontWeight::SEMIBOLD).child("financi"))
            .child(NavItem::Item(Box::new(dashboard)))
            .child(NavItem::Section("Organize".into()))
            .child(NavItem::Item(Box::new(to_categorize)))
            .child(NavItem::Item(Box::new(categories)))
            .child(NavItem::Item(Box::new(transfers)))
            .child(NavItem::Section("Data".into()))
            .child(NavItem::Item(Box::new(imports)))
            .footer(
                v_flex()
                    .w_full()
                    .gap_1()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{} transactions", flow::count(self.total_visible)))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(self.db_path.display().to_string()),
                    ),
            )
    }

    /// The dashboard middle row (3.5): the `BarChart` of spend per category
    /// beside a clickable legend, in the left half; the monthly summary
    /// (3.6) takes the right half later. `BarChart` takes no clicks, so all
    /// interactivity lives in the legend rows (ui-conventions, *Interaction*).
    /// The palette color of every category (ui-conventions, *Category
    /// colors*): position among the siblings sorted by category id, so a
    /// category keeps its color when the date range reorders the rows;
    /// children start at the front of the palette under their own parent.
    /// Shared by the chart, its legend and the monthly summary swatches.
    fn category_colors(&self, theme: &gpui_kit::component::theme::Theme) -> HashMap<i64, Hsla> {
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

    /// The chart's label plan (3.15) for the current window: how many
    /// characters a band label may use and how many labels are drawn.
    /// The plot shares the window with everything left of it: the sidebar
    /// and page padding come off the window width, `CHART_PANEL_SHARE` of
    /// what remains is the chart panel, and inside the panel its padding,
    /// the legend and the gap come off before the plot is left.
    fn chart_label_plan(&self) -> (usize, usize) {
        let page_w = (self.window_width - SIDEBAR_W - 2. * PAGE_PAD).max(0.);
        let chart_w = (page_w * CHART_PANEL_SHARE - 2. * PAGE_PAD - LEGEND_W - REGION_GAP).max(0.);
        band_label_budget(chart_w, self.chart_rows.len())
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
        // Responsive labels (3.15): shorten the names to the current
        // budget and thin them out if the bands are too narrow; the full
        // names stay in the legend and the tooltip.
        let (label_chars, label_count) = self.chart_label_plan();
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
                cents: r.spend_cents,
                share: r.share,
                color: r
                    .id
                    .map_or(muted, |id| colors.get(&id).copied().unwrap_or(muted)),
            })
            .collect();
        // Uncategorized is always the last category row, whatever its amount.
        rows.sort_by_key(|r| r.id.is_none());
        let selection = self.legend_selection;
        // The `Uncategorized` row is active when the 3.4 checkbox drives the
        // same clause, so the legend never contradicts the toolbar.
        let uncategorized_active =
            matches!(selection, Some(LegendSelection::Uncategorized)) || self.uncategorized_only;

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
            let parent_name = self
                .chart_parent
                .and_then(|p| self.categories.iter().find(|n| n.id == p))
                .map(|n| format!(" of {}", n.name))
                .unwrap_or_default();
            div()
                .flex_1()
                .min_w_0()
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
                        .tooltip_title(|r| r.name.clone())
                        .tooltip_value(move |r, _| {
                            format!(
                                "{} · {:.1}%{parent_name}",
                                flow::money(r.cents),
                                r.share * 100.
                            )
                            .into()
                        }),
                )
                .into_any_element()
        };

        let legend = v_flex()
            .id("legend-list")
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
                div()
                    .id(SharedString::from(format!("legend-{ix}")))
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
                            .when(id.is_none(), |name| name.text_color(muted))
                            .child(r.name.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .w(px(76.))
                            .flex_none()
                            .text_right()
                            .child(flow::money(r.cents)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .w(px(52.))
                            .flex_none()
                            .text_right()
                            .child(format!("{:.1}%", r.share * 100.)),
                    )
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

        let header = h_flex()
            .gap_2()
            .px_1()
            .child(div().flex_1().min_w_0())
            .children(month_values.iter().map(|month| {
                div()
                    .w(px(88.))
                    .flex_none()
                    .text_right()
                    .text_sm()
                    .text_color(muted)
                    .child(flow::month_label(month))
            }))
            .child(
                div()
                    .w(px(96.))
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
                    .flex_1()
                    .min_w_0()
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
                    .children(row.cells.iter().enumerate().map(|(col_ix, &cents)| {
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
                        div()
                            .id(SharedString::from(format!("summary-cell-{ix}-{col_ix}")))
                            .w(if col_ix == month_values.len() {
                                px(96.)
                            } else {
                                px(88.)
                            })
                            .flex_none()
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
                    }))
                    .into_any_element()
            }));

        v_flex()
            .h(px(340.))
            .flex_1()
            .min_w_0()
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
            .into_any_element()
    }

    /// Which dashboard empty state applies (task 3.10, ui-conventions
    /// *States*). `total_visible` counts every non-hidden row ignoring all
    /// filters, so it is the "empty database" signal; `row_count` is the
    /// filtered table, so `0` with data present means the filters excluded
    /// everything.
    fn dashboard_body(&self) -> DashboardBody {
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
    fn summary_has_data(&self) -> bool {
        !self.summary.months.is_empty()
            && self
                .summary
                .rows
                .iter()
                .any(|r| r.months.iter().any(|v| *v != 0))
    }

    fn render_dashboard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let body = self.dashboard_body();
        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            // One labelled filter region (3.16): the date range, accounts
            // and transfers controls, then search and the table toggles —
            // every control carries a visible label, none relies on a
            // placeholder. The header row is still the filter row
            // (ui-conventions, *Pages*).
            .child(
                v_flex()
                    .w_full()
                    .debug_selector(|| "filters:region".into())
                    .gap_2()
                    .child(region_title("Filters"))
                    .child(
                        h_flex()
                            .w_full()
                            .debug_selector(|| "filters:row1".into())
                            .gap_3()
                            .items_start()
                            .child(filter_column(
                                muted,
                                "Date range",
                                div()
                                    .debug_selector(|| "filters:date-range".into())
                                    .w(FILTER_COL_W)
                                    .child(Select::new(&self.date_preset_select)),
                            ))
                            // The From/To columns want 140 px but shrink
                            // first when the row runs out of room, so the
                            // custom range fits the minimum window with the
                            // count (3.25 fix); the other columns hold.
                            .when(matches!(self.date_range, DateRange::Custom { .. }), |g| {
                                g.child(
                                    filter_column(
                                        muted,
                                        "From",
                                        div()
                                            .debug_selector(|| "filters:from".into())
                                            .child(DatePicker::new(&self.date_from_picker)),
                                    )
                                    .w(px(140.))
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W),
                                )
                                .child(
                                    filter_column(
                                        muted,
                                        "To",
                                        div()
                                            .debug_selector(|| "filters:to".into())
                                            .child(DatePicker::new(&self.date_to_picker)),
                                    )
                                    .w(px(140.))
                                    .flex_shrink(1.)
                                    .min_w(FILTER_DATE_MIN_W),
                                )
                            })
                            // Accounts multi-select (3.3): a dropdown popover of
                            // checkboxes, one per account, all checked by default.
                            .child(filter_column(
                                muted,
                                "Accounts",
                                // The selector (test builds only, 3.11) lets
                                // headless tests find the trigger and click it.
                                div()
                                    .debug_selector(|| "accounts:filter".into())
                                    .child(self.accounts_popover(cx)),
                            ))
                            .child(filter_column(
                                muted,
                                // No label of its own: the invisible label
                                // line (3.25) keeps the control centred on
                                // the same line as its neighbours.
                                "",
                                checkbox("include-transfers")
                                    .label("Include transfers")
                                    .checked(self.include_transfers)
                                    .on_click(cx.listener(|this, on: &bool, window, cx| {
                                        this.set_include_transfers(*on, window, cx);
                                    })),
                            ))
                            // The count rides the end of the row (3.25): it
                            // shares the control line and anchors the region's
                            // right edge to the page.
                            .child(
                                filter_column(
                                    muted,
                                    "",
                                    div()
                                        .debug_selector(|| "filters:count".into())
                                        .text_sm()
                                        .text_color(muted)
                                        .child(format!(
                                            "{} transactions",
                                            flow::count(self.row_count as i64)
                                        )),
                                )
                                .ml_auto(),
                            ),
                    )
                    // Second filter row (3.4): search box and the two table-only
                    // toggles. Kept on their own line so the date-range pickers
                    // never crowd them. The search column matches the first
                    // column of the row above (3.25, ui-conventions *Pages*).
                    .child(
                        h_flex()
                            .w_full()
                            .debug_selector(|| "filters:row2".into())
                            .gap_3()
                            .items_start()
                            .child(filter_column(
                                muted,
                                "Search",
                                div()
                                    .debug_selector(|| "filters:search".into())
                                    .w(FILTER_COL_W)
                                    .child(Input::new(&self.search_input)),
                            ))
                            .child(filter_column(
                                muted,
                                "",
                                checkbox("uncategorized-only")
                                    .label("Uncategorized only")
                                    .checked(self.uncategorized_only)
                                    .on_click(cx.listener(|this, on: &bool, window, cx| {
                                        this.set_uncategorized_only(*on, window, cx);
                                    })),
                            ))
                            .child(filter_column(
                                muted,
                                "",
                                checkbox("show-hidden")
                                    .label("Show hidden")
                                    .checked(self.show_hidden)
                                    .on_click(cx.listener(|this, on: &bool, window, cx| {
                                        this.set_show_hidden(*on, window, cx);
                                    })),
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
    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn render_imports(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        // The page exposes every file action (3.7): the native menu may be
        // absent (Linux), so Import, Undo, Export and Restore live here.
        let latest_undo_id = self.import_history.first().map(|e| e.id);
        let undo_label =
            if self.confirm_undo_import.is_some() && self.confirm_undo_import == latest_undo_id {
                "Click again to undo"
            } else {
                "Undo…"
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
                            .child("Imports"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("{} imports", self.import_history.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("pick-file")
                            .label("Import CSV…")
                            .primary()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_file(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("undo-last-import")
                            .label(undo_label)
                            .disabled(latest_undo_id.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.undo_last_import(window, cx);
                            })),
                    )
                    .child(
                        Button::new("export-data")
                            .label("Export…")
                            .on_click(cx.listener(|this, _, _, cx| this.export_data(cx))),
                    )
                    .child(
                        Button::new("restore-backup")
                            .label("Restore…")
                            .on_click(cx.listener(|this, _, _, cx| this.restore_backup(cx))),
                    ),
            )
            .children(self.import.clone())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.history_table).stripe(true)),
            )
            .into_any_element()
    }

    /// One clickable row of the category tree.
    fn tree_row(
        &self,
        node: &CategoryNode,
        depth: u8,
        selected: bool,
        palette: (Hsla, Hsla, Hsla),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (muted, accent, accent_foreground) = palette;
        let id = node.id;
        let count = self.node_count(node);
        let radius = cx.theme().radius;
        div()
            .id(SharedString::from(format!("cat-row-{id}")))
            .h_8()
            .px_2()
            .pl(px(8. + 16. * depth as f32))
            .flex()
            .items_center()
            .gap_2()
            .rounded(radius)
            .cursor_pointer()
            .when(selected, |d| d.bg(accent).text_color(accent_foreground))
            .child(
                div()
                    .when(depth == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(node.name.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if selected { accent_foreground } else { muted })
                    .child(flow::count(count)),
            )
            .when(node.is_system, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(if selected { accent_foreground } else { muted })
                        .child("system"),
                )
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select_category(id, window, cx);
            }))
            .into_any_element()
    }

    /// One row of the selected category's rule list (3.12: built lazily
    /// by the virtualized rule list, fixed height to match its
    /// `item_sizes`). 3.24: every row carries Edit and Delete… at its right
    /// end — small ghost buttons, visible on every row, built only for the
    /// rows the list renders.
    fn rule_row(&self, rule: &Rule, muted: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let target = self
            .child_choices
            .iter()
            .find(|(id, _)| *id == rule.category_id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("#{}", rule.category_id));
        let id = rule.id;
        let armed = self.armed_delete_rule == Some(id);
        h_flex()
            .h_6()
            .gap_2()
            .items_center()
            .debug_selector(move || format!("rule-row-{id}"))
            .child(
                div()
                    .w(px(320.))
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(rule.pattern.clone()),
            )
            .child(div().text_xs().text_color(muted).child(if rule.is_regex {
                "regex"
            } else {
                "substring"
            }))
            .child(div().text_color(muted).child("→"))
            .child(div().child(target))
            .child(div().flex_1())
            .child(
                Button::new(SharedString::from(format!("rule-edit-{id}")))
                    .label("Edit")
                    .ghost()
                    .text_xs()
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.start_edit_rule(id, window, cx)
                        }),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("rule-delete-{id}")))
                    .label(if armed {
                        "Click again to delete"
                    } else {
                        "Delete…"
                    })
                    .ghost()
                    .text_xs()
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.arm_delete_rule(id, window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }

    /// 3.24: the in-place editor that replaces one rule's row — the
    /// add-rule row's shape (labelled Pattern input, regex switch, labelled
    /// Category picker, primary Save, Cancel) pre-filled from the rule,
    /// with the same live "matches N transactions" count. A validation
    /// error shows in `danger` under the editor and keeps it open — clamped
    /// to one ellipsized line, because the virtual list doesn't clip per
    /// item and a taller row would paint over the rows below. The row
    /// reserves 68 px in `rule_sizes` instead of 24, 92 px while an error
    /// is open (measured: 55.5 px without, 82 px with one line).
    fn rule_editor_row(
        &self,
        rule: &Rule,
        muted: Hsla,
        danger: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preview = match self.rule_match_count {
            Some(n) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "rule-edit-preview".into())
                .child(format!("matches {} transactions", flow::count(n)))
                .into_any_element(),
            None => div().into_any_element(),
        };
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(field_label(muted, "Pattern"))
                            .child(div().w(px(280.)).child(Input::new(&self.rule_edit_pattern))),
                    )
                    .child(
                        Switch::new(SharedString::from(format!("rule-edit-regex-{}", rule.id)))
                            .label("regex")
                            .checked(self.rule_edit_regex)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.rule_edit_regex = *checked;
                                this.preview_rule_match(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(field_label(muted, "Category"))
                            .child(div().w(px(240.)).child(
                                Select::new(&self.rule_edit_target).placeholder("Category…"),
                            )),
                    )
                    .child(
                        Button::new(SharedString::from(format!("rule-edit-save-{}", rule.id)))
                            .label("Save")
                            .primary()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_rule_edit(window, cx)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("rule-edit-cancel-{}", rule.id)))
                            .label("Cancel")
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_edit_rule(cx))),
                    )
                    .child(preview),
            )
            .children(
                self.rule_error
                    .clone()
                    // One line, ellipsized (newlines flattened): the slot
                    // reserves room for exactly one error line, and the
                    // regex crate's messages are multi-line.
                    .map(|e| e.replace('\n', " "))
                    .map(|e| {
                        div()
                            .text_sm()
                            .text_color(danger)
                            .text_ellipsis()
                            .debug_selector(|| "rule-edit-error".into())
                            .child(e)
                    }),
            )
            .into_any_element()
    }

    fn render_categories(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, border, accent, accent_foreground, danger) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.border,
                theme.accent,
                theme.accent_foreground,
                theme.danger,
            )
        };
        let palette = (muted, accent, accent_foreground);

        // 3.12: the tree is virtualized (gpui-kit `v_virtual_list`). The
        // closure runs during layout with the visible range, so a tree of
        // thousands builds only the rows that can be on screen. Rows are
        // fixed height (`tree_row`'s `h_8` = 32 px); the panel's `gap_1`
        // is added to the stride by the list.
        let n_tree_rows: usize = self.categories.iter().map(|p| 1 + p.children.len()).sum();
        let tree_sizes = Rc::new(vec![size(px(0.), px(32.)); n_tree_rows]);
        // `h_flex` centers children on the cross axis (unlike `v_flex`), so
        // each column needs `h_full` for the virtual list inside it to
        // resolve a real height; without it the lists collapse to 0 px and
        // the page opens blank (3.18).
        let tree = v_flex()
            .w(px(300.))
            .flex_shrink_0()
            .h_full()
            .gap_1()
            .min_h_0()
            .child(
                div()
                    .px_2()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(region_title("Category tree")),
            )
            .child(
                v_virtual_list(
                    cx.entity(),
                    "category-tree",
                    tree_sizes,
                    move |this, range, _window, cx| {
                        let mut rows = Vec::new();
                        let mut ix = 0usize;
                        for parent in &this.categories {
                            if ix >= range.end {
                                break;
                            }
                            if ix >= range.start {
                                let selected = this.selected_category == Some(parent.id);
                                rows.push(this.tree_row(parent, 0, selected, palette, cx));
                            }
                            ix += 1;
                            for child in &parent.children {
                                if ix >= range.end {
                                    break;
                                }
                                if ix >= range.start {
                                    let selected = this.selected_category == Some(child.id);
                                    rows.push(this.tree_row(child, 1, selected, palette, cx));
                                }
                                ix += 1;
                            }
                        }
                        #[cfg(test)]
                        this.tree_rows_built.set(rows.len());
                        #[cfg(test)]
                        this.tree_range.set((range.start, range.end));
                        rows
                    },
                )
                // 3.21: track the Shell-owned handle, not the fresh one
                // `v_virtual_list` creates per render — that one resets the
                // offset to 0 every frame.
                .track_scroll(&self.tree_scroll)
                .gap_1()
                .flex_1()
                .min_h_0(),
            );

        // Detail panel for the selected category.
        let detail: AnyElement = match self.selected_node() {
            None => div()
                .text_sm()
                .text_color(muted)
                .child("Select a category to rename, move or delete it.")
                .into_any_element(),
            Some((node, parent)) => {
                let count = self.node_count(node);
                let name = node.name.clone();
                let kind = node.kind;
                let is_system = node.is_system;
                let is_child = parent.is_some();
                let renaming = self.renaming;
                let confirm_label = if self.confirm_delete_category == Some(node.id) {
                    "Click again to delete"
                } else {
                    "Delete…"
                };
                v_flex()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .child(div().text_sm().text_color(muted).child(format!(
                                "{} · {} transactions",
                                kind_label(kind),
                                flow::count(count)
                            ))),
                    )
                    .child(if renaming {
                        // Rename editor (3.16): the input carries a label.
                        h_flex()
                            .gap_2()
                            .items_end()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(field_label(muted, "New name"))
                                    .child(div().w(px(260.)).child(Input::new(&self.rename_input))),
                            )
                            .child(Button::new("save-rename").label("Save").primary().on_click(
                                cx.listener(|this, _, window, cx| this.save_rename(window, cx)),
                            ))
                            .child(Button::new("cancel-rename").label("Cancel").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.renaming = false;
                                    cx.notify();
                                }),
                            ))
                            .into_any_element()
                    } else {
                        // Detail actions (3.16): each picker or field sits
                        // in its own labelled group beside its button,
                        // instead of a row of unlabelled boxes.
                        h_flex()
                            .flex_wrap()
                            .gap_3()
                            .items_end()
                            .when(!is_child, |row| {
                                row.child(
                                    v_flex()
                                        .gap_1()
                                        .child(field_label(muted, "New child name"))
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_end()
                                                .child(
                                                    div()
                                                        .w(px(220.))
                                                        .child(Input::new(&self.new_child_name)),
                                                )
                                                .child(
                                                    Button::new("add-child")
                                                        .label("Add child")
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.add_child(window, cx)
                                                            },
                                                        )),
                                                ),
                                        ),
                                )
                            })
                            .child(
                                Button::new("rename-category")
                                    .label("Rename…")
                                    .disabled(is_system)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_rename(window, cx)
                                    })),
                            )
                            .when(is_child, |row| {
                                row.child(
                                    v_flex()
                                        .gap_1()
                                        .child(field_label(muted, "Move to parent"))
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_end()
                                                .child(
                                                    div().w(px(200.)).child(
                                                        Select::new(&self.move_target)
                                                            .placeholder("Move to parent…"),
                                                    ),
                                                )
                                                .child(
                                                    Button::new("move-child")
                                                        .label("Move")
                                                        .disabled(is_system)
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.move_child_to_selected_parent(
                                                                    window, cx,
                                                                )
                                                            },
                                                        )),
                                                ),
                                        ),
                                )
                            })
                            .when(!is_system, |row| {
                                row.child(
                                    v_flex()
                                        .gap_1()
                                        .child(field_label(muted, "Move its transactions to"))
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_end()
                                                .child(
                                                    div().w(px(260.)).child(
                                                        Select::new(&self.delete_target)
                                                            .placeholder(
                                                                "Move its transactions to…",
                                                            ),
                                                    ),
                                                )
                                                .child(
                                                    Button::new("delete-category")
                                                        .label(confirm_label)
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.arm_delete_category(window, cx)
                                                            },
                                                        )),
                                                ),
                                        ),
                                )
                            })
                            .into_any_element()
                    })
                    .into_any_element()
            }
        };

        // Rules: list + add row with the live match preview. The list is
        // virtualized like the tree (3.12); rows are fixed height (24 px),
        // except the one row 3.24's editor swapped in for, which reserves
        // 68 px (labelled fields) — 92 px while a validation error is open
        // under it, one ellipsized line (`rule_editor_row`).
        let editing = self.editing_rule;
        let edit_error = self.rule_error.is_some();
        let rule_sizes = Rc::new(
            self.rules
                .iter()
                .map(|r| {
                    if Some(r.id) == editing {
                        size(px(0.), if edit_error { px(92.) } else { px(68.) })
                    } else {
                        size(px(0.), px(24.))
                    }
                })
                .collect::<Vec<_>>(),
        );

        let preview = match self
            .rule_match_count
            .filter(|_| self.editing_rule.is_none())
        {
            Some(n) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "add-rule-preview".into())
                .child(format!("matches {} transactions", flow::count(n)))
                .into_any_element(),
            None => div().into_any_element(),
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
                            .child("Categories"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("{} parents", self.categories.len())),
                    ),
            )
            .children(
                self.cat_error
                    .clone()
                    .map(|e| div().text_sm().text_color(danger).child(e)),
            )
            // New-parent group (3.16): labelled fields, one deliberate row.
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(field_label(muted, "New parent name"))
                            .child(div().w(px(220.)).child(Input::new(&self.new_parent_name))),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(field_label(muted, "Kind"))
                            .child(div().w(px(140.)).child(Select::new(&self.new_parent_kind))),
                    )
                    .child(
                        Button::new("add-parent")
                            .label("Add parent")
                            .primary()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_parent(window, cx)),
                            ),
                    ),
            )
            .child(
                h_flex().flex_1().min_h_0().gap_6().child(tree).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .min_h_0()
                        .gap_4()
                        .child(detail)
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                .gap_2()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            div().font_weight(FontWeight::SEMIBOLD).child("Rules"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(self.rules.len().to_string()),
                                        )
                                        // 3.24: label over the Edit/Delete…
                                        // column at the rows' right end.
                                        .child(div().flex_1())
                                        .child(div().text_xs().text_color(muted).child("Actions")),
                                )
                                // Add-rule group (3.16): the pattern field
                                // and the category picker carry labels.
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_end()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(muted, "Pattern"))
                                                .child(
                                                    div()
                                                        .w(px(280.))
                                                        .child(Input::new(&self.rule_pattern)),
                                                ),
                                        )
                                        .child(
                                            Switch::new("rule-regex")
                                                .label("regex")
                                                .checked(self.rule_regex)
                                                .on_click(cx.listener(
                                                    |this, checked: &bool, _, cx| {
                                                        this.rule_regex = *checked;
                                                        this.preview_rule_match(cx);
                                                        cx.notify();
                                                    },
                                                )),
                                        )
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(muted, "Category"))
                                                .child(
                                                    div().w(px(240.)).child(
                                                        Select::new(&self.rule_target)
                                                            .placeholder("Category…"),
                                                    ),
                                                ),
                                        )
                                        .child(
                                            Button::new("add-rule")
                                                .label("Add rule")
                                                .primary()
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.add_rule(window, cx)
                                                })),
                                        )
                                        .child(preview),
                                )
                                .children(
                                    self.rule_error
                                        .clone()
                                        // 3.24: while a row's editor is open
                                        // it shows the error under itself.
                                        .filter(|_| self.editing_rule.is_none())
                                        .map(|e| div().text_sm().text_color(danger).child(e)),
                                )
                                .child(
                                    v_virtual_list(
                                        cx.entity(),
                                        "rule-list",
                                        rule_sizes,
                                        move |this, range, _window, cx| {
                                            let rows: Vec<AnyElement> = this
                                                .rules
                                                .iter()
                                                .skip(range.start)
                                                .take(range.len())
                                                .map(|r| {
                                                    if this.editing_rule == Some(r.id) {
                                                        this.rule_editor_row(r, muted, danger, cx)
                                                    } else {
                                                        this.rule_row(r, muted, cx)
                                                    }
                                                })
                                                .collect();
                                            #[cfg(test)]
                                            this.rule_rows_built.set(rows.len());
                                            #[cfg(test)]
                                            this.rule_range.set((range.start, range.end));
                                            rows
                                        },
                                    )
                                    // 3.21: Shell-owned handle (see the tree).
                                    .track_scroll(&self.rule_scroll)
                                    .gap_1()
                                    .flex_1()
                                    .min_h_0(),
                                ),
                        ),
                ),
            )
            .into_any_element()
    }

    fn render_queue(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, danger) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.danger)
        };
        let body: AnyElement = if self.queue.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child("Nothing waiting — every transaction has a category.")
                .into_any_element()
        } else {
            // 3.19: virtualized like the Categories-page lists (3.12). The
            // closure runs during layout with the visible range, so a queue
            // of thousands builds only the rows that can be on screen, and
            // `queue_picker` creates a picker only with its row. Rows are a
            // fixed 40 px (32 px controls with the old `py_1` of spacing);
            // the list's `gap_1` is added to the stride by the list.
            let sizes = Rc::new(vec![size(px(0.), px(40.)); self.queue.len()]);
            // 3.16: a heading row labels every column, the category picker
            // included — its placeholder alone is not a label. The children
            // mirror the queue rows below in order and width, down to an
            // invisible copy of the Categorize button, so each heading sits
            // over its column (the picker's heading would otherwise slide
            // right by the button's width). 3.22: the widths come from the
            // QUEUE_*_W consts and Merchant is the only flex column, so the
            // whole row fits the minimum window and the headings stay put.
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_3()
                        .child(
                            field_label(muted, "Merchant")
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis(),
                        )
                        .child(
                            field_label(muted, "Transactions")
                                .w(px(QUEUE_TXNS_W))
                                .text_right(),
                        )
                        .child(
                            field_label(muted, "Total")
                                .w(px(QUEUE_TOTAL_W))
                                .text_right(),
                        )
                        .child(field_label(muted, "Dates").w(px(QUEUE_DATES_W)))
                        .child(field_label(muted, "Category").w(px(QUEUE_PICKER_W)))
                        .child(
                            Button::new("queue-heading-categorize-spacer")
                                .label("Categorize")
                                .primary()
                                .opacity(0.0)
                                .flex_shrink_0(),
                        ),
                )
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "queue-list",
                        sizes,
                        move |this, range, window, cx| {
                            #[cfg(test)]
                            let range_bounds = (range.start, range.end);
                            let groups = this.queue[range].to_vec();
                            let mut rows = Vec::with_capacity(groups.len());
                            for g in groups {
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
                                        .gap_3()
                                        .items_center()
                                        .child(
                                            // 3.22: Merchant is the flex
                                            // column; a name too long for
                                            // the leftover space truncates,
                                            // and the tooltip still shows
                                            // the full one.
                                            div()
                                                .id(SharedString::from(format!(
                                                    "queue-merchant-{merchant}"
                                                )))
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .font_weight(FontWeight::SEMIBOLD)
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
                                                .child(
                                                    Button::new(SharedString::from(format!(
                                                        "categorize-{}",
                                                        g.merchant
                                                    )))
                                                    .label("Categorize")
                                                    .primary()
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.categorize_merchant(
                                                                merchant.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    )),
                                                ),
                                        ),
                                );
                            }
                            #[cfg(test)]
                            this.queue_rows_built.set(rows.len());
                            #[cfg(test)]
                            this.queue_range.set(range_bounds);
                            rows
                        },
                    )
                    // 3.21: Shell-owned handle (see the tree).
                    .track_scroll(&self.queue_scroll)
                    .gap_1()
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
                    .child(div().text_sm().text_color(muted).child(format!(
                        "{} uncategorized",
                        flow::count(self.uncategorized_visible)
                    ))),
            )
            .children(
                self.queue_error
                    .clone()
                    .map(|e| div().text_sm().text_color(danger).child(e)),
            )
            .child(body)
            .into_any_element()
    }

    /// The Transfers page (4.2): one bordered panel per open review — the
    /// row under review, then every candidate with a *Pair* button, then a
    /// *Not a transfer* opt-out. No banner, no sheet (ui-conventions,
    /// *Pages*, D8); the sidebar pill carries the count.
    fn render_transfers(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, border, radius) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.border, theme.radius)
        };
        let reviews = self.transfer_reviews.clone();
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
                        .debug_selector(move || format!("transfer-review:{txn}"))
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
                                .debug_selector(move || format!("transfer-candidate:{txn}:{cand}"))
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
                        flow::count(self.transfer_reviews.len() as i64)
                    ))),
            )
            .child(body)
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (danger, muted, background, foreground) = {
            let theme = cx.theme();
            (
                theme.danger,
                theme.muted_foreground,
                theme.background,
                theme.foreground,
            )
        };

        let page_area: AnyElement = match self.page {
            Page::Dashboard => self.render_dashboard(cx),
            Page::ToCategorize => self.render_queue(cx),
            Page::Categories => self.render_categories(cx),
            Page::Transfers => self.render_transfers(cx),
            Page::Imports => self.render_imports(cx),
        };
        // 3.11: key the page area by the page. A switch to another page
        // lands on a fresh element id, so GPUI cannot reuse the previous
        // page's painted subtree or its element state under the new page.
        // The debug selector (`page:<id>`, test builds only) lets headless
        // tests assert exactly one page paints after a switch.
        let page_area = v_flex()
            .id(self.page.content_id())
            .debug_selector(|| format!("page:{}", self.page.content_id()))
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(page_area)
            .into_any_element();

        h_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            // Esc clears the dashboard category selection (3.5); the binding
            // is registered in `Shell::new`.
            .on_action(
                cx.listener(|this, action: &ClearCategorySelection, window, cx| {
                    this.clear_category_selection(action, window, cx);
                }),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(path) = paths.paths().first() {
                    this.open_file(path.clone(), window, cx);
                }
            }))
            .child(self.sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .size_full()
                    .gap_3()
                    .p_4()
                    .children(self.status.clone().map(|(msg, is_error)| {
                        div()
                            .text_sm()
                            .text_color(if is_error { danger } else { muted })
                            .child(msg)
                    }))
                    .child(page_area),
            )
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
