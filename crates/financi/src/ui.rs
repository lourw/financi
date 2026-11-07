//! GPUI views. Thin by design: logic lives in `flow` and `financi-db`, and
//! every database call goes through the [`DbWorker`] thread.

use std::path::PathBuf;
use std::sync::Arc;

use financi_db::import::{
    CommitOptions, TransactionRow, commit_import, list_transactions, plan_import, undo_last_import,
};
use financi_db::{Account, DbWorker, create_account, list_accounts, rusqlite::Connection};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
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
// Import preview panel

pub enum ImportEvent {
    Imported(String),
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
        let options = CommitOptions {
            allow_other_account_overlap: self.override_overlap,
        };
        self.busy = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            let name = file.file_name.clone();
            let result = on_db(&db, move |c| {
                let parser = flow::parser(file.parser_id);
                commit_import(
                    c,
                    account_id,
                    parser.as_ref(),
                    &file.raw,
                    &file.file_name,
                    &now_iso(),
                    options,
                )
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(out) => cx.emit(ImportEvent::Imported(format!(
                        "Imported {name}: {} new, {} already imported",
                        out.inserted, out.already_present
                    ))),
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
        let (danger, muted) = (theme.danger, theme.muted_foreground);

        let plan: AnyElement = match &self.plan {
            PlanState::NeedAccount => div()
                .text_color(muted)
                .child("Choose the account this file belongs to.")
                .into_any_element(),
            PlanState::Loading => div().child("Checking for duplicates…").into_any_element(),
            PlanState::Failed(e) => div().text_color(danger).child(e.clone()).into_any_element(),
            PlanState::Ready(s) => {
                let mut col = v_flex().gap_2().child(s.headline());
                if let Some((other, rows)) = &s.overlap {
                    col = col
                        .child(div().text_color(danger).child(format!(
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
                        );
                }
                col.into_any_element()
            }
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

        v_flex()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(
                div()
                    .text_lg()
                    .child(format!("Import {}", self.file.file_name)),
            )
            .child(
                div()
                    .text_color(muted)
                    .child(format!("Detected format: {}", self.format)),
            )
            .child(
                div()
                    .w(px(360.))
                    .child(Select::new(&self.account_select).placeholder("Choose an account")),
            )
            .children(new_account)
            .child(plan)
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_color(danger).child(e)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("do-import")
                            .label("Import")
                            .primary()
                            .disabled(!self.can_import())
                            .on_click(cx.listener(|this, _, _, cx| this.import(cx))),
                    )
                    .child(
                        Button::new("cancel-import")
                            .label("Cancel")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ImportEvent::Cancelled))),
                    ),
            )
    }
}

// ---------------------------------------------------------------------------
// Main window

pub struct Shell {
    db: DbWorker,
    db_path: PathBuf,
    accounts: Vec<Account>,
    filter: Entity<SelectState<Names>>,
    selected_account: Option<i64>,
    table: Entity<TableState<TxTable>>,
    row_count: usize,
    import: Option<Entity<ImportPanel>>,
    status: Option<(String, bool)>,
    confirm_undo: bool,
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
                this.confirm_undo = false;
                this.refresh(window, cx);
            },
        );
        let mut shell = Self {
            db,
            db_path,
            accounts: Vec::new(),
            filter,
            selected_account: None,
            table,
            row_count: 0,
            import: None,
            status: None,
            confirm_undo: false,
            refresh_task: None,
            task: None,
            _subs: vec![sub],
        };
        shell.refresh(window, cx);
        shell
    }

    /// Re-query accounts and transactions, then update the filter and table.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (db, account) = (self.db.clone(), self.selected_account);
        self.refresh_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                let accounts = list_accounts(c)?;
                let rows = list_transactions(c, account)?;
                Ok::<_, financi_db::DbError>((accounts, rows))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((accounts, rows)) => {
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
                        this.accounts = accounts;
                        this.row_count = rows.len();
                        this.table.update(cx, |t, cx| {
                            t.delegate_mut().rows = rows;
                            t.refresh(cx);
                        });
                    }
                    Err(e) => this.status = Some((format!("Database error: {e}"), true)),
                }
                cx.notify();
            });
        }));
    }

    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match flow::load_file(&path) {
            Ok(file) => {
                let db = self.db.clone();
                let panel = cx.new(|cx| ImportPanel::new(db, file, window, cx));
                let sub =
                    cx.subscribe_in(&panel, window, |this, _, ev: &ImportEvent, window, cx| {
                        match ev {
                            ImportEvent::Imported(msg) => {
                                this.status = Some((msg.clone(), false));
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

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account_id) = self.selected_account else {
            return;
        };
        if !self.confirm_undo {
            self.confirm_undo = true;
            cx.notify();
            return;
        }
        self.confirm_undo = false;
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| undo_last_import(c, account_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.status = Some(match result {
                    Ok(Some(u)) => (
                        format!(
                            "Undid import of {}: removed {} rows",
                            u.file_name, u.removed
                        ),
                        false,
                    ),
                    Ok(None) => ("This account has no imports to undo.".into(), false),
                    Err(e) => (e, true),
                });
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (danger, muted) = (theme.danger, theme.muted_foreground);
        let undo_label = if self.confirm_undo {
            "Click again to undo"
        } else {
            "Undo last import"
        };

        let empty = self.row_count == 0 && self.import.is_none();

        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(path) = paths.paths().first() {
                    this.open_file(path.clone(), window, cx);
                }
            }))
            .child(
                h_flex()
                    .gap_3()
                    .child(div().text_xl().child("financi"))
                    .child(div().w(px(320.)).child(Select::new(&self.filter)))
                    .child(
                        Button::new("pick-file")
                            .label("Import CSV…")
                            .primary()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_file(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("undo-import")
                            .label(undo_label)
                            .disabled(self.selected_account.is_none())
                            .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))),
                    ),
            )
            .children(self.status.clone().map(|(msg, is_error)| {
                div()
                    .text_color(if is_error { danger } else { muted })
                    .child(msg)
            }))
            .children(self.import.clone())
            .child(if empty {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop an Amex or Simplii CSV export here, or use Import CSV…")
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .child(DataTable::new(&self.table).stripe(true))
                    .into_any_element()
            })
            .child(div().text_xs().text_color(muted).child(format!(
                "{} transactions · {}",
                self.row_count,
                self.db_path.display()
            )))
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
