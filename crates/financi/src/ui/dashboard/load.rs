//! The dashboard filter state and the page's one keyed load (3.56, 3.65, 3.77).

use super::*;
use financi_db::QueryLog;

/// 3.56: the dashboard's whole filter state in one comparable value —
/// every input its queries take, from the date range down to the sort.
/// A change to any filter control updates this and starts the dashboard's
/// one keyed load; the table, chart and summary are its three results.
/// `PartialEq` is what makes "did anything actually change?" one check.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DashboardFilter {
    /// The saved date range shown by the preset picker.
    pub(in crate::ui) date_range: DateRange,
    /// The accounts the user unchecked in the dropdown (empty = all).
    pub(in crate::ui) excluded_accounts: Vec<i64>,
    /// The search term. Empty means "no narrowing".
    pub(in crate::ui) search: String,
    /// The "Include transfers" flag persisted as `ui.include_transfers`.
    pub(in crate::ui) include_transfers: bool,
    /// The "Uncategorized only" and "Show hidden" table toggles.
    pub(in crate::ui) uncategorized_only: bool,
    pub(in crate::ui) show_hidden: bool,
    /// The legend selection (3.5) and the summary-cell month (3.6) —
    /// the dashboard's category and month narrowing — and the chart and
    /// legend's drill-down level (`None` = the parent level).
    pub(in crate::ui) legend_selection: Option<LegendSelection>,
    pub(in crate::ui) summary_month: Option<String>,
    pub(in crate::ui) chart_parent: Option<i64>,
    /// The table's column sort (3.4).
    pub(in crate::ui) tx_sort: TxSort,
}

impl DashboardFilter {
    /// The shared [`ViewFilter`] for a run of the queries: the range's
    /// dates as of `today`, the accounts the checkboxes still include,
    /// and the transfers flag. An empty exclusion list means all
    /// accounts — an empty `account_ids` too.
    fn view_filter(&self, today: NaiveDate, account_ids: Vec<i64>) -> ViewFilter {
        let (from, to) = self.date_range.range(today);
        ViewFilter {
            from,
            to,
            account_ids: if self.excluded_accounts.is_empty() {
                Vec::new()
            } else {
                account_ids
            },
            include_transfers: self.include_transfers,
        }
    }

    /// The dashboard's view-wide narrowing (3.46): the search and the two
    /// toggles, applied to the chart, the summary and the table alike.
    /// The legend selection is not part of it — on chart and summary a
    /// selection is a highlight; it filters only the table.
    fn view_narrow(&self) -> ViewNarrow {
        ViewNarrow {
            search: if self.search.is_empty() {
                None
            } else {
                Some(self.search.clone())
            },
            uncategorized_only: self.uncategorized_only,
            show_hidden: self.show_hidden,
        }
    }

    /// The table's extra filters (3.4): the view-wide narrowing, the sort,
    /// plus the legend selection (3.5) — the `Uncategorized` row rides
    /// `uncategorized_only` here and not in [`Self::view_narrow`] — and
    /// the summary-cell month (3.6).
    fn table_filter(&self) -> TxTableFilter {
        TxTableFilter {
            narrow: ViewNarrow {
                uncategorized_only: self.uncategorized_only
                    || matches!(self.legend_selection, Some(LegendSelection::Uncategorized)),
                ..self.view_narrow()
            },
            category_id: match self.legend_selection {
                Some(LegendSelection::Category(id)) => Some(id),
                Some(LegendSelection::Uncategorized) | None => None,
            },
            month: self.summary_month.clone(),
            sort: self.tx_sort,
        }
    }
}

/// The result of the dashboard's one load (3.56): everything the page
/// renders from the database, and the [`ViewFilter`] the queries actually
/// ran with — the shell stamps it as the record of the last applied
/// query. `settings` carries the persisted pair the launch load reads
/// inside its own query (3.65), so the first frame shows the saved view.
pub(crate) struct DashboardData {
    pub(in crate::ui) rows: Vec<TransactionRow>,
    pub(in crate::ui) chart: Vec<CategorySpend>,
    pub(in crate::ui) summary: MonthlySummary,
    pub(in crate::ui) view_filter: ViewFilter,
    pub(in crate::ui) settings: Option<(DateRange, bool)>,
}

/// The dashboard's job body (3.56): resolve the checked accounts against
/// the account list, then run the filtered table query and the one
/// category/summary aggregation — or, drilled into a parent, the child
/// chart and the grid alone. `startup` reads the two persisted view
/// settings in the same trip and queries *with* them (3.65).
///
/// `accounts` is the shell's mirror when a filter control changed (the
/// list is on screen already); `None` makes the job read the list itself,
/// which is what the launch and the after-a-write reloads need
/// so account changes resolve in the same round trip (3.17).
pub(crate) fn load_dashboard(
    c: &mut Connection,
    log: &QueryLog,
    mut filter: DashboardFilter,
    accounts: Option<Vec<Account>>,
    startup: bool,
) -> Result<DashboardData, DbError> {
    let settings = if startup {
        log.record("settings::date_range");
        let range = settings::date_range(c)?;
        log.record("settings::include_transfers");
        let transfers = settings::include_transfers(c)?;
        // The queries below run with the saved values, so the first
        // frame is already the saved view.
        filter.date_range = range;
        filter.include_transfers = transfers;
        Some((range, transfers))
    } else {
        None
    };
    let fetched;
    let accounts: &[Account] = match &accounts {
        Some(list) => list,
        None => {
            log.record("list_accounts");
            fetched = list_accounts(c)?;
            &fetched
        }
    };
    let ids: Vec<i64> = accounts
        .iter()
        .filter(|a| !filter.excluded_accounts.contains(&a.id))
        .map(|a| a.id)
        .collect();
    // "Nothing checked" with accounts present: empty results, no queries.
    if ids.is_empty() && !accounts.is_empty() {
        let view_filter = filter.view_filter(DashboardPage::today(), ids);
        return Ok(DashboardData {
            rows: Vec::new(),
            chart: Vec::new(),
            summary: MonthlySummary {
                months: Vec::new(),
                rows: Vec::new(),
            },
            view_filter,
            settings,
        });
    }
    let view_filter = filter.view_filter(DashboardPage::today(), ids);
    let table_filter = filter.table_filter();
    let narrow = filter.view_narrow();
    log.record("list_transactions_filtered");
    let rows = list_transactions_filtered(c, &view_filter, &table_filter)?;
    let (chart, summary) = match filter.chart_parent {
        None => {
            // Both aggregations in one pass (3.88); the selected month
            // narrows the chart rows only, never the grid (3.46).
            log.record("dashboard");
            financi_db::dashboard(c, &view_filter, &narrow, filter.summary_month.as_deref())?
        }
        Some(parent) => {
            log.record("spend_by_category");
            let chart = spend_by_category(
                c,
                &view_filter,
                Some(parent),
                &narrow,
                filter.summary_month.as_deref(),
            )?;
            log.record("monthly_summary");
            let summary = monthly_summary(c, &view_filter, &narrow)?;
            (chart, summary)
        }
    };
    Ok(DashboardData {
        rows,
        chart,
        summary,
        view_filter,
        settings,
    })
}

impl DashboardPage {
    /// Take the next dispatch generation. It comes off the shell's shared
    /// reload counter (3.75): the tests' `wait_idle` compares that
    /// counter's two marks, and this page's loads must be counted in it
    /// or waiting would race them. The cell — not the shell entity — is
    /// what makes the bump safe to take from inside the page's own
    /// update even when the shell is borrowed around it (an apply
    /// arriving through `go_page`).
    fn next_generation(&self) -> u64 {
        let g = self.refresh_started.get() + 1;
        self.refresh_started.set(g);
        g
    }

    /// Stamp the idle marker (3.75): the maximum, since generations only
    /// move forward and a replaced load never stamps.
    fn stamp_applied(&self, generation: u64) {
        self.refresh_applied
            .set(self.refresh_applied.get().max(generation));
    }

    /// 3.56: start the page's one load — filtered rows, chart and summary
    /// in a single keyed job (3.77: a burst of filter changes leaves at
    /// most one in flight plus the newest, and every replaced job resolves
    /// `Superseded` without touching the screen). The filter is snapshotted
    /// now, so the load runs with exactly what the controls showed when it
    /// started. `accounts`: the snapshot list `apply` just pulled (3.82);
    /// `None` reads it in the job (the stale-filter re-query below).
    /// `startup` reads the persisted settings in the same trip (3.65).
    pub(super) fn start_load(
        &mut self,
        accounts: Option<Vec<Account>>,
        startup: bool,
        cx: &mut Context<Self>,
    ) {
        let generation = self.next_generation();
        self.load_gen = generation;
        let filter = self.filter.clone();
        let db = self.db.clone();
        let log = db.query_log();
        // Detached (3.64): nothing may cancel the continuation between
        // the query finishing and the result being guarded in.
        cx.spawn(async move |this, cx| {
            let result = db
                .call_keyed("dashboard", move |c| {
                    load_dashboard(c, &log, filter, accounts, startup)
                })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(data) => this.finish_load(generation, data, cx),
                // A newer filter change took over while this ran; its
                // result replaces this one's.
                Err(DbError::Superseded) => {}
                Err(e) => {
                    this.notify_err(format!("Database error: {e}"), cx);
                    this.stamp_applied(generation);
                }
            });
        })
        .detach();
    }

    /// A dashboard load came back: apply it, unless a newer load was
    /// dispatched while it ran — then this result is stale whole, rows
    /// included, and the newer load's landing replaces it (3.56).
    fn finish_load(&mut self, generation: u64, mut data: DashboardData, cx: &mut Context<Self>) {
        if generation < self.load_gen {
            return;
        }
        // 3.65: the launch settings the job already queried with land on
        // the state and the filter controls before the data of the same
        // round trip is applied, so the first frame shows the saved range
        // everywhere. The guard above is what "the user changed it since,
        // and their change wins" means here: a newer dispatch replaced
        // this load, and its result carries the newer controls. In 3.82
        // the change can also arrive *before* the launch load was even
        // dispatched (a scripted preset change right after opening): the
        // job still ran with the saved values, so the result is dropped
        // and the user's filter is re-queried instead of applied.
        if let Some((range, include_transfers)) = data.settings {
            if self.filter_touched {
                self.reload(cx);
                self.stamp_applied(generation);
                return;
            }
            self.set_date_range(range, cx);
            self.filter.include_transfers = include_transfers;
            data.view_filter.include_transfers = include_transfers;
        }
        self.apply_dashboard_rows(data, cx);
        self.stamp_applied(generation);
        cx.notify();
    }

    /// Re-run the load with the current controls (3.56): what every
    /// filter-control change below ends in, in place of the old
    /// `Shell::changed(Domains::SETTINGS)` round trip — the shared
    /// sections never change on a filter change, only this page's rows.
    pub(in crate::ui) fn reload(&mut self, cx: &mut Context<Self>) {
        // A control can move before the first snapshot lands (the tests
        // script a preset change right after opening the window): with no
        // account list yet, the job reads it, so a scripted change never
        // supersedes the launch load with an empty-account query.
        let accounts = (!self.accounts.is_empty()).then(|| self.accounts.clone());
        self.start_load(accounts, false, cx);
    }

    /// Feedback out: the shell paints toasts (3.27); the page only
    /// reports what happened.
    pub(super) fn notify_err(&self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast {
            message: message.into(),
            error: true,
        });
    }

    pub(super) fn notify_ok(&self, message: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast {
            message: message.into(),
            error: false,
        });
    }

    /// Apply the dashboard's one load result (3.56), last: the table's
    /// delegate reads the child choices and card-payment category the
    /// shell's `sync_category_choices` set on the last `DataChanged` —
    /// which arrived before this job was even dispatched. A selection the
    /// new data no longer supports is corrected and the load restarted —
    /// and the half-matched rows are *not* applied (3.56: the screen
    /// never shows rows queried under a filter the controls no longer
    /// hold).
    fn apply_dashboard_rows(&mut self, data: DashboardData, cx: &mut Context<Self>) {
        // Revalidate the legend state against the rows we can actually
        // show: a date/accounts change can empty the chart out from under
        // a selection or a drill-down, and the table filter must not keep
        // narrowing by a category that is no longer there.
        let mut stale = false;
        let drilled = self.filter.chart_parent;
        if drilled.is_some() && data.chart.is_empty() {
            self.filter.chart_parent = None;
            stale = true;
        }
        let selection_gone = match self.filter.legend_selection {
            Some(LegendSelection::Category(id)) => {
                // While drilled, the selected parent itself is not among
                // the child rows — it stays.
                drilled != Some(id) && !data.chart.iter().any(|r| r.id == Some(id))
            }
            Some(LegendSelection::Uncategorized) => {
                self.filter.chart_parent.is_none() && !data.chart.iter().any(|r| r.id.is_none())
            }
            None => false,
        };
        if selection_gone {
            self.filter.legend_selection = None;
            stale = true;
        }
        // The month selection (3.6) goes the same way when the grid no
        // longer contains that month.
        if self.filter.summary_month.is_some()
            && !data
                .summary
                .months
                .iter()
                .any(|m| Some(m.as_str()) == self.filter.summary_month.as_deref())
        {
            self.filter.summary_month = None;
            stale = true;
        }
        if stale {
            // Re-query with the corrected filter so the table, chart and
            // state agree again. The half-matched rows are dropped: the
            // new load's result takes their place.
            self.reload(cx);
            return;
        }
        self.row_count = data.rows.len();
        self.chart_rows = data.chart;
        // A range change shows new months: park the month strip (3.30) at
        // its right end so the most recent month sits beside the pinned
        // `Total` column. The huge negative offset clamps to the true
        // maximum when the strip next paints.
        if self.summary.months != data.summary.months {
            self.summary_scroll
                .set_offset(point(px(-1_000_000.), px(0.)));
            // 3.93: the stacked bars' month strip parks at the latest
            // month the same way.
            self.stacks_scroll
                .set_offset(point(px(-1_000_000.), px(0.)));
        }
        self.summary = data.summary;
        // 3.85: the grid's cached views follow the summary.
        self.summary_views_dirty = true;
        // 3.93: and so do the stacked bars' shaped months.
        self.stacks_dirty = true;
        // Record the filter the applied rows were queried with (3.56).
        self.view_filter = data.view_filter;
        // The rule offer follows a row the user just edited; a load
        // that drops the row (hidden, filtered out, deleted) drops the
        // offer with it.
        if self
            .rule_offer
            .is_some_and(|(id, _)| !data.rows.iter().any(|r| r.id == id))
        {
            self.rule_offer = None;
        }
        let rows = data.rows;
        // 3.82: the row-menu mirrors live on the shell (the Rules and
        // Categories pages read them too); every write refreshed them
        // before this job was dispatched, so they name the same rows
        // these do. 3.85: the *Show rule* matcher takes the Catalog's
        // compiled rules — built once per rules reload, not per menu.
        let compiled_rules = self.catalog.read(cx).compiled_rules();
        let (child_choices, card_payment) = self
            .shell
            .update(cx, |shell, _| {
                (shell.child_choices.clone(), shell.card_payment_category)
            })
            .unwrap_or_default();
        let rule_offer = self.rule_offer.map(|(id, _)| id);
        let sort = self.filter.tx_sort;
        self.table.update(cx, |t, cx| {
            let delegate = t.delegate_mut();
            delegate.set_rows(rows);
            delegate.child_choices = child_choices;
            delegate.rule_offer = rule_offer;
            delegate.rules = compiled_rules;
            delegate.card_payment_category = card_payment;
            delegate.sort = sort;
            t.refresh(cx);
        });
    }
}
