//! The Imports page: the import panel and its history table (3.62).

use super::*;
use crate::ui::widgets::*;

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

pub struct ImportHistoryTable {
    pub(super) rows: Vec<ImportEntry>,
    /// The import whose Undo button is waiting for its second click.
    pub(super) confirm_id: Option<i64>,
    /// Widths of the two flexible columns (`File`, `Account`), kept in
    /// step with the window by the shell (3.20); they truncate with an
    /// ellipsis rather than push the table past the page.
    pub(super) flex_widths: (f32, f32),
    pub(super) on_undo: UndoFn,
}

/// Callback from the history table's Undo button back to the [`Shell`].
type UndoFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

impl ImportHistoryTable {
    pub(super) fn time(entry: &ImportEntry) -> String {
        history_time(&entry.imported_at)
    }

    pub(super) fn dates(entry: &ImportEntry) -> String {
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

pub(super) enum PlanState {
    NeedAccount,
    Loading,
    Ready(PlanSummary),
    Failed(String),
}

pub struct ImportPanel {
    pub(super) db: DbWorker,
    csv_root: PathBuf,
    file: Arc<LoadedFile>,
    pub(super) format: String,
    pub(super) accounts: Vec<Account>,
    pub(super) names: Names,
    account_select: Entity<SelectState<Names>>,
    pub(super) new_name: Entity<InputState>,
    pub(super) creating_new: bool,
    account_id: Option<i64>,
    pub(super) plan: PlanState,
    pub(super) override_overlap: bool,
    /// Near-match review (4.3): pairs the user marked "Same charge", keyed
    /// by `(source_line, existing transaction id)`. Absent means "Different
    /// charge" — the default, so an unreviewed row imports as new.
    pub(super) same_charges: HashSet<(usize, i64)>,
    busy: bool,
    pub(super) error: Option<String>,
    /// 3.64: only the replaceable preview lives here — a newer plan
    /// supersedes the old one. Account load and the import commit run
    /// detached so no later action can cancel them mid-flight.
    plan_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

pub(super) const NEW_ACCOUNT: &str = "+ New account…";

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
        // file.parser_id comes from detection, so it always resolves; if it
        // ever didn't, the panel shows an error instead of panicking (3.90).
        let parser = flow::parser(file.parser_id);
        let error = parser
            .is_none()
            .then(|| format!("Unknown import format {:?}.", file.parser_id));
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
        // 3.64: the account list is the panel's own load — detached so a
        // later action can't cancel it.
        {
            let db = db.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = on_db(&db, |c| list_accounts(c)).await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(all) => this.set_accounts(all, None, window, cx),
                    Err(e) => this.error = Some(e),
                });
            })
            .detach();
        }
        Self {
            db,
            csv_root,
            format: parser
                .map(|p| flow::format_label(p.as_ref()))
                .unwrap_or_default(),
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
            error,
            plan_task: None,
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
        self.accounts = match flow::parser(self.file.parser_id) {
            Some(p) => flow::eligible_accounts(&all, p.as_ref()),
            None => Vec::new(),
        };
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

    pub(super) fn choose(&mut self, choice: SharedString, cx: &mut Context<Self>) {
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
        // 3.64: the preview is the one piece of panel work that *should*
        // be replaced when the account changes — it keeps the panel's own
        // slot. The commit is not replaceable: see `import`.
        self.plan_task = Some(cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| {
                let Some(parser) = flow::parser(file.parser_id) else {
                    return Err(format!("unknown import format {:?}", file.parser_id));
                };
                plan_import(c, account_id, parser.as_ref(), &file.raw, &file.file_name)
                    .map(|p| PlanSummary::from_plan(&p))
                    .map_err(|e| e.to_string())
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

    pub(super) fn create_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("Give the account a name.".into());
            cx.notify();
            return;
        }
        let Some(parser) = flow::parser(self.file.parser_id) else {
            self.error = Some(format!("Unknown import format {:?}.", self.file.parser_id));
            cx.notify();
            return;
        };
        let Some((institution, kind)) = flow::new_account_defaults(parser.as_ref()) else {
            self.error = Some(format!(
                "The {:?} format has no account type.",
                self.file.parser_id
            ));
            cx.notify();
            return;
        };
        let db = self.db.clone();
        self.busy = true;
        // 3.64: detached — the commit's result must always be applied.
        cx.spawn_in(window, async move |this, cx| {
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
        })
        .detach();
        cx.notify();
    }

    pub(super) fn import(&mut self, cx: &mut Context<Self>) {
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
        // 3.64: detached — the commit's result must always be applied,
        // whatever the user clicks while the DB is busy.
        cx.spawn(async move |this, cx| {
            let name = file.file_name.clone();
            let result = on_db(&db, move |c| {
                let Some(parser) = flow::parser(file.parser_id) else {
                    return Err(format!("unknown import format {:?}", file.parser_id));
                };
                let out = commit_import(
                    c,
                    account_id,
                    parser.as_ref(),
                    &file.raw,
                    &file.file_name,
                    &now_iso(),
                    options,
                )
                .map_err(|e| e.to_string())?;
                let csv_error = if out.import_id.is_some() {
                    let years: Vec<i32> = out.affected_years.iter().copied().collect();
                    financi_db::master_csv::rewrite_years(c, &csv_root, account_id, &years)
                        .err()
                        .map(|e| e.to_string())
                } else {
                    None
                };
                Ok::<_, String>((out, csv_error))
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
        })
        .detach();
        cx.notify();
    }

    pub(super) fn can_import(&self) -> bool {
        match &self.plan {
            PlanState::Ready(s) => !self.busy && (s.overlap.is_none() || self.override_overlap),
            _ => false,
        }
    }

    /// Choose "Same charge" / "Different charge" for one near-match pair
    /// (4.3). The default (absent) is Different: the row imports as new.
    pub(super) fn set_same_charge(
        &mut self,
        key: (usize, i64),
        same: bool,
        cx: &mut Context<Self>,
    ) {
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
        let busy = self.busy;
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
                            // 3.64: while a commit runs the panel is the
                            // record of it — Cancel is disabled until the
                            // result lands.
                            .disabled(busy)
                            .debug_selector(move || {
                                format!(
                                    "imports:cancel-state:{}",
                                    if busy { "disabled" } else { "enabled" }
                                )
                            })
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ImportEvent::Cancelled))),
                    )
                    .child(
                        Button::new("do-import")
                            .label(import_label)
                            .primary()
                            .disabled(!self.can_import())
                            // 3.64: loading while the commit runs.
                            .loading(busy)
                            .debug_selector(move || {
                                format!(
                                    "imports:import-state:{}",
                                    if busy { "loading" } else { "ready" }
                                )
                            })
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

// Imports history table widths (3.20). The old fixed columns summed to
// 1,110 px, wider than the page beside the sidebar at the minimum window
// size, which forced the table to scroll sideways. Now `Imported`,
// `Rows added` and `Dates` keep their widths, `Undo` is sized so the
// armed button — its icon and its widest label, "Click again to undo" —
// shows in full (3.32), and `File` and `Account` share whatever width is
// left — they truncate with an ellipsis as the window narrows and grow
// with it, so the table fits the page from the minimum window size up.
pub(super) const HISTORY_TIME_W: f32 = 150.;
pub(super) const HISTORY_ROWS_W: f32 = 110.;
pub(super) const HISTORY_DATES_W: f32 = 220.;
/// Floor for one flexible column: below this a File or Account cell is
/// unreadable anyway. The app's minimum window never reaches it.
pub(super) const HISTORY_FLEX_MIN_W: f32 = 80.;
/// The Undo button's widest label; the column must fit this without
/// clipping whatever the window size.
pub(super) const UNDO_LABEL_MAX: &str = "Click again to undo";
/// The Undo column's width: the longest label at `TEXT_SM_CHAR_PX`,
/// plus the button's icon and horizontal padding and the cell's.
pub(super) fn history_undo_col_w() -> f32 {
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
pub(super) fn history_flex_widths(page_w: f32) -> (f32, f32) {
    let fixed =
        HISTORY_TIME_W + HISTORY_ROWS_W + HISTORY_DATES_W + history_undo_col_w() + TABLE_BORDER_X;
    let flex = (page_w - fixed).max(2. * HISTORY_FLEX_MIN_W);
    (flex / 2., flex / 2.)
}

/// This page's state (3.74): the import panel and the history table.
pub(super) struct ImportsState {
    /// The import panel, built once by [`Shell::import`] and kept so its
    /// picked file survives page switches. 3.64: the panel's subscription
    /// lives beside it — replacing the panel drops the subscription with
    /// it, so opening many files can't accumulate subscriptions.
    pub(super) import: Option<(Entity<ImportPanel>, Subscription)>,
    /// The history table's rows, newest first.
    pub(super) import_history: Vec<ImportEntry>,
    pub(super) history_table: Entity<TableState<ImportHistoryTable>>,
    /// The import whose Undo… button is armed ("Click again to undo").
    pub(super) confirm_undo_import: Option<i64>,
}

impl ImportsState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
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
        Self {
            import: None,
            import_history: Vec::new(),
            history_table,
            confirm_undo_import: None,
        }
    }
}

impl Shell {
    /// Apply this page's part of a refresh result (3.74).
    pub(super) fn apply_imports(&mut self, history: Vec<ImportEntry>, cx: &mut Context<Shell>) {
        if !history
            .iter()
            .any(|e| Some(e.id) == self.imports.confirm_undo_import)
        {
            self.imports.confirm_undo_import = None;
        }
        self.imports.import_history = history;
        let confirm = self.imports.confirm_undo_import;
        let history = self.imports.import_history.clone();
        self.imports.history_table.update(cx, |t, cx| {
            let delegate = t.delegate_mut();
            delegate.rows = history;
            delegate.confirm_id = confirm;
            t.refresh(cx);
        });
    }
}

impl Shell {
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // 3.64: while a commit is in flight the panel is still the record
        // of it — opening another file would replace it and strand the
        // result. The request is ignored (the Import button is likewise
        // disabled; the drop and ⌘O paths share this guard).
        if self
            .imports
            .import
            .as_ref()
            .is_some_and(|(panel, _)| panel.read(cx).busy)
        {
            return;
        }
        match flow::load_file(&path) {
            Ok(file) => {
                let db = self.db.clone();
                let csv_root = self.csv_root.clone();
                let panel = cx.new(|cx| ImportPanel::new(db, csv_root, file, window, cx));
                // 3.64: the subscription is stored beside the panel, so
                // replacing the panel drops it — ten opens leave exactly
                // one subscription in `self._subs`.
                let sub =
                    cx.subscribe_in(&panel, window, |this, _, ev: &ImportEvent, window, cx| {
                        match ev {
                            ImportEvent::Imported { message, is_error } => {
                                if *is_error {
                                    this.notify_err(message.clone(), cx);
                                } else {
                                    this.notify_ok(message.clone(), cx);
                                }
                                this.imports.import = None;
                            }
                            ImportEvent::Cancelled => this.imports.import = None,
                            ImportEvent::AccountsChanged => {}
                        }
                        this.refresh(window, cx);
                    });
                // 3.64: the subscription lives in the tuple; keeping the
                // tuple's copy in `_subs` as well would double-unsubscribe
                // on drop, so `_subs` only ever holds the panel *while it
                // is open* — replaced panels drop theirs.
                let stored = self.imports.import.replace((panel, sub));
                if let Some((_, old_sub)) = stored {
                    // Unsubscribe the previous panel exactly once (the
                    // tuple's clone below stays alive until replaced).
                    old_sub.detach();
                }
                // Opening a file switches to the Imports page with the
                // preview open (ui-conventions, *Pages*).
                self.page = Page::Imports;
            }
            Err(e) => self.notify_err(e.to_string(), cx),
        }
        cx.notify();
    }

    pub(super) fn pick_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        });
        // 3.64: detached — the prompt's result must arrive whatever the
        // user did meanwhile (`open_file` itself refuses while busy).
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| this.open_file(path, window, cx));
            }
        })
        .detach();
    }

    /// Undo the most recent successful import (the File menu item and the
    /// Imports header button): the same path as the newest history row's
    /// Undo, two-click confirmation included; a no-op with no history.
    pub(super) fn undo_last_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.imports.import_history.first() {
            let id = entry.id;
            self.undo_import(id, window, cx);
        }
    }

    /// Export… (⌘E): wired in M5. Until then the menu item, the ⌘E
    /// shortcut and the Imports header button all say so plainly.
    pub(super) fn export_data(&mut self, cx: &mut Context<Self>) {
        self.notify_ok("Export is not available yet.", cx);
    }

    /// Restore…: wired in M5, like [`Shell::export_data`].
    pub(super) fn restore_backup(&mut self, cx: &mut Context<Self>) {
        self.notify_ok("Restore is not available yet.", cx);
    }

    /// Undo an import from the history: the first click arms the button,
    /// the second runs the undo (ui-conventions, *Interaction*).
    pub(super) fn undo_import(
        &mut self,
        import_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.imports.confirm_undo_import != Some(import_id) {
            self.imports.confirm_undo_import = Some(import_id);
            self.imports.history_table.update(cx, |t, _| {
                t.delegate_mut().confirm_id = Some(import_id);
            });
            cx.notify();
            return;
        }
        self.imports.confirm_undo_import = None;
        let Some(account_id) = self
            .imports
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
        // 3.64: detached — the undo's result must always be applied.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                // 3.70: name the import the user clicked; if the history
                // went stale the DB refuses instead of undoing another file.
                let undone = undo_last_import(c, account_id, import_id)?;
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
        })
        .detach();
        cx.notify();
    }

    // -- categories --------------------------------------------------------

    pub(super) fn render_imports(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        // The page exposes every file action (3.7): the native menu may be
        // absent (Linux), so Import, Undo, Export and Restore live here.
        // 3.32: each button carries its Lucide icon left of the label, and
        // no ellipsis — the action runs straight away, any confirmation is
        // the button's own armed state, not a hidden dialog.
        let latest_undo_id = self.imports.import_history.first().map(|e| e.id);
        let undo_label = if self.imports.confirm_undo_import.is_some()
            && self.imports.confirm_undo_import == latest_undo_id
        {
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
                            .child(format!("{} imports", self.imports.import_history.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("pick-file")
                            .label("Import CSV")
                            .icon(IconName::FileUp)
                            .primary()
                            .debug_selector(|| {
                                format!("imports:header:{}:{}", "Import CSV", "icons/file-up.svg")
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
                                format!("imports:header:{}:{}", undo_label, "icons/undo-2.svg")
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
                                format!("imports:header:{}:{}", "Export", "icons/download.svg")
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.export_data(cx))),
                    )
                    .child(
                        Button::new("restore-backup")
                            .label("Restore")
                            .icon(IconName::ArchiveRestore)
                            .debug_selector(|| {
                                format!(
                                    "imports:header:{}:{}",
                                    "Restore", "icons/archive-restore.svg"
                                )
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.restore_backup(cx))),
                    ),
            )
            .children(self.imports.import.as_ref().map(|(p, _)| p.clone()))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.imports.history_table).stripe(true)),
            )
            .into_any_element()
    }
}
