//! The add-rule row and its in-place editor (3.24, 3.28): the live
//! pattern preview and the add, save and delete handlers of the Rules page
//! entity (S1 split from `ui/rules.rs`).

use super::*;

impl RulesPage {
    /// The draft the preview runs against: the editor's pattern and kind
    /// while an editor is open, the add row's otherwise.
    fn preview_draft(&self, editing: Option<i64>, cx: &App) -> (String, bool) {
        match editing {
            Some(_) => (
                self.rule_edit_pattern.read(cx).value().trim().to_string(),
                self.rule_edit_regex,
            ),
            None => (
                self.rule_pattern.read(cx).value().trim().to_string(),
                self.rule_regex,
            ),
        }
    }

    /// Show what the current pattern would categorize, live as the user
    /// types (ui-conventions, Categories page). 3.103: the read is debounced
    /// by 150 ms — each keystroke replaces the pending timer, so a burst of
    /// typing previews once after the last key (the 3.58 search box's
    /// pattern); a toggle or an editor open runs [`Self::run_pattern_preview`
    /// right away instead.
    pub(in crate::ui) fn preview_rule_match(&mut self, cx: &mut Context<Self>) {
        let editing = self.editing_rule;
        let (pattern, _) = self.preview_draft(editing, cx);
        if pattern.is_empty() {
            self.pattern_preview_task = None;
            self.pattern_preview = None;
            self.pattern_preview_error = None;
            cx.notify();
            return;
        }
        self.pattern_preview_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.pattern_preview_task = None;
                this.run_pattern_preview(cx);
            });
        }));
    }

    /// Run the preview now: validate the draft, then queue a latest-wins
    /// read (3.77) of [`preview_pattern_matches`] — a newer keystroke
    /// supersedes a running read, and a result whose draft isn't the
    /// current one is dropped, so the preview never shows a pattern the
    /// user has already retyped. 3.24: while an editor is open the draft is
    /// the editor's and that rule is excluded, so the rows are what the
    /// edit would newly categorize.
    pub(in crate::ui) fn run_pattern_preview(&mut self, cx: &mut Context<Self>) {
        let editing = self.editing_rule;
        let (pattern, is_regex) = self.preview_draft(editing, cx);
        if pattern.is_empty() {
            self.pattern_preview_task = None;
            self.pattern_preview = None;
            self.pattern_preview_error = None;
            cx.notify();
            return;
        }
        // `preview_pattern_matches` reads an invalid regex as
        // match-nothing; the user needs the compiler's words instead —
        // the same check `new_rule` applies, worded the same way.
        if is_regex && let Err(message) = financi_core::rules::validate_regex(&pattern) {
            self.pattern_preview_task = None;
            self.pattern_preview = None;
            self.pattern_preview_error = Some(
                DbError::InvalidRulePattern {
                    pattern: pattern.clone(),
                    message,
                }
                .to_string()
                .into(),
            );
            cx.notify();
            return;
        }
        self.pattern_preview_error = None;
        let db = self.db.clone();
        let requested = pattern.clone();
        self.pattern_preview_task = Some(cx.spawn(async move |this, cx| {
            let result = db
                .call_keyed("rules:pattern-preview", move |conn| {
                    preview_pattern_matches(conn, &requested, is_regex, editing, PREVIEW_ROWS)
                })
                .await;
            // A superseded read, or any failure, keeps what's on screen;
            // the next keystroke tries again.
            if let Ok(preview) = result {
                let _ = this.update(cx, |this, cx| {
                    let (current, _) = this.preview_draft(editing, cx);
                    if this.editing_rule == editing && current == pattern {
                        this.pattern_preview = Some(Rc::new(preview));
                        cx.notify();
                    }
                });
            }
        }));
    }

    /// 3.103: the height the preview adds under the open editor — the
    /// virtualized rule lists on both pages must reserve it (gpui doesn't
    /// clip per item), and it changes when the preview arrives, so the page
    /// notify rebuilds `sizes`. Zero while the editor is closed or the
    /// first read is still in flight: the row grows when the rows do.
    pub(in crate::ui) fn editor_preview_extra_h(&self) -> f32 {
        if self.editing_rule.is_none() {
            return 0.;
        }
        if self.pattern_preview_error.is_some() {
            return PREVIEW_LINE_H + 4.;
        }
        match self.pattern_preview.as_deref() {
            Some(preview) => {
                let list = if preview.rows.is_empty() {
                    PREVIEW_LINE_H
                } else {
                    preview.rows.len() as f32 * PREVIEW_ROW_H
                };
                PREVIEW_LINE_H + 4. + list
            }
            None => 0.,
        }
    }

    /// 3.40: is `action` the rule button currently waiting on the DB? The
    /// matching button renders `.loading(true)` and disabled while it is,
    /// so a slow save can't be double-submitted.
    pub(in crate::ui) fn rule_action_pending(&self, action: PendingRuleAction) -> bool {
        self.pending_rule_action == Some(action)
    }

    /// Feedback goes to the shell's toast stack (3.79: a page never owns
    /// window chrome).
    fn toast(&mut self, message: &str, error: bool, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast {
            message: message.to_string(),
            error,
        });
    }

    /// 3.64: show a rule write's failure where it can be read. `rule_error`
    /// is the one line under the add row *and* under the open editor, so it
    /// belongs to whichever of the two is showing: with an editor open the
    /// failure goes out as a toast instead of painting under another rule's
    /// fields (and the add row hides its own line then anyway).
    fn report_rule_error(&mut self, error: String, cx: &mut Context<Self>) {
        match self.editing_rule {
            None => self.rule_error = Some(error),
            Some(_) => self.toast(&error, true, cx),
        }
    }

    /// 3.54: re-set the two shared category pickers when `apply` flagged a
    /// newer grouped-items `Rc` — `set_items` needs a window, so this runs
    /// from `render` (where a shown page picks the change up) and from the
    /// actions that read the pickers (the harness and the Categories-driven
    /// add flow work pages that never rendered). Fresh pickers: a pointer
    /// compare, no work (the queue page's 3.19/3.83 pattern).
    pub(super) fn refresh_cat_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cat_pickers_need_items {
            return;
        }
        self.cat_pickers_need_items = false;
        if let Some(items) = self.cat_choices_held.clone() {
            let items = SearchableVec::new((*items).clone());
            self.rule_target
                .update(cx, |s, cx| s.set_items(items.clone(), window, cx));
            self.move_rules_target
                .update(cx, |s, cx| s.set_items(items, window, cx));
            #[cfg(test)]
            self.cat_picker_sets.set(self.cat_picker_sets.get() + 2);
        }
    }

    pub(in crate::ui) fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 3.54: the pick resolves against current items (see the method).
        self.refresh_cat_pickers(window, cx);
        // 3.59: the grouped items carry the sub-category id as their
        // value, so the pick resolves directly.
        let Some(category_id) = self.rule_target.read(cx).selected_value().copied() else {
            self.rule_error = Some("Choose the category the rule assigns.".into());
            cx.notify();
            return;
        };
        self.commit_new_rule(category_id, window, cx);
    }

    /// The shared tail of both add-rule paths: validate the pattern, run
    /// [`financi_db::rules::add_rule`], clear the row and refresh (3.40:
    /// the button shows loading, disabled, while the DB is busy — a save
    /// that re-runs the rules can take seconds).
    pub(super) fn commit_new_rule(
        &mut self,
        category_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let pattern = self.rule_pattern.read(cx).value().trim().to_string();
        let is_regex = self.rule_regex;
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.40: the Add button shows loading, disabled, until the DB is
        // done (a save that re-runs the rules can take seconds).
        self.pending_rule_action = Some(PendingRuleAction::Add);
        let submitted = pattern.clone();
        // 3.47: the new row appears in the list at once with a temporary
        // negative id; the Ok arm swaps in the real id, the Err arm drops
        // the row again (the later refresh reconciles either way).
        let temp_id = self.next_temp_rule_id;
        self.next_temp_rule_id -= 1;
        self.rules.push(Rule {
            id: temp_id,
            pattern: pattern.clone(),
            is_regex,
            category_id,
        });
        self.rules_gen += 1;
        // 3.64: detached — a later action must not cancel this write's
        // continuation and swallow its refresh.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| add_rule(c, &pattern, is_regex, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // 3.64: only clear the pending marker if it still marks
                // *this* add; a newer action owns the slot now.
                let own = this.pending_rule_action == Some(PendingRuleAction::Add);
                if own {
                    this.pending_rule_action = None;
                }
                match result {
                    Ok(new_id) => {
                        // 3.47: the provisional row carries its real id.
                        if let Some(rule) = this.rules.iter_mut().find(|r| r.id == temp_id) {
                            rule.id = new_id;
                            this.rules_gen += 1;
                        }
                        // The input still shows what was submitted — don't
                        // wipe a pattern typed after this one landed.
                        if this.rule_pattern.read(cx).value().trim() == submitted {
                            this.rule_pattern
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        }
                        // 3.64: this add clears the shared error line only
                        // while it still owns it — a stale add must not wipe
                        // a genuine error another action just reported.
                        if own && this.editing_rule.is_none() {
                            this.rule_error = None;
                        }
                    }
                    // 3.47: a failed add takes the provisional row back
                    // out, whether or not this add still owns the badge.
                    // 3.64: and only this add reports the error: a failure
                    // that lands after the user moved on (another action
                    // owns the badge, or an editor is open) must not paint
                    // under someone else's fields — `report_rule_error`
                    // toasts it while one is open, and a stale add's
                    // failure is the newer action's to report.
                    Err(e) => {
                        if let Some(ix) = this.rules.iter().position(|r| r.id == temp_id) {
                            this.rules.remove(ix);
                            this.rules_gen += 1;
                        }
                        if own {
                            this.report_rule_error(e, cx);
                        }
                    }
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 3.24: close the rule editor and drop everything it owns — the
    /// error line and the live preview, including its in-flight task
    /// (dropping a `Task` cancels it, so a late result can't repopulate
    /// the preview after the editor is gone). Returns whether an editor was
    /// open. Every editor-close path goes through here.
    pub(in crate::ui) fn close_rule_editor(&mut self) -> bool {
        if self.editing_rule.take().is_some() {
            self.rule_error = None;
            self.pattern_preview_task = None;
            self.pattern_preview = None;
            self.pattern_preview_error = None;
            true
        } else {
            false
        }
    }

    /// 3.24: close the rule editor without saving. The editor's controls
    /// keep their values (invisible while closed); `start_edit_rule`
    /// re-fills them from the next rule. 3.103: the add row's preview,
    /// which the editor had taken over, runs again right away.
    pub(in crate::ui) fn cancel_edit_rule(&mut self, cx: &mut Context<Self>) {
        if self.close_rule_editor() {
            self.run_pattern_preview(cx);
            cx.notify();
        }
    }

    /// 3.24: swap a rule's row for its editor, pre-filled from the rule —
    /// the two-line form 3.103 gave the add-rule row (Pattern over the
    /// input, then the regex switch with Save and Cancel), and like it no
    /// Category picker (3.53: Edit changes pattern and kind only), with
    /// [`pattern_preview_block`] under it. One rule is edited at a time,
    /// so opening a second row's editor closes the first and disarms any
    /// pending delete.
    pub(in crate::ui) fn start_edit_rule(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rule) = self.rules.iter().find(|r| r.id == id).cloned() else {
            return;
        };
        self.editing_rule = Some(id);
        self.armed_delete_rule = None;
        self.rule_error = None;
        self.rule_edit_regex = rule.is_regex;
        self.rule_edit_pattern.update(cx, |i, cx| {
            i.set_value(rule.pattern.clone(), window, cx);
        });
        // 3.53: no category picker — Edit changes pattern and kind only.
        self.run_pattern_preview(cx);
        cx.notify();
        // 3.48: focus lands in the pattern input with the caret at the
        // end (`set_value` places a single-line caret at the end of the
        // text); the focus waits for the end of the frame so the editor's
        // `Input` element exists first, like the page's search focus.
        let handle = self.rule_edit_pattern.focus_handle(cx);
        window.defer(cx, move |window, cx| window.focus(&handle, cx));
    }

    /// 3.24: save the editor through [`update_rule`], which re-runs every
    /// rule over the unlocked rows (2.5). A validation error (empty
    /// pattern, bad regex) lands in `rule_error` — shown in `danger` under
    /// the editor — and the editor stays open. On success every view
    /// refreshes, as after `add_rule`.
    pub(in crate::ui) fn save_rule_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some(id) = self.editing_rule else {
            return;
        };
        let pattern = self.rule_edit_pattern.read(cx).value().trim().to_string();
        let is_regex = self.rule_edit_regex;
        // 3.53: the editor has no picker, so the rule keeps its category;
        // `update_rule` is still handed the rule's own `category_id`.
        let Some(category_id) = self
            .rules
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.category_id)
        else {
            return;
        };
        if pattern.is_empty() {
            self.rule_error = Some("Give the rule a pattern.".into());
            cx.notify();
            return;
        }
        let db = self.db.clone();
        // 3.40: the Save button shows loading, disabled, until the DB is
        // done; on error the editor stays open and pending clears too.
        self.pending_rule_action = Some(PendingRuleAction::Save(id));
        // 3.47: the row shows the new pattern at once; the Err arm puts
        // the saved copy back (the refresh reconciles on success).
        let previous = self
            .rules
            .iter()
            .position(|r| r.id == id)
            .map(|ix| self.rules[ix].clone());
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == id) {
            rule.pattern = pattern.clone();
            rule.is_regex = is_regex;
            self.rules_gen += 1;
        }
        // 3.64: detached — see `commit_new_rule`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                update_rule(c, id, &pattern, is_regex, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.pending_rule_action == Some(PendingRuleAction::Save(id)) {
                    this.pending_rule_action = None;
                }
                match result {
                    Ok(()) => {
                        // The editor may have moved to another rule (or
                        // closed) while this save ran: only the editor on
                        // *this* rule gets its result.
                        if this.editing_rule == Some(id) {
                            this.close_rule_editor();
                        }
                    }
                    Err(e) => {
                        // 3.47: roll the optimistic edit back.
                        if let Some(prev) = previous
                            && let Some(rule) = this.rules.iter_mut().find(|r| r.id == id)
                        {
                            *rule = prev;
                            this.rules_gen += 1;
                        }
                        if this.editing_rule == Some(id) {
                            this.rule_error = Some(e);
                        }
                    }
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 3.24: Delete with the two-click confirm (ui-conventions,
    /// *Interaction*): the first click only arms the button ("Click again
    /// to delete"), the second deletes through [`delete_rule`] — whose
    /// re-run leaves the freed rows to the remaining rules or
    /// Uncategorized — and refreshes every view.
    pub(in crate::ui) fn arm_delete_rule(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        if self.armed_delete_rule == Some(id) {
            let db = self.db.clone();
            // 3.40: the delete runs (and the button shows loading,
            // disabled) while the rules re-run over the unlocked rows.
            self.pending_rule_action = Some(PendingRuleAction::Delete(id));
            // 3.47: the row leaves the list at once; a failed delete puts
            // it back at its position and reports the error (3.24's
            // "rule removed underneath" case).
            let removed = self
                .rules
                .iter()
                .position(|r| r.id == id)
                .map(|ix| (ix, self.rules.remove(ix)));
            if removed.is_some() {
                self.rules_gen += 1;
            }
            // 3.64: detached — see `commit_new_rule`.
            cx.spawn_in(window, async move |this, cx| {
                let result = on_db(&db, move |c| delete_rule(c, id)).await;
                let _ = this.update_in(cx, |this, _window, cx| {
                    if this.pending_rule_action == Some(PendingRuleAction::Delete(id)) {
                        this.pending_rule_action = None;
                    }
                    match result {
                        Ok(()) => {
                            if this.armed_delete_rule == Some(id) {
                                this.armed_delete_rule = None;
                            }
                            if this.editing_rule == Some(id) {
                                this.close_rule_editor();
                            }
                        }
                        // 3.64: the failure of *this* delete never paints
                        // under an editor that belongs to another rule —
                        // `report_rule_error` toasts it while one is open.
                        Err(e) => {
                            // 3.47: restore the row unless a refresh has
                            // already brought it back (it can't be gone
                            // twice — but neither may it be listed twice).
                            if let Some((ix, rule)) = removed
                                && !this.rules.iter().any(|r| r.id == rule.id)
                            {
                                let ix = ix.min(this.rules.len());
                                this.rules.insert(ix, rule);
                                this.rules_gen += 1;
                            }
                            this.report_rule_error(e, cx)
                        }
                    }
                    cx.emit(PageEvent::Changed(Domains::RULES_TX));
                    cx.notify();
                });
            })
            .detach();
        } else {
            self.armed_delete_rule = Some(id);
        }
        cx.notify();
    }
}
