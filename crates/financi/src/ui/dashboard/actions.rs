//! The filter-control handlers, the row actions from the table menu,
//! "Clear filters" and the shell forwards (2.6, 2.9, 3.2–3.6, 3.82).

use super::*;
use crate::ui::page::{Cancel, PageEvent};

/// What one optimistic click changed in the table (3.84), so a failed
/// write can put it back exactly.
enum RowUndo {
    /// The row was edited in place; the saved copy goes back where its id
    /// is — `None` when the row wasn't in the table at click time, which
    /// leaves nothing to undo.
    Edited(Option<TransactionRow>),
    /// The row left the table; it goes back at its old index unless a
    /// reload has already brought it back.
    Removed(usize, TransactionRow),
}

impl RowUndo {
    /// Put back what the click painted (3.84). The failed write left the
    /// database at the pre-click values and the `PageEvent::Changed` that
    /// follows reloads them, so this only covers the frames in between;
    /// a row a reload has meanwhile replaced or dropped is left alone.
    fn restore(&self, cx: &mut App, table: &Entity<TableState<TxTable>>) {
        table.update(cx, |t, cx| {
            let rows = &mut t.delegate_mut().rows;
            match self {
                RowUndo::Edited(Some(saved)) => {
                    if let Some(row) = rows.iter_mut().find(|r| r.id == saved.id) {
                        *row = saved.clone();
                    }
                }
                RowUndo::Edited(None) => {}
                RowUndo::Removed(index, row) => {
                    if !rows.iter().any(|r| r.id == row.id) {
                        rows.insert((*index).min(rows.len()), row.clone());
                    }
                }
            }
            t.refresh(cx);
        });
    }
}

/// Roll back one failed write's optimistic paint (3.84); see
/// [`RowUndo::restore`].
fn restore_row(cx: &mut App, table: &Entity<TableState<TxTable>>, undo: &RowUndo) {
    undo.restore(cx, table)
}

impl DashboardPage {
    /// Put a date range on the state and the controls without persisting
    /// and without refreshing (3.65: split from `apply_date_range` so the
    /// launch settings can land inside the refresh that loaded them).
    /// 3.82: the widgets are only marked — a load landing has no window
    /// to drive them with, and `render` catches up the same frame.
    pub(in crate::ui) fn set_date_range(&mut self, range: DateRange, _cx: &mut Context<Self>) {
        self.filter.date_range = range;
        let (from, to) = range.range(Self::today());
        self.view_filter.from = from;
        self.view_filter.to = to;
        self.date_widgets_stale = true;
    }

    /// Mirror `filter.date_range` into the preset Select and the From/To
    /// pickers (3.82, run from `render`): `set_selected_index`,
    /// `set_value` and `set_date` don't emit their Change events, so this
    /// never re-persists or re-selects.
    pub(super) fn sync_date_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.date_widgets_stale) {
            return;
        }
        let index = DatePreset::ALL
            .iter()
            .position(|p| *p == self.filter.date_range.preset())
            .unwrap_or(0);
        self.date_preset_select.update(cx, |select, cx| {
            select.set_selected_index(Some(gpui_kit::component::IndexPath::new(index)), window, cx);
        });
        if let DateRange::Custom { from, to } = self.filter.date_range {
            self.date_from_picker.update(cx, |picker, cx| {
                picker.set_date(CalendarDate::Single(Some(from)), window, cx);
            });
            self.date_to_picker.update(cx, |picker, cx| {
                picker.set_date(CalendarDate::Single(Some(to)), window, cx);
            });
        }
    }

    /// Apply a date range to the state and the controls without
    /// persisting; user changes persist on top via [`Self::persist_date_range`].
    /// The table reads the date range, so this re-queries.
    fn apply_date_range(&mut self, range: DateRange, cx: &mut Context<Self>) {
        self.filter_touched = true;
        self.set_date_range(range, cx);
        self.reload(cx);
        cx.notify();
    }

    /// Write the current selection to the `settings` table through the
    /// worker. A failed write only costs the restore at the next launch.
    fn persist_date_range(&mut self, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let db = self.db.clone();
        let range = self.filter.date_range;
        self.date_range_task = Some(cx.spawn(async move |_, _| {
            let _ = on_db(&db, move |c| settings::set_date_range(c, &range)).await;
        }));
    }

    /// The user picked a preset in the Select (task 3.2).
    pub(in crate::ui) fn change_date_preset(
        &mut self,
        preset: DatePreset,
        _window: &mut Window,
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
        self.apply_date_range(range, cx);
        self.persist_date_range(cx);
    }

    /// The user changed one of the two custom [`DatePicker`]s.
    pub(in crate::ui) fn date_picked(
        &mut self,
        is_end: bool,
        day: NaiveDate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (from, to) = match self.filter.date_range {
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
        self.apply_date_range(DateRange::Custom { from, to }, cx);
        self.persist_date_range(cx);
    }

    /// The accounts checked in the filter dropdown: every account whose id
    /// is not in `excluded_accounts` (task 3.3).
    fn selected_account_ids(&self) -> Vec<i64> {
        self.accounts
            .iter()
            .filter(|a| !self.filter.excluded_accounts.contains(&a.id))
            .map(|a| a.id)
            .collect()
    }

    /// Mirror the account checkboxes into [`ViewFilter::account_ids`]
    /// (empty = all accounts). With *every* account unchecked the list
    /// collapses to empty, which reads as "all" to the chart queries; the
    /// table path handles that case itself in [`Self::refresh`].
    pub(in crate::ui) fn sync_account_filter(&mut self) {
        self.view_filter.account_ids = if self.filter.excluded_accounts.is_empty() {
            Vec::new()
        } else {
            self.selected_account_ids()
        };
    }

    /// Check or uncheck one account in the filter dropdown; the table
    /// follows immediately.
    pub(in crate::ui) fn set_account_checked(
        &mut self,
        id: i64,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter.excluded_accounts.retain(|x| *x != id);
        if !on {
            self.filter.excluded_accounts.push(id);
        }
        self.sync_account_filter();
        // 3.65: the checkbox repaints with this click, not when the
        // answer comes back.
        cx.notify();
        self.reload(cx);
    }

    /// The "Include transfers" checkbox: update the filter, refresh the
    /// table, and persist the choice as `ui.include_transfers`.
    pub(in crate::ui) fn set_include_transfers(
        &mut self,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        if on == self.filter.include_transfers {
            return;
        }
        self.filter_touched = true;
        self.filter.include_transfers = on;
        self.view_filter.include_transfers = on;
        let db = self.db.clone();
        self.include_transfers_task = Some(cx.spawn(async move |_, _| {
            let _ = on_db(&db, move |c| settings::set_include_transfers(c, on)).await;
        }));
        // 3.65: the checkbox and the Options label repaint with this click.
        cx.notify();
        self.reload(cx);
    }

    /// The search box changed: re-run the table query with the new term.
    /// Empty text clears the search.
    pub(in crate::ui) fn apply_search(
        &mut self,
        term: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if term == self.filter.search {
            return;
        }
        self.filter.search = term.to_string();
        // 3.65: the box repaints with the keystroke, not with the answer.
        cx.notify();
        self.reload(cx);
    }

    /// The "Uncategorized only" checkbox: narrow the table to rows with no
    /// category. Mutually exclusive with a category legend selection (3.5) —
    /// both feed the one table filter, and `category = X AND no category`
    /// could only ever be an empty table.
    pub(in crate::ui) fn set_uncategorized_only(
        &mut self,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if on == self.filter.uncategorized_only {
            return;
        }
        self.filter.uncategorized_only = on;
        if on
            && matches!(
                self.filter.legend_selection,
                Some(LegendSelection::Category(_))
            )
        {
            self.filter.legend_selection = None;
            self.filter.summary_month = None;
        }
        // 3.65: the box and any cleared legend highlight repaint at once.
        cx.notify();
        self.reload(cx);
    }

    /// The "Show hidden" checkbox: include hidden rows in the table (greyed).
    pub(in crate::ui) fn set_show_hidden(
        &mut self,
        on: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if on == self.filter.show_hidden {
            return;
        }
        self.filter.show_hidden = on;
        // 3.65: the checkbox repaints with this click, not with the answer.
        cx.notify();
        self.reload(cx);
    }

    /// A header sort icon was clicked: adopt the new ordering and re-query.
    /// This runs through `on_sort` from inside the table's own update, so it
    /// must not touch the table entity again (that would double-lease and
    /// panic). `TxTable::perform_sort` already set the delegate's `sort` to
    /// this value, and the async refresh continuation re-syncs it outside the
    /// lease, so only the shell's copy needs updating here.
    pub(in crate::ui) fn set_tx_sort(
        &mut self,
        sort: TxSort,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter.tx_sort = sort;
        // 3.65: the header icon repaints with this click.
        cx.notify();
        self.reload(cx);
    }

    /// Click the "← All categories" row: back to the parent level with no
    /// category filter on the table.
    pub(in crate::ui) fn show_all_categories(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter.chart_parent = None;
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        // 3.65: the legend repaints with this click, not with the answer.
        cx.notify();
        self.reload(cx);
    }

    /// Esc (the `dashboard::Cancel` action): clear the
    /// drill-down, the legend selection and the summary month together —
    /// they are one concept. 3.24: an open rule editor is the innermost
    /// thing, so Esc cancels it first and nothing else.
    pub(in crate::ui) fn clear_category_selection(
        &mut self,
        _: &Cancel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 3.80: the editor lives on the Rules page.
        let editing = self.rules_page.read(cx).editing_rule.is_some();
        if editing {
            self.rules_page.update(cx, |p, cx| p.cancel_edit_rule(cx));
            return;
        }
        if self.filter.chart_parent.is_none()
            && self.filter.legend_selection.is_none()
            && self.filter.summary_month.is_none()
        {
            return;
        }
        self.filter.chart_parent = None;
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        // 3.65: the drill-down, legend highlight and summary-cell highlight
        // repaint with this Esc, not with the answer.
        cx.notify();
        self.reload(cx);
    }
}

impl DashboardPage {
    /// Change one transaction's category from the row menu (2.9). `set_category`
    /// locks the row so rules never move it again (2.6); on success the row
    /// starts offering to extend this choice to its whole merchant group.
    /// 3.84: the row repaints with the new category in the click's frame,
    /// before the write runs; a failed write restores the saved row.
    pub(in crate::ui) fn change_row_category(
        &mut self,
        transaction_id: i64,
        category_id: i64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in
        // flight — the row id may name a different transaction after it.
        if self.writes_blocked.get() {
            return;
        }
        // 3.84: paint the new category on the row now — same frame as the
        // click, before the write runs. The page's category mirror gives
        // the DB's own label for a child: its name, not the menu's
        // "Parent › Child" (3.64); an id the mirror doesn't know (a parent,
        // or a category added since the last load) paints nothing and just
        // waits for the write's reload. The saved row is the rollback.
        let label = self
            .categories
            .iter()
            .find_map(|p| p.children.iter().find(|c| c.id == category_id))
            .map(|c| c.name.clone());
        let undo = self.table.update(cx, |t, cx| {
            let mut undo = RowUndo::Edited(None);
            if let Some(row) = t
                .delegate_mut()
                .rows
                .iter_mut()
                .find(|r| r.id == transaction_id)
            {
                undo = RowUndo::Edited(Some(row.clone()));
                if let Some(label) = &label {
                    row.category_id = Some(category_id);
                    row.category = Some(label.clone());
                    row.category_locked = true;
                }
            }
            t.refresh(cx);
            undo
        });
        let db = self.db.clone();
        // 3.64: record the choice now — the continuation compares against
        // it, and the table's rows lag a whole refresh behind.
        self.last_category_choice = Some((transaction_id, category_id));
        // 3.64: detached — a later action must not cancel the write's
        // continuation and swallow its refresh.
        cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| set_category(c, transaction_id, category_id)).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        // The row itself starts offering the rule (2.6); no
                        // separate feedback. Don't open the offer over a
                        // newer choice made on this row while the write was
                        // in flight (3.64): only a *different* later choice
                        // on the same row supersedes this one.
                        let superseded = this
                            .last_category_choice
                            .is_some_and(|(id, cat)| id == transaction_id && cat != category_id);
                        if !superseded {
                            this.rule_offer = Some((transaction_id, category_id));
                        }
                    }
                    Err(e) => {
                        restore_row(cx, &this.table, &undo);
                        this.notify_err(e, cx);
                    }
                }
                // 3.82: the write's report goes out as a `PageEvent`; the
                // shell reloads the shared sections, the `DataChanged`
                // comes back, and this page reloads its rows with it.
                cx.emit(PageEvent::Changed(Domains::TX));
            });
        })
        .detach();
    }

    /// Hide or unhide a transaction from the row menu (2.9). Hidden rows leave
    /// the table, the sidebar counts and the To-categorize queue at once.
    /// 3.84: the table answers in the click's frame — the row flips its
    /// hidden flag in place where the filter still shows it, and leaves the
    /// table where it doesn't (the row and its slot kept for a rollback);
    /// a failed write puts it back.
    pub(in crate::ui) fn set_row_hidden(
        &mut self,
        transaction_id: i64,
        hidden: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Hiding the row a rule offer is on would strand the offer.
        if self.rule_offer.is_some_and(|(id, _)| id == transaction_id) {
            self.rule_offer = None;
        }
        // 3.84: the paint. With *Show hidden* on, a hidden row stays in the
        // table greyed (its own `hidden` flag renders it), so the flip is
        // in place; with it off, a hide takes the row out of the table now,
        // slot and all — the rollback puts it back at its index.
        let show_hidden = self.filter.show_hidden;
        let undo = self.table.update(cx, |t, cx| {
            let rows = &mut t.delegate_mut().rows;
            let undo = match rows.iter().position(|r| r.id == transaction_id) {
                Some(index) if hidden && !show_hidden => {
                    let row = rows.remove(index);
                    RowUndo::Removed(index, row)
                }
                Some(index) => {
                    let saved = rows[index].clone();
                    rows[index].hidden = hidden;
                    RowUndo::Edited(Some(saved))
                }
                None => RowUndo::Edited(None),
            };
            t.refresh(cx);
            undo
        });
        let db = self.db.clone();
        // 3.64: detached — see `change_row_category`.
        cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| set_hidden(c, transaction_id, hidden)).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = result {
                    restore_row(cx, &this.table, &undo);
                    this.notify_err(e, cx);
                }
                cx.emit(PageEvent::Changed(Domains::TX));
            });
        })
        .detach();
    }

    /// Accept the post-edit offer from the row (2.6, 2.9): turn the merchant of
    /// the row just edited into a substring rule for the category chosen,
    /// categorizing every matching row, then clear the offer.
    pub(in crate::ui) fn apply_row_rule(
        &mut self,
        transaction_id: i64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some((_, category_id)) = self.rule_offer.filter(|(id, _)| *id == transaction_id) else {
            return;
        };
        self.rule_offer = None;
        let db = self.db.clone();
        // 3.64: detached — see `change_row_category`.
        cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| {
                apply_rule_from_merchant(c, transaction_id, category_id)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.notify_ok("Applied the category to all matching rows", cx);
                    }
                    Err(e) => this.notify_err(e, cx),
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    // -- render (3.82) --------------------------------------------------------
}
impl DashboardPage {
    /// "Clear filters": drop every filter that narrows the table — search,
    /// "Uncategorized only", and the category/month selection with its
    /// chart drill-down. `set_value` doesn't emit `InputEvent::Change`, so
    /// `self.filter.search` is cleared here too before the single re-query.
    pub(in crate::ui) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A keystroke within the last 200 ms left a debounced search pending
        // that would re-apply the dead query right after this reset, so drop
        // it with the text.
        self.search_task = None;
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.filter.search.clear();
        self.filter.uncategorized_only = false;
        self.filter.chart_parent = None;
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        // 3.65: the cleared box, legend highlight and summary-cell highlight
        // repaint with this click, not with the answer.
        cx.notify();
        self.reload(cx);
    }
}
impl DashboardPage {
    // -- shell forwards (3.82) ------------------------------------------------
    //
    // The file actions live on the Imports page and transfer pairing on the
    // Transfers page, like the other extracted pages' forwards; the shared
    // rule rows and the *Show rule* reveal live on the Rules page. The page
    // reaches them through the shell handle — the same routes the sidebar
    // and the global action listeners use, so the behaviour is identical.

    pub(in crate::ui) fn restore_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shell = self.shell.clone();
        if let Some(shell) = shell.upgrade() {
            shell.update(cx, |this, cx| this.restore_backup(window, cx));
        }
    }

    pub(in crate::ui) fn unpair_row(
        &mut self,
        transaction_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = self.shell.clone();
        if let Some(shell) = shell.upgrade() {
            shell.update(cx, |this, cx| this.unpair_row(transaction_id, window, cx));
        }
    }

    pub(in crate::ui) fn show_rule(
        &mut self,
        rule_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rules_page
            .update(cx, |p, cx| p.show_rule(rule_id, window, cx));
    }

    /// A restore replaced the whole database behind the UI (5.3): the
    /// legend, month and drill-down selections name categories and months
    /// the restored file may not have, and the rule offer names rows whose
    /// dense ids may point elsewhere now. `Shell::reset_pages_after_restore`
    /// forwards here (3.82).
    pub(in crate::ui) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
        self.filter.legend_selection = None;
        self.filter.summary_month = None;
        self.filter.chart_parent = None;
        self.rule_offer = None;
        self.last_category_choice = None;
        cx.notify();
    }
}
