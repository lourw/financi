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
mod dashboard_chart;
mod dashboard_filters;
mod imports;
mod queue;
mod rules;
mod sidebar;
mod transfers;
mod widgets;

use self::categories::*;
use self::dashboard::*;
use self::imports::*;
use self::queue::*;
use self::rules::*;
use self::transfers::*;
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
    // -- Shared data (3.74): the lists every page renders from. Moves to
    // `Catalog` in 3.78.
    accounts: Vec<Account>,
    page: Page,
    // -- Responsive chart (3.15): the window's current width in pixels,
    // tracked with `observe_window_bounds`, so the chart can shorten and
    // thin its band labels to the space left beside the legend, and the
    // two tables (3.20, 3.25) can flex their columns.
    window_width: f32,
    // -- Sidebar counts: the visible and uncategorized transaction totals
    // (the Transfers pill counts `transfers.transfer_reviews`).
    total_visible: i64,
    uncategorized_visible: i64,
    categories: Vec<CategoryNode>,
    cat_counts: HashMap<i64, i64>,
    rules: Vec<Rule>,
    child_choices: Vec<(i64, String)>,
    /// 3.29: the id of the system `Transfers > Card Payment` category, read
    /// off the category tree on every refresh (same name lookup the DB uses
    /// in [`financi_db::categories::card_payment_category`]). `None` only
    /// before the first refresh.
    card_payment_category: Option<i64>,
    // -- The add-rule row and its in-place editor (3.24, 3.28): rendered by
    // both the Categories page and the Rules page, so this state stays
    // shared until the rule data moves to `Catalog` (3.78).
    rule_pattern: Entity<InputState>,
    rule_regex: bool,
    rule_target: Entity<SelectState<Names>>,
    /// The add row's and the editor's shared "matches N transactions"
    /// preview (the editor owns it while open; see [`Shell::preview_rule_match`]).
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
    rule_edit_pattern: Entity<InputState>,
    rule_edit_regex: bool,
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
    /// 3.75: idle markers for the tests. `refresh_started` counts calls to
    /// `refresh` (the generation number); `refresh_applied` holds the
    /// generation of the last refresh whose result was applied — the shell
    /// is idle once the two agree (`wait_idle`). A refresh replaced by a
    /// later one is cancelled, which is why the continuation stamps rather
    /// than counts.
    refresh_started: u64,
    refresh_applied: u64,
    /// 3.64: there is deliberately no shared task slot. A mutation whose
    /// result must always be applied runs detached (`cx.spawn_in(..).detach()`)
    /// so a later action can never cancel it mid-flight; its handler checks
    /// the state it touches is still current before applying. Only work that
    /// *should* be replaced when the user moves on keeps a slot — one per
    /// purpose: `refresh_task`, `rule_task` (live preview), `toast_task`, and
    /// the settings writes in `dashboard`.
    // -- Page state (3.74): each page's fields live with its page module,
    // built by its own `new` and updated by its own `apply_*` in the
    // `refresh` continuation.
    dashboard: DashboardState,
    queue: QueueState,
    categories_page: CategoriesState,
    rules_page: RulesPageState,
    transfers: TransfersState,
    imports: ImportsState,
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
        if self.rules_page.rules_focus_pending.take() {
            let handle = self.rules_page.rules_search.focus_handle(cx);
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
        Self::wire_file_actions(&cx.entity(), window.window_handle(), cx);
        // The add-rule row and its editor (3.24, 3.28): rendered by both
        // the Categories page and the Rules page, so their controls stay on
        // `Shell` (3.78 moves them with the rule data).
        let rule_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        let rule_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        // 3.24: the editor's Pattern input, separate from the add row's.
        let rule_edit_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        // The chart's label plan follows the window width (3.15), and so
        // do the Imports history table's (3.20) and the dashboard table's
        // flexible columns (3.25).
        let bounds_sub = cx.observe_window_bounds(window, |this, window, cx| {
            this.window_bounds_changed(window, cx);
        });
        // Each page's state is built by its own constructor (3.74).
        let dashboard = DashboardState::new(window, cx);
        let queue = QueueState::new(window, cx);
        let categories_page = CategoriesState::new(window, cx);
        let rules_page = RulesPageState::new(window, cx);
        let transfers = TransfersState::new(window, cx);
        let imports = ImportsState::new(window, cx);
        let mut shell = Self {
            db,
            db_path,
            csv_root,
            accounts: Vec::new(),
            page: Page::Dashboard,
            window_width: window.bounds().size.width.as_f32(),
            total_visible: 0,
            uncategorized_visible: 0,
            categories: Vec::new(),
            cat_counts: HashMap::new(),
            rules: Vec::new(),
            child_choices: Vec::new(),
            card_payment_category: None,
            rule_pattern,
            rule_regex: false,
            rule_target,
            rule_match_count: None,
            rule_task: None,
            rule_error: None,
            editing_rule: None,
            armed_delete_rule: None,
            pending_rule_action: None,
            rule_edit_pattern,
            rule_edit_regex: false,
            toasts: ToastManager::new(ToastMotion::sonner()),
            toast_stack: ToastStackState::default(),
            toast_next_id: 0,
            toast_clock: Instant::now(),
            toast_task: None,
            refresh_task: None,
            refresh_started: 0,
            refresh_applied: 0,
            dashboard,
            queue,
            categories_page,
            rules_page,
            transfers,
            imports,
            _subs: vec![bounds_sub],
        };
        // The live "matches N transactions" preview runs on every pattern
        // keystroke (ui-conventions, Categories page).
        shell._subs.extend(shell.subscribe_rule_row(window, cx));
        shell.load_date_range(window, cx);
        shell.load_include_transfers(window, cx);
        shell.refresh(window, cx);
        shell
    }

    /// The file actions are handled globally, not on the root element:
    /// the native menu dispatches them without a focused element, and
    /// `App::is_action_available` — what macOS uses to enable menu
    /// items — only counts global listeners in that case.
    fn wire_file_actions(shell: &Entity<Shell>, window: AnyWindowHandle, cx: &mut App) {
        Self::on_shell_action(shell, window, cx, |this, _: &ImportCsv, window, cx| {
            this.pick_file(window, cx);
        });
        Self::on_shell_action(shell, window, cx, |this, _: &UndoLastImport, window, cx| {
            this.undo_last_import(window, cx);
        });
        Self::on_shell_action(shell, window, cx, |this, _: &ExportData, _, cx| {
            this.export_data(cx);
        });
        Self::on_shell_action(shell, window, cx, |this, _: &RestoreBackup, _, cx| {
            this.restore_backup(cx);
        });
        Self::on_shell_action(shell, window, cx, |_, _: &QuitApp, _, cx| cx.quit());
    }

    /// A window resize re-flexes the two tables' columns (3.20, 3.25) and
    /// re-renders for the chart's label plan (3.15).
    fn window_bounds_changed(&mut self, window: &mut Window, cx: &mut Context<Shell>) {
        let width = window.bounds().size.width.as_f32();
        if self.window_width == width {
            return;
        }
        self.window_width = width;
        let page_w = (width - SIDEBAR_W - 2. * PAGE_PAD).max(0.);
        let flex = history_flex_widths(page_w);
        self.imports.history_table.update(cx, |t, cx| {
            t.delegate_mut().flex_widths = flex;
            // Re-read the delegate's columns with the new widths.
            t.refresh(cx);
        });
        let flex = tx_flex_widths(page_w);
        self.dashboard.table.update(cx, |t, cx| {
            t.delegate_mut().flex_widths = flex;
            t.refresh(cx);
        });
        cx.notify();
    }

    /// Re-query everything the pages show: accounts and transactions for
    /// the dashboard, sidebar counts, and the page lists. The shared lists
    /// are set here; each page then applies its own part of the result in
    /// its `apply_*` (3.74), keeping this one DB round-trip.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        // The account filter resolves against the accounts loaded *inside*
        // this refresh (task 3.17): an import can add accounts while every
        // existing checkbox is unchecked, and reading `self.accounts` here
        // would read "nothing selected" from the previous refresh and blank
        // every view until some other filter changes.
        let excluded = self.dashboard.excluded_accounts.clone();
        let mut view_filter = self.dashboard.view_filter.clone();
        view_filter.include_transfers = self.dashboard.include_transfers;
        // The table-only filters (3.4): search, the two toggles and the sort,
        // folded into the query alongside the shared date-range/accounts/
        // transfers filter. The category slot carries the legend selection
        // (3.5), shared with the summary cells (3.6); the `Uncategorized`
        // legend row rides `uncategorized_only`. The month is the summary
        // cell selection (3.6).
        let table_filter = TxTableFilter {
            category_id: match self.dashboard.legend_selection {
                Some(LegendSelection::Category(id)) => Some(id),
                Some(LegendSelection::Uncategorized) | None => None,
            },
            month: self.dashboard.summary_month.clone(),
            search: if self.dashboard.search.is_empty() {
                None
            } else {
                Some(self.dashboard.search.clone())
            },
            uncategorized_only: self.dashboard.uncategorized_only
                || matches!(
                    self.dashboard.legend_selection,
                    Some(LegendSelection::Uncategorized)
                ),
            show_hidden: self.dashboard.show_hidden,
            sort: self.dashboard.tx_sort,
        };
        let chart_parent = self.dashboard.chart_parent;
        // 3.75: generation counter for `wait_idle` in the tests. A refresh
        // replaced before it completes is cancelled and its continuation
        // never runs, so the continuation stamps its own generation into
        // `refresh_applied`; the two agree only once the *latest* refresh
        // has landed.
        self.refresh_started += 1;
        let generation = self.refresh_started;
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
                        // Shared data (3.74): the lists every page renders
                        // from.
                        this.accounts = accounts;
                        // Keep the shared `ViewFilter` in step with the
                        // checkboxes: empty means all accounts.
                        this.sync_account_filter();
                        // Then each page applies its own part of the result.
                        this.apply_imports(history, cx);
                        this.apply_categories(tree, counts);
                        this.apply_rules(rules, uses);
                        this.apply_queue(queue);
                        this.apply_transfers(reviews);
                        this.sync_category_choices(window, cx);
                        this.sync_queue(window, cx);
                        this.apply_dashboard(
                            rows,
                            chart_rows,
                            summary,
                            (total, uncategorized),
                            window,
                            cx,
                        );
                    }
                    Err(e) => this.notify_err(format!("Database error: {e}"), cx),
                }
                this.refresh_applied = generation;
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
        self.imports.confirm_undo_import = None;
        self.dashboard.rule_offer = None;
        self.categories_page.renaming = false;
        self.categories_page.new_category_open = false;
        self.categories_page.confirm_delete_category = None;
        self.close_rule_editor();
        self.armed_delete_rule = None;
        // 3.28: the Rules page's add row, ticked rows and move error belong
        // to that page too; entering it focuses the search box (D5).
        // 3.29: so does the *Show rule* highlight — it belongs to the row
        // the menu was opened on.
        self.rules_page.new_rule_open = false;
        self.rules_page.selected_rules.clear();
        self.rules_page.rules_move_error = None;
        self.rules_page.rules_reveal = None;
        self.rules_page.rules_focus_pending.set(page == Page::Rules);
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
