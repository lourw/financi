//! 3.78: the shared data model. `Catalog` owns the read-model data every
//! page renders from, reloads from the database only the parts a reported
//! change can touch (keyed latest-wins jobs, 3.77), and announces the
//! result with [`DataChanged`] so `Shell` repaints exactly the pages that
//! depend on it. Writers report [`Domain`]s, readers declare them — see
//! the table on [`Domain`] and [`Catalog::sections_for`].

use super::*;
use crate::ui::widgets::on_db;
use financi_core::rules::{CompiledPattern, CompiledRules};
use financi_db::rules::RuleUse;
use financi_db::{DbError, QueryLog};

/// One slice of the shared data.
///
/// This is *the* reload table (3.78, 3.57), both lists in one. The
/// **write** rows are the writer side: every mutation's UI handler
/// reports exactly the domains marked `●` — nothing picks a reload per
/// click, and only the full-reload writes ([`Domains::ALL`]: import, undo
/// import, restore) report everything. The **page** rows are the reader
/// side: each page entity's `const DEPENDS` (see `ui/page.rs`). A write
/// reloads a shown page — and dirties a hidden one, which reloads when
/// next shown — exactly when its `●` meets that page's `✓` in some
/// column; the page switch itself never queries.
/// [`Catalog::sections_for`] maps a report to the queries behind the
/// columns.
///
/// | | Accounts | Categories | Rules | Transactions | Imports | Transfers | Settings |
/// |---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
/// | **write** rule add / edit / delete / move | | | ● | ● | | | |
/// | **write** set category, hide, unhide | | | | ● | | | |
/// | **write** category add / rename / move / delete | | ● | ● | ● | | | |
/// | **write** pair / unpair / transfer review | | | | ● | | ● | |
/// | **write** new account (import panel) | ● | | | | | | |
/// | **write** import, undo import, restore | ● | ● | ● | ● | ● | ● | ● |
/// | **page** Dashboard | ✓ | ✓ | ✓ | ✓ | ✓ | | ✓ |
/// | **page** To-categorize | | ✓ | ✓ | ✓ | | | |
/// | **page** Categories | | ✓ | ✓ | ✓ | | | |
/// | **page** Rules | | ✓ | ✓ | ✓ | | | |
/// | **page** Transfers | | | | | ✓ | ✓ | |
/// | **page** Imports | | | | | ✓ | | |
///
/// The dashboard's view settings are the one write that never travels
/// through a report: they move only the dashboard's rows, chart and
/// summary, and the page reloads itself off them (3.56) — which is why
/// `sections_for` maps `Settings` to no shared section at all.
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
    /// 3.85: the one-domain set, spelled by the cache-rebuild test (the
    /// writer table's writes all report shorthands that bundle domains).
    #[cfg(test)]
    pub(super) fn single(domain: Domain) -> Self {
        Self(domain.bit())
    }
    pub(super) const ALL: Self = Self(0b0111_1111);
    // The writer table's shorthands, spelled once.
    pub(super) const TX: Self = Self(Domain::Transactions.bit());
    pub(super) const RULES_TX: Self = Self(Domain::Rules.bit() | Domain::Transactions.bit());
    pub(super) const CATEGORIES_RULES_TX: Self =
        Self(Domain::Categories.bit() | Domain::Rules.bit() | Domain::Transactions.bit());
    pub(super) const TRANSFERS_TX: Self =
        Self(Domain::Transfers.bit() | Domain::Transactions.bit());
    pub(super) const SETTINGS: Self = Self(Domain::Settings.bit());
    // The single-domain sets the page entities' `DEPENDS` (3.79) spell
    // with [`Domains::union`].
    pub(super) const IMPORTS: Self = Self(Domain::Imports.bit());
    pub(super) const TRANSFERS: Self = Self(Domain::Transfers.bit());
    pub(super) const ACCOUNTS: Self = Self(Domain::Accounts.bit());

    pub(super) fn contains(self, domain: Domain) -> bool {
        self.0 & domain.bit() != 0
    }

    /// Union of two sets, const-callable so a page's `DEPENDS` can be a
    /// `const` (the `BitOr` impls are not const).
    pub(super) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Do the two sets share a domain? A page's `DataChanged` filter (3.79)
    /// is `event.domains.intersects(Self::DEPENDS)`.
    pub(super) fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
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
const S_ALL: u16 = 0x7F;

/// The reader dependency map (3.78): which sections a change to `domains`
/// can invalidate. Over-fetching is fine where nothing forbids it (a
/// change to one domain may load a little more than strictly needed);
/// under-fetching is not. The dashboard is not in this map (3.56): its
/// rows, chart and summary are a keyed load of their own, started by
/// [`Shell`](crate::ui::Shell) whenever a domain it reads — or one of its
/// own filter controls — changes. The Imports history answers to Imports
/// alone and the transfer reviews to Transfers alone, which is what keeps
/// a rule edit off both.
fn sections_for(domains: Domains) -> u16 {
    let mut s: u16 = 0;
    if domains.contains(Domain::Accounts) {
        // A new account joins the filter checkboxes, and the sidebar
        // counts count its rows.
        s |= S_ACCOUNTS | S_SIDEBAR;
    }
    if domains.contains(Domain::Categories) {
        // The tree and its per-category counts.
        s |= S_CATEGORIES;
    }
    if domains.contains(Domain::Rules) {
        s |= S_RULES;
    }
    if domains.contains(Domain::Transactions) {
        // Counts, rule uses, sidebar totals and the queue all read the
        // transaction rows.
        s |= S_CATEGORIES | S_RULES | S_SIDEBAR | S_QUEUE;
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
        // View settings move the dashboard only, and that is the
        // dashboard loader's job now (3.56).
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
    /// 3.58: rule id → signed sum, in cents, of the transactions the rule
    /// decided (3.87's `rule_use_stats` total). Rules that decided nothing
    /// are absent. The Rules page renders it as the Total column and sums
    /// it per group.
    pub rule_totals: HashMap<i64, i64>,
    pub total: i64,
    pub uncategorized: i64,
    pub queue: Vec<UncategorizedGroup>,
    pub reviews: Vec<TransferReview>,
    pub imports: Vec<ImportEntry>,
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
    /// A launch-load master-CSV self-check failure to toast (3.76). The
    /// launch *settings* moved to the dashboard's own load (3.56), which
    /// is where the filter controls need them.
    pub startup: Option<String>,
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
    /// 3.78 review fix: the newest generation whose result has been
    /// applied to each section bit. The queues order *jobs*, not the
    /// continuations that apply their results: a finished job's result
    /// waits in the entity queue, and a newer dispatch — same key on the
    /// reader queue (its supersede only reaches pending and running jobs),
    /// or any partial against a writer-queue full reload — can run to
    /// completion and land first. A result whose dispatch is older than
    /// what a bit has already applied is stale and must not write that
    /// bit; it may still write the rest.
    applied_gen: [u64; 7],
    /// 3.85: `snap.rules` compiled once per rules reload, so render paths
    /// (the table's *Show rule* marker, the Rules page's test mode) match
    /// descriptions without recompiling patterns every frame. Same
    /// semantics as [`financi_core::rules::winning_rule`].
    compiled: Rc<CompiledRules>,
    /// 3.85: every rule's own compiled pattern, keyed by rule id — the
    /// Rules page's test-mode filter asks "does this rule match the text"
    /// per rule, which [`CompiledRules`] does not expose per rule.
    patterns: Rc<HashMap<i64, CompiledPattern>>,
    /// 3.85: rule id → "Parent › Child" target label (the `#{id}`
    /// fallback for a missing child included), so the Rules page builds
    /// its rows without a per-rule tree lookup. Rebuilt whenever the
    /// tree or the rules change.
    rule_labels: Rc<HashMap<i64, SharedString>>,
    /// 3.85: category id → index into the 12-colour palette (the 3.23
    /// assignment order). Theme-independent, so it survives a repaint:
    /// consumers resolve `palette[ix % 12]` with the *current* theme,
    /// exactly as the per-frame `category_colors` build did.
    cat_colors: Rc<HashMap<i64, usize>>,
    /// 3.59: the shared grouped category-picker items — one `SelectGroup`
    /// per parent category, its children as `SubChoice` rows. A pure
    /// function of the tree, rebuilt with it; every category picker wraps
    /// a clone in its own `SearchableVec` (the filter state is per picker).
    /// `Rc::ptr_eq` on the accessor is the "did the tree change" test.
    cat_choices: Rc<Vec<SelectGroup<SubChoice>>>,
    /// 3.85 test hook: rebuild counts per derived cache, asserted by
    /// `caches_rebuild_only_when_their_inputs_change`.
    #[cfg(test)]
    pub(super) cache_rebuilds: std::cell::Cell<(usize, usize, usize)>,
}

impl Catalog {
    pub(super) fn new(db: DbWorker) -> Self {
        Self {
            db,
            snap: Snapshot::default(),
            full_task: None,
            applied_gen: [0; 7],
            compiled: Rc::new(CompiledRules::new(&[])),
            patterns: Rc::new(HashMap::new()),
            rule_labels: Rc::new(HashMap::new()),
            cat_colors: Rc::new(HashMap::new()),
            cat_choices: Rc::new(Vec::new()),
            #[cfg(test)]
            cache_rebuilds: std::cell::Cell::new((0, 0, 0)),
        }
    }

    /// The rules compiled once per rules reload (3.85).
    pub(super) fn compiled_rules(&self) -> Rc<CompiledRules> {
        Rc::clone(&self.compiled)
    }

    /// The rule id → compiled pattern map (3.85).
    pub(super) fn rule_patterns(&self) -> Rc<HashMap<i64, CompiledPattern>> {
        Rc::clone(&self.patterns)
    }

    /// The rule id → target label map (3.85).
    pub(super) fn rule_labels(&self) -> Rc<HashMap<i64, SharedString>> {
        Rc::clone(&self.rule_labels)
    }

    /// The category id → palette index map (3.85); resolve indices with
    /// [`category_palette`] at draw time.
    pub(super) fn cat_colors(&self) -> Rc<HashMap<i64, usize>> {
        Rc::clone(&self.cat_colors)
    }

    /// The grouped picker items (3.59); rebuild a picker's `SearchableVec`
    /// from a clone whenever this pointer changes.
    pub(super) fn cat_choices(&self) -> Rc<Vec<SelectGroup<SubChoice>>> {
        Rc::clone(&self.cat_choices)
    }

    /// The reader dependency map: the sections `domains` reloads.
    pub(super) fn sections_for(domains: Domains) -> u16 {
        sections_for(domains)
    }

    /// A write landed: report the sections `domains` touch, reload them,
    /// then emit [`DataChanged`]. The job is keyed by its section set, so
    /// a burst of the same kind of change runs once (3.77) while different
    /// section sets still run side by side. The dashboard is not one of
    /// these sections (3.56) — `Shell` starts its keyed load alongside.
    /// `rule_use` (3.55) is the use counts a rule write already counted:
    /// when present the rules section takes them and skips the
    /// `rule_use_stats` scan.
    pub(super) fn changed(
        &mut self,
        domains: Domains,
        generation: u64,
        rule_use: Option<HashMap<i64, RuleUse>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sections = sections_for(domains);
        if sections == S_ALL {
            self.reload_full(domains, generation, None, window, cx);
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
        let db = self.db.clone();
        let log = db.query_log();
        let key = format!("catalog:{sections}");
        cx.spawn_in(window, async move |this, cx| {
            let result = db
                .call_keyed(&key, move |c| load_sections(c, &log, sections, rule_use))
                .await;
            let _ = this.update_in(cx, |this, _window, cx| match result {
                Ok(snapshot) => {
                    // Staleness guard (3.78 review fix): merge only the
                    // bits no newer dispatch has applied yet; the emit
                    // still settles the generation either way.
                    let fresh = this.fresh(sections, generation);
                    this.merge(fresh, snapshot);
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

    /// Reload everything in one writer-thread job: the launch load and the
    /// import / undo / restore flows, where every part of the read model
    /// really is suspect. One job, so `job_count` stays at one for the
    /// launch invariant — the dashboard's own load is the second job of a
    /// launch (3.56). `startup` is `Some(master CSV root)` only for the
    /// launch load.
    pub(super) fn reload_full(
        &mut self,
        domains: Domains,
        generation: u64,
        startup: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let db = self.db.clone();
        let log = db.query_log();
        self.full_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                // 3.76: the master-CSV self-check rides this job when it
                // is the launch one, so startup stays a single worker job
                // and the window shows at once. (3.56: the launch
                // settings moved to the dashboard's load, where the
                // filter controls consume them.)
                let csv_check = match startup {
                    Some(csv_root) => {
                        log.record("master_csv::self_check");
                        financi_db::master_csv::self_check(c, &csv_root)
                            .err()
                            .map(|e| e.to_string())
                    }
                    None => None,
                };
                let snap = load_sections(c, &log, S_ALL, None)?;
                Ok::<_, DbError>((snap, csv_check))
            })
            .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok((snap, startup)) => {
                        // Cross-queue guard (3.78 review fix): the keyed
                        // jobs dispatched while this writer-queue job ran
                        // can land on the reader queue first, and this
                        // job's own result can still wait in the entity
                        // queue behind theirs. Merge every bit no newer
                        // dispatch has applied — the rest of the snapshot
                        // is still the only copy of that data.
                        let fresh = this.fresh(S_ALL, generation);
                        this.merge(fresh, snap);
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
                }
            });
        }));
    }

    /// The section bits a result of dispatch `generation` may still write,
    /// recorded as applied at that generation: a bit a newer dispatch has
    /// already applied is stale (see [`Catalog::applied_gen`]).
    fn fresh(&mut self, sections: u16, generation: u64) -> u16 {
        let mut fresh_bits = 0;
        for b in 0..7 {
            let bit = 1u16 << b;
            if sections & bit != 0 && self.applied_gen[b] < generation {
                self.applied_gen[b] = generation;
                fresh_bits |= bit;
            }
        }
        fresh_bits
    }

    fn merge(&mut self, sections: u16, s: Snapshot) {
        if sections & S_ACCOUNTS != 0 {
            self.snap.accounts = s.accounts;
        }
        let fresh_categories = sections & S_CATEGORIES != 0;
        let fresh_rules = sections & S_RULES != 0;
        if fresh_categories {
            self.snap.tree = s.tree;
            self.snap.counts = s.counts;
            // 3.85: the palette-index map is a pure function of the tree.
            let mut colors = HashMap::new();
            let mut parents: Vec<&CategoryNode> = self.snap.tree.iter().collect();
            parents.sort_by_key(|n| n.id);
            for (ix, parent) in parents.iter().enumerate() {
                colors.insert(parent.id, ix);
                let mut kids: Vec<&CategoryNode> = parent.children.iter().collect();
                kids.sort_by_key(|n| n.id);
                for (cix, kid) in kids.iter().enumerate() {
                    colors.insert(kid.id, cix);
                }
            }
            self.cat_colors = Rc::new(colors);
            // 3.59: the grouped picker items are a pure function of the
            // tree too — built once here, shared by every picker.
            self.cat_choices = Rc::new(sub_choice_groups(&self.snap.tree));
            #[cfg(test)]
            self.bump_cache(1);
        }
        if fresh_rules {
            self.snap.rules = s.rules;
            self.snap.uses = s.uses;
            self.snap.rule_totals = s.rule_totals;
            // 3.85: patterns and the winner table are pure functions of
            // the rule list; compile them once here, not per frame.
            self.compiled = Rc::new(CompiledRules::new(&self.snap.rules));
            self.patterns = self
                .snap
                .rules
                .iter()
                .map(|r| (r.id, CompiledPattern::new(&r.pattern, r.is_regex)))
                .collect::<HashMap<_, _>>()
                .into();
            #[cfg(test)]
            self.bump_cache(0);
        }
        if fresh_categories || fresh_rules {
            // 3.85: "Parent › Child" per rule — the child map is a pure
            // function of the tree, resolved against every rule.
            let children: HashMap<i64, SharedString> = self
                .snap
                .tree
                .iter()
                .flat_map(|p| {
                    p.children
                        .iter()
                        .map(move |c| (c.id, format!("{} › {}", p.name, c.name).into()))
                })
                .collect();
            self.rule_labels = self
                .snap
                .rules
                .iter()
                .map(|r| {
                    let label = children
                        .get(&r.category_id)
                        .cloned()
                        .unwrap_or_else(|| format!("#{}", r.category_id).into());
                    (r.id, label)
                })
                .collect::<HashMap<_, _>>()
                .into();
            #[cfg(test)]
            self.bump_cache(2);
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
    }

    /// 3.85 test hook: count a rebuild of one derived cache —
    /// 0 = compiled rules, 1 = category colours, 2 = rule labels.
    #[cfg(test)]
    fn bump_cache(&mut self, which: usize) {
        let (a, b, c) = self.cache_rebuilds.get();
        self.cache_rebuilds.set(match which {
            0 => (a + 1, b, c),
            1 => (a, b + 1, c),
            _ => (a, b, c + 1),
        });
    }
}

/// Run the sections' queries on the worker thread, recording each name so
/// a test can see exactly what ran (the query-count hook, 3.78). The
/// dashboard's own queries (3.56) live in `dashboard.rs`, not here.
/// `rule_use` (3.55) short-circuits the `rule_use_stats` scan when the
/// triggering rule write already counted the uses.
fn load_sections(
    c: &mut Connection,
    log: &QueryLog,
    sections: u16,
    rule_use: Option<HashMap<i64, RuleUse>>,
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
        // 3.28: the Rules page Uses column. 3.87 added the per-rule
        // totals; 3.58 renders them as the Total column, so both halves
        // of the stat ride along now. 3.55: a rule write hands the fresh
        // counts to the reload, and only reloads without them — launch,
        // import, undo — still pay for the scan.
        let stats = match rule_use {
            Some(stats) => stats,
            None => {
                log.record("rule_use_stats");
                rule_use_stats(c)?
            }
        };
        let mut counts = HashMap::new();
        let mut totals = HashMap::new();
        for (id, stats) in stats {
            counts.insert(id, stats.count);
            totals.insert(id, stats.total_cents);
        }
        snap.uses = counts;
        snap.rule_totals = totals;
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
    Ok(snap)
}
