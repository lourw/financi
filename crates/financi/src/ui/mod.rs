//! GPUI views. Thin by design: logic lives in `flow` and `financi-db`, and
//! every database call goes through the [`DbWorker`] thread.
//!
//! The window is a [`Sidebar`] of pages and one page area, per
//! `docs/ui-conventions.md` (*Window layout*). The pages live in sibling
//! modules of this one (3.62): `widgets` (shared helpers and toasts),
//! `sidebar`, `dashboard`, `queue`, `categories`, `rules`, `transfers`
//! and `imports`.

mod catalog;
mod categories;
mod dashboard;
mod dashboard_chart;
mod dashboard_filters;
mod imports;
mod page;
mod queue;
mod rules;
mod sidebar;
mod transfers;
mod widgets;

use self::categories::*;
use self::dashboard::*;
use self::imports::*;
use self::page::{Cancel, PageEntity as _, PageEvent};
use self::queue::*;
use self::rules::*;
use self::transfers::*;
use self::widgets::*;

use std::cell::Cell;
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
    Account, CategorySpend, DbError, DbWorker, MonthlySummary, TransferReview, TxSort,
    TxSortColumn, TxTableFilter, ViewFilter, apply_rule_from_merchant, create_account,
    list_accounts, list_transactions_filtered, list_transfer_reviews, monthly_summary,
    resolve_transfer_review, rusqlite::Connection, set_category, set_hidden, sidebar_counts,
    spend_by_category, unpair_transaction,
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
// 3.78: the shared data model. The page modules reach `Domains` through
// the `use super::*` they all carry.
use catalog::{
    Catalog, DataChanged, Domain, Domains, S_ACCOUNTS, S_CATEGORIES, S_QUEUE, S_RULES, S_SIDEBAR,
};

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

/// One-time global UI wiring (3.76): the key bindings, the macOS menu bar
/// and the global file-action listeners. `main` calls this once before the
/// first window; the test harness calls it once per test `App`.
///
/// It is idempotent because `App::on_action` listeners can never be
/// removed: registering them per `Shell` (as before) doubled every ⌘
/// action once a second window existed, and each listener kept its
/// `Shell` alive forever. They are registered here once and resolve the
/// window's `Shell` at dispatch time (see [`route_to_shell`]).
pub fn init(cx: &mut App) {
    if cx.try_global::<UiInitialized>().is_some() {
        return;
    }
    // Esc clears the dashboard's category selection (3.5); the root view
    // handles the action in `Render::render`. Scoped to the kit root's
    // `Root` context (3.80): gpui resolves a context-less binding at the
    // deepest context, so an unscoped Escape would outrank the pages'
    // `Cancel` bindings whenever focus sits inside one (a context-bound
    // binding only wins at or above the depth it names) — and the pages
    // focus their inputs. `Root` bottoms out every window's context
    // stack, so this still reaches the pages that have no context of
    // their own, and any page context (depth ≥ 2) wins over it.
    cx.bind_keys([KeyBinding::new(
        "escape",
        ClearCategorySelection,
        Some("Root"),
    )]);
    // 3.79: Escape is each extracted page's own `Cancel` on its key
    // context (the page root handles it in its `Render::render`); the
    // `Root`-scoped binding above still reaches the pages that have not
    // been extracted, and each page's context outranks it for its own.
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some("Transfers")),
        KeyBinding::new("escape", Cancel, Some("Imports")),
        KeyBinding::new("escape", Cancel, Some("Rules")),
    ]);
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
    cx.on_action(|_: &ImportCsv, cx| {
        route_to_shell(cx, |this, window, cx| this.pick_file(window, cx));
    });
    cx.on_action(|_: &UndoLastImport, cx| {
        route_to_shell(cx, |this, window, cx| this.undo_last_import(window, cx));
    });
    cx.on_action(|_: &ExportData, cx| {
        route_to_shell(cx, |this, window, cx| this.export_data(window, cx));
    });
    cx.on_action(|_: &RestoreBackup, cx| {
        route_to_shell(cx, |this, window, cx| this.restore_backup(window, cx));
    });
    cx.on_action(|_: &QuitApp, cx| cx.quit());
    cx.set_global(UiInitialized);
}

struct UiInitialized;

impl Global for UiInitialized {}

/// Run `handler` on the `Shell` of the window that owns the action —
/// the active window's, resolved from its root view at dispatch time and
/// never stored, so nothing keeps a closed or replaced `Shell` alive
/// (3.76). A window whose root is not the `Shell` (the startup-error
/// window, a popover left open over nothing) simply has no file actions.
///
/// Global listeners run inside the window's update loop, so the work is
/// deferred out of it (same pattern as `Window::dispatch_action`). The
/// test platform never activates a window, so with no active window the
/// application's own windows are tried in turn.
fn route_to_shell(
    cx: &mut App,
    handler: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) {
    cx.defer(move |cx| {
        let candidates: Vec<AnyWindowHandle> = match cx.active_window() {
            Some(window) => vec![window],
            None => cx.windows(),
        };
        for window in candidates {
            let handled = window
                .update(cx, |root, window, cx| {
                    // gpui-kit windows wrap the app view in the kit's
                    // `Root`, so unwrap two layers: Root, then `Shell`.
                    let Ok(root) = root.downcast::<gpui_kit::base::Root>() else {
                        return false;
                    };
                    let Some(shell) = root.read(cx).view().clone().downcast::<Shell>().ok() else {
                        return false;
                    };
                    shell.update(cx, |this, cx| handler(this, window, cx));
                    true
                })
                .unwrap_or(false);
            if handled {
                return;
            }
        }
    });
}

/// A minimal window naming a startup failure (3.76). It is what the user
/// sees when the database could not be opened, so it must need nothing
/// from the database: themed text and one button.
struct StartupError {
    message: SharedString,
}

impl Render for StartupError {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().child("financi could not start"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(self.message.clone()),
                    ),
            )
            .child(
                Button::new("startup-error-close")
                    .label("Close")
                    .primary()
                    .on_click(|_, window, _cx| window.remove_window()),
            )
    }
}

/// Open the startup-error window (3.76) with `message`. Used by `main`
/// when the database could not be opened, so the reason is on screen and
/// not only in the terminal and `<data dir>/financi.log`.
pub fn open_error_window(message: impl Into<SharedString>, cx: &mut App) {
    let message = message.into();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(480.), px(240.)), cx)),
        window_min_size: Some(size(px(360.), px(180.))),
        ..Default::default()
    };
    let _ = cx.open_window(options, move |_window, cx| {
        cx.new(|_cx| StartupError { message })
    });
}

pub struct Shell {
    db: DbWorker,
    db_path: PathBuf,
    csv_root: PathBuf,
    /// 3.78: the shared data model. Every reload runs there; the fields
    /// below mirror its snapshot for the pages until each page is its own
    /// view (3.79–3.83), updated in one place — [`Shell::data_changed`].
    catalog: Entity<Catalog>,
    // -- Shared data (3.74): the lists every page renders from. 3.78
    // moved the loading to `Catalog`; these are its mirrors, dropped when
    // the pages own their data (3.79–3.83).
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
    // 3.80: the add-rule row and its in-place editor moved to `RulesPage`,
    // which also owns the list's search, filters, sort and checked rows.
    // The Categories page renders the shared rule rows through thin
    // wrappers here (see `ui/rules.rs`).
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
    /// 5.3 review fix: `exporting || restoring` as a cell shared with
    /// every [`ImportPanel`] and with the Imports page, so no UI write
    /// dispatch — panel commits included — queues behind an archive job.
    /// A queued write may name rows the restore replaces (dense ids land
    /// on different rows). The job flags themselves live on the Imports
    /// page (3.79), which renders the busy header buttons.
    writes_blocked: Rc<Cell<bool>>,
    /// 3.75: idle markers for the tests. `refresh_started` counts every
    /// reload dispatched — the full `refresh` and the partial `changed`
    /// calls alike (3.78); `refresh_applied` holds the generation of the
    /// last reload whose result was applied — the shell is idle once the
    /// two agree (`wait_idle`). A reload replaced by a later one emits
    /// nothing, which is why the subscriber stamps the maximum rather
    /// than counting events.
    refresh_started: u64,
    /// 3.78 review fix: the first dispatch generation whose result may
    /// drop the category selection. [`Shell::select_category`] sets it to
    /// the generation of the reload that follows the selection, so a
    /// reload whose queries ran before that selection — and which lands
    /// after it, the queues carrying no order between them — cannot clear
    /// it (or an add's claim of it) as "stale".
    category_selection_settled: u64,
    refresh_applied: u64,
    /// 3.56: the dispatch generation of the newest dashboard load. A
    /// result whose dispatch is older than this was overtaken — the newer
    /// load's result replaces it, so the stale one is dropped whole.
    dash_gen: u64,
    /// 3.64: there is deliberately no shared task slot. A mutation whose
    /// result must always be applied runs detached (`cx.spawn_in(..).detach()`)
    /// so a later action can never cancel it mid-flight; its handler checks
    /// the state it touches is still current before applying. Only work that
    /// *should* be replaced when the user moves on keeps a slot — one per
    /// purpose: `toast_task`, the settings writes in `dashboard`, and the
    /// Rules page's live-preview task; the reload slots live in
    /// `Catalog`.
    // -- Page state (3.74): each page's fields live with its page module,
    // built by its own `new` and updated by its own `apply_*` in the
    // `refresh` continuation. 3.79: the Transfers and Imports pages are
    // entities of their own — they own their state, render themselves, and
    // refresh from the `Catalog` (see `ui/page.rs`); `Shell` navigates
    // them with `set_visible` and hears from them through `PageEvent`.
    // 3.80: the Rules page joins them.
    dashboard: DashboardState,
    queue: QueueState,
    categories_page: CategoriesState,
    rules_page: Entity<RulesPage>,
    transfers: Entity<TransfersPage>,
    imports: Entity<ImportsPage>,
    _subs: Vec<Subscription>,
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground) = {
            let theme = cx.theme();
            (theme.background, theme.foreground)
        };
        // 3.28: opening the Rules page focuses its search box (D5) — the
        // page itself defers the focus in its own `render` (3.80).

        let page_area: AnyElement = match self.page {
            Page::Dashboard => self.render_dashboard(cx),
            Page::ToCategorize => self.render_queue(cx),
            Page::Categories => self.render_categories(cx),
            // 3.80: the Rules page renders itself.
            Page::Rules => self.rules_page.clone().into_any_element(),
            // 3.79: the extracted pages render themselves — the entity is
            // the element.
            Page::Transfers => self.transfers.clone().into_any_element(),
            Page::Imports => self.imports.clone().into_any_element(),
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
            // is registered in `ui::init`.
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
        // 3.76: the global wiring (key bindings, menus, file-action
        // listeners) lives in `ui::init` now — once per app, not once per
        // window.
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
        // 3.78: one shared data model, fed by the same worker.
        let catalog = cx.new(|_| Catalog::new(db.clone()));
        // 3.79: the extracted pages are entities watching that model. Both
        // start hidden and dirty; the `DataChanged` from the launch
        // `refresh` below leaves them dirty (hidden), and the first visit
        // applies.
        let writes_blocked = Rc::new(Cell::new(false));
        let transfers = cx
            .new(|cx| TransfersPage::new(db.clone(), writes_blocked.clone(), catalog.clone(), cx));
        let imports = cx.new(|cx| {
            ImportsPage::new(
                db.clone(),
                csv_root.clone(),
                writes_blocked.clone(),
                &catalog,
                window,
                cx,
            )
        });
        // 3.80: the Rules page entity. Built after `catalog` so its
        // watcher subscribes before the shell's below — on a reload the
        // page applies before `sync_category_choices` refills its pickers.
        let rules_page =
            cx.new(|cx| RulesPage::new(db.clone(), writes_blocked.clone(), &catalog, window, cx));
        // 3.79: the one page → `Shell` listener: toasts, reloads, page
        // resets and navigation arrive as `PageEvent`s on the page's own
        // entity, never as a call into `Shell`.
        let transfers_events = cx.subscribe_in(&transfers, window, |this, _, ev, window, cx| {
            this.on_page_event(ev, window, cx);
        });
        let imports_events = cx.subscribe_in(&imports, window, |this, _, ev, window, cx| {
            this.on_page_event(ev, window, cx);
        });
        let rules_events = cx.subscribe_in(&rules_page, window, |this, _, ev, window, cx| {
            this.on_page_event(ev, window, cx);
        });
        // 3.80: the Categories page paints the shared rule rows from the
        // Rules page's state, so a page notify repaints the shell.
        let rules_paint = cx.observe(&rules_page, |_, _, cx| cx.notify());
        let mut shell = Self {
            db,
            db_path,
            csv_root,
            catalog: catalog.clone(),
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
            toasts: ToastManager::new(ToastMotion::sonner()),
            toast_stack: ToastStackState::default(),
            toast_next_id: 0,
            toast_clock: Instant::now(),
            toast_task: None,
            writes_blocked,
            refresh_started: 0,
            category_selection_settled: 0,
            refresh_applied: 0,
            dash_gen: 0,
            dashboard,
            queue,
            categories_page,
            rules_page,
            transfers,
            imports,
            _subs: vec![
                bounds_sub,
                transfers_events,
                imports_events,
                rules_events,
                rules_paint,
            ],
        };
        // 3.78: the one mapping. Every reload the catalog finishes lands
        // here, and this applies the page sections its domains touched.
        shell._subs.push(
            cx.subscribe_in(&catalog, window, |this, _, event, window, cx| {
                this.data_changed(event, window, cx);
            }),
        );
        // 3.65: exactly one refresh at launch. The saved date range and
        // transfers setting load inside its query (see `refresh`), so the
        // first data frame already uses them and nothing re-queries.
        shell.refresh(window, cx);
        shell
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
        // 3.79: the Imports history table lives on its page entity now.
        self.imports
            .update(cx, |page, cx| page.widths_changed(page_w, cx));
        let flex = tx_flex_widths(page_w);
        self.dashboard.table.update(cx, |t, cx| {
            t.delegate_mut().flex_widths = flex;
            t.refresh(cx);
        });
        cx.notify();
    }

    /// Re-query everything the pages show — the full reload, plus the
    /// dashboard's own load alongside it (3.56). 3.78 left it two callers:
    /// the launch load and the import / undo / restore flows, where every
    /// part of the read model really is suspect. Every other write reports
    /// its [`Domain`]s to [`Shell::changed`], which reloads only the
    /// sections they touch.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 3.65: the very first load also reads the saved settings in the
        // same round trip — the date range and the transfers checkbox.
        // That read moved to the dashboard's load (3.56), whose queries
        // run with the saved values applied; the landing puts them on the
        // state and the filter controls, so launch is still one round
        // trip whose first frame shows the saved range.
        let startup = std::mem::take(&mut self.dashboard.startup_settings);
        // 3.76: the master-CSV self-check rides this job when it is the
        // launch one, so startup stays a single worker job and the window
        // shows at once.
        let csv_root = startup.then(|| self.csv_root.clone());
        let generation = self.next_generation();
        self.catalog.update(cx, |catalog, cx| {
            catalog.reload_full(Domains::ALL, generation, csv_root, window, cx);
        });
        // 3.56: the dashboard's rows, chart and summary ride their own
        // keyed job, started alongside; `None` makes it read the account
        // list itself, so launch and the after-an-import reload resolve
        // the checkboxes in the same round trip (3.17).
        self.start_dashboard(None, startup, window, cx);
    }

    /// 3.75: generation counter for `wait_idle` in the tests. Every reload
    /// dispatch takes the next number; the subscriber stamps the newest
    /// one that has landed into `refresh_applied`, and a reload replaced
    /// before it finishes never emits at all.
    fn next_generation(&mut self) -> u64 {
        self.refresh_started += 1;
        self.refresh_started
    }

    /// A write landed, or a dashboard control changed: report the
    /// [`Domain`]s it changed (the table on [`Domain`]) and reload only
    /// the sections they touch (3.78). This is the entry point the page
    /// handlers call instead of the old whole-screen `refresh`; a page
    /// switch never calls it, so switching pages issues no queries.
    ///
    /// 3.56: the dashboard is reloaded here too whenever a domain it
    /// reads changes — its own keyed job takes its rows, chart and
    /// summary, so a filter change never runs anyone else's queries and a
    /// burst of changes leaves the newest one in charge.
    fn changed(&mut self, domains: Domains, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.next_generation();
        self.catalog.update(cx, |catalog, cx| {
            catalog.changed(domains, generation, window, cx);
        });
        if Self::dash_touched(domains) {
            // The job resolves the checkboxes against the account list.
            // A write that can have changed the list (a new account, an
            // import) makes the job read it fresh; otherwise the mirror
            // on screen is current and goes along for the ride.
            let stale_accounts =
                domains.contains(Domain::Accounts) || domains.contains(Domain::Imports);
            let accounts = if stale_accounts {
                None
            } else {
                Some(self.accounts.clone())
            };
            self.start_dashboard(accounts, false, window, cx);
        }
    }

    /// Does a change to `domains` move the dashboard's rows, chart or
    /// summary? The rows answer to almost everything; the transfer
    /// reviews do not appear on the dashboard at all, so pairing never
    /// re-runs its queries.
    fn dash_touched(domains: Domains) -> bool {
        domains.contains(Domain::Accounts)
            || domains.contains(Domain::Categories)
            || domains.contains(Domain::Rules)
            || domains.contains(Domain::Transactions)
            || domains.contains(Domain::Imports)
            || domains.contains(Domain::Settings)
    }

    /// 3.56: start the dashboard's one load — filtered rows, chart and
    /// summary in a single keyed job (3.77: a burst of filter changes
    /// leaves at most one in flight plus the newest, and every replaced
    /// job resolves `Superseded` without touching the screen). The filter
    /// is snapshotted now, so the load runs with exactly what the
    /// controls showed when it started.
    ///
    /// `accounts`: the shell's mirror with a filter-control change;
    /// `None` reads the list in the job (launch, imports — 3.17).
    /// `startup` reads the persisted settings in the same trip (3.65).
    pub(super) fn start_dashboard(
        &mut self,
        accounts: Option<Vec<Account>>,
        startup: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_generation();
        self.dash_gen = generation;
        let filter = self.dashboard.filter.clone();
        let db = self.db.clone();
        let log = db.query_log();
        // Detached (3.64): nothing may cancel the continuation between
        // the query finishing and the result being guarded in.
        cx.spawn_in(window, async move |this, cx| {
            let result = db
                .call_keyed("dashboard", move |c| {
                    load_dashboard(c, &log, filter, accounts, startup)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(data) => this.finish_dashboard(generation, data, window, cx),
                // A newer filter change took over while this ran; its
                // result replaces this one's.
                Err(DbError::Superseded) => {}
                Err(e) => {
                    this.notify_err(format!("Database error: {e}"), cx);
                    this.refresh_applied = this.refresh_applied.max(generation);
                }
            });
        })
        .detach();
    }

    /// A dashboard load came back: apply it, unless a newer load was
    /// dispatched while it ran — then this result is stale whole, rows
    /// included, and the newer load's landing replaces it (3.56).
    fn finish_dashboard(
        &mut self,
        generation: u64,
        mut data: DashboardData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if generation < self.dash_gen {
            return;
        }
        // 3.65: the launch settings the job already queried with land on
        // the state and the filter controls before the data of the same
        // round trip is applied, so the first frame shows the saved range
        // everywhere. The guard above is what "the user changed it since,
        // and their change wins" means here: a newer dispatch replaced
        // this load, and its result carries the newer controls.
        if let Some((range, include_transfers)) = data.settings {
            self.set_date_range(range, window, cx);
            self.dashboard.filter.include_transfers = include_transfers;
            data.view_filter.include_transfers = include_transfers;
        }
        self.apply_dashboard_rows(data, window, cx);
        self.refresh_applied = self.refresh_applied.max(generation);
        cx.notify();
    }

    /// The one mapping (3.78): a reload settled, so mirror the snapshot's
    /// reloaded sections into the state the pages render from and run the
    /// pages' `apply_*` for exactly those sections — the order the old
    /// single `refresh` continuation used.
    fn data_changed(&mut self, event: &DataChanged, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = &event.error {
            self.notify_err(format!("Database error: {e}"), cx);
        }
        let sections = Catalog::sections_for(event.domains);
        // Each read of the snapshot is short-lived: it borrows `cx`, and
        // the `apply_*` calls below need `cx` mutably.
        if sections & S_ACCOUNTS != 0 {
            self.accounts = self.catalog.read(cx).snap.accounts.clone();
            // The account filter resolves against the accounts just
            // loaded (task 3.17).
            self.sync_account_filter();
        }
        // 3.56: the launch *settings* land with the dashboard's own load
        // (`finish_dashboard`); only the master-CSV self-check failure
        // still rides the full reload's event.
        if let Some(e) = event.startup.clone() {
            self.notify_err(format!("Master CSV check failed: {e}"), cx);
        }
        // 3.79: the Imports and Transfers pages are entities and apply
        // their own sections of the snapshot (see `ui/page.rs`), so this
        // mapping only covers the state still shared across pages.
        if sections & S_CATEGORIES != 0 {
            let (tree, counts) = {
                let cat = self.catalog.read(cx);
                (cat.snap.tree.clone(), cat.snap.counts.clone())
            };
            self.apply_categories(tree, counts);
        }
        if sections & S_RULES != 0 {
            // 3.80: the Rules page pulls its own rules and use-counts from
            // the snapshot; the shell keeps only the mirror the dashboard's
            // *Show rule* menu and the counts read.
            self.rules = self.catalog.read(cx).snap.rules.clone();
        }
        if sections & S_QUEUE != 0 {
            let queue = self.catalog.read(cx).snap.queue.clone();
            self.apply_queue(queue);
        }
        if sections & (S_CATEGORIES | S_RULES) != 0 {
            self.sync_category_choices(event.generation, window, cx);
        }
        if sections & S_QUEUE != 0 {
            self.sync_queue(window, cx);
        }
        if sections & S_SIDEBAR != 0 {
            let (total, uncategorized) = {
                let cat = self.catalog.read(cx);
                (cat.snap.total, cat.snap.uncategorized)
            };
            self.total_visible = total;
            self.uncategorized_visible = uncategorized;
        }
        // 3.75: the idle marker. Generations only move forward, and the
        // newest dispatch always emits, so the maximum is what `wait_idle`
        // waits for.
        self.refresh_applied = self.refresh_applied.max(event.generation);
        cx.notify();
    }

    fn go_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        // 3.11: no transient UI travels across a page switch — the two-click
        // confirmations, the rule offer and the inline rename editor all
        // belong to the page they were opened on. 3.24: the rule editor and
        // the armed rule delete join that list.
        self.dashboard.rule_offer = None;
        self.categories_page.renaming = false;
        self.categories_page.new_category_open = false;
        self.categories_page.confirm_delete_category = None;
        // 3.80: the rule editor is shared with the Categories page (it may
        // be open while the Rules page is hidden), so every switch runs
        // through the page: close it, disarm deletes, then show or hide.
        // `set_visible` clears the page-only transients, marks it dirty on
        // hide, and queues the search focus on show (D5).
        self.rules_page.update(cx, |p, cx| {
            p.close_rule_editor();
            p.armed_delete_rule = None;
            p.set_visible(page == Page::Rules, cx);
        });
        // 3.79: the extracted pages follow the switch too — the one shown
        // applies the snapshot if it drifted while hidden and takes focus,
        // the hidden one marks itself dirty. The armed import Undo clears
        // first (3.11) so the apply does not re-arm the history button.
        self.imports.update(cx, |p, cx| {
            p.confirm_undo_import = None;
            p.set_visible(page == Page::Imports, cx);
        });
        self.transfers
            .update(cx, |p, cx| p.set_visible(page == Page::Transfers, cx));
        cx.notify();
    }

    /// 3.79: the one page → `Shell` listener. Pages never reach the shell;
    /// everything they need from it arrives as a [`PageEvent`] on their
    /// own entity and is run here.
    fn on_page_event(&mut self, event: &PageEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PageEvent::Toast { message, error } => {
                if *error {
                    self.notify_err(message.clone(), cx);
                } else {
                    self.notify_ok(message.clone(), cx);
                }
            }
            PageEvent::Changed(domains) => self.changed(*domains, window, cx),
            PageEvent::Refresh => self.refresh(window, cx),
            PageEvent::ResetPages => self.reset_pages_after_restore(window, cx),
            PageEvent::Go(page) => self.go_page(*page, cx),
        }
    }

    /// Drop every page's transient state after a restore replaced the
    /// whole database behind the UI (5.3): row ids, open editors, armed
    /// confirmations and picker selections all name rows that the new
    /// file may not have, or — worse — ids reused for other rows. 3.79:
    /// the extracted pages clear their own share.
    pub(super) fn reset_pages_after_restore(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dashboard.filter.legend_selection = None;
        self.dashboard.filter.summary_month = None;
        self.dashboard.filter.chart_parent = None;
        self.dashboard.rule_offer = None;
        self.queue.pickers.clear();
        self.queue.picker_subs.clear();
        self.queue.picker_items = None;
        self.queue.error = None;
        self.categories_page.selected_category = None;
        self.categories_page.renaming = false;
        self.categories_page.new_category_open = false;
        self.categories_page.confirm_delete_category = None;
        // 3.80: the Rules page clears its own share — editor, armed
        // delete, add row, ticks, collapsed groups, move error, highlight.
        self.rules_page
            .update(cx, |p, cx| p.reset_after_restore(cx));
        // An armed Undo or a half-filled import panel would commit the
        // old database's ids and categories into the restored one.
        self.imports.update(cx, |p, cx| p.reset_after_restore(cx));
        self.refresh(window, cx);
    }

    // -- Forwarders (3.79): the file actions live on the Imports page and
    // transfer pairing on the Transfers page; the global action listeners
    // and the drag-and-drop target reach them through here.

    pub(super) fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.imports
            .update(cx, |p, cx| p.open_file(path, window, cx));
    }

    pub(super) fn pick_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.imports.update(cx, |p, cx| p.pick_file(window, cx));
    }

    pub(super) fn undo_last_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.imports
            .update(cx, |p, cx| p.undo_last_import(window, cx));
    }

    pub(super) fn export_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.imports.update(cx, |p, cx| p.export_data(window, cx));
    }

    pub(super) fn restore_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.imports
            .update(cx, |p, cx| p.restore_backup(window, cx));
    }

    pub(super) fn unpair_row(
        &mut self,
        transaction_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfers
            .update(cx, |p, cx| p.unpair_row(transaction_id, window, cx));
    }
}
