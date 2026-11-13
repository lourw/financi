//! The Rules page (3.62).

use super::*;
use crate::ui::widgets::*;

/// 3.28: the Rules page's header action; it opens the add-rule row in
/// place under the header (ui-conventions, *Pages*).
pub(super) const NEW_RULE: &str = "New rule…";
/// 3.28: the Move bar's target picker (moving checked rules between
/// categories).
const MOVE_RULES_TO_PH: &str = "Move rules to…";

/// 3.23: the Categories rule list's fixed columns — the row and its column
/// headings share these widths so the headings sit over their columns
/// (same rule as the queue's `QUEUE_*_W`). Pattern takes the remaining
/// width (`flex_1`) and truncates with a tooltip.
pub(super) const RULE_TYPE_W: f32 = 80.;
pub(super) const RULE_CATEGORY_W: f32 = 200.;
/// 3.53: the parent-scoped add-rule picker's open-menu ceiling — wide
/// enough for a long subcategory name where the trigger has to ellipsize
/// (a 40-character name fits, per the menu test).
pub(super) const RULE_CATEGORY_MENU_W: f32 = 320.;
/// 3.28: the Rules page list's fixed columns beside the flexed Pattern: a
/// checkbox column, and Uses (right-aligned). Same sharing rule as
/// [`RULE_TYPE_W`]: row and column headings use these.
const RULE_SEL_W: f32 = 24.;
const RULE_USES_W: f32 = 80.;

/// 3.28: one entry of the Rules page virtual list — a group header row or a
/// rule row. Group headers are items too, so collapsing a group just drops
/// its rows and the list stays virtualized over everything (D5).
enum RulesItem {
    Group {
        /// Child-category id; also the collapse key.
        id: i64,
        /// "Parent › Child", from `child_choices`.
        label: String,
        /// Rules shown in this group (after the page filters).
        count: usize,
        collapsed: bool,
    },
    Rule {
        id: i64,
    },
}

/// 3.40: the rule-editor button currently waiting on the DB (Save on a
/// rule edit, Add rule, or a row's Delete…). While one is pending that
/// button renders `.loading(true)` and disabled, so a slow save can't be
/// double-submitted; the task clears it when it finishes, on success or
/// error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PendingRuleAction {
    Add,
    Save(i64),
    Delete(i64),
}

/// This page's state (3.74): the Rules page's search, filters, selection
/// and list state. The add-rule row and its in-place editor stay on
/// `Shell` (shared with the Categories page until 3.78).
pub(super) struct RulesPageState {
    /// 3.28: the Rules page search box; live filter over the list (pattern
    /// and target category name, case-insensitive), focused when the page
    /// opens (D5). In "Test a description" mode it takes the description.
    pub(super) rules_search: Entity<InputState>,
    /// 3.28: the "Test a description" switch (D5): the list shows only the
    /// rules matching the text in the search box, winner first.
    pub(super) rules_test_mode: bool,
    /// 3.28: Type picker over the list (row 0 = All, 1 = Substring, 2 = Regex;
    /// nothing selected reads as All).
    pub(super) rules_type_select: Entity<SelectState<Names>>,
    /// 3.28: the "Unused" checkbox: only show rules deciding zero
    /// transactions.
    pub(super) rules_unused_only: bool,
    /// 3.28: child-category ids whose group header row is collapsed.
    pub(super) collapsed_rule_groups: HashSet<i64>,
    /// 3.28: sort rows within a group by Uses (descending) instead of by
    /// Pattern (the default, ascending).
    pub(super) rules_sort_by_uses: bool,
    /// 3.28: rules whose row checkbox is ticked; the Move bar acts on these.
    pub(super) selected_rules: HashSet<i64>,
    /// 3.28: the Move bar's target picker (child categories only, like the
    /// rule target pickers).
    pub(super) move_rules_target: Entity<SelectState<Names>>,
    /// 3.28: the Move bar's error line (a move the DB refused).
    pub(super) rules_move_error: Option<SharedString>,
    /// 3.28: how many unlocked rows each rule's match decides (the Uses
    /// column); rules deciding nothing are absent.
    pub(super) rule_uses: HashMap<i64, usize>,
    /// 3.28: whether the add-rule row is open on the Rules page.
    pub(super) new_rule_open: bool,
    /// 3.28: set when the Rules page opens; focuses the search box on the
    /// next render (D5).
    pub(super) rules_focus_pending: std::cell::Cell<bool>,
    /// 3.29: the rule the dashboard's *Show rule* item just landed on; its
    /// row carries the `list_active` background until the user searches or
    /// leaves the page (transient, like the page's other one-shot state).
    pub(super) rules_reveal: Option<i64>,
    /// 3.28: the Rules page list's scroll handle (headers and rows together).
    pub(super) rules_scroll: VirtualListScrollHandle,
    /// 3.28: how many items the Rules page list built last frame (headers
    /// and rows together) and the range it drew, for the long-list tests.
    #[cfg(test)]
    pub(super) rules_page_rows_built: std::cell::Cell<usize>,
    #[cfg(test)]
    pub(super) rules_page_range: std::cell::Cell<(usize, usize)>,
    /// The pickers and inputs above subscribe here.
    _subs: Vec<Subscription>,
}

impl RulesPageState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        // 3.28: the Rules page search box filters the list on every
        // keystroke, and in "Test a description" mode it is the description
        // the rules are tried against (D5). Typing replaces the list, so the
        // scroll offset goes back to the top.
        let rules_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search pattern or category…"));
        let rules_search_sub =
            cx.subscribe(&rules_search, |this: &mut Shell, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.rules_page.rules_scroll.set_offset(Point::default());
                    // 3.29: the reveal highlight is one-shot; anything typed
                    // after it is the user's own search.
                    this.rules_page.rules_reveal = None;
                    cx.notify();
                }
            });
        // 3.28: the Type and Move pickers. The Type picker filters the list
        // (nothing selected reads as All types); the Move picker holds the
        // child categories, refreshed by `sync_category_choices`.
        let rules_type_select = cx.new(|cx| {
            SelectState::new(
                vec!["All types".into(), "Substring".into(), "Regex".into()],
                None,
                window,
                cx,
            )
        });
        let rules_type_sub = cx.subscribe_in(
            &rules_type_select,
            window,
            |this: &mut Shell, _select, ev: &SelectEvent<Names>, _window, cx| {
                if matches!(ev, SelectEvent::Confirm(_)) {
                    this.rules_page.rules_scroll.set_offset(Point::default());
                    cx.notify();
                }
            },
        );
        let move_rules_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        Self {
            rules_search,
            rules_test_mode: false,
            rules_type_select,
            rules_unused_only: false,
            collapsed_rule_groups: HashSet::default(),
            rules_sort_by_uses: false,
            selected_rules: HashSet::default(),
            move_rules_target,
            rules_move_error: None,
            rule_uses: HashMap::default(),
            new_rule_open: false,
            rules_focus_pending: std::cell::Cell::new(false),
            rules_reveal: None,
            rules_scroll: VirtualListScrollHandle::new(),
            #[cfg(test)]
            rules_page_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            rules_page_range: std::cell::Cell::new((0, 0)),
            _subs: vec![rules_search_sub, rules_type_sub],
        }
    }
}

impl Shell {
    /// Apply this page's part of a refresh result (3.74): the shared rule
    /// list renders from `Shell`'s list (3.78 moves it here).
    pub(super) fn apply_rules(&mut self, rules: Vec<Rule>, uses: HashMap<i64, usize>) {
        self.rules = rules;
        self.rules_page.rule_uses = uses;
    }

    /// The shared add-rule row's and the in-place editor's subscriptions
    /// (3.24): every pattern keystroke re-runs the "matches N transactions"
    /// preview, and Enter in the editor saves. Built by `Shell::new`.
    pub(super) fn subscribe_rule_row(
        &self,
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) -> Vec<Subscription> {
        // The live "matches N transactions" preview runs on every pattern
        // keystroke (ui-conventions, Categories page).
        let preview_sub = cx.subscribe(&self.rule_pattern, |this, _, ev: &InputEvent, cx| {
            // 3.24: while a rule editor is open, the preview belongs to it.
            if matches!(ev, InputEvent::Change) && this.editing_rule.is_none() {
                this.preview_rule_match(cx);
            }
        });
        // 3.24: the rule editor's Pattern input drives the same preview and
        // saves on Enter (Escape cancels through the window-level binding
        // handled in `clear_category_selection`).
        let rule_edit_sub = cx.subscribe_in(
            &self.rule_edit_pattern,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change if this.editing_rule.is_some() => this.preview_rule_match(cx),
                InputEvent::PressEnter {
                    secondary: false, ..
                } if this.editing_rule.is_some() => this.save_rule_edit(window, cx),
                _ => {}
            },
        );
        vec![preview_sub, rule_edit_sub]
    }
}

impl Shell {
    /// Count rows the current pattern would categorize, live as the user
    /// types (ui-conventions, Categories page).
    pub(super) fn preview_rule_match(&mut self, cx: &mut Context<Self>) {
        // 3.24: while a rule is being edited, the preview reads the
        // editor's controls and excludes that rule, so the count shows what
        // the edit would newly categorize (`count_rule_matches`' contract).
        let editing = self.editing_rule;
        let pattern = match editing {
            Some(_) => self.rule_edit_pattern.read(cx).value().trim().to_string(),
            None => self.rule_pattern.read(cx).value().trim().to_string(),
        };
        if pattern.is_empty() {
            self.rule_task = None;
            self.rule_match_count = None;
            cx.notify();
            return;
        }
        let is_regex = match editing {
            Some(_) => self.rule_edit_regex,
            None => self.rule_regex,
        };
        let db = self.db.clone();
        self.rule_task = Some(cx.spawn(async move |this, cx| {
            let result = on_db(&db, move |c| {
                count_rule_matches(c, &pattern, is_regex, editing).map_err(|e| e.to_string())
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(count) = result {
                    this.rule_match_count = Some(count);
                    cx.notify();
                }
            });
        }));
    }

    /// 3.40: is `action` the rule button currently waiting on the DB? The
    /// matching button renders `.loading(true)` and disabled while it is,
    /// so a slow save can't be double-submitted.
    pub(super) fn rule_action_pending(&self, action: PendingRuleAction) -> bool {
        self.pending_rule_action == Some(action)
    }

    pub(super) fn add_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.rule_target.read(cx).selected_index(cx) else {
            self.rule_error = Some("Choose the category the rule assigns.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        self.commit_new_rule(category_id, window, cx);
    }

    /// 3.53: Add rule on the Categories page. A selected subcategory is
    /// the target — nothing to pick. A parent needs the row's picker:
    /// rules must target a child (`ensure_assignable`), so no pick is an
    /// error. No selection (nothing selected) can't render the row at all.
    pub(super) fn add_rule_to_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let category_id = match self.selected_node() {
            Some((_, None)) => {
                let Some(ix) = self
                    .categories_page
                    .cat_rule_target
                    .read(cx)
                    .selected_index(cx)
                else {
                    self.rule_error = Some("Choose the subcategory the rule assigns.".into());
                    cx.notify();
                    return;
                };
                let Some(cid) = self.categories_page.cat_rule_child_ids.get(ix.row).copied() else {
                    return;
                };
                cid
            }
            Some((node, Some(_))) => node.id,
            None => return,
        };
        self.commit_new_rule(category_id, window, cx);
    }

    /// The shared tail of both add-rule paths: validate the pattern, run
    /// [`financi_db::rules::add_rule`], clear the row and refresh (3.40:
    /// the button shows loading, disabled, while the DB is busy — a save
    /// that re-runs the rules can take seconds).
    fn commit_new_rule(&mut self, category_id: i64, window: &mut Window, cx: &mut Context<Self>) {
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
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| add_rule(c, &pattern, is_regex, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending_rule_action = None;
                match result {
                    Ok(_) => {
                        this.rule_pattern
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.rule_error = None;
                    }
                    Err(e) => this.rule_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// 3.24: close the rule editor and drop everything it owns — the
    /// error line and the live preview count, including its in-flight task
    /// (dropping a `Task` cancels it, so a late result can't repopulate
    /// the count after the editor is gone). Returns whether an editor was
    /// open. Every editor-close path goes through here.
    pub(super) fn close_rule_editor(&mut self) -> bool {
        if self.editing_rule.take().is_some() {
            self.rule_error = None;
            self.rule_task = None;
            self.rule_match_count = None;
            true
        } else {
            false
        }
    }

    /// 3.24: close the rule editor without saving. The editor's controls
    /// keep their values (invisible while closed); `start_edit_rule`
    /// re-fills them from the next rule.
    pub(super) fn cancel_edit_rule(&mut self, cx: &mut Context<Self>) {
        if self.close_rule_editor() {
            cx.notify();
        }
    }

    /// 3.24: swap a rule's row for its editor, pre-filled from the rule —
    /// same shape as the add-rule row (Pattern, regex switch, Category
    /// picker, Save, Cancel). One rule is edited at a time, so opening a
    /// second row's editor closes the first and disarms any pending delete.
    pub(super) fn start_edit_rule(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
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
        self.preview_rule_match(cx);
        cx.notify();
    }

    /// 3.24: save the editor through [`update_rule`], which re-runs every
    /// rule over the unlocked rows (2.5). A validation error (empty
    /// pattern, bad regex) lands in `rule_error` — shown in `danger` under
    /// the editor — and the editor stays open. On success every view
    /// refreshes, as after `add_rule`.
    pub(super) fn save_rule_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| {
                update_rule(c, id, &pattern, is_regex, category_id)
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending_rule_action = None;
                match result {
                    Ok(()) => {
                        this.close_rule_editor();
                    }
                    Err(e) => this.rule_error = Some(e),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// 3.24: Delete… with the two-click confirm (ui-conventions,
    /// *Interaction*): the first click only arms the button ("Click again
    /// to delete"), the second deletes through [`delete_rule`] — whose
    /// re-run leaves the freed rows to the remaining rules or
    /// Uncategorized — and refreshes every view.
    pub(super) fn arm_delete_rule(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.armed_delete_rule == Some(id) {
            let db = self.db.clone();
            // 3.40: the delete runs (and the button shows loading,
            // disabled) while the rules re-run over the unlocked rows.
            self.pending_rule_action = Some(PendingRuleAction::Delete(id));
            self.task = Some(cx.spawn_in(window, async move |this, cx| {
                let result = on_db(&db, move |c| delete_rule(c, id)).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.pending_rule_action = None;
                    match result {
                        Ok(()) => {
                            this.armed_delete_rule = None;
                            if this.editing_rule == Some(id) {
                                this.close_rule_editor();
                            }
                        }
                        Err(e) => this.rule_error = Some(e),
                    }
                    this.refresh(window, cx);
                });
            }));
        } else {
            self.armed_delete_rule = Some(id);
        }
        cx.notify();
    }

    // -- To categorize queue ------------------------------------------------

    /// One row of the selected category's rule list (3.12: built lazily
    /// by the virtualized rule list, fixed height to match its
    /// `item_sizes`). 3.24: every row carries Edit and Delete… at its right
    /// end — small ghost buttons, visible on every row, built only for the
    /// rows the list renders. 3.23: the columns line up with the list's
    /// headings (`RULE_TYPE_W`/`RULE_CATEGORY_W`); the pattern column is
    /// the flex one and truncates with a tooltip, like the queue's
    /// merchant column.
    pub(super) fn rule_row(&self, rule: &Rule, muted: Hsla, cx: &mut Context<Self>) -> AnyElement {
        let target = self
            .child_choices
            .iter()
            .find(|(id, _)| *id == rule.category_id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("#{}", rule.category_id));
        let id = rule.id;
        let armed = self.armed_delete_rule == Some(id);
        h_flex()
            .h_6()
            .w_full()
            .gap_2()
            .items_center()
            .debug_selector(move || format!("rule-row-{id}"))
            .child(
                div()
                    .id(SharedString::from(format!("rule-pattern-{id}")))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .debug_selector(move || format!("rule:pattern-col:{id}"))
                    .tooltip({
                        let full = rule.pattern.clone();
                        move |window, cx| {
                            let tip = full.clone();
                            Tooltip::element(move |_, _| {
                                div()
                                    .debug_selector({
                                        let p = tip.clone();
                                        move || format!("rule:pattern-tooltip:{p}")
                                    })
                                    .child(tip.clone())
                            })
                            .build(window, cx)
                        }
                    })
                    .child(rule.pattern.clone()),
            )
            .child(
                div()
                    .w(px(RULE_TYPE_W))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(muted)
                    .debug_selector(move || format!("rule:type-col:{id}"))
                    .child(if rule.is_regex { "regex" } else { "substring" }),
            )
            .child(
                div()
                    .w(px(RULE_CATEGORY_W))
                    .flex_shrink_0()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(muted)
                    .debug_selector(move || format!("rule:category-col:{id}"))
                    .child(target),
            )
            .child(
                Button::new(SharedString::from(format!("rule-edit-{id}")))
                    .label("Edit")
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.start_edit_rule(id, window, cx)
                        }),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("rule-delete-{id}")))
                    .label(if armed {
                        "Click again to delete"
                    } else {
                        "Delete…"
                    })
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
                    // 3.40: loading + disabled while the delete runs.
                    .loading(self.rule_action_pending(PendingRuleAction::Delete(id)))
                    .disabled(self.rule_action_pending(PendingRuleAction::Delete(id)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.arm_delete_rule(id, window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }

    /// 3.24: the in-place editor that replaces one rule's row — the
    /// add-rule row's shape (labelled Pattern input, regex switch, labelled
    /// Category picker, primary Save, Cancel) pre-filled from the rule,
    /// with the same live "matches N transactions" count. A validation
    /// error shows in `danger` under the editor and keeps it open — clamped
    /// to one ellipsized line, because the virtual list doesn't clip per
    /// item and a taller row would paint over the rows below. 3.23: inside
    /// the detail panel the row is too narrow for the count beside the
    /// fields, so it moves under them (like the add-rule row's) and the
    /// row reserves 80 px in `rule_sizes` instead of 24, 108 px while an
    /// error is open (measured: 75.5 px with the count, 102.5 with one
    /// error line).
    pub(super) fn rule_editor_row(
        &self,
        rule: &Rule,
        muted: Hsla,
        danger: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preview = match self.rule_match_count {
            Some(n) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "rule-edit-preview".into())
                .child(format!("matches {} transactions", flow::count(n)))
                .into_any_element(),
            None => div().into_any_element(),
        };
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(
                        // 3.23: the pattern field is the flex column, so
                        // the editor fits the detail panel's width; the
                        // other controls keep their size.
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(field_label(muted, "Pattern"))
                            .child(
                                div()
                                    .w_full()
                                    .debug_selector(|| "rule:edit-pattern-input".into())
                                    .child(Input::new(&self.rule_edit_pattern)),
                            ),
                    )
                    .child(
                        // 3.42: the unlabelled switch sits in a control-height
                        // slot so its centre matches the input, picker and
                        // buttons.
                        control_slot(
                            "rule:regex-slot:edit".into(),
                            "rule:switch:edit".into(),
                            Switch::new(SharedString::from(format!("rule-edit-regex-{}", rule.id)))
                                .label("regex")
                                .checked(self.rule_edit_regex)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.rule_edit_regex = *checked;
                                    this.preview_rule_match(cx);
                                    cx.notify();
                                })),
                        ),
                    )
                    // 3.53: no Category picker — the editor changes
                    // pattern and kind only, and the rule keeps its
                    // category (moving is the Rules page Move bar's job).
                    .child(
                        Button::new(SharedString::from(format!("rule-edit-save-{}", rule.id)))
                            .label("Save")
                            .primary()
                            .flex_shrink_0()
                            // 3.42: the alignment test measures its centre.
                            .debug_selector(|| "rule:edit-save-btn".into())
                            // 3.40: loading + disabled while the save runs.
                            .loading(self.rule_action_pending(PendingRuleAction::Save(rule.id)))
                            .disabled(self.rule_action_pending(PendingRuleAction::Save(rule.id)))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_rule_edit(window, cx)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("rule-edit-cancel-{}", rule.id)))
                            .label("Cancel")
                            .ghost()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_edit_rule(cx))),
                    ),
            )
            // 3.23: the count sits under the fields (as in the add-rule
            // row): the detail panel is too narrow to keep it inline.
            .child(preview)
            .children(
                self.rule_error
                    .clone()
                    // One line, ellipsized (newlines flattened): the slot
                    // reserves room for exactly one error line, and the
                    // regex crate's messages are multi-line.
                    .map(|e| e.replace('\n', " "))
                    .map(|e| {
                        div()
                            .text_sm()
                            .text_color(danger)
                            .text_ellipsis()
                            .debug_selector(|| "rule-edit-error".into())
                            .child(e)
                    }),
            )
            .into_any_element()
    }

    // -- Rules page (3.28) ----------------------------------------------------

    /// "Parent › Child" label of the rule's target category, shared by the
    /// row, the group headers and the search matching.
    fn rule_target_label(&self, rule: &Rule) -> String {
        self.child_choices
            .iter()
            .find(|(id, _)| *id == rule.category_id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("#{}", rule.category_id))
    }

    /// The Rules page list's entries after the page filters (D5). Normal
    /// mode groups the rules by target subcategory (groups in label order,
    /// rows within a group sorted by Pattern — the default — or by Uses),
    /// with a group header row per group. "Test a description" mode shows
    /// a flat list of the rules matching the text, in the order
    /// [`winning_rule`] tries them (substring before regex, longest pattern
    /// first, lowest id), so the winner is the first row.
    fn rules_page_items(&self, cx: &App) -> Vec<RulesItem> {
        let query = self
            .rules_page
            .rules_search
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let query = (!query.is_empty()).then_some(query);
        // Type picker: row 1 keeps substrings, row 2 regexes, nothing
        // selected or row 0 shows all.
        let type_ix = self
            .rules_page
            .rules_type_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        let test = self.rules_page.rules_test_mode;
        let mut filtered: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| match type_ix {
                1 => !r.is_regex,
                2 => r.is_regex,
                _ => true,
            })
            .filter(|r| {
                !self.rules_page.rules_unused_only
                    || self.rules_page.rule_uses.get(&r.id).copied().unwrap_or(0) == 0
            })
            .filter(|r| {
                if test {
                    // The search box is the description: only rules that
                    // match it stay.
                    query
                        .as_deref()
                        .is_some_and(|q| pattern_matches(&r.pattern, r.is_regex, q))
                } else if let Some(q) = &query {
                    // Pattern and target category name, case-insensitive.
                    r.pattern.to_ascii_lowercase().contains(q)
                        || self.rule_target_label(r).to_ascii_lowercase().contains(q)
                } else {
                    true
                }
            })
            .collect();
        if test {
            filtered.sort_by(|a, b| {
                a.is_regex
                    .cmp(&b.is_regex)
                    .then_with(|| b.pattern.chars().count().cmp(&a.pattern.chars().count()))
                    .then_with(|| a.id.cmp(&b.id))
            });
            return filtered
                .into_iter()
                .map(|r| RulesItem::Rule { id: r.id })
                .collect();
        }
        // Group by target label, groups in label order.
        let mut groups: Vec<(String, i64, Vec<&Rule>)> = Vec::new();
        for r in &filtered {
            let label = self.rule_target_label(r);
            match groups.iter_mut().find(|(l, _, _)| *l == label) {
                Some((_, _, rules)) => rules.push(r),
                None => groups.push((label, r.category_id, vec![r])),
            }
        }
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        let by_uses = self.rules_page.rules_sort_by_uses;
        let mut items = Vec::with_capacity(filtered.len() + groups.len());
        for (label, cid, mut rules) in groups {
            let count = rules.len();
            let collapsed = self.rules_page.collapsed_rule_groups.contains(&cid);
            items.push(RulesItem::Group {
                id: cid,
                label,
                count,
                collapsed,
            });
            if collapsed {
                continue;
            }
            rules.sort_by(|a, b| {
                if by_uses {
                    let ua = self.rules_page.rule_uses.get(&a.id).copied().unwrap_or(0);
                    let ub = self.rules_page.rule_uses.get(&b.id).copied().unwrap_or(0);
                    ub.cmp(&ua).then_with(|| a.pattern.cmp(&b.pattern))
                } else {
                    a.pattern.cmp(&b.pattern)
                }
            });
            items.extend(rules.into_iter().map(|r| RulesItem::Rule { id: r.id }));
        }
        items
    }

    fn toggle_rule_group(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.rules_page.collapsed_rule_groups.remove(&id) {
            self.rules_page.collapsed_rule_groups.insert(id);
        }
        cx.notify();
    }

    pub(super) fn set_rule_selected(&mut self, id: i64, on: bool, cx: &mut Context<Self>) {
        if on {
            self.rules_page.selected_rules.insert(id);
        } else {
            self.rules_page.selected_rules.remove(&id);
        }
        self.rules_page.rules_move_error = None;
        cx.notify();
    }

    /// The Move bar's action: one [`move_rules`] call moves every checked
    /// rule and re-runs the rules over the unlocked rows once; then the
    /// selection clears and everything refreshes.
    pub(super) fn move_selected_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self
            .rules_page
            .move_rules_target
            .read(cx)
            .selected_index(cx)
        else {
            self.rules_page.rules_move_error = Some("Choose the category to move to.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        let mut ids: Vec<i64> = self.rules_page.selected_rules.iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        ids.sort();
        let db = self.db.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_rules(c, &ids, category_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.rules_page.selected_rules.clear();
                        this.rules_page.rules_move_error = None;
                    }
                    Err(e) => this.rules_page.rules_move_error = Some(e.into()),
                }
                this.refresh(window, cx);
            });
        }));
        cx.notify();
    }

    /// The *Show rule* item (3.29): open the Rules page on the given rule.
    /// The page's filters are cleared so the rule is certainly in the list
    /// (test mode, the Type picker and Unused could all hide it), its group
    /// is expanded, and the search box takes the rule's pattern — the list
    /// narrows to it find-in-page style, and a rule always matches its own
    /// pattern. The row is scrolled into view and carries the `list_active`
    /// background until the user searches or leaves the page.
    pub(super) fn show_rule(&mut self, rule_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // Navigate first: `go_page` clears the page's transient state, and
        // the reveal must survive that reset.
        self.go_page(Page::Rules, cx);
        let Some(rule) = self.rules.iter().find(|r| r.id == rule_id).cloned() else {
            return; // a refresh dropped the rule between menu and click
        };
        self.rules_page.rules_test_mode = false;
        self.rules_page.rules_unused_only = false;
        self.rules_page
            .collapsed_rule_groups
            .remove(&rule.category_id);
        self.rules_page.rules_type_select.update(cx, |s, cx| {
            s.set_selected_index(Some(gpui_kit::component::IndexPath::new(0)), window, cx)
        });
        // `set_value` scrolls the list back to the top, so the reveal's
        // `scroll_to_item` must come after it. Any later keystroke fires
        // the search subscription, which drops the reveal.
        let pattern = rule.pattern.clone();
        self.rules_page
            .rules_search
            .update(cx, |i, cx| i.set_value(pattern, window, cx));
        let items = self.rules_page_items(cx);
        if let Some(ix) = items
            .iter()
            .position(|it| matches!(it, RulesItem::Rule { id } if *id == rule_id))
        {
            self.rules_page.rules_reveal = Some(rule_id);
            self.rules_page
                .rules_scroll
                .scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// The Rules page group header row: "Food › Coffee · 4 rules", clicking
    /// it collapses or expands the group's rows (D5).
    fn rules_group_header(
        &self,
        id: i64,
        label: &str,
        count: usize,
        collapsed: bool,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.to_string();
        div()
            .id(SharedString::from(format!("rules-group-{id}")))
            .h_6()
            .w_full()
            .flex()
            .items_center()
            .cursor_pointer()
            .text_sm()
            .text_color(muted)
            .debug_selector(move || format!("rules:group-{id}"))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_rule_group(id, cx)))
            .child(format!(
                "{} {} · {} rules",
                if collapsed { "▸" } else { "▾" },
                label,
                count
            ))
            .into_any_element()
    }

    /// One rule row of the Rules page: checkbox, Pattern (the flex column,
    /// ellipsized with a tooltip like the Categories row), a "wins" marker
    /// in test mode, Type, Uses (right-aligned) and Edit / Delete…. The
    /// row the editor is open on swaps for [`rule_editor_row`], which is
    /// the same editor the Categories page uses. The row *Show rule* (3.29)
    /// landed on paints with the `list_active` background; its debug
    /// selector grows an `:active` suffix so tests can tell the highlight
    /// apart from a plain row.
    fn rules_page_row(
        &self,
        rule: &Rule,
        winner: bool,
        revealed: bool,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = rule.id;
        let armed = self.armed_delete_rule == Some(id);
        let uses = self.rules_page.rule_uses.get(&id).copied().unwrap_or(0);
        let reveal_bg = revealed.then(|| cx.theme().list_active);
        h_flex()
            .h_6()
            .w_full()
            .gap_2()
            .items_center()
            .debug_selector(move || {
                if revealed {
                    format!("rules:row-{id}:active")
                } else {
                    format!("rules:row-{id}")
                }
            })
            .when_some(reveal_bg, |row, bg| row.bg(bg))
            .child(
                div().w(px(RULE_SEL_W)).flex_shrink_0().child(
                    checkbox(SharedString::from(format!("rule-sel-{id}")))
                        .checked(self.rules_page.selected_rules.contains(&id))
                        .on_click(cx.listener(move |this, on: &bool, _, cx| {
                            this.set_rule_selected(id, *on, cx);
                        })),
                ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("rules-pattern-{id}")))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .debug_selector(move || format!("rules:pattern-col:{id}"))
                    .tooltip({
                        let full = rule.pattern.clone();
                        move |window, cx| {
                            let tip = full.clone();
                            Tooltip::element(move |_, _| {
                                div()
                                    .debug_selector({
                                        let p = tip.clone();
                                        move || format!("rules:pattern-tooltip:{p}")
                                    })
                                    .child(tip.clone())
                            })
                            .build(window, cx)
                        }
                    })
                    .child(rule.pattern.clone()),
            )
            .when(winner, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().primary)
                        .debug_selector(move || format!("rules:wins-{id}"))
                        .child("wins"),
                )
            })
            .child(
                div()
                    .w(px(RULE_TYPE_W))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(muted)
                    .debug_selector(move || format!("rules:type-col:{id}"))
                    .child(if rule.is_regex { "regex" } else { "substring" }),
            )
            .child(
                div()
                    .w(px(RULE_USES_W))
                    .flex_shrink_0()
                    .text_right()
                    .text_xs()
                    .text_color(muted)
                    .debug_selector(move || format!("rules:uses-col:{id}"))
                    .child(flow::count(uses as i64)),
            )
            .child(
                Button::new(SharedString::from(format!("rule-edit-{id}")))
                    .label("Edit")
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.start_edit_rule(id, window, cx)
                        }),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("rule-delete-{id}")))
                    .label(if armed {
                        "Click again to delete"
                    } else {
                        "Delete…"
                    })
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
                    // 3.40: loading + disabled while the delete runs.
                    .loading(self.rule_action_pending(PendingRuleAction::Delete(id)))
                    .disabled(self.rule_action_pending(PendingRuleAction::Delete(id)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.arm_delete_rule(id, window, cx)
                        }),
                    ),
            )
            .into_any_element()
    }

    /// The Rules page (3.28, D5): header with the rule count and New rule…,
    /// the add-rule row under it, the filter row (Search, Test a
    /// description, Type, Unused), the Move bar over the list while rows
    /// are checked, and the virtualized list of group headers and rule
    /// rows.
    pub(super) fn render_rules(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (muted, danger) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.danger)
        };
        let count_text = format!("{} rules", self.rules.len());
        let count_sel = count_text.clone();
        let query = self
            .rules_page
            .rules_search
            .read(cx)
            .value()
            .trim()
            .to_string();
        // The "wins" marker follows [`winning_rule`] exactly, so the page
        // marks the rule the categoriser would actually apply (D5).
        let winner = if self.rules_page.rules_test_mode && !query.is_empty() {
            winning_rule(&self.rules, &query)
        } else {
            None
        };
        let items = Rc::new(self.rules_page_items(cx));
        let editing = self.editing_rule;
        let edit_error = self.rule_error.is_some();
        // Rows are 24 px; the row the editor replaced reserves 80 px
        // (108 px while a validation error is open), like the Categories
        // rule list.
        let item_sizes = Rc::new(
            items
                .iter()
                .map(|it| match it {
                    RulesItem::Group { .. } => size(px(0.), px(24.)),
                    RulesItem::Rule { id } => size(
                        px(0.),
                        if editing == Some(*id) {
                            if edit_error { px(108.) } else { px(80.) }
                        } else {
                            px(24.)
                        },
                    ),
                })
                .collect::<Vec<_>>(),
        );
        let list_items = Rc::clone(&items);
        let list = v_virtual_list(
            cx.entity(),
            "rules-list",
            item_sizes,
            move |this, range, _window, cx| {
                let rows: Vec<AnyElement> = list_items
                    .iter()
                    .skip(range.start)
                    .take(range.len())
                    .filter_map(|it| match it {
                        RulesItem::Group {
                            id,
                            label,
                            count,
                            collapsed,
                        } => {
                            Some(this.rules_group_header(*id, label, *count, *collapsed, muted, cx))
                        }
                        RulesItem::Rule { id } => {
                            this.rules.iter().find(|r| r.id == *id).map(|r| {
                                if this.editing_rule == Some(*id) {
                                    this.rule_editor_row(r, muted, danger, cx)
                                } else {
                                    let revealed = this.rules_page.rules_reveal == Some(*id);
                                    this.rules_page_row(r, winner == Some(*id), revealed, muted, cx)
                                }
                            })
                        }
                    })
                    .collect();
                #[cfg(test)]
                this.rules_page.rules_page_rows_built.set(rows.len());
                #[cfg(test)]
                this.rules_page
                    .rules_page_range
                    .set((range.start, range.end));
                rows
            },
        )
        // 3.21: Shell-owned handle (see the tree).
        .track_scroll(&self.rules_page.rules_scroll)
        .gap_1()
        .flex_1()
        .min_h_0();
        let add_preview = match self
            .rule_match_count
            .filter(|_| self.editing_rule.is_none())
        {
            Some(n) => div()
                .text_sm()
                .text_color(muted)
                .debug_selector(|| "rules:add-preview".into())
                .child(format!("matches {} transactions", flow::count(n)))
                .into_any_element(),
            None => div().into_any_element(),
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            // Page header: title, count, New rule… (opens the add row in
            // place, like the Categories header's New category…).
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Rules"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .debug_selector(move || format!("label:{count_sel}"))
                            .child(count_text),
                    )
                    .child(
                        Button::new("new-rule")
                            .label(NEW_RULE)
                            .debug_selector(move || format!("label:{NEW_RULE}"))
                            .primary()
                            .ml_auto()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                // Only one rule editor is open at a time:
                                // opening the add row closes any row editor.
                                this.close_rule_editor();
                                this.rules_page.new_rule_open = !this.rules_page.new_rule_open;
                                cx.notify();
                            })),
                    ),
            )
            // Add-rule row (3.28): the same row the Categories page has,
            // hidden until the header button opens it.
            .when(self.rules_page.new_rule_open, |page| {
                page.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .debug_selector(|| "rules:add-row".into())
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(field_label(muted, "Pattern"))
                                        .child(
                                            div()
                                                .w_full()
                                                .debug_selector(|| "rule:pattern-input:add".into())
                                                .child(Input::new(&self.rule_pattern)),
                                        ),
                                )
                                .child(
                                    // 3.42: control-height slot, centred.
                                    control_slot(
                                        "rule:regex-slot:add".into(),
                                        "rule:switch:add".into(),
                                        Switch::new("rule-regex")
                                            .label("regex")
                                            .checked(self.rule_regex)
                                            .on_click(cx.listener(
                                                |this, checked: &bool, _, cx| {
                                                    this.rule_regex = *checked;
                                                    this.preview_rule_match(cx);
                                                    cx.notify();
                                                },
                                            )),
                                    ),
                                )
                                .child(
                                    v_flex()
                                        .flex_shrink_0()
                                        .gap_1()
                                        .child(field_label(muted, "Category"))
                                        .child(div().w(px(160.)).child(
                                            Select::new(&self.rule_target).placeholder("Category…"),
                                        )),
                                )
                                .child(
                                    Button::new("add-rule")
                                        .label("Add rule")
                                        .primary()
                                        .flex_shrink_0()
                                        // 3.42: the alignment test measures its centre.
                                        .debug_selector(|| "rule:add-rule-btn:add".into())
                                        // 3.40: loading + disabled while the add runs.
                                        .loading(self.rule_action_pending(PendingRuleAction::Add))
                                        .disabled(self.rule_action_pending(PendingRuleAction::Add))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.add_rule(window, cx)
                                        })),
                                ),
                        )
                        .child(add_preview)
                        .children(
                            self.rule_error
                                .clone()
                                .filter(|_| self.editing_rule.is_none())
                                .map(|e| div().text_sm().text_color(danger).child(e)),
                        ),
                )
            })
            // Filter row (D5): Search flexes; Test a description switches
            // the search box into a description; the Type picker and the
            // Unused checkbox trim the list.
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .debug_selector(|| "rules:filter-row".into())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .debug_selector(|| "rules:search".into())
                            .child(Input::new(&self.rules_page.rules_search)),
                    )
                    .child(
                        Switch::new("rules-test-mode")
                            .label("Test a description")
                            .checked(self.rules_page.rules_test_mode)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.rules_page.rules_test_mode = *on;
                                this.rules_page.rules_scroll.set_offset(Point::default());
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_shrink_0()
                            .debug_selector(|| "rules:type-select".into())
                            .child(
                                Select::new(&self.rules_page.rules_type_select)
                                    .placeholder("All types"),
                            ),
                    )
                    .child(
                        checkbox("rules-unused-only")
                            .label("Unused")
                            .checked(self.rules_page.rules_unused_only)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.rules_page.rules_unused_only = *on;
                                this.rules_page.rules_scroll.set_offset(Point::default());
                                cx.notify();
                            })),
                    ),
            )
            // Move bar (D5): only while rows are checked.
            .when(!self.rules_page.selected_rules.is_empty(), |page| {
                let n = self.rules_page.selected_rules.len();
                page.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .debug_selector(|| "rules:move-bar".into())
                                .child(
                                    div()
                                        .text_sm()
                                        .debug_selector(|| "rules:selected-count".into())
                                        .child(format!("{n} selected")),
                                )
                                .child(
                                    v_flex().gap_1().child(field_label(muted, "Move to")).child(
                                        div().w(px(240.)).child(
                                            Select::new(&self.rules_page.move_rules_target)
                                                .placeholder(MOVE_RULES_TO_PH),
                                        ),
                                    ),
                                )
                                .child(
                                    Button::new("rules-move")
                                        .label("Move")
                                        .primary()
                                        .flex_shrink_0()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.move_selected_rules(window, cx)
                                        })),
                                ),
                        )
                        .children(self.rules_page.rules_move_error.clone().map(|e| {
                            div()
                                .text_sm()
                                .text_color(danger)
                                .debug_selector(|| "rules:move-error".into())
                                .child(e)
                        })),
                )
            })
            .when(
                !self.rules_page.rules_test_mode && !items.is_empty(),
                |page| {
                    page.child(
                        h_flex()
                            .gap_2()
                            .items_end()
                            .debug_selector(|| "rules:heading-row".into())
                            .child(div().w(px(RULE_SEL_W)).flex_shrink_0())
                            .child(
                                div().flex_1().min_w_0().child(
                                    Button::new("rules-sort-pattern")
                                        .label(if self.rules_page.rules_sort_by_uses {
                                            "Pattern"
                                        } else {
                                            "Pattern ↓"
                                        })
                                        .ghost()
                                        .text_xs()
                                        .debug_selector(|| "rules:sort-pattern".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.rules_page.rules_sort_by_uses = false;
                                            cx.notify();
                                        })),
                                ),
                            )
                            .child(
                                div().w(px(RULE_TYPE_W)).flex_shrink_0().child(
                                    field_label(muted, "Type")
                                        .debug_selector(|| "rules:heading-type".into()),
                                ),
                            )
                            .child(
                                div().w(px(RULE_USES_W)).flex_shrink_0().child(
                                    Button::new("rules-sort-uses")
                                        .label(if self.rules_page.rules_sort_by_uses {
                                            "Uses ↓"
                                        } else {
                                            "Uses"
                                        })
                                        .ghost()
                                        .text_xs()
                                        .debug_selector(|| "rules:sort-uses".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.rules_page.rules_sort_by_uses = true;
                                            cx.notify();
                                        })),
                                ),
                            )
                            .child(div().flex_shrink_0().child(field_label(muted, "Actions"))),
                    )
                },
            )
            .when(items.is_empty(), |page| {
                page.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .debug_selector(|| "rules:empty".into())
                        .child(if self.rules_page.rules_test_mode {
                            "No rules match this description"
                        } else {
                            "No rules match"
                        }),
                )
            })
            .child(list)
            .into_any_element()
    }
}
