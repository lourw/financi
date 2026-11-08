//! GPUI views. Thin by design: logic lives in `flow` and `financi-db`, and
//! every database call goes through the [`DbWorker`] thread.
//!
//! The window is a [`Sidebar`] of pages and one page area, per
//! `docs/ui-conventions.md` (*Window layout*).

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use financi_db::import::{
    CommitOptions, ImportEntry, TransactionRow, commit_import, list_imports, list_transactions,
    plan_import, undo_last_import,
};
use financi_db::{
    Account, DbWorker, create_account, list_accounts, rusqlite::Connection, sidebar_counts,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::sidebar::{Sidebar, SidebarItem, SidebarMenuItem};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Collapsible, Disableable as _, h_flex, v_flex};
use gpui_kit::*;

use crate::flow::{self, LoadedFile, PlanSummary};

type Names = Vec<SharedString>;

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
            Self::Item(item) => Self::Item(Box::new(item.collapsed(collapsed))),
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
                .px_2()
                .text_xs()
                .text_color(cx.theme().sidebar_foreground.opacity(0.7))
                .child(label.to_uppercase())
                .into_any_element(),
        }
    }
}

// ---------------------------------------------------------------------------
// Transaction table

pub struct TxTable {
    rows: Vec<TransactionRow>,
}

impl TableDelegate for TxTable {
    fn columns_count(&self, _: &App) -> usize {
        5
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            0 => Column::new("date", "Date").width(px(110.)),
            1 => Column::new("account", "Account").width(px(300.)),
            2 => Column::new("description", "Description").width(px(360.)),
            3 => Column::new("category", "Category").width(px(140.)),
            _ => Column::new("amount", "Amount").width(px(120.)).text_right(),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div().child(self.text(row_ix, col_ix))
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
            0 => Column::new("time", "Imported").width(px(150.)),
            1 => Column::new("file", "File").width(px(240.)),
            2 => Column::new("account", "Account").width(px(240.)),
            3 => Column::new("rows", "Rows added")
                .width(px(110.))
                .text_right(),
            4 => Column::new("dates", "Dates").width(px(220.)),
            _ => Column::new("undo", "").width(px(150.)),
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
                        let mut message = format!(
                            "Imported {name}: {} new, {} already imported",
                            out.inserted, out.already_present
                        );
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
                        Checkbox::new("override-overlap")
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

        let new_account = self.creating_new.then(|| {
            h_flex()
                .gap_2()
                .child(div().w(px(280.)).child(Input::new(&self.new_name)))
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
            PlanState::Ready(s) => format!("Import {} transactions", s.new_rows),
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

pub struct Shell {
    db: DbWorker,
    db_path: PathBuf,
    csv_root: PathBuf,
    accounts: Vec<Account>,
    page: Page,
    filter: Entity<SelectState<Names>>,
    selected_account: Option<i64>,
    table: Entity<TableState<TxTable>>,
    row_count: usize,
    total_visible: i64,
    uncategorized_visible: i64,
    import: Option<Entity<ImportPanel>>,
    import_history: Vec<ImportEntry>,
    history_table: Entity<TableState<ImportHistoryTable>>,
    confirm_undo_import: Option<i64>,
    status: Option<(String, bool)>,
    refresh_task: Option<Task<()>>,
    task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

const ALL_ACCOUNTS: &str = "All accounts";

impl Shell {
    pub fn new(
        db_path: PathBuf,
        db: DbWorker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| TableState::new(TxTable { rows: Vec::new() }, window, cx));
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
                    on_undo,
                },
                window,
                cx,
            )
        });
        let filter = cx.new(|cx| {
            SelectState::new(
                vec![SharedString::from(ALL_ACCOUNTS)],
                Some(gpui_kit::component::IndexPath::new(0)),
                window,
                cx,
            )
        });
        let sub = cx.subscribe_in(
            &filter,
            window,
            |this, _, ev: &SelectEvent<Names>, window, cx| {
                let SelectEvent::Confirm(choice) = ev;
                this.selected_account = choice.as_ref().and_then(|c| {
                    this.accounts
                        .iter()
                        .find(|a| a.display_name() == c.as_ref())
                        .map(|a| a.id)
                });
                this.confirm_undo_import = None;
                this.refresh(window, cx);
            },
        );
        let csv_root = financi_db::master_csv::root_for_db(&db_path);
        let mut shell = Self {
            db,
            db_path,
            csv_root,
            accounts: Vec::new(),
            page: Page::Dashboard,
            filter,
            selected_account: None,
            table,
            row_count: 0,
            total_visible: 0,
            uncategorized_visible: 0,
            import: None,
            import_history: Vec::new(),
            history_table,
            confirm_undo_import: None,
            status: None,
            refresh_task: None,
            task: None,
            _subs: vec![sub],
        };
        shell.refresh(window, cx);
        shell
    }

    /// Re-query everything the pages show: accounts and transactions for
    /// the dashboard, sidebar counts, and the import history.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (db, account) = (self.db.clone(), self.selected_account);
        self.refresh_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                let accounts = list_accounts(c)?;
                let rows = list_transactions(c, account)?;
                let (total, uncategorized) = sidebar_counts(c)?;
                let history = list_imports(c)?;
                Ok::<_, financi_db::DbError>((accounts, rows, total, uncategorized, history))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((accounts, rows, total, uncategorized, history)) => {
                        let names: Names = std::iter::once(SharedString::from(ALL_ACCOUNTS))
                            .chain(accounts.iter().map(|a| a.display_name().into()))
                            .collect();
                        let selected = this
                            .selected_account
                            .and_then(|id| accounts.iter().position(|a| a.id == id))
                            .map_or(0, |ix| ix + 1);
                        this.filter.update(cx, |s, cx| {
                            s.set_items(names, window, cx);
                            s.set_selected_index(
                                Some(gpui_kit::component::IndexPath::new(selected)),
                                window,
                                cx,
                            );
                        });
                        if !history
                            .iter()
                            .any(|e| Some(e.id) == this.confirm_undo_import)
                        {
                            this.confirm_undo_import = None;
                        }
                        this.accounts = accounts;
                        this.row_count = rows.len();
                        this.total_visible = total;
                        this.uncategorized_visible = uncategorized;
                        this.import_history = history;
                        this.table.update(cx, |t, cx| {
                            t.delegate_mut().rows = rows;
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

    fn go_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.confirm_undo_import = None;
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
        let transfers = self.nav_item("Transfers", IconName::ArrowLeftRight, Page::Transfers, cx);
        let imports = self.nav_item("Imports", IconName::Download, Page::Imports, cx);

        Sidebar::new("nav")
            .w(px(210.))
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

    fn render_dashboard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let empty = self.row_count == 0;
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(320.)).child(Select::new(&self.filter)))
                    .child(div().text_sm().text_color(muted).child(format!(
                        "{} transactions",
                        flow::count(self.row_count as i64)
                    ))),
            )
            .child(if empty {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop an Amex or Simplii CSV export here, or press ⌘O")
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true))
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_imports(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
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

    fn render_placeholder(
        &mut self,
        title: &'static str,
        body: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().text_sm().text_color(muted).child(body))
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
            Page::ToCategorize => self.render_placeholder("To categorize", "Nothing waiting.", cx),
            Page::Categories => self.render_placeholder("Categories", "Nothing here yet.", cx),
            Page::Transfers => self.render_placeholder("Transfers", "No transfer reviews yet.", cx),
            Page::Imports => self.render_imports(cx),
        };

        h_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
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
