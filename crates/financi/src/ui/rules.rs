//! The Rules page (3.62), a page entity since 3.80.
//!
//! The page owns its search, filters, collapse and sort state, the checked
//! rows, and the add-rule row with its in-place editor (3.24, 3.28). The
//! editor is shared with the Categories page — 3.53 kept one editor, not
//! two — so `rule_row` and `rule_editor_row` are free functions here that
//! render from this entity's state, and `Shell` keeps thin wrapper methods
//! for the Categories rule list. While the Rules page is hidden the
//! Categories page still paints that state, so `Shell` observes this
//! entity: a page notify repaints both.

use super::*;
use crate::ui::page::{Cancel, PageEntity, PageEvent, watch_catalog};
use crate::ui::widgets::*;

/// 3.28: the Rules page's header action; it opens the add-rule row in
/// place under the header (ui-conventions, *Pages*). 3.43: no trailing
/// "…" — the button carries a `plus` icon instead.
pub(super) const NEW_RULE: &str = "New rule";
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
/// [`RULE_TYPE_W`]: row and column headings use these. 3.49: the
/// Categories page's rule list swapped its Category column for the same
/// Uses column, so the width is shared with it.
const RULE_SEL_W: f32 = 24.;
pub(super) const RULE_USES_W: f32 = 80.;

/// 3.48: the height the editor row reserves in the virtualized lists —
/// the Rules page's `item_sizes` and the Categories page's `rule_sizes`
/// share these, so the two-line [`inline_form`] editor (labelled Pattern
/// line + 32 px controls line, preview count under it) never paints over
/// the row below it on either page. Measured at 1200 × 720: 148 px with
/// the preview, and 174.5 px with an error line open (the preview stays
/// under a refused save), so the slots reserve 148 / 176.
/// `the_rule_editor_gives_the_pattern_room` and
/// `the_editor_error_stays_inside_its_row` pin them down.
pub(super) const RULE_EDITOR_H: f32 = 148.;
pub(super) const RULE_EDITOR_ERROR_H: f32 = 176.;

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
/// rule edit, Add rule, or a row's Delete). While one is pending that
/// button renders `.loading(true)` and disabled, so a slow save can't be
/// double-submitted; the task clears it when it finishes, on success or
/// error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PendingRuleAction {
    Add,
    Save(i64),
    Delete(i64),
}

/// The Rules page (3.74 state, 3.80 entity): the search, filters, checked
/// rows and list state, plus the add-rule row and its in-place editor —
/// the same editor the Categories page renders (see the module docs). The
/// rule list and Uses counts are mirrors of the `Catalog` snapshot;
/// `apply` refreshes them and prunes the page's per-rule state to the
/// rules that still exist.
pub(super) struct RulesPage {
    db: DbWorker,
    /// The shared data model (3.78); `apply` reads the rule list, the Uses
    /// counts and the category tree off its snapshot, never a query of
    /// its own.
    catalog: Entity<Catalog>,
    /// 5.3 review fix: shared with the shell — while an export or restore
    /// is in flight every write here is refused (the row ids may name
    /// different rules after it).
    writes_blocked: Rc<Cell<bool>>,
    focus: FocusHandle,
    visible: bool,
    dirty: bool,
    focus_pending: bool,
    /// The `Catalog` subscription (3.80); lives as long as the page.
    _catalog: Subscription,
    // -- Page state (3.74).
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
    /// 3.28: whether the add-rule row is open on the Rules page.
    pub(super) new_rule_open: bool,
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
    // -- `Catalog` snapshot mirrors (3.80).
    /// The shared rule list; the Categories page's list is a view over
    /// the same rows (`Shell` mirrors it too for its other pages).
    pub(super) rules: Vec<Rule>,
    /// 3.28: how many unlocked rows each rule's match decides (the Uses
    /// column); rules deciding nothing are absent.
    pub(super) rule_uses: HashMap<i64, usize>,
    /// "Parent › Child" labels of the child categories, the target the
    /// rows and group headers show and what the pickers offer.
    pub(super) child_choices: Vec<(i64, String)>,
    // -- The add-rule row and its in-place editor (3.24, 3.28): rendered
    // by both the Categories page and the Rules page.
    pub(super) rule_pattern: Entity<InputState>,
    pub(super) rule_regex: bool,
    pub(super) rule_target: Entity<SelectState<Names>>,
    /// The add row's and the editor's shared "matches N transactions"
    /// preview (the editor owns it while open; see
    /// [`RulesPage::preview_rule_match`]).
    pub(super) rule_match_count: Option<i64>,
    rule_task: Option<Task<()>>,
    pub(super) rule_error: Option<String>,
    /// 3.24: the rule whose row currently shows the in-place editor. One at
    /// a time; `select_category` and `go_page` close it. `start_edit_rule`
    /// loads the rule into the editor's own controls, so the editor is the
    /// add-rule row's shape with the row swapped out.
    pub(super) editing_rule: Option<i64>,
    /// 3.24: the rule whose Delete button is armed ("Click again to
    /// delete"). Armed per row; `select_category` and `go_page` disarm it
    /// with the page's other transient confirmations.
    pub(super) armed_delete_rule: Option<i64>,
    /// 3.40: which rule button is waiting on the DB (see
    /// [`PendingRuleAction`]); rendered loading + disabled while set.
    pub(super) pending_rule_action: Option<PendingRuleAction>,
    /// 3.24: the editor's own controls, separate entities from the add-rule
    /// row's so the two never share an `InputState` on screen at once.
    /// While the editor is open the shared `rule_match_count`
    /// preview belongs to it (the add-rule row hides it, like `rule_error`)
    /// and excludes that rule (2.5 re-run semantics); every close path
    /// clears it, and [`RulesPage::apply`] closes the editor when its rule
    /// is gone.
    pub(super) rule_edit_pattern: Entity<InputState>,
    pub(super) rule_edit_regex: bool,
    /// The pickers and inputs above subscribe here.
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for RulesPage {}

impl PageEntity for RulesPage {
    /// The list renders the rules and the category labels; a category
    /// rename moves a group header, and the writes that touch either
    /// re-run the rules, so the transactions too.
    const DEPENDS: Domains = Domains::CATEGORIES_RULES_TX;

    fn visible(&self) -> bool {
        self.visible
    }

    fn dirty(&self) -> bool {
        self.dirty
    }

    fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    fn set_shown(&mut self, visible: bool) {
        self.visible = visible;
    }

    fn set_focus_pending(&mut self, pending: bool) {
        self.focus_pending = pending;
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let (rules, uses, tree) = {
            let cat = self.catalog.read(cx);
            (
                cat.snap.rules.clone(),
                cat.snap.uses.clone(),
                cat.snap.tree.clone(),
            )
        };
        self.rules = rules;
        self.rule_uses = uses;
        self.child_choices = tree
            .iter()
            .flat_map(|p| {
                p.children
                    .iter()
                    .map(move |c| (c.id, format!("{} › {}", p.name, c.name)))
            })
            .collect();
        // 3.28: a refresh that dropped a rule drops its checkbox with it;
        // a deleted category drops its collapsed-group marker.
        self.selected_rules
            .retain(|id| self.rules.iter().any(|r| r.id == *id));
        self.collapsed_rule_groups.retain(|id| {
            tree.iter()
                .any(|p| p.id == *id || p.children.iter().any(|c| c.id == *id))
        });
        // 3.24: a refresh that dropped the rule (its category was deleted,
        // say) drops the editor open on it and its armed delete with it,
        // like the `rule_offer` check in `refresh`.
        if let Some(id) = self.editing_rule
            && !self.rules.iter().any(|r| r.id == id)
        {
            self.close_rule_editor();
        }
        if let Some(id) = self.armed_delete_rule
            && !self.rules.iter().any(|r| r.id == id)
        {
            self.armed_delete_rule = None;
        }
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.set_shown(visible);
        if visible {
            self.apply_if_dirty(cx);
            self.set_focus_pending(true);
        } else {
            self.set_dirty(true);
            // 3.11: no transient UI travels across a page switch — the
            // Rules-only state (the add row, the ticked rows, the move
            // error and the *Show rule* highlight) clears on leaving; the
            // shared editor and armed delete close in `go_page`, which
            // runs on every switch.
            self.new_rule_open = false;
            self.selected_rules.clear();
            self.rules_move_error = None;
            self.rules_reveal = None;
        }
    }
}

impl RulesPage {
    pub(super) fn new(
        db: DbWorker,
        writes_blocked: Rc<Cell<bool>>,
        catalog: &Entity<Catalog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 3.28: the Rules page search box filters the list on every
        // keystroke, and in "Test a description" mode it is the description
        // the rules are tried against (D5). Typing replaces the list, so the
        // scroll offset goes back to the top.
        let rules_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search pattern or category…"));
        let rules_search_sub = cx.subscribe(&rules_search, |this, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.rules_scroll.set_offset(Point::default());
                // 3.29: the reveal highlight is one-shot; anything typed
                // after it is the user's own search.
                this.rules_reveal = None;
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
            |this, _select, ev: &SelectEvent<Names>, _window, cx| {
                if matches!(ev, SelectEvent::Confirm(_)) {
                    this.rules_scroll.set_offset(Point::default());
                    cx.notify();
                }
            },
        );
        let move_rules_target = cx.new(|cx| SelectState::new(Names::new(), None, window, cx));
        // The add-rule row and its editor (3.24, 3.28): rendered by both
        // the Categories page and the Rules page.
        let rule_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        let rule_edit_pattern = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text or regex a description must contain")
        });
        // The live "matches N transactions" preview runs on every pattern
        // keystroke (ui-conventions, Categories page).
        let preview_sub = cx.subscribe(&rule_pattern, |this, _, ev: &InputEvent, cx| {
            // 3.24: while a rule editor is open, the preview belongs to it.
            if matches!(ev, InputEvent::Change) && this.editing_rule.is_none() {
                this.preview_rule_match(cx);
            }
        });
        // 3.24: the rule editor's Pattern input drives the same preview and
        // saves on Enter (Escape cancels through the page's `Cancel`).
        let rule_edit_sub = cx.subscribe_in(
            &rule_edit_pattern,
            window,
            |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change if this.editing_rule.is_some() => this.preview_rule_match(cx),
                InputEvent::PressEnter {
                    secondary: false, ..
                } if this.editing_rule.is_some() => this.save_rule_edit(window, cx),
                _ => {}
            },
        );
        let _catalog = watch_catalog(catalog, cx);
        Self {
            db,
            catalog: catalog.clone(),
            writes_blocked,
            focus: cx.focus_handle(),
            // `Shell::new` marks the initial page visible right after
            // construction; every page starts hidden.
            visible: false,
            dirty: true,
            focus_pending: false,
            _catalog,
            rules_search,
            rules_test_mode: false,
            rules_type_select,
            rules_unused_only: false,
            collapsed_rule_groups: HashSet::default(),
            rules_sort_by_uses: false,
            selected_rules: HashSet::default(),
            move_rules_target,
            rules_move_error: None,
            new_rule_open: false,
            rules_reveal: None,
            rules_scroll: VirtualListScrollHandle::new(),
            #[cfg(test)]
            rules_page_rows_built: std::cell::Cell::new(0),
            #[cfg(test)]
            rules_page_range: std::cell::Cell::new((0, 0)),
            rules: Vec::new(),
            rule_uses: HashMap::default(),
            child_choices: Vec::new(),
            rule_pattern,
            rule_regex: false,
            rule_target: cx.new(|cx| SelectState::new(Names::new(), None, window, cx)),
            rule_match_count: None,
            rule_task: None,
            rule_error: None,
            editing_rule: None,
            armed_delete_rule: None,
            pending_rule_action: None,
            rule_edit_pattern,
            rule_edit_regex: false,
            _subs: vec![rules_search_sub, rules_type_sub, preview_sub, rule_edit_sub],
        }
    }
}

impl RulesPage {
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

    /// The shared tail of both add-rule paths: validate the pattern, run
    /// [`financi_db::rules::add_rule`], clear the row and refresh (3.40:
    /// the button shows loading, disabled, while the DB is busy — a save
    /// that re-runs the rules can take seconds).
    fn commit_new_rule(&mut self, category_id: i64, window: &mut Window, cx: &mut Context<Self>) {
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
                    Ok(_) => {
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
                    // 3.64: and only this add reports it: a failure that
                    // lands after the user moved on (another action owns the
                    // badge, or an editor is open) must not paint under
                    // someone else's fields — `report_rule_error` toasts it
                    // while an editor is open, and a stale add's failure is
                    // the newer action's to report.
                    Err(e) if own => this.report_rule_error(e, cx),
                    Err(_) => {}
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
                cx.notify();
            });
        })
        .detach();
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
    pub(super) fn save_rule_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn arm_delete_rule(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        if self.armed_delete_rule == Some(id) {
            let db = self.db.clone();
            // 3.40: the delete runs (and the button shows loading,
            // disabled) while the rules re-run over the unlocked rows.
            self.pending_rule_action = Some(PendingRuleAction::Delete(id));
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
                        Err(e) => this.report_rule_error(e, cx),
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

/// One row of a category's rule list (3.12: built lazily by the
/// virtualized rule list, fixed height to match its `item_sizes`). 3.24:
/// every row carries Edit and Delete at its right end — small ghost
/// buttons, visible on every row, built only for the rows the list
/// renders. 3.23: the columns line up with the list's headings
/// (`RULE_TYPE_W`/`RULE_USES_W`); the pattern column is the flex one and
/// truncates with a tooltip, like the queue's merchant column.
///
/// 3.49: the third column is the rule's Uses count — transactions this
/// rule categorizes, the same number the Rules page shows — right-aligned
/// through `flow::count`; 0 is muted, nonzero takes the row's own color.
/// `subcat` is the tag the Categories page passes when its selection is a
/// parent: the subcategory the rule targets, painted after the pattern
/// inside the Pattern column (the pattern ellipsizes first).
///
/// 3.80: a free function over the page entity — the Categories page paints
/// these rows from the Rules page's state (the module docs). The handlers
/// update the entity directly, so they work from either page.
fn rule_row(
    rule: &Rule,
    muted: Hsla,
    uses: usize,
    subcat: Option<SharedString>,
    page: &RulesPage,
    entity: &Entity<RulesPage>,
) -> AnyElement {
    let id = rule.id;
    let armed = page.armed_delete_rule == Some(id);
    let delete_pending = page.rule_action_pending(PendingRuleAction::Delete(id));
    let edit_entity = entity.clone();
    let delete_entity = entity.clone();
    h_flex()
        .h_6()
        .w_full()
        .gap_2()
        .items_center()
        .debug_selector(move || format!("cat:rule-row:{id}"))
        .child(
            h_flex()
                .id(SharedString::from(format!("rule-pattern-{id}")))
                .flex_1()
                .min_w_0()
                .debug_selector(move || format!("cat:rule-pattern-col:{id}"))
                .child(
                    div()
                        .id(SharedString::from(format!("rule-pattern-text-{id}")))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .tooltip({
                            let full = rule.pattern.clone();
                            move |window, cx| {
                                let tip = full.clone();
                                Tooltip::element(move |_, _| {
                                    div()
                                        .debug_selector({
                                            let p = tip.clone();
                                            move || format!("cat:rule-pattern-tooltip:{p}")
                                        })
                                        .child(tip.clone())
                                })
                                .build(window, cx)
                            }
                        })
                        .child(rule.pattern.clone()),
                )
                .when_some(subcat, |col, name| {
                    let sel = format!("cat:rule-subcat:{id}:{name}");
                    col.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .debug_selector(move || sel.clone())
                            .child(name),
                    )
                }),
        )
        .child(
            div()
                .w(px(RULE_TYPE_W))
                .flex_shrink_0()
                .text_xs()
                .text_color(muted)
                .debug_selector(move || format!("cat:rule-type-col:{id}"))
                .child(if rule.is_regex { "regex" } else { "substring" }),
        )
        .child(
            div()
                .w(px(RULE_USES_W))
                .flex_shrink_0()
                .text_right()
                .text_xs()
                // 3.49: a rule nothing has categorized yet reads muted.
                .when(uses == 0, |cell| cell.text_color(muted))
                .debug_selector(move || format!("cat:rule-uses-col:{id}"))
                .child(
                    div()
                        .debug_selector(move || format!("cat:rule-uses:{id}:{uses}"))
                        .child(flow::count(uses as i64)),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("rule-edit-{id}")))
                .icon(IconName::Pencil)
                .label("Edit")
                .ghost()
                .text_xs()
                .flex_shrink_0()
                .debug_selector(move || format!("cat:rule-edit:{id}:{}", "icons/pencil.svg"))
                .on_click(move |_, window, cx| {
                    edit_entity.update(cx, |page, cx| page.start_edit_rule(id, window, cx));
                }),
        )
        .child(
            Button::new(SharedString::from(format!("rule-delete-{id}")))
                .icon(IconName::Trash)
                // 3.43: "Delete" with a trash icon, no "…"; armed it still
                // reads "Click again to delete".
                .label(if armed {
                    "Click again to delete"
                } else {
                    "Delete"
                })
                .ghost()
                .text_xs()
                .flex_shrink_0()
                .debug_selector(move || format!("cat:rule-delete:{id}:{}", "icons/trash.svg"))
                // 3.40: loading + disabled while the delete runs.
                .loading(delete_pending)
                .disabled(delete_pending)
                .on_click(move |_, window, cx| {
                    delete_entity.update(cx, |page, cx| page.arm_delete_rule(id, window, cx));
                }),
        )
        .into_any_element()
}

/// 3.24: the in-place editor that replaces one rule's row, pre-filled
/// from the rule, with the same live "matches N transactions" count. A
/// validation error shows in `danger` under the editor and keeps it open —
/// clamped to one ellipsized line, because the virtual list doesn't clip
/// per item and a taller row would paint over the rows below.
///
/// 3.48: the editor is 3.89's [`inline_form`] — a bordered form opened in
/// place of the row, two lines tall so the Pattern input gets the editor's
/// full width (the old single row shrank it past editing in the detail
/// panel). Line 1 is the "Pattern" label over the input; line 2 is one
/// `items_center` row of the 32 px controls: the regex switch, then Save
/// and Cancel pushed right with `ml_auto`. Line 2 carries no labels, so
/// the switch lines up on its own — no 3.42 `control_slot` here (the
/// add-rule rows keep theirs). The preview count and error line stay
/// under both lines; the row reserves [`RULE_EDITOR_H`]
/// ([`RULE_EDITOR_ERROR_H`] with an error open).
///
/// 3.53: no Category picker — the editor changes pattern and kind only,
/// and the rule keeps its category (moving is the Rules page Move bar's
/// job). 3.80: a free function over the page entity, like [`rule_row`].
fn rule_editor_row(
    rule: &Rule,
    muted: Hsla,
    danger: Hsla,
    page: &RulesPage,
    entity: &Entity<RulesPage>,
    cx: &App,
) -> AnyElement {
    let preview = match page.rule_match_count {
        Some(n) => div()
            .text_sm()
            .text_color(muted)
            .debug_selector(|| "rules:edit-preview".into())
            .child(format!("matches {} transactions", flow::count(n)))
            .into_any_element(),
        None => div().into_any_element(),
    };
    let switch_entity = entity.clone();
    let save_entity = entity.clone();
    let cancel_entity = entity.clone();
    v_flex()
        .w_full()
        .gap_1()
        .debug_selector(|| "rules:edit-form".into())
        .child(
            inline_form(cx).child(
                v_flex()
                    .w_full()
                    .gap_2()
                    // Line 1: the "Pattern" label over the input, at the
                    // editor's full width (the user-reported bug: the
                    // one-row editor squeezed the input to nothing).
                    .child(
                        v_flex().gap_1().child(field_label(muted, "Pattern")).child(
                            div()
                                .w_full()
                                .debug_selector(|| "rules:edit-pattern-input".into())
                                .child(Input::new(&page.rule_edit_pattern)),
                        ),
                    )
                    // Line 2: the controls, no labels — the switch at the
                    // left, Save and Cancel at the right end (`ml_auto`
                    // on the first of them). `items_center` lines the
                    // switch up with the buttons without a slot (3.48).
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .debug_selector(|| "rules:edit-controls-row".into())
                            .child(
                                div()
                                    .flex_shrink_0()
                                    // 3.42's probe selector: the
                                    // alignment test measures the switch
                                    // itself, not the row around it.
                                    .debug_selector(|| "rules:switch:edit".into())
                                    .child(
                                        Switch::new(SharedString::from(format!(
                                            "rule-edit-regex-{}",
                                            rule.id
                                        )))
                                        .label("regex")
                                        .checked(page.rule_edit_regex)
                                        .on_click(
                                            move |checked: &bool, _, cx| {
                                                switch_entity.update(cx, |page, cx| {
                                                    page.rule_edit_regex = *checked;
                                                    page.preview_rule_match(cx);
                                                    cx.notify();
                                                });
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "rule-edit-save-{}",
                                    rule.id
                                )))
                                .label("Save")
                                .primary()
                                .ml_auto()
                                .flex_shrink_0()
                                // 3.42: the alignment test measures its centre.
                                .debug_selector(|| "rules:edit-save-btn".into())
                                // 3.40: loading + disabled while the save runs.
                                .loading(page.rule_action_pending(PendingRuleAction::Save(rule.id)))
                                .disabled(
                                    page.rule_action_pending(PendingRuleAction::Save(rule.id)),
                                )
                                .on_click(
                                    move |_, window, cx| {
                                        save_entity
                                            .update(cx, |page, cx| page.save_rule_edit(window, cx));
                                    },
                                ),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "rule-edit-cancel-{}",
                                    rule.id
                                )))
                                .label("Cancel")
                                .ghost()
                                .flex_shrink_0()
                                .debug_selector(|| "rules:edit-cancel-btn".into())
                                .on_click(move |_, _, cx| {
                                    cancel_entity.update(cx, |page, cx| page.cancel_edit_rule(cx));
                                }),
                            ),
                    ),
            ),
        )
        // 3.23: the count sits under the fields (as in the add-rule
        // row): the detail panel is too narrow to keep it inline.
        .child(preview)
        .children(
            page.rule_error
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
                        .debug_selector(|| "rules:edit-error".into())
                        .child(e)
                }),
        )
        .into_any_element()
}

impl RulesPage {
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
            .rules_search
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let query = (!query.is_empty()).then_some(query);
        // Type picker: row 1 keeps substrings, row 2 regexes, nothing
        // selected or row 0 shows all.
        let type_ix = self
            .rules_type_select
            .read(cx)
            .selected_index(cx)
            .map(|ix| ix.row)
            .unwrap_or(0);
        let test = self.rules_test_mode;
        let mut filtered: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| match type_ix {
                1 => !r.is_regex,
                2 => r.is_regex,
                _ => true,
            })
            .filter(|r| {
                !self.rules_unused_only || self.rule_uses.get(&r.id).copied().unwrap_or(0) == 0
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
        let by_uses = self.rules_sort_by_uses;
        let mut items = Vec::with_capacity(filtered.len() + groups.len());
        for (label, cid, mut rules) in groups {
            let count = rules.len();
            let collapsed = self.collapsed_rule_groups.contains(&cid);
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
                    let ua = self.rule_uses.get(&a.id).copied().unwrap_or(0);
                    let ub = self.rule_uses.get(&b.id).copied().unwrap_or(0);
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
        if !self.collapsed_rule_groups.remove(&id) {
            self.collapsed_rule_groups.insert(id);
        }
        cx.notify();
    }

    pub(super) fn set_rule_selected(&mut self, id: i64, on: bool, cx: &mut Context<Self>) {
        if on {
            self.selected_rules.insert(id);
        } else {
            self.selected_rules.remove(&id);
        }
        self.rules_move_error = None;
        cx.notify();
    }

    /// The Move bar's action: one [`move_rules`] call moves every checked
    /// rule and re-runs the rules over the unlocked rows once; then the
    /// selection clears and everything refreshes.
    pub(super) fn move_selected_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 5.3 review fix: refuse writes while an export/restore is in flight.
        if self.writes_blocked.get() {
            return;
        }
        let Some(ix) = self.move_rules_target.read(cx).selected_index(cx) else {
            self.rules_move_error = Some("Choose the category to move to.".into());
            cx.notify();
            return;
        };
        let Some((category_id, _)) = self.child_choices.get(ix.row).cloned() else {
            return;
        };
        let mut ids: Vec<i64> = self.selected_rules.iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        ids.sort();
        let moved = ids.clone();
        let db = self.db.clone();
        // 3.64: detached — see `commit_new_rule`.
        cx.spawn_in(window, async move |this, cx| {
            let result = on_db(&db, move |c| move_rules(c, &ids, category_id)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(()) => {
                        // Only the moved rules leave the selection: rules
                        // checked while this move ran keep their checkbox.
                        this.selected_rules.retain(|id| !moved.contains(id));
                        this.rules_move_error = None;
                    }
                    Err(e) => this.rules_move_error = Some(e.into()),
                }
                cx.emit(PageEvent::Changed(Domains::RULES_TX));
                cx.notify();
            });
        })
        .detach();
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
        // Reached from the dashboard menu, the page may not have been
        // shown since the last change: apply first, so the mirrors name
        // real rows.
        self.apply_if_dirty(cx);
        let Some(rule) = self.rules.iter().find(|r| r.id == rule_id).cloned() else {
            // a refresh dropped the rule between menu and click
            cx.emit(PageEvent::Go(crate::ui::Page::Rules));
            return;
        };
        self.rules_test_mode = false;
        self.rules_unused_only = false;
        self.collapsed_rule_groups.remove(&rule.category_id);
        self.rules_type_select.update(cx, |s, cx| {
            s.set_selected_index(Some(gpui_kit::component::IndexPath::new(0)), window, cx)
        });
        // `set_value` scrolls the list back to the top, so the reveal's
        // `scroll_to_item` must come after it. Any later keystroke fires
        // the search subscription, which drops the reveal.
        let pattern = rule.pattern.clone();
        self.rules_search
            .update(cx, |i, cx| i.set_value(pattern, window, cx));
        let items = self.rules_page_items(cx);
        if let Some(ix) = items
            .iter()
            .position(|it| matches!(it, RulesItem::Rule { id } if *id == rule_id))
        {
            self.rules_reveal = Some(rule_id);
            self.rules_scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        // Navigate last (3.80): the emit runs at the end of this update,
        // so `go_page` and `set_visible(true)` — which keep the state set
        // here — cannot clobber the search text or the scroll.
        cx.emit(PageEvent::Go(crate::ui::Page::Rules));
    }

    /// Drop the page's transient state after a restore replaced the whole
    /// database behind the UI (5.3): the ids here would name rows the new
    /// file may not have, or ids reused for other rules.
    pub(super) fn reset_after_restore(&mut self, cx: &mut Context<Self>) {
        self.close_rule_editor();
        self.armed_delete_rule = None;
        self.new_rule_open = false;
        self.selected_rules.clear();
        self.collapsed_rule_groups.clear();
        self.rules_move_error = None;
        self.rules_reveal = None;
        cx.notify();
    }
}

impl RulesPage {
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
            .debug_selector(move || format!("rules:group:{id}"))
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
    /// in test mode, Type, Uses (right-aligned) and Edit / Delete. The
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
        let uses = self.rule_uses.get(&id).copied().unwrap_or(0);
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
                        .checked(self.selected_rules.contains(&id))
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
                    .icon(IconName::Pencil)
                    .label("Edit")
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
                    .debug_selector(move || format!("rules:row-edit:{id}:{}", "icons/pencil.svg"))
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.start_edit_rule(id, window, cx)
                        }),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("rule-delete-{id}")))
                    .icon(IconName::Trash)
                    .label(if armed {
                        "Click again to delete"
                    } else {
                        "Delete"
                    })
                    .ghost()
                    .text_xs()
                    .flex_shrink_0()
                    .debug_selector(move || format!("rules:row-delete:{id}:{}", "icons/trash.svg"))
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
}

impl Render for RulesPage {
    /// The Rules page (3.28, D5): header with the rule count and New rule,
    /// the add-rule row under it, the filter row (Search, Test a
    /// description, Type, Unused), the Move bar over the list while rows
    /// are checked, and the virtualized list of group headers and rule
    /// rows.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 3.28: opening the Rules page focuses its search box (D5). The
        // focus waits for the end of the frame so the input's element
        // exists first (3.80: raised by `set_visible`, like the other
        // pages' root focus).
        if self.focus_pending {
            self.focus_pending = false;
            let handle = self.rules_search.focus_handle(cx);
            window.defer(cx, move |window, cx| {
                window.focus(&handle, cx);
            });
        }
        let (muted, danger) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.danger)
        };
        let count_text = format!("{} rules", self.rules.len());
        let count_sel = count_text.clone();
        let query = self.rules_search.read(cx).value().trim().to_string();
        // The "wins" marker follows [`winning_rule`] exactly, so the page
        // marks the rule the categoriser would actually apply (D5).
        let winner = if self.rules_test_mode && !query.is_empty() {
            winning_rule(&self.rules, &query)
        } else {
            None
        };
        let items = Rc::new(self.rules_page_items(cx));
        let editing = self.editing_rule;
        let edit_error = self.rule_error.is_some();
        // Rows are 24 px; the row the editor replaced reserves
        // `RULE_EDITOR_H` (`RULE_EDITOR_ERROR_H` while a validation error
        // is open), like the Categories rule list.
        let item_sizes = Rc::new(
            items
                .iter()
                .map(|it| match it {
                    RulesItem::Group { .. } => size(px(0.), px(24.)),
                    RulesItem::Rule { id } => size(
                        px(0.),
                        if editing == Some(*id) {
                            if edit_error {
                                px(RULE_EDITOR_ERROR_H)
                            } else {
                                px(RULE_EDITOR_H)
                            }
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
                                    rule_editor_row(r, muted, danger, this, &cx.entity(), cx)
                                } else {
                                    let revealed = this.rules_reveal == Some(*id);
                                    this.rules_page_row(r, winner == Some(*id), revealed, muted, cx)
                                }
                            })
                        }
                    })
                    .collect();
                #[cfg(test)]
                this.rules_page_rows_built.set(rows.len());
                #[cfg(test)]
                this.rules_page_range.set((range.start, range.end));
                rows
            },
        )
        // 3.21: page-owned handle (see the tree).
        .track_scroll(&self.rules_scroll)
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
            // 3.80: the page's own key context, so Escape is *this* page's
            // Cancel (`ui::init` binds it per context); the root takes
            // focus when the page is shown.
            .track_focus(&self.focus)
            .key_context("Rules")
            .on_action(cx.listener(|this, _: &Cancel, _, cx| {
                // Escape unwinds one level per press (3.80,
                // *Interaction*): the open editor first, then the armed
                // delete, then the ticked rows; with nothing left it
                // propagates to the window-level bindings.
                if this.close_rule_editor() {
                    cx.notify();
                    return;
                }
                if this.armed_delete_rule.take().is_some() {
                    cx.notify();
                    return;
                }
                if !this.selected_rules.is_empty() {
                    this.selected_rules.clear();
                    cx.notify();
                    return;
                }
                cx.propagate();
            }))
            // Page header: title, count, New rule (opens the add row in
            // place, like the Categories header's New category.).
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
                            .icon(IconName::Plus)
                            .label(NEW_RULE)
                            .debug_selector(move || format!("label:{NEW_RULE}"))
                            .primary()
                            .ml_auto()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                // Only one rule editor is open at a time:
                                // opening the add row closes any row editor.
                                this.close_rule_editor();
                                this.new_rule_open = !this.new_rule_open;
                                cx.notify();
                            })),
                    ),
            )
            // Add-rule row (3.28): the same row the Categories page has,
            // hidden until the header button opens it.
            .when(self.new_rule_open, |page| {
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
                                                .debug_selector(|| "rules:pattern-input:add".into())
                                                .child(Input::new(&self.rule_pattern)),
                                        ),
                                )
                                .child(
                                    // 3.42: control-height slot, centred.
                                    control_slot(
                                        "rules:regex-slot:add".into(),
                                        "rules:switch:add".into(),
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
                                        .debug_selector(|| "rules:add-rule-btn:add".into())
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
                            .child(Input::new(&self.rules_search)),
                    )
                    .child(
                        Switch::new("rules-test-mode")
                            .label("Test a description")
                            .checked(self.rules_test_mode)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.rules_test_mode = *on;
                                this.rules_scroll.set_offset(Point::default());
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .flex_shrink_0()
                            .debug_selector(|| "rules:type-select".into())
                            .child(Select::new(&self.rules_type_select).placeholder("All types")),
                    )
                    .child(
                        checkbox("rules-unused-only")
                            .label("Unused")
                            .checked(self.rules_unused_only)
                            .on_click(cx.listener(|this, on: &bool, _, cx| {
                                this.rules_unused_only = *on;
                                this.rules_scroll.set_offset(Point::default());
                                cx.notify();
                            })),
                    ),
            )
            // Move bar (D5): only while rows are checked.
            .when(!self.selected_rules.is_empty(), |page| {
                let n = self.selected_rules.len();
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
                                            Select::new(&self.move_rules_target)
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
                        .children(self.rules_move_error.clone().map(|e| {
                            div()
                                .text_sm()
                                .text_color(danger)
                                .debug_selector(|| "rules:move-error".into())
                                .child(e)
                        })),
                )
            })
            .when(!self.rules_test_mode && !items.is_empty(), |page| {
                page.child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .debug_selector(|| "rules:heading-row".into())
                        .child(div().w(px(RULE_SEL_W)).flex_shrink_0())
                        .child(
                            div().flex_1().min_w_0().child(
                                Button::new("rules-sort-pattern")
                                    .label(if self.rules_sort_by_uses {
                                        "Pattern"
                                    } else {
                                        "Pattern ↓"
                                    })
                                    .ghost()
                                    .text_xs()
                                    .debug_selector(|| "rules:sort-pattern".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rules_sort_by_uses = false;
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
                                    .label(if self.rules_sort_by_uses {
                                        "Uses ↓"
                                    } else {
                                        "Uses"
                                    })
                                    .ghost()
                                    .text_xs()
                                    .debug_selector(|| "rules:sort-uses".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rules_sort_by_uses = true;
                                        cx.notify();
                                    })),
                            ),
                        )
                        .child(div().flex_shrink_0().child(field_label(muted, "Actions"))),
                )
            })
            .when(items.is_empty(), |page| {
                page.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .debug_selector(|| "rules:empty".into())
                        .child(if self.rules_test_mode {
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

impl Shell {
    /// 3.80: thin wrappers over the free row builders — the Categories
    /// page paints the shared rule rows from the Rules page's state (the
    /// `rules` module docs). 3.49: `uses` and `subcat` come from the
    /// Categories page, which mirrors the catalog itself.
    pub(super) fn rule_row(
        &self,
        rule: &Rule,
        muted: Hsla,
        uses: usize,
        subcat: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = &self.rules_page;
        rule_row(rule, muted, uses, subcat, entity.read(cx), entity)
    }

    pub(super) fn rule_editor_row(
        &self,
        rule: &Rule,
        muted: Hsla,
        danger: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = &self.rules_page;
        rule_editor_row(rule, muted, danger, entity.read(cx), entity, cx)
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
                    self.rules_page.update(cx, |p, cx| {
                        p.rule_error = Some("Choose the subcategory the rule assigns.".into());
                        cx.notify();
                    });
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
        self.rules_page.update(cx, |p, cx| {
            p.commit_new_rule(category_id, window, cx);
        });
    }
}
