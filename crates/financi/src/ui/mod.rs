//! GPUI views. Thin by design: logic lives in `flow` and `financi-db`, and
//! every database call goes through the [`DbWorker`] thread.
//!
//! The window is a [`Sidebar`] of pages and one page area, per
//! `docs/ui-conventions.md` (*Window layout*). The pages live in sibling
//! modules of this one (3.62): `widgets` (shared helpers and toasts),
//! `sidebar`, `dashboard`, `queue`, `categories`, `rules`, `transfers`
//! and `imports`.

mod categories;
mod dashboard;
mod imports;
mod queue;
mod rules;
mod sidebar;
mod transfers;
mod widgets;

use self::categories::*;
use self::dashboard::*;
use self::imports::*;
use self::rules::*;
use self::widgets::*;

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
    add_rule, count_rule_matches, delete_rule, list_rules, move_rules, rule_use_counts, update_rule,
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
mod tests;

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
                // each rule's match decides).
                let uses = rule_use_counts(c)?;
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
}
