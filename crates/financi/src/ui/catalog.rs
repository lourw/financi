//! 3.78: the shared data model. `Catalog` owns the read-model data every
//! page renders from, reloads from the database only the parts a reported
//! change can touch (keyed latest-wins jobs, 3.77), and announces the
//! result with [`DataChanged`] so `Shell` repaints exactly the pages that
//! depend on it. Writers report [`Domain`]s, readers declare them — see
//! the table on [`Domain`] and [`Catalog::sections_for`].

use super::*;
use crate::ui::widgets::on_db;
use financi_db::{DbError, QueryLog};

/// One slice of the shared data.
///
/// This is *the* writer table (3.78): every mutation's UI handler reports
/// what it changed with these, and page sections declare what they depend
/// on ( [`Catalog::sections_for`]). Nothing picks a reload per click.
///
/// | write | reports |
/// |---|---|
/// | rule add / edit / delete / move | `Rules \| Transactions` |
/// | set category, hide, unhide | `Transactions` |
/// | category add / rename / move / delete | `Categories \| Rules \| Transactions` |
/// | pair / unpair / transfer review | `Transfers \| Transactions` |
/// | import, undo import, restore | [`Domains::ALL`] (the full reload) |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Domain {
    Accounts,
    Categories,
    Rules,
    Transactions,
    Imports,
    Transfers,
    /// The dashboard's own view settings (date range, accounts checked,
    /// search, legend, month, drill-down, sort): they change only the
    /// dashboard's rows, chart and summary.
    Settings,
}

impl Domain {
    const fn bit(self) -> u8 {
        match self {
            Domain::Accounts => 1 << 0,
            Domain::Categories => 1 << 1,
            Domain::Rules => 1 << 2,
            Domain::Transactions => 1 << 3,
            Domain::Imports => 1 << 4,
            Domain::Transfers => 1 << 5,
            Domain::Settings => 1 << 6,
        }
    }
}

/// The set of [`Domain`]s a write touched: a bitset over the seven.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Domains(u8);

impl Domains {
    pub(super) const NONE: Self = Self(0);
    pub(super) const ALL: Self = Self(0b0111_1111);
    // The writer table's shorthands, spelled once.
    pub(super) const TX: Self = Self(Domain::Transactions.bit());
    pub(super) const RULES_TX: Self = Self(Domain::Rules.bit() | Domain::Transactions.bit());
    pub(super) const CATEGORIES_RULES_TX: Self =
        Self(Domain::Categories.bit() | Domain::Rules.bit() | Domain::Transactions.bit());
    pub(super) const TRANSFERS_TX: Self =
        Self(Domain::Transfers.bit() | Domain::Transactions.bit());
    pub(super) const SETTINGS: Self = Self(Domain::Settings.bit());

    pub(super) fn contains(self, domain: Domain) -> bool {
        self.0 & domain.bit() != 0
    }
}

impl std::ops::BitOr for Domain {
    type Output = Domains;
    fn bitor(self, rhs: Domain) -> Domains {
        Domains(self.bit() | rhs.bit())
    }
}

impl std::ops::BitOr for Domains {
    type Output = Domains;
    fn bitor(self, rhs: Domains) -> Domains {
        Domains(self.0 | rhs.0)
    }
}

// The reload is built from sections — one query or one tight group of
// them — so a change reloads exactly the data it can have touched.
// `Shell`'s subscriber uses these to decide which pages to apply.
pub(super) const S_ACCOUNTS: u16 = 1 << 0; // list_accounts
pub(super) const S_CATEGORIES: u16 = 1 << 1; // list_categories + transaction_counts
pub(super) const S_RULES: u16 = 1 << 2; // list_rules + rule_use_stats
pub(super) const S_SIDEBAR: u16 = 1 << 3; // sidebar_counts
pub(super) const S_QUEUE: u16 = 1 << 4; // uncategorized_by_merchant
pub(super) const S_TRANSFERS: u16 = 1 << 5; // list_transfer_reviews
pub(super) const S_IMPORTS: u16 = 1 << 6; // list_imports
pub(super) const S_DASHBOARD: u16 = 1 << 7; // rows + chart + monthly summary
const S_ALL: u16 = 0xFF;

/// The reader dependency map (3.78): which sections a change to `domains`
/// can invalidate. Over-fetching is fine where nothing forbids it (a
/// change to one domain may load a little more than strictly needed);
/// under-fetching is not. The dashboard reads from everything, so any
/// domain that can move a transaction row reloads it — but the Imports
/// history answers to Imports alone and the transfer reviews to Transfers
/// alone, which is what keeps a rule edit off both.
fn sections_for(domains: Domains) -> u16 {
    let mut s: u16 = 0;
    if domains.contains(Domain::Accounts) {
        // A new account joins the filter checkboxes; the sidebar counts
        // and the dashboard's account resolution follow.
        s |= S_ACCOUNTS | S_SIDEBAR | S_DASHBOARD;
    }
    if domains.contains(Domain::Categories) {
        // The tree, its per-category counts, and everything the dashboard
        // groups by category.
        s |= S_CATEGORIES | S_DASHBOARD;
    }
    if domains.contains(Domain::Rules) {
        s |= S_RULES | S_DASHBOARD;
    }
    if domains.contains(Domain::Transactions) {
        // Counts, rule uses, sidebar totals and the queue all read the
        // transaction rows.
        s |= S_CATEGORIES | S_RULES | S_SIDEBAR | S_QUEUE | S_DASHBOARD;
    }
    if domains.contains(Domain::Imports) {
        s |= S_ALL;
    }
    if domains.contains(Domain::Transfers) {
        // The open reviews feed the Transfers page and its sidebar pill
        // (which reads the review list, not the counts).
        s |= S_TRANSFERS;
    }
    if domains.contains(Domain::Settings) {
        s |= S_DASHBOARD;
    }
    s
}

/// The shared read model as of the last reload: everything the pages
/// render from. `Shell` mirrors its fields into its own — the one mapping
/// it keeps until the pages become entities of their own (3.79–3.83).
#[derive(Debug, Default)]
pub(super) struct Snapshot {
    pub accounts: Vec<Account>,
    pub tree: Vec<CategoryNode>,
    pub counts: HashMap<i64, i64>,
    pub rules: Vec<Rule>,
    pub uses: HashMap<i64, usize>,
    pub total: i64,
    pub uncategorized: i64,
    pub queue: Vec<UncategorizedGroup>,
    pub reviews: Vec<TransferReview>,
    pub imports: Vec<ImportEntry>,
    pub rows: Vec<TransactionRow>,
    pub chart: Vec<CategorySpend>,
    pub summary: Option<MonthlySummary>,
}

/// The query inputs for one reload, snapshotted by `Shell` at dispatch:
/// the account checkboxes and the dashboard's filter values, so the job
/// runs with exactly the filters the screen shows.
#[derive(Clone, Debug, Default)]
pub(super) struct LoadFilters {
    pub excluded: Vec<i64>,
    pub view_filter: ViewFilter,
    pub table_filter: TxTableFilter,
    pub chart_parent: Option<i64>,
}

/// Where the dashboard queries' account ids come from (task 3.17: the
/// filter must resolve against accounts as fresh as the rows it queries).
enum DashAccounts {
    /// Resolved against the accounts the catalog already had — this
    /// change cannot have added an account.
    Resolved { ids: Vec<i64>, none_selected: bool },
    /// This load also loads the accounts; resolve against those.
    Fresh,
}

/// What a reload finished with. `Shell` subscribes once and applies the
/// matching page sections (the one mapping).
#[derive(Debug)]
pub(super) struct DataChanged {
    /// What changed; empty when nothing on screen reads it (yet).
    pub domains: Domains,
    /// The dispatch generation (3.75's idle markers): every settled
    /// dispatch carries its own back.
    pub generation: u64,
    /// The reload failed; pages keep the previous data.
    pub error: Option<String>,
    /// The launch settings (3.65) the full job loaded and already ran
    /// its queries with: the date range, the transfers toggle, and a
    /// master-CSV self-check failure to toast.
    pub startup: Option<(DateRange, bool, Option<String>)>,
}

impl EventEmitter<DataChanged> for Catalog {}

/// The owner of the shared data. One [`changed`](Self::changed) entry
/// point for writes (partial reloads, keyed so a burst coalesces); the
/// full reload stays a single writer-thread job, used at launch and for
/// import / undo import / restore.
pub(super) struct Catalog {
    db: DbWorker,
    /// The data as of the last reload. `Shell` mirrors it into the state
    /// its pages render from; nothing else reads it.
    pub(super) snap: Snapshot,
    /// The full reload's waiter. A newer full reload replaces it; the
    /// writer thread runs the jobs in order.
    full_task: Option<Task<()>>,
}

impl Catalog {
    pub(super) fn new(db: DbWorker) -> Self {
        Self {
            db,
            snap: Snapshot::default(),
            full_task: None,
        }
    }

    /// The reader dependency map: the sections `domains` reloads.
    pub(super) fn sections_for(domains: Domains) -> u16 {
        sections_for(domains)
    }

    /// A write reported `domains`: reload only the sections they touch,
    /// then emit [`DataChanged`]. `filters` is the screen's filter state,
    /// snapshotted by `Shell` at dispatch. The job is keyed by its section
    /// set, so a burst of the same kind of change runs once (3.77) while
    /// different section sets still run side by side.
    pub(super) fn changed(
        &mut self,
        domains: Domains,
        generation: u64,
        filters: LoadFilters,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sections = sections_for(domains);
        if sections == S_ALL {
            self.reload_full(domains, generation, filters, None, window, cx);
            return;
        }
        if sections == 0 {
            // Nothing on screen reads this domain (yet); settle so the
            // idle markers still line up.
            cx.emit(DataChanged {
                domains: Domains::NONE,
                generation,
                error: None,
                startup: None,
            });
            return;
        }
        let (ids, none_selected) = self.resolved_account_ids(&filters.excluded);
        let db = self.db.clone();
        let log = db.query_log();
        let key = format!("catalog:{sections}");
        cx.spawn_in(window, async move |this, cx| {
            let result = db
                .call_keyed(&key, move |c| {
                    load_sections(
                        c,
                        &log,
                        sections,
                        filters,
                        DashAccounts::Resolved { ids, none_selected },
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, _window, cx| match result {
                Ok(snapshot) => {
                    this.merge(sections, snapshot);
                    cx.emit(DataChanged {
                        domains,
                        generation,
                        error: None,
                        startup: None,
                    });
                }
                // A newer change to the same sections took over; its
                // event carries a newer generation and the fresher data.
                Err(DbError::Superseded) => {}
                Err(e) => cx.emit(DataChanged {
                    domains,
                    generation,
                    error: Some(e.to_string()),
                    startup: None,
                }),
            });
        })
        .detach();
    }

    /// Reload everything in one writer-thread job: the launch load (with
    /// the saved settings folded in, 3.65) and the import / undo / restore
    /// flows, where every part of the read model really is suspect. One
    /// job, so `job_count` stays at one for the launch invariant.
    /// `startup` is `Some(master CSV root)` only for the launch load.
    pub(super) fn reload_full(
        &mut self,
        domains: Domains,
        generation: u64,
        mut filters: LoadFilters,
        startup: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let db = self.db.clone();
        let log = db.query_log();
        self.full_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                // 3.65: launch settings first — the queries below run
                // with them, so the first frame is already the saved
                // view. 3.76: the master-CSV self-check rides this same
                // job when it is the launch one.
                let startup = if let Some(csv_root) = startup {
                    log.record("settings::date_range");
                    let range = settings::date_range(c)?;
                    log.record("settings::include_transfers");
                    let transfers = settings::include_transfers(c)?;
                    log.record("master_csv::self_check");
                    let csv_check = financi_db::master_csv::self_check(c, &csv_root)
                        .err()
                        .map(|e| e.to_string());
                    let (from, to) = range.range(Shell::today());
                    filters.view_filter.from = from;
                    filters.view_filter.to = to;
                    filters.view_filter.include_transfers = transfers;
                    Some((range, transfers, csv_check))
                } else {
                    None
                };
                let snap = load_sections(c, &log, S_ALL, filters, DashAccounts::Fresh)?;
                Ok::<_, DbError>((snap, startup))
            })
            .await;
            let _ = this.update_in(cx, |this, _window, cx| match result {
                Ok((snap, startup)) => {
                    this.snap = snap;
                    cx.emit(DataChanged {
                        domains,
                        generation,
                        error: None,
                        startup,
                    });
                }
                Err(e) => cx.emit(DataChanged {
                    domains,
                    generation,
                    error: Some(e),
                    startup: None,
                }),
            });
        }));
    }

    /// The account ids the dashboard queries use: every account the
    /// checkboxes have not excluded. `none_selected` is the "nothing
    /// checked" case — with accounts present but none selected the
    /// queries are skipped entirely.
    fn resolved_account_ids(&self, excluded: &[i64]) -> (Vec<i64>, bool) {
        let ids: Vec<i64> = self
            .snap
            .accounts
            .iter()
            .filter(|a| !excluded.contains(&a.id))
            .map(|a| a.id)
            .collect();
        let none_selected = ids.is_empty() && !self.snap.accounts.is_empty();
        (ids, none_selected)
    }

    fn merge(&mut self, sections: u16, s: Snapshot) {
        if sections & S_ACCOUNTS != 0 {
            self.snap.accounts = s.accounts;
        }
        if sections & S_CATEGORIES != 0 {
            self.snap.tree = s.tree;
            self.snap.counts = s.counts;
        }
        if sections & S_RULES != 0 {
            self.snap.rules = s.rules;
            self.snap.uses = s.uses;
        }
        if sections & S_SIDEBAR != 0 {
            self.snap.total = s.total;
            self.snap.uncategorized = s.uncategorized;
        }
        if sections & S_QUEUE != 0 {
            self.snap.queue = s.queue;
        }
        if sections & S_TRANSFERS != 0 {
            self.snap.reviews = s.reviews;
        }
        if sections & S_IMPORTS != 0 {
            self.snap.imports = s.imports;
        }
        if sections & S_DASHBOARD != 0 {
            self.snap.rows = s.rows;
            self.snap.chart = s.chart;
            self.snap.summary = s.summary;
        }
    }
}

/// Run the sections' queries on the worker thread, recording each name so
/// a test can see exactly what ran (the query-count hook, 3.78).
fn load_sections(
    c: &mut Connection,
    log: &QueryLog,
    sections: u16,
    f: LoadFilters,
    dash: DashAccounts,
) -> Result<Snapshot, DbError> {
    let mut snap = Snapshot::default();
    if sections & S_ACCOUNTS != 0 {
        log.record("list_accounts");
        snap.accounts = list_accounts(c)?;
    }
    if sections & S_CATEGORIES != 0 {
        log.record("list_categories");
        snap.tree = list_categories(c)?;
        log.record("transaction_counts");
        snap.counts = transaction_counts(c)?;
    }
    if sections & S_RULES != 0 {
        log.record("list_rules");
        snap.rules = list_rules(c)?;
        // 3.28: the Rules page Uses column. 3.87 adds per-rule totals;
        // the page still shows counts only, so keep just those.
        log.record("rule_use_stats");
        snap.uses = rule_use_stats(c)?
            .into_iter()
            .map(|(id, stats)| (id, stats.count))
            .collect();
    }
    if sections & S_SIDEBAR != 0 {
        log.record("sidebar_counts");
        let (total, uncategorized) = sidebar_counts(c)?;
        snap.total = total;
        snap.uncategorized = uncategorized;
    }
    if sections & S_QUEUE != 0 {
        log.record("uncategorized_by_merchant");
        snap.queue = uncategorized_by_merchant(c)?;
    }
    if sections & S_TRANSFERS != 0 {
        log.record("list_transfer_reviews");
        snap.reviews = list_transfer_reviews(c)?;
    }
    if sections & S_IMPORTS != 0 {
        log.record("list_imports");
        snap.imports = list_imports(c)?;
    }
    if sections & S_DASHBOARD != 0 {
        let (ids, none_selected) = match &dash {
            DashAccounts::Resolved { ids, none_selected } => (ids.clone(), *none_selected),
            DashAccounts::Fresh => {
                let ids: Vec<i64> = snap
                    .accounts
                    .iter()
                    .filter(|a| !f.excluded.contains(&a.id))
                    .map(|a| a.id)
                    .collect();
                let none_selected = ids.is_empty() && !snap.accounts.is_empty();
                (ids, none_selected)
            }
        };
        // Keep the shared `ViewFilter` in step with the checkboxes:
        // empty means all accounts.
        let mut view_filter = f.view_filter.clone();
        view_filter.account_ids = if f.excluded.is_empty() {
            Vec::new()
        } else {
            ids
        };
        if none_selected {
            snap.rows = Vec::new();
            snap.chart = Vec::new();
            snap.summary = Some(MonthlySummary {
                months: Vec::new(),
                rows: Vec::new(),
            });
        } else {
            log.record("list_transactions_filtered");
            snap.rows = list_transactions_filtered(c, &view_filter, &f.table_filter)?;
            // Chart and legend rows: parents, or the drilled-in parent's
            // children (3.5).
            log.record("spend_by_category");
            snap.chart = spend_by_category(c, &view_filter, f.chart_parent)?;
            // The monthly summary grid (3.6): the same filter as the
            // chart, one query for the whole panel.
            log.record("monthly_summary");
            snap.summary = Some(monthly_summary(c, &view_filter)?);
        }
    }
    Ok(snap)
}
