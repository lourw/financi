//! Optimistic writes from the Categories page: add parent and child,
//! rename, move, the armed delete and its note, and the tree list's row
//! type.

use super::*;
impl CategoriesPage {
    pub(in crate::ui) fn add_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let name = self.new_parent_name.read(cx).value().trim().to_string();
        let kind = self
            .new_parent_kind
            .read(cx)
            .selected_index(cx)
            .map(|ix| KIND_LABELS[ix.row].1)
            .unwrap_or(CategoryKind::Expense);
        if name.is_empty() {
            self.cat_error = Some("Give the category a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        let submitted = name.clone();
        // 3.84: the Add button shows loading, disabled, until the DB is
        // done.
        self.cat_pending = Some(PendingCatWrite::AddParent);
        // 3.64: the selection this add may claim — once the user selects
        // another row, that click owns the selection from then on.
        let prior_selection = self.selected_category;
        // 3.64: detached — a later action must not cancel this write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_parent(c, &name, kind)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.cat_pending == Some(PendingCatWrite::AddParent) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(id) => {
                        // 3.64: the form still shows what was submitted —
                        // don't wipe a newer draft typed while the write ran.
                        let unchanged = this.new_parent_name.read(cx).value().trim() == submitted;
                        if unchanged {
                            this.new_parent_name
                                .update(cx, |i, cx| i.set_value("", window, cx));
                            // 3.23: adding closes the New category form.
                            this.new_category_open = false;
                        }
                        // 3.64: and the new row takes the selection only
                        // while nothing newer claimed it. Through
                        // `select_category`, so the selection's transient UI
                        // (the rename editor, the delete confirm, the rule
                        // editor) closes with the move instead of staying
                        // open re-attached to the new category.
                        if unchanged && this.selected_category == prior_selection {
                            this.select_category(id, window, cx);
                        }
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// Add a child under the selected parent (3.96: the parent keeps the
    /// selection — the new child never takes it).
    pub(in crate::ui) fn add_child(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // The action resolves against the tree (3.81): catch the mirror up
        // first, as `apply_if_dirty` documents for paths that can fire
        // while the page is not shown.
        self.apply_if_dirty(cx);
        let Some((parent, None)) = self.selected_node() else {
            return;
        };
        let parent_id = parent.id;
        let name = self.new_child_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.cat_error = Some("Give the subcategory a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        let submitted = name.clone();
        // 3.84: the Add subcategory button shows loading, disabled,
        // until the DB is done.
        self.cat_pending = Some(PendingCatWrite::AddChild);
        // 3.64 / 3.96: the selection the open form belongs to — a success
        // must not disturb any selection made while the write ran.
        let prior_selection = self.selected_category;
        // 3.51: submitted from the Add subcategory form? Then a success
        // keeps the form open and focused, parent still selected, so
        // several subcategories can be added in a row (2026-11-15).
        let from_form = self.detail_form == Some(DetailForm::AddSubcategory);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| create_child(c, parent_id, &name)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.cat_pending == Some(PendingCatWrite::AddChild) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(_id) => {
                        // 3.96: the new child never takes the selection —
                        // `select_category(id)` used to run here and moved
                        // the detail panel onto the new row, so the user
                        // had to click back to the parent for the next
                        // name (2026-11-15). The parent stays selected, and
                        // the child appears in the tree and in the parent's
                        // rule-target picker with the refresh below.
                        // 3.64: don't wipe a newer draft (see `add_parent`).
                        let unchanged = this.new_child_name.read(cx).value().trim() == submitted;
                        if unchanged {
                            this.new_child_name
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        }
                        if from_form {
                            // 3.51: the form keeps the selection it was
                            // opened on; a newer selection is not stolen
                            // back and the closed form is not reopened.
                            if unchanged && this.selected_category == prior_selection {
                                this.detail_form = Some(DetailForm::AddSubcategory);
                                // Focus stays in the cleared input, once
                                // the frame has rebuilt it (the 3.48 rule
                                // editor's pattern).
                                let handle = this.new_child_name.focus_handle(cx);
                                window.defer(cx, move |window, cx| window.focus(&handle, cx));
                            }
                        }
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        let Some((node, _)) = self.selected_node() else {
            return;
        };
        let name = node.name.clone();
        self.renaming = true;
        // 3.51: the rename editor replaces the title row, so whatever
        // inline form was open closes with it.
        self.detail_form = None;
        self.rename_input
            .update(cx, |i, cx| i.set_value(name, window, cx));
        cx.notify();
    }

    pub(in crate::ui) fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some(id) = self.selected_category else {
            return;
        };
        let name = self.rename_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.cat_error = Some("Give the category a name.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.84: the Save button shows loading, disabled, until the DB is
        // done.
        self.cat_pending = Some(PendingCatWrite::Rename);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| rename_category(c, id, &name)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.cat_pending == Some(PendingCatWrite::Rename) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(()) => {
                        // 3.64: the rename still belongs to this row's
                        // session — leave a newer selection's UI alone.
                        if this.selected_category == Some(id) {
                            this.renaming = false;
                        }
                        this.cat_error = None;
                    }
                    Err(e) => this.cat_error = Some(e),
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    pub(in crate::ui) fn move_child_to_selected_parent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        let Some((node, Some(_))) = self.selected_node() else {
            return;
        };
        let child_id = node.id;
        let Some(ix) = self.move_target.read(cx).selected_index(cx) else {
            self.cat_error = Some("Choose the category to move it under.".into());
            cx.notify();
            return;
        };
        let Some((new_parent_id, _)) = self.move_parent_choices().get(ix.row).cloned() else {
            return;
        };
        let db = self.db.clone();
        // 3.84: the Move button shows loading, disabled, until the DB is
        // done.
        self.cat_pending = Some(PendingCatWrite::Move);
        // 3.54: the node re-parents in the tree at once; a failed write
        // restores the snapshot below and the refresh reconciles either way.
        let snapshot = self.tree.clone();
        self.reparent_optimistically(child_id, new_parent_id);
        // 3.51: a successful move closes the Move form it was submitted
        // from (a successful action closes its form).
        let from_form = self.detail_form == Some(DetailForm::Move);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_child(c, child_id, new_parent_id)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.cat_pending == Some(PendingCatWrite::Move) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(()) => {
                        if from_form && this.detail_form == Some(DetailForm::Move) {
                            this.detail_form = None;
                        }
                        this.cat_error = None;
                    }
                    Err(e) => {
                        // 3.54: roll the optimistic tree edit back.
                        this.tree = snapshot;
                        this.cat_error = Some(e);
                    }
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// First click arms the delete, the second runs it (ui-conventions,
    /// *Interaction*). 3.59: the reassign target is the Delete picker's
    /// item value — the grouped `SubChoice` carries the sub-category id,
    /// so the shell's positional reassign list is no longer needed here
    /// (the 3.81 forwarder constraint it worked around is moot).
    pub(in crate::ui) fn arm_delete_category(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        // Resolve against a current mirror (see `add_child`).
        self.apply_if_dirty(cx);
        // 3.54: the reassign pick resolves against current items (see the
        // method).
        self.refresh_cat_pickers(window, cx);
        let Some(id) = self.selected_category else {
            return;
        };
        if self.confirm_delete_category != Some(id) {
            self.confirm_delete_category = Some(id);
            cx.notify();
            return;
        }
        // 3.59: the grouped items carry the sub-category id as their
        // value, so the pick resolves directly.
        let Some(reassign_to) = self.delete_target.read(cx).selected_value().copied() else {
            self.cat_error = Some("Choose where its transactions should go.".into());
            cx.notify();
            return;
        };
        self.confirm_delete_category = None;
        // 3.51: the second click ran the delete, so its form goes too.
        self.detail_form = None;
        let db = self.db.clone();
        // 3.84: the Delete button shows loading, disabled, until the DB
        // is done.
        self.cat_pending = Some(PendingCatWrite::Delete);
        // 3.54: the node leaves the tree at once, its counts moving to the
        // reassign target; a failed write restores the snapshot below and
        // the later refresh reconciles either way.
        let snapshot = (self.tree.clone(), self.counts.clone(), self.rules.clone());
        self.remove_node_optimistically(id, reassign_to);
        // 3.64: detached — see `add_parent`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| delete_category(c, id, reassign_to)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.cat_pending == Some(PendingCatWrite::Delete) {
                    this.cat_pending = None;
                }
                match result {
                    Ok(()) => {
                        // 3.64: the deleted row is gone; only drop the
                        // selection if it still points at it — and the
                        // shell's mirror with it (3.81).
                        if this.selected_category == Some(id) {
                            this.selected_category = None;
                            if let Some(shell) = this.shell.upgrade() {
                                shell.update(cx, |shell, _| {
                                    if shell.categories_page.selected_category == Some(id) {
                                        shell.categories_page.selected_category = None;
                                    }
                                });
                            }
                        }
                        this.cat_error = None;
                    }
                    Err(e) => {
                        // 3.54: roll the optimistic tree edit back.
                        let (tree, counts, rules) = snapshot;
                        this.tree = tree;
                        this.counts = counts;
                        this.rules = rules;
                        this.cat_error = Some(e);
                    }
                }
                cx.emit(PageEvent::Changed(Domains::CATEGORIES_RULES_TX));
            });
        })
        .detach();
        cx.notify();
    }

    /// 3.91: record one tree paint pass in the counters the tests read.
    /// H4: `DataList::on_pass` hands over the exact range — like the
    /// Queue page's `note_paint_pass`, minus the sequence heuristic that
    /// could not see an exact-page scroll (ids continuing with no break).
    /// Unlike the Queue page's window nothing here drives a prune: the
    /// counters are the tests' own.
    pub(super) fn note_tree_paint_pass(&self, _range: std::ops::Range<usize>) {
        #[cfg(test)]
        {
            self.tree_rows_built.set(_range.len());
            self.tree_range.set((_range.start, _range.end));
        }
    }
}
/// 3.91: one line of the category tree as the shared list sees it — the
/// fields the cells callback needs, so it never borrows the tree itself.
pub(super) struct TreeListRow {
    pub(super) id: i64,
    pub(super) name: SharedString,
    pub(super) depth: u8,
    pub(super) is_system: bool,
    pub(super) count: i64,
}
