//! GPUI views. Thin by design: logic lives in `flow` and `financi-db`, and
//! every database call goes through the [`DbWorker`] thread.
//!
//! The window is a [`Sidebar`] of pages and one page area, per
//! `docs/ui-conventions.md` (*Window layout*).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use financi_core::CategoryKind;
use financi_core::dates::{DatePreset, preset_range};
use financi_core::rules::{Rule, pattern_matches, winning_rule};
use financi_db::categories::{
    CategoryNode, create_child, create_parent, delete_category, list_categories, move_child,
    rename as rename_category, transaction_counts,
};
use financi_db::import::{
    CommitOptions, ImportEntry, TransactionRow, commit_import, list_imports, plan_import,
    undo_last_import,
};
use financi_db::queue::{UncategorizedGroup, categorize_group, uncategorized_by_merchant};
use financi_db::rules::{
    add_rule, count_rule_matches, delete_rule, list_rules, move_rules, rule_use_stats, update_rule,
};
use financi_db::settings::{self, DateRange};
use financi_db::{
    Account, CategorySpend, DbWorker, MonthlySummary, TransferReview, TxSort, TxSortColumn,
    TxTableFilter, ViewFilter, apply_rule_from_merchant, create_account, list_accounts,
    list_transactions_filtered, list_transfer_reviews, monthly_summary, resolve_transfer_review,
    rusqlite::Connection, set_category, set_hidden, sidebar_counts, spend_by_category,
    unpair_transaction,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{
    Scrollbar, ScrollbarMode, Toast, ToastManager, ToastMotion, ToastOptions, ToastStack,
    ToastStackState,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::chart::BarChart;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::sidebar::{Sidebar, SidebarItem, SidebarMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Disableable as _, Icon, Sizable as _, VirtualListScrollHandle,
    h_flex, v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::flow::{self, LoadedFile, PlanSummary};

type Names = Vec<SharedString>;

/// 3.53: one subcategory entry of the Categories page's parent-scoped
/// add-rule picker. Plain text as shown, plus debug selectors on the open
/// menu's row so headless tests can measure the drawn text against the
/// menu's width: the visible box fills the row, and an absolutely
/// positioned invisible copy keeps its natural (unclipped) text width —
/// the two bound each other exactly when the name fits.
#[derive(Clone, PartialEq)]
struct RuleChoice(SharedString);

impl SearchableListItem for RuleChoice {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.0.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.0
    }

    fn render(&self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let full = self.0.clone();
        let probe = self.0.clone();
        div()
            .id("rule-choice")
            .debug_selector(move || format!("rule-choice:{full}"))
            .w_full()
            .min_w_0()
            .relative()
            .child(self.0.clone())
            .child(
                div()
                    .id("rule-choice-natural")
                    .debug_selector({
                        let probe = probe.clone();
                        move || format!("rule-choice-natural:{probe}")
                    })
                    .absolute()
                    .whitespace_nowrap()
                    .opacity(0.)
                    .child(probe),
            )
    }
}

/// 3.53: the parent-scoped picker's delegate. `SearchableVec` (not a
/// plain `Vec`) so the menu's search box actually narrows the list as you
/// type, like every other picker that can hold long names (3.59).
type RuleChoices = SearchableVec<RuleChoice>;

fn rule_choices(names: &[(i64, String)]) -> RuleChoices {
    SearchableVec::new(
        names
            .iter()
            .map(|(_, n)| RuleChoice(SharedString::from(n.clone())))
            .collect::<Vec<_>>(),
    )
}

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

/// 3.34: the To-categorize panel's column headings — like `field_label`
/// but `text_sm` (a table heading row, not a form field), keeping the same
/// `label:<text>` debug selector so headless tests still find them.
fn heading_label(muted: Hsla, text: &'static str) -> Div {
    div()
        .text_sm()
        .text_color(muted)
        .debug_selector(move || format!("label:{text}"))
        .child(text)
}

/// 3.53: the Categories page's parent-scoped add-rule picker (the chosen
/// parent's subcategories by their own names). `trigger_w` fits the
/// longest child name up to 200 px — longer names ellipsize, and the
/// trigger carries a hover tooltip with the full name (ui-conventions
/// *Interaction*) — and the open menu is `menu_w` wide (at least the
/// longest name, max [`RULE_CATEGORY_MENU_W`]), so no item is cut off.
/// `id` tells the rows apart (`rule:subcategory-select:<id>`).
fn subcategory_select(
    target: &Entity<SelectState<RuleChoices>>,
    id: &'static str,
    trigger_w: f32,
    menu_w: f32,
    cx: &App,
) -> Stateful<Div> {
    let state = target.clone();
    let wrapper = div()
        .id(SharedString::from(id))
        .w(px(trigger_w))
        .flex_shrink_0()
        .debug_selector(move || format!("rule:subcategory-select:{id}"));
    let tip = target.read(cx).selected_value().cloned();
    let wrapper = match tip {
        Some(fallback) => wrapper.tooltip(move |window, cx| {
            let shown = state
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            let for_text = shown.clone();
            Tooltip::element(move |_, _| {
                div()
                    .debug_selector({
                        let for_sel = shown.clone();
                        move || format!("rule:category-tooltip:{for_sel}")
                    })
                    .child(for_text.clone())
            })
            .build(window, cx)
        }),
        None => wrapper,
    };
    wrapper.child(
        div()
            .debug_selector(move || format!("label:{SUBCATEGORY_PH}"))
            .child(
                Select::new(target)
                    .menu_width(px(menu_w))
                    .placeholder(SUBCATEGORY_PH),
            ),
    )
}

/// 3.53: the Select chrome beside the label text — horizontal padding
/// plus the chevron — generous so the width estimates below err towards
/// controls that are too wide rather than names that clip (same spirit
/// as [`TEXT_SM_CHAR_PX`]).
const SELECT_CHROME_W: f32 = 40.;
/// 3.53: floor for the parent-scoped picker: its placeholder
/// ("Subcategory…") must fit whatever the child names are.
const SUBCATEGORY_MIN_W: f32 = 130.;

/// 3.53: widths for the parent-scoped picker from the longest child
/// name's character count — the trigger up to [`RULE_CATEGORY_W`] (names
/// past that ellipsize with a tooltip), the menu up to
/// [`RULE_CATEGORY_MENU_W`] and never narrower than the trigger.
/// Estimated, never measured: the app has no text-measurement path here
/// (the Undo column does the same, see [`TEXT_SM_CHAR_PX`]).
fn subcategory_widths(longest_chars: usize) -> (f32, f32) {
    let est = longest_chars as f32 * TEXT_SM_CHAR_PX + SELECT_CHROME_W;
    let trigger = est.clamp(SUBCATEGORY_MIN_W, RULE_CATEGORY_W);
    (trigger, est.max(trigger).min(RULE_CATEGORY_MENU_W))
}

/// 3.42 (ui-conventions *Forms*): an unlabelled control in a labelled
/// `items_end` row — the regex switch beside Pattern and Category — goes
/// in a slot the height of one control (32 px), centred, so its centre
/// matches the controls beside it while the row's bottoms stay aligned.
/// The debug selectors record the slot (`<selector>`) and, in a
/// shrink-wrap probe div (the pattern `RuleChoice::render` uses), the
/// control itself (`<probe>`) — the Switch widget carries no selector,
/// so without the probe a test could only ever measure the slot.
fn control_slot(selector: String, probe: String, control: impl IntoElement) -> Div {
    div()
        .h(px(32.))
        .flex()
        .items_center()
        .flex_shrink_0()
        .debug_selector(move || selector.clone())
        .child(
            div()
                .flex_shrink_0()
                .debug_selector(move || probe.clone())
                .child(control),
        )
}

/// Words for Categories page controls whose painted text is reachable by
/// tests only through debug selectors (3.33): each const feeds both the
/// label/placeholder and its `label:<text>` selector, so the two can't
/// desync and the scan test pins the exact words.
const ADD_CATEGORY: &str = "Add category";
const ADD_SUBCATEGORY: &str = "Add subcategory";
const MOVE_TO_CATEGORY_PH: &str = "Move to category…";
const MOVE_TXNS_TO_PH: &str = "Move its transactions to…";
/// 3.53: the parent-scoped add-rule picker's placeholder; like the Move
/// pickers, its text is pinned by a `label:<text>` selector on the
/// control's wrapper.
const SUBCATEGORY_PH: &str = "Subcategory…";
/// 3.23: the Categories page's header action; it opens the new-category
/// form in place under the header (ui-conventions, *Pages*).
const NEW_CATEGORY: &str = "New category…";
/// 3.28: the Rules page's header action; it opens the add-rule row in
/// place under the header (ui-conventions, *Pages*).
const NEW_RULE: &str = "New rule…";
/// 3.28: the Move bar's target picker (moving checked rules between
/// categories).
const MOVE_RULES_TO_PH: &str = "Move rules to…";

/// 3.23: the Categories rule list's fixed columns — the row and its column
/// headings share these widths so the headings sit over their columns
/// (same rule as the queue's `QUEUE_*_W`). Pattern takes the remaining
/// width (`flex_1`) and truncates with a tooltip.
const RULE_TYPE_W: f32 = 80.;
const RULE_CATEGORY_W: f32 = 200.;
/// 3.53: the parent-scoped add-rule picker's open-menu ceiling — wide
/// enough for a long subcategory name where the trigger has to ellipsize
/// (a 40-character name fits, per the menu test).
const RULE_CATEGORY_MENU_W: f32 = 320.;
/// 3.28: the Rules page list's fixed columns beside the flexed Pattern: a
/// checkbox column, and Uses (right-aligned). Same sharing rule as
/// [`RULE_TYPE_W`]: row and column headings use these.
const RULE_SEL_W: f32 = 24.;
const RULE_USES_W: f32 = 80.;

/// 3.28: one entry of the Rules page virtual list — a group header row or a
/// rule row. Group headers are items too, so collapsing a group just drops
/// its rows and the list stays virtualized over everything (D5).
enum RulesItem {
    Group {
        /// Child-category id; also the collapse key.
        id: i64,
        /// "Parent › Child", from `child_choices`.
        label: String,
        /// Rules shown in this group (after the page filters).
        count: usize,
        collapsed: bool,
    },
    Rule {
        id: i64,
    },
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

/// Hover tooltip naming a dashboard filter control that has no visible
/// label (3.35, ui-conventions *Interaction*). The tooltip body carries the
/// `filters:tooltip:<name>` selector so headless tests can see it appear.
fn filter_tip(id: &'static str, name: &'static str, control: impl IntoElement) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .tooltip(move |window, cx| {
            Tooltip::element(move |_, _| {
                div()
                    .debug_selector(move || format!("filters:tooltip:{name}"))
                    .child(name)
            })
            .build(window, cx)
        })
        .child(control)
}

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
    /// 3.28: find and organise the rules (D5).
    Rules,
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
            Page::Rules => "page-rules",
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
/// Called with the deciding rule's id from the *Show rule* menu item (3.29).
type ShowRuleFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

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
    rows: Vec<TransactionRow>,
    /// Child categories as `(id, "Parent › Child")` for the *Change category…*
    /// submenu, mirroring the shell's `child_choices`.
    child_choices: Vec<(i64, String)>,
    /// The row that just got a manual category and now offers to apply it to
    /// every matching row (2.6's rule offer, 2.9). Its transaction id.
    rule_offer: Option<i64>,
    /// The shell's rules, kept in sync on every refresh: the *Show rule*
    /// item (3.29) runs [`winning_rule`] over them when the menu opens.
    rules: Vec<Rule>,
    /// The id of the system `Transfers > Card Payment` category (3.29), the
    /// same lookup `financi-db` uses; `None` only before the first refresh.
    card_payment_category: Option<i64>,
    /// The table's current ordering, kept in sync with the shell so the
    /// header's sort icon reflects it after every refresh (3.4).
    sort: TxSort,
    on_set_category: SetCategoryFn,
    on_set_hidden: SetHiddenFn,
    on_apply_rule: ApplyRuleFn,
    /// Called with a paired row's id from the *Unpair* menu item (4.2).
    on_unpair: UnpairFn,
    /// Called with the deciding rule's id from the *Show rule* menu item (3.29).
    on_show_rule: ShowRuleFn,
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
        cx: &mut Context<TableState<Self>>,
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
            // The Undo column (3.32): a `h_full` marker spans the cell's
            // content box on every row, so tests can pin the button's
            // vertical centring and that the newest row is no taller than
            // the others; the cell itself centers it.
            let cell = div().h_full().flex().items_center();
            // Undo only on the newest row (ui-conventions, Pages).
            if row_ix == 0 {
                let id = entry.id;
                let armed = self.confirm_id == Some(id);
                let on_undo = self.on_undo.clone();
                // xsmall ghost button: the undo glyph alone at rest;
                // armed, it grows the prompt in danger text.
                let btn = Button::new("undo-import")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Undo2)
                    .debug_selector(|| "history:undo-btn".to_string())
                    .on_click(move |_, window, cx| on_undo(id, window, cx));
                let btn = if armed {
                    btn.label(UNDO_LABEL_MAX)
                        .text_color(cx.theme().danger)
                        .into_any_element()
                } else {
                    btn.into_any_element()
                };
                return cell
                    .id(SharedString::from("undo-import-tip"))
                    .debug_selector(|| "history:undo-cell:0".to_string())
                    .tooltip(|window, cx| {
                        Tooltip::element(|_, _| {
                            div()
                                .debug_selector(|| "history:undo-tooltip".to_string())
                                .child("Undo this import")
                        })
                        .build(window, cx)
                    })
                    .child(btn)
                    .into_any_element();
            }
            return cell
                .debug_selector(move || format!("history:undo-cell:{row_ix}"))
                .into_any_element();
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
/// label (3.15), produced by [`short_chart_labels`]; `bar_label` is the
/// share drawn above the bar (3.31), empty when the bar is thinned out
/// of the label plan by [`bar_label_plan`].
#[derive(Clone)]
struct ChartRow {
    id: Option<i64>,
    name: SharedString,
    short: SharedString,
    bar_label: SharedString,
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
/// The legend column inside the chart panel (3.31): just swatch and name,
/// so it no longer carries the amount and share columns and the plot gets
/// the width back.
const LEGEND_W: f32 = 140.;
const CHART_PANEL_SHARE: f32 = 0.45;
const PAGE_PAD: f32 = 16.;
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
// `Rows added` and `Dates` keep their widths, `Undo` is sized so the
// armed button — its icon and its widest label, "Click again to undo" —
// shows in full (3.32), and `File` and `Account` share whatever width is
// left — they truncate with an ellipsis as the window narrows and grow
// with it, so the table fits the page from the minimum window size up.
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
/// Horizontal padding inside an xsmall `Button`: `px_1` per side.
const BUTTON_PAD_X: f32 = 8.;
/// The in-row Undo button's icon plus the gap between icon and label:
/// xsmall renders its icon at `size_3` (12 px) with a `gap_1` (4 px)
/// before the label (3.32).
const UNDO_ICON_W: f32 = 16.;
/// Horizontal padding inside a table cell (8 px per side); gpui-kit
/// draws the cell inside the column width, so it comes off the space
/// the label has.
const CELL_PAD_X: f32 = 16.;
/// The table's 1 px border on each side; the page width includes it.
const TABLE_BORDER_X: f32 = 2.;

/// The Undo column's width: the longest label at `TEXT_SM_CHAR_PX`,
/// plus the button's icon and horizontal padding and the cell's.
fn history_undo_col_w() -> f32 {
    (UNDO_LABEL_MAX.chars().count() as f32 * TEXT_SM_CHAR_PX
        + BUTTON_PAD_X
        + UNDO_ICON_W
        + CELL_PAD_X)
        .ceil()
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
const BAR_LABEL_MAX_CHARS: usize = 4;

/// A bar's share label: whole percent (`18%`), one decimal only below
/// one percent (`0.4%`) so a thin slice still reads — and never `0.0%`:
/// a sliver too small to round to a tenth of a percent reads `0.1%`.
fn bar_share_label(share: f64) -> String {
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
fn chart_tooltip_value(cents: i64, share: f64, parent: &str) -> String {
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
fn bar_label_plan(chart_width: f32, count: usize) -> usize {
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
fn bar_label_slots(count: usize, labels: usize) -> Vec<bool> {
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

/// 3.27: how long a success/info toast stays up before dismissing itself.
/// Errors have no timeout; they stay until the user closes them
/// (`docs/ui-conventions.md`, *Feedback*).
const TOAST_TIMEOUT: Duration = Duration::from_secs(4);
/// How often the toast ticker advances the manager's lifecycle clock while
/// any toast is mounted. Coarser than a frame on purpose: the stack's
/// motion is spring-driven, and the clock only gates enter/timeout/exit.
const TOAST_TICK: Duration = Duration::from_millis(100);
/// Width of one toast card; wide enough for the longest message the app
/// writes on one or two lines.
const TOAST_W: f32 = 360.;

/// 3.27: one toast in the manager — its text and whether it is an error
/// (error text uses `danger` and the toast persists until dismissed).
struct ToastMessage {
    message: SharedString,
    is_error: bool,
}

/// 3.40: the rule-editor button currently waiting on the DB (Save on a
/// rule edit, Add rule, or a row's Delete…). While one is pending that
/// button renders `.loading(true)` and disabled, so a slow save can't be
/// double-submitted; the task clears it when it finishes, on success or
/// error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PendingRuleAction {
    Add,
    Save(i64),
    Delete(i64),
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
    /// One horizontal scroll handle for the whole month strip of the
    /// dashboard summary (3.30): the header's month row and every row's
    /// month cells track it, so they scroll together, while the name
    /// column and the `Total` column stay pinned.
    summary_scroll: ScrollHandle,
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
    // -- Feedback (3.27): toasts, never inline status lines
    // (`docs/ui-conventions.md`, *Feedback*). The manager owns the mounted
    // toasts, their per-toast timeouts and their lifecycle; it is advanced
    // by a ticker task that runs only while something is mounted, against
    // `toast_clock` — a virtual clock the ticker owns (see `tick_toasts`).
    toasts: ToastManager<u64, ToastMessage>,
    toast_stack: ToastStackState,
    toast_next_id: u64,
    toast_clock: Instant,
    toast_task: Option<Task<()>>,
    refresh_task: Option<Task<()>>,
    task: Option<Task<()>>,
    // -- Categories page
    categories: Vec<CategoryNode>,
    cat_counts: HashMap<i64, i64>,
    selected_category: Option<i64>,
    new_parent_name: Entity<InputState>,
    new_parent_kind: Entity<SelectState<Names>>,
    new_child_name: Entity<InputState>,
    /// 3.23: the New category… form under the page header is open. The
    /// header button opens it, Cancel and a successful add close it, and
    /// `go_page` clears it like the page's other transient UI.
    new_category_open: bool,
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
    /// 3.53: the Categories page's add-rule picker, shown only when the
    /// selection is a parent: its own child categories by their own names
    /// (no "Parent › Child"), searchable, the wide menu and the
    /// full-name tooltip. `cat_rule_child_ids` maps the picker's rows to
    /// category ids (the items carry names only).
    cat_rule_target: Entity<SelectState<RuleChoices>>,
    cat_rule_child_ids: Vec<i64>,
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
    /// 3.40: which rule button is waiting on the DB (see
    /// [`PendingRuleAction`]); rendered loading + disabled while set.
    pending_rule_action: Option<PendingRuleAction>,
    /// 3.24: the editor's own controls, separate entities from the add-rule
    /// row's so the two never share an `InputState` on screen at once.
    /// While the editor is open the shared `rule_match_count`
    /// preview belongs to it (the add-rule row hides it, like `rule_error`)
    /// and excludes that rule (2.5 re-run semantics); every close path
    /// clears it, and `sync_category_choices` closes the editor when its
    /// rule is gone.
    /// 3.53: no picker — Edit changes pattern and kind only, and keeps the
    /// rule's category; moving is the Rules page Move bar's job (3.28).
    rule_edit_pattern: Entity<InputState>,
    rule_edit_regex: bool,
    /// 3.28: the Rules page search box; live filter over the list (pattern
    /// and target category name, case-insensitive), focused when the page
    /// opens (D5). In "Test a description" mode it takes the description.
    rules_search: Entity<InputState>,
    /// 3.28: the "Test a description" switch (D5): the list shows only the
    /// rules matching the text in the search box, winner first.
    rules_test_mode: bool,
    /// 3.28: Type picker over the list (row 0 = All, 1 = Substring, 2 = Regex;
    /// nothing selected reads as All).
    rules_type_select: Entity<SelectState<Names>>,
    /// 3.28: the "Unused" checkbox: only show rules deciding zero
    /// transactions.
    rules_unused_only: bool,
    /// 3.28: child-category ids whose group header row is collapsed.
    collapsed_rule_groups: HashSet<i64>,
    /// 3.28: sort rows within a group by Uses (descending) instead of by
    /// Pattern (the default, ascending).
    rules_sort_by_uses: bool,
    /// 3.28: rules whose row checkbox is ticked; the Move bar acts on these.
    selected_rules: HashSet<i64>,
    /// 3.28: the Move bar's target picker (child categories only, like the
    /// rule target pickers).
    move_rules_target: Entity<SelectState<Names>>,
    /// 3.28: the Move bar's error line (a move the DB refused).
    rules_move_error: Option<SharedString>,
    /// 3.28: how many unlocked rows each rule's match decides (the Uses
    /// column); rules deciding nothing are absent.
    rule_uses: HashMap<i64, usize>,
    /// 3.28: whether the add-rule row is open on the Rules page.
    new_rule_open: bool,
    /// 3.28: set when the Rules page opens; focuses the search box on the
    /// next render (D5).
    rules_focus_pending: std::cell::Cell<bool>,
    /// 3.29: the rule the dashboard's *Show rule* item just landed on; its
    /// row carries the `list_active` background until the user searches or
    /// leaves the page (transient, like the page's other one-shot state).
    rules_reveal: Option<i64>,
    /// 3.29: the id of the system `Transfers > Card Payment` category, read
    /// off the category tree on every refresh (same name lookup the DB uses
    /// in [`financi_db::categories::card_payment_category`]). `None` only
    /// before the first refresh.
    card_payment_category: Option<i64>,
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
    /// 3.28: the Rules page list (headers and rows together).
    rules_scroll: VirtualListScrollHandle,
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
    /// 3.28: how many items the Rules page list built last frame (headers
    /// and rows together) and the range it drew, for the long-list tests.
    #[cfg(test)]
    rules_page_rows_built: std::cell::Cell<usize>,
    #[cfg(test)]
    rules_page_range: std::cell::Cell<(usize, usize)>,
    // -- To categorize queue
    queue: Vec<UncategorizedGroup>,
    /// 3.19: pickers exist only for rows `render_queue` has built, so this
    /// map stays the size of the visible slice, not the queue.
    queue_pickers: HashMap<String, Entity<SelectState<Names>>>,
    /// The category list the pickers above currently hold; `sync_queue`
    /// re-sets items only when it changes, not on every refresh.
    queue_picker_items: Option<Names>,
    /// 3.34: one per live picker — a `Confirm` re-renders the rows so the
    /// row's Categorize button flips from disabled to enabled the frame a
    /// category is picked. Retired with its picker in `sync_queue`.
    queue_picker_subs: HashMap<String, Subscription>,
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
            cx.new(|cx| InputState::new(window, cx).placeholder("New category name"));
        let new_parent_kind = cx.new(|cx| {
            SelectState::new(
                KIND_LABELS.iter().map(|(l, _)| (*l).into()).collect(),
                Some(gpui_kit::component::IndexPath::new(0)),
                window,
                cx,
            )
        });
        let new_child_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("New subcategory name"));
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New name"));
        let move_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        let delete_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        let rule_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        let rule_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        // 3.53: the Categories page's parent-scoped subcategory picker —
        // searchable, wide menu, full-name tooltip (see `subcategory_select`).
        let cat_rule_target =
            cx.new(|cx| SelectState::new(rule_choices(&[]), None, window, cx).searchable(true));
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
        // 3.28: the Rules page search box filters the list on every
        // keystroke, and in "Test a description" mode it is the description
        // the rules are tried against (D5). Typing replaces the list, so the
        // scroll offset goes back to the top.
        let rules_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search pattern or category…"));
        let rules_search_sub =
            cx.subscribe(&rules_search, |this: &mut Self, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.rules_scroll.set_offset(Point::default());
                    // 3.29: the reveal highlight is one-shot; anything typed
                    // after it is the user's own search.
                    this.rules_reveal = None;
                    cx.notify();
                }
            });
        // 3.28: the Type and Move pickers. The Type picker filters the list
        // (nothing selected reads as All types); the Move picker holds the
        // child categories, refreshed by `sync_category_choices`.
        let rules_type_select = cx.new(|cx| {
            SelectState::new(
                vec!["All types".into(), "Substring".into(), "Regex".into()],
                None,
                window,
                cx,
            )
        });
        let rules_type_sub = cx.subscribe_in(
            &rules_type_select,
            window,
            |this: &mut Self, _select, ev: &SelectEvent<Names>, _window, cx| {
                if matches!(ev, SelectEvent::Confirm(_)) {
                    this.rules_scroll.set_offset(Point::default());
                    cx.notify();
                }
            },
        );
        let move_rules_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        // The dashboard search box filters the table on every keystroke.
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search transactions"));
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
            summary_scroll: ScrollHandle::new(),
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
            toasts: ToastManager::new(ToastMotion::sonner()),
            toast_stack: ToastStackState::default(),
            toast_next_id: 0,
            toast_clock: Instant::now(),
            toast_task: None,
            refresh_task: None,
            task: None,
            categories: Vec::new(),
            cat_counts: HashMap::new(),
            selected_category: None,
            new_parent_name,
            new_parent_kind,
            new_child_name,
            new_category_open: false,
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
            cat_rule_target,
            cat_rule_child_ids: Vec::new(),
            rule_match_count: None,
            rule_task: None,
            rule_error: None,
            editing_rule: None,
            armed_delete_rule: None,
            pending_rule_action: None,
            rule_edit_pattern,
            rule_edit_regex: false,
            rules_search,
            rules_test_mode: false,
            rules_type_select,
            rules_unused_only: false,
            collapsed_rule_groups: HashSet::default(),
            rules_sort_by_uses: false,
            selected_rules: HashSet::default(),
            move_rules_target,
            rules_move_error: None,
            rule_uses: HashMap::default(),
            new_rule_open: false,
            rules_focus_pending: std::cell::Cell::new(false),
            rules_reveal: None,
            card_payment_category: None,
            tree_scroll: VirtualListScrollHandle::new(),
            rule_scroll: VirtualListScrollHandle::new(),
            queue_scroll: VirtualListScrollHandle::new(),
            rules_scroll: VirtualListScrollHandle::new(),
            #[cfg(test)]
            tree_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            tree_range: std::cell::Cell::new((0, 0)),
            #[cfg(test)]
            rule_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            rule_range: std::cell::Cell::new((0, 0)),
            #[cfg(test)]
            rules_page_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            rules_page_range: std::cell::Cell::new((0, 0)),
            queue: Vec::new(),
            queue_pickers: HashMap::new(),
            queue_picker_items: None,
            queue_picker_subs: HashMap::new(),
            queue_error: None,
            #[cfg(test)]
            queue_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            queue_range: std::cell::Cell::new((0, 0)),
            _subs: vec![
                preview_sub,
                rule_edit_sub,
                rules_search_sub,
                rules_type_sub,
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
                // 3.28: the Rules page Uses column (how many unlocked rows
                // each rule's match decides). 3.87 adds per-rule totals;
                // the page still shows counts only, so keep just those.
                let uses: HashMap<i64, usize> = rule_use_stats(c)?
                    .into_iter()
                    .map(|(id, stats)| (id, stats.count))
                    .collect();
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
                    uses,
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
                        uses,
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
                        // A range change shows new months: park the month
                        // strip (3.30) at its right end so the most recent
                        // month sits beside the pinned `Total` column. The
                        // huge negative offset clamps to the true maximum
                        // when the strip next paints.
                        if this.summary.months != summary.months {
                            this.summary_scroll
                                .set_offset(point(px(-1_000_000.), px(0.)));
                        }
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
                        this.rule_uses = uses;
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
                        let rules = this.rules.clone();
                        let card_payment = this.card_payment_category;
                        this.table.update(cx, |t, cx| {
                            let delegate = t.delegate_mut();
                            delegate.rows = rows;
                            delegate.child_choices = child_choices;
                            delegate.rule_offer = rule_offer;
                            delegate.rules = rules;
                            delegate.card_payment_category = card_payment;
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
                    Err(e) => this.notify_err(format!("Database error: {e}"), cx),
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
        self.new_category_open = false;
        self.confirm_delete_category = None;
        self.close_rule_editor();
        self.armed_delete_rule = None;
        // 3.28: the Rules page's add row, ticked rows and move error belong
        // to that page too; entering it focuses the search box (D5).
        // 3.29: so does the *Show rule* highlight — it belongs to the row
        // the menu was opened on.
        self.new_rule_open = false;
        self.selected_rules.clear();
        self.rules_move_error = None;
        self.rules_reveal = None;
        self.rules_focus_pending.set(page == Page::Rules);
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
                                if *is_error {
                                    this.notify_err(message.clone(), cx);
                                } else {
                                    this.notify_ok(message.clone(), cx);
                                }
                                this.import = None;
                            }
                            ImportEvent::Cancelled => this.import = None,
                            ImportEvent::AccountsChanged => {}
                        }
                        this.refresh(window, cx);
                    });
                self._subs.push(sub);
                self.import = Some(panel);
                // Opening a file switches to the Imports page with the
                // preview open (ui-conventions, *Pages*).
                self.page = Page::Imports;
            }
            Err(e) => self.notify_err(e.to_string(), cx),
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
        self.notify_ok("Export is not available yet.", cx);
    }

    /// Restore…: wired in M5, like [`Shell::export_data`].
    fn restore_backup(&mut self, cx: &mut Context<Self>) {
        self.notify_ok("Restore is not available yet.", cx);
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
            let _ =
                this.update_in(cx, |this, window, cx| {
                    match result {
                    Ok((Some(u), None)) => this.notify_ok(
                        format!("Undid import of {}: removed {} rows", u.file_name, u.removed),
                        cx,
                    ),
                    // The undo landed but the master CSV did not: the
                    // message is the same, flagged as an error.
                    Ok((Some(u), Some(e))) => this.notify_err(
                        format!(
                            "Undid import of {}: removed {} rows (master CSV update failed: {e})",
                            u.file_name, u.removed
                        ),
                        cx,
                    ),
                    Ok((None, _)) => {
                        this.notify_ok("This account has no imports to undo.", cx);
                    }
                    Err(e) => this.notify_err(e, cx),
                }
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
        // 3.29: the system `Transfers > Card Payment` category, so the
        // dashboard menu can tell a built-in card payment from a hand-set
        // locked row. Same name-and-parent lookup `financi-db` uses; ids are
        // never hardcoded.
        self.card_payment_category = self
            .categories
            .iter()
            .find(|p| p.name == "Transfers")
            .and_then(|p| {
                p.children
                    .iter()
                    .find(|c| c.is_system && c.name == "Card Payment")
                    .map(|c| c.id)
            });
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
            .update(cx, |s, cx| s.set_items(names.clone(), window, cx));
        // 3.28: the Move bar's picker offers the same child categories.
        self.move_rules_target
            .update(cx, |s, cx| s.set_items(names, window, cx));
        // 3.28: a refresh that dropped a rule drops its checkbox with it;
        // a deleted category drops its collapsed-group marker.
        self.selected_rules
            .retain(|id| self.rules.iter().any(|r| r.id == *id));
        self.collapsed_rule_groups.retain(|id| known(*id));
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
        // 3.53: the Categories add-rule picker follows the selection and
        // the (possibly renamed) child list; the pick survives while its
        // id is one of the parent's children.
        self.sync_cat_rule_picker(false, window, cx);
    }

    /// 3.53: the selected parent's subcategories by their own names (no
    /// "Parent › Child"), empty when the selection is a subcategory or
    /// none. Order matches the tree.
    fn parent_child_choices(&self) -> Vec<(i64, String)> {
        let Some(parent_id) = self.selected_category else {
            return Vec::new();
        };
        // Only a parent selection has children to offer the picker; a
        // subcategory targets itself, so the picker is not shown.
        self.categories
            .iter()
            .find(|p| p.id == parent_id)
            .map(|p| p.children.iter().map(|c| (c.id, c.name.clone())).collect())
            .unwrap_or_default()
    }

    /// 3.53: repoint the Categories add-rule picker at the selected
    /// parent's subcategories. `reset` clears the pick (a fresh selection
    /// never pre-picks); a refresh keeps the pick when its id is still one
    /// of the parent's children.
    fn sync_cat_rule_picker(&mut self, reset: bool, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.parent_child_choices();
        let keep = if reset {
            None
        } else {
            self.cat_rule_target
                .read(cx)
                .selected_index(cx)
                .and_then(|ix| self.cat_rule_child_ids.get(ix.row).copied())
        };
        self.cat_rule_child_ids = choices.iter().map(|(id, _)| *id).collect();
        let selected = keep.and_then(|id| self.cat_rule_child_ids.iter().position(|c| *c == id));
        self.cat_rule_target.update(cx, |s, cx| {
            s.set_items(rule_choices(&choices), window, cx);
            s.set_selected_index(
                selected.map(gpui_kit::component::IndexPath::new),
                window,
                cx,
            );
        });
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

    /// 3.23: the rules the Categories page lists — the ones targeting the
    /// selected category, which for a parent means its subcategories' too
    /// (rules always target a child, but count the parent's own id in case
    /// one ever lands there). Nothing without a selection.
    fn visible_rules(&self) -> Vec<Rule> {
        let Some(id) = self.selected_category else {
            return Vec::new();
        };
        let mut ids: HashSet<i64> = [id].into_iter().collect();
        if let Some(parent) = self.categories.iter().find(|p| p.id == id) {
            ids.extend(parent.children.iter().map(|c| c.id));
        }
        self.rules
            .iter()
            .filter(|r| ids.contains(&r.category_id))
            .cloned()
            .collect()
    }

    fn select_category(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.selected_category != Some(id);
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
        // 3.23: the rule list now shows only this category's rules, so its
        // content is replaced with the selection and the scroll offset
        // resets (the reset point 3.21 documented).
        if changed {
            self.rule_scroll.set_offset(Point::default());
            // 3.53: the add-rule picker belongs to the selection: a
            // subcategory needs none, a parent picks among its own
            // children — never a default pick.
            self.sync_cat_rule_picker(true, window, cx);
        }
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
            self.cat_error = Some("Give the category a name.".into());
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
                        // 3.23: adding closes the New category… form.
                        this.new_category_open = false;
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
            self.cat_error = Some("Give the subcategory a name.".into());
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
            self.cat_error = Some("Choose the category to move it under.".into());
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

    /// 3.40: is `action` the rule button currently waiting on the DB? The
    /// matching button renders `.loading(true)` and disabled while it is,
    /// so a slow save can't be double-submitted.
    fn rule_action_pending(&self, action: PendingRuleAction) -> bool {
        self.pending_rule_action == Some(action)
    }

    fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.rule_target.read(cx).selected_index(cx) else {
            self.rule_error = Some("Choose the category the rule assigns.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        self.commit_new_rule(category_id, window, cx);
    }

    /// 3.53: Add rule on the Categories page. A selected subcategory is
    /// the target — nothing to pick. A parent needs the row's picker:
    /// rules must target a child (`ensure_assignable`), so no pick is an
    /// error. No selection (nothing selected) can't render the row at all.
    fn add_rule_to_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let category_id = match self.selected_node() {
            Some((_, None)) => {
                let Some(ix) = self.cat_rule_target.read(cx).selected_index(cx) else {
                    self.rule_error = Some("Choose the subcategory the rule assigns.".into());
                    cx.notify();
                    return;
                };
                let Some(cid) = self.cat_rule_child_ids.get(ix.row).copied() else {
                    return;
                };
                cid
            }
            Some((node, Some(_))) => node.id,
            None => return,
        };
        self.commit_new_rule(category_id, window, cx);
    }

    /// The shared tail of both add-rule paths: validate the pattern, run
    /// [`financi_db::rules::add_rule`], clear the row and refresh (3.40:
    /// the button shows loading, disabled, while the DB is busy — a save
    /// that re-runs the rules can take seconds).
    fn commit_new_rule(&mut self, category_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let pattern = self.rule_pattern.read(cx).value().trim().to_string();
        let is_regex = self.rule_regex;
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.40: the Add button shows loading, disabled, until the DB is
        // done (a save that re-runs the rules can take seconds).
        self.pending_rule_action = Some(PendingRuleAction::Add);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| add_rule(c, &pattern, is_regex, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending_rule_action = None;
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
        // 3.53: no category picker — Edit changes pattern and kind only.
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
        // 3.53: the editor has no picker, so the rule keeps its category;
        // `update_rule` is still handed the rule's own `category_id`.
        let Some(category_id) = self
            .rules
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.category_id)
        else {
            return;
        };
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.40: the Save button shows loading, disabled, until the DB is
        // done; on error the editor stays open and pending clears too.
        self.pending_rule_action = Some(PendingRuleAction::Save(id));
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                update_rule(c, id, &pattern, is_regex, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending_rule_action = None;
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
            // 3.40: the delete runs (and the button shows loading,
            // disabled) while the rules re-run over the unlocked rows.
            self.pending_rule_action = Some(PendingRuleAction::Delete(id));
            self.task = Some(cx.spawn_in(window, async move |this, cx| {
                let result = on_db(&db, move |c| delete_rule(c, id)).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.pending_rule_action = None;
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
        // 3.34: a picker's Categorize-mirror subscription retires with it.
        self.queue_picker_subs
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
        // 3.34: the row's Categorize button mirrors the picker — disabled
        // with no pick, enabled with one — so a confirmed pick re-renders
        // the rows. The subscription lives exactly as long as the picker.
        let sub = cx.subscribe(&picker, |_, _, ev: &SelectEvent<Names>, cx| {
            if matches!(ev, SelectEvent::Confirm(_)) {
                cx.notify();
            }
        });
        self.queue_pickers
            .insert(merchant.to_string(), picker.clone());
        self.queue_picker_subs.insert(merchant.to_string(), sub);
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
                        this.notify_ok(format!("Categorized {merchant}"), cx);
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
                    this.notify_err(e, cx);
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
                    Ok(_) => this.notify_ok("Paired transfer.", cx),
                    Err(e) => this.notify_err(format!("Could not pair: {e}"), cx),
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
                    Ok(_) => this.notify_ok("Marked as not a transfer.", cx),
                    Err(e) => this.notify_err(format!("Could not update: {e}"), cx),
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
                    Ok(_) => this.notify_ok("Unpaired transaction.", cx),
                    Err(e) => this.notify_err(format!("Could not unpair: {e}"), cx),
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

    /// Success/info feedback: a toast in the bottom-right corner that
    /// dismisses itself after [`TOAST_TIMEOUT`]. Every former
    /// `self.status = Some((msg, false))` call site goes through here.
    fn notify_ok(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast(message.into(), false, cx);
    }

    /// Failure feedback: a toast that stays until the user closes it.
    fn notify_err(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast(message.into(), true, cx);
    }

    fn push_toast(&mut self, message: SharedString, is_error: bool, cx: &mut Context<Self>) {
        let id = self.toast_next_id;
        self.toast_next_id += 1;
        self.toasts.push(
            id,
            ToastMessage { message, is_error },
            ToastOptions {
                timeout: if is_error { None } else { Some(TOAST_TIMEOUT) },
            },
            self.toast_clock,
        );
        self.tick_toasts(cx);
        cx.notify();
    }

    /// The user closed a toast — gpui-base toasts dismiss on click.
    fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.toasts.dismiss(&id, self.toast_clock) {
            self.tick_toasts(cx);
            cx.notify();
        }
    }

    /// Drive the toast lifecycle while anything is mounted.
    ///
    /// `ToastManager` is a pure function of the clock it is handed, so the
    /// ticker advances a *virtual* clock by the timer's interval rather than
    /// calling `Instant::now()`: the test executor's `advance_clock` moves
    /// its timers without moving real time, and the auto-dismiss tests need
    /// both to agree. The clock only matters relatively — between toasts it
    /// stands still, and every entry's deadlines are deltas from its push.
    ///
    /// A hovered or focused stack pauses the auto-hide timers (gpui-base's
    /// semantics), so a toast the user is reaching for never vanishes under
    /// the pointer.
    fn tick_toasts(&mut self, cx: &mut Context<Self>) {
        if self.toast_task.is_some() || self.toasts.is_empty() {
            return;
        }
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TOAST_TICK).await;
                let running = this.update(cx, |this, cx| {
                    this.toast_clock += TOAST_TICK;
                    let advanced = this
                        .toasts
                        .advance(this.toast_clock, this.toast_stack.is_expanded());
                    let running = !this.toasts.is_empty();
                    if !running {
                        this.toast_task = None;
                    } else if advanced.changed {
                        cx.notify();
                    }
                    running
                });
                if !running.unwrap_or(false) {
                    return;
                }
            }
        }));
    }

    /// The toast overlay (`docs/ui-conventions.md`, *Feedback*): anchored
    /// bottom-right, stacked, newest in front. It is absolutely positioned,
    /// so it overlays the window and takes no layout space — the page area
    /// starts at the top of the content padding whether or not a toast
    /// shows. Card styling follows the panel rule (border, radius) on the
    /// `popover` surface, `text_sm`; error text is `danger`.
    fn render_toasts(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.toasts.is_empty() {
            return div().into_any_element();
        }
        let (popover, foreground, border, danger, radius) = {
            let theme = cx.theme();
            (
                theme.popover,
                theme.foreground,
                theme.border,
                theme.danger,
                theme.radius,
            )
        };
        let mut stack = ToastStack::new("toasts", self.toast_stack.clone())
            .placement(Anchor::BottomRight)
            .absolute()
            .bottom_4()
            .right_4()
            .w(px(TOAST_W));
        for (id, toast, status) in self.toasts.iter() {
            let id = *id;
            stack = stack.item(
                id,
                Toast::new(format!("toast-{id}"))
                    .transition_status(status)
                    .w_full()
                    .child(
                        div()
                            .id(("toast-dismiss", id))
                            .w_full()
                            .rounded(radius)
                            .border_1()
                            .border_color(border)
                            .bg(popover)
                            .text_color(if toast.is_error { danger } else { foreground })
                            .px_3()
                            .py_2()
                            .debug_selector({
                                let message = toast.message.clone();
                                move || format!("toast:{message}")
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dismiss_toast(id, cx);
                            }))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_start()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .child(toast.message.clone()),
                                    )
                                    .child(div().flex_none().child("×")),
                            ),
                    ),
            );
        }
        stack.into_any_element()
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
        // 3.28: the rules live on their own page now, after Categories.
        let rules_nav = self.nav_item("Rules", IconName::Wrench, Page::Rules, cx);
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
            .child(NavItem::Item(Box::new(rules_nav)))
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
                        // 3.38: the path ellipsizes at the sidebar's width;
                        // hovering shows the full path prefixed "Database: "
                        // (ui-conventions, *Sidebar*).
                        div()
                            .id("sidebar:db-path")
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .debug_selector({
                                let path = self.db_path.display().to_string();
                                move || format!("sidebar:db-path:{path}")
                            })
                            .tooltip({
                                let full = format!("Database: {}", self.db_path.display());
                                move |window, cx| {
                                    let tip = full.clone();
                                    Tooltip::element(move |_, _| {
                                        div()
                                            .debug_selector({
                                                let s = tip.clone();
                                                move || format!("sidebar:db-path-tooltip:{s}")
                                            })
                                            .child(tip.clone())
                                    })
                                    .build(window, cx)
                                }
                            })
                            .child(self.db_path.display().to_string()),
                    )
                    .child({
                        // 3.39: the build version as the footer's last line,
                        // e.g. "v0.17.0-3-gabc1234-dirty" (plan.md → *Show
                        // the version subtly in the UI*). Ellipsized with a
                        // hover tooltip, like the path above (3.38).
                        let label = crate::version::version_label();
                        div()
                            .id("sidebar:version")
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .debug_selector({
                                let s = label.clone();
                                move || format!("sidebar:version-text:{s}")
                            })
                            .tooltip({
                                let label = label.clone();
                                move |window, cx| {
                                    let tip = label.clone();
                                    Tooltip::element(move |_, _| {
                                        div()
                                            .debug_selector({
                                                let s = tip.clone();
                                                move || format!("sidebar:version-tooltip:{s}")
                                            })
                                            .child(tip.clone())
                                    })
                                    .build(window, cx)
                                }
                            })
                            .child(label)
                    }),
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

    /// The plot's width for the current window: the sidebar and page
    /// padding come off the window width, `CHART_PANEL_SHARE` of what
    /// remains is the chart panel, and inside the panel its padding, the
    /// legend and the gap come off before the plot is left.
    fn chart_plot_width(&self) -> f32 {
        let page_w = (self.window_width - SIDEBAR_W - 2. * PAGE_PAD).max(0.);
        (page_w * CHART_PANEL_SHARE - 2. * PAGE_PAD - LEGEND_W - REGION_GAP).max(0.)
    }

    /// The chart's label plan (3.15) for the current window: how many
    /// characters a band label may use and how many labels are drawn.
    fn chart_label_plan(&self) -> (usize, usize) {
        band_label_budget(self.chart_plot_width(), self.chart_rows.len())
    }

    /// The chart's rows prepared for drawing (3.15, 3.31): band labels
    /// fitted to the current budget, each row's share over its bar
    /// thinned to `plot_width`, and `Uncategorized` last whatever its
    /// amount. `colors` maps category ids to palette colors; `muted`
    /// paints the `Uncategorized` swatch.
    fn chart_bars(
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

    fn render_imports(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        // The page exposes every file action (3.7): the native menu may be
        // absent (Linux), so Import, Undo, Export and Restore live here.
        // 3.32: each button carries its Lucide icon left of the label, and
        // no ellipsis — the action runs straight away, any confirmation is
        // the button's own armed state, not a hidden dialog.
        let latest_undo_id = self.import_history.first().map(|e| e.id);
        let undo_label =
            if self.confirm_undo_import.is_some() && self.confirm_undo_import == latest_undo_id {
                "Click again to undo"
            } else {
                "Undo"
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
                            .label("Import CSV")
                            .icon(IconName::FileUp)
                            .primary()
                            .debug_selector(|| {
                                "imports:header:Import CSV:icons/file-up.svg".to_string()
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_file(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("undo-last-import")
                            .label(undo_label)
                            .icon(IconName::Undo2)
                            .disabled(latest_undo_id.is_none())
                            .debug_selector(move || {
                                format!("imports:header:{undo_label}:icons/undo-2.svg")
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.undo_last_import(window, cx);
                            })),
                    )
                    .child(
                        Button::new("export-data")
                            .label("Export")
                            .icon(IconName::Download)
                            .debug_selector(|| {
                                "imports:header:Export:icons/download.svg".to_string()
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.export_data(cx))),
                    )
                    .child(
                        Button::new("restore-backup")
                            .label("Restore")
                            .icon(IconName::ArchiveRestore)
                            .debug_selector(|| {
                                "imports:header:Restore:icons/archive-restore.svg".to_string()
                            })
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

    /// One clickable row of the category tree (3.23): a color swatch
    /// (`category_colors`, the same color the transaction views show),
    /// the name — semibold for a top-level category — truncating with an
    /// ellipsis, and the transaction count right-aligned. The selected row
    /// takes `list_active`, an unselected one lights up `list_hover` under
    /// the pointer (gpui-component's `list_item` pattern).
    fn tree_row(
        &self,
        node: &CategoryNode,
        depth: u8,
        selected: bool,
        palette: (Hsla, Hsla, Hsla),
        colors: &HashMap<i64, Hsla>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (muted, list_active, list_hover) = palette;
        let id = node.id;
        let count = self.node_count(node);
        let radius = cx.theme().radius;
        let color = colors.get(&id).copied().unwrap_or(muted);
        div()
            .id(SharedString::from(format!("cat-row-{id}")))
            // Fill the list's width, or the row stays at its natural width:
            // the counts would not line up at the right edge, the selection
            // background would cover only the text, and a long name would
            // never truncate (the queue rows, 3.16).
            .w_full()
            .h_8()
            .px_2()
            .pl(px(8. + 16. * depth as f32))
            .flex()
            .items_center()
            .gap_2()
            .rounded(radius)
            .cursor_pointer()
            .debug_selector(move || format!("cat-tree-row-{id}"))
            .when(selected, |d| d.bg(list_active))
            // A selected row keeps its `list_active` under the pointer.
            .hover(move |s| if selected { s } else { s.bg(list_hover) })
            .child(
                div()
                    .w(px(10.))
                    .h(px(10.))
                    .flex_shrink_0()
                    .rounded(px(2.))
                    .bg(color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .when(depth == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(node.name.clone()),
            )
            .when(node.is_system, |d| {
                d.child(div().text_xs().text_color(muted).child("system"))
            })
            .child(
                div()
                    .w(px(48.))
                    .flex_shrink_0()
                    .text_right()
                    .text_xs()
                    .text_color(muted)
                    .child(flow::count(count)),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select_category(id, window, cx);
            }))
            .into_any_element()
    }

    /// One row of the selected category's rule list (3.12: built lazily
    /// by the virtualized rule list, fixed height to match its
    /// `item_sizes`). 3.24: every row carries Edit and Delete… at its right
    /// end — small ghost buttons, visible on every row, built only for the
    /// rows the list renders. 3.23: the columns line up with the list's
    /// headings (`RULE_TYPE_W`/`RULE_CATEGORY_W`); the pattern column is
    /// the flex one and truncates with a tooltip, like the queue's
    /// merchant column.
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
            .w_full()
            .gap_2()
            .items_center()
            .debug_selector(move || format!("rule-row-{id}"))
            .child(
                div()
                    .id(SharedString::from(format!("rule-pattern-{id}")))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .debug_selector(move || format!("rule:pattern-col:{id}"))
                    .tooltip({
                        let full = rule.pattern.clone();
                        move |window, cx| {
                            let tip = full.clone();
                            Tooltip::element(move |_, _| {
                                div()
                                    .debug_selector({
                                        let p = tip.clone();
                                        move || format!("rule:pattern-tooltip:{p}")
                                    })
                                    .child(tip.clone())
                            })
                            .build(window, cx)
                        }
                    })
                    .child(rule.pattern.clone()),
            )
            .child(
                div()
                    .w(px(RULE_TYPE_W))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(muted)
                    .debug_selector(move || format!("rule:type-col:{id}"))
                    .child(if rule.is_regex { "regex" } else { "substring" }),
            )
            .child(
                div()
                    .w(px(RULE_CATEGORY_W))
                    .flex_shrink_0()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(muted)
                    .debug_selector(move || format!("rule:category-col:{id}"))
                    .child(target),
            )
            .child(
                Button::new(SharedString::from(format!("rule-edit-{id}")))
                    .label("Edit")
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
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
                    .flex_shrink_0()
                    // 3.40: loading + disabled while the delete runs.
                    .loading(self.rule_action_pending(PendingRuleAction::Delete(id)))
                    .disabled(self.rule_action_pending(PendingRuleAction::Delete(id)))
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
    /// item and a taller row would paint over the rows below. 3.23: inside
    /// the detail panel the row is too narrow for the count beside the
    /// fields, so it moves under them (like the add-rule row's) and the
    /// row reserves 80 px in `rule_sizes` instead of 24, 108 px while an
    /// error is open (measured: 75.5 px with the count, 102.5 with one
    /// error line).
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
                        // 3.23: the pattern field is the flex column, so
                        // the editor fits the detail panel's width; the
                        // other controls keep their size.
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(field_label(muted, "Pattern"))
                            .child(
                                div()
                                    .w_full()
                                    .debug_selector(|| "rule:edit-pattern-input".into())
                                    .child(Input::new(&self.rule_edit_pattern)),
                            ),
                    )
                    .child(
                        // 3.42: the unlabelled switch sits in a control-height
                        // slot so its centre matches the input, picker and
                        // buttons.
                        control_slot(
                            "rule:regex-slot:edit".into(),
                            "rule:switch:edit".into(),
                            Switch::new(SharedString::from(format!("rule-edit-regex-{}", rule.id)))
                                .label("regex")
                                .checked(self.rule_edit_regex)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.rule_edit_regex = *checked;
                                    this.preview_rule_match(cx);
                                    cx.notify();
                                })),
                        ),
                    )
                    // 3.53: no Category picker — the editor changes
                    // pattern and kind only, and the rule keeps its
                    // category (moving is the Rules page Move bar's job).
                    .child(
                        Button::new(SharedString::from(format!("rule-edit-save-{}", rule.id)))
                            .label("Save")
                            .primary()
                            .flex_shrink_0()
                            // 3.42: the alignment test measures its centre.
                            .debug_selector(|| "rule:edit-save-btn".into())
                            // 3.40: loading + disabled while the save runs.
                            .loading(self.rule_action_pending(PendingRuleAction::Save(rule.id)))
                            .disabled(self.rule_action_pending(PendingRuleAction::Save(rule.id)))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_rule_edit(window, cx)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("rule-edit-cancel-{}", rule.id)))
                            .label("Cancel")
                            .ghost()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_edit_rule(cx))),
                    ),
            )
            // 3.23: the count sits under the fields (as in the add-rule
            // row): the detail panel is too narrow to keep it inline.
            .child(preview)
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

    // -- Rules page (3.28) ----------------------------------------------------

    /// "Parent › Child" label of the rule's target category, shared by the
    /// row, the group headers and the search matching.
    fn rule_target_label(&self, rule: &Rule) -> String {
        self.child_choices
            .iter()
            .find(|(id, _)| *id == rule.category_id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("#{}", rule.category_id))
    }

    /// The Rules page list's entries after the page filters (D5). Normal
    /// mode groups the rules by target subcategory (groups in label order,
    /// rows within a group sorted by Pattern — the default — or by Uses),
    /// with a group header row per group. "Test a description" mode shows
    /// a flat list of the rules matching the text, in the order
    /// [`winning_rule`] tries them (substring before regex, longest pattern
    /// first, lowest id), so the winner is the first row.
    fn rules_page_items(&self, cx: &App) -> Vec<RulesItem> {
        let query = self
            .rules_search
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let query = (!query.is_empty()).then_some(query);
        // Type picker: row 1 keeps substrings, row 2 regexes, nothing
        // selected or row 0 shows all.
        let type_ix = self
            .rules_type_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        let test = self.rules_test_mode;
        let mut filtered: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| match type_ix {
                1 => !r.is_regex,
                2 => r.is_regex,
                _ => true,
            })
            .filter(|r| {
                !self.rules_unused_only || self.rule_uses.get(&r.id).copied().unwrap_or(0) == 0
            })
            .filter(|r| {
                if test {
                    // The search box is the description: only rules that
                    // match it stay.
                    query
                        .as_deref()
                        .is_some_and(|q| pattern_matches(&r.pattern, r.is_regex, q))
                } else if let Some(q) = &query {
                    // Pattern and target category name, case-insensitive.
                    r.pattern.to_ascii_lowercase().contains(q)
                        || self.rule_target_label(r).to_ascii_lowercase().contains(q)
                } else {
                    true
                }
            })
            .collect();
        if test {
            filtered.sort_by(|a, b| {
                a.is_regex
                    .cmp(&b.is_regex)
                    .then_with(|| b.pattern.chars().count().cmp(&a.pattern.chars().count()))
                    .then_with(|| a.id.cmp(&b.id))
            });
            return filtered
                .into_iter()
                .map(|r| RulesItem::Rule { id: r.id })
                .collect();
        }
        // Group by target label, groups in label order.
        let mut groups: Vec<(String, i64, Vec<&Rule>)> = Vec::new();
        for r in &filtered {
            let label = self.rule_target_label(r);
            match groups.iter_mut().find(|(l, _, _)| *l == label) {
                Some((_, _, rules)) => rules.push(r),
                None => groups.push((label, r.category_id, vec![r])),
            }
        }
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        let by_uses = self.rules_sort_by_uses;
        let mut items = Vec::with_capacity(filtered.len() + groups.len());
        for (label, cid, mut rules) in groups {
            let count = rules.len();
            let collapsed = self.collapsed_rule_groups.contains(&cid);
            items.push(RulesItem::Group {
                id: cid,
                label,
                count,
                collapsed,
            });
            if collapsed {
                continue;
            }
            rules.sort_by(|a, b| {
                if by_uses {
                    let ua = self.rule_uses.get(&a.id).copied().unwrap_or(0);
                    let ub = self.rule_uses.get(&b.id).copied().unwrap_or(0);
                    ub.cmp(&ua).then_with(|| a.pattern.cmp(&b.pattern))
                } else {
                    a.pattern.cmp(&b.pattern)
                }
            });
            items.extend(rules.into_iter().map(|r| RulesItem::Rule { id: r.id }));
        }
        items
    }

    fn toggle_rule_group(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.collapsed_rule_groups.remove(&id) {
            self.collapsed_rule_groups.insert(id);
        }
        cx.notify();
    }

    fn set_rule_selected(&mut self, id: i64, on: bool, cx: &mut Context<Self>) {
        if on {
            self.selected_rules.insert(id);
        } else {
            self.selected_rules.remove(&id);
        }
        self.rules_move_error = None;
        cx.notify();
    }

    /// The Move bar's action: one [`move_rules`] call moves every checked
    /// rule and re-runs the rules over the unlocked rows once; then the
    /// selection clears and everything refreshes.
    fn move_selected_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.move_rules_target.read(cx).selected_index(cx) else {
            self.rules_move_error = Some("Choose the category to move to.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        let mut ids: Vec<i64> = self.selected_rules.iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        ids.sort();
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_rules(c, &ids, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.selected_rules.clear();
                        this.rules_move_error = None;
                    }
                    Err(e) => this.rules_move_error = Some(e.into()),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// The *Show rule* item (3.29): open the Rules page on the given rule.
    /// The page's filters are cleared so the rule is certainly in the list
    /// (test mode, the Type picker and Unused could all hide it), its group
    /// is expanded, and the search box takes the rule's pattern — the list
    /// narrows to it find-in-page style, and a rule always matches its own
    /// pattern. The row is scrolled into view and carries the `list_active`
    /// background until the user searches or leaves the page.
    fn show_rule(&mut self, rule_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // Navigate first: `go_page` clears the page's transient state, and
        // the reveal must survive that reset.
        self.go_page(Page::Rules, cx);
        let Some(rule) = self.rules.iter().find(|r| r.id == rule_id).cloned() else {
            return; // a refresh dropped the rule between menu and click
        };
        self.rules_test_mode = false;
        self.rules_unused_only = false;
        self.collapsed_rule_groups.remove(&rule.category_id);
        self.rules_type_select.update(cx, |s, cx| {
            s.set_selected_index(Some(gpui_kit::component::IndexPath::new(0)), window, cx)
        });
        // `set_value` scrolls the list back to the top, so the reveal's
        // `scroll_to_item` must come after it. Any later keystroke fires
        // the search subscription, which drops the reveal.
        let pattern = rule.pattern.clone();
        self.rules_search
            .update(cx, |i, cx| i.set_value(pattern, window, cx));
        let items = self.rules_page_items(cx);
        if let Some(ix) = items
            .iter()
            .position(|it| matches!(it, RulesItem::Rule { id } if *id == rule_id))
        {
            self.rules_reveal = Some(rule_id);
            self.rules_scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// The Rules page group header row: "Food › Coffee · 4 rules", clicking
    /// it collapses or expands the group's rows (D5).
    fn rules_group_header(
        &self,
        id: i64,
        label: &str,
        count: usize,
        collapsed: bool,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.to_string();
        div()
            .id(SharedString::from(format!("rules-group-{id}")))
            .h_6()
            .w_full()
            .flex()
            .items_center()
            .cursor_pointer()
            .text_sm()
            .text_color(muted)
            .debug_selector(move || format!("rules:group-{id}"))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_rule_group(id, cx)))
            .child(format!(
                "{} {} · {} rules",
                if collapsed { "▸" } else { "▾" },
                label,
                count
            ))
            .into_any_element()
    }

    /// One rule row of the Rules page: checkbox, Pattern (the flex column,
    /// ellipsized with a tooltip like the Categories row), a "wins" marker
    /// in test mode, Type, Uses (right-aligned) and Edit / Delete…. The
    /// row the editor is open on swaps for [`rule_editor_row`], which is
    /// the same editor the Categories page uses. The row *Show rule* (3.29)
    /// landed on paints with the `list_active` background; its debug
    /// selector grows an `:active` suffix so tests can tell the highlight
    /// apart from a plain row.
    fn rules_page_row(
        &self,
        rule: &Rule,
        winner: bool,
        revealed: bool,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = rule.id;
        let armed = self.armed_delete_rule == Some(id);
        let uses = self.rule_uses.get(&id).copied().unwrap_or(0);
        let reveal_bg = revealed.then(|| cx.theme().list_active);
        h_flex()
            .h_6()
            .w_full()
            .gap_2()
            .items_center()
            .debug_selector(move || {
                if revealed {
                    format!("rules:row-{id}:active")
                } else {
                    format!("rules:row-{id}")
                }
            })
            .when_some(reveal_bg, |row, bg| row.bg(bg))
            .child(
                div().w(px(RULE_SEL_W)).flex_shrink_0().child(
                    checkbox(SharedString::from(format!("rule-sel-{id}")))
                        .checked(self.selected_rules.contains(&id))
                        .on_click(cx.listener(move |this, on: &bool, _, cx| {
                            this.set_rule_selected(id, *on, cx);
                        })),
                ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("rules-pattern-{id}")))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .debug_selector(move || format!("rules:pattern-col:{id}"))
                    .tooltip({
                        let full = rule.pattern.clone();
                        move |window, cx| {
                            let tip = full.clone();
                            Tooltip::element(move |_, _| {
                                div()
                                    .debug_selector({
                                        let p = tip.clone();
                                        move || format!("rules:pattern-tooltip:{p}")
                                    })
                                    .child(tip.clone())
                            })
                            .build(window, cx)
                        }
                    })
                    .child(rule.pattern.clone()),
            )
            .when(winner, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().primary)
                        .debug_selector(move || format!("rules:wins-{id}"))
                        .child("wins"),
                )
            })
            .child(
                div()
                    .w(px(RULE_TYPE_W))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(muted)
                    .debug_selector(move || format!("rules:type-col:{id}"))
                    .child(if rule.is_regex { "regex" } else { "substring" }),
            )
            .child(
                div()
                    .w(px(RULE_USES_W))
                    .flex_shrink_0()
                    .text_right()
                    .text_xs()
                    .text_color(muted)
                    .debug_selector(move || format!("rules:uses-col:{id}"))
                    .child(flow::count(uses as i64)),
            )
            .child(
                Button::new(SharedString::from(format!("rule-edit-{id}")))
                    .label("Edit")
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
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
                    .flex_shrink_0()
                    // 3.40: loading + disabled while the delete runs.
                    .loading(self.rule_action_pending(PendingRuleAction::Delete(id)))
                    .disabled(self.rule_action_pending(PendingRuleAction::Delete(id)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.arm_delete_rule(id, window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }

    /// The Rules page (3.28, D5): header with the rule count and New rule…,
    /// the add-rule row under it, the filter row (Search, Test a
    /// description, Type, Unused), the Move bar over the list while rows
    /// are checked, and the virtualized list of group headers and rule
    /// rows.
    fn render_rules(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, danger) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.danger)
        };
        let count_text = format!("{} rules", self.rules.len());
        let count_sel = count_text.clone();
        let query = self.rules_search.read(cx).value().trim().to_string();
        // The "wins" marker follows [`winning_rule`] exactly, so the page
        // marks the rule the categoriser would actually apply (D5).
        let winner = if self.rules_test_mode && !query.is_empty() {
            winning_rule(&self.rules, &query)
        } else {
            None
        };
        let items = Rc::new(self.rules_page_items(cx));
        let editing = self.editing_rule;
        let edit_error = self.rule_error.is_some();
        // Rows are 24 px; the row the editor replaced reserves 80 px
        // (108 px while a validation error is open), like the Categories
        // rule list.
        let item_sizes = Rc::new(
            items
                .iter()
                .map(|it| match it {
                    RulesItem::Group { .. } => size(px(0.), px(24.)),
                    RulesItem::Rule { id } => size(
                        px(0.),
                        if editing == Some(*id) {
                            if edit_error { px(108.) } else { px(80.) }
                        } else {
                            px(24.)
                        },
                    ),
                })
                .collect::<Vec<_>>(),
        );
        let list_items = Rc::clone(&items);
        let list = v_virtual_list(
            cx.entity(),
            "rules-list",
            item_sizes,
            move |this, range, _window, cx| {
                let rows: Vec<AnyElement> = list_items
                    .iter()
                    .skip(range.start)
                    .take(range.len())
                    .filter_map(|it| match it {
                        RulesItem::Group {
                            id,
                            label,
                            count,
                            collapsed,
                        } => {
                            Some(this.rules_group_header(*id, label, *count, *collapsed, muted, cx))
                        }
                        RulesItem::Rule { id } => {
                            this.rules.iter().find(|r| r.id == *id).map(|r| {
                                if this.editing_rule == Some(*id) {
                                    this.rule_editor_row(r, muted, danger, cx)
                                } else {
                                    let revealed = this.rules_reveal == Some(*id);
                                    this.rules_page_row(r, winner == Some(*id), revealed, muted, cx)
                                }
                            })
                        }
                    })
                    .collect();
                #[cfg(test)]
                this.rules_page_rows_built.set(rows.len());
                #[cfg(test)]
                this.rules_page_range.set((range.start, range.end));
                rows
            },
        )
        // 3.21: Shell-owned handle (see the tree).
        .track_scroll(&self.rules_scroll)
        .gap_1()
        .flex_1()
        .min_h_0();
        let add_preview = match self
            .rule_match_count
            .filter(|_| self.editing_rule.is_none())
        {
            Some(n) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "rules:add-preview".into())
                .child(format!("matches {} transactions", flow::count(n)))
                .into_any_element(),
            None => div().into_any_element(),
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            // Page header: title, count, New rule… (opens the add row in
            // place, like the Categories header's New category…).
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Rules"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .debug_selector(move || format!("label:{count_sel}"))
                            .child(count_text),
                    )
                    .child(
                        Button::new("new-rule")
                            .label(NEW_RULE)
                            .debug_selector(move || format!("label:{NEW_RULE}"))
                            .primary()
                            .ml_auto()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                // Only one rule editor is open at a time:
                                // opening the add row closes any row editor.
                                this.close_rule_editor();
                                this.new_rule_open = !this.new_rule_open;
                                cx.notify();
                            })),
                    ),
            )
            // Add-rule row (3.28): the same row the Categories page has,
            // hidden until the header button opens it.
            .when(self.new_rule_open, |page| {
                page.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .debug_selector(|| "rules:add-row".into())
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(field_label(muted, "Pattern"))
                                        .child(
                                            div()
                                                .w_full()
                                                .debug_selector(|| "rule:pattern-input:add".into())
                                                .child(Input::new(&self.rule_pattern)),
                                        ),
                                )
                                .child(
                                    // 3.42: control-height slot, centred.
                                    control_slot(
                                        "rule:regex-slot:add".into(),
                                        "rule:switch:add".into(),
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
                                    ),
                                )
                                .child(
                                    v_flex()
                                        .flex_shrink_0()
                                        .gap_1()
                                        .child(field_label(muted, "Category"))
                                        .child(div().w(px(160.)).child(
                                            Select::new(&self.rule_target).placeholder("Category…"),
                                        )),
                                )
                                .child(
                                    Button::new("add-rule")
                                        .label("Add rule")
                                        .primary()
                                        .flex_shrink_0()
                                        // 3.42: the alignment test measures its centre.
                                        .debug_selector(|| "rule:add-rule-btn:add".into())
                                        // 3.40: loading + disabled while the add runs.
                                        .loading(self.rule_action_pending(PendingRuleAction::Add))
                                        .disabled(self.rule_action_pending(PendingRuleAction::Add))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.add_rule(window, cx)
                                        })),
                                ),
                        )
                        .child(add_preview)
                        .children(
                            self.rule_error
                                .clone()
                                .filter(|_| self.editing_rule.is_none())
                                .map(|e| div().text_sm().text_color(danger).child(e)),
                        ),
                )
            })
            // Filter row (D5): Search flexes; Test a description switches
            // the search box into a description; the Type picker and the
            // Unused checkbox trim the list.
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .debug_selector(|| "rules:filter-row".into())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .debug_selector(|| "rules:search".into())
                            .child(Input::new(&self.rules_search)),
                    )
                    .child(
                        Switch::new("rules-test-mode")
                            .label("Test a description")
                            .checked(self.rules_test_mode)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.rules_test_mode = *on;
                                this.rules_scroll.set_offset(Point::default());
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_shrink_0()
                            .debug_selector(|| "rules:type-select".into())
                            .child(Select::new(&self.rules_type_select).placeholder("All types")),
                    )
                    .child(
                        checkbox("rules-unused-only")
                            .label("Unused")
                            .checked(self.rules_unused_only)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.rules_unused_only = *on;
                                this.rules_scroll.set_offset(Point::default());
                                cx.notify();
                            })),
                    ),
            )
            // Move bar (D5): only while rows are checked.
            .when(!self.selected_rules.is_empty(), |page| {
                let n = self.selected_rules.len();
                page.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .debug_selector(|| "rules:move-bar".into())
                                .child(
                                    div()
                                        .text_sm()
                                        .debug_selector(|| "rules:selected-count".into())
                                        .child(format!("{n} selected")),
                                )
                                .child(
                                    v_flex().gap_1().child(field_label(muted, "Move to")).child(
                                        div().w(px(240.)).child(
                                            Select::new(&self.move_rules_target)
                                                .placeholder(MOVE_RULES_TO_PH),
                                        ),
                                    ),
                                )
                                .child(
                                    Button::new("rules-move")
                                        .label("Move")
                                        .primary()
                                        .flex_shrink_0()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.move_selected_rules(window, cx)
                                        })),
                                ),
                        )
                        .children(self.rules_move_error.clone().map(|e| {
                            div()
                                .text_sm()
                                .text_color(danger)
                                .debug_selector(|| "rules:move-error".into())
                                .child(e)
                        })),
                )
            })
            .when(!self.rules_test_mode && !items.is_empty(), |page| {
                page.child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .debug_selector(|| "rules:heading-row".into())
                        .child(div().w(px(RULE_SEL_W)).flex_shrink_0())
                        .child(
                            div().flex_1().min_w_0().child(
                                Button::new("rules-sort-pattern")
                                    .label(if self.rules_sort_by_uses {
                                        "Pattern"
                                    } else {
                                        "Pattern ↓"
                                    })
                                    .ghost()
                                    .text_xs()
                                    .debug_selector(|| "rules:sort-pattern".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rules_sort_by_uses = false;
                                        cx.notify();
                                    })),
                            ),
                        )
                        .child(
                            div().w(px(RULE_TYPE_W)).flex_shrink_0().child(
                                field_label(muted, "Type")
                                    .debug_selector(|| "rules:heading-type".into()),
                            ),
                        )
                        .child(
                            div().w(px(RULE_USES_W)).flex_shrink_0().child(
                                Button::new("rules-sort-uses")
                                    .label(if self.rules_sort_by_uses {
                                        "Uses ↓"
                                    } else {
                                        "Uses"
                                    })
                                    .ghost()
                                    .text_xs()
                                    .debug_selector(|| "rules:sort-uses".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rules_sort_by_uses = true;
                                        cx.notify();
                                    })),
                            ),
                        )
                        .child(div().flex_shrink_0().child(field_label(muted, "Actions"))),
                )
            })
            .when(items.is_empty(), |page| {
                page.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .debug_selector(|| "rules:empty".into())
                        .child(if self.rules_test_mode {
                            "No rules match this description"
                        } else {
                            "No rules match"
                        }),
                )
            })
            .child(list)
            .into_any_element()
    }

    fn render_categories(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, border, list_active, list_hover, danger) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.border,
                theme.list_active,
                theme.list_hover,
                theme.danger,
            )
        };
        let palette = (muted, list_active, list_hover);
        // 3.23: the tree's swatches use the same palette and assignment the
        // transaction views show (ui-conventions, *Category colors*).
        let colors = Rc::new(self.category_colors(cx.theme()));

        // 3.12: the tree is virtualized (gpui-kit `v_virtual_list`). The
        // closure runs during layout with the visible range, so a tree of
        // thousands builds only the rows that can be on screen. Rows are
        // fixed height (`tree_row`'s `h_8` = 32 px); the panel's `gap_1`
        // is added to the stride by the list.
        let n_tree_rows: usize = self.categories.iter().map(|p| 1 + p.children.len()).sum();
        let tree_sizes = Rc::new(vec![size(px(0.), px(32.)); n_tree_rows]);
        // 3.23: the tree lives in its own bordered panel beside the detail
        // panel (ui-conventions, *Pages*).
        // `h_flex` centers children on the cross axis (unlike `v_flex`), so
        // each column needs `h_full` for the virtual list inside it to
        // resolve a real height; without it the lists collapse to 0 px and
        // the page opens blank (3.18).
        let tree_colors = Rc::clone(&colors);
        let tree = v_flex()
            .w(px(300.))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .p_2()
            .gap_1()
            .border_1()
            .border_color(border)
            .rounded_lg()
            .debug_selector(|| "cat:tree-panel".into())
            .child(region_title("Category tree"))
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
                                rows.push(this.tree_row(
                                    parent,
                                    0,
                                    selected,
                                    palette,
                                    &tree_colors,
                                    cx,
                                ));
                            }
                            ix += 1;
                            for child in &parent.children {
                                if ix >= range.end {
                                    break;
                                }
                                if ix >= range.start {
                                    let selected = this.selected_category == Some(child.id);
                                    rows.push(this.tree_row(
                                        child,
                                        1,
                                        selected,
                                        palette,
                                        &tree_colors,
                                        cx,
                                    ));
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

        // Rules of the selected category (3.23): the list shows only these
        // (`visible_rules`). The list is virtualized like the tree (3.12);
        // rows are fixed height (24 px), except the one row 3.24's editor
        // swapped in for, which reserves 68 px (labelled fields) — 92 px
        // while a validation error is open under it, one ellipsized line
        // (`rule_editor_row`).
        let editing = self.editing_rule;
        let edit_error = self.rule_error.is_some();
        let visible = Rc::new(self.visible_rules());
        let rule_sizes = Rc::new(
            visible
                .iter()
                .map(|r| {
                    if Some(r.id) == editing {
                        size(px(0.), if edit_error { px(108.) } else { px(80.) })
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

        // Detail panel: the selected category, or the prompt to pick one.
        let detail: AnyElement = match self.selected_node() {
            None => v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .min_h_0()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(border)
                .rounded_lg()
                .debug_selector(|| "cat:detail-empty".into())
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Select a category to see its rules and settings."),
                )
                .into_any_element(),
            Some((node, parent)) => {
                let count = self.node_count(node);
                let name = node.name.clone();
                let kind = node.kind;
                let is_system = node.is_system;
                let is_child = parent.is_some();
                // 3.53: the add-rule row's Category control follows the
                // selection: a subcategory names the target itself (no
                // picker); a parent picks among its own children by their
                // own names; a childless parent gets a hint, not a row.
                let childless_parent = !is_child && node.children.is_empty();
                let sub_picker = if is_child || childless_parent {
                    None
                } else {
                    // Trigger and menu fit the longest child name, up to
                    // 200 px / 320 px (see `subcategory_widths`).
                    let longest = node
                        .children
                        .iter()
                        .map(|c| c.name.chars().count())
                        .max()
                        .unwrap_or(0);
                    let (trigger_w, menu_w) = subcategory_widths(longest);
                    Some(
                        v_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .child(field_label(muted, "Subcategory"))
                            .child(subcategory_select(
                                &self.cat_rule_target,
                                "cat-add",
                                trigger_w,
                                menu_w,
                                cx,
                            ))
                            .into_any_element(),
                    )
                };
                let renaming = self.renaming;
                let confirm_label = if self.confirm_delete_category == Some(node.id) {
                    "Click again to delete"
                } else {
                    "Delete…"
                };
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .min_h_0()
                    .gap_3()
                    .p_4()
                    .border_1()
                    .border_color(border)
                    .rounded_lg()
                    .debug_selector(|| "cat:detail-panel".into())
                    // Title line (3.23): the name, its kind and transaction
                    // count, Rename… at the right end; the rename editor
                    // (3.16) replaces this line in place.
                    .child(if renaming {
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
                        h_flex()
                            .gap_2()
                            .items_center()
                            .debug_selector(|| "cat:detail-title".into())
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .child(div().flex_shrink_0().text_sm().text_color(muted).child(
                                format!(
                                    "{} · {} transactions",
                                    kind_label(kind),
                                    flow::count(count)
                                ),
                            ))
                            .child(
                                Button::new("rename-category")
                                    .label("Rename…")
                                    .disabled(is_system)
                                    .ml_auto()
                                    .flex_shrink_0()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_rename(window, cx)
                                    })),
                            )
                            .into_any_element()
                    })
                    .child(div().w_full().border_t_1().border_color(border))
                    // Organize (3.16's labelled groups): a category gains
                    // a subcategory; a subcategory moves under another.
                    .child(
                        v_flex()
                            .gap_1()
                            .child(region_title("Organize"))
                            .when(!is_child, |section| {
                                section.child(
                                    h_flex()
                                        .gap_2()
                                        .items_end()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(muted, "New subcategory name"))
                                                .child(
                                                    div()
                                                        .w(px(220.))
                                                        .child(Input::new(&self.new_child_name)),
                                                ),
                                        )
                                        .child(
                                            Button::new("add-child")
                                                .label(ADD_SUBCATEGORY)
                                                .debug_selector(move || {
                                                    format!("label:{ADD_SUBCATEGORY}")
                                                })
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.add_child(window, cx)
                                                })),
                                        ),
                                )
                            })
                            .when(is_child, |section| {
                                section.child(
                                    h_flex()
                                        .gap_2()
                                        .items_end()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(muted, "Move to category"))
                                                .child(
                                                    div()
                                                        .w(px(200.))
                                                        .debug_selector(move || {
                                                            format!("label:{MOVE_TO_CATEGORY_PH}")
                                                        })
                                                        .child(
                                                            Select::new(&self.move_target)
                                                                .placeholder(MOVE_TO_CATEGORY_PH),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            Button::new("move-child")
                                                .label("Move")
                                                .disabled(is_system)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.move_child_to_selected_parent(window, cx)
                                                })),
                                        ),
                                )
                            }),
                    )
                    // Rules for this category (3.23): the list below holds
                    // only these. 3.53: the add-rule row carries no
                    // category picker when the selection names the target
                    // (a subcategory); a parent picks among its own
                    // children instead.
                    .child(div().w_full().border_t_1().border_color(border))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .gap_2()
                            .child(region_title("Rules"))
                            // 3.53: a parent with no subcategories can have
                            // no rule at all (rules target a child), so the
                            // hint stands in for the add-rule row.
                            .when(childless_parent, |section| {
                                section.child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .debug_selector(|| "cat:add-rule-need-child".into())
                                        .child("Add a subcategory first"),
                                )
                            })
                            // Add-rule group (3.16): labelled fields; the
                            // pattern column is the flex one, so the row
                            // fits the panel's width.
                            .when(!childless_parent, |section| {
                                section
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_end()
                                            .debug_selector(|| "cat:add-rule-row".into())
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .gap_1()
                                                    .child(field_label(muted, "Pattern"))
                                                    .child(
                                                        div().w_full().debug_selector(|| {
                                                            "rule:pattern-input:cat-add".into()
                                                        }).child(Input::new(&self.rule_pattern)),
                                                    ),
                                            )
                                            .child(
                                                // 3.42: control-height slot, centred.
                                                control_slot(
                                                    "rule:regex-slot:cat-add".into(),
                                                    "rule:switch:cat-add".into(),
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
                                                ),
                                            )
                                            // 3.53: a parent selection adds
                                            // the picker over its own children
                                            // (`sub_picker`, built above); a
                                            // subcategory selection adds no
                                            // picker — the row targets it.
                                            .children(sub_picker)
                                            .child(
                                                Button::new("add-rule")
                                                    .label("Add rule")
                                                    .primary()
                                                    .flex_shrink_0()
                                                    // 3.42: the alignment test measures its centre.
                                                    .debug_selector(|| {
                                                        "rule:add-rule-btn:cat-add".into()
                                                    })
                                                    // 3.40: loading + disabled while the add runs.
                                                    .loading(
                                                        self.rule_action_pending(
                                                            PendingRuleAction::Add,
                                                        ),
                                                    )
                                                    .disabled(
                                                        self.rule_action_pending(
                                                            PendingRuleAction::Add,
                                                        ),
                                                    )
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.add_rule_to_selected(window, cx)
                                                    })),
                                            ),
                                    )
                                    .child(preview)
                            })
                            .children(
                                self.rule_error
                                    .clone()
                                    // 3.24: while a row's editor is open it
                                    // shows the error under itself.
                                    .filter(|_| self.editing_rule.is_none())
                                    .map(|e| div().text_sm().text_color(danger).child(e)),
                            )
                            .when(visible.is_empty(), |section| {
                                section.child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .debug_selector(|| "cat:rules-empty".into())
                                        .child("No rules for this category yet"),
                                )
                            })
                            .when(!visible.is_empty(), |section| {
                                // The headings mirror `rule_row`'s columns
                                // (the queue's pattern, 3.16). Actions sits
                                // on the same line as the other labels,
                                // right-aligned over the buttons: invisible
                                // copies of the row's Edit and Delete… give
                                // the column the cluster's exact width, and
                                // their box is clipped to 0 height so the
                                // heading stays a single line.
                                let heading = h_flex()
                                    .gap_2()
                                    .items_end()
                                    .debug_selector(|| "rule:heading-row".into())
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            field_label(muted, "Pattern")
                                                .debug_selector(|| "rule:heading-pattern".into()),
                                        ),
                                    )
                                    .child(
                                        div().w(px(RULE_TYPE_W)).flex_shrink_0().child(
                                            field_label(muted, "Type")
                                                .debug_selector(|| "rule:heading-type".into()),
                                        ),
                                    )
                                    .child(
                                        div().w(px(RULE_CATEGORY_W)).flex_shrink_0().child(
                                            field_label(muted, "Category")
                                                .debug_selector(|| "rule:heading-category".into()),
                                        ),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_shrink_0()
                                            .child(
                                                field_label(muted, "Actions")
                                                    .text_right()
                                                    .debug_selector(|| {
                                                        "rule:heading-actions".into()
                                                    }),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .h_0()
                                                    .overflow_hidden()
                                                    .child(
                                                        Button::new("rule-heading-edit-spacer")
                                                            .label("Edit")
                                                            .ghost()
                                                            .text_xs()
                                                            .opacity(0.0),
                                                    )
                                                    .child(
                                                        Button::new("rule-heading-delete-spacer")
                                                            .label("Delete…")
                                                            .ghost()
                                                            .text_xs()
                                                            .opacity(0.0),
                                                    ),
                                            ),
                                    );
                                section.child(heading).child(
                                    v_virtual_list(
                                        cx.entity(),
                                        "rule-list",
                                        rule_sizes,
                                        move |this, range, _window, cx| {
                                            let rows: Vec<AnyElement> = visible
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
                                )
                                // 3.53: the editor changes pattern and kind
                                // only — moving a rule to another category
                                // is the Rules page Move bar's job (3.28).
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(muted)
                                        .debug_selector(|| "cat:rules-move-hint".into())
                                        .child(
                                            "To move rules to another category, use the Rules page.",
                                        ),
                                )
                            }),
                    )
                    // Delete (3.16's labelled group): the transactions move
                    // first; system categories can't be deleted, so their
                    // detail has no Delete section at all.
                    .when(!is_system, |detail| {
                        detail
                            .child(div().w_full().border_t_1().border_color(border))
                            .child(
                                v_flex().gap_1().child(region_title("Delete")).child(
                                    h_flex()
                                        .gap_2()
                                        .items_end()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .child(field_label(
                                                    muted,
                                                    "Move its transactions to",
                                                ))
                                                .child(
                                                    div()
                                                        .w(px(260.))
                                                        .debug_selector(move || {
                                                            format!("label:{MOVE_TXNS_TO_PH}")
                                                        })
                                                        .child(
                                                            Select::new(&self.delete_target)
                                                                .placeholder(MOVE_TXNS_TO_PH),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            Button::new("delete-category")
                                                .label(confirm_label)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.arm_delete_category(window, cx)
                                                })),
                                        ),
                                ),
                            )
                    })
                    .into_any_element()
            }
        };

        // The header count paints `count_text` and its selector mirrors it
        // (3.33): the scan test asserts the exact words and cannot be
        // satisfied by a retired one. 3.23: it counts categories and
        // subcategories apart.
        let n_subcategories: usize = self.categories.iter().map(|p| p.children.len()).sum();
        let count_text = format!(
            "{} categories · {} subcategories",
            self.categories.len(),
            n_subcategories
        );
        let count_sel = count_text.clone();

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            // Page header (3.23): title, count summary, and the primary
            // action that opens the new-category form in place.
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
                            .debug_selector(move || format!("label:{count_sel}"))
                            .child(count_text),
                    )
                    .child(
                        Button::new("new-category")
                            .label(NEW_CATEGORY)
                            .debug_selector(move || format!("label:{NEW_CATEGORY}"))
                            .primary()
                            .ml_auto()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.new_category_open = true;
                                cx.notify();
                            })),
                    ),
            )
            .children(
                self.cat_error
                    .clone()
                    .map(|e| div().text_sm().text_color(danger).child(e)),
            )
            // New-category form (3.23): hidden until the header button
            // opens it; Cancel and a successful add close it.
            .when(self.new_category_open, |page| {
                page.child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .p_2()
                        .border_1()
                        .border_color(border)
                        .rounded_lg()
                        .debug_selector(|| "cat:new-form".into())
                        .child(
                            v_flex()
                                .gap_1()
                                .child(field_label(muted, "New category name"))
                                .child(div().w(px(220.)).child(Input::new(&self.new_parent_name))),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(field_label(muted, "Kind"))
                                .child(div().w(px(140.)).child(Select::new(&self.new_parent_kind))),
                        )
                        .child(
                            Button::new("cancel-new-category")
                                .label("Cancel")
                                .debug_selector(|| "cat:new-form-cancel".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.new_category_open = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("add-parent")
                                .label(ADD_CATEGORY)
                                .debug_selector(move || format!("label:{ADD_CATEGORY}"))
                                .primary()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_parent(window, cx)),
                                ),
                        ),
                )
            })
            // Body: the tree panel and the detail panel side by side.
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(tree)
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_queue(&mut self, cx: &mut Context<Self>) -> AnyElement {
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
            flow::count(self.queue.iter().map(|g| g.count).sum()),
            flow::count(self.queue.len() as i64)
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
        let body: AnyElement = if self.queue.is_empty() {
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
            let sizes = Rc::new(vec![size(px(0.), px(40.)); self.queue.len()]);
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
                            let groups = this.queue[range].to_vec();
                            let last = this.queue.len().saturating_sub(1);
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
                            this.queue_rows_built.set(rows.len());
                            #[cfg(test)]
                            this.queue_range.set(range_bounds);
                            rows
                        },
                    )
                    // 3.21: Shell-owned handle (see the tree).
                    .track_scroll(&self.queue_scroll)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground) = {
            let theme = cx.theme();
            (theme.background, theme.foreground)
        };
        // 3.28: opening the Rules page focuses its search box (D5). The
        // focus waits for the end of the frame so the input's element
        // exists first.
        if self.rules_focus_pending.take() {
            let handle = self.rules_search.focus_handle(cx);
            window.defer(cx, move |window, cx| {
                window.focus(&handle, cx);
            });
        }

        let page_area: AnyElement = match self.page {
            Page::Dashboard => self.render_dashboard(cx),
            Page::ToCategorize => self.render_queue(cx),
            Page::Categories => self.render_categories(cx),
            Page::Rules => self.render_rules(cx),
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
            .relative()
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
                    .child(page_area),
            )
            // 3.27: feedback toasts overlay the window (absolute, so no
            // layout space) instead of a status line pushing the page down.
            .child(self.render_toasts(cx))
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
